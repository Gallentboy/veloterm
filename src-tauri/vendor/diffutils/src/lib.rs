// VeloTerm vendor 修改：新增 `pub mod diff;`（上游 diff 模块只在 bin 的 main.rs
// 中声明，lib 不导出，外部无法调用 diff 的 CLI 逻辑）与 `pub mod sdiff;`
// （VeloTerm vendor 新增文件，上游无 sdiff 命令）。
pub mod cmp;
pub mod context_diff;
pub mod diff;
pub mod ed_diff;
pub mod macros;
pub mod normal_diff;
pub mod params;
pub mod sdiff;
pub mod side_diff;
pub mod unified_diff;
pub mod utils;

// Re-export the public functions/types you need
pub use context_diff::diff as context_diff;
pub use ed_diff::diff as ed_diff;
pub use normal_diff::diff as normal_diff;
pub use side_diff::diff as side_by_side_diff;
pub use unified_diff::diff as unified_diff;

// ===== VeloTerm vendor 新增（上游无以下内容）=====
// 为 VeloTerm coreutils 多呼叫二进制提供 diff / cmp / sdiff 三个命令入口，
// 风格与 uucore 的 `uumain(impl uucore::Args) -> i32` 一致：迭代器首元素为
// 命令名（作为诊断信息里的 executable），其余为参数，可从外部注入，因此可
// 走主工程的通用 match 分发臂（构造 [命令名, 参数...] 后传入）。
// 退出码与 GNU diffutils 相同：0=相同，1=有差异，2=出错。

use std::ffi::OsString;
use std::process::ExitCode;

/// `diff` 命令入口：`diffutilslib::diff_main(["diff", ...参数]) -> i32`
pub fn diff_main(args: impl Iterator<Item = OsString>) -> i32 {
    exit_code_to_i32(diff::main(args.peekable()))
}

/// `cmp` 命令入口：`diffutilslib::cmp_main(["cmp", ...参数]) -> i32`
pub fn cmp_main(args: impl Iterator<Item = OsString>) -> i32 {
    exit_code_to_i32(cmp::main(args.peekable()))
}

/// `sdiff` 命令入口：`diffutilslib::sdiff_main(["sdiff", ...参数]) -> i32`
pub fn sdiff_main(args: impl Iterator<Item = OsString>) -> i32 {
    exit_code_to_i32(sdiff::main(args.peekable()))
}

/// `ExitCode` 没有稳定的取值 API，用等值比较还原 0-255 的退出码
/// （本 crate 的退出码均经 `ExitCode::SUCCESS` 或 `ExitCode::from(u8)`
/// 构造）。
fn exit_code_to_i32(code: ExitCode) -> i32 {
    for v in 0..=255u8 {
        if code == ExitCode::from(v) {
            return i32::from(v);
        }
    }
    0
}
