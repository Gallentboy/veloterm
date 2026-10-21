//! VeloTerm 内置工具集 —— **主程序侧**只剩一件事：安装硬链接短名入口。
//!
//! 命令实现（多呼叫二进制分发核心、ps/top/tar/which）已拆到独立 crate
//! `crates/coreutils`，命令名清单在零依赖的 `crates/tools-catalog`。主程序
//! 只依赖清单，因此不再链接 uutils / ripgrep / openssl-app / putty-telnet。
//!
//! - `hardlinks`：Windows 启动时在 `tools/` 下为 coreutils 创建硬链接短名入口
//!   （`ls.exe` `grep.exe`…），PATH 注入后用户在终端直接敲 `ls` / `grep` / `rg`
//!   等短名即用。
//!
//! 无任何 .bat 转发，不依赖 Git Bash / MSYS2。
pub mod hardlinks;
pub mod jq;
