//! 系统服务安装器：`veloterm_relay install / uninstall`。
//!
//! 目标场景——把单个 musl 静态二进制丢到服务器上后一条命令完成部署：
//! ```bash
//! ./veloterm_relay install                 # 自动识别 systemd，装配服务并启动
//! ./veloterm_relay install --dry-run       # 只打印将执行的动作，不落盘
//! ./veloterm_relay uninstall --purge-config
//! ```
//!
//! 装配布局（均可覆盖）：
//! - 二进制：当前可执行文件 → `/usr/local/bin/veloterm_relay`
//! - 配置：`/etc/veloterm/relay.toml`（已存在则**保留不覆盖**——里面有凭证；
//!   缺失时从 `--config` 来源复制，来源也没有则现场生成模板）
//! - 单元：`/etc/systemd/system/veloterm-relay.service`（专用系统用户 +
//!   Restart=on-failure + 常规加固指令），`systemctl enable --now`
//!
//! 非 systemd 环境（SysV/OpenRC 等）明确报错退出，不猜格式。
//! 所有动作经 dry-run 可完整预演（不落盘、不调 systemctl/useradd）。

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

pub const DEFAULT_SERVICE_NAME: &str = "veloterm-relay";
pub const DEFAULT_SERVICE_USER: &str = "veloterm-relay";
pub const DEFAULT_BIN_DST: &str = "/usr/local/bin/veloterm_relay";
pub const DEFAULT_CONFIG_DIR: &str = "/etc/veloterm";
pub const DEFAULT_CONFIG_DST: &str = "/etc/veloterm/relay.toml";
pub const DEFAULT_UNIT_PATH: &str = "/etc/systemd/system/veloterm-relay.service";

/// install / uninstall 的完整可覆盖参数（clap 侧组装默认值）。
#[derive(Debug, Clone)]
pub struct InstallOptions {
    /// 配置来源（本地已编辑好的 relay.toml；不存在则现场生成模板）
    pub config_src: PathBuf,
    /// 二进制来源（默认当前可执行文件）
    pub bin_src: PathBuf,
    /// 二进制安装目标
    pub bin_dst: PathBuf,
    /// 配置安装目标（含凭证，权限收紧为 600）
    pub config_dst: PathBuf,
    /// 单元文件路径
    pub unit_path: PathBuf,
    /// 运行服务的系统用户（root 表示直接以 root 运行、跳过建用户）
    pub service_user: String,
    /// 已存在的目标配置是否允许覆盖（默认保留，防误灭凭证）
    pub force: bool,
    /// 预演模式：不写文件、不执行系统命令，只打印动作
    pub dry_run: bool,
}

impl InstallOptions {
    /// 生产默认值（binary 来源 = 当前进程可执行文件）。
    pub fn defaults(config_src: PathBuf) -> anyhow::Result<Self> {
        let bin_src = std::env::current_exe().context("解析当前可执行文件路径失败")?;
        Ok(Self {
            config_src,
            bin_src,
            bin_dst: DEFAULT_BIN_DST.into(),
            config_dst: DEFAULT_CONFIG_DST.into(),
            unit_path: DEFAULT_UNIT_PATH.into(),
            service_user: DEFAULT_SERVICE_USER.to_string(),
            force: false,
            dry_run: false,
        })
    }
}

/// init 系统探测结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitSystem {
    Systemd,
    /// 其余环境（携带人话原因）
    Unsupported(String),
}

/// 由路径存在性判定 init 系统（参数化便于测试）：
/// `/run/systemd/system` 目录存在是 systemd 官方判据；systemctl 可执行文件存在作次级判据。
pub fn init_system_from_paths(run_systemd_dir: bool, systemctl_bin: Option<&Path>) -> InitSystem {
    if run_systemd_dir {
        InitSystem::Systemd
    } else if let Some(bin) = systemctl_bin {
        if bin.is_file() {
            InitSystem::Systemd
        } else {
            InitSystem::Unsupported("未检测到 systemd（/run/systemd/system 不存在且无 systemctl）".into())
        }
    } else {
        InitSystem::Unsupported("未检测到 systemd（/run/systemd/system 不存在且无 systemctl）".into())
    }
}

/// 运行时探测（Linux）。
pub fn detect_init_system() -> InitSystem {
    if !cfg!(target_os = "linux") {
        return InitSystem::Unsupported("install 仅支持 Linux（当前平台不支持）".into());
    }
    let systemctl = which_systemctl();
    init_system_from_paths(Path::new("/run/systemd/system").is_dir(), systemctl.as_deref())
}

/// 在常见 PATH 里找 systemctl（不引入 which 依赖）。
fn which_systemctl() -> Option<PathBuf> {
    for dir in ["/usr/bin", "/usr/sbin", "/bin", "/sbin", "/usr/local/bin"] {
        let p = Path::new(dir).join("systemctl");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// 解析 `/etc/os-release` 键值（去引号；`#` 注释与空行跳过）。
pub fn parse_os_release(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (k, v) = line.split_once('=')?;
            let v = v.trim().trim_matches('"');
            Some((k.trim().to_string(), v.to_string()))
        })
        .collect()
}

/// 人类可读的发行版名（PRETTY_NAME 优先；读不到返回 "Linux"）。
pub fn detect_distro_pretty_name() -> String {
    for candidate in ["/etc/os-release", "/usr/lib/os-release"] {
        if let Ok(text) = std::fs::read_to_string(candidate) {
            let pairs = parse_os_release(&text);
            if let Some((_, name)) = pairs.iter().find(|(k, _)| k == "PRETTY_NAME") {
                return name.clone();
            }
            if let Some((_, id)) = pairs.iter().find(|(k, _)| k == "ID") {
                return id.clone();
            }
        }
    }
    "Linux".to_string()
}

/// 渲染 systemd 单元（纯函数）。加固取「不挡常规部署」的稳妥档：
/// ProtectSystem=full（/usr /etc 只读——本服务无需写系统目录）+ PrivateTmp 等；
/// 证书/密钥放任意路径均可读。
pub fn render_systemd_unit(opts: &InstallOptions) -> String {
    let user_line = if opts.service_user == "root" {
        String::new()
    } else {
        format!("User={}\nGroup={}\n", opts.service_user, opts.service_user)
    };
    format!(
        r#"[Unit]
Description=veloterm SSH relay (VLR2)
Documentation=https://github.com/Gallentboy/veloterm
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
{user_line}ExecStart={bin} --config {config}
Restart=on-failure
RestartSec=2s

# 加固（证书/密钥在任意路径均可读；如需写目录自行放开对应 ReadOnly/ReadWrite）
NoNewPrivileges=true
ProtectSystem=full
ProtectHome=true
PrivateTmp=true
RestrictSUIDSGID=true
LockPersonality=true

[Install]
WantedBy=multi-user.target
"#,
        bin = opts.bin_dst.display(),
        config = opts.config_dst.display(),
    )
}

/// dry-run 感知的命令执行：真跑或打印 `[dry-run] $ cmd`。
fn exec(parts: &[&str], dry_run: bool) -> anyhow::Result<()> {
    let (head, rest) = parts
        .split_first()
        .context("exec: 空命令")?;
    if dry_run {
        println!("[dry-run] $ {}", parts.join(" "));
        return Ok(());
    }
    let status = Command::new(head)
        .args(rest)
        .status()
        .with_context(|| format!("执行 {} 失败", parts.join(" ")))?;
    anyhow::ensure!(status.success(), "命令失败（exit {:?}）：{}", status.code(), parts.join(" "));
    Ok(())
}

/// dry-run 感知的写文件。
fn write_file(path: &Path, content: &str, dry_run: bool) -> anyhow::Result<()> {
    if dry_run {
        println!("[dry-run] 写入 {}（{} 字节）", path.display(), content.len());
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建目录 {} 失败", parent.display()))?;
    }
    std::fs::write(path, content).with_context(|| format!("写入 {} 失败", path.display()))?;
    Ok(())
}

/// dry-run 感知的复制文件。
fn copy_file(src: &Path, dst: &Path, dry_run: bool) -> anyhow::Result<()> {
    if dry_run {
        println!("[dry-run] 复制 {} -> {}", src.display(), dst.display());
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建目录 {} 失败", parent.display()))?;
    }
    std::fs::copy(src, dst).with_context(|| format!("复制 {} -> {} 失败", src.display(), dst.display()))?;
    Ok(())
}

/// install 主流程。返回值 Ok 即全部动作成功（或 dry-run 预演完成）。
pub fn install(opts: &InstallOptions, template_secret_hex: &str) -> anyhow::Result<()> {
    if !cfg!(target_os = "linux") && !opts.dry_run {
        bail!("install 仅支持 Linux（其他平台用 dry-run 预览动作）");
    }

    let distro = detect_distro_pretty_name();
    println!("目标系统：{distro}");

    // init 系统探测：dry-run 只提示不强拦（便于在任意机器预演）
    match detect_init_system() {
        InitSystem::Systemd => println!("init 系统：systemd"),
        InitSystem::Unsupported(reason) => {
            if opts.dry_run {
                println!("[dry-run] init 系统不满足：{reason}（真实执行将在此终止）");
            } else {
                bail!("{reason}；SysV/OpenRC 等环境请手写服务脚本（见 README）");
            }
        }
    }

    // root 检查（dry-run 只提示）
    let is_root = std::env::var("USER").as_deref() == Ok("root")
        || std::env::var("EUID").as_deref() == Ok("0");
    if !is_root {
        if opts.dry_run {
            println!("[dry-run] 当前非 root（真实执行需要 root：写 /usr/local/bin、/etc、调 systemctl）");
        } else {
            bail!("需要 root 权限（sudo ./veloterm_relay install）");
        }
    }

    // 1) 二进制
    anyhow::ensure!(opts.bin_src.is_file(), "二进制来源不存在：{}", opts.bin_src.display());
    if opts.bin_dst.exists() && !opts.force {
        println!("二进制已存在，跳过复制（--force 覆盖）：{}", opts.bin_dst.display());
    } else {
        copy_file(&opts.bin_src, &opts.bin_dst, opts.dry_run)?;
        exec(&["chmod", "0755", &opts.bin_dst.to_string_lossy()], opts.dry_run)?;
    }

    // 2) 配置：目标已存在一律保留（凭证不可误灭），除非 --force
    if opts.config_dst.exists() && !opts.force {
        println!("配置已存在，保留不覆盖：{}", opts.config_dst.display());
    } else if opts.config_src.is_file() {
        copy_file(&opts.config_src, &opts.config_dst, opts.dry_run)?;
    } else {
        println!("配置来源不存在，生成新模板（含新凭证，编辑后 restart 生效）：{}", opts.config_dst.display());
        write_file(&opts.config_dst, &crate::config::init_template(template_secret_hex), opts.dry_run)?;
    }

    // 3) 服务用户
    if opts.service_user != "root" {
        let user = opts.service_user.clone();
        let user_str = user.as_str();
        let exists = Command::new("id").arg(&user).output().map(|o| o.status.success()).unwrap_or(false);
        if exists {
            println!("服务用户已存在，复用：{user}");
        } else {
            exec(&["useradd", "--system", "--no-create-home", "--shell", "/usr/sbin/nologin", user_str], opts.dry_run)?;
        }
        // 配置目录属主与权限：凭证文件 600
        let dir = opts.config_dst.parent().unwrap_or(Path::new("/etc"));
        exec(&["chown", &format!("{user_str}:{user_str}"), &dir.to_string_lossy()], opts.dry_run)?;
        exec(&["chmod", "0750", &dir.to_string_lossy()], opts.dry_run)?;
        exec(&["chown", &format!("{user_str}:{user_str}"), &opts.config_dst.to_string_lossy()], opts.dry_run)?;
        exec(&["chmod", "0600", &opts.config_dst.to_string_lossy()], opts.dry_run)?;
    } else {
        println!("以 root 运行（--user root）：跳过建用户与属主收紧");
    }

    // 4) 单元 + 启用
    write_file(&opts.unit_path, &render_systemd_unit(opts), opts.dry_run)?;
    exec(&["systemctl", "daemon-reload"], opts.dry_run)?;
    exec(&["systemctl", "enable", "--now", DEFAULT_SERVICE_NAME], opts.dry_run)?;

    println!();
    println!("完成。常用操作：");
    println!("  状态：  systemctl status {DEFAULT_SERVICE_NAME}");
    println!("  日志：  journalctl -u {DEFAULT_SERVICE_NAME} -f");
    println!("  配置：  {}（改后 systemctl restart {DEFAULT_SERVICE_NAME}）", opts.config_dst.display());
    Ok(())
}

/// uninstall：停用并移除服务；默认保留配置（凭证）与二进制。
pub fn uninstall(opts: &InstallOptions, purge_config: bool, purge_bin: bool) -> anyhow::Result<()> {
    if !cfg!(target_os = "linux") && !opts.dry_run {
        bail!("uninstall 仅支持 Linux（其他平台用 dry-run 预览动作）");
    }
    exec(&["systemctl", "disable", "--now", DEFAULT_SERVICE_NAME], opts.dry_run)?;
    if opts.dry_run {
        println!("[dry-run] 删除 {}", opts.unit_path.display());
    } else {
        let _ = std::fs::remove_file(&opts.unit_path);
    }
    exec(&["systemctl", "daemon-reload"], opts.dry_run)?;
    exec(&["systemctl", "reset-failed", DEFAULT_SERVICE_NAME], opts.dry_run)?;

    if purge_config {
        let dir = opts.config_dst.parent().unwrap_or(Path::new("/etc/veloterm"));
        if opts.dry_run {
            println!("[dry-run] 删除目录 {}", dir.display());
        } else {
            std::fs::remove_dir_all(dir).with_context(|| format!("删除 {} 失败", dir.display()))?;
        }
    } else {
        println!("配置已保留（含凭证）：{}（--purge-config 一并删除）", opts.config_dst.display());
    }
    if purge_bin {
        if opts.dry_run {
            println!("[dry-run] 删除 {}", opts.bin_dst.display());
        } else {
            let _ = std::fs::remove_file(&opts.bin_dst);
        }
    } else {
        println!("二进制已保留：{}（--purge-bin 一并删除）", opts.bin_dst.display());
    }
    println!("服务用户未删除（如需：userdel {}）", opts.service_user);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts_for_test() -> InstallOptions {
        InstallOptions {
            config_src: PathBuf::from("/tmp/relay.toml"),
            bin_src: PathBuf::from("/tmp/veloterm_relay"),
            bin_dst: PathBuf::from("/usr/local/bin/veloterm_relay"),
            config_dst: PathBuf::from("/etc/veloterm/relay.toml"),
            unit_path: PathBuf::from("/etc/systemd/system/veloterm-relay.service"),
            service_user: "veloterm-relay".to_string(),
            force: false,
            dry_run: true,
        }
    }

    #[test]
    fn unit_contains_exec_and_hardening() {
        let unit = render_systemd_unit(&opts_for_test());
        assert!(unit.contains("ExecStart=/usr/local/bin/veloterm_relay --config /etc/veloterm/relay.toml"));
        assert!(unit.contains("User=veloterm-relay"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=multi-user.target"));
        assert!(unit.contains("NoNewPrivileges=true"));
    }

    #[test]
    fn unit_root_user_omits_user_lines() {
        let mut opts = opts_for_test();
        opts.service_user = "root".into();
        let unit = render_systemd_unit(&opts);
        assert!(!unit.contains("User="));
    }

    #[test]
    fn os_release_parses_pretty_name_with_quotes() {
        let text = "# comment\n\nID=\"centos\"\nPRETTY_NAME=\"CentOS Linux 7 (Core)\"\n";
        let pairs = parse_os_release(text);
        assert_eq!(
            pairs.iter().find(|(k, _)| k == "PRETTY_NAME").map(|(_, v)| v.as_str()),
            Some("CentOS Linux 7 (Core)")
        );
    }

    #[test]
    fn init_detection_prefers_run_dir() {
        assert_eq!(
            init_system_from_paths(true, None),
            InitSystem::Systemd
        );
        assert_eq!(
            init_system_from_paths(false, Some(Path::new("/definitely/not/here"))),
            InitSystem::Unsupported("未检测到 systemd（/run/systemd/system 不存在且无 systemctl）".into())
        );
    }

    /// dry-run 全链路在任意平台可跑通（macOS 开发机也覆盖编排顺序不崩）。
    #[test]
    fn install_dry_run_completes() {
        // bin 来源必须存在：用本测试二进制自身
        let mut opts = opts_for_test();
        opts.bin_src = std::env::current_exe().unwrap();
        opts.config_src = PathBuf::from("/nonexistent/relay.toml"); // 走生成模板分支
        install(&opts, &"ab".repeat(32)).expect("dry-run install should succeed");
    }

    #[test]
    fn uninstall_dry_run_completes() {
        uninstall(&opts_for_test(), true, true).expect("dry-run uninstall should succeed");
    }
}
