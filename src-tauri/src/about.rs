//! 自绘「关于」窗口（架构对齐 ZCode 的 aboutWindow/about.ts：Electron
//! frameless+transparent 自绘弹窗 → Tauri 无边框透明子窗口）。
//!
//! 设计要点（参考 ZCode）：
//! - **统一入口**：macOS 菜单栏「关于 VeloTerm」与右上角帮助菜单的「关于」
//!   都经 `show_about` 打开同一个 `about` 窗口（ZCode 的两条入口同走
//!   `DesktopCommandIds.ShowAbout`）。macOS 侧需要替换默认菜单 App 子菜单
//!   里的原生 About 面板项（Tauri 不自定义菜单时默认菜单带 PredefinedMenuItem
//!   ::about，弹的是系统原生面板，与自绘窗不一致）。
//! - **圆角**：根面板在 WebView 里画 CSS 圆角（12px，ZCode 的面板圆角语言，
//!   其 desktopWindowChrome 注释「内层 12px 面板」），窗口 transparent 露出
//!   四角——同 ZCode「透明底只负责露出四角」的做法；系统阴影对透明窗口
//!   会按方形窗计算（ZCode windowsCuaOperationIndicator 踩过同坑），故
//!   shadow(false)。
//! - **内容供给**：复用主 bundle 的 `index.html?window=about` 查询参数，
//!   前端 bootstrap 走轻量分支（不启 SQLCipher 前端水合，主题/语言直接
//!   invoke db_get_kv 读全局池）。

use tauri::{
    AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
    menu::{Menu, MenuItem, MenuItemKind},
};

pub const ABOUT_WINDOW_LABEL: &str = "about";
/// 窗口 = 256×340 卡片 + 四周 22px 出血边距（透明区）：给卡片的 CSS 投影
/// 留出绘制空间（阴影画在窗口内不会被裁），立体感不依赖系统阴影
const ABOUT_WINDOW_WIDTH: f64 = 300.0;
const ABOUT_WINDOW_HEIGHT: f64 = 384.0;
/// 主窗淡化/关闭联动事件（payload: bool = 关于窗是否可见）
pub const ABOUT_VISIBILITY_EVENT: &str = "about-visibility";

/// 幂等打开（或聚焦已存在的）关于窗口。
pub fn show_about(app: &AppHandle) {
    if let Some(existing) = app.get_webview_window(ABOUT_WINDOW_LABEL) {
        let _ = existing.set_focus();
        return;
    }

    let builder = WebviewWindowBuilder::new(
        app,
        ABOUT_WINDOW_LABEL,
        WebviewUrl::App("index.html?window=about".into()),
    )
    .title("VeloTerm")
    .inner_size(ABOUT_WINDOW_WIDTH, ABOUT_WINDOW_HEIGHT)
    .decorations(false)
    .transparent(true)
    .resizable(false)
    .minimizable(false)
    .maximizable(false)
    .fullscreen(false)
    // 透明窗口的系统阴影按方形窗计算，会画出出界的灰影，关掉由内容自带形态
    .shadow(false)
    // modal 语义：关于窗存在期间置顶——主窗被点击也不会抬到它前面把它挡住
    // （主窗激活时另有焦点弹回逻辑，见 lib.rs setup）
    .always_on_top(true)
    // Windows 上不进任务栏（macOS 本就无此项）
    .skip_taskbar(true)
    .visible(false)
    .on_page_load(|window, payload| {
        if payload.event() == tauri::webview::PageLoadEvent::Finished {
            let _ = window.show();
            let _ = window.set_focus();
        }
    });

    let window = match builder.build() {
        Ok(window) => window,
        Err(err) => {
            log::warn!("[VeloTerm] 创建关于窗口失败：{err}");
            return;
        }
    };

    // 主窗淡化联动时机由前端驱动：AboutWindow mount（卡片入场动画首帧）广播
    // true、退场动画首帧广播 false——与卡片动效同帧、不割裂。这里只保留窗口
    // 销毁时的兜底广播（前端来不及发事件的极端路径，主窗 setState 幂等）
    let app_for_close = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            let _ = app_for_close.emit(ABOUT_VISIBILITY_EVENT, false);
        }
    });

    // 居中到主窗口（ZCode 是 parent modal 弹在父窗上；这里至少做到视觉居中，
    // 不做真 modal——Tauri 的 parent/child 组合与 transparent 有平台边角）
    if let Some(main) = app.get_webview_window("main")
      && let (Ok(pos), Ok(size), Ok(scale)) =
        (main.outer_position(), main.outer_size(), main.scale_factor())
    {
        let x = pos.x as f64 + (size.width as f64 - ABOUT_WINDOW_WIDTH * scale) / 2.0;
        let y = pos.y as f64 + (size.height as f64 - ABOUT_WINDOW_HEIGHT * scale) / 2.0;
        let _ = window.set_position(tauri::PhysicalPosition::new(
            x.round() as i32,
            y.round() as i32,
        ));
    }
}

/// 前端入口（右上角帮助菜单）：与菜单栏事件同走 [`show_about`]。
#[tauri::command]
pub fn show_about_window(app: AppHandle) {
    show_about(&app);
}

/// macOS：替换默认菜单 App 子菜单首项（原生 About 面板）为自定义关于项。
/// 默认菜单的其余部分（Edit/View/Window 与 App 子菜单的 services/hide/quit
/// 等预定义项）保持不动——Edit 菜单的复制粘贴快捷键被系统文本框依赖。
/// 已知边界：菜单文案在启动时按当前语言构建，语言切换后需重启生效
/// （ZCode 有菜单重建通道，veloterm 首版从简）。
#[cfg(target_os = "macos")]
pub fn install_macos_menu(app: &AppHandle) {
    let pool = app.state::<crate::db::DbPool>();
    let locale = crate::db::get_kv(&pool, "veloterm-locale")
        .ok()
        .flatten()
        .unwrap_or_default();
    let label = if locale == "en-US" { "About VeloTerm" } else { "关于 VeloTerm" };

    let menu = match Menu::default(app) {
        Ok(menu) => menu,
        Err(err) => {
            log::warn!("[VeloTerm] 构建默认菜单失败，保留原生 About：{err}");
            return;
        }
    };
    // 默认菜单第一项是 App 子菜单（macOS），其首项是 PredefinedMenuItem::about
    let Ok(items) = menu.items() else {
        return;
    };
    let Some(MenuItemKind::Submenu(app_submenu)) = items.first() else {
        return;
    };
    let about = match MenuItem::with_id(app, "veloterm-about", label, true, None::<&str>) {
        Ok(item) => item,
        Err(err) => {
            log::warn!("[VeloTerm] 构建关于菜单项失败：{err}");
            return;
        }
    };
    if let Err(err) = app_submenu.remove_at(0).and_then(|_| app_submenu.insert(&about, 0)) {
        log::warn!("[VeloTerm] 替换 About 菜单项失败：{err}");
        return;
    }

    // 在系统 Help（帮助）子菜单中追加「使用说明 / User Manual」
    let manual_label = if locale == "en-US" { "User Manual" } else { "使用说明" };
    if let Ok(manual) = MenuItem::with_id(app, "veloterm-manual", manual_label, true, None::<&str>)
        && let Some(MenuItemKind::Submenu(help_submenu)) = items.last()
    {
        let _ = help_submenu.append(&manual);
    }

    if let Err(err) = app.set_menu(menu) {
        log::warn!("[VeloTerm] 安装菜单失败：{err}");
    }
}
