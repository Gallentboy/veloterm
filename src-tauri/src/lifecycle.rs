//! SSH / SFTP 连接生命周期事件钩子：
//! 在关键节点（连接中 / 认证通过 / Shell 就绪 / 失败 / 断开）向前端发射 Tauri 事件，
//! 前端经 `src/services/sshLifecycle.ts` 统一订阅（listen + 按 kind 过滤），
//! 供状态指示、通知、自动重连等消费——不侵入现有业务逻辑，纯增量发射点。

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// 生命周期事件种类
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LifecycleKind {
    Connecting,
    Connected,
    ShellReady,
    Failed,
    Disconnected,
}

/// 断开原因分类（前端可据此区分 UI 表现：网络闪断→可重连、服务端关闭→提示）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DisconnectReason {
    Network,
    Server,
    User,
    Timeout,
}

/// 生命周期事件负载（SSH 与 SFTP 共用同一结构）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleEvent {
    pub kind: LifecycleKind,
    /// 前端会话 id（终端为 descriptor.id，SFTP 为后端连接序号）
    pub session_id: usize,
    pub host_id: Option<usize>,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    /// 仅 kind = failed 时有值
    pub error: Option<String>,
    /// 仅 kind = disconnected 时有值
    pub reason: Option<DisconnectReason>,
    /// Unix 毫秒时间戳
    pub timestamp: u64,
}

/// 生命周期事件发射器 trait（AppHandle 实现真发射，测试可用 noop）
pub trait LifecycleEmitter: Send + Sync + 'static {
    fn emit_lifecycle(&self, prefix: &str, event: &LifecycleEvent);
}

impl LifecycleEmitter for AppHandle {
    fn emit_lifecycle(&self, prefix: &str, event: &LifecycleEvent) {
        let event_name = format!(
            "{prefix}://{}",
            match event.kind {
                LifecycleKind::Connecting => "connecting",
                LifecycleKind::Connected => "connected",
                LifecycleKind::ShellReady => "shell-ready",
                LifecycleKind::Failed => "failed",
                LifecycleKind::Disconnected => "disconnected",
            }
        );
        if let Err(e) = self.emit(&event_name, event) {
            log::warn!("[Lifecycle] emit {} failed: {e}", event_name);
        }
    }
}

/// 便捷构造 + 发射（避免在每个调用点手写完整结构体）
pub fn emit_ssh_lifecycle(
    emitter: &dyn LifecycleEmitter,
    kind: LifecycleKind,
    session_id: usize,
    host_id: Option<usize>,
    host: &str,
    port: u16,
    username: Option<&str>,
    error: Option<&str>,
    reason: Option<DisconnectReason>,
) {
    emitter.emit_lifecycle(
        "ssh",
        &LifecycleEvent {
            kind,
            session_id,
            host_id,
            host: host.to_string(),
            port,
            username: username.map(str::to_string),
            error: error.map(str::to_string),
            reason,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        },
    );
}

pub fn emit_sftp_lifecycle(
    emitter: &dyn LifecycleEmitter,
    kind: LifecycleKind,
    session_id: usize,
    host_id: Option<usize>,
    host: &str,
    port: u16,
    error: Option<&str>,
) {
    emitter.emit_lifecycle(
        "sftp",
        &LifecycleEvent {
            kind,
            session_id,
            host_id,
            host: host.to_string(),
            port,
            username: None,
            error: error.map(str::to_string),
            reason: None,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        },
    );
}
