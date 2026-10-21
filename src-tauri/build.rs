fn main() {
    tauri_build::build();
    link_static_pdfium_if_needed();
}

/// PDFium 静态链接接线（仅 macOS Apple Silicon）。
///
/// 其他平台（Windows 全架构、macOS Intel）走动态加载，本函数直接返回——
/// pdf.rs 里用对称的 `cfg` 选择 `bind_to_library` / `bind_to_statically_linked_library`。
///
/// 三个档案必须一起链：Chromium 自编的 libcxx/libcxxabi 使用 `__Cr` namespace，
/// 与系统 libc++ 的符号互不兼容，缺一个就是几百个未定义符号。
///
/// 用 `rustc-link-arg` 而非 `rustc-link-lib`：后者只作用于当前 crate 的库目标，
/// 集成测试/示例等独立二进制拿不到 `-l`（实测会报满屏 FPDF 符号 UND）。
fn link_static_pdfium_if_needed() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if target_os != "macos" || target_arch != "aarch64" {
        return;
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let vendor_dir = std::path::Path::new(&manifest_dir)
        .join("vendor")
        .join("pdfium");

    // 档案校验：必须是 regular archive（`!<arch>\n` 魔数）。Chromium 的 ninja
    // 默认产 thin archive，Apple ld 不认——早失败好过链接期满屏 UND 符号。
    for name in ["libpdfium.a", "libcxx.a", "libcxxabi.a"] {
        let path = vendor_dir.join(name);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            eprintln!(
                "[pdfium] 静态档案读取失败: {} ({e})\n  期望位置: {}",
                name,
                path.display()
            );
            std::process::exit(1);
        });
        if !bytes.starts_with(b"!<arch>\n") {
            eprintln!(
                "[pdfium] {} 不是 regular archive（可能是 thin archive）——用 `ar rcs` 重打",
                path.display()
            );
            std::process::exit(1);
        }
        println!("cargo:rerun-if-changed={}", path.display());
    }

    println!("cargo:rustc-link-arg=-L{}", vendor_dir.display());
    // 顺序即符号解析顺序：pdfium 的未定义符号由后面的 libcxx/libcxxabi 满足
    println!("cargo:rustc-link-arg=-lpdfium");
    println!("cargo:rustc-link-arg=-lcxx");
    println!("cargo:rustc-link-arg=-lcxxabi");
    // 系统框架：PDFium 用到 CoreGraphics（位图/颜色空间）、CoreFoundation、
    // Security（签名校验路径）。缺任一个链接期即报未定义符号。
    for framework in ["CoreGraphics", "CoreFoundation", "Security"] {
        println!("cargo:rustc-link-arg=-framework");
        println!("cargo:rustc-link-arg={framework}");
    }
    // C++ 运行时：libcxxabi 依赖系统的 libc++ 头/运行时补丁符号
    println!("cargo:rustc-link-arg=-lc++");
}
