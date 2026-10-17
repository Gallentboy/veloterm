//! 仅 Windows：FFI 调现场编译的 jq CLI main。
//!
//! build.rs（仅 `target_os = windows` 生效）用 cc 把 vendor/jq（1.8.2）本体
//! + decNumber + 捆绑 oniguruma 编进本 crate。jq 在 WIN32 下的 main.c 只有
//! `wmain`（宽字符入口）与 `umain`（真实逻辑，UTF-8 char**，wmain 只是它的
//! WideCharToMultiByte 前置转换）；本 crate 越过 wmain 直调 umain——
//! `-Dumain=veloterm_jq_main` 改名后由下面的 extern 声明引用，Rust 侧传入的
//! 本就是 UTF-8 参数，与 wmain 的转换职责等价。
//!
//! 其他平台不编译任何 C 代码，`jq_cmd_main` 是打印提示并返回 2 的桩。

#[cfg(windows)]
mod imp {
    use std::ffi::{c_char, c_int, CString, OsString};

    // jq main.c 的 umain 经 -Dumain=veloterm_jq_main 改名后的 C 入口。
    // 签名即 `int umain(int argc, char *argv[])`，按 C ABI 调用。
    // edition 2024 起 `extern` 块必须标 `unsafe`（RFC 3484）。
    unsafe extern "C" {
        fn veloterm_jq_main(argc: c_int, argv: *const *const c_char) -> c_int;
    }

    /// jq CLI 入口：`args` 首元素约定为程序名（"jq"），其后为过滤式与参数，
    /// 等价于直接执行 `jq <args[1..]>`。返回其退出码，由外层 coreutils
    /// 分发决定进程退出。
    pub fn jq_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        // Windows 下 OsString 内部是任意 UTF-16，可能含无效代理对；CLI 参数
        // 场景用 to_string_lossy 折损成 UTF-8 字节已足够（umain 即按 UTF-8
        // 消费）。内嵌 NUL（U+0000）进不了 C 字符串：降级为空串，不 panic
        // —— 上游按空参数处理比崩溃更合理。
        let argv: Vec<CString> = args
            .into_iter()
            .map(|a| CString::new(a.to_string_lossy().into_owned()).unwrap_or_default())
            .collect();
        // C 约定 argv[argc] == NULL：必须补哨兵。jq 的 umain 以 argv[0] 为
        // progname、逐下标访问到 argc 为止；缺哨兵的越界读是未定义行为
        // （同因曾修过 openssl-app 的真实 bug，见其 lib.rs 注释）。
        let mut ptrs: Vec<*const c_char> = argv.iter().map(|s| s.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        // SAFETY：ptrs（含末尾 NULL 哨兵）与其指向的 CString 在 FFI 调用期间
        // 全程存活；veloterm_jq_main 是上游 umain，按约定只读 argc/argv，
        // 不在返回后继续持有指针（JQ_ORIGIN 取值发生在返回前的调用内）。
        unsafe { veloterm_jq_main((ptrs.len() - 1) as c_int, ptrs.as_ptr()) }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::ffi::OsString;

    /// 非 Windows 桩：本 crate 的 C 侧只在 Windows 构建，其他平台调用即
    /// 提示并返回 2（命令不可用的 CLI 退出惯例）。
    pub fn jq_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let _ = args.into_iter().count();
        eprintln!("jq 仅在 Windows 构建");
        2
    }
}

pub use imp::jq_cmd_main;

/// build.rs 的纯函数不变量（builtin.inc 渲染 / 源清单 / 垫片形状等）原本只在
/// build 脚本的 `#[cfg(test)]` 里——cargo 从不执行 build 脚本的测试，vendor 又
/// 被 workspace exclude，这些防线实际从未运行过。经 `#[path]` 把 build.rs
/// 纳入 lib 测试编译，`cargo test`（vendor 目录内独立跑，见 AGENTS.md 门禁
/// 约定）即可执行全部用例；`#[allow(dead_code)]` 放行 fn main 等测试外符号。
#[cfg(test)]
#[allow(dead_code)]
#[path = "../build.rs"]
mod build_script_tests;

#[cfg(all(test, not(windows)))]
mod tests {
    use super::jq_cmd_main;
    use std::ffi::OsString;

    #[test]
    fn stub_returns_2() {
        let args = vec![OsString::from("jq"), OsString::from("--version")];
        assert_eq!(jq_cmd_main(args), 2);
    }

    #[test]
    fn stub_accepts_empty_iter() {
        let n: i32 = jq_cmd_main(std::iter::empty());
        assert_eq!(n, 2);
    }

    #[test]
    fn signature_accepts_generic_iters() {
        // 入口约定：首元素为程序名；这里验证迭代器泛型（Vec/iter 均可）
        let v: Vec<OsString> = vec!["jq".into()];
        assert_eq!(jq_cmd_main(v.into_iter()), 2);
    }
}
