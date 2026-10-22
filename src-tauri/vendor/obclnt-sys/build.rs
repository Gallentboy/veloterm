//! obclnt-sys 的构建接线：把预编译的 libobclient 静态库链进最终二进制。
//!
//! 本脚本**不编译任何东西**（gif2mp4-sys 同型）——档案从
//! `vendor/obclient-lib/<平台>/` 取（产物方式见该目录 README.md），这里
//! 只做两件事：
//! 1. 校验档案存在且为 ar/COFF 归档（缺失时早失败并给出指引，好过链接
//!    期满屏 UND）；
//! 2. 按平台发链接旗标：
//!    - macOS：`libobclnt.a` 自包含（上游 build.sh 的 .a 合并分支已并入
//!      zlib + OpenSSL 1.1.1u），仅链 obclnt 本体；
//!    - Windows：`obclnt.lib` 仅连接器对象（OpenSSL 不在内——Unix 才走
//!      合并分支），需补链 vcpkg 静态 ssl/crypto（`OPENSSL_DIR` 指向
//!      x64/arm64-windows-static 安装根，与 publish/test-obclient 车道
//!      同源）与系统库（ws2_32/advapi32/crypt32/shlwapi/user32/bcrypt——
//!      bcrypt 来自静态插件 caching_sha2/ob_sm3 的 CNG 引用）。
//!
//! 取档位：环境变量 `OBCLIENT_LIB_DIR`（整目录覆盖）→
//! `vendor/obclient-lib/<平台>/`（mac-aarch64 / mac-x86_64 / win-x64 /
//! win-arm64，按编译目标映射）。
//!
//! CRT 注记：上游把 MSVC 默认 /MD 全量替换为 /MT（与 vcpkg static
//! triplet 一致），最终若链入 /MD 的宿主会有 LNK4098 双 CRT 混链警告——
//! 与本仓库既有实践（libsqlite3-sys 的 sqlcipher + vcpkg 静态 OpenSSL）
//! 同型，先容忍；根治手段（宿主开 crt-static 或上游改回 /MD）另议。

use std::env;
use std::path::{Path, PathBuf};

/// 平台目录名（vendor/obclient-lib/ 下）
fn platform_dir_name(target_os: &str, target_arch: &str) -> &'static str {
    match (target_os, target_arch) {
        ("macos", "aarch64") => "mac-aarch64",
        ("macos", "x86_64") => "mac-x86_64",
        ("windows", "x86_64") => "win-x64",
        ("windows", "aarch64") => "win-arm64",
        _ => panic!(
            "[obclnt-sys] target {target_os}/{target_arch} 未配置（仅支持 macos/windows 四平台目录）"
        ),
    }
}

/// 归档魔数校验：ar 归档（macOS .a）与 MSVC COFF 静态库（.lib）的首行
/// 都是 `!<arch>\n`——缺失/截断/传成了 DLL 导入库以外的杂物时早失败
fn assert_archive(path: &Path) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        panic!(
            "[obclnt-sys] 静态档案缺失: {}\n  读取失败: {e}\n  请把档案提供到 vendor/obclient-lib/ 对应平台目录（产物方式见该目录 README.md），或用 OBCLIENT_LIB_DIR 指向整目录",
            path.display()
        )
    });
    if bytes.len() < 8 || &bytes[..8] != b"!<arch>\n" {
        panic!(
            "[obclnt-sys] {} 不是静态库归档（首 8 字节非 !<arch>\\n）——请检查是否误放了 DLL/导入库/文本文件",
            path.display()
        );
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

    let lib_dir = env::var("OBCLIENT_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            vendor_dir
                .join("obclient-lib")
                .join(platform_dir_name(&target_os, &target_arch))
        });
    println!("cargo:rerun-if-env-changed=OBCLIENT_LIB_DIR");
    println!("cargo:rerun-if-changed={}", lib_dir.display());

    // obclnt 本体：mac 为 libobclnt.a（自包含），win 为 obclnt.lib
    // （MSVC 静态库无 lib 前缀，见 test-obclient 车道产物断言）
    let (file_name, link_name) = if target_os == "macos" {
        ("libobclnt.a", "obclnt")
    } else {
        ("obclnt.lib", "obclnt")
    };
    let archive = lib_dir.join(file_name);
    assert_archive(&archive);
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static={link_name}");

    if target_os == "windows" {
        // Windows 的 obclnt.lib 不含 OpenSSL（上游只在 Unix 的 .a 合并分支
        // 并入 ssl/crypto，车道冒烟已实证需消费方自带）——经 OPENSSL_DIR
        // 取 vcpkg 静态库，与 publish 的 app 构建同一份约定
        println!("cargo:rerun-if-env-changed=OPENSSL_DIR");
        let openssl_dir = env::var("OPENSSL_DIR")
            .unwrap_or_else(|_| panic!(
                "[obclnt-sys] Windows 构建需 OPENSSL_DIR 指向 vcpkg {}-windows-static 安装根（obclnt.lib 未内置 OpenSSL，需补链静态 libssl/libcrypto）",
                if target_arch == "aarch64" { "arm64" } else { "x64" }
            ));
        let openssl_lib = PathBuf::from(&openssl_dir).join("lib");
        for name in ["libssl", "libcrypto"] {
            let path = openssl_lib.join(format!("{name}.lib"));
            assert_archive(&path);
        }
        println!("cargo:rustc-link-search=native={}", openssl_lib.display());
        // 链接序：依赖者在前——obclnt 已在上方先发，ssl/crypto 随后，
        // 系统库殿后（静态插件 caching_sha2/ob_sm3 的 BCrypt* 引用需 bcrypt）
        println!("cargo:rustc-link-lib=static=libssl");
        println!("cargo:rustc-link-lib=static=libcrypto");
        for sys in ["ws2_32", "advapi32", "crypt32", "shlwapi", "user32", "bcrypt"] {
            println!("cargo:rustc-link-lib=dylib={sys}");
        }
    }
}
