//! coreutils 多呼叫二进制的分发核心（BusyBox 模式）。
//!
//! 单一 `coreutils` 可执行文件承载全部 GNU coreutils 命令，两种调用方式：
//! 1. **argv[0] 分发**：安装时在 `tools/` 目录创建指向本二进制的硬链接
//!    （`ls.exe` `grep.exe`…，由主程序启动钩子 `src/tools/hardlinks.rs` 建立），
//!    PATH 注入后用户敲短名，进程以对应名字启动，按 argv[0] 路由到命令实现；
//! 2. **子命令分发**：直接执行 `coreutils <cmd> [args...]`。
//!
//! 命令名清单（哪些命令存在）在零依赖的 `tools-catalog` crate —— 主程序只依赖
//! 清单，不依赖本 crate，从而不链接 uutils/OpenSSL/PuTTY 整张依赖图。
//!
//! 命令实现取自工程内 vendor 的 uutils/coreutils 源码（tag 0.12.0，MIT，
//! 见 `src-tauri/vendor/coreutils`），分组与 uutils 官方 feature 对齐；
//! 家族补充：grep/sed（uutils/grep、uutils/sed）、find/xargs（uutils/findutils）、
//! diff/cmp/sdiff（uutils/diffutils）；现代替代品：rg（ripgrep）、fd（fd-find）、
//! sd（chmln/sd）、awk（frawk，Cranelift JIT）、parallel（rust-parallel，tokio
//! 并行执行器，GNU parallel 风格接口）、bat（sharkdp/bat 0.26.1，语法高亮
//! cat，syntect/fancy-regex 纯 Rust 后端）、onefetch（o2sh/onefetch，git 仓库
//! 信息面板，gix/tokei 纯 Rust 栈）、tree/eza（eza）、wget（wget-rs，
//! rustls）；tar 由 tar-rs 库 + 自研 CLI 壳（tar_impl.rs）实现；
//! base32hex/base64url/hex/hexupper 由 data-encoding 库 + 自研 CLI 壳
//! （encodings_impl.rs）实现，仅注册进 Windows 清单（见 tools-catalog）；
//! which 为 unix_extras.rs 自研；urlencode/urldecode 由 urlcoding_impl.rs
//! 基于 crates.io 的 urlcoding 库自研（仅进 Windows 清单，同 data-encoding
//! 家族的门控形态）；zoxide 由 vendor 源码（ajeetdsouza/zoxide）静态链入
//! （lib.rs 可注入入口，eza 同型，仅进 Windows 清单）；dog 由 vendor 源码
//! （ogham/dog，DNS 客户端）静态链入（lib.rs 可注入入口，eza/zoxide 同型，
//! 仅进 Windows 清单，TLS 走 native-tls 无需 OpenSSL）。
//! chcon/runcon 依赖 SELinux，不集成。

use std::ffi::OsString;
use tools_catalog::{all_commands, is_known_command};

/// 读自身 argv 的内嵌命令（解析器无参数注入接口，仅硬链接 argv[0] 模式
/// 可用，见 coreutils_main；子命令模式下给出提示）。vim 的解析器是上游
/// VimMain 内部的 GetCommandLineW 自解析（见 vim_impl.rs），同一受限语义，
/// 归入本家族
const OWN_ARGV_COMMANDS: &[&str] = &["awk", "bat", "fd", "onefetch", "parallel", "rg", "vim"];

/// 从 argv[0] 解析被调用的命令名：取 basename 并去掉 `.exe` 后缀。
/// 分隔符同时兼容 `/` 与 `\`（macOS 单测里也要能解析 Windows 风格路径，
/// 不能依赖 `std::path::Path` 的平台语义）；`.exe` 无条件剥离，保证
/// 跨平台行为确定（Windows 链接名必带后缀，Unix 上不会出现该形式）。
pub fn util_name_from_argv0(argv0: &str) -> String {
    let name = argv0.rsplit(['/', '\\']).next().unwrap_or(argv0);
    let lower = name.to_ascii_lowercase();
    match lower.strip_suffix(".exe") {
        Some(stripped) => stripped.to_string(),
        None => lower,
    }
}

/// 执行 uutils 命令：locale 初始化后分发到对应 `uumain`。
/// 迭代器首元素为命令名（被 clap 当作 bin name 消费，与官方 `bin_inner!`
/// 传 `args_os()`（含真实 argv[0]）等价）。`#[uucore::main]` 保证返回 `i32`。
fn run_uutils(name: &str, args: &[OsString]) -> i32 {
    let _ = uucore::locale::setup_localization(name);
    let mut full: Vec<OsString> = Vec::with_capacity(args.len() + 1);
    full.push(OsString::from(name));
    full.extend_from_slice(args);
    let it = full.into_iter();
    match name {
        "arch" => uu_arch::uumain(it),
        "b2sum" => uu_b2sum::uumain(it),
        "base32" => uu_base32::uumain(it),
        // data-encoding 家族（GNU 风格 CLI 壳在 encodings_impl.rs）。仅注册进
        // Windows 命令清单；分发臂跨平台编译，macOS 单测直接经 run_uutils
        // 验证接线（见 data_encoding_family_dispatch）
        "base32hex" => crate::encodings_impl::enc_main(it),
        "base64" => uu_base64::uumain(it),
        "base64url" => crate::encodings_impl::enc_main(it),
        "basename" => uu_basename::uumain(it),
        "basenc" => uu_basenc::uumain(it),
        "cat" => uu_cat::uumain(it),
        "cksum" => uu_cksum::uumain(it),
        "cmp" => diffutilslib::cmp_main(it),
        "comm" => uu_comm::uumain(it),
        "cp" => uu_cp::uumain(it),
        "csplit" => uu_csplit::uumain(it),
        "cut" => uu_cut::uumain(it),
        "date" => uu_date::uumain(it),
        "dd" => uu_dd::uumain(it),
        "df" => uu_df::uumain(it),
        "diff" => diffutilslib::diff_main(it),
        "dir" => uu_dir::uumain(it),
        "dircolors" => uu_dircolors::uumain(it),
        "dirname" => uu_dirname::uumain(it),
        // dog（ogham/dog，纯 Rust DNS 客户端）：vendor 源码静态链入（lib.rs
        // 可注入入口，eza/zoxide 同型）。同 data-encoding / urlcoding 家族：
        // 仅注册进 Windows 命令清单，分发臂跨平台编译，macOS 单测直接经
        // run_uutils 验证接线与行为（见 dog_dispatch_and_behavior）
        "dog" => dog::dog_main_from(it),
        "du" => uu_du::uumain(it),
        "echo" => uu_echo::uumain(it),
        "env" => uu_env::uumain(it),
        "eza" => eza::eza_main_from(it),
        "expand" => uu_expand::uumain(it),
        "expr" => uu_expr::uumain(it),
        "factor" => uu_factor::uumain(it),
        "false" => uu_false::uumain(it),
        "fmt" => uu_fmt::uumain(it),
        "fold" => uu_fold::uumain(it),
        "grep" => uu_grep::uumain(it),
        "head" => uu_head::uumain(it),
        "hex" => crate::encodings_impl::enc_main(it),
        "hexupper" => crate::encodings_impl::enc_main(it),
        "hostid" => uu_hostid::uumain(it),
        "hostname" => uu_hostname::uumain(it),
        "join" => uu_join::uumain(it),
        "kill" => uu_kill::uumain(it),
        "link" => uu_link::uumain(it),
        "ln" => uu_ln::uumain(it),
        "ls" => uu_ls::uumain(it),
        "md5sum" => uu_md5sum::uumain(it),
        "mkdir" => uu_mkdir::uumain(it),
        "mktemp" => uu_mktemp::uumain(it),
        "more" => uu_more::uumain(it),
        "mv" => uu_mv::uumain(it),
        "nl" => uu_nl::uumain(it),
        "nice" => uu_nice::uumain(it),
        "nohup" => uu_nohup::uumain(it),
        "nproc" => uu_nproc::uumain(it),
        "numfmt" => uu_numfmt::uumain(it),
        "od" => uu_od::uumain(it),
        "paste" => uu_paste::uumain(it),
        "pathchk" => uu_pathchk::uumain(it),
        "pr" => uu_pr::uumain(it),
        "printenv" => uu_printenv::uumain(it),
        "printf" => uu_printf::uumain(it),
        "ptx" => uu_ptx::uumain(it),
        "pwd" => uu_pwd::uumain(it),
        "readlink" => uu_readlink::uumain(it),
        "realpath" => uu_realpath::uumain(it),
        "rm" => uu_rm::uumain(it),
        "rmdir" => uu_rmdir::uumain(it),
        "sd" => sd_cli::sd_main_from(it),
        "sdiff" => diffutilslib::sdiff_main(it),
        "sed" => sed::sed::uumain(it),
        "seq" => uu_seq::uumain(it),
        "sha1sum" => uu_sha1sum::uumain(it),
        "sha224sum" => uu_sha224sum::uumain(it),
        "sha256sum" => uu_sha256sum::uumain(it),
        "sha384sum" => uu_sha384sum::uumain(it),
        "sha512sum" => uu_sha512sum::uumain(it),
        "shred" => uu_shred::uumain(it),
        "shuf" => uu_shuf::uumain(it),
        "sleep" => uu_sleep::uumain(it),
        "sort" => uu_sort::uumain(it),
        "split" => uu_split::uumain(it),
        "sum" => uu_sum::uumain(it),
        "sync" => uu_sync::uumain(it),
        "tac" => uu_tac::uumain(it),
        "tar" => crate::tar_impl::tar_main(it),
        "tail" => uu_tail::uumain(it),
        "tee" => uu_tee::uumain(it),
        "test" => uu_test::uumain(it),
        "timeout" => uu_timeout::uumain(it),
        "touch" => uu_touch::uumain(it),
        "tr" => uu_tr::uumain(it),
        // tree = eza --tree：在 bin name 后注入 --tree
        "tree" => {
            let mut full: Vec<OsString> = Vec::with_capacity(args.len() + 2);
            full.push(OsString::from("tree"));
            full.push(OsString::from("--tree"));
            full.extend_from_slice(args);
            eza::eza_main_from(full)
        }
        "true" => uu_true::uumain(it),
        "truncate" => uu_truncate::uumain(it),
        "tsort" => uu_tsort::uumain(it),
        "tty" => uu_tty::uumain(it),
        "uname" => uu_uname::uumain(it),
        "unexpand" => uu_unexpand::uumain(it),
        "uniq" => uu_uniq::uumain(it),
        "unlink" => uu_unlink::uumain(it),
        "uptime" => uu_uptime::uumain(it),
        "vdir" => uu_vdir::uumain(it),
        "wc" => uu_wc::uumain(it),
        "wget" => wget_rs::wget_main_from(it),
        "whoami" => uu_whoami::uumain(it),
        "xargs" => {
            let argv = argv0_strs(name, args);
            let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
            findutils::xargs::xargs_main(&refs)
        }
        "yes" => uu_yes::uumain(it),
        // unix 专属
        #[cfg(unix)]
        "chgrp" => uu_chgrp::uumain(it),
        #[cfg(unix)]
        "chmod" => uu_chmod::uumain(it),
        #[cfg(unix)]
        "chown" => uu_chown::uumain(it),
        #[cfg(unix)]
        "chroot" => uu_chroot::uumain(it),
        #[cfg(unix)]
        "groups" => uu_groups::uumain(it),
        #[cfg(unix)]
        "id" => uu_id::uumain(it),
        #[cfg(unix)]
        "install" => uu_install::uumain(it),
        #[cfg(unix)]
        "logname" => uu_logname::uumain(it),
        #[cfg(unix)]
        "mkfifo" => uu_mkfifo::uumain(it),
        #[cfg(unix)]
        "mknod" => uu_mknod::uumain(it),
        #[cfg(unix)]
        "stat" => uu_stat::uumain(it),
        #[cfg(unix)]
        "stty" => uu_stty::uumain(it),
        #[cfg(unix)]
        "pinky" => uu_pinky::uumain(it),
        #[cfg(unix)]
        "users" => uu_users::uumain(it),
        #[cfg(unix)]
        "who" => uu_who::uumain(it),
        // unix + windows 均可用
        #[cfg(any(unix, target_os = "windows"))]
        "stdbuf" => uu_stdbuf::uumain(it),
        // 仅 Windows：vcpkg 静态库 + 现场编译的 OpenSSL CLI（FFI argc/argv）
        #[cfg(target_os = "windows")]
        "openssl" => openssl_app::openssl_cmd_main(it),
        // 仅 Windows：vendor 源码（gavinhoward/bc 7.1.0）现场编译的 bc CLI；
        // FFI 直调改名后的 main（内部按 argv[0] 分发 bc/dc，见 vendor/bc-app）
        #[cfg(target_os = "windows")]
        "bc" => bc_app::bc_cmd_main(it),
        // 仅 Windows：vendor 源码（jqlang/jq 1.8.2 + 捆绑 oniguruma）现场编译的
        // jq CLI；FFI 直调 umain（UTF-8 char** 入口，见 vendor/jq-app）
        #[cfg(target_os = "windows")]
        "jq" => jq_app::jq_cmd_main(it),
        // 仅 Windows：源码内嵌 PuTTY 的 telnet（C 层 GNU 参数解析）
        #[cfg(target_os = "windows")]
        "telnet" => putty_telnet::telnet_cmd_main(it),
        // urlcoding 家族（urlencoding 库，crates.io 纯 Rust）：仅注册进 Windows
        // 命令清单；分发臂跨平台编译（同 data-encoding 家族，macOS 单测经
        // run_uutils 验证接线，见 urlcoding_family_dispatch）
        "urldecode" => crate::urlcoding_impl::urldecode_cmd_main(it),
        "urlencode" => crate::urlcoding_impl::urlencode_cmd_main(it),
        // 仅 Windows：yq 与 fzf（Go）经合并 veloterm_goffi.dll 的 C 导出
        // （运行时按符号加载，见 goffi_impl.rs；fzf 是交互式 TUI，在宿主
        // 进程内直跑，stdio 是继承的 ConPTY 句柄）
        #[cfg(target_os = "windows")]
        "yq" => crate::goffi_impl::yq_cmd_main(it),
        #[cfg(target_os = "windows")]
        "fzf" => crate::goffi_impl::fzf_cmd_main(it),
        // 仅 Windows：qalc（libqalculate C++）经 qalcffi.dll 的 C 导出
        // （运行时加载，见 qalc_impl.rs；MinGW 构建，无 MSVC 链接路径）
        #[cfg(target_os = "windows")]
        "qalc" => crate::qalc_impl::qalc_cmd_main(it),
        // 仅 Windows：nmap（C/C++，官方 MSVC 路线）经 nmapffi.dll 的 C 导出
        // （运行时加载，见 nmap_impl.rs；NMAPDIR 指向随包 nmap-data/，
        // Packet.dll 延迟导入——未装 Npcap 时降级 connect 扫描）
        #[cfg(target_os = "windows")]
        "nmap" => crate::nmap_impl::nmap_cmd_main(it),
        // 仅 Windows：ncat（Nmap 项目自带的 netcat 替代，C）经 ncatffi.dll
        // 的 C 导出（运行时加载，见 ncat_impl.rs；复用 nmap 的 nbase/nsock/
        // lua/wpcap 依赖；ca-bundle.crt 随包放安装根供 SSL 验证）
        #[cfg(target_os = "windows")]
        "ncat" => crate::ncat_impl::ncat_cmd_main(it),
        // 仅 Windows：nping（报文生成/探测，C++）经 npingffi.dll 的 C 导出
        // （运行时加载，见 nping_impl.rs；复用 nbase/nsock/libnetutil/dnet）
        #[cfg(target_os = "windows")]
        "nping" => crate::nping_impl::nping_cmd_main(it),
        // zoxide（ajeetdsouza/zoxide，纯 Rust）：vendor 源码静态链入（lib.rs
        // 可注入入口，eza 同型）。同 data-encoding / urlcoding 家族：仅注册进
        // Windows 命令清单，分发臂跨平台编译，macOS 单测直接经 run_uutils
        // 验证接线与行为（见 zoxide_dispatch_and_behavior）
        "zoxide" => zoxide::zoxide_main_from(it),
        // 不可达：run_command 已按 all_commands 白名单拦截
        _ => {
            eprintln!("coreutils: unimplemented command '{name}'");
            1
        }
    }
}

/// 构造含 argv[0] 的有损参数串（findutils 家族约定：args[0] 为程序名）
fn argv0_strs(name: &str, args: &[OsString]) -> Vec<String> {
    let mut v = Vec::with_capacity(args.len() + 1);
    v.push(name.to_string());
    v.extend(args.iter().map(|a| a.to_string_lossy().into_owned()));
    v
}

/// 执行命令并返回退出码；未知命令返回 `None`。
/// `args` 为命令行参数（不含命令名）。
pub fn run_command(name: &str, args: &[OsString]) -> Option<i32> {
    if !is_known_command(name) {
        return None;
    }
    let args_lossy: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    match name {
        "ps" => return Some(crate::ps::run(args)),
        "top" => return Some(crate::top::run(args)),
        "which" => return Some(crate::unix_extras::cmd_which(&args_lossy)),
        // OWN_ARGV 家族（rg/fd/awk/bat/onefetch/parallel）：解析器直接读自身进程 argv，无注入接口，
        // 仅在硬链接短名模式下由 coreutils_main 直调各自入口；走到这里说明是
        // `coreutils <cmd> ...` 子命令模式，首参会误判，给出提示。
        cmd if OWN_ARGV_COMMANDS.contains(&cmd) => {
            eprintln!("coreutils: {cmd} 直接读取自身命令行，请通过短名 {cmd} 调用");
            return Some(2);
        }
        // find：入口需要 argv[0] + Dependencies 注入（非 uumain 迭代器风格）
        "find" => {
            let argv = argv0_strs(name, args);
            let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
            return Some(findutils::find::find_main(
                &refs,
                &findutils::find::StandardDependencies::new(),
            ));
        }
        _ => {}
    }
    Some(run_uutils(name, args))
}

/// coreutils 二进制 main 入口（供本 crate 的 `main.rs` 调用）：
/// - 以已知命令名（硬链接短名）启动 → argv[0] 分发；
/// - 否则（`coreutils` / `coreutils.exe`）→ 子命令模式 `coreutils <cmd> [args...]`。
pub fn coreutils_main() -> i32 {
    let argv: Vec<OsString> = std::env::args_os().collect();
    let argv0 = argv
        .first()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let invoked_as = util_name_from_argv0(&argv0);
    let rest: Vec<OsString> = argv.iter().skip(1).cloned().collect();

    if is_known_command(&invoked_as) {
        // OWN_ARGV 家族（rg/fd/awk/bat/onefetch/parallel/vim）：解析器直接读真实进程 argv
        //（vim 是 Win32 进程命令行，见 vim_impl.rs）；硬链接短名模式下 argv 恰好就是用户
        // 参数，直接执行。
        match invoked_as.as_str() {
            "rg" => return ripgrep::rg_main(),
            "fd" => return fd_find::fd_main(),
            "awk" => return frawk::awk_main(),
            "onefetch" => return onefetch::onefetch_main(),
            "parallel" => return rust_parallel::parallel_main(),
            "bat" => return bat_app::bat_main(),
            // vim 仅在 Windows 命令表里：上游 VimMain 在 MSWIN 下用
            // GetCommandLineW 自解析进程命令行，入口无需（也无法）注入参数
            #[cfg(target_os = "windows")]
            "vim" => return crate::vim_impl::vim_cmd_main(),
            _ => {}
        }
        return run_command(&invoked_as, &rest).unwrap_or(1);
    }

    // 子命令模式：coreutils <cmd> [args...]
    let Some(cmd) = rest.first().map(|s| s.to_string_lossy().into_owned()) else {
        let all = all_commands();
        eprintln!(
            "coreutils (uutils) + veloterm extras: {} commands",
            all.len()
        );
        eprintln!("usage: coreutils <command> [args...]");
        eprintln!("commands: {}", all.join(" "));
        return 1;
    };
    let cmd_args: Vec<OsString> = rest.iter().skip(1).cloned().collect();
    match run_command(&cmd, &cmd_args) {
        Some(code) => code,
        None => {
            eprintln!("coreutils: unknown command '{cmd}'");
            eprintln!("commands: {}", all_commands().join(" "));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "veloterm-coreutils-test-{}-{}",
            tag,
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn argv0_resolves_command_names() {
        assert_eq!(util_name_from_argv0("ls"), "ls");
        assert_eq!(util_name_from_argv0("/usr/bin/ls"), "ls");
        assert_eq!(
            util_name_from_argv0(r"C:\Program Files\VeloTerm\tools\ls.exe"),
            "ls"
        );
        assert_eq!(util_name_from_argv0(r"...\tools\GREP.EXE"), "grep");
        // coreutils 本名不是命令 → 进入子命令模式
        assert_eq!(util_name_from_argv0("coreutils"), "coreutils");
        assert!(!is_known_command(&util_name_from_argv0("coreutils.exe")));
    }

    #[test]
    fn unknown_command_returns_none() {
        let none: Vec<OsString> = vec![];
        assert!(run_command("not-a-command", &none).is_none());
        assert!(run_command("", &none).is_none());
    }

    #[test]
    fn command_table_consistency() {
        // 清单自身的结构性约束（无重复 / 自研命令不与 uutils 重名）已随命令表
        // 迁到 tools-catalog 的 `command_table_consistency`；此处只保证本 crate
        // 的分发侧把整张表都当成已知命令，不会出现「清单里有、分发认不出」。
        for cmd in all_commands() {
            assert!(is_known_command(cmd), "{cmd} 应在分发白名单内");
        }
    }

    /// 全命令分发冒烟：每个命令真实跑一次 `--version`。能捕获依赖缺失 /
    /// cfg 分组错误 / 名字错配。预期非 0 的例外：
    /// - `false`：GNU 语义恒返回 1（连 --version 也不打印）；
    /// - `test`：无 --version（跑空参数 = false，退出 1）；
    /// - which（自研）：不支持 --version，按 Usage 退出 1。
    ///
    /// 跳过（真实行为由 new_families_smoke / release 冒烟覆盖）：
    /// - OWN_ARGV 家族（rg/fd/awk/bat/onefetch/parallel）读自身进程 argv，单测无法注入；
    /// - diff/cmp：上游 uutils/diffutils 未实现 --version——cmp 返回 2，
    ///   diff 的解析失败路径会 std::process::exit(2) 直接杀掉测试进程；
    /// - sd / wget：入口内部走 clap 的 `Parser::parse_from`（分别是
    ///   `sd_cli::sd_main_from` 与 `wget_rs::wget_main_from`），`--version`
    ///   命中 clap 的 DisplayVersion 分支后 `std::process::exit(0)`。这会杀掉
    ///   测试进程，且因退出码为 0，libtest 会**误报通过**并静默截断其后所有
    ///   命令的扫描——历史上 sd 之后的 sdiff/sed/…/ps/top 等 60+ 条命令因此
    ///   从未被真正扫到。
    #[test]
    fn all_commands_dispatch_version_sweep() {
        let expect_one = ["test", "false", "which"];
        let mut skip = vec!["cmp", "diff", "sd", "wget"];
        // yq/fzf/qalc/nmap/ncat/nping 仅在 Windows 命令表里，且依赖打包产物
        // DLL（运行时加载），cargo test 环境不保证存在 —— 跳过
        // （真实行为由安装包冒烟覆盖）
        if cfg!(target_os = "windows") {
            skip.push("yq");
            skip.push("fzf");
            skip.push("qalc");
            skip.push("nmap");
            skip.push("ncat");
            skip.push("nping");
        }
        let all = all_commands();
        let mut visited = 0usize;
        for name in all.iter().copied() {
            if OWN_ARGV_COMMANDS.contains(&name) || skip.contains(&name) {
                continue;
            }
            let code = if name == "test" {
                run_command(name, &[]).expect(name)
            } else {
                run_command(name, &[OsString::from("--version")]).expect(name)
            };
            let expected = i32::from(expect_one.contains(&name));
            assert_eq!(code, expected, "{name} 分发异常");
            visited += 1;
        }
        // 兜底：确认扫描确实走完了整张表（若某命令令进程静默退出，此处不会被
        // 执行到——libtest 会误报通过，故再断言一次数量以留下书面预期）。
        // OWN_ARGV / skip 按与命令表的**实际交集**计数：vim 这类平台不对称的
        // OWN_ARGV 命令只在 Windows 表里，macOS 上全量扣减会多数出一个。
        let own_argv_in_table = all
            .iter()
            .filter(|c| OWN_ARGV_COMMANDS.contains(c))
            .count();
        let skip_in_table = all.iter().filter(|c| skip.contains(c)).count();
        assert_eq!(
            visited,
            all.len() - own_argv_in_table - skip_in_table,
            "扫描条数与命令表不符"
        );
    }

    #[test]
    fn new_families_smoke() {
        // find/xargs（uutils/findutils）、diff/cmp/sdiff（uutils/diffutils）、
        // sed（uutils/sed）、sd（chmln/sd）四族真实行为抽查
        let dir = tempdir("families");
        let s = |v: &[&str]| -> Vec<OsString> { v.iter().map(OsString::from).collect() };

        // sed：流替换（文件参数）
        let sed_f = dir.join("sed.txt");
        fs::write(&sed_f, "aaa bbb\n").unwrap();
        let sed_p = sed_f.to_string_lossy().into_owned();
        assert_eq!(
            run_command("sed", &s(&["s/bbb/ccc/", sed_p.as_str()])).unwrap(),
            0
        );

        // diff / cmp / sdiff：差异 → 1，相同 → 0（GNU 语义）
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        fs::write(&a, "x\ny\n").unwrap();
        fs::write(&b, "x\nz\n").unwrap();
        let (ap, bp) = (
            a.to_string_lossy().into_owned(),
            b.to_string_lossy().into_owned(),
        );
        assert_eq!(
            run_command("diff", &s(&[ap.as_str(), bp.as_str()])).unwrap(),
            1
        );
        assert_eq!(
            run_command("diff", &s(&[ap.as_str(), ap.as_str()])).unwrap(),
            0
        );
        assert_eq!(
            run_command("cmp", &s(&[ap.as_str(), bp.as_str()])).unwrap(),
            1
        );
        assert_eq!(
            run_command("cmp", &s(&[ap.as_str(), ap.as_str()])).unwrap(),
            0
        );
        assert_eq!(
            run_command("sdiff", &s(&[ap.as_str(), bp.as_str()])).unwrap(),
            1
        );

        // find：-name 过滤
        let dir_p = dir.to_string_lossy().into_owned();
        assert_eq!(
            run_command("find", &s(&[dir_p.as_str(), "-name", "a.txt"])).unwrap(),
            0
        );

        // sd：文件内容原位替换（abc → xyz）
        let sd_f = dir.join("sd.txt");
        fs::write(&sd_f, "abc abc").unwrap();
        let sd_p = sd_f.to_string_lossy().into_owned();
        assert_eq!(
            run_command("sd", &s(&["abc", "xyz", sd_p.as_str()])).unwrap(),
            0
        );
        assert_eq!(fs::read_to_string(&sd_f).unwrap(), "xyz xyz");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn eza_tree_smoke() {
        let dir = tempdir("eza");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub").join("x.txt"), "x").unwrap();
        let dir_p = dir.to_string_lossy().into_owned();
        let s = |v: &[&str]| -> Vec<OsString> { v.iter().map(OsString::from).collect() };

        // eza 普通列表与 tree（= eza --tree）均应正常退出
        assert_eq!(run_command("eza", &s(&[dir_p.as_str()])).unwrap(), 0);
        assert_eq!(run_command("tree", &s(&[dir_p.as_str()])).unwrap(), 0);
        assert_eq!(
            run_command("tree", &s(&["--level", "1", dir_p.as_str()])).unwrap(),
            0
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn uutils_grep_real_search() {
        // grep 已换用 vendor 的 uutils/grep：验证真实搜索行为与退出码语义
        let dir = tempdir("grep");
        let file = dir.join("data.txt");
        fs::write(&file, "apple\nbanana\ncherry\n").unwrap();
        let path = file.to_string_lossy().into_owned();

        let s = |v: &[&str]| -> Vec<OsString> { v.iter().map(OsString::from).collect() };

        // 命中 → 0；未命中 → 1（GNU grep 语义）
        assert_eq!(run_command("grep", &s(&["an", path.as_str()])).unwrap(), 0);
        assert_eq!(run_command("grep", &s(&["zzz", path.as_str()])).unwrap(), 1);
        // -i 忽略大小写
        assert_eq!(
            run_command("grep", &s(&["-i", "APPLE", path.as_str()])).unwrap(),
            0
        );
        // -c 计数模式
        assert_eq!(
            run_command("grep", &s(&["-c", "an", path.as_str()])).unwrap(),
            0
        );
        // 非法正则 → 2
        assert_eq!(run_command("grep", &s(&["[", path.as_str()])).unwrap(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn uutils_commands_smoke() {
        // 抽查新加入的命令与经典组合的真实行为
        let dir = tempdir("smoke");
        let file_a = dir.join("a.txt");
        fs::write(&file_a, "b\na\nc\n").unwrap();
        let a = file_a.to_string_lossy().into_owned();

        let s = |v: &[&str]| -> Vec<OsString> { v.iter().map(OsString::from).collect() };

        // sort 输出排序结果
        assert_eq!(run_command("sort", &s(&[a.as_str()])).unwrap(), 0);
        // wc 统计
        assert_eq!(run_command("wc", &s(&["-l", a.as_str()])).unwrap(), 0);
        // du 目录
        assert_eq!(
            run_command("du", &s(&[dir.to_string_lossy().as_ref()])).unwrap(),
            0
        );
        // seq
        assert_eq!(run_command("seq", &s(&["3"])).unwrap(), 0);
        // 非法参数非零退出
        assert_ne!(
            run_command("ls", &s(&["--veloterm-invalid-flag"])).unwrap(),
            0
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// data-encoding 家族接线：命令仅注册进 Windows 清单（run_command 在
    /// 其他平台会先被清单拦截），这里绕过清单直接验证分发臂 → enc_main 的
    /// 通路；真实行为由 encodings_impl 自带的参数化单测覆盖。
    #[test]
    fn data_encoding_family_dispatch() {
        for name in ["base32hex", "base64url", "hex", "hexupper"] {
            assert_eq!(
                run_uutils(name, &[OsString::from("--version")]),
                0,
                "{name} 分发异常"
            );
        }
    }

    /// urlcoding 家族接线：命令仅注册进 Windows 清单（同 data-encoding 家族），
    /// 绕过清单直接验证分发臂 → urlcoding_impl 的通路；真实行为由
    /// urlcoding_impl 自带的参数化单测覆盖（含真实库编解码向量）。
    #[test]
    fn urlcoding_family_dispatch() {
        for name in ["urldecode", "urlencode"] {
            assert_eq!(
                run_uutils(name, &[OsString::from("--version")]),
                0,
                "{name} 分发异常"
            );
        }
    }

    /// zoxide 接线与行为：命令仅注册进 Windows 清单（同 data-encoding /
    /// urlcoding 家族），绕过清单直接验证分发臂 → zoxide::zoxide_main_from
    /// 的通路；行为侧做 add → query 数据库闭环（_ZO_DATA_DIR /
    /// _ZO_EXCLUDE_DIRS 指向临时目录：后者必须显式设成不匹配任何路径的
    /// glob，上游默认排除 $HOME，而 Windows 的 TEMP 在用户目录内，不设会
    /// 把测试目录整个排除掉）。shell 钩子（zoxide init）的真实行为由上游
    /// 测试覆盖；注意上游不支持 cmd.exe 集成。
    #[test]
    fn zoxide_dispatch_and_behavior() {
        let s = |v: &[&str]| -> Vec<OsString> { v.iter().map(OsString::from).collect() };

        // --version 走 clap DisplayVersion，打印后返回 0（不杀测试进程）；
        // 非法参数走 clap usage 错误，退出码 2
        assert_eq!(run_uutils("zoxide", &s(&["--version"])), 0);
        assert_eq!(run_uutils("zoxide", &s(&["--veloterm-invalid-flag"])), 2);

        let dir = tempdir("zoxide");
        let data_dir = dir.join("data");
        let target = dir.join("projects");
        fs::create_dir_all(&target).unwrap();
        unsafe {
            std::env::set_var("_ZO_DATA_DIR", data_dir.as_os_str());
            std::env::set_var("_ZO_EXCLUDE_DIRS", "__veloterm_no_exclude__");
        }

        let target_s = target.to_string_lossy().into_owned();
        assert_eq!(run_uutils("zoxide", &s(&["add", target_s.as_str()])), 0);
        // 命中 → 0；未命中 → 1（上游 no match found 语义）
        assert_eq!(run_uutils("zoxide", &s(&["query", "projects"])), 0);
        assert_eq!(run_uutils("zoxide", &s(&["query", "zzz-no-match"])), 1);
        // 数据库文件确已落在 _ZO_DATA_DIR（上游固定文件名 db.zo）
        assert!(data_dir.join("db.zo").exists());

        unsafe {
            std::env::remove_var("_ZO_DATA_DIR");
            std::env::remove_var("_ZO_EXCLUDE_DIRS");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// dog 接线与行为：命令仅注册进 Windows 清单（同 zoxide），绕过清单
    /// 直接验证分发臂 → dog::dog_main_from 的通路。行为侧覆盖无网络路径：
    /// --version / --help → 0；无域名（打印 usage）与非法选项/非法查询类型
    /// → 3（OPTIONS_ERROR，上游退出码约定）；再取一个刚释放的回环端口做
    /// --tcp 查询，连接被拒 → 1（NETWORK_ERROR——output.rs 的 print 对空
    /// 响应除 Short 模式外恒返回 true）。真实 DNS 解析由上游 xtests 覆盖；
    /// Windows 的系统默认名字服务器发现（ipconfig crate）只能靠 Windows
    /// 真机/CI 回归。
    #[test]
    fn dog_dispatch_and_behavior() {
        let s = |v: &[&str]| -> Vec<OsString> { v.iter().map(OsString::from).collect() };

        assert_eq!(run_uutils("dog", &s(&["--version"])), 0);
        assert_eq!(run_uutils("dog", &s(&["--help"])), 0);
        assert_eq!(run_uutils("dog", &s(&[])), 3);
        assert_eq!(run_uutils("dog", &s(&["--veloterm-invalid-flag"])), 3);
        assert_eq!(
            run_uutils("dog", &s(&["example.com", "--type", "not-a-type"])),
            3
        );

        // 回环已关闭端口：TCP 立即 ECONNREFUSED → NETWORK_ERROR 1
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let ns = format!("@127.0.0.1:{port}");
        assert_eq!(
            run_uutils("dog", &s(&[ns.as_str(), "example.com", "--tcp", "--colour", "never"])),
            1
        );
    }

    #[test]
    fn ps_and_top_smoke() {
        let s = |v: &[&str]| -> Vec<OsString> { v.iter().map(OsString::from).collect() };

        // ps 基础命令与各种格式
        assert_eq!(run_command("ps", &s(&["--version"])).unwrap(), 0);
        assert_eq!(run_command("ps", &s(&["--help"])).unwrap(), 0);
        assert_eq!(run_command("ps", &[]).unwrap(), 0);
        assert_eq!(run_command("ps", &s(&["-ef"])).unwrap(), 0);
        assert_eq!(run_command("ps", &s(&["aux"])).unwrap(), 0);
        let pid_str = std::process::id().to_string();
        assert_eq!(run_command("ps", &s(&["-p", &pid_str])).unwrap(), 0);

        // top 版本、帮助与批处理模式
        assert_eq!(run_command("top", &s(&["--version"])).unwrap(), 0);
        assert_eq!(run_command("top", &s(&["--help"])).unwrap(), 0);
        assert_eq!(run_command("top", &s(&["-b", "-n", "1"])).unwrap(), 0);
    }
}
