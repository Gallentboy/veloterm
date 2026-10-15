pub mod channel_direct_tcpip;
pub mod channel_session;
pub mod client;
pub mod common;
pub mod error;
pub mod handler;
pub mod keys;
pub mod known_hosts;

use crate::lifecycle::{DisconnectReason, LifecycleEmitter, LifecycleKind, emit_ssh_lifecycle};

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
pub use client::*;
pub use common::*;
pub use error::*;
pub use keys::*;
pub use known_hosts::*;
use russh::keys::PublicKeyBase64;
use tauri::ipc::Channel;
use tokio::sync::mpsc::UnboundedSender;

use super::session::{TerminalBackendKind, TerminalBackendSession, TerminalCreateResult};

/// 基于 `warpgate-protocol-ssh` (`RemoteClient` + `SessionChannel`) 与 `warpgate-web-ssh` (`WebSshSession`)
/// 架构迁移的 VeloTerm SSH 终端会话实现，完整实现 [`TerminalBackendSession`]。
pub struct SshTerminalSession {
    id: usize,
    shell_channel_id: usize,
    command_tx: UnboundedSender<(RCCommand, Option<RCCommandReply>)>,
    abort_tx: UnboundedSender<()>,
    closed_by_client: Arc<AtomicBool>,
}

impl SshTerminalSession {
    pub fn spawn<F>(
        id: usize,
        cols: u16,
        rows: u16,
        chain: Vec<TargetSSHOptions>,
        on_output: Channel<Vec<u8>>,
        on_exit: Channel<i32>,
        on_session_closed: F,
        lifecycle: Option<std::sync::Arc<dyn LifecycleEmitter>>,
        lifecycle_host_id: Option<usize>,
    ) -> Result<(Arc<Self>, TerminalCreateResult), String>
    where
        F: FnOnce(usize) + Send + 'static,
    {
        let target = chain
            .last()
            .cloned()
            .ok_or_else(|| "SSH connection chain must not be empty".to_string())?;

        let shell_channel_id: usize = 1;
        let known_hosts = KnownHosts::global();

        let RemoteClientHandles {
            mut event_rx,
            command_tx,
            abort_tx,
        } = RemoteClient::create(id, known_hosts)
            .map_err(|e| format!("failed to create SSH RemoteClient: {e}"))?;

        // 1. 发起连接（支持单跳直连或多跳 Jump Host 链式隧道）
        let chain_for_verification = chain.clone();
        let _ = command_tx.send((RCCommand::Connect(chain), None));

        // 2. 预入队打开 Shell Channel、请求 PTY 与 Shell（对齐 `warpgate-web-ssh::WebSshSession::open_shell_channel`）：
        //    `RemoteClient` 会在 `state != RCState::Connected` 期间将其暂存在 `pending_ops`，待认证通过后按序自动执行。
        let pty_request = make_pty_request(u32::from(cols), u32::from(rows));
        let _ = command_tx.send((
            RCCommand::Channel(shell_channel_id, ChannelOperation::OpenShell),
            None,
        ));
        let _ = command_tx.send((
            RCCommand::Channel(shell_channel_id, ChannelOperation::RequestPty(pty_request)),
            None,
        ));
        let _ = command_tx.send((
            RCCommand::Channel(shell_channel_id, ChannelOperation::RequestShell),
            None,
        ));

        let closed_by_client = Arc::new(AtomicBool::new(false));
        let session = Arc::new(Self {
            id,
            shell_channel_id,
            command_tx: command_tx.clone(),
            abort_tx,
            closed_by_client: Arc::clone(&closed_by_client),
        });

        // 3. 启动事件中继协程（对齐 `warpgate-web-ssh/src/manager.rs::relay_events`）
        let relay_closed_flag = Arc::clone(&closed_by_client);
        let relay_command_tx = command_tx;
        let default_verification_mode = target.host_key_verification;
        let display_target = format!("{}@{}:{}", target.username, target.host, target.port);
        let lifecycle_target_host = target.host.clone();
        let lifecycle_target_port = target.port;
        let lifecycle_target_user = target.username.clone();
        // host_id 由 manager 层解析后传入（TargetSSHOptions 自身无此字段）
        let lifecycle_host_id = lifecycle_host_id;

        tokio::spawn(async move {
            let mut exit_code: i32 = 0;
            // shell-ready 只发一次：首次收到 Output 时标记
            let mut shell_ready_emitted = false;
            // 生命周期便捷发射（emitter 为 None 时静默跳过）
            let emit =
                |kind: LifecycleKind, error: Option<&str>, reason: Option<DisconnectReason>| {
                    if let Some(ref emitter) = lifecycle {
                        emit_ssh_lifecycle(
                            emitter.as_ref(),
                            kind,
                            id,
                            lifecycle_host_id,
                            &lifecycle_target_host,
                            lifecycle_target_port,
                            Some(&lifecycle_target_user),
                            error,
                            reason,
                        );
                    }
                };

            while let Some(event) = event_rx.recv().await {
                match event {
                    RCEvent::State(RCState::Connecting) => {
                        emit(LifecycleKind::Connecting, None, None);
                        let msg = format!(
                            "\x1b[90m[SSH] Connecting to {}...\x1b[0m\r\n",
                            display_target
                        );
                        let _ = on_output.send(msg.into_bytes());
                    }
                    RCEvent::HopConnected => {
                        let msg =
                            b"\x1b[90m[SSH] Jump host connected, opening tunnel...\x1b[0m\r\n"
                                .to_vec();
                        let _ = on_output.send(msg);
                    }
                    RCEvent::State(RCState::Connected) => {
                        emit(LifecycleKind::Connected, None, None);
                        log::info!(
                            "[SshTerminalSession {}] connected to {}",
                            id,
                            display_target
                        );
                    }
                    RCEvent::Output(channel_id, data) if channel_id == shell_channel_id => {
                        if !shell_ready_emitted {
                            shell_ready_emitted = true;
                            emit(LifecycleKind::ShellReady, None, None);
                        }
                        if on_output.send(data.to_vec()).is_err() {
                            break;
                        }
                    }
                    RCEvent::ExtendedData { channel, data, .. } if channel == shell_channel_id => {
                        if on_output.send(data.to_vec()).is_err() {
                            break;
                        }
                    }
                    RCEvent::ExitStatus(channel_id, code) if channel_id == shell_channel_id => {
                        exit_code = code as i32;
                    }
                    RCEvent::Close(channel_id) | RCEvent::ChannelFailure(channel_id)
                        if channel_id == shell_channel_id =>
                    {
                        let _ = relay_command_tx.send((RCCommand::Disconnect, None));
                    }
                    RCEvent::HostKeyReceived(key, host, port) => {
                        log::debug!(
                            "[SshTerminalSession {}] Host key received for {}:{}: {}",
                            id,
                            host,
                            port,
                            key.algorithm()
                        );
                    }
                    RCEvent::HostKeyUnknown(key, host, port, reply) => {
                        let verification_mode = chain_for_verification
                            .iter()
                            .find(|hop| hop.host == host && hop.port == port)
                            .map(|hop| hop.host_key_verification)
                            .unwrap_or(default_verification_mode);

                        match verification_mode {
                            SshHostKeyVerificationMode::Ignore
                            | SshHostKeyVerificationMode::AutoAccept
                            | SshHostKeyVerificationMode::Prompt => {
                                log::info!(
                                    "[SshTerminalSession {}] Trusting host key for {}:{} ({} {})",
                                    id,
                                    host,
                                    port,
                                    key.algorithm(),
                                    key.public_key_base64()
                                );
                                let msg = format!(
                                    "\x1b[90m[SSH] Trusted new host key for {}:{} ({})\x1b[0m\r\n",
                                    host,
                                    port,
                                    key.algorithm()
                                );
                                let _ = on_output.send(msg.into_bytes());
                                let _ = reply.send(true);
                            }
                            SshHostKeyVerificationMode::AutoReject => {
                                log::warn!(
                                    "[SshTerminalSession {}] Unknown host key rejected for {}:{}",
                                    id,
                                    host,
                                    port
                                );
                                let msg = format!(
                                    "\r\n\x1b[1;31m[SSH Host Key Rejected]\x1b[0m Unknown host key for {}:{} ({}) rejected by strict verification policy.\r\n",
                                    host,
                                    port,
                                    key.algorithm()
                                );
                                let _ = on_output.send(msg.into_bytes());
                                let _ = reply.send(false);
                            }
                        }
                    }
                    RCEvent::ConnectionError(err) => {
                        exit_code = 1;
                        emit(LifecycleKind::Failed, Some(&err.to_string()), None);
                        let msg = format!("\r\n\x1b[1;31m[SSH Connection Error]\x1b[0m {err}\r\n");
                        let _ = on_output.send(msg.into_bytes());
                    }
                    RCEvent::Error(err) => {
                        exit_code = 1;
                        emit(LifecycleKind::Failed, Some(&err.to_string()), None);
                        let msg = format!("\r\n\x1b[1;31m[SSH Error]\x1b[0m {err}\r\n");
                        let _ = on_output.send(msg.into_bytes());
                    }
                    RCEvent::Done => break,
                    _ => {}
                }
            }

            on_session_closed(id);
            // 断开事件：用户主动关闭 → user；其他（网络中断、服务端关闭）→ network
            let reason = if relay_closed_flag.load(Ordering::SeqCst) {
                DisconnectReason::User
            } else {
                DisconnectReason::Network
            };
            emit(LifecycleKind::Disconnected, None, Some(reason));
            if !relay_closed_flag.load(Ordering::SeqCst) {
                let _ = on_exit.send(exit_code);
            }
        });

        let shell_label = format!("ssh {}@{}", target.username, target.host);
        let result = TerminalCreateResult {
            id,
            kind: TerminalBackendKind::Ssh,
            shell: shell_label,
            cwd: None,
        };

        Ok((session, result))
    }
}

impl TerminalBackendSession for SshTerminalSession {
    fn id(&self) -> usize {
        self.id
    }

    fn kind(&self) -> TerminalBackendKind {
        TerminalBackendKind::Ssh
    }

    fn write(&self, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        self.command_tx
            .send((
                RCCommand::Channel(
                    self.shell_channel_id,
                    ChannelOperation::Data(Bytes::copy_from_slice(data)),
                ),
                None,
            ))
            .map_err(|_| format!("SSH session {} command channel closed", self.id))
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let pty_request = make_pty_request(u32::from(cols), u32::from(rows));
        self.command_tx
            .send((
                RCCommand::Channel(
                    self.shell_channel_id,
                    ChannelOperation::ResizePty(pty_request),
                ),
                None,
            ))
            .map_err(|_| format!("SSH session {} command channel closed", self.id))
    }

    fn close(&self) -> Result<(), String> {
        self.closed_by_client.store(true, Ordering::SeqCst);
        let _ = self.command_tx.send((
            RCCommand::Channel(self.shell_channel_id, ChannelOperation::Close),
            None,
        ));
        let _ = self.command_tx.send((RCCommand::Disconnect, None));
        let _ = self.abort_tx.send(());
        Ok(())
    }
}
