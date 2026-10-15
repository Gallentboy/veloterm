//! putty-telnet 构建脚本：把 PuTTY（MIT 许可）的 telnet 客户端以源码级
//! 现场编译并静态链接进本 crate，提供 `telnet` 命令。
//!
//! 仅 `target_os = windows` 时工作，其余平台直接 return（对齐工程内
//! vendor/openssl-app 的约定）。
//!
//! # 源码与版本（来源即事实）
//!
//! - vendored 源码树：`CARGO_MANIFEST_DIR/putty/`，来自
//!   `git clone --depth 1 https://git.tartarus.org/simon/putty.git`
//!   （PuTTY 官方仓库），master 快照
//!   `7507c54bf7172e5e3381887f8300a344207f60a3`（2026-10-14，
//!   LATEST.VER = 0.85 开发版）。许可证：MIT，全文见 putty/LICENCE。
//! - C 入口为本 crate 的 `c_src/telnet_main.c`（派生自 windows/plink.c，
//!   GNU/inetutils 风格参数解析在 C 层原生完成），导出
//!   `putty_telnet_main(int argc, char **argv)`。
//!
//! # 源文件集的推导依据（PuTTY 自己的构建清单，非凭空猜测）
//!
//! 该版 PuTTY 用 CMake 组建（根 CMakeLists.txt + windows/CMakeLists.txt +
//! cmake/setup.cmake 的 be_list 宏 + cmake/platforms/windows.cmake）：
//!
//! - plink 目标（CMakeLists.txt:135-148）= `windows/plink.c` +
//!   `stubs/no-lineedit.c` + be_list(SSH=1, SERIAL=1, OTHERBACKENDS=1) +
//!   链接库 { eventloop, noterminal, console, sshclient, otherbackends,
//!   settings, network, crypto, utils } + windows/CMakeLists.txt:91 追加
//!   { no-jump-list.c, nohelp.c, plink.rc }；platform_libraries =
//!   { advapi32 comdlg32 gdi32 imm32 ole32 shell32 user32 ws2_32
//!   kernel32 }（cmake/platforms/windows.cmake）。
//! - 本 crate 取「plink 的 console 驱动」+「telnet 后端」，剔除 SSH 与
//!   SERIAL：be_list 以 SSH=0 / SERIAL=0 / OTHERBACKENDS=1 编译
//!   （backends[] = telnet/rlogin/supdup/raw，be_default_protocol=
//!   PROT_TELNET，与 be_list.c 的 #if 组合一致）。
//! - 「无 SSH 但有全部 otherbackends」的选法不是自创：同清单里的
//!   puttytel 目标（CMakeLists.txt:214-232）就是这样的组合，其对 SSH
//!   依赖项的官方替身是 `stubs/no-gss.c`、`stubs/no-ca-config.c`、
//!   `stubs/no-rand.c`、`stubs/no-dit.c`、`proxy/nocproxy.c`、
//!   `proxy/nosshproxy.c`，并因此完全不链接 crypto/ 与 ssh/。本 crate
//!   照抄该替身组合（serial 以 SERIAL=0 剔除，no-dit 的空实现问题由
//!   telnet_main.c 去掉 enable_dit() 调用解决，见其头注释）。
//! - 各静态库按根 CMakeLists.txt 逐个展开：
//!   * eventloop  = callback.c timing.c + windows/{cliloop.c,
//!     handle-wait.c, handle-io.c}
//!   * noterminal = stubs/no-term.c ldisc.c + stubs/no-lineedit.c
//!     （plink 目标自带）
//!   * console    = clicons.c console.c + windows/{select-cli.c,
//!     nohelp.c, console.c}
//!   * otherbackends = otherbackends/{raw,rlogin,supdup,telnet,testback}.c
//!     （testback.c 在上游库里，保留以完全对齐）+ all-backends 的
//!     pinger.c + logging 对象库 { logging.c, utils/logeventf.c }
//!     （windows/serial.c 仅 SERIAL=1 时需要，剔除）
//!   * settings   = cmdline.c settings.c + windows/storage.c
//!   * network    = errsock.c x11disp.c + proxy/{proxy,http,socks4,
//!     socks5,telnet,newmain,local,interactor}.c + windows/{network.c,
//!     handle-socket.c,named-pipe-client.c,named-pipe-server.c,
//!     local-proxy.c,x11.c} + handle-io 对象
//!   * utils      = 根 utils/CMakeLists.txt 全表（编译时 glob，剔除
//!     ltime.c——那是 Unix 专用 ISO-C 版本，Windows 用 windows/utils/
//!     ltime.c，两者同名符号不能同编）+ windows/CMakeLists.txt 追加的
//!     windows/utils/*.c 全表（glob，剔除 strtoumax.c——MSVC 2013 起
//!     自带 strtoumax，上游 cmake 的 HAVE_STRTOUMAX 探测为真时不编它）
//!     + windows/unicode.c + stubs/null-*.c（stubs/CMakeLists.txt 把
//!     它们并入 utils）
//!   * sshclient / crypto：整体不取（SSH=0），其代理面替身见上。
//! - 额外生成：commitid（上游由 cmake/gitcommit.cmake 生成
//!  `cmake_commit.c`；此处写入 OUT_DIR，值为上面的 vendored commit）。
//!   未定义 HAVE_CMAKE_H，不生成/不需要 cmake.h（defs.h 的
//!  `#if HAVE_CMAKE_H` 走默认路径）。
//! - 编译定义按 cmake/platforms/windows.cmake：_WINDOWS、
//!   _CRT_SECURE_NO_WARNINGS、_WINSOCK_DEPRECATED_NO_WARNINGS、
//!   _ARM_WINAPI_PARTITION_DESKTOP_SDK_AVAILABLE；MSVC 的告警压制
//!   /wd4244 /wd4267 /wd4018 /wd4146 /wd4293 /wd4090 同上游（仅当编译器
//!   接受该 flag 时附加）。
//!
//! # 防御式发现
//!
//! 所有文件清单先逐个校验存在，缺失即 panic 并列出**全部已尝试路径**
//! 与缺失项；链接系统库照抄上游 platform_libraries 全表。CI 首跑若报
//! 缺符号/缺库，依据 = 上游 windows/CMakeLists.txt 与本文件清单的差集。

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// 统一日志前缀
const TAG: &str = "[putty-telnet]";
/// vendored PuTTY 的来源 commit（git.tartarus.org/simon/putty.git master）
const PUTTY_COMMIT: &str = "7507c54bf7172e5e3381887f8300a344207f60a3";

/// 根目录 / proxy / otherbackends / stubs 的显式源文件（相对 putty/）
/// ——全部取自上游 CMakeLists 的对应库展开，见模块头注释。
const ROOT_SOURCES: &[&str] = &[
    "callback.c",   // eventloop
    "timing.c",     // eventloop
    "clicons.c",    // console
    "console.c",    // console
    "ldisc.c",      // noterminal
    "settings.c",   // settings
    "errsock.c",    // network
    "logging.c",    // logging 对象库
    "pinger.c",     // all-backends 对象库
    "x11disp.c",    // network
];
const PROXY_SOURCES: &[&str] = &[
    "proxy/proxy.c",
    "proxy/http.c",
    "proxy/socks4.c",
    "proxy/socks5.c",
    "proxy/telnet.c",
    "proxy/newmain.c",
    "proxy/local.c",
    "proxy/interactor.c",
    // 无 SSH 构建的官方替身（puttytel/pterm 同款）：代理协议本身保留，
    // 仅 SOCKS5 CHAP / HTTP DIGEST 认证与「经 SSH 的代理」不可用。
    "proxy/nocproxy.c",
    "proxy/nosshproxy.c",
];
const OTHERBACKENDS_SOURCES: &[&str] = &[
    "otherbackends/telnet.c",
    "otherbackends/rlogin.c",
    "otherbackends/supdup.c",
    "otherbackends/raw.c",
    "otherbackends/testback.c",
];
const STUB_SOURCES: &[&str] = &[
    "stubs/no-term.c",      // noterminal：无 terminal 的 seat/ldisc 面
    "stubs/no-lineedit.c",  // plink 目标自带：ldisc.c 的行编辑引用
    "stubs/no-gss.c",       // puttytel 同款：settings.c 的 gsslibkeywords
    "stubs/no-ca-config.c", // puttytel 同款：无 SSH 的 CA 配置框替身
    "stubs/no-rand.c",      // puttytel 同款：noise_ultralight / random_*
];
const WINDOWS_SOURCES: &[&str] = &[
    "windows/console.c",           // console：cleanup_exit / 提示 I/O
    "windows/select-cli.c",        // console：winselcli_setup
    "windows/nohelp.c",            // console：has_embedded_chm
    "windows/cliloop.c",           // eventloop：cli_main_loop
    "windows/handle-wait.c",       // eventloop
    "windows/handle-io.c",         // eventloop+network 共享对象
    "windows/network.c",           // network：winnet 世代（sk_init 等）
    "windows/handle-socket.c",     // network
    "windows/named-pipe-client.c", // network
    "windows/named-pipe-server.c", // network
    "windows/local-proxy.c",       // network
    "windows/x11.c",               // network
    "windows/storage.c",           // settings：注册表 do_defaults
    "windows/no-jump-list.c",      // plink 目标自带
    "windows/unicode.c",           // utils 追加
];

fn main() {
    // 用 CARGO_CFG_TARGET_OS（而非 HOST）判断，保证 canadian 交叉也正确
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=c_src/");
    if target_os != "windows" {
        println!("{TAG} 非 Windows target（{target_os}），按约定早退，不编译任何东西");
        return;
    }
    if let Err(msg) = run() {
        panic!(
            "{TAG} 构建失败：{msg}\n{TAG} 提示：以上日志列出了每一步校验过的路径；\
             若为链接期缺符号，请对照上游 windows/CMakeLists.txt 与本 build.rs \
             头注释里的库展开表补源文件；若为缺系统库，对照上游 \
             cmake/platforms/windows.cmake 的 platform_libraries。"
        );
    }
}

fn run() -> Result<(), String> {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let putty = manifest.join("putty");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // ── 1. 校验 vendored PuTTY 源码树 ────────────────────────────────────
    let markers = [
        "putty.h",
        "LICENCE",
        "be_list.c",
        "cmdline.c",
        "settings.c",
        "console.c",
        "windows/plink.c",
        "windows/console.c",
        "windows/network.c",
        "windows/storage.c",
        "otherbackends/telnet.c",
        "proxy/proxy.c",
        "stubs/no-gss.c",
    ];
    let mut missing = Vec::new();
    for m in markers {
        if !putty.join(m).is_file() {
            missing.push(putty.join(m).display().to_string());
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "vendored PuTTY 源码树不完整（{}），缺失：{}",
            putty.display(),
            missing.join("、")
        ));
    }
    println!("{TAG} vendored PuTTY 源码树 = {}（commit {PUTTY_COMMIT}）", putty.display());

    // ── 2. 显式清单逐个校验存在 ─────────────────────────────────────────
    let mut sources: Vec<PathBuf> = Vec::new();
    for rel in ROOT_SOURCES.iter().chain(PROXY_SOURCES).chain(OTHERBACKENDS_SOURCES).chain(STUB_SOURCES).chain(WINDOWS_SOURCES) {
        let p = putty.join(rel);
        if !p.is_file() {
            return Err(format!(
                "清单文件缺失：{}（来源：上游 CMakeLists 对应库展开，见 build.rs 头注释）",
                p.display()
            ));
        }
        sources.push(p);
    }

    // ── 3. utils 两张表：glob + 定向剔除 ────────────────────────────────
    // 根 utils/（utils/CMakeLists.txt 全表；logeventf.c 属 logging 对象库，
    // 由本 crate 一并编译一次）。剔除 ltime.c：Unix 专用 ISO-C 版本，
    // Windows 链接的是 windows/utils/ltime.c，同名符号 ltime() 不能同编。
    let utils_srcs = glob_c_files(&putty.join("utils"), &["ltime.c"])?;
    // windows/utils/（windows/CMakeLists.txt 的 utils 追加全表）。
    // 剔除 strtoumax.c：上游仅在 !HAVE_STRTOUMAX 时编译；MSVC 2013+ 自带。
    // 若 CI 报 strtoumax 未定义（极老工具链），把它加回清单即可。
    let win_utils_srcs = glob_c_files(&putty.join("windows").join("utils"), &["strtoumax.c"])?;
    // stubs/null-*.c：stubs/CMakeLists.txt 并入 utils 的空实现
    let null_srcs = glob_c_files(&putty.join("stubs"), &[])?;
    let null_srcs: Vec<PathBuf> = null_srcs
        .into_iter()
        .filter(|p| p.file_name().unwrap().to_str().unwrap().starts_with("null-"))
        .collect();
    println!(
        "{TAG} utils：根 {} 个 + windows {} 个 + stubs/null {} 个",
        utils_srcs.len(),
        win_utils_srcs.len(),
        null_srcs.len()
    );
    sources.extend(utils_srcs);
    sources.extend(win_utils_srcs);
    sources.extend(null_srcs);

    // ── 4. 生成 commitid（上游 cmake_commit.c 的等价物）─────────────────
    let commit_c = out_dir.join("veloterm_commit.c");
    fs::write(
        &commit_c,
        format!(
            "/* 由 putty-telnet build.rs 生成：上游 cmake/gitcommit.cmake 的等价物 */\n\
             #include \"putty.h\"\n\
             const char commitid[] = \"{PUTTY_COMMIT}\";\n"
        ),
    )
    .map_err(|e| format!("写 {} 失败：{e}", commit_c.display()))?;
    println!("{TAG} 生成 {}（commitid={PUTTY_COMMIT}）", commit_c.display());
    sources.push(commit_c);

    // ── 5. 本 crate 的 C 入口 ───────────────────────────────────────────
    let telnet_main = manifest.join("c_src").join("telnet_main.c");
    if !telnet_main.is_file() {
        return Err(format!("{} 不存在", telnet_main.display()));
    }
    sources.push(telnet_main);

    println!("{TAG} 待编译源文件共 {} 个（不含 be_list.c）", sources.len());

    // ── 6. cc 编译 ───────────────────────────────────────────────────────
    // include 路径 = 上游根 CMakeLists 的 include_directories()：
    // putty 根、生成物目录（OUT_DIR）、windows/，另加 terminal/
    //（ldisc.c / stubs/no-term.c / stubs/no-lineedit.c include "terminal.h"）。
    let includes = [
        putty.clone(),
        out_dir.clone(),
        putty.join("windows"),
        putty.join("terminal"),
    ];

    // be_list.c 单独编译：SSH/SERIAL/OTHERBACKENDS/APPNAME 是上游按
    // 「每个程序一份 be_list 对象」给出的命令行宏（cmake/setup.cmake
    // 的 be_list() 函数），只作用于这一个翻译单元。
    let mut belist = cc::Build::new();
    belist
        .file(putty.join("be_list.c"))
        .define("APPNAME", "telnet")
        .define("SSH", "0")
        .define("SERIAL", "0")
        .define("OTHERBACKENDS", "1")
        .define("_WINDOWS", None)
        .define("_CRT_SECURE_NO_WARNINGS", None)
        .define("_WINSOCK_DEPRECATED_NO_WARNINGS", None)
        .define("_ARM_WINAPI_PARTITION_DESKTOP_SDK_AVAILABLE", None);
    for inc in &includes {
        belist.include(inc);
    }
    silence_msvc_warnings(&mut belist);
    println!("{TAG} be_list.c：APPNAME=telnet SSH=0 SERIAL=0 OTHERBACKENDS=1");
    belist.compile("veloterm_putty_belist");

    let mut build = cc::Build::new();
    build
        .define("_WINDOWS", None)
        .define("_CRT_SECURE_NO_WARNINGS", None)
        .define("_WINSOCK_DEPRECATED_NO_WARNINGS", None)
        .define("_ARM_WINAPI_PARTITION_DESKTOP_SDK_AVAILABLE", None);
    for inc in &includes {
        println!("{TAG} include += {}", inc.display());
        build.include(inc);
    }
    for f in &sources {
        build.file(f);
    }
    silence_msvc_warnings(&mut build);
    build.compile("veloterm_putty_telnet");

    // ── 7. 链接指令 ─────────────────────────────────────────────────────
    // 照抄上游 cmake/platforms/windows.cmake 的 platform_libraries 全表。
    // （PuTTY 不用 crypt32/secur32——get_username/security 模块对它们是
    // LoadLibrary 动态加载，不产生链接期依赖。）
    for sys in [
        "advapi32", "comdlg32", "gdi32", "imm32", "ole32", "shell32", "user32", "ws2_32",
        "kernel32",
    ] {
        println!("cargo:rustc-link-lib={sys}");
    }
    Ok(())
}

/// glob 某目录下的 *.c 并按 exclude（按文件名）剔除；目录为空/缺失即报错
fn glob_c_files(dir: &Path, exclude: &[&str]) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let rd = fs::read_dir(dir).map_err(|e| format!("读取目录 {} 失败：{e}", dir.display()))?;
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("c") {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if exclude.iter().any(|x| *x == name) {
                println!("{TAG} 跳过 {} （排除名单：Unix/老工具链专用）", p.display());
                continue;
            }
            out.push(p);
        }
    }
    out.sort();
    if out.is_empty() {
        return Err(format!(
            "目录 {} 下没有可编译的 .c 文件（排除名单：{:?}）",
            dir.display(),
            exclude
        ));
    }
    Ok(out)
}

/// 上游 windows.cmake 对 MSVC 的告警压制（C4244/4267/4018/4146/4293/4090），
/// 仅当编译器接受该 flag 时附加（cc 会现场探测，非 MSVC 工具链自动跳过）
fn silence_msvc_warnings(build: &mut cc::Build) {
    for flag in [
        "/wd4244", "/wd4267", "/wd4018", "/wd4146", "/wd4293", "/wd4090",
    ] {
        if build.is_flag_supported(flag).unwrap_or(false) {
            build.flag(flag);
        }
    }
}
