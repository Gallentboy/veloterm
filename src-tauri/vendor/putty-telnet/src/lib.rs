//! 仅 Windows：源码级内嵌 PuTTY（MIT）telnet 客户端的 `telnet` 命令。
//!
//! build.rs（仅 `target_os = windows` 生效）会用 cc 把 vendored PuTTY
//! 源码树（putty/，master @ 7507c54b，0.85 开发版）中「plink console 驱动
//! + telnet 后端」的最小源文件集与本 crate 的 c_src/telnet_main.c 编译
//! 进本 crate。telnet_main.c 派生自 windows/plink.c，其
//! `putty_telnet_main` 在 C 层原生解析 GNU/inetutils 风格的 telnet 参数
//! （不做 Rust 侧参数翻译壳），会话循环复用 plink 的 console 主循环。
//!
//! 注意：与 openssl-app 同款事实——PuTTY 的收尾走 C 层 `exit()`（经由
//! cleanup_exit），因此 telnet_cmd_main 在连接建立后的正常路径上是
//! 「以 C 侧退出码终止进程」而非把返回值交回 Rust 分发层；两者最终
//! 的进程退出语义一致。
//!
//! 其他平台不编译任何 C 代码，`telnet_cmd_main` 是打印提示并返回 2 的桩。

#[cfg(windows)]
mod imp {
    use std::ffi::{c_char, c_int, CString, OsString};

    // c_src/telnet_main.c 的入口：标准 telnet 参数（GNU/inetutils 风格）
    // 在 C 层解析后直接驱动 PuTTY telnet 会话。签名即
    // `int putty_telnet_main(int argc, char **argv)`，按 C ABI 调用；
    // args 首元素约定为程序名（"telnet"），其后为参数。
    extern "C" {
        fn putty_telnet_main(argc: c_int, argv: *const *const c_char) -> c_int;
    }

    /// telnet CLI 入口：`args` 首元素约定为程序名（"telnet"），等价于
    /// 直接执行 `telnet <args[1..]>`。返回其退出码（连接建立后的正常
    /// 收尾在 C 层 exit，见模块注释）。
    pub fn telnet_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        // Windows 下 OsString 内部是任意 UTF-16，可能含无效代理对；CLI 参数
        // 场景用 to_string_lossy 折损成 UTF-8 字节已足够。内嵌 NUL（U+0000）
        // 进不了 C 字符串：降级为空串，不 panic —— 上游按空参数处理比崩溃
        // 更合理（与 openssl-app 的取舍一致）。
        let argv: Vec<CString> = args
            .into_iter()
            .map(|a| CString::new(a.to_string_lossy().into_owned()).unwrap_or_default())
            .collect();
        let ptrs: Vec<*const c_char> = argv.iter().map(|s| s.as_ptr()).collect();
        // SAFETY：ptrs 与其指向的 CString 在 FFI 调用期间全程存活；
        // putty_telnet_main 只读 argc/argv，不在返回后继续持有指针。
        unsafe { putty_telnet_main(ptrs.len() as c_int, ptrs.as_ptr()) }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::ffi::OsString;

    /// 非 Windows 桩：本 crate 的 C 侧只在 Windows 构建，其他平台调用即
    /// 提示并返回 2（命令不可用的 CLI 退出惯例）。
    pub fn telnet_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let _ = args.into_iter().count();
        eprintln!("telnet 仅在 Windows 构建");
        2
    }
}

pub use imp::telnet_cmd_main;

#[cfg(all(test, not(windows)))]
mod tests {
    use super::telnet_cmd_main;
    use std::ffi::OsString;

    #[test]
    fn stub_returns_2() {
        let args = vec![
            OsString::from("telnet"),
            OsString::from("host.example.com"),
        ];
        assert_eq!(telnet_cmd_main(args), 2);
    }

    #[test]
    fn stub_accepts_empty_iter() {
        let n: i32 = telnet_cmd_main(std::iter::empty());
        assert_eq!(n, 2);
    }

    #[test]
    fn signature_accepts_generic_iters() {
        // 入口约定：首元素为程序名；这里验证迭代器泛型（Vec/iter 均可）
        let v: Vec<OsString> = vec!["telnet".into()];
        assert_eq!(telnet_cmd_main(v.into_iter()), 2);
    }
}
