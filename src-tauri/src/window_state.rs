//! 窗口尺寸持久化：把主窗口「未最大化时的逻辑尺寸」写入 SQLCipher `app_kv`，
//! 下次启动时在首帧绘制前恢复，从而记住上次关闭时的大小。
//!
//! 为什么要单独缓存而不是直接读窗口：
//! 最大化 / 全屏期间 `inner_size()` 返回的是最大化后的尺寸，直接落库会让下次启动
//! 还原出一个铺满屏幕的「普通」窗口。因此这里只在非最大化、非全屏时记录尺寸，
//! 并单独保存 `maximized` 标志，恢复时先设尺寸再最大化。
//!
//! 落库时机只有「关闭 / 退出」两处：`Resized` 期间只更新内存缓存，
//! 避免拖拽改变窗口大小时高频写库（SQLite 每次提交都要 fsync）。

use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::{LogicalSize, Runtime, Window};

use crate::db;

/// `app_kv` 存储键（与 `veloterm:settings:log-level` 同风格）。
const KV_KEY: &str = "veloterm:window:state";

/// 尺寸下限，需与 `tauri.conf.json` 的 `minWidth` / `minHeight` 保持一致。
const MIN_WIDTH: u32 = 480;
const MIN_HEIGHT: u32 = 360;

/// 主窗口标签（`tauri.conf.json` 的 `windows[0]` 未显式指定 label，Tauri 默认 `main`）。
const MAIN_WINDOW_LABEL: &str = "main";

/// 持久化的窗口尺寸（逻辑像素）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowState {
  pub width: u32,
  pub height: u32,
  /// 关闭时是否处于最大化；为 `true` 时 `width` / `height` 是最大化之前的还原尺寸。
  #[serde(default)]
  pub maximized: bool,
}

/// 最近一次观察到的窗口尺寸。
static CACHE: OnceLock<Mutex<WindowState>> = OnceLock::new();

fn cache() -> &'static Mutex<WindowState> {
  CACHE.get_or_init(|| {
    Mutex::new(WindowState {
      width: 0,
      height: 0,
      maximized: false,
    })
  })
}

/// 是否为主窗口 —— 供 `on_window_event` 过滤其它窗口的事件。
pub fn is_main_window(label: &str) -> bool {
  label == MAIN_WINDOW_LABEL
}

/// 读取窗口当前内尺寸（逻辑像素）。`scale_factor` 异常或尺寸非正时返回 `None`。
fn logical_inner_size<R: Runtime>(window: &Window<R>) -> Option<(u32, u32)> {
  let scale = window.scale_factor().ok()?;
  if !scale.is_finite() || scale <= 0.0 {
    return None;
  }

  let logical = window.inner_size().ok()?.to_logical::<f64>(scale);
  if !logical.width.is_finite() || !logical.height.is_finite() {
    return None;
  }

  let width = logical.width.round();
  let height = logical.height.round();
  if width < 1.0 || height < 1.0 {
    return None;
  }
  Some((width as u32, height as u32))
}

fn write_cache(state: WindowState) {
  if let Ok(mut guard) = cache().lock() {
    *guard = state;
  }
}

/// 纯函数：把尺寸夹到 `[最小尺寸, max]` 区间；`max` 为 `None` 时只应用下限。
/// 极端小屏（`max` 比最小尺寸还小）时以最小尺寸为准 —— 否则会出现窗口比约束还小的非法状态。
fn clamp_size(width: u32, height: u32, max: Option<(u32, u32)>) -> (u32, u32) {
  let mut width = width.max(MIN_WIDTH);
  let mut height = height.max(MIN_HEIGHT);

  if let Some((max_width, max_height)) = max {
    width = width.min(max_width);
    height = height.min(max_height);
  }

  (width.max(MIN_WIDTH), height.max(MIN_HEIGHT))
}

/// 当前显示器的逻辑尺寸上限（拿不到显示器信息时返回 `None`，即不做上限夹取）。
fn monitor_logical_max<R: Runtime>(window: &Window<R>) -> Option<(u32, u32)> {
  let monitor = window
    .current_monitor()
    .ok()
    .flatten()
    .or_else(|| window.primary_monitor().ok().flatten())?;

  let scale = monitor.scale_factor();
  if !scale.is_finite() || scale <= 0.0 {
    return None;
  }

  let logical = monitor.size().to_logical::<f64>(scale);
  if !logical.width.is_finite() || !logical.height.is_finite() {
    return None;
  }

  let width = logical.width.round();
  let height = logical.height.round();
  if width < 1.0 || height < 1.0 {
    return None;
  }
  Some((width as u32, height as u32))
}

/// 把尺寸夹到「不小于窗口最小尺寸、不大于当前显示器逻辑尺寸」的区间，
/// 避免在大屏上保存的尺寸被还原到小屏时超出可视范围。
fn clamp_to_monitor<R: Runtime>(window: &Window<R>, width: u32, height: u32) -> (u32, u32) {
  clamp_size(width, height, monitor_logical_max(window))
}

/// 记录当前窗口尺寸到内存缓存（不写库）。由 `WindowEvent::Resized` 调用。
pub fn observe<R: Runtime>(window: &Window<R>) {
  let maximized = window.is_maximized().unwrap_or(false);
  let fullscreen = window.is_fullscreen().unwrap_or(false);

  // 全屏期间尺寸和最大化状态都不代表「还原意图」，整体跳过
  if fullscreen {
    return;
  }

  let Ok(mut guard) = cache().lock() else {
    return;
  };
  guard.maximized = maximized;

  // 最大化期间 inner_size() 不是还原尺寸，保持上一次记录的值
  if !maximized && let Some((width, height)) = logical_inner_size(window) {
    guard.width = width;
    guard.height = height;
  }
}

/// 把缓存中的尺寸写入 `app_kv`。窗口关闭 / 应用退出时调用（幂等）。
pub fn persist() {
  let Ok(guard) = cache().lock() else {
    return;
  };
  let state = *guard;
  drop(guard);

  // 缓存尚未初始化（未收到过任何尺寸事件）时不写入，避免把 0×0 落库
  if state.width < MIN_WIDTH || state.height < MIN_HEIGHT {
    return;
  }

  let Some(pool) = db::global_pool() else {
    return;
  };

  match serde_json::to_string(&state) {
    Ok(json) => {
      if let Err(err) = db::set_kv(&pool, KV_KEY, &json) {
        log::warn!("[Window] failed to persist window state: {err}");
      }
    }
    Err(err) => log::warn!("[Window] failed to serialize window state: {err}"),
  }
}

/// 读取存档并恢复窗口尺寸。应在 `setup` 中、首帧绘制前调用
/// （Tauri 在同一个事件循环迭代内先创建窗口、再执行 setup，此时改尺寸不会产生跳变）。
pub fn restore<R: Runtime>(window: &Window<R>) {
  let saved = db::global_pool()
    .and_then(|pool| db::get_kv(&pool, KV_KEY).ok().flatten())
    .and_then(|raw| serde_json::from_str::<WindowState>(&raw).ok())
    .filter(|state| state.width >= MIN_WIDTH && state.height >= MIN_HEIGHT);

  let Some(saved) = saved else {
    // 首次运行或存档损坏：以当前（配置默认）尺寸初始化缓存
    observe(window);
    return;
  };

  let (width, height) = clamp_to_monitor(window, saved.width, saved.height);
  write_cache(WindowState {
    width,
    height,
    maximized: saved.maximized,
  });

  if let Err(err) = window.set_size(LogicalSize::new(f64::from(width), f64::from(height))) {
    log::warn!("[Window] failed to restore window size: {err}");
  }
  if saved.maximized && let Err(err) = window.maximize() {
    log::warn!("[Window] failed to restore maximized state: {err}");
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn window_state_serde_roundtrip() {
    let state = WindowState {
      width: 1280,
      height: 800,
      maximized: true,
    };
    let json = serde_json::to_string(&state).expect("serialize window state");
    assert_eq!(
      serde_json::from_str::<WindowState>(&json).expect("deserialize window state"),
      state
    );
  }

  #[test]
  fn missing_maximized_field_defaults_to_false() {
    // 早期存档可能没有 maximized 字段，解析不应失败
    let parsed: WindowState =
      serde_json::from_str(r#"{"width":1024,"height":768}"#).expect("deserialize legacy payload");
    assert_eq!(parsed.width, 1024);
    assert_eq!(parsed.height, 768);
    assert!(!parsed.maximized);
  }

  #[test]
  fn clamp_size_applies_min_and_monitor_bounds() {
    // 低于下限 → 抬到下限
    assert_eq!(clamp_size(100, 100, None), (MIN_WIDTH, MIN_HEIGHT));
    // 高于显示器 → 压到显示器
    assert_eq!(clamp_size(4000, 3000, Some((1920, 1080))), (1920, 1080));
    // 落在区间内 → 原样保留
    assert_eq!(clamp_size(1280, 800, Some((1920, 1080))), (1280, 800));
    // 显示器比最小尺寸还小 → 以最小尺寸为准
    assert_eq!(clamp_size(1280, 800, Some((320, 200))), (MIN_WIDTH, MIN_HEIGHT));
  }
}
