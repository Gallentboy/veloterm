// Lib is present to allow for benchmarks and integration tests
pub mod cli;
pub mod info;
pub mod ui;

// VeloTerm vendor 新增：本文件在 upstream lib（仅三个 pub mod）之外追加
// `onefetch_main`，供 VeloTerm coreutils 多呼叫二进制内嵌调用；其余内容
// 与 upstream 一致，src/main.rs 保持原样仍作为独立 bin 入口。
//
// 函数体忠实镜像 src/main.rs 的 `main()`：human-panic 安装、Windows ANSI
// 启用、clap 解析与打印流程全部一致。注意 `CliOptions::parse()` 直读自身
// 进程 argv（clap derive 无参数注入接口），因此本函数只适合在「以
// onefetch 短名（硬链接 argv[0]）启动」的场景调用——`coreutils onefetch
// ...` 子命令模式下首个参数会被误认为路径，coreutils 侧已给出提示。
//
// Result 的退出码映射遵循 Rust `Termination` 对 `Result<(), E: Debug>` 的
// 语义：Err 时向 stderr 打印 `Error: {:?}` 并退出 1（与独立 bin 一致）。
pub fn onefetch_main() -> i32 {
    match onefetch_main_impl() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("Error: {err:?}");
            1
        }
    }
}

/// `onefetch_main` 的实现体，与 src/main.rs 的 `main()` 逐行对应。
fn onefetch_main_impl() -> anyhow::Result<()> {
    use clap::{CommandFactory, Parser};
    use human_panic::setup_panic;
    use std::io;

    setup_panic!();

    #[cfg(windows)]
    enable_ansi_support::enable_ansi_support()?;

    let cli_options = cli::CliOptions::parse();

    if cli_options.other.languages {
        return cli::print_supported_languages();
    }

    if cli_options.other.package_managers {
        return cli::print_supported_package_managers();
    }

    if let Some(generator) = cli_options.developer.completion {
        let mut cmd = cli::CliOptions::command();
        cli::print_completions(generator, &mut cmd);
        return Ok(());
    }

    let info = info::build_info(&cli_options)?;

    let printer = ui::printer::factory::PrinterFactory::new(info, cli_options)?.create()?;

    let mut writer = io::BufWriter::new(io::stdout());

    printer.print(&mut writer)?;

    Ok(())
}
