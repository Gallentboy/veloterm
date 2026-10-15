//! coreutils 多呼叫二进制入口。
//!
//! 以任意命令名（硬链接短名）启动时按 argv[0] 分发；以本名启动时走
//! `coreutils <cmd> [args...]` 子命令模式。实现见 `coreutils.rs`。
fn main() {
    std::process::exit(app_lib::tools::coreutils::coreutils_main());
}
