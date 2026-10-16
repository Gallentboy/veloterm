//! VeloTerm 内置 Unix 工具集：单一 `coreutils` 多呼叫二进制（BusyBox 模式）。
//!
//! 本 crate 是**独立 member**，主程序（app）不依赖它——这样 uutils / ripgrep /
//! openssl-app / putty-telnet 的整张依赖图只进入 coreutils.exe，主程序保持轻量，
//! 且改工具与改 UI 的增量构建互不牵连。命令名清单在零依赖的 `tools-catalog`。
//!
//! 命令实现全部来自工程 vendor 源码：uutils/coreutils 0.12.0（GNU coreutils
//! 全集）、uutils/grep、uutils/sed、uutils/findutils（find/xargs）、
//! uutils/diffutils（diff/cmp/sdiff）、ripgrep（rg）、fd-find（fd）、chmln/sd（sd）、
//! frawk（awk，Cranelift JIT）、rust-parallel（parallel，tokio 并行执行器）、
//! onefetch（onefetch，git 仓库信息面板，gix/tokei 纯 Rust 栈）、
//! tar-rs（tar，CLI 壳在 tar_impl.rs）、eza（eza/tree）、
//! data-encoding（base32hex/base64url/hex/hexupper，CLI 壳在 encodings_impl.rs，
//! 仅注册进 Windows 清单）、
//! wget-rs（wget，rustls）、openssl 与 telnet（仅 Windows：vcpkg 静态库 FFI、
//! PuTTY 源码内嵌，见 vendor/openssl-app 与 vendor/putty-telnet）、
//! yq（仅 Windows：Go 源码 vendor/yq + yqffi.dll 运行时加载 FFI，见 yq_impl.rs）、
//! fzf（仅 Windows：junegunn/fzf 的 Go 源码 vendor/fzf + fzfffi.dll 运行时
//! 加载 FFI，交互式模糊查找器，见 fzf_impl.rs）、
//! qalc（仅 Windows：libqalculate 源码 vendor/libqalculate + MinGW 构建的
//! qalcffi.dll 运行时加载 FFI，见 qalc_impl.rs）、
//! urlencode / urldecode（仅 Windows：crates.io urlencoding 库封装，见
//! urlcoding_impl.rs）、
//! zoxide（仅 Windows：ajeetdsouza/zoxide 源码 vendor/zoxide 纯 Rust 静态链入，
//! lib.rs 可注入入口，eza 同型）；
//! which / ps / top 为自研（unix_extras.rs / ps.rs / top.rs）。
//!
//! 安装侧：主程序启动时在 `tools/` 下为本二进制创建硬链接短名入口
//! （`ls.exe`/`grep.exe`/…），PATH 注入后用户直接敲短名即用；无任何 .bat 转发，
//! 不依赖 Git Bash / MSYS2。

mod coreutils;
mod encodings_impl;
mod fzf_impl;
mod ps;
mod qalc_impl;
mod tar_impl;
mod top;
mod unix_extras;
mod urlcoding_impl;
mod yq_impl;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, ProcessStatus, System, UpdateKind};

fn main() {
    std::process::exit(coreutils::coreutils_main());
}

/// 刷新全部进程（ps 与 top 共用），显式指明刷新内容。
///
/// **不能用 `System::refresh_processes` 快捷法**：其默认种类是
/// `with_memory().with_cpu().with_disk_usage().with_exe()`，**不含 cmd**——
/// 不刷新则 `Process::cmd()` 恒为空，COMMAND 列退化为进程名，
/// `ps -C` 按命令行参数搜索也永远命中不了（Windows/macOS 表现一致）。
/// 同时去掉 ps/top 用不到的 disk_usage 与 exe，少读两类内核数据。
pub(crate) fn refresh_all_processes(sys: &mut System) {
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        ProcessRefreshKind::new()
            .with_memory()
            .with_cpu()
            .with_cmd(UpdateKind::Always),
    );
}

/// 将 sysinfo 的进程状态映射为 GNU ps/top 的 STAT 单字母码（ps 与 top 共用）。
///
/// sysinfo 0.31 的 `ProcessStatus` 未实现 `Display`，而其 `Debug` 名
/// （如 `UninterruptibleDiskSleep`，23 字符）会撑爆定宽 STAT 列，故自行映射。
/// 语义对齐 procps-ng 的 `ps` STAT 码表。
pub(crate) fn status_code(status: ProcessStatus) -> &'static str {
    use sysinfo::ProcessStatus::{
        Dead, Idle, LockBlocked, Parked, Run, Sleep, Stop, Tracing, UninterruptibleDiskSleep,
        Unknown, Wakekill, Waking, Zombie,
    };

    match status {
        Idle => "I",                     // 空闲内核线程
        Run => "R",                      // 运行或可运行（在运行队列）
        Sleep => "S",                    // 可中断睡眠（等待事件完成）
        Stop => "T",                     // 已停止（作业控制信号或调试）
        Zombie => "Z",                   // 僵尸（已终止但未被父进程回收）
        Tracing => "t",                  // 被调试器追踪而停止
        Dead => "X",                     // 已死亡
        Wakekill => "K",                 // 唤醒即被杀死
        Waking => "W",                   // 唤醒中
        Parked => "P",                   // 已停驻
        LockBlocked => "L",              // 等待内核锁
        UninterruptibleDiskSleep => "D", // 不可中断睡眠（通常为磁盘 IO）
        Unknown(_) => "?",               // 平台未识别的状态码
    }
}
