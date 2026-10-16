//! nmap（Nmap 7.992，C/C++ 实现）的内置适配：Windows 上运行时加载
//! `nmapffi.dll`，调其 C 导出入口 `veloterm_nmap_main`。
//!
//! # 为什么是 DLL，而不是像 jq-app 那样 cc 静态链入
//!
//! nmap 是 C/C++ + autotools 体系（libpcap/libdnet/liblua/libpcre2 等七个子
//! 构建），上游官方 Windows 构建是 MSVC 工程（无 MinGW 支持承诺），不像纯 C
//! 自包含的 jq 那样能现场编译链入。故与 yq/fzf/qalc 同一形态：源码锁在
//! `vendor/nmap/`（含树内 winfix.cc 的 MinGW 补丁与 veloterm_ffi.cc 壳），
//! 由 `scripts/build-tools.mjs` 在 Windows 上经 MSYS2 MinGW64 构建，依赖全
//! 静态打入 `nmapffi.dll`，数据文件树（scripts/nselib/nmap-services 等）随
//! 包放安装根目录的 `nmap-data/`（见 vendor/nmap/README.md）。
//!
//! # 为什么运行时 LoadLibrary，而不是链接期导入库
//!
//! 与 yqffi/qalcffi 同理：coreutils 的构建/测试机（macOS 开发机、CI）不必装
//! MSYS2 工具链与 DLL——无链接期依赖，DLL 缺失时给出可诊断的提示并按
//! 「命令不可用」惯例返回 2。
//!
//! # DLL 搜索路径与数据文件目录
//!
//! 与 yqffi/qalcffi 相同：安装后 coreutils.exe 与 nmapffi.dll 同在安装根
//! 目录；用户经 `tools\nmap.exe` 硬链接短名启动时按绝对路径依次尝试 exe 同
//! 目录 → exe 上级目录（tools\ 场景的安装根）。
//!
//! 数据文件：nmap 在 Windows 下默认找 **exe 同目录**（nmap_fetchfile_sub
//! 的搜索序），DLL 形态下 `GetModuleFileName(GetModuleHandle(0))` 取到的是
//! 宿主 coreutils.exe 路径（tools\ 子目录），找不到随包数据——故调用前把
//! `NMAPDIR` 指到安装根的 `nmap-data/`（仅当用户未设时；用户显式
//! `--datadir`/`NMAPDIR` 在上游搜索序中优先级更高，不受影响）。coreutils
//! 是单命令生命周期进程，环境变量的影响窗口仅限本次调用。
//!
//! # Npcap 运行期依赖
//!
//! Packet.dll 以 `-Wl,--delayload` 延迟导入：未装 Npcap 的机器
//! LoadLibrary(nmapffi.dll) 仍成功，nmap 经打补丁的 win_init 降级为
//! connect() 扫描（`-sT`、服务/版本探测可用），SYN/UDP/OS 指纹等原始包
//! 能力提示安装 Npcap（https://npcap.com）。

#[cfg(windows)]
mod imp {
    use std::ffi::{c_char, c_int, CString, OsString};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use libloading::Library;

    /// nmapffi.dll 导出的 C 入口签名（vendor/nmap/veloterm_ffi.cc）。
    type NmapMainFn = unsafe extern "C" fn(c_int, *const *const c_char) -> c_int;

    /// 进程内首次调用加载一次；错误串缓存避免每次调用重复打日志。
    static NMAP_MAIN: OnceLock<Result<NmapMainFn, String>> = OnceLock::new();

    /// DLL 候选路径（纯函数，单测覆盖）：exe 同目录 → exe 上级目录；
    /// 进程路径不可得（exe 为 None 或无父目录）时返回空候选。
    pub(super) fn dll_candidates_for(exe: Option<&Path>) -> Vec<PathBuf> {
        let Some(dir) = exe.and_then(|p| p.parent()) else {
            return Vec::new();
        };
        let mut v = vec![dir.join("nmapffi.dll")];
        if let Some(parent) = dir.parent() {
            v.push(parent.join("nmapffi.dll"));
        }
        v
    }

    /// nmap 数据目录候选（与 DLL 同一套定位约定：nmap-data/ 平铺在安装根）。
    pub(super) fn data_dir_candidates_for(exe: Option<&Path>) -> Vec<PathBuf> {
        let Some(dir) = exe.and_then(|p| p.parent()) else {
            return Vec::new();
        };
        let mut v = vec![dir.join("nmap-data")];
        if let Some(parent) = dir.parent() {
            v.push(parent.join("nmap-data"));
        }
        v
    }

    fn load_nmap_main() -> Result<NmapMainFn, String> {
        let candidates = dll_candidates_for(std::env::current_exe().ok().as_deref());
        let mut tried = Vec::new();
        for path in candidates {
            // SAFETY：DLL 只含 nmap 与静态依赖，无 DllMain 副作用；
            // LoadLibrary 的 Windows API 调用本身对进程状态无破坏性
            match unsafe { Library::new(&path) } {
                Ok(lib) => {
                    // Library 与符号同生命周期：进程内只加载一次，leak 无害
                    let lib: &'static Library = Box::leak(Box::new(lib));
                    // SAFETY：veloterm_nmap_main 是 C ABI 导出符号，签名见 NmapMainFn
                    let sym = unsafe { lib.get::<NmapMainFn>(b"veloterm_nmap_main\0") }
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

    /// NMAPDIR 兜底：未设时指向第一个实际存在的 nmap-data/ 候选。DLL 已加载
    /// 说明安装根存在，数据目录正常必然在候选里；都不在（开发模式裸 DLL）时
    /// 不设，nmap 按上游语义继续找 exe 同目录并给出可诊断的缺文件告警。
    fn ensure_nmapdir() {
        if std::env::var_os("NMAPDIR").is_some() {
            return;
        }
        for dir in data_dir_candidates_for(std::env::current_exe().ok().as_deref()) {
            if dir.is_dir() {
                // SAFETY：单线程的命令分发路径上设置环境变量（coreutils 为
                // 单命令生命周期进程，影响窗口仅限本次调用）
                unsafe { std::env::set_var("NMAPDIR", dir.as_os_str()) };
                return;
            }
        }
    }

    /// nmap 入口：`args` 首元素约定为程序名（"nmap"），其后为目标与选项，
    /// 等价于直接执行 `nmap <args[1..]>`。返回其退出码。
    pub fn nmap_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        // 数据目录兜底须在入口逻辑读环境变量之前完成（NMAP_ARGS/--datadir
        // 的解析都在 DLL 内部）
        ensure_nmapdir();
        let cached = NMAP_MAIN.get_or_init(load_nmap_main);
        let Ok(nmap_main) = cached.as_ref().copied() else {
            let msg = cached.as_ref().err().map(String::as_str).unwrap_or("");
            eprintln!("nmap: nmapffi.dll 不可用（{msg}）");
            eprintln!("nmapffi.dll 随安装包分发；开发模式请先运行 node scripts/build-tools.mjs --only nmap 构建。");
            return 2;
        };

        // Windows 下 OsString 内部是任意 UTF-16；CLI 参数场景用 to_string_lossy
        // 折损成 UTF-8 已足够（与 yq/qalc/openssl-app 同一取舍）。内嵌 NUL 进
        // 不了 C 字符串：降级为空串，不 panic。
        let argv: Vec<CString> = args
            .into_iter()
            .map(|a| CString::new(a.to_string_lossy().into_owned()).unwrap_or_default())
            .collect();
        let mut ptrs: Vec<*const c_char> = argv.iter().map(|s| s.as_ptr()).collect();
        // C 约定 argv[argc] == NULL：补哨兵防指针数组越界读
        // （与 yq/qalc/openssl-app/putty-telnet 的 FFI 桥同一约定）
        ptrs.push(std::ptr::null());
        // SAFETY：ptrs（含末尾 NULL 哨兵）与其指向的 CString 在 FFI 调用期间
        // 全程存活；nmap 侧只按 argc 读取，返回后不持有指针
        unsafe { nmap_main((ptrs.len() - 1) as c_int, ptrs.as_ptr()) }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::ffi::OsString;

    /// 非 Windows 桩：nmap 只随 Windows 安装包分发，其他平台调用即提示并
    /// 返回 2（命令不可用的 CLI 退出惯例，同 yq/qalc 桩）。
    /// 分发臂是 cfg(windows)，本桩在 mac 构建中无人调用——允许 dead_code。
    #[allow(dead_code)]
    pub fn nmap_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let _ = args.into_iter().count();
        eprintln!("nmap 仅在 Windows 构建");
        2
    }
}

// bin crate：分发臂按 crate::nmap_impl::nmap_cmd_main 全路径调用；
// nmap_cmd_main 定义在 cfg 分家的 imp 子模块里，需顶层 re-export。
// cfg 限定 windows：非 Windows 构建没有调用方，无条件 re-export 反而
// 触发 unused_imports。
#[cfg(windows)]
pub use imp::nmap_cmd_main;

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::imp::{data_dir_candidates_for, dll_candidates_for};
    #[cfg(windows)]
    use std::path::Path;

    /// 硬链接短名场景：exe 在 tools\ 子目录 → 候选含上级目录（安装根）
    #[test]
    #[cfg(windows)]
    fn dll_candidates_cover_hardlink_shortname_dir() {
        let exe = Path::new(r"C:\app\tools\nmap.exe");
        let c = dll_candidates_for(Some(exe));
        assert_eq!(c[0], Path::new(r"C:\app\tools\nmapffi.dll"));
        assert_eq!(c[1], Path::new(r"C:\app\nmapffi.dll"));
    }

    /// coreutils 本体直跑场景：exe 在安装根 → 首候选即同目录
    #[test]
    #[cfg(windows)]
    fn dll_candidates_cover_install_root() {
        let exe = Path::new(r"C:\app\coreutils.exe");
        let c = dll_candidates_for(Some(exe));
        assert_eq!(c[0], Path::new(r"C:\app\nmapffi.dll"));
        assert_eq!(c.len(), 2);
    }

    /// 数据目录候选与 DLL 候选同一套定位（nmap-data/ 平铺在安装根）
    #[test]
    #[cfg(windows)]
    fn data_dir_candidates_mirror_dll_candidates() {
        let exe = Path::new(r"C:\app\tools\nmap.exe");
        let c = data_dir_candidates_for(Some(exe));
        assert_eq!(c[0], Path::new(r"C:\app\tools\nmap-data"));
        assert_eq!(c[1], Path::new(r"C:\app\nmap-data"));
    }

    /// 进程路径不可得时不 panic，返回空候选
    #[test]
    #[cfg(windows)]
    fn candidates_empty_without_exe() {
        assert!(dll_candidates_for(None).is_empty());
        assert!(data_dir_candidates_for(None).is_empty());
    }
}
