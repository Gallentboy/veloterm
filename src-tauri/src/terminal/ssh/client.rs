use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::time::Duration;
use std::{fmt, io};

use anyhow::Result;
use bytes::Bytes;
use futures::{FutureExt, pin_mut};
use russh::client::{AuthResult, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{PrivateKeyWithHashAlg, PublicKey};
use russh::{MethodKind, Preferred, Sig, kex, mac};
use serde::Serialize;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc::{
    Receiver, Sender, UnboundedReceiver, UnboundedSender, channel, unbounded_channel,
};
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;

use super::channel_direct_tcpip::DirectTCPIPChannel;
use super::channel_session::SessionChannel;
use super::common::{
    ChannelOperation, DirectTCPIPParams, ForwardedStreamlocalParams, ForwardedTcpIpParams,
    SSHTargetAuth, TargetSSHOptions,
};
use super::error::{ConnectionError, SshClientError};
use super::handler::{ClientHandler, ClientHandlerError, ClientHandlerEvent};
use super::keys::{discover_default_local_keys, load_client_private_key, public_key_line};
use super::known_hosts::KnownHosts;
use crate::common::SafeUsizeCounter;

/// Follow `jump_host` links from `start`, returning the ordered target ids of
/// the chain (the target itself first, then each successive jump host).
/// Migrated from `warpgate-protocol-ssh/src/client/mod.rs::resolve_chain_ids`.
pub fn resolve_chain_ids(
    start: usize,
    lookup: impl Fn(usize) -> Option<Option<usize>>,
) -> Result<Vec<usize>, String> {
    let mut ids = vec![];
    let mut visited = HashSet::new();
    let mut current = Some(start);
    while let Some(id) = current {
        if !visited.insert(id) {
            return Err(format!(
                "SSH jump host chain contains a cycle at target {id}"
            ));
        }
        let Some(jump_host) = lookup(id) else {
            return Err(format!(
                "SSH jump host {id} does not resolve to an SSH target"
            ));
        };
        ids.push(id);
        current = jump_host;
    }
    Ok(ids)
}

#[derive(Debug)]
pub enum RCEvent {
    State(RCState),
    Output(usize, Bytes),
    Success(usize),
    ChannelFailure(usize),
    Eof(usize),
    Close(usize),
    Error(anyhow::Error),
    ExitStatus(usize, u32),
    ExitSignal {
        channel: usize,
        signal_name: Sig,
        core_dumped: bool,
        error_message: String,
        lang_tag: String,
    },
    ExtendedData {
        channel: usize,
        data: Bytes,
        ext: u32,
    },
    ConnectionError(ConnectionError),
    HopConnected,
    Done,
    HostKeyReceived(PublicKey, String, u16),
    HostKeyUnknown(PublicKey, String, u16, oneshot::Sender<bool>),
    ForwardedTcpIp(usize, ForwardedTcpIpParams),
    ForwardedStreamlocal(usize, ForwardedStreamlocalParams),
    ForwardedAgent(usize),
    X11(usize, String, u32),
}

impl RCEvent {
    /// The already-open channel this event refers to, if any.
    #[allow(dead_code)]
    pub const fn channel(&self) -> Option<usize> {
        match self {
            Self::Output(channel, _)
            | Self::Success(channel)
            | Self::ChannelFailure(channel)
            | Self::Eof(channel)
            | Self::Close(channel)
            | Self::ExitStatus(channel, _)
            | Self::ExitSignal { channel, .. }
            | Self::ExtendedData { channel, .. } => Some(*channel),
            Self::State(_)
            | Self::Error(_)
            | Self::ConnectionError(_)
            | Self::HopConnected
            | Self::Done
            | Self::HostKeyReceived(..)
            | Self::HostKeyUnknown(..)
            | Self::ForwardedTcpIp(..)
            | Self::ForwardedStreamlocal(..)
            | Self::ForwardedAgent(_)
            | Self::X11(..) => None,
        }
    }
}

pub type RCCommandReply = oneshot::Sender<Result<(), SshClientError>>;

enum ChannelOpen {
    Session,
    DirectTcpIp(DirectTCPIPParams),
    DirectStreamlocal(String),
}

// 不 derive Clone：ExecCapture 携带一次性的 oneshot 回捕端（全仓对
// RCCommand 只做所有权 send，无克隆需求）；Debug 手写（InnerEvent 需要）
pub enum RCCommand {
    Connect(Vec<TargetSSHOptions>),
    Channel(usize, ChannelOperation),
    /// 旁路一次性命令执行（资源监控采集）：在连接上开独立 exec 通道并捕获
    /// 完整输出，不经交互 shell、不占用既有 channel、不产生 RCEvent（回捕结果
    /// 经自带 oneshot 返回）。未连接时立即回 Err
    ExecCapture {
        command: String,
        reply: oneshot::Sender<Result<String, String>>,
    },
    #[allow(dead_code)]
    ForwardTCPIP(String, u32),
    #[allow(dead_code)]
    CancelTCPIPForward(String, u32),
    #[allow(dead_code)]
    StreamlocalForward(String),
    #[allow(dead_code)]
    CancelStreamlocalForward(String),
    Disconnect,
}

/// 手写 Debug：ExecCapture 的 oneshot 回捕端不可格式化，仅展示变体名
impl fmt::Debug for RCCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RCCommand::Connect(_) => f.write_str("Connect(..)"),
            RCCommand::Channel(id, op) => write!(f, "Channel({id}, {op:?})"),
            RCCommand::ExecCapture { .. } => f.write_str("ExecCapture{..}"),
            RCCommand::ForwardTCPIP(_, port) => write!(f, "ForwardTCPIP(_, {port})"),
            RCCommand::CancelTCPIPForward(_, port) => {
                write!(f, "CancelTCPIPForward(_, {port})")
            }
            RCCommand::StreamlocalForward(_) => f.write_str("StreamlocalForward(..)"),
            RCCommand::CancelStreamlocalForward(_) => {
                f.write_str("CancelStreamlocalForward(..)")
            }
            RCCommand::Disconnect => f.write_str("Disconnect"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum RCState {
    NotInitialized,
    Connecting,
    Connected,
    Disconnected,
}

#[derive(Debug)]
enum InnerEvent {
    RCCommand(RCCommand, Option<RCCommandReply>),
    ClientHandlerEvent(ClientHandlerEvent),
    Connected(Result<Connection, ConnectionError>),
}

pub struct Connection(
    pub Handle<ClientHandler>,
    pub UnboundedReceiver<ClientHandlerEvent>,
);

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Connection")
    }
}

pub async fn connect_ssh_chain(
    session_id: usize,
    chain: Vec<TargetSSHOptions>,
    known_hosts: KnownHosts,
) -> Result<Connection, ConnectionError> {
    let (tx, _rx) = channel(64);
    let connector = Connector {
        id: session_id,
        app_handle: None,
        host_id: None,
        tx,
        known_hosts,
    };
    connector.connect_chain(chain).await
}

struct Connector {
    id: usize,
    /// AppHandle（预留给后续 kbd-int UI 交互；终端问答路径未用）
    #[allow(dead_code)]
    app_handle: Option<tauri::AppHandle>,
    /// 主机库 id（kbd-int 改密成功后回写保存密码；None=临时连接不回写）
    host_id: Option<usize>,
    tx: Sender<RCEvent>,
    known_hosts: KnownHosts,
}


/// 认证期终端输入等待者（标准 ssh 客户端行为的复刻）：kbd-int 提示写到
/// 终端显示后，等待用户在该终端按键（terminal_write → Write 命令 → 此处），
/// 拼行直到回车作为应答。同一时刻只有一个连接在认证，单槽即可。
pub static AUTH_INPUT_WAITER: std::sync::LazyLock<
    std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

/// 远程 SSH 客户端状态机（迁移自 `warpgate-protocol-ssh/src/client/mod.rs::RemoteClient`）：
/// - 支持在连接尚未完成时预入队 `ChannelOperation`（如 `OpenShell` / `RequestPty` / `RequestShell` / `ResizePty`）；
/// - 支持单跳直连与多跳 Jump Host 链式隧道（`channel_open_direct_tcpip` + `connect_stream`）；
/// - 支持密码、私钥（PEM/OpenSSH/文件路径 + Passphrase）、本地 `ssh-agent` 与 `keyboard-interactive` 认证。
pub struct RemoteClient {
    id: usize,
    /// AppHandle（kbd-int 改密输入事件用）
    app_handle: Option<tauri::AppHandle>,
    /// 主机库 id（改密成功回写；None=临时连接）
    host_id: Option<usize>,
    tx: Sender<RCEvent>,
    session: Option<Arc<Handle<ClientHandler>>>,
    channel_pipes: Arc<Mutex<HashMap<usize, UnboundedSender<ChannelOperation>>>>,
    pending_ops: Vec<(usize, ChannelOperation)>,
    pending_forwards: Vec<(String, u32)>,
    pending_streamlocal_forwards: Vec<String>,
    state: RCState,
    abort_rx: UnboundedReceiver<()>,
    connecting: Option<JoinHandle<()>>,
    inner_event_rx: UnboundedReceiver<InnerEvent>,
    inner_event_tx: UnboundedSender<InnerEvent>,
    child_tasks: Vec<JoinHandle<Result<(), SshClientError>>>,
    known_hosts: KnownHosts,
    channel_counter: SafeUsizeCounter,
}

pub struct RemoteClientHandles {
    pub event_rx: Receiver<RCEvent>,
    pub command_tx: UnboundedSender<(RCCommand, Option<RCCommandReply>)>,
    pub abort_tx: UnboundedSender<()>,
}

impl RemoteClient {
    pub fn create(
        id: usize,
        known_hosts: KnownHosts,
        app_handle: Option<tauri::AppHandle>,
        host_id: Option<usize>,
    ) -> io::Result<RemoteClientHandles> {
        let (event_tx, event_rx) = channel(1024);
        let (command_tx, mut command_rx) = unbounded_channel();
        let (abort_tx, abort_rx) = unbounded_channel();
        let (inner_event_tx, inner_event_rx) = unbounded_channel();

        let this = Self {
            id,
            app_handle,
            host_id,
            tx: event_tx,
            session: None,
            channel_pipes: Arc::new(Mutex::new(HashMap::new())),
            pending_ops: vec![],
            pending_forwards: vec![],
            pending_streamlocal_forwards: vec![],
            state: RCState::NotInitialized,
            inner_event_rx,
            inner_event_tx: inner_event_tx.clone(),
            child_tasks: vec![],
            known_hosts,
            abort_rx,
            connecting: None,
            channel_counter: SafeUsizeCounter::new(100),
        };

        tokio::spawn(async move {
            while let Some((e, response)) = command_rx.recv().await {
                if inner_event_tx
                    .send(InnerEvent::RCCommand(e, response))
                    .is_err()
                {
                    break;
                }
            }
        });

        this.start();

        Ok(RemoteClientHandles {
            event_rx,
            command_tx,
            abort_tx,
        })
    }

    async fn set_disconnected(&mut self) {
        self.session = None;
        for (id, op) in self.pending_ops.drain(..) {
            if matches!(
                op,
                ChannelOperation::OpenShell | ChannelOperation::OpenDirectTCPIP(..)
            ) {
                let _ = self.tx.try_send(RCEvent::Close(id));
            }
        }
        let _ = self.set_state(RCState::Disconnected).await;
        let _ = self.tx.send(RCEvent::Done).await;
    }

    async fn set_state(&mut self, state: RCState) -> Result<(), SshClientError> {
        self.state = state.clone();
        self.tx
            .send(RCEvent::State(state))
            .await
            .map_err(|_| SshClientError::MpscError)?;
        Ok(())
    }

    async fn apply_channel_op(
        &mut self,
        channel_id: usize,
        op: ChannelOperation,
        reply: &mut Option<RCCommandReply>,
    ) -> Result<(), SshClientError> {
        if self.state != RCState::Connected {
            self.pending_ops.push((channel_id, op));
            return Ok(());
        }

        match op {
            ChannelOperation::OpenShell => {
                self.open_channel(channel_id, ChannelOpen::Session, reply.take())
                    .await?;
            }
            ChannelOperation::OpenDirectTCPIP(params) => {
                self.open_channel(channel_id, ChannelOpen::DirectTcpIp(params), reply.take())
                    .await?;
            }
            ChannelOperation::OpenDirectStreamlocal(path) => {
                self.open_channel(
                    channel_id,
                    ChannelOpen::DirectStreamlocal(path),
                    reply.take(),
                )
                .await?;
            }
            op => {
                let mut channel_pipes = self.channel_pipes.lock().await;
                if let Some(tx) = channel_pipes.get(&channel_id) {
                    if tx.send(op).is_err() {
                        channel_pipes.remove(&channel_id);
                    }
                } else {
                    log::debug!(
                        "[SSH {}] operation for unknown channel {}",
                        self.id,
                        channel_id
                    );
                }
            }
        }
        Ok(())
    }

    pub fn start(mut self) -> JoinHandle<anyhow::Result<()>> {
        tokio::spawn(async move {
            let run_res: anyhow::Result<()> = async {
                loop {
                    tokio::select! {
                      Some(event) = self.inner_event_rx.recv() => {
                        log::trace!("[SSH {}] inner event: {:?}", self.id, event);
                        if self.handle_event(event).await? {
                          break;
                        }
                      }
                      Some(()) = self.abort_rx.recv() => {
                        log::debug!("[SSH {}] Abort requested", self.id);
                        self.disconnect().await;
                        break;
                      }
                    }
                }
                Ok(())
            }
            .await;

            if let Err(error) = run_res {
                log::error!("[SSH {}] error in command loop: {:?}", self.id, error);
                let _ = self.tx.try_send(RCEvent::Error(error));
            }
            log::info!("[SSH {}] Client session closed", self.id);
            Ok(())
        })
    }

    async fn handle_event(&mut self, event: InnerEvent) -> Result<bool> {
        match event {
            InnerEvent::RCCommand(cmd, mut reply) => {
                let result = self.handle_command(cmd, &mut reply).await;
                let brk = matches!(result, Ok(true));
                if let Some(reply) = reply {
                    let _ = reply.send(result.map(|_| ()));
                }
                return Ok(brk);
            }
            InnerEvent::Connected(result) => {
                self.connecting = None;
                match result {
                    Ok(connection) => self.on_connected(connection).await?,
                    Err(e) => {
                        log::warn!("[SSH {}] Connect error: {}", self.id, e);
                        let _ = self.tx.send(RCEvent::ConnectionError(e)).await;
                        self.set_disconnected().await;
                        return Ok(true);
                    }
                }
            }
            InnerEvent::ClientHandlerEvent(client_event) => {
                log::debug!("[SSH {}] Client handler event: {:?}", self.id, client_event);
                match client_event {
                    ClientHandlerEvent::Disconnect => {
                        self._on_disconnect().await;
                    }
                    ClientHandlerEvent::ForwardedTcpIp(channel, params) => {
                        log::info!("[SSH {}] New forwarded connection: {:?}", self.id, params);
                        let id = self.setup_server_initiated_channel(channel).await?;
                        let _ = self.tx.send(RCEvent::ForwardedTcpIp(id, params)).await;
                    }
                    ClientHandlerEvent::ForwardedStreamlocal(channel, params) => {
                        log::info!(
                            "[SSH {}] New forwarded socket connection: {:?}",
                            self.id,
                            params
                        );
                        let id = self.setup_server_initiated_channel(channel).await?;
                        let _ = self
                            .tx
                            .send(RCEvent::ForwardedStreamlocal(id, params))
                            .await;
                    }
                    ClientHandlerEvent::ForwardedAgent(channel) => {
                        log::info!("[SSH {}] New forwarded agent connection", self.id);
                        let id = self.setup_server_initiated_channel(channel).await?;
                        let _ = self.tx.send(RCEvent::ForwardedAgent(id)).await;
                    }
                    ClientHandlerEvent::X11(channel, originator_address, originator_port) => {
                        log::info!(
                            "[SSH {}] New X11 connection from {}:{}",
                            self.id,
                            originator_address,
                            originator_port
                        );
                        let id = self.setup_server_initiated_channel(channel).await?;
                        let _ = self
                            .tx
                            .send(RCEvent::X11(id, originator_address, originator_port))
                            .await;
                    }
                    event => {
                        log::warn!(
                            "[SSH {}] Unhandled client handler event: {:?}",
                            self.id,
                            event
                        );
                    }
                }
            }
        }
        Ok(false)
    }

    async fn setup_server_initiated_channel(
        &mut self,
        channel: russh::Channel<russh::client::Msg>,
    ) -> Result<usize> {
        let id = self.channel_counter.next();
        let (tx, rx) = unbounded_channel();
        self.channel_pipes.lock().await.insert(id, tx);

        let session_channel = SessionChannel::new(channel, id, rx, self.tx.clone(), self.id);
        self.child_tasks.push(tokio::spawn(session_channel.run()));
        Ok(id)
    }

    async fn handle_command(
        &mut self,
        cmd: RCCommand,
        reply: &mut Option<RCCommandReply>,
    ) -> Result<bool, SshClientError> {
        match cmd {
            RCCommand::Connect(chain) => {
                self.set_state(RCState::Connecting).await?;
                let connector = Connector {
                    id: self.id,
                    app_handle: self.app_handle.clone(),
                    host_id: self.host_id,
                    tx: self.tx.clone(),
                    known_hosts: self.known_hosts.clone(),
                };
                let inner_event_tx = self.inner_event_tx.clone();
                let attempt = tokio::spawn(async move {
                    let _ = inner_event_tx
                        .send(InnerEvent::Connected(connector.connect_chain(chain).await));
                });
                if let Some(previous) = self.connecting.replace(attempt) {
                    previous.abort();
                }
            }
            RCCommand::Channel(ch, op) => {
                // 认证期终端输入（标准 ssh 复刻）：kbd-int 改密询问在等用户
                // 输入时，Write 命令的字节转给等待者（拼行回车提交），
                // 不进 channel（此时 shell channel 未开）
                if let ChannelOperation::Data(ref data) = op {
                    let waiter = AUTH_INPUT_WAITER.lock().unwrap().clone();
                    if let Some(tx) = waiter {
                        let _ = tx.send(data.to_vec());
                        return Ok(false);
                    }
                }
                self.apply_channel_op(ch, op, reply).await?;
            }
            RCCommand::ExecCapture { command, reply } => {
                // 未连接直接回错（资源监控等轮询方据此降级，不排队重试）
                let Some(session) = self.session.clone() else {
                    let _ = reply.send(Err("SSH 未连接".to_string()));
                    return Ok(false);
                };
                // 独立通道旁路执行：不占用 channel_pipes / 不产生 RCEvent，
                // 结果直接经 oneshot 回捕；全程限时防止慢链路挂死轮询方
                tokio::spawn(async move {
                    let result = async {
                        let channel = tokio::time::timeout(
                            Duration::from_secs(8),
                            session.channel_open_session(),
                        )
                        .await
                        .map_err(|_| "旁路命令超时：打开通道失败".to_string())?
                        .map_err(|e| format!("打开通道失败: {e}"))?;
                        channel
                            .exec(true, command)
                            .await
                            .map_err(|e| format!("执行旁路命令失败: {e}"))?;
                        let mut stream = channel.into_stream();
                        let mut output = String::new();
                        tokio::time::timeout(
                            Duration::from_secs(8),
                            stream.read_to_string(&mut output),
                        )
                        .await
                        .map_err(|_| "旁路命令超时：读取输出失败".to_string())?
                        .map_err(|e| format!("读取旁路命令输出失败: {e}"))?;
                        Ok(output)
                    }
                    .await;
                    let _ = reply.send(result);
                });
            }
            RCCommand::ForwardTCPIP(address, port) => {
                self.tcpip_forward(address, port).await?;
            }
            RCCommand::CancelTCPIPForward(address, port) => {
                self.cancel_tcpip_forward(address, port).await?;
            }
            RCCommand::StreamlocalForward(socket_path) => {
                self.streamlocal_forward(socket_path).await?;
            }
            RCCommand::CancelStreamlocalForward(socket_path) => {
                self.cancel_streamlocal_forward(socket_path).await?;
            }
            RCCommand::Disconnect => {
                self.disconnect().await;
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn on_connected(
        &mut self,
        Connection(session, mut event_rx): Connection,
    ) -> Result<(), SshClientError> {
        self.session = Some(Arc::new(session));
        log::info!("[SSH {}] Connected", self.id);

        let inner_event_tx = self.inner_event_tx.clone();
        tokio::spawn(async move {
            while let Some(e) = event_rx.recv().await {
                if inner_event_tx
                    .send(InnerEvent::ClientHandlerEvent(e))
                    .is_err()
                {
                    break;
                }
            }
        });

        self.set_state(RCState::Connected).await?;

        let ops = std::mem::take(&mut self.pending_ops);
        for (id, op) in ops {
            self.apply_channel_op(id, op, &mut None).await?;
        }

        let forwards = std::mem::take(&mut self.pending_forwards);
        for (address, port) in forwards {
            self.tcpip_forward(address, port).await?;
        }

        let forwards = std::mem::take(&mut self.pending_streamlocal_forwards);
        for socket_path in forwards {
            self.streamlocal_forward(socket_path).await?;
        }

        Ok(())
    }

    async fn open_channel(
        &mut self,
        channel_id: usize,
        op: ChannelOpen,
        reply: Option<RCCommandReply>,
    ) -> Result<(), SshClientError> {
        let Some(session) = self.session.clone() else {
            if let Some(reply) = reply {
                let _ = reply.send(Ok(()));
            }
            return Ok(());
        };

        let (ops_tx, ops_rx) = unbounded_channel();
        // Registered before the open completes so that operations arriving for
        // this channel meanwhile are buffered instead of reported as unknown.
        self.channel_pipes.lock().await.insert(channel_id, ops_tx);

        let channel_pipes = self.channel_pipes.clone();
        let events_tx = self.tx.clone();
        let session_id = self.id;

        let task = async move {
            let is_session = matches!(op, ChannelOpen::Session);
            let opened = match op {
                ChannelOpen::Session => session.channel_open_session().await,
                ChannelOpen::DirectTcpIp(params) => {
                    session
                        .channel_open_direct_tcpip(
                            params.host_to_connect,
                            params.port_to_connect,
                            params.originator_address,
                            params.originator_port,
                        )
                        .await
                }
                ChannelOpen::DirectStreamlocal(path) => {
                    session.channel_open_direct_streamlocal(path).await
                }
            };

            let channel = match opened {
                Ok(channel) => channel,
                Err(error) => {
                    channel_pipes.lock().await.remove(&channel_id);
                    if let Some(reply) = reply {
                        let _ = reply.send(Err(error.into()));
                    } else {
                        log::error!(
                            "[SSH {}] Failed to open channel {}: {:?}",
                            session_id,
                            channel_id,
                            error
                        );
                        let _ = events_tx.send(RCEvent::Close(channel_id)).await;
                    }
                    return Ok(());
                }
            };

            if let Some(reply) = reply {
                let _ = reply.send(Ok(()));
            }

            if is_session {
                SessionChannel::new(channel, channel_id, ops_rx, events_tx, session_id)
                    .run()
                    .await
            } else {
                DirectTCPIPChannel::new(channel, channel_id, ops_rx, events_tx, session_id)
                    .run()
                    .await
            }
        };

        self.child_tasks.push(tokio::spawn(task));
        Ok(())
    }

    async fn tcpip_forward(&mut self, address: String, port: u32) -> Result<(), SshClientError> {
        if let Some(session) = &self.session {
            session.tcpip_forward(address, port).await?;
        } else {
            self.pending_forwards.push((address, port));
        }
        Ok(())
    }

    async fn cancel_tcpip_forward(
        &mut self,
        address: String,
        port: u32,
    ) -> Result<(), SshClientError> {
        if let Some(session) = &self.session {
            session.cancel_tcpip_forward(address, port).await?;
        } else {
            self.pending_forwards
                .retain(|x| x.0 != address || x.1 != port);
        }
        Ok(())
    }

    async fn streamlocal_forward(&mut self, socket_path: String) -> Result<(), SshClientError> {
        if let Some(session) = &self.session {
            session.streamlocal_forward(socket_path).await?;
        } else {
            self.pending_streamlocal_forwards.push(socket_path);
        }
        Ok(())
    }

    async fn cancel_streamlocal_forward(
        &mut self,
        socket_path: String,
    ) -> Result<(), SshClientError> {
        if let Some(session) = &self.session {
            session.cancel_streamlocal_forward(socket_path).await?;
        } else {
            self.pending_streamlocal_forwards
                .retain(|x| x != &socket_path);
        }
        Ok(())
    }

    async fn disconnect(&mut self) {
        if let Some(attempt) = self.connecting.take() {
            log::info!("[SSH {}] Abort requested while connecting", self.id);
            attempt.abort();
            let _ = self
                .tx
                .send(RCEvent::ConnectionError(ConnectionError::Aborted))
                .await;
            self.set_disconnected().await;
        } else if let Some(session) = &mut self.session {
            let _ = session
                .disconnect(russh::Disconnect::ByApplication, "", "")
                .await;
            self.set_disconnected().await;
        }
    }

    async fn _on_disconnect(&mut self) {
        self.set_disconnected().await;
    }
}

impl Drop for RemoteClient {
    fn drop(&mut self) {
        if let Some(attempt) = self.connecting.take() {
            attempt.abort();
        }
        for task in self.child_tasks.drain(..) {
            task.abort();
        }
        log::info!("[SSH {}] RemoteClient dropped", self.id);
    }
}

impl Connector {
    fn build_ssh_config(&self, ssh_options: &TargetSSHOptions) -> Arc<russh::client::Config> {
        let algos = if ssh_options.allow_insecure_algos {
            Preferred {
                kex: Cow::Borrowed(&[
                    kex::MLKEM768X25519_SHA256,
                    kex::CURVE25519,
                    kex::CURVE25519_PRE_RFC_8731,
                    kex::ECDH_SHA2_NISTP256,
                    kex::ECDH_SHA2_NISTP384,
                    kex::ECDH_SHA2_NISTP521,
                    kex::DH_G16_SHA512,
                    kex::DH_G14_SHA256,
                    kex::DH_GEX_SHA256,
                    kex::DH_G1_SHA1,
                    kex::EXTENSION_SUPPORT_AS_CLIENT,
                    kex::EXTENSION_SUPPORT_AS_SERVER,
                    kex::EXTENSION_OPENSSH_STRICT_KEX_AS_CLIENT,
                    kex::EXTENSION_OPENSSH_STRICT_KEX_AS_SERVER,
                ]),
                key: Cow::Borrowed(&[
                    russh::keys::Algorithm::Ed25519,
                    russh::keys::Algorithm::Ecdsa {
                        curve: russh::keys::EcdsaCurve::NistP256,
                    },
                    russh::keys::Algorithm::Ecdsa {
                        curve: russh::keys::EcdsaCurve::NistP384,
                    },
                    russh::keys::Algorithm::Ecdsa {
                        curve: russh::keys::EcdsaCurve::NistP521,
                    },
                    russh::keys::Algorithm::Rsa {
                        hash: Some(russh::keys::HashAlg::Sha256),
                    },
                    russh::keys::Algorithm::Rsa {
                        hash: Some(russh::keys::HashAlg::Sha512),
                    },
                    russh::keys::Algorithm::Rsa { hash: None },
                    russh::keys::Algorithm::Dsa,
                ]),
                cipher: Cow::Borrowed(&[
                    russh::cipher::CHACHA20_POLY1305,
                    russh::cipher::AES_256_GCM,
                    russh::cipher::AES_256_CTR,
                    russh::cipher::AES_256_CBC,
                    russh::cipher::AES_192_CTR,
                    russh::cipher::AES_192_CBC,
                    russh::cipher::AES_128_CTR,
                    russh::cipher::AES_128_CBC,
                    russh::cipher::TRIPLE_DES_CBC,
                ]),
                mac: Cow::Borrowed(&[
                    mac::HMAC_SHA512_ETM,
                    mac::HMAC_SHA256_ETM,
                    mac::HMAC_SHA512,
                    mac::HMAC_SHA256,
                    mac::HMAC_SHA1_ETM,
                    mac::HMAC_SHA1,
                ]),
                ..<_>::default()
            }
        } else {
            Preferred::default()
        };

        let mut config = russh::client::Config {
            preferred: algos,
            nodelay: true,
            inactivity_timeout: Some(Duration::from_secs(3600)),
            keepalive_interval: Some(Duration::from_secs(30)),
            ..Default::default()
        };
        if ssh_options.allow_insecure_algos
            && let Ok(gex) = russh::client::GexParams::new(2048, 2048, 8192)
        {
            config.gex = gex;
        }
        Arc::new(config)
    }

    /// Connect through a pre-resolved chain of SSH hops, each tunnelled through the previous.
    /// `chain` must be non-empty; the first entry is connected directly, subsequent ones via
    /// `channel_open_direct_tcpip` through the previous session.
    async fn connect_chain(
        &self,
        chain: Vec<TargetSSHOptions>,
    ) -> Result<Connection, ConnectionError> {
        let mut iter = chain.into_iter();
        let first = iter.next().ok_or(ConnectionError::Resolve)?;

        let config = self.build_ssh_config(&first);
        let (event_tx, event_rx) = unbounded_channel();
        let handler = ClientHandler {
            ssh_options: first.clone(),
            event_tx,
            known_hosts: self.known_hosts.clone(),
            session_id: self.id,
        };

        // 首跳分支：中继（VLR2 握手后的传输流上 connect_stream）或直连 TCP。
        // 中继把流量送达链首 SSH 端点，后续跳板链逻辑不变。
        let (mut session, mut active_rx) = match first.relay.clone() {
            Some(relay_cfg) => {
                let ssh_endpoint = veloterm_relay_proto::frame::TargetAddr {
                    host: first.host.clone(),
                    port: first.port,
                };
                log::info!(
                    "[SSH {}] Connecting via relay {}:{} ({:?}) to {}:{}...",
                    self.id,
                    relay_cfg.host,
                    relay_cfg.port,
                    relay_cfg.transport,
                    first.host,
                    first.port,
                );
                let stream = super::relay_transport::dial_relay(&relay_cfg, &ssh_endpoint)
                    .await
                    .map_err(|e| ConnectionError::Io(std::io::Error::other(e)))?;
                let fut = russh::client::connect_stream(config, stream, handler);
                self.wait_for_connection(&first, fut, event_rx, false)
                    .boxed()
                    .await?
            }
            None => {
                let address_str = format!("{}:{}", first.host, first.port);
                let address = address_str
                    .to_socket_addrs()
                    .map_err(ConnectionError::Io)
                    .and_then(|mut x| x.next().ok_or(ConnectionError::Resolve))
                    .inspect_err(|e| {
                        log::error!(
                            "[SSH {}] Cannot resolve address {}: {:?}",
                            self.id,
                            address_str,
                            e
                        )
                    })?;
                log::info!(
                    "[SSH {}] Connecting to {:?} as {}",
                    self.id,
                    address,
                    first.username
                );
                let fut = russh::client::connect(config, address, handler);
                self.wait_for_connection(&first, fut, event_rx, false)
                    .boxed()
                    .await?
            }
        };

        for ssh_options in iter {
            let _ = self.tx.send(RCEvent::HopConnected).await;
            log::info!(
                "[SSH {}] Opening direct-tcpip channel through jump host to {}:{}",
                self.id,
                ssh_options.host,
                ssh_options.port
            );
            let channel = session
                .channel_open_direct_tcpip(
                    ssh_options.host.clone(),
                    u32::from(ssh_options.port),
                    "localhost".to_string(),
                    0,
                )
                .await
                .map_err(ConnectionError::Ssh)?;
            let stream = channel.into_stream();
            let config = self.build_ssh_config(&ssh_options);
            let (event_tx, event_rx) = unbounded_channel();
            let handler = ClientHandler {
                ssh_options: ssh_options.clone(),
                event_tx,
                known_hosts: self.known_hosts.clone(),
                session_id: self.id,
            };
            let fut = russh::client::connect_stream(config, stream, handler);
            let (new_session, new_rx) = self
                .wait_for_connection(&ssh_options, fut, event_rx, false)
                .boxed()
                .await?;
            session = new_session;
            active_rx = new_rx;
        }

        Ok(Connection(session, active_rx))
    }

    async fn wait_for_connection<Fut>(
        &self,
        ssh_options: &TargetSSHOptions,
        fut_connect: Fut,
        mut event_rx: UnboundedReceiver<ClientHandlerEvent>,
        _is_jump_host: bool,
    ) -> Result<(Handle<ClientHandler>, UnboundedReceiver<ClientHandlerEvent>), ConnectionError>
    where
        Fut: Future<Output = Result<Handle<ClientHandler>, ClientHandlerError>>,
    {
        pin_mut!(fut_connect);

        loop {
            tokio::select! {
              Some(event) = event_rx.recv() => {
                match event {
                  ClientHandlerEvent::HostKeyReceived(key) => {
                    self.tx
                      .send(RCEvent::HostKeyReceived(
                        key,
                        ssh_options.host.clone(),
                        ssh_options.port,
                      ))
                      .await
                      .map_err(|_| ConnectionError::Internal)?;
                  }
                  ClientHandlerEvent::HostKeyUnknown(key, reply) => {
                    self.tx
                      .send(RCEvent::HostKeyUnknown(
                        key,
                        ssh_options.host.clone(),
                        ssh_options.port,
                        reply,
                      ))
                      .await
                      .map_err(|_| ConnectionError::Internal)?;
                  }
                  _ => {}
                }
              }
              session = &mut fut_connect => {
                let mut session = match session {
                  Ok(session) => session,
                  Err(error) => {
                    let connection_error = match error {
                      ClientHandlerError::ConnectionError(e) => e,
                      ClientHandlerError::Ssh(e) => ConnectionError::Ssh(e),
                      ClientHandlerError::Internal => ConnectionError::Internal,
                    };
                    log::error!("[SSH {}] Connection error: {:?}", self.id, connection_error);
                    return Err(connection_error);
                  }
                };

                self.authenticate_session(
                  &mut session,
                  &ssh_options.username,
                  &ssh_options.auth,
                  ssh_options.allow_insecure_algos,
                ).await?;

                return Ok((session, event_rx));
              }
            }
        }
    }

    /// 终端内询问（标准 ssh 行为复刻）：提示写到终端显示，等待用户在
    /// 该终端输入一行（回车提交）。输入不回显（PAM echo=0，同 ssh 客户端）。
    /// 终端内问答（复刻 OpenSSH readpassphrase 语义）。
    ///
    /// `echo = false`：密码类输入，不回显；`echo = true`：回显用户输入
    /// （用户名/验证码等，OpenSSH 用 RP_ECHO）。
    async fn ask_in_terminal(&self, prompt: &str, echo: bool) -> Option<String> {
        // 提示显示到终端。OpenSSH 的 read_passphrase 在开屏前先写一个 '\r'
        // （readpass.c:160 "ensure that show the prompt at the beginning of the
        // line"），随后 readpassphrase 写 prompt 原文。这里保持同序：\r + prompt。
        let line = format!("\r{}", prompt);
        let _ = self
            .tx
            .send(RCEvent::Output(1, bytes::Bytes::from(line)))
            .await;
        // 注册输入等待
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        *AUTH_INPUT_WAITER.lock().unwrap() = Some(tx);
        // 拼行直到回车。echo=true 时把可打印字符回显到终端（同 tty ECHO 语义）。
        let mut buf: Vec<u8> = Vec::new();
        let mut done: Option<Option<String>> = None;
        while done.is_none() {
            match tokio::time::timeout(std::time::Duration::from_secs(180), rx.recv()).await {
                Ok(Some(bytes)) => {
                    for b in bytes {
                        if b == b'\r' || b == b'\n' {
                            let text = String::from_utf8_lossy(&buf).into_owned();
                            // readpassphrase 在关闭回显时于读完行后写一个 '\n'
                            // （readpassphrase.c:166 `write(output, "\n", 1)`）；
                            // 开着回显时 tty 已随回车回显换行，不再补。
                            if !echo {
                                let _ = self
                                    .tx
                                    .send(RCEvent::Output(1, bytes::Bytes::from("\r\n")))
                                    .await;
                            }
                            done = Some(Some(text));
                            break;
                        }
                        if b == 0x7f || b == 0x08 {
                            buf.pop();
                            if echo {
                                // 回显退格：退格 + 空格 + 退格
                                let _ = self
                                    .tx
                                    .send(RCEvent::Output(
                                        1,
                                        bytes::Bytes::from("\x08 \x08"),
                                    ))
                                    .await;
                            }
                        } else {
                            buf.push(b);
                            if echo {
                                let _ = self
                                    .tx
                                    .send(RCEvent::Output(1, bytes::Bytes::from(vec![b])))
                                    .await;
                            }
                        }
                    }
                }
                _ => {
                    done = Some(None); // 超时或通道关闭
                }
            }
        }
        *AUTH_INPUT_WAITER.lock().unwrap() = None;
        done.flatten()
    }

    async fn authenticate_session(
        &self,
        session: &mut Handle<ClientHandler>,
        username: &str,
        auth: &SSHTargetAuth,
        allow_insecure_algos: bool,
    ) -> Result<(), ConnectionError> {
        let mut auth_result = false;
        let mut auth_error_msg: Option<String> = None;

        match auth {
            SSHTargetAuth::Password { password } => {
                // 【kbd-int 优先】PAM 密码过期场景：password 方法会被直接放行
                // （SUCCESS + banner，改密被推迟到登录后 shell 内强制 passwd），
                // 交互链（RFC 4256 INFO_REQUEST 三连提示）只有客户端**主动
                // 发起 keyboard-interactive** 才触发——libssh/系统 ssh 均此序。
                // 先试 kbd-int：命中交互链 → libssh 式感知循环（提示全文 +
                // 改密特征检测）；服务器不支持 → 回落 password。
                let mut kb = session
                    .authenticate_keyboard_interactive_start(username.to_string(), None)
                    .await?;
                let mut kbdint_worked = false;
                let mut round = 0u32;
                // 过期/改密流程已登记（首次 INFO 后续轮降 DEBUG 防刷屏）
                let mut expiry_logged = false;
                // 改密新密码跟踪（协议驱动）：收集改密流程中所有 echo=false 的
                // 非空应答；PAM 只在新密码两次一致时返回 SUCCESS，故成功时
                // 「最后两条相等」即为新密码（无需识别提示文本语义）
                let mut secret_answers: Vec<String> = Vec::new();
                // 普通密码提示只自动填一次（见下方 prompt 循环注释）
                let mut plain_password_filled = false;
                // 用户手输的普通密码（非改密链）：用于「保存密码错了、用户手输对了」
                // 的自动回写。判定 100% 可靠的依据是服务器亲口确认——
                //   ① 该提示是普通密码（非 current/new/retype 改密链）
                //   ② 值来自用户手输（非自动填的保存密码）
                //   ③ 本轮最终认证 Success（服务器已用该密码验证通过）
                //   ④ 本轮只有一次这样的手输（无歧义）
                // 四条同时满足才写库，任一不满足则宁可不写。
                let mut user_entered_password: Option<String> = None;
                // kbd-int 整体失败重试计数（对齐 OpenSSH sshconnect2.c：
                // `if (attempt_kbdint++ >= number_of_password_prompts) return 0;`
                // ——后置递增，默认值 3，实际是「初始 1 次 + 重试 3 次 = 4 次」）
                let mut kbdint_attempts = 0u32;
                const KBDINT_MAX_ATTEMPTS: u32 = 4;
                loop {
                    match kb {
                        KeyboardInteractiveAuthResponse::Success => {
                            // 零 prompt 的 Success = 服务器未验证密码就放行。
                            // 这是 OpenSSH 对「密码已过期」账号的行为：pam_acct_mgmt
                            // 返回 PAM_NEW_AUTHTOK_REQD 时 sshd 视认证为成功，改密
                            // 推迟到登录后的 do_pwchange()（shell 内强制 passwd）。
                            // 后果：用户在 shell 的 passwd 里输错当前密码时，passwd
                            // 失败退出但 shell 仍会起来（实测），看起来像「密码错了
                            // 还能进」——客户端无法阻止（是服务端策略），但必须让
                            // 用户知道这次登录未经密码校验。
                            if round == 0 {
                                log::warn!(
                                    "[SSH {}] keyboard-interactive 未经任何提示即 Success：\
                                     密码未被服务器校验（典型为密码过期，改密被推迟到 shell 内 passwd）",
                                    self.id
                                );
                                let notice = "\r\n\x1b[93m[提示] 服务器未校验密码即放行（密码可能已过期）。\
如果随后出现 passwd 提示，请在 shell 内完成改密；当前密码输错时 passwd 会失败，\
但服务器仍可能放行登录。\x1b[0m\r\n";
                                let _ = self
                                    .tx
                                    .send(RCEvent::Output(1, bytes::Bytes::from(notice)))
                                    .await;
                            }
                            log::info!(
                                "[SSH {}] Authenticated via keyboard-interactive as {}",
                                self.id,
                                username
                            );
                            // 保存密码错了、用户手输对了 → 自动回写（100% 可靠）：
                            // 服务器已用这个值通过认证（Success），且它来自用户手输、
                            // 不是改密链的一环、本轮无歧义（无第二次手输）。三个条件
                            // 缺一不可；改密链成功走下面 verified_new_password 路径。
                            if secret_answers.is_empty()
                                && let Some(entered) = user_entered_password.as_deref()
                                && entered != password
                                && let Some(host_id) = self.host_id
                            {
                                match crate::db::global_pool() {
                                    Some(pool) => {
                                        match crate::db::update_host_password(
                                            &pool, host_id, entered,
                                        ) {
                                            Ok(true) => {
                                                log::info!(
                                                    "[SSH {}] 用户手输密码通过认证，\
                                                     已更新主机库 hosts.id={}（原保存密码错误）",
                                                    self.id,
                                                    host_id
                                                );
                                                // 前端同步：广播密码已更新，让 dbStorage
                                                // 重拉 bootstrap 并刷新内存/React 状态，
                                                // 否则主机编辑页仍显示旧密码（同改密路径）
                                                use tauri::Emitter;
                                                if let Some(app) = self.app_handle.as_ref() {
                                                    let _ = app.emit(
                                                        "host-password-updated",
                                                        serde_json::json!({
                                                            "hostId": host_id
                                                        }),
                                                    );
                                                }
                                            }
                                            Ok(false) => log::info!(
                                                "[SSH {}] 主机 {} 非密码认证，跳过回写",
                                                self.id,
                                                host_id
                                            ),
                                            Err(e) => log::warn!(
                                                "[SSH {}] 回写手输密码失败: {}",
                                                self.id,
                                                e
                                            ),
                                        }
                                    }
                                    None => log::warn!(
                                        "[SSH {}] DB pool 不可用，手输密码未保存",
                                        self.id
                                    ),
                                }
                            }
                            kbdint_worked = true;
                            break;
                        }
                        KeyboardInteractiveAuthResponse::Failure { .. } => {
                            // 复刻 OpenSSH sshconnect2.c：kbd-int 失败**不是**终止
                            // 认证，而是由 input_userauth_failure → userauth() 重新
                            // 发起一轮（attempt_kbdint 计数上限 number_of_password_prompts，
                            // 默认 3）。密码过期场景下用户在 Current password 输错，
                            // 服务器回 FAILURE，标准 ssh 会重新发一轮 Password: 让用户
                            // 重试（实测：Password: → Current password: 输错 →
                            // 又回到 Password:）。
                            // 此前这里直接 break 回落 password 认证，于是服务器把改密
                            // 推迟到 shell 内 passwd，出现「Current 输错也能进 shell」。
                            kbdint_attempts += 1;
                            if kbdint_attempts >= KBDINT_MAX_ATTEMPTS {
                                log::warn!(
                                    "[SSH {}] kbd-int 已失败 {} 轮，停止重试",
                                    self.id,
                                    kbdint_attempts
                                );
                                break;
                            }
                            log::info!(
                                "[SSH {}] kbd-int 认证失败，重新发起（第 {} 次）",
                                self.id,
                                kbdint_attempts + 1
                            );
                            // 新一轮：清空本轮的应答收集。**不再重置
                            // plain_password_filled**——保存的密码只在第一次尝试时
                            // 自动填一次：若它错了，重试时再填同一个错密码只会让
                            // 服务器重复做 PAM 校验（pam_unix 的哈希计算刻意慢，
                            // 防暴力破解），4 次 kbd-int + 1 次 password 回落累计
                            // 5s+ 才报错（实测体感）。保持 filled=true 后，重试轮
                            // 会显示 Password: 等用户输入，失败也能立刻让用户改。
                            secret_answers.clear();
                            // 手输密码也清：失败轮的值未通过服务器验证，不能留作
                            // 写库依据（下一轮用户重新输入的才是有效的）
                            user_entered_password = None;
                            round = 0;
                            kb = session
                                .authenticate_keyboard_interactive_start(
                                    username.to_string(),
                                    None,
                                )
                                .await?;
                        }
                        KeyboardInteractiveAuthResponse::InfoRequest {
                            name,
                            instructions,
                            ref prompts,
                        } => {
                            round += 1;
                            let prompt_texts: Vec<&str> =
                                prompts.iter().map(|p| p.prompt.as_str()).collect();
                            let lower_all = prompt_texts
                                .iter()
                                .map(|p| p.to_ascii_lowercase())
                                .collect::<Vec<_>>()
                                .join("\x1e");
                            let looks_change = lower_all.contains("new password")
                                || lower_all.contains("new unix password")
                                || (lower_all.contains('新') && lower_all.contains("密码"));
                            // 检测登记（改密流程进入时打一次 INFO，验证模式）。
                            // 调试期日志：默认注释，排查时取消注释即可
                            if !expiry_logged && (looks_change || !instructions.is_empty()) {
                                expiry_logged = true;
                                // log::info!(
                                //     "[SSH-PASSWD-CHANGE] session={} round={} \\
                                //      form=keyboard-interactive name={:?} \\
                                //      instructions={:?} prompts={:?} \\
                                //      -> 终端内问答（标准 ssh 复刻）\\
                                //      (verification mode: log-only)",
                                //     self.id, round, name, instructions, prompt_texts
                                // );
                            }
                            // 终端内问答（标准 ssh 客户端行为复刻）：
                            // - instructions 有内容（PAM info/warning）→ 显示到终端
                            // - Password:/Current password: → 显示提示 + 等用户输入
                            //   （不自动填保存的密码——检测到过期即交给用户）
                            // - New password:/Retype: → 同上；Retype 校验一致后记住
                            // - 应答推进循环直到 SUCCESS/FAILURE（改密在认证内完成）
                            if round >= 20 {
                                log::warn!(
                                    "[SSH {}] kbd-int 轮次超上限（{}），中止回落 password",
                                    self.id, round
                                );
                                break;
                            }
                            // 复刻 sshconnect2.c::input_userauth_info_req 的输出顺序：
                            //   if (name 非空) fputs(name, stderr);
                            //   if (inst 非空) fputs(inst, stderr);
                            // 两处都是 addnl = (末字符 != '\n') 时才补一个 '\n'——
                            // 服务器已带换行时不重复补（此前无条件补 \r\n 会多空行）。
                            let write_block = |text: &str| {
                                if text.is_empty() {
                                    return None;
                                }
                                let needs_nl = !text.ends_with('\n');
                                Some(format!("{text}{}", if needs_nl { "\r\n" } else { "" }))
                            };
                            if let Some(block) = write_block(&name) {
                                let _ = self
                                    .tx
                                    .send(RCEvent::Output(1, bytes::Bytes::from(block)))
                                    .await;
                            }
                            if let Some(block) = write_block(&instructions) {
                                let _ = self
                                    .tx
                                    .send(RCEvent::Output(1, bytes::Bytes::from(block)))
                                    .await;
                            }
                            let mut responses: Vec<String> = Vec::with_capacity(prompts.len());
                            for p in prompts {
                                let lp = p.prompt.to_ascii_lowercase();
                                // 协议驱动的判定（RFC 4256 的 echo 字段）：
                                // PAM 提示文本由各 pam_*.so 自由书写、随 locale
                                // 翻译（pam_unix 一条提示就有几十种语言变体），
                                // 文本匹配永远追不上。但 echo 是协议层语义——
                                // echo=false 即「输入不显示」，密码类输入恒为
                                // false，用户名/OTP 码通常为 true。
                                //
                                // 改密流程的结构不变量（比文本可靠）：
                                //   1. PAM 只在新密码两次一致时才返回 SUCCESS
                                //   2. 因此「echo=false 应答序列的最后两条相等」
                                //      就是新密码，无需知道哪条提示叫 new
                                //   3. 流程中的第一个 echo=false 应答是当前密码
                                //
                                // 文本关键词仅作降级兜底：echo 全为 true 的异常
                                // 模块（少见）仍能靠它识别改密步骤。
                                let text_is_retype = lp.contains("retype")
                                    || lp.contains("re-enter")
                                    || lp.contains("重新输入")
                                    || lp.contains("再次输入");
                                let text_is_current = lp.contains("current")
                                    || lp.contains("当前")
                                    || lp.contains("原来");
                                let text_is_new = !text_is_retype
                                    && (lp.contains("new password")
                                        || lp.contains("new unix password")
                                        || (lp.contains('新') && lp.contains("密码")));
                                let text_is_change_step =
                                    text_is_current || text_is_new || text_is_retype;
                                // 是否为「需要用户输入的改密步骤」：协议信号优先
                                let is_secret = !p.echo;
                                let _ = is_secret;
                                // 第一次普通密码提示（Password:，非改密链）→ 自动填
                                // 保存的密码，省去手输；此后的所有提示（含改密链的
                                // Current/New/Retype）与 OpenSSH 一致交给用户输入
                                // （sshconnect2.c::input_userauth_info_req 的
                                // read_passphrase(prompt, echo?RP_ECHO:0) 就是每轮都问）。
                                //
                                // 判据只看「是不是第一次普通密码提示」，不看
                                // expiry_logged：过期场景第一轮的 instructions 就带
                                // 「required to change」，会让 expiry_logged 立即置位，
                                // 若用它做判据则第一轮 Password: 既不自动填也不问用户。
                                if !text_is_change_step && !plain_password_filled {
                                    plain_password_filled = true;
                                    responses.push(password.to_string());
                                    continue;
                                }
                                let answer = self.ask_in_terminal(&p.prompt, p.echo).await;
                                match answer {
                                    Some(a) => {
                                        // 只收集**改密链**的应答（current/new/retype）。
                                        // 不能用 (is_secret || text_is_change_step)：
                                        // 第一轮的普通 Password: 也是 echo=false，
                                        // 收进来会让「尾部两条相等」在「用户两次输
                                        // 同一个密码」时误判为改密成功并写库。
                                        // 改密链的识别靠文本分类（PAM 的改密提示
                                        // 文案有限且稳定：current/new/retype 三族），
                                        // 写库判定仍走协议不变量（尾部两条相等）。
                                        if text_is_change_step && !a.is_empty() {
                                            secret_answers.push(a.clone());
                                        } else if !text_is_change_step && !a.is_empty() {
                                            // 普通密码提示 + 用户手输：每次覆盖记录。
                                            // 取**最后一次**而不是要求「只有一次」——
                                            // 用户可能输错两次第三次才对，而服务器
                                            // 最终 Success 验证的正是最后那一次。
                                            user_entered_password = Some(a.clone());
                                        }
                                        responses.push(a);
                                    }
                                    None => {
                                        // 超时/用户无输入：空应答继续（PAM 拒绝后重试）
                                        log::warn!(
                                            "[SSH {}] kbd-int 终端输入超时/取消",
                                            self.id
                                        );
                                        responses.push(String::new());
                                    }
                                }
                            }
                            kb = session
                                .authenticate_keyboard_interactive_respond(responses)
                                .await?;
                        }
                    }
                }
                if kbdint_worked {
                    // 认证内改密成功（PAM 全链验证通过才返回 Success）。
                    // 新密码取协议驱动收集的序列尾部：PAM 只在新密码两次一致时
                    // 才 SUCCESS，故最后两条相等且非空即可确认（不看提示文本）
                    let verified_new_password: Option<String> = {
                        // 取尾部两条（用 get 避免索引 panic：仓库 deny indexing_slicing）
                        let n = secret_answers.len();
                        if n >= 2 {
                            match (
                                secret_answers.get(n - 2),
                                secret_answers.get(n - 1),
                            ) {
                                (Some(prev), Some(last))
                                    if !last.is_empty() && last == prev =>
                                {
                                    Some(last.clone())
                                }
                                _ => None,
                            }
                        } else {
                            None
                        }
                    };
                    if let Some(new_pw) = &verified_new_password {
                        // 调试期明文日志（含新密码）：默认注释，排查时取消注释
                        // log::info!(
                        //     "[SSH-PASSWD-CHANGE] session={} CONFIRMED: \\
                        //      password changed via kbd-int new={:?} \\
                        //      (verification mode: plaintext log-only)",
                        //     self.id, new_pw
                        // );
                        if let Some(host_id) = self.host_id {
                            match crate::db::global_pool() {
                                Some(pool) => {
                                    match crate::db::update_host_password(
                                        &pool, host_id, new_pw,
                                    ) {
                                        Ok(true) => {
                                            log::info!(
                                                "[SSH-PASSWD-CHANGE] session={} \
                                                 DB-UPDATED: hosts.id={} password saved \
                                                 (automatic, verified via kbd-int)",
                                                self.id, host_id
                                            );
                                            // 比对打印（验证期用，已确认落库正确）
                                            // 默认注释，排查时取消注释即可
                                            // log::info!(
                                            //     "[SSH-PASSWD-CHANGE] session={} \
                                            //      DB-SAVED-PASSWORD={:?}",
                                            //     self.id, new_pw
                                            // );
                                            // 通知前端刷新内存 host store：前端建
                                            // 会话用内存副本，不刷新会继续用旧密码
                                            //（实测：DB 已更新但新会话被拒）
                                            use tauri::Emitter;
                                            if let Some(app) = self.app_handle.as_ref() {
                                                let _ = app.emit(
                                                    "host-password-updated",
                                                    serde_json::json!({
                                                        "hostId": host_id
                                                    }),
                                                );
                                            }
                                        }
                                        Ok(false) => log::info!(
                                            "[SSH-PASSWD-CHANGE] session={} \\
                                             DB-SKIP: hosts.id={} not password-auth \\
                                             or missing (no update)",
                                            self.id, host_id
                                        ),
                                        Err(e) => log::warn!(
                                            "[SSH-PASSWD-CHANGE] session={} \\
                                             DB-UPDATE failed: {}",
                                            self.id, e
                                        ),
                                    }
                                }
                                None => log::warn!(
                                    "[SSH-PASSWD-CHANGE] session={} DB pool \\
                                     unavailable, password not saved",
                                    self.id
                                ),
                            }
                        } else {
                            log::info!(
                                "[SSH-PASSWD-CHANGE] session={} no host_id \\
                                 (temporary connection), password not saved",
                                self.id
                            );
                        }
                    }
                    return Ok(());
                }
                // 回落：password 方法（仅当 kbd-int 从未成功发起过交互——
                // 即服务器不支持 kbd-int）。若 kbd-int 已经用同一个保存密码
                // 试过并失败，再走 password 必然同样失败，只会白等一次服务器
                // PAM 校验（pam_unix 哈希刻意慢，实测 ~3.2s/次），故直接跳过。
                if kbdint_attempts > 0 {
                    log::info!(
                        "[SSH {}] kbd-int 已用保存密码失败过，跳过 password 回落（避免重复等待）",
                        self.id
                    );
                    return Err(ConnectionError::Authentication(
                        "kbd-int 已用保存密码失败过，跳过 password 回落".to_string(),
                    ));
                }
                let response = session
                    .authenticate_password(username.to_string(), password)
                    .await?;
                auth_result = self
                    ._handle_auth_result(
                        session,
                        username.to_string(),
                        Some(password.as_str()),
                        response,
                    )
                    .await
                    .unwrap_or(false);
                if auth_result {
                    log::info!(
                        "[SSH {}] Authenticated with password as {}",
                        self.id,
                        username
                    );
                } else {
                    auth_error_msg =
                        Some("Password authentication was rejected by the SSH target".to_string());
                }
            }

            SSHTargetAuth::Key {
                private_key,
                passphrase,
            } => {
                let best_hash = session.best_supported_rsa_hash().await?.flatten();
                let keys = if private_key.trim().is_empty() {
                    discover_default_local_keys()
                } else {
                    vec![load_client_private_key(private_key, passphrase.as_deref())?]
                };

                if keys.is_empty() {
                    auth_error_msg =
                        Some("No SSH client private key provided or found in ~/.ssh".into());
                }

                for key in keys {
                    let key = Arc::new(key);
                    if key.key_data().is_rsa() && best_hash.is_none() && !allow_insecure_algos {
                        log::info!(
                            "[SSH {}] Skipping ssh-rsa (SHA1) key authentication since insecure SSH algos are disabled",
                            self.id
                        );
                        continue;
                    }
                    let key_str =
                        public_key_line(&key).unwrap_or_else(|_| "<public-key>".to_string());
                    let mut response = session
                        .authenticate_publickey(
                            username.to_string(),
                            PrivateKeyWithHashAlg::new(key.clone(), best_hash),
                        )
                        .await?;

                    auth_result = self
                        ._handle_auth_result(session, username.to_string(), None, response)
                        .await
                        .unwrap_or(false);

                    if !auth_result
                        && key.key_data().is_rsa()
                        && best_hash.is_some()
                        && allow_insecure_algos
                    {
                        response = session
                            .authenticate_publickey(
                                username.to_string(),
                                PrivateKeyWithHashAlg::new(key.clone(), None),
                            )
                            .await?;

                        auth_result = self
                            ._handle_auth_result(session, username.to_string(), None, response)
                            .await
                            .unwrap_or(false);
                    }

                    if auth_result {
                        log::info!(
                            "[SSH {}] Authenticated with public key ({}) as {}",
                            self.id,
                            key_str,
                            username
                        );
                        break;
                    }
                    auth_error_msg =
                        Some("Public key authentication was rejected by the SSH target".into());
                }
            }

            SSHTargetAuth::Agent => {
                let best_hash = session.best_supported_rsa_hash().await?.flatten();

                // 本地 SSH Agent：Unix 走 SSH_AUTH_SOCK 域套接字，Windows 走 OpenSSH 命名管道。
                #[cfg(unix)]
                {
                    if let Ok(sock_path) = std::env::var("SSH_AUTH_SOCK")
                        && !sock_path.trim().is_empty()
                        && let Ok(stream) = tokio::net::UnixStream::connect(&sock_path).await
                    {
                        auth_result = self
                            ._authenticate_with_local_agent(
                                session,
                                russh::keys::agent::client::AgentClient::connect(stream),
                                username,
                                best_hash,
                            )
                            .await;
                    }
                }

                #[cfg(target_os = "windows")]
                {
                    // Windows OpenSSH Agent（ssh-agent 服务）固定监听该命名管道；
                    // 未安装/未启动时 open 失败，静默回退到 ~/.ssh 默认密钥。
                    if !auth_result
                        && let Ok(stream) = tokio::net::windows::named_pipe::ClientOptions::new()
                            .open(r"\\.\pipe\openssh-ssh-agent")
                    {
                        auth_result = self
                            ._authenticate_with_local_agent(
                                session,
                                russh::keys::agent::client::AgentClient::connect(stream),
                                username,
                                best_hash,
                            )
                            .await;
                    }
                }

                // 若本地 ssh-agent 未命中，回退尝试 ~/.ssh 默认密钥
                if !auth_result {
                    for key in discover_default_local_keys() {
                        let key = Arc::new(key);
                        if let Ok(response) = session
                            .authenticate_publickey(
                                username.to_string(),
                                PrivateKeyWithHashAlg::new(key.clone(), best_hash),
                            )
                            .await
                        {
                            auth_result = self
                                ._handle_auth_result(session, username.to_string(), None, response)
                                .await
                                .unwrap_or(false);
                            if auth_result {
                                log::info!(
                                    "[SSH {}] Authenticated via ~/.ssh default key as {}",
                                    self.id,
                                    username
                                );
                                break;
                            }
                        }
                    }
                }

                if !auth_result {
                    auth_error_msg = Some(
            "Local SSH Agent (and ~/.ssh default keys) authentication was rejected by the SSH target"
              .into(),
          );
                }
            }
        }

        if !auth_result {
            let reason = auth_error_msg
                .unwrap_or_else(|| "Authentication was rejected by the SSH target".to_string());
            log::error!(
                "[SSH {}] Could not authenticate with SSH target: {}",
                self.id,
                reason
            );
            let _ = session
                .disconnect(russh::Disconnect::ByApplication, "", "")
                .await;
            return Err(ConnectionError::Authentication(reason));
        }

        Ok(())
    }

    /// 用本地 SSH Agent 中的身份列表逐个尝试公钥认证。
    ///
    /// 泛型 `S` 让同一段逻辑同时服务两种传输：
    /// - Unix：`SSH_AUTH_SOCK` 指向的 domain socket（`tokio::net::UnixStream`）；
    /// - Windows：OpenSSH Agent 的命名管道（`\\.\pipe\openssh-ssh-agent`）。
    ///
    /// 命中即返回 `true`；全部失败返回 `false`，由调用方继续回退其它认证方式。
    #[cfg(any(unix, target_os = "windows"))]
    async fn _authenticate_with_local_agent<S>(
        &self,
        session: &mut Handle<ClientHandler>,
        mut agent: russh::keys::agent::client::AgentClient<S>,
        username: &str,
        best_hash: Option<russh::keys::HashAlg>,
    ) -> bool
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
        let Ok(identities) = agent.request_identities().await else {
            return false;
        };

        for identity in identities {
            let Ok(response) = session
                .authenticate_publickey_with(
                    username.to_string(),
                    identity.public_key().into_owned(),
                    best_hash,
                    &mut agent,
                )
                .await
            else {
                continue;
            };

            if self
                ._handle_auth_result(session, username.to_string(), None, response)
                .await
                .unwrap_or(false)
            {
                log::info!(
                    "[SSH {}] Authenticated via local ssh-agent as {}",
                    self.id,
                    username
                );
                return true;
            }
        }

        false
    }

    /// 处理认证结果与 `keyboard-interactive` 挑战（迁移自 `warpgate-protocol-ssh/src/client/mod.rs::_handle_auth_result`）：
    /// - 若服务端禁用 `password` 方法而改走 PAM `keyboard-interactive`，当存在密码且提示请求输入密码时自动填入密码；
    /// - 其余可选二次空挑战（如 Warpgate 注释中提到的 optional 2FA challenge）填充空字符串。
    async fn _handle_auth_result(
        &self,
        session: &mut Handle<ClientHandler>,
        username: String,
        password_fallback: Option<&str>,
        result: AuthResult,
    ) -> Result<bool> {
        match result {
            AuthResult::Success => Ok(true),
            AuthResult::Failure {
                remaining_methods: methods,
                ..
            } => {
                for method in methods.iter() {
                    if matches!(method, MethodKind::KeyboardInteractive) {
                        log::debug!("[SSH {}] Found keyboard-interactive challenge", self.id);
                        let mut kb_result = session
                            .authenticate_keyboard_interactive_start(username.clone(), None)
                            .await?;

                        let mut used_password = false;
                        // 【验证模式】libssh 式 kbd-int 感知（对齐
                        // ssh_userauth_kbdint_getname/getinstruction/getprompt
                        // 的拉取模型）：每轮 InfoRequest 的 name/instruction/
                        // prompts 全文暴露；含「新密码」特征 = PAM 强制改密
                        //（RFC 4256 多轮：弱密码被拒后服务器重发提示自然循环）。
                        // 仅日志登记，不动数据库密码。
                        let mut round = 0u32;
                        while let KeyboardInteractiveAuthResponse::InfoRequest {
                            name,
                            instructions,
                            ref prompts,
                        } = kb_result
                        {
                            round += 1;
                            let prompt_texts: Vec<&str> =
                                prompts.iter().map(|p| p.prompt.as_str()).collect();
                            let lower_all = prompt_texts
                                .iter()
                                .map(|p| p.to_ascii_lowercase())
                                .collect::<Vec<_>>()
                                .join("\x1e");
                            let looks_change = prompts.len() >= 2
                                && lower_all.contains("new password")
                                || lower_all.contains("new unix password")
                                || lower_all.contains("新密码")
                                || lower_all.contains("新的密码");
                            if looks_change {
                                log::info!(
                                    "[SSH-PASSWD-CHANGE] session={} round={} \
                                     form=keyboard-interactive name={:?} \
                                     instructions={:?} prompts={:?} \
                                     (verification mode: log-only)",
                                    self.id, round, name, instructions, prompt_texts
                                );
                            } else {
                                log::info!(
                                    "[SSH {}] kbd-int round {}: name={:?} \
                                     instructions={:?} prompts={:?}",
                                    self.id, round, name, instructions, prompt_texts
                                );
                            }
                            let responses: Vec<String> = prompts
                                .iter()
                                .map(|p| {
                                    if !used_password
                                        && let Some(pw) = password_fallback
                                        && (!p.echo
                                            || p.prompt.to_ascii_lowercase().contains("password"))
                                    {
                                        used_password = true;
                                        pw.to_string()
                                    } else {
                                        String::new()
                                    }
                                })
                                .collect();

                            kb_result = session
                                .authenticate_keyboard_interactive_respond(responses)
                                .await?;
                        }

                        match kb_result {
                            KeyboardInteractiveAuthResponse::Success => {
                                log::debug!(
                                    "[SSH {}] keyboard-interactive challenge successful",
                                    self.id
                                );
                                return Ok(true);
                            }
                            KeyboardInteractiveAuthResponse::Failure { .. } => {
                                log::debug!(
                                    "[SSH {}] keyboard-interactive challenge failed",
                                    self.id
                                );
                                return Ok(false);
                            }
                            KeyboardInteractiveAuthResponse::InfoRequest { .. } => {}
                        }
                    }
                }
                Ok(false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_chain_ids;
    use std::collections::HashMap;

    #[test]
    fn resolve_chain_ids_returns_ordered_chain() {
        let (a, b, c) = (1usize, 2usize, 3usize);
        let jumps: HashMap<usize, Option<usize>> =
            HashMap::from([(a, Some(b)), (b, Some(c)), (c, None)]);
        let ids = resolve_chain_ids(a, |id| jumps.get(&id).copied()).unwrap();
        assert_eq!(ids, vec![a, b, c]);
    }

    #[test]
    fn resolve_chain_ids_detects_cycle() {
        let (a, b) = (1usize, 2usize);
        let jumps: HashMap<usize, Option<usize>> = HashMap::from([(a, Some(b)), (b, Some(a))]);
        assert!(resolve_chain_ids(a, |id| jumps.get(&id).copied()).is_err());
    }

    #[test]
    fn resolve_chain_ids_rejects_unresolvable_jump_host() {
        let (a, b) = (1usize, 2usize);
        let jumps: HashMap<usize, Option<usize>> = HashMap::from([(a, Some(b))]);
        assert!(resolve_chain_ids(a, |id| jumps.get(&id).copied()).is_err());
    }
}
