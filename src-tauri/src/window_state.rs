//! 窗口几何持久化：把主窗口的**尺寸 + 位置**（未最大化时的逻辑像素）写入 SQLCipher `app_kv`，
//! 下次启动时在首帧绘制前恢复，从而「记住上次关闭时的窗口大小和位置」。
//!
//! 三条关键约定：
//!
//! 1. **落库时实时读窗口，不用缓存。** 缓存依赖 `Resized` / `Moved` 事件，而「刚拖完就退出」
//!    时事件可能还没被事件循环处理完，缓存就是旧值 —— 这会造成「记录的大小不准 / 刚改完就丢失」。
//!    `persist()` 因此直接读窗口当前几何。缓存只用于两处：最大化 / 全屏时提供「还原几何」，
//!    以及窗口已销毁后的退出兜底。
//! 2. **最大化 / 全屏期间不记录几何。** 此时 `inner_size()` / `outer_position()` 拿到的是
//!    最大化后的值，直接落库会让下次启动还原出一个铺满屏幕的「普通」窗口。因此单独存
//!    `maximized` 标志，恢复时先设几何再最大化。
//! 3. **`Resized` / `Moved` 只做节流落库。** 拖拽时这两个事件约 60 次/秒，而 SQLite 每次提交
//!    都要 fsync，不节流会明显卡顿。
//! 4. **最小化期间不记录几何。** Windows 上 `GetWindowRect` 对最小化窗口返回 `(-32000, -32000)`
//!    这类哨兵坐标（窗口被系统挪到屏幕外），`GetClientRect` 也不代表用户意图。不排除掉会把
//!    「垃圾坐标」当成上次的位置存下来。
//! 5. **位置按「平台原生坐标」存取，尺寸始终用逻辑像素。** 见 [`position_is_physical`]。
//!    Windows 的窗口坐标与显示器坐标原生都是**物理像素**，macOS 的排布坐标系是**逻辑点**。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{
  AppHandle, LogicalPosition, LogicalSize, Manager, Monitor, PhysicalPosition, Runtime, Window,
};

use crate::db;

/// `app_kv` 存储键（与 `veloterm:settings:log-level` 同风格）。
const KV_KEY: &str = "veloterm:window:state";

/// 尺寸下限，需与 `tauri.conf.json` 的 `minWidth` / `minHeight` 保持一致。
const MIN_WIDTH: u32 = 480;
const MIN_HEIGHT: u32 = 360;

/// 主窗口标签（`tauri.conf.json` 的 `windows[0]` 未显式指定 label，Tauri 默认 `main`）。
const MAIN_WINDOW_LABEL: &str = "main";

/// 拖拽期间的最小落库间隔。
const PERSIST_THROTTLE: Duration = Duration::from_millis(800);

/// 还原位置时要求窗口与显示器至少重叠这么多（单位同位置的原生单位），
/// 保证标题栏能被鼠标抓到。Windows 上是物理像素，200% 缩放下相当于 60 逻辑像素，仍够抓。
const MIN_VISIBLE_WIDTH: i64 = 120;
const MIN_VISIBLE_HEIGHT: i64 = 48;

/// 持久化的窗口几何（尺寸为逻辑像素；位置单位见 [`position_is_physical`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WindowState {
  pub width: u32,
  pub height: u32,
  /// 窗口左上角 x（平台原生单位：Windows 为物理像素，其它平台为逻辑像素）。
  /// 旧存档无该字段时为 `None`，此时不恢复位置。
  #[serde(default)]
  pub x: Option<i32>,
  /// 窗口左上角 y（平台原生单位，同 `x`）。
  #[serde(default)]
  pub y: Option<i32>,
  /// 关闭时是否处于最大化；为 `true` 时 `width` / `height` / `x` / `y` 是最大化之前的还原几何。
  #[serde(default)]
  pub maximized: bool,
}

/// 逻辑像素下的矩形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
  left: i64,
  top: i64,
  right: i64,
  bottom: i64,
}

/// 内存缓存：最近一次观察到的还原几何 + 上次落库时间（用于节流）。
struct Cache {
  state: WindowState,
  last_write: Option<Instant>,
}

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

fn cache() -> &'static Mutex<Cache> {
  CACHE.get_or_init(|| {
    Mutex::new(Cache {
      state: WindowState::default(),
      last_write: None,
    })
  })
}

fn read_cache() -> WindowState {
  match cache().lock() {
    Ok(guard) => guard.state,
    Err(_) => WindowState::default(),
  }
}

fn write_cache(state: WindowState) {
  if let Ok(mut guard) = cache().lock() {
    guard.state = state;
  }
}

/// 是否为主窗口 —— 供 `on_window_event` 过滤其它窗口的事件。
pub fn is_main_window(label: &str) -> bool {
  label == MAIN_WINDOW_LABEL
}

// ===== 纯函数（可单测） =====

/// 位置是否以**物理像素**存取。
///
/// 结论：Windows 用物理像素，其它平台用逻辑像素。这不是风格偏好，而是两平台坐标系的客观差异：
///
/// - **Windows**：`GetWindowRect` / `SetWindowPos` 收发的是虚拟屏幕的物理像素，
///   `Monitor::position()` 取的也是 `MONITORINFO.rcMonitor`（物理像素）。若改存逻辑像素，
///   还原时 `to_physical(window.scale_factor())` 会用「窗口此刻所在显示器」的缩放比，
///   去换算「当初在另一块不同缩放比显示器上」的坐标 —— 笔记本内屏 100% + 外接屏 200%
///   这种常见组合下，窗口会落到错误的位置甚至另一块屏上。存物理像素时
///   `PhysicalPosition` 的换算恒等，与缩放比无关。
/// - **macOS**：`CGDisplayBounds` / `NSScreen.frame` 的排布坐标系是**逻辑点**，tao 的
///   `outer_position()` 返回的是「点 × scale_factor」，必须除回去才与 `available_monitors()`
///   的位置可比；`set_outer_position` 又用 `to_logical` 换回点。此平台存逻辑像素才是精确的。
///
/// 尺寸则两平台统一用逻辑像素 —— 这与 per-monitor DPI 下「跨屏保持视觉大小」的系统行为一致。
fn position_is_physical() -> bool {
  cfg!(windows)
}

/// 物理像素 → 平台原生单位（单轴）。`physical` 为真时恒等，否则按 `scale` 换算成逻辑像素。
fn physical_to_native(value: i64, scale: f64, physical: bool) -> Option<i64> {
  if physical {
    return Some(value);
  }
  if !scale.is_finite() || scale <= 0.0 {
    return None;
  }
  Some((value as f64 / scale).round() as i64)
}

/// 逻辑像素 → 平台原生单位（单轴），[`physical_to_native`] 的反方向。
fn logical_to_native(value: i64, scale: f64, physical: bool) -> Option<i64> {
  if !physical {
    return Some(value);
  }
  if !scale.is_finite() || scale <= 0.0 {
    return None;
  }
  Some((value as f64 * scale).round() as i64)
}

/// 把尺寸夹到 `[最小尺寸, max]` 区间；`max` 为 `None` 时只应用下限。
/// 极端小屏（`max` 比最小尺寸还小）时以最小尺寸为准 —— 否则会得到比窗口约束还小的非法尺寸。
fn clamp_size(width: u32, height: u32, max: Option<(u32, u32)>) -> (u32, u32) {
  let mut width = width.max(MIN_WIDTH);
  let mut height = height.max(MIN_HEIGHT);

  if let Some((max_width, max_height)) = max {
    width = width.min(max_width);
    height = height.min(max_height);
  }

  (width.max(MIN_WIDTH), height.max(MIN_HEIGHT))
}

/// 窗口矩形与显示器矩形的重叠是否达到最小可见面积。
fn overlaps_enough(window: Rect, monitor: Rect) -> bool {
  let overlap_width = window.right.min(monitor.right) - window.left.max(monitor.left);
  let overlap_height = window.bottom.min(monitor.bottom) - window.top.max(monitor.top);
  overlap_width >= MIN_VISIBLE_WIDTH && overlap_height >= MIN_VISIBLE_HEIGHT
}

// ===== 窗口读取 =====

/// 读取窗口内尺寸（逻辑像素）。`scale_factor` 异常或尺寸非正时返回 `None`。
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

/// 读取窗口左上角位置，单位由 [`position_is_physical`] 决定。
///
/// `outer_position()` 在 tao 两平台都返回 `PhysicalPosition`，但含义不同：Windows 是真的物理像素，
/// macOS 是「逻辑点 × scale_factor」。`physical_to_native` 负责按平台折回原生坐标系。
/// （tao 的 `outer_position()` 已把 macOS 的左下原点换算成左上原点。）
fn native_outer_position<R: Runtime>(window: &Window<R>) -> Option<(i32, i32)> {
  let position = window.outer_position().ok()?;

  // Windows 直接可用，连缩放比都不用读（少一个失败点）
  if position_is_physical() {
    return Some((position.x, position.y));
  }

  let scale = window.scale_factor().ok()?;
  let x = physical_to_native(i64::from(position.x), scale, false)?;
  let y = physical_to_native(i64::from(position.y), scale, false)?;
  Some((x as i32, y as i32))
}

/// 显示器在平台原生坐标系下的矩形。
fn monitor_native_rect(monitor: &Monitor, physical: bool) -> Option<Rect> {
  let scale = monitor.scale_factor();
  let position = monitor.position();
  let size = monitor.size();

  let left = physical_to_native(i64::from(position.x), scale, physical)?;
  let top = physical_to_native(i64::from(position.y), scale, physical)?;
  let width = physical_to_native(i64::from(size.width), scale, physical)?;
  let height = physical_to_native(i64::from(size.height), scale, physical)?;
  Some(Rect {
    left,
    top,
    right: left + width,
    bottom: top + height,
  })
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

/// 读取窗口当前几何。最大化 / 全屏 / 最小化时当前几何不代表「还原意图」，沿用缓存中最后一次的普通态几何。
fn capture<R: Runtime>(window: &Window<R>) -> WindowState {
  let cached = read_cache();

  // 全屏：尺寸、位置、最大化状态都不代表还原意图，整体沿用缓存
  if window.is_fullscreen().unwrap_or(false) {
    return cached;
  }

  // 最小化：Windows 的 `GetWindowRect` 对最小化窗口返回 `(-32000, -32000)`（系统把窗口挪到了
  // 屏幕外），此时 `outer_position()` 拿到的是哨兵值而非真实位置；`GetClientRect` 的尺寸同样
  // 不可信。不排除掉就会把垃圾坐标当成「上次关闭的位置」落库，下次启动窗口直接跑到屏幕外。
  // 与全屏同理，整体沿用缓存里最小化之前的几何。
  if window.is_minimized().unwrap_or(false) {
    return cached;
  }

  // 最大化：只更新标志，几何沿用缓存里最大化之前的还原值
  if window.is_maximized().unwrap_or(false) {
    return WindowState {
      maximized: true,
      ..cached
    };
  }

  let (width, height) = logical_inner_size(window).unwrap_or((cached.width, cached.height));
  let (x, y) = match native_outer_position(window) {
    Some((x, y)) => (Some(x), Some(y)),
    None => (cached.x, cached.y),
  };

  WindowState {
    width,
    height,
    x,
    y,
    maximized: false,
  }
}

// ===== 落库 / 恢复 =====

fn write_kv(state: &WindowState) {
  // 缓存尚未初始化（从未成功读到几何）时不写入，避免把 0×0 落库
  if state.width < MIN_WIDTH || state.height < MIN_HEIGHT {
    return;
  }

  let Some(pool) = db::global_pool() else {
    return;
  };

  match serde_json::to_string(state) {
    Ok(json) => {
      if let Err(err) = db::set_kv(&pool, KV_KEY, &json) {
        log::warn!("[Window] failed to persist window state: {err}");
      }
    }
    Err(err) => log::warn!("[Window] failed to serialize window state: {err}"),
  }
}

/// 记录窗口几何：更新缓存，并按 `PERSIST_THROTTLE` 节流落库。
/// 由 `WindowEvent::Resized` / `WindowEvent::Moved` 调用（拖拽时高频触发）。
pub fn observe<R: Runtime>(window: &Window<R>) {
  let state = capture(window);

  let should_write = match cache().lock() {
    Ok(mut guard) => {
      guard.state = state;
      let now = Instant::now();
      let allowed = match guard.last_write {
        Some(last) => now.duration_since(last) >= PERSIST_THROTTLE,
        None => true,
      };
      if allowed {
        guard.last_write = Some(now);
      }
      allowed
    }
    Err(_) => false,
  };

  if should_write {
    write_kv(&state);
  }
}

/// 实时读取窗口几何并落库（幂等）。窗口关闭时调用。
///
/// 刻意不用缓存：见模块头注释第 1 条 —— 「刚拖完就退出」时缓存可能是旧值。
pub fn persist<R: Runtime>(window: &Window<R>) {
  let state = capture(window);
  write_cache(state);
  write_kv(&state);
}

/// 退出兜底：窗口仍存在就实时读取，已销毁则回落到缓存。
/// 覆盖 macOS ⌘Q / 应用菜单退出等不经过窗口 `CloseRequested` 的路径。
pub fn persist_from_handle<R: Runtime>(app: &AppHandle<R>) {
  match app.get_webview_window(MAIN_WINDOW_LABEL) {
    Some(webview_window) => {
      let webview: &tauri::Webview<R> = webview_window.as_ref();
      persist(&webview.window());
    }
    None => write_kv(&read_cache()),
  }
}

/// 存档位置是否仍落在某块显示器上。拔掉外接屏后旧坐标可能完全在屏幕外，
/// 此时放弃位置恢复、交给系统默认摆放，避免窗口「打开即消失」。
///
/// `x` / `y` 是平台原生单位，`width` / `height` 是逻辑像素：窗口矩形要换算到原生单位才能和
/// 显示器矩形比较。换算用**该显示器自身**的缩放比 —— 窗口最终落在这块屏上时这个换算就是精确的。
fn is_position_visible<R: Runtime>(
  window: &Window<R>,
  x: i32,
  y: i32,
  width: u32,
  height: u32,
) -> bool {
  let Ok(monitors) = window.available_monitors() else {
    return false;
  };

  let physical = position_is_physical();
  monitors.iter().any(|monitor| {
    let scale = monitor.scale_factor();
    let (Some(monitor_rect), Some(window_width), Some(window_height)) = (
      monitor_native_rect(monitor, physical),
      logical_to_native(i64::from(width), scale, physical),
      logical_to_native(i64::from(height), scale, physical),
    ) else {
      return false;
    };

    overlaps_enough(
      Rect {
        left: i64::from(x),
        top: i64::from(y),
        right: i64::from(x) + window_width,
        bottom: i64::from(y) + window_height,
      },
      monitor_rect,
    )
  })
}

/// 读取存档并恢复窗口几何。应在 `setup` 中、首帧绘制前调用
/// （Tauri 在同一个事件循环迭代内先创建窗口、再执行 setup，此时改几何不会产生跳变）。
pub fn restore<R: Runtime>(window: &Window<R>) {
  let saved = db::global_pool()
    .and_then(|pool| db::get_kv(&pool, KV_KEY).ok().flatten())
    .and_then(|raw| serde_json::from_str::<WindowState>(&raw).ok())
    .filter(|state| state.width >= MIN_WIDTH && state.height >= MIN_HEIGHT);

  let Some(saved) = saved else {
    // 首次运行或存档损坏：以当前（配置默认）几何初始化缓存
    observe(window);
    return;
  };

  let (width, height) = clamp_size(saved.width, saved.height, monitor_logical_max(window));
  let position = match (saved.x, saved.y) {
    (Some(x), Some(y)) if is_position_visible(window, x, y, width, height) => Some((x, y)),
    _ => None,
  };

  write_cache(WindowState {
    width,
    height,
    x: position.map(|p| p.0),
    y: position.map(|p| p.1),
    maximized: saved.maximized,
  });

  // 先尺寸、后位置：改尺寸会连带改动窗口原点（macOS 的 `setFrame` 以左下角为锚点向上生长，
  // Windows 跨屏时系统按 DPI 重算窗口矩形），把位置放在最后写才能保证坐标精确落位。
  if let Err(err) = window.set_size(LogicalSize::new(f64::from(width), f64::from(height))) {
    log::warn!("[Window] failed to restore window size: {err}");
  }
  if let Some((x, y)) = position {
    // Windows 传 PhysicalPosition：tao 的 `to_physical` 对物理坐标是恒等变换，
    // 不受「窗口此刻所在显示器」缩放比影响；其它平台传 LogicalPosition。
    let result = if position_is_physical() {
      window.set_position(PhysicalPosition::new(x, y))
    } else {
      window.set_position(LogicalPosition::new(f64::from(x), f64::from(y)))
    };
    if let Err(err) = result {
      log::warn!("[Window] failed to restore window position: {err}");
    }
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
      x: Some(-320),
      y: Some(140),
      maximized: true,
    };
    let json = serde_json::to_string(&state).expect("serialize window state");
    assert_eq!(
      serde_json::from_str::<WindowState>(&json).expect("deserialize window state"),
      state
    );
  }

  #[test]
  fn legacy_payload_without_position_or_maximized_still_parses() {
    // 旧存档只有 width/height，解析不应失败，且位置视为「未保存」
    let parsed: WindowState =
      serde_json::from_str(r#"{"width":1024,"height":768}"#).expect("deserialize legacy payload");
    assert_eq!(parsed.width, 1024);
    assert_eq!(parsed.height, 768);
    assert_eq!(parsed.x, None);
    assert_eq!(parsed.y, None);
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

  #[test]
  fn native_units_are_identity_when_position_is_physical() {
    // Windows：位置原生就是物理像素，读取方向不做换算
    assert_eq!(physical_to_native(-32000, 2.0, true), Some(-32000));
    assert_eq!(physical_to_native(3840, 2.0, true), Some(3840));
    // 写入方向才需要把逻辑尺寸乘回物理
    assert_eq!(logical_to_native(1200, 2.0, true), Some(2400));
  }

  #[test]
  fn native_units_are_logical_points_when_position_is_not_physical() {
    // macOS：tao 报的是「点 × scale_factor」，除以缩放比才是排布坐标
    assert_eq!(physical_to_native(3840, 2.0, false), Some(1920));
    assert_eq!(physical_to_native(1440, 2.0, false), Some(720));
    // 逻辑方向已是排布坐标，恒等
    assert_eq!(logical_to_native(1920, 2.0, false), Some(1920));
  }

  #[test]
  fn native_units_reject_invalid_scale_factor() {
    assert_eq!(physical_to_native(100, 0.0, false), None);
    assert_eq!(physical_to_native(100, f64::NAN, false), None);
    assert_eq!(logical_to_native(100, -1.0, true), None);
    // 物理方向不依赖缩放比，缩放比异常也应原样通过
    assert_eq!(physical_to_native(100, f64::NAN, true), Some(100));
  }

  #[test]
  fn logical_position_storage_drifts_when_restore_scale_factor_differs() {
    // 这条测试记录「Windows 为什么必须存物理像素」：窗口停在 200% 的外接屏上，物理 x = 4000。
    let physical_x = 4000i64;
    let saved_logical = physical_to_native(physical_x, 2.0, false).expect("save as logical");
    assert_eq!(saved_logical, 2000);

    // 启动时窗口先出现在 100% 的内屏上，按 1.0 还原 → 落到 2000，跑到了错误的显示器上
    assert_eq!(logical_to_native(saved_logical, 1.0, false), Some(2000));

    // 存物理像素则与还原时的缩放比无关，精确回到 4000
    assert_eq!(physical_to_native(physical_x, 2.0, true), Some(4000));
  }

  #[test]
  fn position_unit_matches_platform() {
    let expected = cfg!(windows);
    assert_eq!(position_is_physical(), expected);
  }

  #[test]
  fn overlaps_enough_accepts_partially_visible_and_rejects_offscreen() {
    let monitor = Rect {
      left: 0,
      top: 0,
      right: 1920,
      bottom: 1080,
    };
    let rect = |left, top, right, bottom| Rect {
      left,
      top,
      right,
      bottom,
    };

    // 完全在屏内
    assert!(overlaps_enough(rect(100, 100, 900, 700), monitor));
    // 左边缘溢出但可见区域足够（拖到屏幕左边）→ 接受
    assert!(overlaps_enough(rect(-800, 100, 200, 700), monitor));
    // 只露出很窄一条 → 拒绝（标题栏抓不住）
    assert!(!overlaps_enough(rect(-800, 100, 50, 700), monitor));
    // 完全在屏幕外（外接屏被拔掉）→ 拒绝
    assert!(!overlaps_enough(rect(2600, 200, 3400, 900), monitor));
    // 在显示器正上方且纵向重叠不足 → 拒绝
    assert!(!overlaps_enough(rect(200, -900, 900, 20), monitor));
  }
}
