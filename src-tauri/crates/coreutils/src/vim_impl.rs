//! vim（Vim 9.2.1202，C 实现）的内置适配：Windows 上运行时加载 `vim64.dll`
//!（安装根 `lib/` 子目录），调其 C 导出入口 `VimMain`。
//!
//! # 为什么是 DLL，而不是像 jq-app 那样 cc 静态链入
//!
//! vim 是 ~300 个源文件、自带 nmake/autotools 生成链（pathdef.c 等）的大工程，
//! 不可能像纯 C 自包含的 jq 那样 cc 现场编译链入。但与 nmap「自建 CMake 复刻
//! 官方工程」不同，vim 上游自带官方 DLL 拆分（`VIMDLL=yes`，src/INSTALLpc.txt
//! 的 MSVC 构建方式）：`nmake -f Make_mvc.mak VIMDLL=yes CPU=AMD64 vim.exe` 产出
//! 瘦启动器 `vim.exe` + 主体 `vim64.dll`（`__declspec(dllexport) int
//! VimMain(int, char **)`，见 src/main.c），静态 CRT、零外部依赖。VeloTerm 随包
//! 只带 DLL（`lib/` 子目录，候选解析见共享模块 `crate::dll_paths`）与
//! `vim-runtime/` 数据树（平铺在安装根，源码锁在 `vendor/vim/`，见其 README——
//! 零本地修补），构建由 `scripts/build-tools.mjs` 的 vim 段在 Windows 上完成。
//!
//! # OWN_ARGV 家族：入口不带参数
//!
//! MSWIN 下 `VimMain` **无条件**用 `get_cmd_argsW`（os_win32.c）经
//! `GetCommandLineW` 重新解析进程命令行，传入的 argc/argv 被忽略——上游瘦
//! 启动器同样只调 `VimMain(0, NULL)`。故 vim 与 rg/fd/awk/bat/onefetch/parallel
//! 同属 OWN_ARGV 家族：**仅硬链接短名（`tools\vim.exe`）模式可用**（此时进程
//! 命令行恰为用户输入），`coreutils vim ...` 子命令模式由 coreutils.rs 给出
//! 提示。GUI/控制台形态亦由上游在运行期按**宿主进程 exe** 的 PE subsystem
//! 决定（`mch_is_gui_executable`）：宿主 coreutils.exe 是控制台程序，恒为
//! 控制台 vim。
//!
//! # 为什么运行时 LoadLibrary，而不是链接期导入库
//!
//! 与 goffi/qalc/nmap 同理：coreutils 的构建/测试机（macOS 开发机、CI）不必装
//! MSVC 工具链与 DLL——无链接期依赖，DLL 缺失时给出可诊断的提示并按
//! 「命令不可用」惯例返回 2。
//!
//! # 数据目录与进程生命周期的取舍
//!
//! `vim-runtime/` 候选由 `dll_paths::flat_dir_candidates` 生成（平铺在安装根，
//! 不随 DLL 进 lib/），调用前设 `VIMRUNTIME` 指过去（仅当用户未设；上游搜索序
//! 中用户显式设置优先级更高）。vim 的部分退出路径直接 `exit()`（mch_exit）、
//! 且会临时改动控制台代码页与模式（退出时恢复）——对单命令生命周期的
//! coreutils.exe 进程，`exit()` 等价于返回退出码（与 nmap 三件套同一约定）；
//! 交互式 TUI 的真实手感只能靠 Windows 真机回归（与 fzf 同一取舍）。

#[cfg(windows)]
mod imp {
    use std::ffi::{c_char, c_int};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use libloading::Library;

    /// vim64.dll 导出的 C 入口签名（上游 src/main.c，VIMDLL 下 dllexport）。
    /// MSWIN 下入口参数被忽略（VimMain 自解析进程命令行），恒传 (0, NULL)。
    type VimMainFn = unsafe extern "C" fn(c_int, *mut *mut c_char) -> c_int;

    /// 进程内首次调用加载一次；错误串缓存避免每次调用重复打日志。
    static VIM_MAIN: OnceLock<Result<VimMainFn, String>> = OnceLock::new();

    /// DLL 候选路径（共享约定，见 crate::dll_paths）：
    /// 本级 lib/ → 上级 lib/ → 同目录平铺兜底。随包名 vim.dll（与其它随包
    /// DLL 同一去架构后缀约定）；老版安装位的 vim64.dll 作升级兼容兜底
    fn dll_candidates_for(exe: Option<&Path>) -> Vec<PathBuf> {
        let mut v = crate::dll_paths::dll_candidates(exe, "vim.dll");
        v.extend(crate::dll_paths::dll_candidates(exe, "vim64.dll"));
        v
    }

    /// vim 数据目录候选（平铺在安装根，不随 DLL 进 lib/）：
    /// 本级 vim-runtime → 上级 vim-runtime。
    fn runtime_candidates_for(exe: Option<&Path>) -> Vec<PathBuf> {
        crate::dll_paths::flat_dir_candidates(exe, "vim-runtime")
    }

    fn load_vim_main() -> Result<VimMainFn, String> {
        let candidates = dll_candidates_for(std::env::current_exe().ok().as_deref());
        let mut tried = Vec::new();
        for path in candidates {
            // SAFETY：vim64.dll 只含 vim 与静态 CRT，无 DllMain 副作用；
            // LoadLibrary 的 Windows API 调用本身对进程状态无破坏性
            match unsafe { Library::new(&path) } {
                Ok(lib) => {
                    // Library 与符号同生命周期：进程内只加载一次，leak 无害
                    let lib: &'static Library = Box::leak(Box::new(lib));
                    // SAFETY：VimMain 是 C ABI 导出符号，签名见 VimMainFn
                    let sym = unsafe { lib.get::<VimMainFn>(b"VimMain\0") }
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

    /// VIMRUNTIME 兜底：未设时指向第一个实际存在的 vim-runtime/ 候选。DLL 已
    /// 加载说明安装根存在，数据目录正常必然在候选里；都不在（开发模式裸
    /// DLL）时不设，vim 按上游语义继续找 exe 同目录并给出可诊断的告警。
    fn ensure_vimruntime() {
        if std::env::var_os("VIMRUNTIME").is_some() {
            return;
        }
        for dir in runtime_candidates_for(std::env::current_exe().ok().as_deref()) {
            if dir.is_dir() {
                // SAFETY：单线程的命令分发路径上设置环境变量（coreutils 为
                // 单命令生命周期进程，影响窗口仅限本次调用）
                unsafe { std::env::set_var("VIMRUNTIME", dir.as_os_str()) };
                return;
            }
        }
    }

    /// vim 入口：不带参数（OWN_ARGV 家族——VimMain 经 GetCommandLineW 自解析
    /// 真实进程命令行，等价于直接执行 `vim <用户参数>`）。返回其退出码。
    pub fn vim_cmd_main() -> i32 {
        // 数据目录兜底须在 vim 读环境变量之前完成（$VIMRUNTIME 的消费在
        // VimMain 内部的初始化序里）
        ensure_vimruntime();
        let cached = VIM_MAIN.get_or_init(load_vim_main);
        let Ok(vim_main) = cached.as_ref().copied() else {
            let msg = cached.as_ref().err().map(String::as_str).unwrap_or("");
            eprintln!("vim: vim64.dll 不可用（{msg}）");
            eprintln!("vim64.dll 随安装包分发；开发模式请先运行 node scripts/build-tools.mjs --only vim 构建。");
            return 2;
        };
        // SAFETY：与上游瘦启动器 os_w32exe.c 的调用完全一致（VimMain(0, NULL)），
        // 参数由 DLL 内部经 GetCommandLineW 重新取得
        unsafe { vim_main(0, std::ptr::null_mut()) }
    }
}

#[cfg(not(windows))]
mod imp {
    /// 非 Windows 桩：vim 只随 Windows 安装包分发，其他平台调用即提示并
    /// 返回 2（命令不可用的 CLI 退出惯例，同 goffi/qalc/nmap 桩）。
    /// 分发臂在 coreutils_main 里按 cfg(windows) 门控，本桩在 mac 构建中
    /// 无人调用——允许 dead_code。
    #[allow(dead_code)]
    pub fn vim_cmd_main() -> i32 {
        eprintln!("vim 仅在 Windows 构建");
        2
    }
}

// bin crate：coreutils_main 按 crate::vim_impl::vim_cmd_main 全路径调用；
// vim_cmd_main 定义在 cfg 分家的 imp 子模块里，需顶层 re-export。
// cfg 限定 windows：非 Windows 构建没有调用方（分发臂亦 cfg 门控），
// 无条件 re-export 反而触发 unused_imports。
#[cfg(windows)]
pub use imp::vim_cmd_main;

#[cfg(test)]
mod tests {
    // 候选路径的 macOS 覆盖在共享模块 crate::dll_paths（dll_candidates 与
    // flat_dir_candidates 的跨平台用例）；本文件不重复测。vim 是 OWN_ARGV
    // 家族，version sweep 亦跳过——运行期行为由 test-jq 车道的
    // verify-dll-layout.ps1（-OwnArgv 探针）与安装包冒烟覆盖。
}
