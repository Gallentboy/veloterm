//! 仅 Windows：FFI 调现场编译的 bc CLI main。
//!
//! build.rs（仅 `target_os = "windows"` 生效）用 cc 把 vendor/bc（7.1.0）
//! 本体 + 4 个 strgen 生成物编进本 crate。bc 的 `src/main.c` 入口经
//! `-Dmain=veloterm_bc_main` 改名后由下面的 extern 声明引用——它内部按
//! argv[0] 分发 bc/dc（`vm->name[0] != 'd'` 即 bc），本入口恒传首参 "bc"，
//! 与上游 vcxproj 的「同一 exe 复制出 bc.exe/dc.exe 双名字」形态等价；
//! 参数解析器 `bc_opt_init` 置 optind=1，argv[0] 按程序名跳过。
//!
//! 其他平台不编译任何 C 代码，`bc_cmd_main` 是打印提示并返回 2 的桩。

#[cfg(windows)]
mod imp {
    use std::ffi::{c_char, c_int, CString, OsString};

    // bc src/main.c 的 main 经 -Dmain=veloterm_bc_main 改名后的 C 入口。
    // 签名即 `int main(int argc, const char* argv[])`，按 C ABI 调用。
    // edition 2024 起 `extern` 块必须标 `unsafe`（RFC 3484）。
    unsafe extern "C" {
        fn veloterm_bc_main(argc: c_int, argv: *const *const c_char) -> c_int;
    }

    /// bc CLI 入口：`args` 首元素约定为程序名（恒 "bc"，决定 main.c 内部
    /// 走 bc_main 而非 dc_main），其后为真实参数，等价于直接执行
    /// `bc <args[1..]>`。返回其退出码，由外层 coreutils 分发决定进程退出。
    pub fn bc_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        // Windows 下 OsString 内部是任意 UTF-16，可能含无效代理对；CLI 参数
        // 场景用 to_string_lossy 折损成 UTF-8 字节已足够（bc 按 UTF-8 消费，
        // 读入后自行做 UTF-8 校验）。内嵌 NUL（U+0000）进不了 C 字符串：
        // 降级为空串，不 panic——上游按空参数处理比崩溃更合理
        // （与 jq-app 同策略）。
        let argv: Vec<CString> = args
            .into_iter()
            .map(|a| CString::new(a.to_string_lossy().into_owned()).unwrap_or_default())
            .collect();
        // C 约定 argv[argc] == NULL：必须补哨兵。bc 的 main 以 argv[0] 取
        // 程序名（vm->name），逐下标访问到 argc 为止；缺哨兵的越界读是
        // 未定义行为。
        let mut ptrs: Vec<*const c_char> = argv.iter().map(|s| s.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        // SAFETY：ptrs（含末尾 NULL 哨兵）与其指向的 CString 在 FFI 调用期间
        // 全程存活；veloterm_bc_main 是上游 main，按约定只读 argc/argv，
        // 不在返回后继续持有指针（bc_vm_atexit 收尾发生在返回前）。
        unsafe { veloterm_bc_main((ptrs.len() - 1) as c_int, ptrs.as_ptr()) }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::ffi::OsString;

    /// 非 Windows 桩：本 crate 的 C 侧只在 Windows 构建，其他平台调用即
    /// 提示并返回 2（命令不可用的 CLI 退出惯例）。
    pub fn bc_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let _ = args.into_iter().count();
        eprintln!("bc 仅在 Windows 构建");
        2
    }
}

pub use imp::bc_cmd_main;

/// build.rs 的纯函数不变量（strgen 生成物复刻 / 宏清单对齐 vcxproj 等）原本
/// 只在 build 脚本的 `#[cfg(test)]` 里——cargo 从不执行 build 脚本的测试，
/// vendor 又被 workspace exclude，这些防线实际从未运行过。经 `#[path]` 把
/// build.rs 纳入 lib 测试编译，`cargo test`（vendor 目录内独立跑，见
/// AGENTS.md 门禁约定）即可执行全部用例；`#[allow(dead_code)]` 放行 fn main
/// 等测试外符号。strgen 复刻的正确性防线此前形同虚设，本模块使其生效。
#[cfg(test)]
#[allow(dead_code)]
#[path = "../build.rs"]
mod build_script_tests;

#[cfg(all(test, not(windows)))]
mod tests {
    use super::bc_cmd_main;
    use std::ffi::OsString;

    #[test]
    fn stub_returns_2() {
        let args = vec![OsString::from("bc"), OsString::from("--version")];
        assert_eq!(bc_cmd_main(args), 2);
    }

    #[test]
    fn stub_accepts_empty_iter() {
        let n: i32 = bc_cmd_main(std::iter::empty());
        assert_eq!(n, 2);
    }

    #[test]
    fn signature_accepts_generic_iters() {
        // 入口约定：首元素为程序名；这里验证迭代器泛型（Vec/iter 均可）
        let v: Vec<OsString> = vec!["bc".into()];
        assert_eq!(bc_cmd_main(v.into_iter()), 2);
    }
}
