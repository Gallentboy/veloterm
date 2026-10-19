use std::collections::HashSet;
use std::env;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tauri::ipc::Channel;

use super::session::{TerminalBackendKind, TerminalBackendSession, TerminalCreateResult};

const DARWIN_GUI_FALLBACK_PATHS: &[&str] = &[
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
    "/usr/local/bin",
    "/usr/local/sbin",
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
];

/// Windows 本地 cmd 会话的「命令适配层」启动命令（作为 `cmd /k` 参数注入）：
/// doskey 宏两组——把交互输入的 `cd`/`chdir` 透明补上 `/d` 开关（跨盘符适配），
/// 并预置 `ll` 别名（= `ls -lh`，对齐 Linux 常见习惯；`ls` 由内置工具链硬链接
/// 短名提供，未装工具链的裸跑场景宏定义无害——执行时才报「'ls' 不是内部或外部命令」）。
///
/// 背景（cd 适配）：cmd 的 `cd` 不带 `/d` 时不切换当前盘符——`cd D:\proj` 只是替 D 盘记住
/// 「该盘的当前目录」，进程 CWD 与提示符都停在原盘，用户视角即 cd 未生效
/// （PowerShell / Git Bash 无此问题：前者 cd 内建切盘，后者是 POSIX shell，
/// 盘符统一挂成 `/c/` `/d/` 单一根命名空间，没有「每盘各记一个目录」的概念；
/// 该语义无法移植给 cmd，doskey 宏是 cmd 体系内等价的适配点）。
///
/// 约束与取舍：
/// - 会话级注入（`/k` 启动命令），不写注册表 AutoRun，不波及系统其他 cmd 会话；
/// - doskey 宏只作用于交互式命令行，批处理与 `cmd /c` 保持原生语义；
/// - 宏是纯文本替换，`cd "D:\a & b"` 这类含空格/特殊字符的路径不会被二次
///   解析破坏（相比 .bat 包装必须经 `call` 重解析，特殊字符会碎）；
/// - 已知边界：`cd /d D:\x` 这类已带开关的输入会展开成 `cd /d /d D:\x`，
///   cmd 只消费一次 `/D`（ReactOS cmd_chdir：`param += 2` 后余量直接作路径），
///   重复形态会报错——适配生效后无需再手写 `/d`。真机表现需随 Windows 构建回归
///   （macOS 无法验证 cmd 语义）。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const WINDOWS_CMD_CD_COMPAT: &str =
    "doskey cd=cd /d $* & doskey chdir=chdir /d $* & doskey ll=ls -lh $*";

// 现在 Unix 分支的 Shell 探测专用；Windows 固定 cmd.exe 后暂时无调用点，
// 保留实现并允许 Windows 下 dead_code，供后续按需复用。
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = path.metadata() {
            return meta.permissions().mode() & 0o111 != 0;
        }
        false
    }
    #[cfg(windows)]
    {
        // Windows 没有 Unix 的「可执行位」，改按 `PATHEXT` 判定扩展名；
        // 无扩展名（如 Git for Windows 的 `bash`）保守放行，避免误判为不可用。
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return true;
        };
        let ext = format!(".{}", ext.to_ascii_lowercase());
        env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .any(|entry| entry.eq_ignore_ascii_case(&ext))
    }
    #[cfg(not(any(unix, windows)))]
    {
        true
    }
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
fn is_command_available(command: &str) -> bool {
    let path = Path::new(command);
    if command.contains('/') || command.contains('\\') {
        return is_executable_file(path);
    }
    if let Some(path_var) = env::var_os("PATH") {
        for dir in env::split_paths(&path_var) {
            let candidate = dir.join(command);
            if is_executable_file(&candidate) {
                return true;
            }
        }
    }
    false
}

#[cfg(unix)]
fn get_user_login_shell() -> Option<String> {
    unsafe {
        let uid = libc::getuid();
        let pw = libc::getpwuid(uid);
        if !pw.is_null() && !(*pw).pw_shell.is_null() {
            let c_str = std::ffi::CStr::from_ptr((*pw).pw_shell);
            if let Ok(s) = c_str.to_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

/// 解析本机可用 Shell（优先继承系统终端 Profile 配置）：
/// - Unix/macOS 优先从系统用户数据库 (`getpwuid`) / `$SHELL` 获取用户默认登录 Shell，
///   不可用时按 `/bin/zsh` -> `/bin/bash` -> `/bin/sh` 回退；
/// - Windows 固定使用 `cmd.exe`（不做 pwsh/powershell 探测，也不读取自定义配置），
///   内置 Unix 工具（coreutils 硬链接短名）经 PATH 注入在 cmd 内直接可用，
///   cd/chdir 跨盘符经 doskey 宏自动补 `/d`（见 `WINDOWS_CMD_CD_COMPAT`）。
fn resolve_local_shell(preferred: Option<&str>) -> String {
    #[cfg(target_os = "windows")]
    {
        let _ = preferred;
        "cmd.exe".to_string()
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Some(custom) = preferred.map(str::trim).filter(|s| !s.is_empty())
            && is_command_available(custom)
        {
            return custom.to_string();
        }

        let passwd_shell = get_user_login_shell();
        let env_shell = env::var("SHELL").ok();
        let candidates = [
            passwd_shell.as_deref(),
            env_shell.as_deref(),
            Some("/bin/zsh"),
            Some("/bin/bash"),
            Some("/bin/sh"),
        ];
        for candidate in candidates.into_iter().flatten() {
            if is_command_available(candidate) {
                return candidate.to_string();
            }
        }
        "/bin/sh".to_string()
    }
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// 解析工作目录：支持 `~` / `~/...` 展开，若目录失效则回退到 `$HOME` 或 `/`。
fn resolve_local_cwd(requested: Option<&str>) -> PathBuf {
    if let Some(raw) = requested.map(str::trim).filter(|s| !s.is_empty()) {
        if raw == "~" {
            if let Some(home) = home_dir() {
                return home;
            }
        } else if let Some(stripped) = raw.strip_prefix("~/") {
            if let Some(home) = home_dir() {
                let expanded = home.join(stripped);
                if expanded.is_dir() {
                    return expanded;
                }
            }
        } else {
            let path = PathBuf::from(raw);
            if path.is_dir() {
                return path;
            }
        }
    }

    if let Some(home) = home_dir() {
        return home;
    }

    env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}

fn is_missing_or_c_locale(val: Option<&str>) -> bool {
    match val.map(str::trim) {
        None | Some("") => true,
        Some(v) => v.eq_ignore_ascii_case("c") || v.eq_ignore_ascii_case("posix"),
    }
}

fn is_utf8_locale(val: &str) -> bool {
    let lower = val.to_ascii_lowercase();
    lower.contains("utf-8") || lower.contains("utf8")
}

/// 配置终端子进程环境变量（完整继承系统终端 Profile 并对齐标准终端应用）：
/// - 注入 `SHELL`、`TERM_PROGRAM=VeloTerm`、`TERM_PROGRAM_VERSION`、`TERM=xterm-256color`、`COLORTERM=truecolor`；
/// - 补齐 `USER` / `LOGNAME` / `HOME`（通过 getpwuid / USERPROFILE 兜底）；
/// - macOS 从 Finder/Dock 启动时补全 Homebrew 与系统常用 `PATH`；
/// - 修正 GUI 启动缺失 UTF-8 locale 导致中文乱码的问题。
fn configure_command_env(cmd: &mut CommandBuilder, resolved_shell: &str) {
    for (k, v) in env::vars() {
        cmd.env(k, v);
    }

    // Windows：注入 coreutils 硬链接短名目录（主程序同级 tools/，
    // 由启动钩子 tools::hardlinks::ensure_installed_tools 幂等创建），
    // 用户在 cmd 里直接敲 ls/grep/sed 等短名即用（无任何 .bat 转发、不依赖 Git Bash）
    #[cfg(target_os = "windows")]
    {
        if let Ok(exe_path) = env::current_exe()
            && let Some(app_dir) = exe_path.parent()
        {
            let tools_dir = app_dir.join("tools");
            let original_path = env::var("PATH").unwrap_or_default();
            let merged = format!("{};{}", tools_dir.to_string_lossy(), original_path);
            cmd.env("PATH", &merged);
        }
    }

    #[cfg(unix)]
    {
        if env::var("USER").is_err() || env::var("LOGNAME").is_err() {
            unsafe {
                let uid = libc::getuid();
                let pw = libc::getpwuid(uid);
                if !pw.is_null()
                    && !(*pw).pw_name.is_null()
                    && let Ok(name) = std::ffi::CStr::from_ptr((*pw).pw_name).to_str()
                {
                    if env::var("USER").is_err() {
                        cmd.env("USER", name);
                    }
                    if env::var("LOGNAME").is_err() {
                        cmd.env("LOGNAME", name);
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let current_path = env::var("PATH").unwrap_or_default();
        let mut seen = HashSet::new();
        let mut merged = Vec::new();
        for entry in current_path
            .split(':')
            .chain(DARWIN_GUI_FALLBACK_PATHS.iter().copied())
        {
            let trimmed = entry.trim();
            if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
                merged.push(trimmed.to_string());
            }
        }
        cmd.env("PATH", merged.join(":"));
    }

    cmd.env("SHELL", resolved_shell);
    cmd.env("TERM_PROGRAM", "VeloTerm");
    cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
    cmd.env("TERM", "xterm-256color");
    let colorterm = env::var("COLORTERM")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "truecolor".to_string());
    cmd.env("COLORTERM", colorterm);

    let fallback_locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(|k| env::var(k).ok())
        .find(|v| is_utf8_locale(v))
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                "en_US.UTF-8".to_string()
            } else {
                "C.UTF-8".to_string()
            }
        });

    let lang = env::var("LANG").ok();
    if is_missing_or_c_locale(lang.as_deref()) {
        cmd.env("LANG", &fallback_locale);
    }
    let lc_ctype = env::var("LC_CTYPE").ok();
    if is_missing_or_c_locale(lc_ctype.as_deref()) {
        cmd.env("LC_CTYPE", &fallback_locale);
    }
}

/// 基于 `portable-pty` 的本地 PTY 会话实现。
pub struct LocalPtySession {
    id: usize,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    child: Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>>,
    closed_by_client: Arc<AtomicBool>,
}

impl LocalPtySession {
    #[allow(clippy::too_many_arguments)]
    pub fn spawn<F>(
        id: usize,
        cols: u16,
        rows: u16,
        cwd: Option<String>,
        preferred_shell: Option<String>,
        on_output: Channel<Vec<u8>>,
        on_exit: Channel<i32>,
        on_session_closed: F,
    ) -> Result<(Arc<Self>, TerminalCreateResult), String>
    where
        F: FnOnce(usize) + Send + 'static,
    {
        let safe_cols = cols.max(2);
        let safe_rows = rows.max(1);
        let resolved_shell = resolve_local_shell(preferred_shell.as_deref());
        let resolved_cwd = resolve_local_cwd(cwd.as_deref());

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: safe_rows,
                cols: safe_cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("openpty failed: {e}"))?;

        let mut cmd = CommandBuilder::new(&resolved_shell);
        #[cfg(not(target_os = "windows"))]
        {
            // 作为登录 shell 启动，确保加载用户的 .zprofile / .zshrc / .bash_profile
            cmd.arg("-l");
        }
        // Windows：经 /k 注册跨盘符 cd 适配层（doskey 宏，语义与边界见常量文档）；
        // /k 在命令执行完后继续保持交互式会话，生命周期与裸起 cmd.exe 一致
        #[cfg(target_os = "windows")]
        {
            cmd.arg("/k");
            cmd.arg(WINDOWS_CMD_CD_COMPAT);
        }
        cmd.cwd(&resolved_cwd);
        configure_command_env(&mut cmd, &resolved_shell);

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("failed to spawn shell '{resolved_shell}': {e}"))?;

        // 关键：父进程必须立即释放 slave 句柄，否则子进程退出后 master reader 不会收到 EOF
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("failed to clone pty reader: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("failed to take pty writer: {e}"))?;

        let child_arc: Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>> =
            Arc::new(Mutex::new(Some(child)));
        let closed_by_client = Arc::new(AtomicBool::new(false));

        let session = Arc::new(Self {
            id,
            master: Mutex::new(Some(pair.master)),
            writer: Mutex::new(Some(writer)),
            child: Arc::clone(&child_arc),
            closed_by_client: Arc::clone(&closed_by_client),
        });

        let reader_child = Arc::clone(&child_arc);
        let reader_closed_flag = Arc::clone(&closed_by_client);

        thread::Builder::new()
            .name(format!("veloterm-pty-reader-{id}"))
            .spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let chunk = buf.get(..n).map(|slice| slice.to_vec());
                            if chunk.is_none_or(|data| on_output.send(data).is_err()) {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }

                let exit_code = {
                    let mut guard = reader_child.lock().ok();
                    if let Some(Some(mut child_proc)) = guard.as_deref_mut().map(Option::take) {
                        match child_proc.wait() {
                            Ok(status) => status.exit_code() as i32,
                            Err(_) => 0,
                        }
                    } else {
                        0
                    }
                };

                on_session_closed(id);
                // 仅当进程自发退出（而非前端主动调用 terminal_dispose 销毁）时才向前端发送 on_exit
                if !reader_closed_flag.load(Ordering::SeqCst) {
                    let _ = on_exit.send(exit_code);
                }
            })
            .map_err(|e| format!("failed to spawn pty reader thread: {e}"))?;

        let result = TerminalCreateResult {
            id,
            kind: TerminalBackendKind::Local,
            shell: resolved_shell,
            cwd: Some(resolved_cwd.to_string_lossy().into_owned()),
        };

        Ok((session, result))
    }
}

impl TerminalBackendSession for LocalPtySession {
    fn id(&self) -> usize {
        self.id
    }

    fn kind(&self) -> TerminalBackendKind {
        TerminalBackendKind::Local
    }

    fn write(&self, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        let mut guard = self
            .writer
            .lock()
            .map_err(|_| "pty writer mutex poisoned".to_string())?;
        if let Some(writer) = guard.as_mut() {
            writer
                .write_all(data)
                .and_then(|_| writer.flush())
                .map_err(|e| format!("pty write failed: {e}"))?;
        }
        Ok(())
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let safe_cols = cols.max(2);
        let safe_rows = rows.max(1);
        let guard = self
            .master
            .lock()
            .map_err(|_| "pty master mutex poisoned".to_string())?;
        if let Some(master) = guard.as_ref() {
            master
                .resize(PtySize {
                    rows: safe_rows,
                    cols: safe_cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| format!("pty resize failed: {e}"))?;
        }
        Ok(())
    }

    fn close(&self) -> Result<(), String> {
        self.closed_by_client.store(true, Ordering::SeqCst);
        if let Ok(mut writer_guard) = self.writer.lock() {
            writer_guard.take();
        }
        if let Ok(mut master_guard) = self.master.lock() {
            master_guard.take();
        }
        if let Ok(mut child_guard) = self.child.lock()
            && let Some(mut child) = child_guard.take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::WINDOWS_CMD_CD_COMPAT;

    /// cmd 启动适配层命令的形态锁定：
    /// - 成对覆盖 `cd` 与 `chdir`（同一内建命令的两个拼写），宏体只补 `/d`；
    /// - `ll` 别名 = `ls -lh`（内置工具链 ls 短名），`$*` 透传追加参数；
    /// - 全串不得含引号/管道/重定向——`/k` 字符串里出现引号会触发 cmd 的引号
    ///   剥离特判分支，保持无引号才走最朴素的解析路径（portable-pty 仅因空格
    ///   为参数加引号，cmd 按 `/k` 旧规则剥掉首尾引号后即还原为本串）；
    /// - 多条 doskey 以 ` & ` 分隔：cmd 在解析层先按 `&` 切分命令，doskey 的
    ///   宏文本天然吃到行尾，多条定义互不污染。
    #[test]
    fn windows_cmd_cd_compat_shape() {
        let parts: Vec<&str> = WINDOWS_CMD_CD_COMPAT.split(" & ").collect();
        assert_eq!(parts.len(), 3, "应为三条 doskey 定义：{WINDOWS_CMD_CD_COMPAT}");
        for expected_name in ["cd", "chdir"] {
            let part = format!("doskey {expected_name}={expected_name} /d $*");
            assert!(
                parts.contains(&part.as_str()),
                "缺少定义「{part}」，实际：{WINDOWS_CMD_CD_COMPAT}"
            );
        }
        assert!(
            parts.contains(&"doskey ll=ls -lh $*"),
            "缺少 ll 别名定义，实际：{WINDOWS_CMD_CD_COMPAT}"
        );
        assert!(
            !WINDOWS_CMD_CD_COMPAT.contains(['"', '|', '>', '<']),
            "不得引入引号/管道/重定向字符：{WINDOWS_CMD_CD_COMPAT}"
        );
    }
}
