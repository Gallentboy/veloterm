//! trzsz 终端内传输（tsz 下载 / trz 上传），与 ZMODEM（`terminal/zmodem/`）
//! 并列的第二条终端内文件传输通道。
//!
//! 架构（数据全程在 Rust，不经前端）：
//! - `detector`：终端输出流入口探测（`::TRZSZ:TRANSFER:` 触发串状态机 +
//!   传输结束后的残尾过滤）
//! - `codec`：帧编解码（base64+zlib 文本模式载荷）、行读取状态机（tmux
//!   折行/噪声容错、Windows 服务端协议）、MD5
//! - `engine`：传输桥 + 帧引擎 + 下载/上传编排（对话框、落盘、进度事件）
//!
//! 接入点：`terminal/ssh/mod.rs` 的 `RCEvent::Output` 消费处——命中探测后
//! 该会话进入传输模式，后续字节喂桥接引擎、不再转发前端 xterm；结束/取消
//! 后恢复透传（残尾经 `TrszszTailFilter` 过滤）。
//!
//! 协议参照 trzsz-go（MIT）与 trzsz Python 客户端（MIT）的客户端行为，
//! Rust 自实现；MVP 为文本模式（ACT 声明 `binary:false`、`protocol:1`），
//! 二进制模式与隧道/fork/tmux-cc 集成未实现。

pub mod codec;
pub mod detector;
pub mod engine;

pub use detector::{FeedOutcome, TrszszDetector, TrszszTailFilter, TrszszTrigger};
pub use engine::{TRZSZ_EVENT, TrzszBridge, TrzszProgress, start_transfer};
