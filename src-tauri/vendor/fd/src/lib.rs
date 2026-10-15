//! The library entry point into fd.
//!
//! VeloTerm vendor note: this file turns fd into a library so that it can be
//! embedded as the `fd` command inside the VeloTerm `coreutils` multicall binary
//! via `fd_find::fd_main()`. Instead of moving the original `src/main.rs` logic
//! here by hand (which would require re-transcribing the 4 KB `DEFAULT_LS_COLORS`
//! constant and risk transcription errors), we `include!` the untouched
//! `src/main.rs`: after macro expansion its `mod xxx;` declarations resolve
//! against `src/` exactly like in the standalone bin target, and its `fn main()`
//! is simply an uncalled ordinary function inside the library (the bin target
//! keeps using it as its entry point).

// VeloTerm vendor 新增：include! 进来的 `fn main()` 在 lib 中无人调用，
// 关掉 dead_code 告警避免污染主工程构建输出（不影响 bin target 的检查）。
#![allow(dead_code)]

// VeloTerm vendor 新增：复用未改动的 src/main.rs 全部逻辑（模块声明、
// run()、DEFAULT_LS_COLORS 等）。
include!("main.rs");

/// VeloTerm coreutils 入口：执行 fd 并返回进程退出码（0-255）。
///
/// 注意：fd 的 `Opts::parse()` 直接读取真实进程 argv（clap derive 无参数
/// 注入接口），因此本函数只适合在「以 fd 短名（硬链接）启动」的场景调用；
/// `coreutils fd ...` 子命令模式下首个 `fd` 参数会被误认为搜索 pattern。
///
/// 与独立进程的细微差异：`ExitCode::KilledBySigint` 在独立 bin 中会重置
/// SIGINT 处理器并 `raise(SIGINT)` 自杀；内嵌调用时仅返回等值退出码 130。
pub fn fd_main() -> i32 {
    match run() {
        Ok(exit_code) => i32::from(exit_code),
        Err(err) => {
            crate::error::print_error(format!("{err:#}"));
            i32::from(ExitCode::GeneralError)
        }
    }
}
