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

/// 生成下载用的本地临时路径并登记（前端下载远端 PDF/图片时调用）。
///
/// 只负责分配路径与登记，实际下载由既有的 `sftp_download_file` 完成
/// （它已有进度 Channel、取消、续传）。
///
/// `kind`：文件名前缀，用于区分 PDF 源文件（`pdf_src`）与图片源文件（`img_src`）。
#[tauri::command]
pub fn pdf_temp_path(file_name: String, kind: Option<String>) -> Result<String, String> {
    // 只取文件名部分，防止前端传入带目录分隔符的路径逃逸出临时目录
    let safe_name = Path::new(&file_name)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "无效的文件名".to_string())?;
    let prefix = match kind.as_deref() {
        Some("img") => "veloterm_img_src",
        _ => "veloterm_pdf_src",
    };
    let path = std::env::temp_dir().join(format!("{prefix}_{safe_name}"));
    register_temp_file(&path);
    Ok(path.to_string_lossy().into_owned())
}

/// 图片转码结果
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePreviewResult {
    /// 可直接给 `<img src>` 的临时文件路径（原生支持的格式原样返回源文件）
    pub path: String,
    /// 是否经过后端转码（false = 浏览器原生可显示，直接用的源文件）
    pub converted: bool,
    pub width: u32,
    pub height: u32,
}

/// 浏览器原生支持的图片扩展名（WebKit / WebView2 均可直接渲染）。
///
/// 这些格式**不转码**——直接返回源文件路径，由 WebView 原生解码
/// （GPU 加速、流式读取，比后端解码再编码 PNG 快得多，也不占 Rust 内存）。
const NATIVE_IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "jfif", "pjpeg", "pjp", "gif", "webp", "svg", "bmp", "ico", "avif",
    "apng",
];

/// 已知但当前无法支持的格式（给出明确原因，而不是笼统的「不支持」）
fn unsupported_reason(ext: &str) -> Option<&'static str> {
    match ext {
        "heic" | "heif" => Some("HEIC/HEIF 需要 libheif 解码器（当前构建未启用）"),
        "jp2" | "j2k" | "jpf" | "jpx" => Some("JPEG 2000 需要 openjpeg 解码器（当前构建未启用）"),
        "psd" | "psb" => Some("Photoshop 文档（PSD）暂不支持"),
        "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "raf" | "rw2" => {
            Some("相机 RAW 格式需要 libraw（当前构建未启用）")
        }
        _ => None,
    }
}

/// JPEG XL 解码：jpegxl-rs（libjxl 绑定）→ RGBA8 缓冲 → image 的 RgbaImage。
///
/// **为什么不用 jpegxl-rs 的 `image` feature**：它要求 image `^0.25,<0.25.9`，
/// 与本项目（及 onefetch）锁定的 0.25.10 冲突。这里直接用它的 `decode_with::<u8>()`
/// 拿原始像素，自己用 image 0.25 组图并编码 PNG。
///
/// 像素格式：jxl 可能是灰度/灰度+alpha/RGB/RGBA，`decode_with` 返回交错排列的
/// u8 数据；按 metadata 的通道数决定如何组装成 RGBA。
fn decode_jxl_to_rgba(
    src: &Path,
) -> Result<(image::DynamicImage, u32, u32), String> {
    let bytes = std::fs::read(src).map_err(|e| format!("读取 JXL 文件失败: {e}"))?;
    let (metadata, pixels) = jpegxl_rs::decoder_builder()
        .build()
        .map_err(|e| format!("初始化 JXL 解码器失败: {e}"))?
        .decode_with::<u8>(&bytes)
        .map_err(|e| format!("JXL 解码失败: {e}"))?;

    let (width, height) = (metadata.width, metadata.height);
    if width == 0 || height == 0 {
        return Err("JXL 图像尺寸无效（宽或高为 0）".to_string());
    }
    let channels = metadata.num_color_channels
        + u32::from(metadata.has_alpha_channel);
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(channels as usize))
        .ok_or_else(|| "JXL 图像尺寸过大".to_string())?;
    if pixels.len() < expected {
        return Err(format!(
            "JXL 像素数据不足：期望 {expected} 字节，实际 {}",
            pixels.len()
        ));
    }

    // 逐像素组装成 RGBA（灰度的 R=G=B，无 alpha 时 A=255）
    let mut rgba = Vec::with_capacity((width as usize) * (height as usize) * 4);
    let px_count = (width as usize) * (height as usize);
    for i in 0..px_count {
        let base = i * channels as usize;
        let (r, g, b, a) = match channels {
            1 => {
                let v = pixels.get(base).copied().unwrap_or(0);
                (v, v, v, 255)
            }
            2 => {
                let v = pixels.get(base).copied().unwrap_or(0);
                let a = pixels.get(base + 1).copied().unwrap_or(255);
                (v, v, v, a)
            }
            3 => (
                pixels.get(base).copied().unwrap_or(0),
                pixels.get(base + 1).copied().unwrap_or(0),
                pixels.get(base + 2).copied().unwrap_or(0),
                255,
            ),
            _ => (
                pixels.get(base).copied().unwrap_or(0),
                pixels.get(base + 1).copied().unwrap_or(0),
                pixels.get(base + 2).copied().unwrap_or(0),
                pixels.get(base + 3).copied().unwrap_or(255),
            ),
        };
        rgba.extend_from_slice(&[r, g, b, a]);
    }

    let img = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| "JXL 像素缓冲尺寸不匹配".to_string())?;
    Ok((image::DynamicImage::ImageRgba8(img), width, height))
}

/// 准备图片预览：原生格式直传，其余用 image crate 转码为 PNG。
///
/// 返回可直接给 `<img src>` 的本地路径（前端用 convertFileSrc 转 asset:// URL）。
/// 临时文件登记在 PDF_TEMP_FILES 里，由 `release_pdf_temp_files` 统一清理。
#[tauri::command]
pub async fn prepare_image_preview(source_path: String) -> Result<ImagePreviewResult, String> {
    let src = PathBuf::from(&source_path);
    if !src.is_file() {
        return Err(format!("图片文件不存在: {source_path}"));
    }
    let ext = src
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    // 原生支持：不转码，直接返回源文件（WebView 自己解码，最快）
    if NATIVE_IMAGE_EXTS.contains(&ext.as_str()) {
        // 仍读取尺寸供前端占位（避免加载时布局抖动）；读失败不影响预览
        let (width, height) = tokio::task::spawn_blocking({
            let p = src.clone();
            move || image::image_dimensions(&p).unwrap_or((0, 0))
        })
        .await
        .unwrap_or((0, 0));
        return Ok(ImagePreviewResult {
            path: source_path,
            converted: false,
            width,
            height,
        });
    }

    // 明确不支持的格式：给出具体原因
    if let Some(reason) = unsupported_reason(&ext) {
        return Err(format!("无法预览 .{ext} 图片：{reason}"));
    }

    // 其余交给 image crate 解码；它自己按魔数嗅探格式，扩展名只用于报错信息。
    // JPEG XL 例外：image crate 不支持，走 jpegxl-rs（见下）。
    let out_path = std::env::temp_dir().join(format!(
        "veloterm_img_{}_preview.png",
        short_hash(&source_path)
    ));
    let is_jxl = matches!(ext.as_str(), "jxl");
    let result = tokio::task::spawn_blocking({
        let src = src.clone();
        let out = out_path.clone();
        move || -> Result<(u32, u32), String> {
            let (img, w, h) = if is_jxl {
                decode_jxl_to_rgba(&src)?
            } else {
                let img = image::open(&src).map_err(|e| {
                    format!(
                        "解码失败（.{}）：{e}。该格式可能不在支持列表内（支持：TIFF/TGA/DDS/HDR/EXR/PNM/QOI/JXL 等）",
                        src.extension().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default()
                    )
                })?;
                let (w, h) = (img.width(), img.height());
                (img, w, h)
            };
            img.save_with_format(&out, image::ImageFormat::Png)
                .map_err(|e| format!("保存 PNG 失败: {e}"))?;
            Ok((w, h))
        }
    })
    .await
    .map_err(|e| format!("图片转码任务异常: {e}"))??;

    register_temp_file(&out_path);
    log::debug!(
        "[Image] converted {} -> {} ({}x{})",
        source_path,
        out_path.display(),
        result.0,
        result.1
    );
    Ok(ImagePreviewResult {
        path: out_path.to_string_lossy().into_owned(),
        converted: true,
        width: result.0,
        height: result.1,
    })
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

/// 开发期 vendor 目录里的库文件名（带架构后缀，仓库内区分多份产物）。
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn vendor_lib_file_name() -> &'static str {
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

/// 随包分发的标准库名（无架构后缀）。
///
/// 架构由安装包本身决定；源文件的 -arm64/-x64 只是仓库内区分手段。
/// build.rs 的 prepare_pdfium_resource 按 target 拷对应库并重命名为这个名字。
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn packaged_lib_file_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "libpdfium.dylib"
    }
    #[cfg(target_os = "windows")]
    {
        "pdfium.dll"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "libpdfium.dylib"
    }
}

/// 解析 PDFium 动态库的绝对路径。
///
/// **debug / release 两种查找策略**：
///
/// - **debug（开发期）**：直接用仓库 `vendor/pdfium/` 下的源码树文件。开发时
///   只改代码不改库，固定路径最省心，也不受 target/debug 下资源拷贝的影响。
/// - **release（发布）**：按各平台第三方库的标准位置查找——
///   macOS：App Bundle 的 `Contents/Frameworks/`（Tauri 打包 resources 的落点）
///   → `Contents/Resources/` → `/usr/local/lib` → `/opt/homebrew/lib`
///   （Intel Mac 常见前缀）→ 可执行文件同级；
///   Windows：可执行文件同级 → 同级 `lib/`（本项目 DLL 统一约定）→ resources。
///
/// 两条路径都保留 resource_dir 兜底（Tauri 资源映射位置可能随配置变化）。
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn resolve_pdfium_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    // ── debug：固定用源码树 vendor 目录（带架构后缀的原名） ──
    #[cfg(debug_assertions)]
    {
        let _ = app; // debug 分支不需要 AppHandle
        let name = vendor_lib_file_name();
        let dev_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("vendor")
            .join("pdfium")
            .join(name);
        return if dev_path.is_file() {
            Ok(dev_path)
        } else {
            Err(format!(
                "未找到 PDFium 动态库（debug 模式查找 {}）。\
                 请确认 vendor/pdfium/ 下存在对应架构的库文件",
                dev_path.display()
            ))
        };
    }

    // ── release：按平台标准路径查找（随包文件用标准名，见 build.rs） ──
    #[cfg(not(debug_assertions))]
    {
        let name = packaged_lib_file_name();
        use tauri::Manager;
        let mut tried: Vec<PathBuf> = Vec::new();

        #[cfg(target_os = "macos")]
        {
            // App Bundle：可执行文件在 Contents/MacOS/，Frameworks 与 Resources 是同级
            if let Ok(exe) = std::env::current_exe()
                && let Some(contents) = exe.parent().and_then(|p| p.parent())
            {
                for dir in [contents.join("Frameworks"), contents.join("Resources")] {
                    for candidate in [dir.join(name), dir.join("pdfium").join(name)] {
                        if candidate.is_file() {
                            return Ok(candidate);
                        }
                        tried.push(candidate);
                    }
                }
            }
            // 系统/包管理器标准前缀（用户自行安装 PDFium 的场景）
            for dir in ["/usr/local/lib", "/opt/homebrew/lib", "/usr/lib"] {
                let candidate = Path::new(dir).join(name);
                if candidate.is_file() {
                    return Ok(candidate);
                }
                tried.push(candidate);
            }
        }

        #[cfg(target_os = "windows")]
        {
            if let Ok(exe) = std::env::current_exe()
                && let Some(parent) = exe.parent()
            {
                // 本项目约定：随包 DLL 统一放安装根 lib/ 子目录
                for candidate in [parent.join(name), parent.join("lib").join(name)] {
                    if candidate.is_file() {
                        return Ok(candidate);
                    }
                    tried.push(candidate);
                }
            }
        }

        // Tauri resources 兜底（映射位置可能随配置调整）
        if let Ok(dir) = app.path().resource_dir() {
            for candidate in [dir.join("pdfium").join(name), dir.join(name)] {
                if candidate.is_file() {
                    return Ok(candidate);
                }
                tried.push(candidate);
            }
        }

        Err(format!(
            "未找到 PDFium 动态库 {name}（release 模式已查找：{}）",
            tried
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("、")
        ))
    }
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

/// 绑定 PDFium（按平台分流）。
///
/// - macOS Apple Silicon：静态链接形态（build.rs 已把 libpdfium.a + libcxx +
///   libcxxabi 链进二进制），直接取静态绑定，无需任何库文件路径。
/// - 其他平台：动态加载 vendor 下的 dylib/dll。
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn bind_pdfium(_app: &tauri::AppHandle) -> Result<Pdfium, String> {
    let bindings = Pdfium::bind_to_statically_linked_library()
        .map_err(|e| format!("初始化静态链接的 PDFium 失败: {e}"))?;
    log::info!("[PDF] PDFium statically linked");
    Ok(Pdfium::new(bindings))
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
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

    /// JXL 解码器能初始化 = libjxl 动态库加载成功（本机 brew 安装的 0.12）
    #[test]
    fn jxl_decoder_initializes() {
        // build() 会加载 libjxl 并创建解码器实例；失败说明动态库缺失/版本不兼容
        let decoder = jpegxl_rs::decoder_builder().build();
        assert!(decoder.is_ok(), "libjxl 加载失败: {:?}", decoder.err());
    }

    /// 库文件名：开发期带架构后缀，随包用标准名（静态链接的 mac arm64 无此函数）
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn lib_file_names_are_correct() {
        let vendor = vendor_lib_file_name();
        assert!(
            vendor.ends_with(".dylib") || vendor.ends_with(".dll"),
            "vendor name: {vendor}"
        );
        // 随包名必须是标准名（不带架构后缀）：架构由安装包本身决定
        let packaged = packaged_lib_file_name();
        assert!(
            packaged == "libpdfium.dylib" || packaged == "pdfium.dll",
            "packaged name must be the standard library name, got {packaged}"
        );
    }

    #[test]
    fn short_hash_is_stable_and_distinct() {
        let a = short_hash("/tmp/a.pdf");
        assert_eq!(a, short_hash("/tmp/a.pdf"), "same input must hash the same");
        assert_ne!(a, short_hash("/tmp/b.pdf"));
        assert_eq!(a.len(), 8);
    }
}
