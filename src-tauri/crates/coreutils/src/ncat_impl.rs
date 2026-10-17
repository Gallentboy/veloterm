//! ncat（Nmap 项目自带的 netcat 替代，C 实现）的内置适配：Windows 上运行时
//! 加载 `ncatffi.dll`，调其 C 导出入口 `veloterm_ncat_main`。
//!
//! # 形态（与 nmap/qalc 同型）
//!
//! 源码锁在 `vendor/nmap/ncat/`，由 `veloterm-win/CMakeLists.txt` 经 MSVC 编成
//! 自包含 `ncatffi.dll`（链 Npcap SDK 的 wpcap.lib + OpenSSL 静态库 + 树内
//! nbase/nsock/liblua；见 vendor/nmap/README.md）。入口由 `ncat_ffi.cc` 以
//! `#define main` 改名包含 ncat_main.c 提供。
//!
//! ncat 是独立可执行程序，其 main 内有 exit() 调用——coreutils 是「单命令
//! 生命周期进程」，exit() 等价于返回退出码（宿主随之结束），无需改造。
//!
//! # 为什么运行时 LoadLibrary
//!
//! 与 yqffi/qalcffi/nmapffi 同理：构建/测试机（macOS、CI）不必有 MSVC 工具链
//! 与 DLL——无链接期依赖，DLL 缺失时给出可诊断提示并按「命令不可用」返回 2。
//!
//! # DLL 搜索路径
//!
//! 与 nmapffi 同一套约定：安装后 coreutils.exe 与 ncatffi.dll 同在安装根；
//! 用户经 `tools\ncat.exe` 硬链接短名启动时按 exe 同目录 → 上级目录搜索。
//!
//! # 数据文件
//!
//! ncat 的 `ca-bundle.crt`（SSL 客户端证书验证）在 **exe 同目录**查找，缺失
//! 仅 debug 日志告警、不致命。DLL 形态下 `GetModuleHandle(0)` 取到宿主
//! coreutils.exe 路径，故该文件随包放安装根即可命中（与 nmap-data 同层）。

#[cfg(windows)]
mod imp {
    use std::ffi::{c_char, c_int, CString, OsString};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use libloading::Library;

    /// ncatffi.dll 导出的 C 入口签名（veloterm-win/ncat_ffi.cc）。
    type NcatMainFn = unsafe extern "C" fn(c_int, *const *const c_char) -> c_int;

    static NCAT_MAIN: OnceLock<Result<NcatMainFn, String>> = OnceLock::new();

    /// DLL 候选路径（纯函数，单测覆盖）：exe 同目录 → exe 上级目录。
    pub(super) fn dll_candidates_for(exe: Option<&Path>) -> Vec<PathBuf> {
        let Some(dir) = exe.and_then(|p| p.parent()) else {
            return Vec::new();
        };
        let mut v = vec![dir.join("ncatffi.dll")];
        if let Some(parent) = dir.parent() {
            v.push(parent.join("ncatffi.dll"));
        }
        v
    }

    fn load_ncat_main() -> Result<NcatMainFn, String> {
        let candidates = dll_candidates_for(std::env::current_exe().ok().as_deref());
        let mut tried = Vec::new();
        for path in candidates {
            // SAFETY：DLL 只含 ncat 与静态依赖，无 DllMain 副作用
            match unsafe { Library::new(&path) } {
                Ok(lib) => {
                    // Library 与符号同生命周期：进程内只加载一次，leak 无害
                    let lib: &'static Library = Box::leak(Box::new(lib));
                    // SAFETY：veloterm_ncat_main 是 C ABI 导出符号
                    let sym = unsafe { lib.get::<NcatMainFn>(b"veloterm_ncat_main\0") }
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

    /// ncat 入口：`args` 首元素约定为程序名（"ncat"），其后为参数，
    /// 等价于直接执行 `ncat <args[1..]>`。返回其退出码。
    pub fn ncat_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let cached = NCAT_MAIN.get_or_init(load_ncat_main);
        let Ok(ncat_main) = cached.as_ref().copied() else {
            let msg = cached.as_ref().err().map(String::as_str).unwrap_or("");
            eprintln!("ncat: ncatffi.dll 不可用（{msg}）");
            eprintln!("ncatffi.dll 随安装包分发；开发模式请先运行 node scripts/build-tools.mjs --only ncat 构建。");
            return 2;
        };

        // Windows 下 OsString 是任意 UTF-16；CLI 参数用 to_string_lossy 折损
        // 成 UTF-8 已足够（与 yq/qalc/nmap 同一取舍）。内嵌 NUL 降级为空串。
        let argv: Vec<CString> = args
            .into_iter()
            .map(|a| CString::new(a.to_string_lossy().into_owned()).unwrap_or_default())
            .collect();
        let mut ptrs: Vec<*const c_char> = argv.iter().map(|s| s.as_ptr()).collect();
        // C 约定 argv[argc] == NULL：补哨兵防指针数组越界读
        ptrs.push(std::ptr::null());
        // SAFETY：ptrs（含末尾 NULL 哨兵）与其指向的 CString 在 FFI 调用期间
        // 全程存活；ncat 侧只按 argc 读取，返回后不持有指针
        unsafe { ncat_main((ptrs.len() - 1) as c_int, ptrs.as_ptr()) }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::ffi::OsString;

    /// 非 Windows 桩：ncat 只随 Windows 安装包分发（同 yq/qalc/nmap 桩）。
    #[allow(dead_code)]
    pub fn ncat_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let _ = args.into_iter().count();
        eprintln!("ncat 仅在 Windows 构建");
        2
    }
}

// bin crate：分发臂按 crate::ncat_impl::ncat_cmd_main 全路径调用；定义在
// cfg 分家的 imp 子模块里，需顶层 re-export（cfg 限定 windows 避免
// 非 Windows 构建触发 unused_imports）。
#[cfg(windows)]
pub use imp::ncat_cmd_main;

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::imp::dll_candidates_for;
    #[cfg(windows)]
    use std::path::Path;

    /// 硬链接短名场景：exe 在 tools\ 子目录 → 候选含上级目录（安装根）
    #[test]
    #[cfg(windows)]
    fn dll_candidates_cover_hardlink_shortname_dir() {
        let exe = Path::new(r"C:\app\tools\ncat.exe");
        let c = dll_candidates_for(Some(exe));
        assert_eq!(c[0], Path::new(r"C:\app\tools\ncatffi.dll"));
        assert_eq!(c[1], Path::new(r"C:\app\ncatffi.dll"));
    }

    /// coreutils 本体直跑场景：exe 在安装根 → 首候选即同目录
    #[test]
    #[cfg(windows)]
    fn dll_candidates_cover_install_root() {
        let exe = Path::new(r"C:\app\coreutils.exe");
        let c = dll_candidates_for(Some(exe));
        assert_eq!(c[0], Path::new(r"C:\app\ncatffi.dll"));
        assert_eq!(c.len(), 2);
    }

    /// 进程路径不可得时不 panic，返回空候选
    #[test]
    #[cfg(windows)]
    fn dll_candidates_empty_without_exe() {
        assert!(dll_candidates_for(None).is_empty());
    }
}
