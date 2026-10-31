//! GUI 事件化的日志与进度层（mvn-dr 原生 TUI log.rs 的等价替换）。
//!
//! 原版面向终端（Spring Boot 式着色 + 底部状态栏）；本模块保持**完全相同的
//! 公开 API**，实现替换为：
//! - `DL_*` 原子计数器原样保留（全局单运行语义同 CLI），命令层在每次运行
//!   开始时 `reset_counters()`，并由 250ms ticker 轮询 `snapshot()` 发 Stats 事件；
//! - `info/warn/error/verbose` 与 `downloading_line/downloaded_line/uploading_line/
//!   uploaded_line` 等行打印函数改为向**全局事件 sink** 回调（Tauri 命令层安装
//!   sink 转发 Channel；未安装时静默丢弃）；
//! - TUI 专属函数（状态栏区域/光标等）为 no-op 存根——调用点零改动；
//! - `fmt_bytes/fmt_duration` 纯格式化函数照抄原版。

use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

// ---------- 进度计数器（状态栏数据源；CLI 原版同款语义） ----------

pub static DL_DONE: AtomicUsize = AtomicUsize::new(0);
pub static DL_TOTAL: AtomicUsize = AtomicUsize::new(0);
pub static DL_FAILED: AtomicUsize = AtomicUsize::new(0);
pub static DL_ACTIVE: AtomicUsize = AtomicUsize::new(0);
pub static DL_QUEUED: AtomicUsize = AtomicUsize::new(0);
pub static DL_CONCURRENCY: AtomicUsize = AtomicUsize::new(0);
pub static DL_BYTES: AtomicU64 = AtomicU64::new(0);

/// 每次运行前清零（GUI 下同进程可能连续多次运行，CLI 是一次性进程天然归零）。
pub fn reset_counters() {
    DL_DONE.store(0, Ordering::Relaxed);
    DL_TOTAL.store(0, Ordering::Relaxed);
    DL_FAILED.store(0, Ordering::Relaxed);
    DL_ACTIVE.store(0, Ordering::Relaxed);
    DL_QUEUED.store(0, Ordering::Relaxed);
    DL_CONCURRENCY.store(0, Ordering::Relaxed);
    DL_BYTES.store(0, Ordering::Relaxed);
}

/// 计数器快照（ticker 轮询用）。
#[derive(Clone, Copy, Debug, Default)]
pub struct StatsSnapshot {
    pub done: usize,
    pub total: usize,
    pub failed: usize,
    pub active: usize,
    pub queued: usize,
    pub bytes: u64,
}

pub fn snapshot() -> StatsSnapshot {
    StatsSnapshot {
        done: DL_DONE.load(Ordering::Relaxed),
        total: DL_TOTAL.load(Ordering::Relaxed),
        failed: DL_FAILED.load(Ordering::Relaxed),
        active: DL_ACTIVE.load(Ordering::Relaxed),
        queued: DL_QUEUED.load(Ordering::Relaxed),
        bytes: DL_BYTES.load(Ordering::Relaxed),
    }
}

// ---------- 日志级别 ----------

static LEVEL: AtomicU8 = AtomicU8::new(1); // 0 quiet / 1 info / 2 verbose

pub fn set_level(level: u8) {
    LEVEL.store(level, Ordering::Relaxed);
}

pub fn level() -> u8 {
    LEVEL.load(Ordering::Relaxed)
}

// ---------- 事件 sink（命令层安装；未安装时全部静默） ----------

/// 面向 UI 的事件（log 层产出；命令层转 serde DTO 后经 Channel 发前端）。
#[derive(Clone, Debug)]
pub enum UiEvent {
    /// 分级日志行
    Log { level: u8, message: String },
    /// 开始下载某文件（rel = 仓库相对路径）
    DownloadStart { rel: String },
    /// 文件下载完成（size/speed/took 为已格式化文本）
    DownloadDone { rel: String, size: String, speed: String, took: String },
    /// 开始上传某文件
    UploadStart { rel: String },
    /// 上传完成
    UploadDone { rel: String, size: String, took: String },
}

type Sink = Box<dyn Fn(UiEvent) + Send + Sync>;

static SINK: OnceLock<Mutex<Option<Sink>>> = OnceLock::new();

fn sink_cell() -> &'static Mutex<Option<Sink>> {
    SINK.get_or_init(|| Mutex::new(None))
}

/// 安装事件 sink（命令层在运行开始时装、结束时卸）。
pub fn set_sink(f: Option<Sink>) {
    // 中毒恢复：锁被 panic 污染时取出内部数据继续用（纯兜底）
    *sink_cell().lock().unwrap_or_else(|e| e.into_inner()) = f;
}

fn emit(e: UiEvent) {
    if let Ok(guard) = sink_cell().lock() {
        if let Some(f) = guard.as_ref() {
            f(e);
        }
    }
}

// ---------- 行打印（原 TUI 函数的等价 API：改为事件回调） ----------

pub fn info(msg: impl AsRef<str>) {
    if level() >= 1 {
        emit(UiEvent::Log { level: 1, message: msg.as_ref().to_string() });
    }
}

pub fn warn(msg: impl AsRef<str>) {
    if level() >= 1 {
        emit(UiEvent::Log { level: 1, message: msg.as_ref().to_string() });
    }
}

pub fn error(msg: impl AsRef<str>) {
    emit(UiEvent::Log { level: 1, message: msg.as_ref().to_string() });
}

pub fn verbose(msg: impl AsRef<str>) {
    if level() >= 2 {
        emit(UiEvent::Log { level: 2, message: msg.as_ref().to_string() });
    }
}

pub fn downloading_line(rel: &str) {
    emit(UiEvent::DownloadStart { rel: rel.to_string() });
}

pub fn uploading_line(rel: &str) {
    emit(UiEvent::UploadStart { rel: rel.to_string() });
}

pub fn uploaded_line(rel: &str, size: &str, took: &str) {
    emit(UiEvent::UploadDone {
        rel: rel.to_string(),
        size: size.to_string(),
        took: took.to_string(),
    });
}

pub fn downloaded_line(rel: &str, size: &str, speed: &str, took: &str) {
    emit(UiEvent::DownloadDone {
        rel: rel.to_string(),
        size: size.to_string(),
        speed: speed.to_string(),
        took: took.to_string(),
    });
}

// ---------- TUI 存根（调用点零改动；GUI 无终端状态栏） ----------

pub fn set_colors_ok(_on: bool) {}
pub fn set_upload_mode(_on: bool) {}
pub fn set_zip_mode(_on: bool) {}
pub fn enable_status_region() {}
pub fn refresh_status() {}
pub fn hide_cursor() {}
pub fn show_cursor() {}
pub fn clear_status_line() {}
pub fn finalize_terminal() {}
pub fn init_icons(_forced_nerd: bool) {}
pub fn detect_nerd_font() -> bool {
    false
}

// ---------- 纯格式化（原版照抄） ----------

pub fn fmt_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 1.0 {
        format!("{}ms", d.as_millis())
    } else if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        let m = (secs / 60.0) as u64;
        let s = (secs as u64) % 60;
        format!("{m}m{s:02}s")
    }
}

pub fn fmt_bytes(n: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = n as f64;
    let mut u = 0usize;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}
