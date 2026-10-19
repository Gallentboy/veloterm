//! ZMODEM 终端内传输（sz 下载 / rz 上传）。
//!
//! 架构（数据全程在 Rust，不经前端）：
//! - `detector`：终端输出流入口探测（sz 协议帧 / rz 行首公告）
//! - `bridge`：rzsz 引擎（阻塞）与异步 SSH 流之间的 mpsc 桥
//! - `transfer`：下载/上传编排（对话框、落盘、进度事件、取消）
//!
//! 接入点：`terminal/ssh/mod.rs` 的 `RCEvent::Output` 消费处——命中探测后
//! 该会话进入传输模式，后续字节喂桥接引擎、不再转发前端 xterm；结束/取消
//! 后恢复透传。

pub mod aggregator;
pub mod bridge;
pub mod detector;
pub mod transfer;

#[cfg(test)]
mod replay_tests;

pub use bridge::{ZmodemBridge, ZmodemProgress};
pub use detector::{Detection, TailFrameFilter, ZmodemDetector};
pub use aggregator::{PendingHit, ZmodemAggregator};
pub use transfer::{start_batch, start_download_batch, start_upload_batch, ZMODEM_EVENT};

/// 活跃传输注册表：session_id → (取消函数, 中止帧发送函数)。
/// 前端「取消」按钮只知道 session id，经此找到对应桥。
static ACTIVE: std::sync::Mutex<Option<std::collections::HashMap<usize, ActiveTransfer>>> =
    std::sync::Mutex::new(None);

pub struct ActiveTransfer {
    pub cancel: Box<dyn Fn() + Send + Sync>,
}

/// 注册活跃传输（引擎启动时调用；结束/取消时移除）
pub fn register_active(session_id: usize, cancel: Box<dyn Fn() + Send + Sync>) {
    if let Ok(mut guard) = ACTIVE.lock() {
        guard
            .get_or_insert_with(std::collections::HashMap::new)
            .insert(session_id, ActiveTransfer { cancel });
    }
}

/// 移除活跃传输
pub fn unregister_active(session_id: usize) {
    if let Ok(mut guard) = ACTIVE.lock()
        && let Some(map) = guard.as_mut()
    {
        map.remove(&session_id);
    }
}

/// 请求取消（前端按钮 / 会话关闭）：找不到则静默（传输已结束）
pub fn request_cancel(session_id: usize) {
    if let Ok(guard) = ACTIVE.lock()
        && let Some(map) = guard.as_ref()
        && let Some(entry) = map.get(&session_id)
    {
        (entry.cancel)();
    }
}
