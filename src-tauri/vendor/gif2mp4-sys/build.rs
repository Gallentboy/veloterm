//! gif2mp4-sys 的构建接线：把预编译的静态库族链进最终二进制。
//!
//! 本脚本**不编译任何东西**——档案从 `vendor/ffmpeg-lib/<平台>/` 取
//! （8 件齐全：libgif2mp4 + 五个 libav*/libsw* + libx264；产物方式与
//! 目录布局见该目录 README.md），这里只做两件事：
//! 1. 校验档案存在且为 ar 归档（缺失时早失败并给出指引，好过链接期
//!    满屏 UND）；
//! 2. 按依赖序（gif2mp4 在前、avutil 殿后、x264 压轴）发链接旗标。
//!
//! 取档位：环境变量 `GIF2MP4_LIB_DIR`（整目录覆盖）→
//! `vendor/ffmpeg-lib/<平台>/`（mac-aarch64 / mac-x86_64 / win-x64 /
//! win-arm64，按编译目标映射）。不做树内兜底——预编译档案是唯一来源。

use std::env;
use std::path::{Path, PathBuf};

/// 链接序 = 依赖者在前（与命令行链接 gif2mp4_cli 时一致）；x264 压轴
const ALL_LIBS: &[&str] = &[
    "gif2mp4",
    "avfilter",
    "swscale",
    "avformat",
    "avcodec",
    "swresample",
    "avutil",
    "x264",
];

/// 平台目录名（vendor/ffmpeg-lib/ 下）
fn platform_dir_name(target_os: &str, target_arch: &str) -> &'static str {
    match (target_os, target_arch) {
        ("macos", "aarch64") => "mac-aarch64",
        ("macos", "x86_64") => "mac-x86_64",
        ("windows", "x86_64") => "win-x64",
        ("windows", "aarch64") => "win-arm64",
        _ => panic!(
            "[gif2mp4-sys] target {target_os}/{target_arch} 未配置（仅支持 macos/windows 四平台目录）"
        ),
    }
}

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let vendor_dir = PathBuf::from(&manifest_dir)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let lib_dir = env::var("GIF2MP4_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| vendor_dir.join("ffmpeg-lib").join(platform_dir_name(&target_os, &target_arch)));

    for name in ALL_LIBS {
        let path = archive_path(&lib_dir, name, &target_os);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "[gif2mp4-sys] 静态档案缺失: {}\n  读取失败: {e}\n  请把 8 件档案提供到 vendor/ffmpeg-lib/{}/（产物命令见该目录 README.md）\n  或用 GIF2MP4_LIB_DIR 指向整目录",
                path.display(),
                platform_dir_name(&target_os, &target_arch)
            )
        });
        // ar 归档魔数（Windows .lib 与 macOS .a 同为 ar 归档族）
        if !bytes.starts_with(b"!<arch>\n") {
            panic!(
                "[gif2mp4-sys] 档案魔数异常（期望 ar 归档）: {}",
                path.display()
            );
        }
    }

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    for name in ALL_LIBS {
        println!("cargo:rustc-link-lib=static={name}");
    }

    match target_os.as_str() {
        "macos" => {
            println!("cargo:rustc-link-lib=dylib=pthread");
            println!("cargo:rustc-link-lib=dylib=m");
        }
        "windows" => {
            // ffmpeg 在 Windows 下依赖的系统库（configure 的 check_lib 探测项，
            // 见 vendor/ffmpeg/configure:6997-7002 与 avutil_suggest）。
            // 缺任何一个都会在链接期报一堆 UND 符号（BCryptGenRandom /
            // CoTaskMemFree / CommandLineToArgvW / RegCloseKey / GetProcessMemoryInfo）。
            for sys_lib in [
                "bcrypt",    // BCryptGenRandom（avutil 随机数）
                "ole32",     // CoTaskMemFree（COM 内存）
                "advapi32",  // RegCloseKey（注册表）
                "shell32",   // CommandLineToArgvW
                "user32",    // MessageBox 等（avutil_suggest 列出）
                "psapi",     // GetProcessMemoryInfo
                "ws2_32",    // winsock（网络组件虽裁剪，头文件探测仍可能引用）
                "secur32",   // schannel 相关（TLS 组件裁剪后的残留引用）
                "gdi32",     // 位图/显示相关
                "winmm",     // 多媒体定时器
            ] {
                println!("cargo:rustc-link-lib=dylib={sys_lib}");
            }
        }
        other => panic!("[gif2mp4-sys] 未处理的 target os: {other}"),
    }

    for name in ALL_LIBS {
        println!(
            "cargo:rerun-if-changed={}",
            archive_path(&lib_dir, name, &target_os).display()
        );
    }
    println!("cargo:rerun-if-env-changed=GIF2MP4_LIB_DIR");
}

/// 平台的档案命名（macOS: lib<name>.a；Windows MSVC: <name>.lib）
fn archive_path(dir: &Path, name: &str, target_os: &str) -> PathBuf {
    if target_os == "windows" {
        dir.join(format!("{name}.lib"))
    } else {
        dir.join(format!("lib{name}.a"))
    }
}
