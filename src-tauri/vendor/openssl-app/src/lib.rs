//! 仅 Windows：FFI 调 vcpkg OpenSSL 静态库 + 自编译 apps 的 CLI main。
//!
//! build.rs（仅 `target_os = windows` 生效）会用 cc 把 OpenSSL 源码 apps\*.c +
//! apps\lib\*.c（含 perl 现场生成的 apps\progs.c/h）编译进本 crate，全部统一
//! `-Dmain=veloterm_openssl_main` 改名；libcrypto/libssl 链接自 vcpkg
//! x64-windows-static 安装产物（与 SQLCipher 共用同一份，见 build.rs 头注释）。
//!
//! 其他平台不编译任何 C 代码，`openssl_cmd_main` 是打印提示并返回 2 的桩。

#[cfg(windows)]
mod imp {
    use std::ffi::{c_char, c_int, CString, OsString};

    // apps/openssl.c 的 main 经 -Dmain=veloterm_openssl_main 改名后的 C 入口。
    // 签名即 `int main(int argc, char *argv[])`，按 C ABI 调用。
    // edition 2024 起 `extern` 块必须标 `unsafe`（RFC 3484）。
    unsafe extern "C" {
        fn veloterm_openssl_main(argc: c_int, argv: *const *const c_char) -> c_int;
    }

    /// openssl CLI 入口：`args` 首元素约定为程序名（"openssl"），其后为子命令
    /// 与参数，等价于直接执行 `openssl <args[1..]>`。返回其退出码，由外层
    /// coreutils 分发决定进程退出。
    pub fn openssl_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        // Windows 下 OsString 内部是任意 UTF-16，可能含无效代理对；CLI 参数
        // 场景用 to_string_lossy 折损成 UTF-8 字节已足够（注释说明，不做
        // OsStrExt 折腾）。内嵌 NUL（U+0000）进不了 C 字符串：降级为空串，
        // 不 panic —— 上游按空参数处理比崩溃更合理。
        let argv: Vec<CString> = args
            .into_iter()
            .map(|a| CString::new(a.to_string_lossy().into_owned()).unwrap_or_default())
            .collect();
        // C 约定 argv[argc] == NULL：必须补哨兵。裸 `openssl` 时上游 main 会
        // argc-- / argv++ 后取 argv[0]（opt_appname 对 NULL 安全跳过），缺哨兵
        // 则越界读堆内存 —— 未定义行为（实测表现为卡死/崩溃/乱码帮助）。
        let mut ptrs: Vec<*const c_char> = argv.iter().map(|s| s.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        // SAFETY：ptrs（含末尾 NULL 哨兵）与其指向的 CString 在 FFI 调用期间
        // 全程存活；veloterm_openssl_main 是上游 main，按约定只读 argc/argv，
        // 不在返回后继续持有指针（win32_utf8argv 默认 no-op，见 openssl.c）。
        unsafe { veloterm_openssl_main((ptrs.len() - 1) as c_int, ptrs.as_ptr()) }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::ffi::OsString;

    /// 非 Windows 桩：本 crate 的 C 侧只在 Windows 构建，其他平台调用即
    /// 提示并返回 2（命令不可用的 CLI 退出惯例）。
    pub fn openssl_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let _ = args.into_iter().count();
        eprintln!("openssl 仅在 Windows 构建");
        2
    }
}

pub use imp::openssl_cmd_main;

#[cfg(all(test, not(windows)))]
mod tests {
    use super::openssl_cmd_main;
    use std::ffi::OsString;

    #[test]
    fn stub_returns_2() {
        let args = vec![OsString::from("openssl"), OsString::from("version")];
        assert_eq!(openssl_cmd_main(args), 2);
    }

    #[test]
    fn stub_accepts_empty_iter() {
        let n: i32 = openssl_cmd_main(std::iter::empty());
        assert_eq!(n, 2);
    }

    #[test]
    fn signature_accepts_generic_iters() {
        // 入口约定：首元素为程序名；这里验证迭代器泛型（Vec/iter 均可）
        let v: Vec<OsString> = vec!["openssl".into()];
        assert_eq!(openssl_cmd_main(v.into_iter()), 2);
    }
}
