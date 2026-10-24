//! veloterm 分叉新增：连接/认证阶段的详细日志（环境变量 `OB_DEBUG=1` 开启，
//! 输出到 stderr）。真机排查用——auth 分派走了哪条臂、每个包的类型与前缀
//! hex、OK 包解析成败与 status/能力回显结果，一次跑齐；不开零开销
//!（`enabled()` OnceLock 缓存，`format_args!` 惰性求值）。上游无此模块，
//! 升级换树时随分叉补丁保留。

use std::sync::OnceLock;

static ENABLED: OnceLock<bool> = OnceLock::new();

pub(crate) fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        std::env::var("OB_DEBUG")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false)
    })
}

pub(crate) fn log(args: std::fmt::Arguments<'_>) {
    eprintln!("[ob-dbg] {args}");
}

/// 前 `max` 字节的原始终十六进制（空格分隔）——raw 包 dump 用，
/// 手工核对 lenenc/status 偏移时直接看字节
pub(crate) fn hex(bytes: &[u8], max: usize) -> String {
    bytes
        .iter()
        .take(max)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 详细日志宏：`obdbg!("switch_to_ob20: echo={echo:#x}")`。
/// 仅在 `OB_DEBUG` 开启时格式化求值（参数惰性，未开启零分配）。
#[macro_export]
macro_rules! obdbg {
    ($($arg:tt)*) => {
        if $crate::obdbg::enabled() {
            $crate::obdbg::log(format_args!($($arg)*));
        }
    };
}
