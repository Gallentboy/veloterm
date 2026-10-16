//! yq + fzf（Go 实现：mikefarah/yq v4 / junegunn/fzf）的内置适配：Windows 上
//! 运行加载**单一 `veloterm_goffi.dll`**，按符号分发调 `veloterm_yq_main` /
//! `veloterm_fzf_main`（两工具共享一份 Go runtime，构建见 vendor/fzf 的
//! veloterm_goffi.go 与 vendor/go.work）。
//!
//! # 为什么是 DLL，而不是像 openssl-app 那样 cc 静态链入
//!
//! Go 官方不支持把 `buildmode=c-archive` 的产物交给 MSVC link.exe
//! （golang/go#74189，跟踪议题 #20982 多年未动），而本工程 Windows 工具链
//! 是纯 MSVC —— `buildmode=c-shared` 出的 DLL 是 **extern 链接在 MSVC 下
//! 对 Go 源码的唯一形态**。DLL 由 `scripts/build-tools.mjs` 在 Windows 上
//! 以 `go build -buildmode=c-shared -tags "veloterm_ffi veloterm_goffi"`
//! 现场产出（宿主为 vendor/fzf 的 package main，经 vendor/go.work 跨模块
//! 引 yq 的 cmd 包；GOAMD64=v3 以 AVX2 为基线），经 `tauri.windows.conf.json`
//! 的 resources 随包放在安装根目录。
//!
//! # 为什么运行时 LoadLibrary，而不是链接期导入库
//!
//! coreutils 的构建/测试机（macOS 开发机、CI）不必装 Go 工具链与 DLL：
//! 无链接期依赖，DLL 缺失时给出可诊断的提示并按「命令不可用」惯例返回 2，
//! 而不是让整个 crate 无法编译。只在用户真的运行 yq/fzf 时才加载 DLL，
//! 其余命令（jq/ls/…）零感知。库句柄进程内缓存且永不 FreeLibrary——
//! Go 的 c-shared 运行时持有后台线程，卸载 DLL 会在进程退出前崩溃。
//!
//! # DLL 搜索路径
//!
//! 安装后 coreutils.exe 与 veloterm_goffi.dll 同在安装根目录；但用户经
//! `tools\yq.exe` / `tools\fzf.exe` 硬链接短名启动时，进程 exe 目录是
//! `tools\` 子目录（PE 加载器的默认 DLL 搜索目录跟着进程 exe 走），故按
//! 绝对路径依次尝试：exe 同目录 → exe 上级目录（tools\ 场景的安装根）。

#[cfg(windows)]
mod imp {
    use std::ffi::{c_char, c_int, CString, OsString};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use libloading::Library;

    /// veloterm_goffi.dll 导出的 C 入口签名（Go 侧 //export）。
    type GoffiMainFn = unsafe extern "C" fn(c_int, *const *const c_char) -> c_int;

    /// 进程内首次调用加载一次；错误串缓存避免每次调用重复打日志。
    /// 缓存的是库句柄（永不 FreeLibrary，见文件头注释），符号按命令解析。
    static GOFFI_LIB: OnceLock<Result<&'static Library, String>> = OnceLock::new();

    /// DLL 候选路径（纯函数，单测覆盖）：exe 同目录 → exe 上级目录；
    /// 进程路径不可得（exe 为 None 或无父目录）时返回空候选。
    pub(super) fn candidates_for(exe: Option<&Path>) -> Vec<PathBuf> {
        let Some(dir) = exe.and_then(|p| p.parent()) else {
            return Vec::new();
        };
        let mut v = vec![dir.join("veloterm_goffi.dll")];
        if let Some(parent) = dir.parent() {
            v.push(parent.join("veloterm_goffi.dll"));
        }
        v
    }

    fn load_goffi_lib() -> Result<&'static Library, String> {
        let candidates = candidates_for(std::env::current_exe().ok().as_deref());
        let mut tried = Vec::new();
        for path in candidates {
            // SAFETY：DLL 只含 Go 运行时与 yq/fzf，无 DllMain 副作用；
            // LoadLibrary 的 Windows API 调用本身对进程状态无破坏性
            match unsafe { Library::new(&path) } {
                Ok(lib) => {
                    // Library 进程内只加载一次且永不卸载（Go 运行时持有后台
                    // 线程，FreeLibrary 会在进程退出前崩溃），leak 即正确语义
                    return Ok(Box::leak(Box::new(lib)));
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

    /// 通用调用器：解析指定导出符号并按 C main 约定调用。
    /// `progname` 仅用于错误提示（用户视角的命令名）。
    fn invoke(symbol: &[u8], progname: &str, args: &[OsString]) -> i32 {
        let cached = GOFFI_LIB.get_or_init(load_goffi_lib);
        let Ok(lib) = cached.as_ref() else {
            let msg = cached.as_ref().err().map(String::as_str).unwrap_or("");
            eprintln!("{progname}: veloterm_goffi.dll 不可用（{msg}）");
            eprintln!("veloterm_goffi.dll 随安装包分发；开发模式请先运行 node scripts/build-tools.mjs 构建。");
            return 2;
        };
        // SAFETY：veloterm_yq_main / veloterm_fzf_main 是 C ABI 导出符号，
        // 签名见 GoffiMainFn
        let Ok(func) = (unsafe { lib.get::<GoffiMainFn>(symbol) }).map(|f| *f) else {
            let name = String::from_utf8_lossy(&symbol[..symbol.len() - 1]);
            eprintln!("{progname}: veloterm_goffi.dll 已加载但缺少导出符号 {name}");
            return 2;
        };

        // Windows 下 OsString 内部是任意 UTF-16；CLI 参数场景用 to_string_lossy
        // 折损成 UTF-8 已足够（与 openssl-app 同一取舍）。内嵌 NUL 进不了
        // C 字符串：降级为空串，不 panic。
        let argv: Vec<CString> = args
            .iter()
            .map(|a| CString::new(a.to_string_lossy().into_owned()).unwrap_or_default())
            .collect();
        let mut ptrs: Vec<*const c_char> = argv.iter().map(|s| s.as_ptr()).collect();
        // C 约定 argv[argc] == NULL：补哨兵防指针数组越界读
        // （与 openssl-app / putty-telnet 的 FFI 桥同一修复）
        ptrs.push(std::ptr::null());
        // SAFETY：ptrs（含末尾 NULL 哨兵）与其指向的 CString 在 FFI 调用期间
        // 全程存活；Go 侧只按 argc 读取，返回后不持有指针
        unsafe { func((ptrs.len() - 1) as c_int, ptrs.as_ptr()) }
    }

    /// yq 入口：`args` 首元素约定为程序名（"yq"），其后为子命令与参数，
    /// 等价于直接执行 `yq <args[1..]>`。返回其退出码。
    pub fn yq_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let collected: Vec<OsString> = args.into_iter().collect();
        invoke(b"veloterm_yq_main\0", "yq", &collected)
    }

    /// fzf 入口：`args` 首元素约定为程序名（"fzf"）。返回其退出码
    /// （0 / 1=NoMatch / 2=Error / 130=Interrupt，Go 侧映射后返回）。
    pub fn fzf_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let collected: Vec<OsString> = args.into_iter().collect();
        invoke(b"veloterm_fzf_main\0", "fzf", &collected)
    }
}

#[cfg(not(windows))]
mod imp {
    use std::ffi::OsString;

    /// 非 Windows 桩：yq/fzf 只随 Windows 安装包分发，其他平台调用即提示并
    /// 返回 2（命令不可用的 CLI 退出惯例，同 openssl/telnet 桩）。
    /// 分发臂是 cfg(windows)，本桩在 mac 构建中无人调用——允许 dead_code。
    #[allow(dead_code)]
    pub fn yq_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let _ = args.into_iter().count();
        eprintln!("yq 仅在 Windows 构建");
        2
    }

    #[allow(dead_code)]
    pub fn fzf_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let _ = args.into_iter().count();
        eprintln!("fzf 仅在 Windows 构建");
        2
    }
}

// bin crate：分发臂按 crate::goffi_impl::* 全路径调用；入口定义在 cfg 分家的
// imp 子模块里，需顶层 re-export。cfg 限定 windows：非 Windows 构建没有调用方，
// 无条件 re-export 反而触发 unused_imports。
#[cfg(windows)]
pub use imp::{fzf_cmd_main, yq_cmd_main};

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
        assert_eq!(c[0], Path::new(r"C:\app\tools\veloterm_goffi.dll"));
        assert_eq!(c[1], Path::new(r"C:\app\veloterm_goffi.dll"));
    }

    /// coreutils 本体直跑场景：exe 在安装根 → 首候选即同目录
    #[test]
    #[cfg(windows)]
    fn dll_candidates_cover_install_root() {
        let exe = Path::new(r"C:\app\coreutils.exe");
        let c = candidates_for(Some(exe));
        assert_eq!(c[0], Path::new(r"C:\app\veloterm_goffi.dll"));
        assert_eq!(c.len(), 2);
    }

    /// 进程路径不可得时不 panic，返回空候选
    #[test]
    #[cfg(windows)]
    fn dll_candidates_empty_without_exe() {
        assert!(candidates_for(None).is_empty());
    }
}
