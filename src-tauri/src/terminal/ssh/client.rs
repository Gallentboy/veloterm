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
        tx,
        known_hosts,
    };
    connector.connect_chain(chain).await
}

struct Connector {
    id: usize,
    tx: Sender<RCEvent>,
    known_hosts: KnownHosts,
}

/// 远程 SSH 客户端状态机（迁移自 `warpgate-protocol-ssh/src/client/mod.rs::RemoteClient`）：
/// - 支持在连接尚未完成时预入队 `ChannelOperation`（如 `OpenShell` / `RequestPty` / `RequestShell` / `ResizePty`）；
/// - 支持单跳直连与多跳 Jump Host 链式隧道（`channel_open_direct_tcpip` + `connect_stream`）；
/// - 支持密码、私钥（PEM/OpenSSH/文件路径 + Passphrase）、本地 `ssh-agent` 与 `keyboard-interactive` 认证。
pub struct RemoteClient {
    id: usize,
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
    pub fn create(id: usize, known_hosts: KnownHosts) -> io::Result<RemoteClientHandles> {
        let (event_tx, event_rx) = channel(1024);
        let (command_tx, mut command_rx) = unbounded_channel();
        let (abort_tx, abort_rx) = unbounded_channel();
        let (inner_event_tx, inner_event_rx) = unbounded_channel();

        let this = Self {
            id,
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

        let (event_tx, event_rx) = unbounded_channel();
        let handler = ClientHandler {
            ssh_options: first.clone(),
            event_tx,
            known_hosts: self.known_hosts.clone(),
            session_id: self.id,
        };
        let fut = russh::client::connect(config, address, handler);
        let (mut session, mut active_rx) = self
            .wait_for_connection(&first, fut, event_rx, false)
            .boxed()
            .await?;

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
                        while let KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } =
                            kb_result
                        {
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
