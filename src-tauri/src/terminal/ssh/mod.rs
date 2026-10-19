pub mod channel_direct_tcpip;
pub mod channel_session;
pub mod client;
pub mod common;
pub mod error;
pub mod handler;
pub mod keys;
pub mod known_hosts;
pub mod relay_transport;

use crate::lifecycle::{DisconnectReason, LifecycleEmitter, LifecycleKind, emit_ssh_lifecycle};

use std::future::Future;
use std::pin::Pin;
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
use tokio::sync::oneshot;

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
    #[allow(clippy::too_many_arguments)]
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
        app_handle: Option<tauri::AppHandle>,
        zmodem_aggregator: Option<std::sync::Arc<crate::terminal::zmodem::ZmodemAggregator>>,
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
        } = RemoteClient::create(
            id,
            known_hosts,
            app_handle.clone(),
            lifecycle_host_id,
        )
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

        tokio::spawn(async move {
            let mut exit_code: i32 = 0;
            // shell-ready 只发一次：首次收到 Output 时标记
            let mut shell_ready_emitted = false;
            // ── ZMODEM 传输态（探测命中后接管输出流，不再转发前端）──
            let mut zmodem_detector = crate::terminal::zmodem::ZmodemDetector::new();
            // 上次上传结束时刻：lrz 退出时可能重发 ZRINIT（合法入口帧），
            // 2s 冷却窗口内忽略之——正常用户不可能 2s 内完成「敲 rz→服务器
            // 回 ZRINIT」的全流程。下载（ZRQINIT）不受此限制
            let mut last_upload_end: Option<std::time::Instant> = None;
            let mut zmodem_active: Option<crate::terminal::zmodem::ZmodemBridge> = None;
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

            // 传输期定时检查 finished：取消/失败后即使服务器不再输出，
            // 也能及时清理 zmodem_active 恢复透传（否则终端永久卡死）
            let mut zmodem_tick = tokio::time::interval(std::time::Duration::from_millis(200));
            zmodem_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                let event = tokio::select! {
                    maybe = event_rx.recv() => match maybe {
                        Some(ev) => ev,
                        None => break,
                    },
                    _ = zmodem_tick.tick() => {
                        if let Some(bridge) = zmodem_active.as_ref()
                            && bridge.finished_flag().load(std::sync::atomic::Ordering::Relaxed)
                        {
                            if bridge.direction == "upload" {
                                last_upload_end = Some(std::time::Instant::now());
                            }
                            zmodem_active = None;
                            zmodem_detector.reset();
                            log::debug!("[ZMODEM] session {id} transfer ended, passthrough restored");
                        }
                        continue;
                    }
                };
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
                        // 传输中：字节全部喂引擎，不再转发前端。
                        // finished（含取消/失败）→ 立即清理并**把本块字节
                        // 重新走探测/透传**（不能丢弃：其中可能已含下一轮
                        // 传输的帧；旧实现丢弃后若服务器静默，zmodem_active
                        // 永不清理 → 后续所有输出被吞 → 终端卡死）
                        if let Some(bridge) = zmodem_active.as_ref() {
                            if bridge.finished_flag().load(std::sync::atomic::Ordering::Relaxed) {
                                zmodem_active = None;
                                zmodem_detector.reset();
                            } else if bridge.feed(&data) {
                                continue;
                            } else {
                                // 引擎已结束：恢复透传。若刚结束的是上传，
                                // 记录时刻供 ZRINIT 冷却判定
                                if bridge.direction == "upload" {
                                    last_upload_end = Some(std::time::Instant::now());
                                }
                                zmodem_active = None;
                                zmodem_detector.reset();
                            }
                        }
                        // 改密提示词扫描（旁路：不影响透传字节）
                        crate::terminal::passwd_input::PasswdInputLog::global()
                            .feed_output(id, &data);
                        // 未命中：探测（命中则建桥、启动传输任务）
                        let detection = zmodem_detector.feed(&data);
                        if matches!(detection, crate::terminal::zmodem::Detection::Swallow) {
                            // 收尾/进行中帧：吞掉（上一次会话的 ZFIN 等）
                            continue;
                        }
                        if matches!(detection, crate::terminal::zmodem::Detection::None) {
                            log::trace!(
                                "[ZMODEM] session {} no detection, chunk={} bytes",
                                id, data.len()
                            );
                            if on_output.send(data.to_vec()).is_err() {
                                break;
                            }
                        } else if let (Some(app), Some(aggregator)) = (app_handle.clone(), zmodem_aggregator.clone()) {
                            let direction = match detection {
                                crate::terminal::zmodem::Detection::Sz { .. } => "download",
                                _ => "upload",
                            };
                            // 上传冷却：刚结束上传后 2s 内的 ZRINIT 视为 lrz
                            // 退出的重发帧，吞掉（用户来不及发起下一轮）
                            if direction == "upload"
                                && let Some(at) = last_upload_end
                                && std::time::Instant::now().duration_since(at)
                                    < std::time::Duration::from_secs(2)
                            {
                                // 上传冷却：刚结束上传后 2s 内的 ZRINIT 视为 lrz
                                // 退出的重发帧，吞掉（用户来不及发起下一轮）
                                log::debug!("[ZMODEM] session {id} ZRINIT in upload cooldown, swallowed");
                                continue;
                            }
                            if direction == "upload" {
                                last_upload_end = None; // 正常触发时清除
                            }
                            let sid = format!("terminal-{id}");
                            let (bridge, inbound_rx) =
                                crate::terminal::zmodem::ZmodemBridge::new(sid.clone(), direction);
                            // 引擎写回通道 → SSH 通道（写操作走 command 队列）
                            let (out_tx, mut out_rx) =
                                tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
                            let cmd_tx = relay_command_tx.clone();
                            let chan = shell_channel_id;
                            tokio::spawn(async move {
                                while let Some(bytes) = out_rx.recv().await {
                                    let _ = cmd_tx.send((
                                        RCCommand::Channel(
                                            chan,
                                            ChannelOperation::Data(bytes.into()),
                                        ),
                                        None,
                                    ));
                                }
                            });
                            // 下载（sz）方向：立即回 ZRINIT 突发，稳住服务器
                            // sz——它等不到应答会超时放弃（用户还在选保存目录
                            // 时就会退出，导致后续引擎握手无人应答而卡死）。
                            // 上传（rz）方向由引擎主动发起，无需提前应答。
                            if direction == "download" {
                                let mut enc = rzsz::zmodem::frame::FrameEncoder::new();
                                // flags[3] = CANFDX|CANOVIO|CANFC32（与引擎
                                // build_zrinit_flags 一致；buflen=0 = 流式发送）
                                let flags = [
                                    0u8,
                                    0u8,
                                    0u8,
                                    rzsz::zmodem::frame::CANFDX
                                        | rzsz::zmodem::frame::CANOVIO
                                        | rzsz::zmodem::frame::CANFC32,
                                ];
                                let mut burst = Vec::new();
                                for _ in 0..5 {
                                    let _ = enc.send_hex_header(
                                        rzsz::zmodem::frame::FrameType::ZrInit,
                                        &flags,
                                        &mut burst,
                                    );
                                }
                                if !burst.is_empty() {
                                    let _ = out_tx.send(burst);
                                }
                            }
                            // 命中前缀（含本段）先入引擎
                            let _ = bridge.feed(&data);
                            // 登记到聚合器：联动组内凑齐成员后统一弹一次对话框；
                            // 未联动则立即返回单台批
                            let hit = crate::terminal::zmodem::PendingHit {
                                inbound_rx,
                                bridge: bridge.clone(),
                                outbound_tx: out_tx,
                            };
                            log::info!(
                                "[ZMODEM] session {} detected {} ({} bytes chunk, head={:?})",
                                id,
                                direction,
                                data.len(),
                                String::from_utf8_lossy(&data[..data.len().min(16)])
                            );
                            if let Some(batch) = aggregator.register(id, direction, hit) {
                                tokio::spawn(crate::terminal::zmodem::start_batch(
                                    app, batch.direction, batch.hits,
                                ));
                            } else {
                                // 已并入等待中的批：起兜底定时器（凑齐会提前触发）
                                let agg = aggregator.clone();
                                let app_timer = app.clone();
                                tokio::spawn(async move {
                                    tokio::time::sleep(
                                        crate::terminal::zmodem::aggregator::AGGREGATE_WINDOW,
                                    )
                                    .await;
                                    if let Some(group) = agg.group_of(id)
                                        && let Some(batch) = agg.take_batch(&group)
                                    {
                                        crate::terminal::zmodem::start_batch(
                                            app_timer, batch.direction, batch.hits,
                                        )
                                        .await;
                                    }
                                });
                            }
                            zmodem_detector.reset();
                            zmodem_active = Some(bridge);
                        } else {
                            // 无 app handle（测试/无头）→ 透传降级
                            if on_output.send(data.to_vec()).is_err() {
                                break;
                            }
                            zmodem_detector.reset();
                        }
                    }
                    RCEvent::ExtendedData { channel, data, .. } if channel == shell_channel_id => {
                        // stderr 同样喂提示词扫描：passwd 的完成消息
                        // "password updated successfully" 实测走 stderr
                        crate::terminal::passwd_input::PasswdInputLog::global()
                            .feed_output(id, &data);
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

            // 会话结束（关闭/断线）时若仍在 ZMODEM 传输中：给服务器发中止序列，
            // 否则远端 lrz/lsz 会挂在接收态（下次登录可见残留提示）
            if let Some(bridge) = zmodem_active.take() {
                let mut out = Vec::new();
                let mut enc = rzsz::zmodem::frame::FrameEncoder::new();
                let _ = enc.send_cancel(&mut out);
                if !out.is_empty() {
                    let _ = relay_command_tx.send((
                        RCCommand::Channel(
                            shell_channel_id,
                            ChannelOperation::Data(out.into()),
                        ),
                        None,
                    ));
                }
                bridge.cancel();
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

    /// 旁路一次性命令执行（资源监控采集）：经 RemoteClient 的 ExecCapture
    /// 开独立 exec 通道回捕输出；未连接/连接断开时 RemoteClient 立即回 Err
    fn exec_capture(
        &self,
        command: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let _ = self.command_tx.send((
            RCCommand::ExecCapture {
                command,
                reply: reply_tx,
            },
            None,
        ));
        Box::pin(async move {
            match reply_rx.await {
                Ok(result) => result,
                Err(_) => Err("SSH 会话已关闭".to_string()),
            }
        })
    }
}
