//! yq（mikefarah/yq v4，Go 实现）的内置适配：Windows 上运行时加载
//! `yqffi.dll`，调其 C 导出入口 `veloterm_yq_main`。
//!
//! # 为什么是 DLL，而不是像 openssl-app 那样 cc 静态链入
//!
//! Go 官方不支持把 `buildmode=c-archive` 的产物交给 MSVC link.exe
//! （golang/go#74189，跟踪议题 #20982 多年未动），而本工程 Windows 工具链
//! 是纯 MSVC —— `buildmode=c-shared` 出的 DLL 是 **extern 链接在 MSVC 下
//! 对 Go 源码的唯一形态**。DLL 由 `scripts/build-tools.mjs` 在 Windows 上
//! `go build -buildmode=c-shared -tags veloterm_ffi` 现场产出（源码与版本锁
//! 在 `vendor/yq/`，见其 VELOTERM_VERSION 与 veloterm_ffi.go），经
//! `tauri.windows.conf.json` 的 resources 随包放在安装根目录。
//!
//! # 为什么运行时 LoadLibrary，而不是链接期导入库
//!
//! coreutils 的构建/测试机（macOS 开发机、CI）不必装 Go 工具链与 DLL：
//! 无链接期依赖，DLL 缺失时给出可诊断的提示并按「命令不可用」惯例返回 2，
//! 而不是让整个 crate 无法编译。
//!
//! # DLL 搜索路径
//!
//! 安装后 coreutils.exe 与 yqffi.dll 同在安装根目录；但用户经 `tools\yq.exe`
//! 硬链接短名启动时，进程 exe 目录是 `tools\` 子目录（PE 加载器的默认 DLL
//! 搜索目录跟着进程 exe 走），故按绝对路径依次尝试：exe 同目录 → exe 上级
//! 目录（tools\ 场景的安装根）。

#[cfg(windows)]
mod imp {
    use std::ffi::{c_char, c_int, CString, OsString};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use libloading::Library;

    /// yqffi.dll 导出的 C 入口签名（Go 侧 //export veloterm_yq_main）。
    type YqMainFn = unsafe extern "C" fn(c_int, *const *const c_char) -> c_int;

    /// 进程内首次调用加载一次；错误串缓存避免每次调用重复打日志。
    static YQ_MAIN: OnceLock<Result<YqMainFn, String>> = OnceLock::new();

    /// DLL 候选路径（纯函数，单测覆盖）：exe 同目录 → exe 上级目录。
    fn candidates_for(exe: Option<&Path>) -> Vec<PathBuf> {
        let dir = exe?.parent()?;
        let mut v = vec![dir.join("yqffi.dll")];
        if let Some(parent) = dir.parent() {
            v.push(parent.join("yqffi.dll"));
        }
        v
    }

    fn load_yq_main() -> Result<YqMainFn, String> {
        let candidates = candidates_for(std::env::current_exe().ok().as_deref());
        let mut tried = Vec::new();
        for path in candidates {
            // SAFETY：DLL 只含 Go 运行时与 yq，无 DllMain 副作用；LoadLibrary
            // 的 Windows API 调用本身对进程状态无破坏性
            match unsafe { Library::new(&path) } {
                Ok(lib) => {
                    // Library 与符号同生命周期：进程内只加载一次，leak 无害
                    let lib: &'static Library = Box::leak(Box::new(lib));
                    // SAFETY：veloterm_yq_main 是 C ABI 导出符号，签名见 YqMainFn
                    let sym = unsafe { lib.get::<YqMainFn>(b"veloterm_yq_main\0") }
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

    /// yq 入口：`args` 首元素约定为程序名（"yq"），其后为子命令与参数，
    /// 等价于直接执行 `yq <args[1..]>`。返回其退出码。
    pub fn yq_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let cached = YQ_MAIN.get_or_init(load_yq_main);
        let Some(yq_main) = cached.as_ref().copied() else {
            let msg = cached.as_ref().err().map(String::as_str).unwrap_or("");
            eprintln!("yq: 未找到 yqffi.dll（{msg}）。");
            eprintln!("yqffi.dll 随安装包分发；开发模式请先运行 node scripts/build-tools.mjs 构建。");
            return 2;
        };

        // Windows 下 OsString 内部是任意 UTF-16；CLI 参数场景用 to_string_lossy
        // 折损成 UTF-8 已足够（与 openssl-app 同一取舍）。内嵌 NUL 进不了
        // C 字符串：降级为空串，不 panic。
        let argv: Vec<CString> = args
            .into_iter()
            .map(|a| CString::new(a.to_string_lossy().into_owned()).unwrap_or_default())
            .collect();
        let mut ptrs: Vec<*const c_char> = argv.iter().map(|s| s.as_ptr()).collect();
        // C 约定 argv[argc] == NULL：补哨兵防指针数组越界读
        // （与 openssl-app / putty-telnet 的 FFI 桥同一修复）
        ptrs.push(std::ptr::null());
        // SAFETY：ptrs（含末尾 NULL 哨兵）与其指向的 CString 在 FFI 调用期间
        // 全程存活；Go 侧只按 argc 读取，返回后不持有指针
        unsafe { yq_main((ptrs.len() - 1) as c_int, ptrs.as_ptr()) }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::ffi::OsString;

    /// 非 Windows 桩：yq 只随 Windows 安装包分发，其他平台调用即提示并
    /// 返回 2（命令不可用的 CLI 退出惯例，同 openssl/telnet 桩）。
    /// 分发臂是 cfg(windows)，本桩在 mac 构建中无人调用——允许 dead_code。
    #[allow(dead_code)]
    pub fn yq_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let _ = args.into_iter().count();
        eprintln!("yq 仅在 Windows 构建");
        2
    }
}

// bin crate：分发臂直接用 crate::yq_impl::yq_cmd_main 全路径调用，不做 pub use
// （bin 无外部消费者，re-export 会触发 unused_imports）

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::imp::candidates_for;
    #[cfg(windows)]
    use std::path::Path;

    /// 硬链接短名场景：exe 在 tools\ 子目录 → 候选含上级目录（安装根）
    #[test]
    #[cfg(windows)]
    fn dll_candidates_cover_hardlink_shortname_dir() {
        let exe = Path::new(r"C:\app\tools\yq.exe");
        let c = candidates_for(Some(exe));
        assert_eq!(c[0], Path::new(r"C:\app\tools\yqffi.dll"));
        assert_eq!(c[1], Path::new(r"C:\app\yqffi.dll"));
    }

    /// coreutils 本体直跑场景：exe 在安装根 → 首候选即同目录
    #[test]
    #[cfg(windows)]
    fn dll_candidates_cover_install_root() {
        let exe = Path::new(r"C:\app\coreutils.exe");
        let c = candidates_for(Some(exe));
        assert_eq!(c[0], Path::new(r"C:\app\yqffi.dll"));
        assert_eq!(c.len(), 2);
    }

    /// 进程路径不可得时不 panic，返回空候选
    #[test]
    #[cfg(windows)]
    fn dll_candidates_empty_without_exe() {
        assert!(candidates_for(None).is_empty());
    }
}
