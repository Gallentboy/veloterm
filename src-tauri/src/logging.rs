use std::sync::atomic::{AtomicUsize, Ordering};

use log::{Level, LevelFilter};
use tauri::{AppHandle, Manager, Runtime};
use tauri::plugin::TauriPlugin;
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};

/// 单个日志文件最大体积：20 MB
const MAX_LOG_FILE_SIZE_BYTES: u128 = 20 * 1024 * 1024;
const LOG_FILE_NAME: &str = "veloterm";

/// 全局原子日志等级阈值（默认 `Info` = 3）。
/// 编码映射与 `log::LevelFilter as usize` 保持一致：
/// Off = 0, Error = 1, Warn = 2, Info = 3, Debug = 4, Trace = 5
static CURRENT_LOG_LEVEL: AtomicUsize = AtomicUsize::new(LevelFilter::Info as usize);

fn usize_to_level_filter(raw: usize) -> LevelFilter {
  match raw {
    0 => LevelFilter::Off,
    1 => LevelFilter::Error,
    2 => LevelFilter::Warn,
    3 => LevelFilter::Info,
    4 => LevelFilter::Debug,
    _ => LevelFilter::Trace,
  }
}

fn parse_log_level(level: &str) -> Result<LevelFilter, String> {
  match level.trim().to_ascii_lowercase().as_str() {
    "error" => Ok(LevelFilter::Error),
    "warn" | "warning" => Ok(LevelFilter::Warn),
    "info" => Ok(LevelFilter::Info),
    "debug" => Ok(LevelFilter::Debug),
    "trace" => Ok(LevelFilter::Trace),
    other => Err(format!(
      "invalid log level '{other}', expected one of: trace, debug, info, warn, error"
    )),
  }
}

fn level_filter_to_str(filter: LevelFilter) -> &'static str {
  match filter {
    LevelFilter::Error => "error",
    LevelFilter::Warn => "warn",
    LevelFilter::Info | LevelFilter::Off => "info",
    LevelFilter::Debug => "debug",
    LevelFilter::Trace => "trace",
  }
}

pub fn current_level_filter() -> LevelFilter {
  usize_to_level_filter(CURRENT_LOG_LEVEL.load(Ordering::Relaxed))
}

pub fn apply_log_level_filter(filter: LevelFilter) {
  CURRENT_LOG_LEVEL.store(filter as usize, Ordering::SeqCst);
  log::set_max_level(filter);
}

/// 构建 `tauri_plugin_log` 插件实例：
/// - 底层 `fern` 基础通道设为 `LevelFilter::Trace`（放行所有级别进入动态 filter）；
/// - 通过 `.filter(...)` 实时读取 `CURRENT_LOG_LEVEL` 原子变量决定是否输出；
/// - 同时输出至控制台 (`Stdout`) 与本地日志目录 (`LogDir { file_name: Some("veloterm") }`)，单文件上限 20MB 滚动。
pub fn build_log_plugin<R: Runtime>() -> TauriPlugin<R> {
  tauri_plugin_log::Builder::new()
    .level(LevelFilter::Trace)
    .max_file_size(MAX_LOG_FILE_SIZE_BYTES)
    .rotation_strategy(RotationStrategy::KeepOne)
    .targets([
      Target::new(TargetKind::Stdout),
      Target::new(TargetKind::LogDir {
        file_name: Some(LOG_FILE_NAME.into()),
      }),
    ])
    .filter(|metadata| {
      let threshold = current_level_filter();
      let record_level: Level = metadata.level();
      record_level <= threshold
    })
    .build()
}

#[tauri::command]
pub fn set_log_level(level: String) -> Result<(), String> {
  let filter = parse_log_level(&level)?;
  apply_log_level_filter(filter);
  let level_str = level_filter_to_str(filter);

  if let Some(pool) = crate::db::global_pool() {
    let _ = crate::db::set_kv(&pool, "veloterm:settings:log-level", level_str);
  }

  log::info!(
    "[Rust Logger] log level dynamically updated to '{}'",
    level_str
  );
  log::debug!(
    "[Rust Logger] debug probe after switching log level to '{}'",
    level_str
  );

  Ok(())
}

#[tauri::command]
pub fn get_current_log_level() -> Result<String, String> {
  Ok(level_filter_to_str(current_level_filter()).to_string())
}

#[tauri::command]
pub fn get_log_file_path(app: AppHandle) -> Result<String, String> {
  let log_dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
  let file_path = log_dir.join(format!("{LOG_FILE_NAME}.log"));
  Ok(file_path.to_string_lossy().into_owned())
}
