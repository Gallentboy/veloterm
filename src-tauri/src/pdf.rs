//! PDF 页面渲染：PDFium（Chromium 同款引擎）渲染为 PNG，前端用 `<img>` 展示。
//!
//! ## 为什么是「后端渲染成图片」而不是前端 PDF.js
//! PDFium 是原生 C++ 引擎，渲染质量与速度都优于纯 JS 实现（尤其复杂排版、
//! 嵌入字体、CJK 文本），代价是必须走后端 + 图片传输。本项目终端/SFTP 场景
//! 的 PDF 多为文档阅读，一次渲染一页足够。
//!
//! ## 线程模型（关键约束）
//! - `Pdfium` 是进程级单例（`LazyLock<Mutex<Option<...>>>`）。**不能用
//!   thread_local**：pdfium-render 的 bindings 存在库内部全局 `OnceCell`，
//!   `bind_to_library` 第二次调用直接返回 `PdfiumLibraryBindingsAlreadyInitialized`
//!   ——渲染跑在 spawn_blocking 上会换线程，每线程各绑一次就炸。
//! - `Pdfium` 本身 `Send + Sync`（0.9 内置 thread_safe feature），进程级共享安全。
//! - 渲染任务跑在 `spawn_blocking`（阻塞线程池），PDFium 的重活不占 tokio worker。
//!
//! ## 库定位
//! vendor/pdfium/ 下有四份预编译库（按平台+架构命名），运行期按需选择：
//! - macOS：`libpdfium-{arm64,x64}.dylib`（install_name 为 `./libpdfium.dylib`，
//!   但 pdfium-render 用绝对路径 dlopen，实际不受 install_name 影响）
//! - Windows：`pdfium-{arm64,x64}.dll`
//!
//! 查找顺序：Tauri resource_dir（打包后）→ 可执行文件同级 → 编译期 vendor 路径
//! （开发期直接跑 target/debug/app 时命中）。任一路径存在即可。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use pdfium_render::prelude::*;

/// 渲染宽度（px）。1600 宽在 2x 屏上约等于 A4 的 200 DPI，正文与图表都清晰；
/// 再高会显著增加 PNG 体积与编码时间，对阅读无实质收益。
const RENDER_TARGET_WIDTH: i32 = 1600;

// ===== 临时文件生命周期 =====
//
// PDF 预览会产生两类临时文件：下载的源 PDF 与渲染出的 PNG。两者都登记在
// 全局集合里，由前端显式释放（关闭预览时调 `release_pdf_temp_files`）或
// 进程退出时兜底清理。不依赖「打开新文档时删旧的」——同时打开多个预览
// 会互相删对方的文件。

/// 本进程创建的 PDF 相关临时文件（绝对路径）。渲染与下载都登记到这里，
/// 供 release / 退出时统一删除。
static PDF_TEMP_FILES: LazyLock<Mutex<HashSet<PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// 登记一个临时文件（后续由 release / 退出清理）
fn register_temp_file(path: &Path) {
    if let Ok(mut set) = PDF_TEMP_FILES.lock() {
        set.insert(path.to_path_buf());
    }
}

/// 删除单个临时文件并移出登记表（文件已不存在也视为成功）
fn remove_temp_file(path: &Path) {
    if let Ok(mut set) = PDF_TEMP_FILES.lock() {
        set.remove(path);
    }
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("[PDF] 删除临时文件失败（{}）: {e}", path.display());
    }
}

/// 释放一批临时文件（前端关闭预览时调用）。
///
/// `paths` 为空时清理本进程登记的全部 PDF 临时文件（应用退出兜底）。
/// 只删除登记过的路径——防止前端传入任意路径导致误删用户文件。
#[tauri::command]
pub fn release_pdf_temp_files(paths: Vec<String>) {
    let registered: Vec<PathBuf> = match PDF_TEMP_FILES.lock() {
        Ok(set) => set.iter().cloned().collect(),
        Err(_) => return,
    };
    if paths.is_empty() {
        for path in registered {
            remove_temp_file(&path);
        }
        return;
    }
    let wanted: HashSet<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    for path in registered {
        if wanted.contains(&path) {
            remove_temp_file(&path);
        }
    }
}

/// 进程退出兜底清理（由 lib.rs 的退出钩子调用）。
pub fn cleanup_all_temp_files() {
    let registered: Vec<PathBuf> = match PDF_TEMP_FILES.lock() {
        Ok(set) => set.iter().cloned().collect(),
        Err(_) => return,
    };
    if registered.is_empty() {
        return;
    }
    log::info!("[PDF] 退出清理 {} 个临时文件", registered.len());
    for path in registered {
        remove_temp_file(&path);
    }
}

/// 生成下载用的本地临时路径并登记（前端下载远端 PDF 时调用）。
///
/// 只负责分配路径与登记，实际下载由既有的 `sftp_download_file` 完成
/// （它已有进度 Channel、取消、续传）。
#[tauri::command]
pub fn pdf_temp_path(file_name: String) -> Result<String, String> {
    // 只取文件名部分，防止前端传入带目录分隔符的路径逃逸出临时目录
    let safe_name = Path::new(&file_name)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "无效的文件名".to_string())?;
    let path = std::env::temp_dir().join(format!("veloterm_pdf_src_{safe_name}"));
    register_temp_file(&path);
    Ok(path.to_string_lossy().into_owned())
}

/// 单页渲染结果
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfPageImage {
    /// 临时 PNG 文件绝对路径（前端经 convertFileSrc 转 asset:// URL）
    pub path: String,
    /// 文档总页数（前端据此禁用翻页按钮）
    pub page_count: u16,
    /// 实际渲染的像素尺寸（前端可按需做占位布局，避免加载时抖动）
    pub width: u32,
    pub height: u32,
}

/// 当前平台对应的 vendor 库文件名
fn lib_file_name() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "libpdfium-arm64.dylib"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "libpdfium-x64.dylib"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "pdfium-arm64.dll"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "pdfium-x64.dll"
    }
    #[cfg(not(any(
        all(target_os = "macos", any(target_arch = "aarch64", target_arch = "x86_64")),
        all(target_os = "windows", any(target_arch = "aarch64", target_arch = "x86_64"))
    )))]
    {
        "libpdfium.dylib" // 未支持平台：占位，定位失败会给出清晰错误
    }
}

/// 解析 PDFium 动态库的绝对路径。
///
/// 查找顺序（命中即返回）：
/// 1. Tauri `resource_dir`（打包后资源随包，见 tauri.windows.conf.json 的
///    bundle.resources 映射；mac 走 Tauri 默认 resources 目录）
/// 2. 可执行文件同级的 `pdfium/` 子目录（手工部署）
/// 3. 编译期 vendor 路径（开发期 `cargo run` / `tauri dev` 直接命中仓库文件）
fn resolve_pdfium_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let name = lib_file_name();

    // 1. 打包资源目录（Tauri 2：resource_dir 下的相对路径由 resources 映射决定）
    if let Ok(dir) = app.path().resource_dir() {
        for candidate in [dir.join("pdfium").join(name), dir.join(name)] {
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    // 2. 可执行文件同级（便携部署 / 手工拷库）
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        for candidate in [parent.join("pdfium").join(name), parent.join(name)] {
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    // 3. 编译期 vendor 路径（开发期）
    let dev_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor").join("pdfium").join(name);
    if dev_path.is_file() {
        return Ok(dev_path);
    }

    Err(format!(
        "未找到 PDFium 动态库 {name}（已查找：resource_dir、可执行文件同级、{}）",
        dev_path.display()
    ))
}

/// 进程级 Pdfium 单例。
///
/// **不能用 thread_local**：pdfium-render 的 bindings 存在库内部的全局
/// `OnceCell`，`bind_to_library` 第二次调用直接返回
/// `PdfiumLibraryBindingsAlreadyInitialized`（实测：spawn_blocking 换线程后
/// 每线程各绑一次 → 除第一个线程外全失败）。
///
/// 好在 `Pdfium` 本身是 `Send + Sync`（pdfium-render 0.9 内置 `thread_safe`
/// feature，bindings 走 `ThreadSafePdfiumBindings`），可以安全地进程级共享。
static PDFIUM: LazyLock<Mutex<Option<Result<Pdfium, String>>>> =
    LazyLock::new(|| Mutex::new(None));

/// 取（或初始化）进程级 Pdfium 实例。
fn with_pdfium<R>(
    app: &tauri::AppHandle,
    f: impl FnOnce(&Pdfium) -> Result<R, String>,
) -> Result<R, String> {
    let mut slot = PDFIUM.lock().map_err(|_| "PDFium 状态锁损坏".to_string())?;
    if slot.is_none() {
        *slot = Some(bind_pdfium(app));
    }
    match slot.as_ref() {
        Some(Ok(pdfium)) => f(pdfium),
        Some(Err(e)) => Err(e.clone()),
        None => Err("PDFium 初始化状态异常".to_string()),
    }
}

fn bind_pdfium(app: &tauri::AppHandle) -> Result<Pdfium, String> {
    let path = resolve_pdfium_path(app)?;
    let bindings = Pdfium::bind_to_library(&path)
        .map_err(|e| format!("加载 PDFium 失败（{}）: {e}", path.display()))?;
    log::info!("[PDF] PDFium loaded from {}", path.display());
    Ok(Pdfium::new(bindings))
}

/// 渲染 PDF 指定页为 PNG，返回临时文件路径与文档信息。
///
/// `pdf_path`：本地 PDF 绝对路径（SFTP 远端文件需先下载到临时目录）
/// `page_index`：0 基页码；越界返回错误（前端据 page_count 禁用按钮）
///
/// 命令是 async：PDFium 的加载/渲染（同步阻塞）跑在 spawn_blocking 上，
/// 不占 tokio worker 线程。
#[tauri::command]
pub async fn render_pdf_page(
    app: tauri::AppHandle,
    pdf_path: String,
    page_index: u16,
) -> Result<PdfPageImage, String> {
    // 先做存在性检查（避免把 IO 错误混进阻塞池）
    if !Path::new(&pdf_path).is_file() {
        return Err(format!("PDF 文件不存在: {pdf_path}"));
    }

    tokio::task::spawn_blocking(move || render_blocking(&app, &pdf_path, page_index))
        .await
        .map_err(|e| format!("PDF 渲染任务异常: {e}"))?
}

/// 同步渲染实现（在阻塞线程池执行）。
fn render_blocking(
    app: &tauri::AppHandle,
    pdf_path: &str,
    page_index: u16,
) -> Result<PdfPageImage, String> {
    with_pdfium(app, |pdfium| {
        let document = pdfium
            .load_pdf_from_file(pdf_path, None)
            .map_err(|e| format!("打开 PDF 失败: {e}"))?;

        let page_count = document.pages().len();
        let page_count_u16 = u16::try_from(page_count).unwrap_or(u16::MAX);
        if page_index >= page_count_u16 {
            return Err(format!(
                "页码越界：请求第 {} 页，文档共 {} 页",
                u32::from(page_index) + 1,
                page_count_u16
            ));
        }

        let page = document
            .pages()
            .get(i32::from(page_index))
            .map_err(|e| format!("读取第 {} 页失败: {e}", u32::from(page_index) + 1))?;

        // 按目标宽度等比缩放：保持宽高比，避免变形
        let config = PdfRenderConfig::new()
            .set_target_width(RENDER_TARGET_WIDTH)
            .render_form_data(false)
            .render_annotations(true);

        let bitmap = page
            .render_with_config(&config)
            .map_err(|e| format!("渲染第 {} 页失败: {e}", u32::from(page_index) + 1))?;

        let image = bitmap
            .as_image()
            .map_err(|e| format!("转换位图失败: {e}"))?;
        let (width, height) = (image.width(), image.height());

        // 临时目录：文件名带进程无关的固定前缀 + 页索引（同页重复渲染直接覆盖，
        // 避免临时目录堆积；不同文档通过 pdf 文件名哈希区分）
        let stem = Path::new(pdf_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "document".to_string());
        let hash = short_hash(pdf_path);
        let out_path = std::env::temp_dir().join(format!(
            "veloterm_pdf_{stem}_{hash}_p{page_index}.png"
        ));

        image
            .save_with_format(&out_path, image::ImageFormat::Png)
            .map_err(|e| format!("保存 PNG 失败（{}）: {e}", out_path.display()))?;
        // 登记：前端关闭预览时调 release_pdf_temp_files 删除
        register_temp_file(&out_path);

        log::debug!(
            "[PDF] rendered {} page {} -> {} ({}x{})",
            pdf_path,
            page_index,
            out_path.display(),
            width,
            height
        );

        Ok(PdfPageImage {
            path: out_path.to_string_lossy().into_owned(),
            page_count: page_count_u16,
            width,
            height,
        })
    })
}

/// 路径的短哈希（8 位十六进制）：区分同名不同路径的文档，避免临时文件互相覆盖。
fn short_hash(input: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:08x}", hasher.finish() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lib_file_name_is_platform_specific() {
        let name = lib_file_name();
        assert!(name.ends_with(".dylib") || name.ends_with(".dll"), "got {name}");
    }

    #[test]
    fn short_hash_is_stable_and_distinct() {
        let a = short_hash("/tmp/a.pdf");
        assert_eq!(a, short_hash("/tmp/a.pdf"), "same input must hash the same");
        assert_ne!(a, short_hash("/tmp/b.pdf"));
        assert_eq!(a.len(), 8);
    }
}
