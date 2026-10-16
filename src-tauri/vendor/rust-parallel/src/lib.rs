//! The library entry point into rust-parallel.
//!
//! VeloTerm vendor note: this file turns rust-parallel into a library so that
//! it can be embedded as the `parallel` command inside the VeloTerm
//! `coreutils` multicall binary via `rust_parallel::parallel_main()`. Instead
//! of duplicating `src/main.rs` logic here, we `include!` the untouched file:
//! after macro expansion its `mod xxx;` declarations resolve against `src/`
//! exactly like in the standalone bin target, and its `#[tokio::main] async
//! fn main()` is simply an uncalled ordinary function inside the library (the
//! bin target keeps using it as its entry point). main.rs 原顶部的内层属性
//! (`#![forbid(unsafe_code)]` / `#![warn(rust_2018_idioms)]`) 因 include!
//! 展开的内层属性被 rustc 拒绝而移到了本文件顶部（见 main.rs 头部注释），
//! 对 lib 与 bin 目标语义不变。

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

// VeloTerm vendor 新增：include! 进来的 `fn main()` 在 lib 中无人调用，
// 关掉 dead_code 告警避免污染主工程构建输出（不影响 bin target 的检查）。
#![allow(dead_code)]

// VeloTerm vendor 新增：复用未改动的 src/main.rs 全部逻辑（模块声明与
// try_main）。
include!("main.rs");

/// VeloTerm coreutils 入口：执行 rust-parallel 并返回进程退出码（0/1）。
///
/// 忠实镜像上游 `main()`：初始化 tracing 订阅（RUST_LOG 控制）后手动构建
/// 多线程 tokio runtime 并 block_on `try_main`——`#[tokio::main]` 是为 bin
/// 入口生成的包装宏，lib 里不能用。
///
/// 注意：`CommandLineArgs::parse()` 直接读取真实进程 argv（clap derive 无
/// 参数注入接口），因此本函数只适合在「以 parallel 短名（硬链接）启动」的
/// 场景调用；`coreutils parallel ...` 子命令模式下首个 `parallel` 参数会被
/// 误认为 command_and_initial_arguments。
pub fn parallel_main() -> i32 {
    tracing_subscriber::fmt::init();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("rust-parallel: failed to build tokio runtime: {err}");
            return 1;
        }
    };
    match runtime.block_on(try_main()) {
        Ok(()) => 0,
        Err(err) => {
            tracing::error!("fatal error in main:\n{:#}", err);
            1
        }
    }
}
