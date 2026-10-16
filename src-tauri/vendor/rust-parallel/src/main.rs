// VeloTerm vendor 修改：原本位于此处的 `#![forbid(unsafe_code)]` 与
// `#![warn(rust_2018_idioms)]` 内层属性已上移到 src/lib.rs——include! 展开的
// 内层属性会被 rustc 拒绝（fd 的 main.rs 无内层属性故无此问题），lib 目标
// 在 crate 根保留同等 lint 约束，语义不变。
use tracing::{debug, error, instrument};

use crate::command_line_args::CommandLineArgs;

mod command;
mod command_line_args;
mod common;
mod input;
mod output;
mod parser;
mod process;
mod progress;

#[instrument(skip_all, name = "try_main", level = "debug")]
async fn try_main() -> anyhow::Result<()> {
    debug!("begin try_main");

    let command_line_args = CommandLineArgs::instance().await;

    let progress = progress::Progress::new(command_line_args)?;

    let command_service = command::CommandService::new(command_line_args, progress);

    command_service.run_commands().await?;

    debug!("end try_main");

    Ok(())
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    if let Err(err) = try_main().await {
        error!("fatal error in main:\n{:#}", err);
        std::process::exit(1);
    }
}
