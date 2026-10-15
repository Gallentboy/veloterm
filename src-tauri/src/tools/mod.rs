//! VeloTerm 内置 Unix 工具集（Windows 专属能力，随应用分发）：
//!
//! - `coreutils`：单一多呼叫二进制，命令实现全部来自工程 vendor 源码：
//!   uutils/coreutils 0.12.0（GNU coreutils 全集）、uutils/grep、uutils/sed、
//!   uutils/findutils（find/xargs）、uutils/diffutils（diff/cmp/sdiff）、
//!   ripgrep（rg）、fd-find（fd）、chmln/sd（sd）、frawk（awk，Cranelift
//!   JIT）；which 为自研（unix_extras.rs）。
//! - `hardlinks`：启动时在 `tools/` 下为 coreutils 创建硬链接短名入口，
//!   PATH 注入后用户在终端直接敲 `ls` / `grep` / `rg` / `fd` 等短名即用。
//!
//! 无任何 .bat 转发，不依赖 Git Bash / MSYS2。
pub mod coreutils;
pub mod hardlinks;
pub mod unix_extras;
