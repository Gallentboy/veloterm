/*!
The library entry point into the sd CLI.

VeloTerm vendor note: this file moves the original `sd-cli/src/main.rs`
logic here so that sd can be embedded as the `sd` command inside the
VeloTerm `coreutils` multicall binary via `sd_cli::sd_main()` /
`sd_cli::sd_main_from()`. The `main.rs` file remains as the thin entry
for the standalone `sd` binary.
*/

pub mod cli;

use std::{ffi::OsString, io::stdout};

use clap::Parser;
use sd::{Replacer, Result, Source, process_sources};

/// VeloTerm coreutils 入口：执行 sd 并返回进程退出码（0 成功，1 失败）。
///
/// 注意：`sd_main()` 走 clap 的 `Options::parse()`，直接读取真实进程
/// argv（argv[0] 作为程序名被跳过），因此它只适合在「以 sd 短名（硬链接）
/// 启动」的场景调用；`coreutils sd ...` 子命令模式下首个 `sd` 参数会被
/// 误认为 find 表达式。子命令分发请改用可注入的 `sd_main_from()`。
pub fn sd_main() -> i32 {
    sd_main_from(std::env::args_os())
}

/// VeloTerm coreutils 可注入入口：以 `args`（首元素为程序名，将被 clap
/// 作为 bin name 跳过）执行 sd 并返回进程退出码（0 成功，1 失败）。
///
/// `coreutils sd <find> <replace> ...` 子命令模式下可传入
/// `std::iter::once(OsString::from("sd")).chain(rest_args)`。
/// clap 自身的 usage/version 错误仍由 clap 直接打印并 exit(2)。
pub fn sd_main_from<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match try_main(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn try_main<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let options = cli::Options::parse_from(args);

    let replacer = Replacer::new(
        options.find,
        options.replace_with,
        options.literal_mode,
        options.flags,
        options.replacements,
    )?;

    let sources = if !options.files.is_empty() {
        Source::from_paths(options.files)
    } else {
        Ok(Source::from_stdin())
    };
    let sources = sources?;

    let mut handle = stdout().lock();
    process_sources(
        &replacer,
        &sources,
        options.preview,
        !options.across,
        &mut handle,
    )
}
