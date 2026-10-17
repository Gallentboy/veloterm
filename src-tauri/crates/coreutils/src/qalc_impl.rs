//! qalc（Qalculate! 的 CLI，libqalculate 5.13.1，C++ 实现）的内置适配：
//! Windows 上运行时加载 `qalcffi.dll`，调其 C 导出入口
//! `veloterm_qalc_main`。
//!
//! # 为什么是 DLL，而不是像 jq-app 那样 cc 静态链入
//!
//! libqalculate 是 autotools 体系的 C++ 库（GMP/MPFR/libxml2 硬依赖），
//! 上游无 MSVC 工程且官方 Windows 版即 MinGW 构建——本工程的纯 MSVC
//! 工具链无法像纯 C 自包含的 jq 那样现场编译链入。故与 yq 同一形态：
//! 源码锁在 `vendor/libqalculate/`，由 `scripts/build-tools.mjs` 在
//! Windows 上经 MSYS2 MinGW64 环境 autotools 构建静态库后连同 FFI 壳
//! （`veloterm_ffi.cc`，qalc.cc 以 `-Dmain=qalc_main` 改名直调）链接成
//! `qalcffi.dll`（依赖全静态打入，见 vendor README），经
//! `tauri.windows.conf.json` 的 resources 随包放在安装根 `lib/` 子目录。
//!
//! # 为什么运行时 LoadLibrary，而不是链接期导入库
//!
//! 与 yqffi 同理：coreutils 的构建/测试机（macOS 开发机、CI）不必装
//! MSYS2 工具链与 DLL——无链接期依赖，DLL 缺失时给出可诊断的提示并按
//! 「命令不可用」惯例返回 2。
//!
//! # DLL 搜索路径
//!
//! 候选顺序（同目录 → 上级 lib/ → 上级）由共享模块 `crate::dll_paths` 统一
//! 生成，三个 DLL 例外形态（goffi / qalc / nmap）同一套约定，依据见其文件头。
//!
//! # 输出编码
//!
//! 上游在 Windows 下对 Unicode 输出走宽字符/iconv 转码路径，在
//! VeloTerm 的 UTF-8 终端（xterm/ConPTY 管道）里会乱码——vendor 内已打
//! 补丁让输出恒按 UTF-8 字节写出（搜 qalc.cc 的 `VeloTerm patch`）。

#[cfg(windows)]
mod imp {
    use std::ffi::{c_char, c_int, CString, OsString};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use libloading::Library;

    /// qalcffi.dll 导出的 C 入口签名（vendor/libqalculate/veloterm_ffi.cc）。
    type QalcMainFn = unsafe extern "C" fn(c_int, *const *const c_char) -> c_int;

    /// 进程内首次调用加载一次；错误串缓存避免每次调用重复打日志。
    static QALC_MAIN: OnceLock<Result<QalcMainFn, String>> = OnceLock::new();

    /// DLL 候选路径（共享约定，见 crate::dll_paths）：同目录 → 上级 lib/ → 上级。
    pub(super) fn candidates_for(exe: Option<&Path>) -> Vec<PathBuf> {
        crate::dll_paths::dll_candidates(exe, "qalcffi.dll")
    }

    fn load_qalc_main() -> Result<QalcMainFn, String> {
        let candidates = candidates_for(std::env::current_exe().ok().as_deref());
        let mut tried = Vec::new();
        for path in candidates {
            // SAFETY：DLL 只含 libqalculate 与静态依赖，无 DllMain 副作用；
            // LoadLibrary 的 Windows API 调用本身对进程状态无破坏性
            match unsafe { Library::new(&path) } {
                Ok(lib) => {
                    // Library 与符号同生命周期：进程内只加载一次，leak 无害
                    let lib: &'static Library = Box::leak(Box::new(lib));
                    // SAFETY：veloterm_qalc_main 是 C ABI 导出符号，签名见 QalcMainFn
                    let sym = unsafe { lib.get::<QalcMainFn>(b"veloterm_qalc_main\0") }
                        .map_err(|e| format!("{} 已加载但缺少导出符号: {e}", path.display()))?;
                    return Ok(*sym);
                }
                Err(e) => tried.push(format!("{}（{e}）", path.display())),
            }
        }
        Err(if tried.is_empty() {
            "无法定位当前进程路径".to_string()
        } else {
            tried.join("；")
        })
    }

    /// qalc 入口：`args` 首元素约定为程序名（"qalc"），其后为表达式与
    /// 选项，等价于直接执行 `qalc <args[1..]>`。返回其退出码。
    pub fn qalc_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let cached = QALC_MAIN.get_or_init(load_qalc_main);
        let Ok(qalc_main) = cached.as_ref().copied() else {
            let msg = cached.as_ref().err().map(String::as_str).unwrap_or("");
            eprintln!("qalc: qalcffi.dll 不可用（{msg}）");
            eprintln!("qalcffi.dll 随安装包分发；开发模式请先运行 node scripts/build-tools.mjs 构建。");
            return 2;
        };

        // Windows 下 OsString 内部是任意 UTF-16；CLI 参数场景用 to_string_lossy
        // 折损成 UTF-8 已足够（与 yq/openssl-app 同一取舍）。内嵌 NUL 进不了
        // C 字符串：降级为空串，不 panic。
        let argv: Vec<CString> = args
            .into_iter()
            .map(|a| CString::new(a.to_string_lossy().into_owned()).unwrap_or_default())
            .collect();
        let mut ptrs: Vec<*const c_char> = argv.iter().map(|s| s.as_ptr()).collect();
        // C 约定 argv[argc] == NULL：补哨兵防指针数组越界读
        // （与 yq/openssl-app/putty-telnet 的 FFI 桥同一约定）
        ptrs.push(std::ptr::null());
        // SAFETY：ptrs（含末尾 NULL 哨兵）与其指向的 CString 在 FFI 调用期间
        // 全程存活；qalc 侧只按 argc 读取，返回后不持有指针
        unsafe { qalc_main((ptrs.len() - 1) as c_int, ptrs.as_ptr()) }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::ffi::OsString;

    /// 非 Windows 桩：qalc 只随 Windows 安装包分发，其他平台调用即提示并
    /// 返回 2（命令不可用的 CLI 退出惯例，同 yq/openssl/telnet 桩）。
    /// 分发臂是 cfg(windows)，本桩在 mac 构建中无人调用——允许 dead_code。
    #[allow(dead_code)]
    pub fn qalc_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let _ = args.into_iter().count();
        eprintln!("qalc 仅在 Windows 构建");
        2
    }
}

// bin crate：分发臂按 crate::qalc_impl::qalc_cmd_main 全路径调用；
// qalc_cmd_main 定义在 cfg 分家的 imp 子模块里，需顶层 re-export。
// cfg 限定 windows：非 Windows 构建没有调用方，无条件 re-export 反而
// 触发 unused_imports。
#[cfg(windows)]
pub use imp::qalc_cmd_main;

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::imp::candidates_for;
    #[cfg(windows)]
    use std::path::Path;

    /// 硬链接短名场景：exe 在 tools\ 子目录 → 上级 lib/ 是安装后的 DLL 位置
    #[test]
    #[cfg(windows)]
    fn dll_candidates_cover_hardlink_shortname_dir() {
        let exe = Path::new(r"C:\app\tools\qalc.exe");
        let c = candidates_for(Some(exe));
        assert_eq!(c[1], Path::new(r"C:\app\lib\qalcffi.dll"));
    }

    /// coreutils 本体直跑场景：exe 在安装根 → 本级 lib/ 命中且排在平铺之前
    #[test]
    #[cfg(windows)]
    fn dll_candidates_cover_install_root() {
        let exe = Path::new(r"C:\app\coreutils.exe");
        let c = candidates_for(Some(exe));
        assert_eq!(c[0], Path::new(r"C:\app\lib\qalcffi.dll"));
        assert_eq!(c[2], Path::new(r"C:\app\qalcffi.dll"));
        assert_eq!(c.len(), 3);
    }

    /// 进程路径不可得时不 panic，返回空候选
    #[test]
    #[cfg(windows)]
    fn dll_candidates_empty_without_exe() {
        assert!(candidates_for(None).is_empty());
    }
}
