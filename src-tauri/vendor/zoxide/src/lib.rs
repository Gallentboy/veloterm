#![allow(clippy::single_component_path_imports)]

/*!
The library entry point into zoxide.

VeloTerm vendor 新增：上游本 crate 仅有 bin 入口（`src/main.rs` 持有全部模块
声明与 `main()` 主逻辑）。为把 zoxide 内嵌进 VeloTerm `coreutils` 多呼叫二进制，
新增本文件：承接原 `main.rs` 的模块声明与主逻辑，暴露 `zoxide_main()` /
`zoxide_main_from()` 两个入口（同 vendor/eza 的 `eza_main` / `eza_main_from`
约定）；`main.rs` 保留为独立 zoxide 二进制的薄壳入口。
*/

mod cmd;
mod config;
mod db;
mod error;
mod import;
mod shell;
mod util;

use std::env;
use std::ffi::OsString;
use std::io::{self, Write};

use clap::Parser;

use crate::cmd::{Cmd, Run};
use crate::error::SilentExit;

/// VeloTerm coreutils 入口：以真实进程 argv 执行 zoxide 并返回进程退出码。
///
/// 注意：本函数直接读取真实进程 argv（argv[0] 作为程序名被 clap 跳过），
/// 适合「以 zoxide 短名（硬链接）启动」的场景；`coreutils zoxide ...`
/// 子命令模式下请改用可注入的 [`zoxide_main_from`]。
pub fn zoxide_main() -> i32 {
    zoxide_main_from(env::args_os())
}

/// VeloTerm coreutils 可注入入口：以 `args`（首元素为程序名，将被 clap
/// 作为 bin name 跳过）执行 zoxide 并返回进程退出码。
///
/// VeloTerm vendor 修改：原 `Cmd::parse()` 改为 `try_parse_from(args)`，使
/// 参数可注入，且 clap 的 `--help` / `--version` / usage 错误不会被
/// `process::exit` 直接杀死宿主进程（嵌入式多呼叫二进制与单测进程不受
/// 影响），打印后原样返回退出码。
pub fn zoxide_main_from<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    // Forcibly disable backtraces.
    unsafe { env::remove_var("RUST_LIB_BACKTRACE") };
    unsafe { env::remove_var("RUST_BACKTRACE") };

    let cmd = match Cmd::try_parse_from(args) {
        Ok(cmd) => cmd,
        Err(e) => {
            let _ = e.print();
            return e.exit_code();
        }
    };
    match cmd.run() {
        Ok(()) => 0,
        Err(e) => match e.downcast::<SilentExit>() {
            Ok(SilentExit { code }) => i32::from(code),
            Err(e) => {
                let _ = writeln!(io::stderr(), "zoxide: {e:?}");
                1
            }
        },
    }
}
