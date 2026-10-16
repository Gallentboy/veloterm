//! fzf（junegunn/fzf，Go 实现）的内置适配：Windows 上运行时加载
//! `fzfffi.dll`，调其 C 导出入口 `veloterm_fzf_main`。
//!
//! # 为什么是 DLL，而不是像 openssl-app 那样 cc 静态链入
//!
//! 与 yq 同因：Go 官方不支持把 `buildmode=c-archive` 的产物交给 MSVC
//! link.exe（golang/go#74189），而本工程 Windows 工具链是纯 MSVC ——
//! `buildmode=c-shared` 出的 DLL 是 **extern 链接在 MSVC 下对 Go 源码
//! 的唯一形态**。DLL 由 `scripts/build-tools.mjs` 在 Windows 上
//! `go build -buildmode=c-shared -tags veloterm_ffi` 现场产出（源码与
//! 版本锁在 `vendor/fzf/`，见其 VELOTERM_VERSION 与 veloterm_ffi.go），
//! 经 `tauri.windows.conf.json` 的 resources 随包放在安装根目录。
//!
//! # 交互式 TUI 在 FFI 进程内的语义
//!
//! fzf 是交互式模糊查找器：DLL 在宿主 coreutils.exe 进程内直跑，
//! stdin/stdout 是从 cmd 继承的 ConPTY 句柄，全屏渲染与按键直接落在
//! veloterm 终端上（`needWinpty` 只在 mintty 环境触发，ConPTY 下不走
//! winpty 再 exec；`--popup` 的 tmux/zellij 再 exec 经 veloterm_ffi.go
//! 临时替换 os.Args 回到 `tools\fzf.exe` 硬链接，与独立可执行文件等价）。
//! 退出码透传 fzf 语义（0 / 1=NoMatch / 2=Error / 130=Interrupt）。
//! 管道形态（`fzf --filter`、非 tty stdout 的选择输出）同理为纯
//! stdio 数据流，与进程边界无关。
//!
//! # 为什么运行时 LoadLibrary，而不是链接期导入库
//!
//! coreutils 的构建/测试机（macOS 开发机、CI）不必装 Go 工具链与 DLL：
//! 无链接期依赖，DLL 缺失时给出可诊断的提示并按「命令不可用」惯例返回 2，
//! 而不是让整个 crate 无法编译。
//!
//! # DLL 搜索路径
//!
//! 安装后 coreutils.exe 与 fzfffi.dll 同在安装根目录；但用户经 `tools\fzf.exe`
//! 硬链接短名启动时，进程 exe 目录是 `tools\` 子目录（PE 加载器的默认 DLL
//! 搜索目录跟着进程 exe 走），故按绝对路径依次尝试：exe 同目录 → exe 上级
//! 目录（tools\ 场景的安装根）。

#[cfg(windows)]
mod imp {
    use std::ffi::{c_char, c_int, CString, OsString};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use libloading::Library;

    /// fzfffi.dll 导出的 C 入口签名（Go 侧 //export veloterm_fzf_main）。
    type FzfMainFn = unsafe extern "C" fn(c_int, *const *const c_char) -> c_int;

    /// 进程内首次调用加载一次；错误串缓存避免每次调用重复打日志。
    static FZF_MAIN: OnceLock<Result<FzfMainFn, String>> = OnceLock::new();

    /// DLL 候选路径（纯函数，单测覆盖）：exe 同目录 → exe 上级目录；
    /// 进程路径不可得（exe 为 None 或无父目录）时返回空候选。
    pub(super) fn candidates_for(exe: Option<&Path>) -> Vec<PathBuf> {
        let Some(dir) = exe.and_then(|p| p.parent()) else {
            return Vec::new();
        };
        let mut v = vec![dir.join("fzfffi.dll")];
        if let Some(parent) = dir.parent() {
            v.push(parent.join("fzfffi.dll"));
        }
        v
    }

    fn load_fzf_main() -> Result<FzfMainFn, String> {
        let candidates = candidates_for(std::env::current_exe().ok().as_deref());
        let mut tried = Vec::new();
        for path in candidates {
            // SAFETY：DLL 只含 Go 运行时与 fzf，无 DllMain 副作用；LoadLibrary
            // 的 Windows API 调用本身对进程状态无破坏性
            match unsafe { Library::new(&path) } {
                Ok(lib) => {
                    // Library 与符号同生命周期：进程内只加载一次，leak 无害
                    let lib: &'static Library = Box::leak(Box::new(lib));
                    // SAFETY：veloterm_fzf_main 是 C ABI 导出符号，签名见 FzfMainFn
                    let sym = unsafe { lib.get::<FzfMainFn>(b"veloterm_fzf_main\0") }
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

    /// fzf 入口：`args` 首元素约定为程序名（"fzf"），其后为参数，等价于直接
    /// 执行 `fzf <args[1..]>`。返回其退出码。
    pub fn fzf_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let cached = FZF_MAIN.get_or_init(load_fzf_main);
        let Ok(fzf_main) = cached.as_ref().copied() else {
            let msg = cached.as_ref().err().map(String::as_str).unwrap_or("");
            eprintln!("fzf: fzfffi.dll 不可用（{msg}）");
            eprintln!("fzfffi.dll 随安装包分发；开发模式请先运行 node scripts/build-tools.mjs 构建。");
            return 2;
        };

        // Windows 下 OsString 内部是任意 UTF-16；CLI 参数场景用 to_string_lossy
        // 折损成 UTF-8 已足够（与 yq_impl 同一取舍）。内嵌 NUL 进不了
        // C 字符串：降级为空串，不 panic。
        let argv: Vec<CString> = args
            .into_iter()
            .map(|a| CString::new(a.to_string_lossy().into_owned()).unwrap_or_default())
            .collect();
        let mut ptrs: Vec<*const c_char> = argv.iter().map(|s| s.as_ptr()).collect();
        // C 约定 argv[argc] == NULL：补哨兵防指针数组越界读（同 yq_impl）
        ptrs.push(std::ptr::null());
        // SAFETY：ptrs（含末尾 NULL 哨兵）与其指向的 CString 在 FFI 调用期间
        // 全程存活；Go 侧只按 argc 读取，返回后不持有指针
        unsafe { fzf_main((ptrs.len() - 1) as c_int, ptrs.as_ptr()) }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::ffi::OsString;

    /// 非 Windows 桩：fzf 只随 Windows 安装包分发，其他平台调用即提示并
    /// 返回 2（命令不可用的 CLI 退出惯例，同 yq 桩）。
    /// 分发臂是 cfg(windows)，本桩在 mac 构建中无人调用——允许 dead_code。
    #[allow(dead_code)]
    pub fn fzf_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let _ = args.into_iter().count();
        eprintln!("fzf 仅在 Windows 构建");
        2
    }
}

// bin crate：分发臂按 crate::fzf_impl::fzf_cmd_main 全路径调用；fzf_cmd_main
// 定义在 cfg 分家的 imp 子模块里，需顶层 re-export。cfg 限定 windows：
// 非 Windows 构建没有调用方，无条件 re-export 反而触发 unused_imports。
#[cfg(windows)]
pub use imp::fzf_cmd_main;

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
        let exe = Path::new(r"C:\app\tools\fzf.exe");
        let c = candidates_for(Some(exe));
        assert_eq!(c[0], Path::new(r"C:\app\tools\fzfffi.dll"));
        assert_eq!(c[1], Path::new(r"C:\app\fzfffi.dll"));
    }

    /// coreutils 本体直跑场景：exe 在安装根 → 首候选即同目录
    #[test]
    #[cfg(windows)]
    fn dll_candidates_cover_install_root() {
        let exe = Path::new(r"C:\app\coreutils.exe");
        let c = candidates_for(Some(exe));
        assert_eq!(c[0], Path::new(r"C:\app\fzfffi.dll"));
        assert_eq!(c.len(), 2);
    }

    /// 进程路径不可得时不 panic，返回空候选
    #[test]
    #[cfg(windows)]
    fn dll_candidates_empty_without_exe() {
        assert!(candidates_for(None).is_empty());
    }
}
