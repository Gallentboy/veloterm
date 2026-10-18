//! veloterm_relay —— veloterm SSH 中继服务端。
//!
//! 用法：
//! - `veloterm_relay init` 生成配置文件（含新凭证；tcp 默认启用，ws/quic 注释待启）
//! - `veloterm_relay -c relay.toml` 运行（`--check` 仅校验配置）
//! - `veloterm_relay token generate [--key-id N] [--name NAME]` 生成 32 字节 secret
//! - `sudo veloterm_relay install` 安装为系统服务（自动识别 systemd；`--dry-run` 预演）
//! - `sudo veloterm_relay uninstall` 移除服务（默认保留配置与二进制）

use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use tokio::sync::{Semaphore, watch};
use tracing_subscriber::EnvFilter;

/// 日志时间戳：本地时区（服务器 TZ，国内部署即北京时间），
/// 格式 yyyy-MM-dd HH:mm:ss.SSS（tracing 默认计时器只出 UTC RFC3339）。
struct LocalTimer;

impl tracing_subscriber::fmt::time::FormatTime for LocalTimer {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"))
    }
}
use veloterm_relay::config::{self, ListenerProto};
use veloterm_relay::install::{self, InstallOptions};
use veloterm_relay::server;
use veloterm_relay_proto::auth::Secret;

#[derive(Parser)]
#[command(name = "veloterm_relay", version, about = "veloterm SSH 中继服务端")]
struct Cli {
    /// 配置文件路径
    #[arg(short, long, default_value = "relay.toml")]
    config: std::path::PathBuf,

    /// 仅校验配置并打印摘要，不启动
    #[arg(long)]
    check: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 生成配置文件（含新凭证；tcp 默认启用，ws/quic 注释待启）
    Init {
        /// 输出路径（默认 relay.toml，与 -c 一致）
        #[arg(short, long)]
        out: Option<std::path::PathBuf>,
        /// 覆盖已存在的文件
        #[arg(long)]
        force: bool,
    },
    /// 证书管理（QUIC 客户端 pin 指纹需长期稳定的证书文件）
    Cert {
        #[command(subcommand)]
        action: Option<CertAction>,
    },
    /// 凭证管理（不带子命令时等同 generate）
    Token {
        #[command(subcommand)]
        action: Option<TokenAction>,
    },
    /// 安装为系统服务（自动识别 systemd，装配二进制/配置/单元并 enable --now）
    Install {
        /// 配置来源（目标 /etc/veloterm/relay.toml 已存在时保留不覆盖）
        #[arg(short, long)]
        config: Option<std::path::PathBuf>,
        /// 以 root 运行服务（默认创建专用系统用户 veloterm-relay）
        #[arg(long)]
        user: Option<String>,
        /// 覆盖已存在的二进制/配置
        #[arg(long)]
        force: bool,
        /// 预演：只打印动作，不落盘不执行命令
        #[arg(long)]
        dry_run: bool,
    },
    /// 移除系统服务（默认保留配置与二进制）
    Uninstall {
        /// 一并删除 /etc/veloterm（含凭证，不可恢复）
        #[arg(long)]
        purge_config: bool,
        /// 一并删除已安装的二进制
        #[arg(long)]
        purge_bin: bool,
        /// 预演
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum CertAction {
    /// 生成 Ed25519 自签证书对（PEM 落盘），打印 SHA-256 指纹供客户端 pin
    Generate {
        /// 证书输出路径（默认 cert.pem）
        #[arg(long, default_value = "cert.pem")]
        cert: std::path::PathBuf,
        /// 私钥输出路径（默认 key.pem）
        #[arg(long, default_value = "key.pem")]
        key: std::path::PathBuf,
        /// 额外 SAN（域名/IP，可多次；localhost 恒在内）
        #[arg(long)]
        san: Vec<String>,
        /// 覆盖已存在的文件
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum TokenAction {
    /// 生成 32 字节随机 secret（打印十六进制与可粘贴的 [[token]] 段）
    Generate {
        #[arg(long, default_value_t = 1)]
        key_id: u8,
        #[arg(long, default_value = "default")]
        name: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Init { out, force }) => init_config(out.unwrap_or_else(|| cli.config.clone()), force),
        Some(Command::Cert { action }) => {
            let (cert, key, san, force) = match action {
                Some(CertAction::Generate { cert, key, san, force }) => (cert, key, san, force),
                None => (
                    "cert.pem".into(),
                    "key.pem".into(),
                    Vec::new(),
                    false,
                ),
            };
            generate_cert(cert, key, san, force)
        }
        Some(Command::Token { action }) => {
            let (key_id, name) = match action {
                Some(TokenAction::Generate { key_id, name }) => (key_id, name),
                None => (1, "default".into()),
            };
            let secret = Secret::generate();
            println!("secret = \"{}\"", secret.to_hex());
            println!();
            println!("# 粘贴到服务端 relay.toml：");
            println!("[[token]]");
            println!("key_id = {key_id}");
            println!("name  = \"{name}\"");
            println!("secret = \"{}\"", secret.to_hex());
            Ok(())
        }
        Some(Command::Install { config, user, force, dry_run }) => {
            let mut opts = InstallOptions::defaults(config.unwrap_or_else(|| cli.config.clone()))?;
            if let Some(user) = user {
                opts.service_user = user;
            }
            opts.force = force;
            opts.dry_run = dry_run;
            install::install(&opts, &Secret::generate().to_hex())
        }
        Some(Command::Uninstall { purge_config, purge_bin, dry_run }) => {
            let mut opts = InstallOptions::defaults(cli.config.clone())?;
            opts.dry_run = dry_run;
            install::uninstall(&opts, purge_config, purge_bin)
        }
        None => run(cli.config, cli.check),
    }
}

/// cert generate：生成长期稳定的证书对（PEM 落盘）+ 指纹打印。
/// 指纹形态与启动日志一致（冒号分隔大写十六进制）；客户端 pin 字段
/// 直接粘贴，客户端会归一化比对。
fn generate_cert(
    cert_path: std::path::PathBuf,
    key_path: std::path::PathBuf,
    sans: Vec<String>,
    force: bool,
) -> anyhow::Result<()> {
    for p in [&cert_path, &key_path] {
        if p.exists() && !force {
            anyhow::bail!("{} 已存在（--force 覆盖）", p.display());
        }
    }
    let generated = server::quic::self_signed_ed25519_with_sans(sans.clone())?;
    std::fs::write(&cert_path, &generated.cert_pem)
        .with_context(|| format!("写入 {} 失败", cert_path.display()))?;
    std::fs::write(&key_path, &generated.key_pem)
        .with_context(|| format!("写入 {} 失败", key_path.display()))?;
    let fingerprint = server::quic::fingerprint_hex(&server::quic::sha256(generated.cert.as_ref()));
    println!("已生成证书：{} / {}", cert_path.display(), key_path.display());
    println!("SHA-256 指纹（客户端 pin 字段直接粘贴）：");
    println!("{fingerprint}");
    println!();
    println!("# relay.toml 的 quic 监听器指向这两个文件（指纹即长期稳定）：");
    println!("# [[listener]]");
    println!("# proto = \"quic\"");
    println!("# bind  = \"0.0.0.0:7302\"");
    println!("# cert  = \"{}\"", cert_path.display());
    println!("# key   = \"{}\"", key_path.display());
    if !sans.is_empty() {
        println!("#（SAN: localhost + {}）", sans.join(", "));
    }
    Ok(())
}

/// init 子命令：生成含新凭证的配置模板。
/// 不会被覆盖：目标文件已存在时报错退出（--force 覆盖）。
fn init_config(path: std::path::PathBuf, force: bool) -> anyhow::Result<()> {
    if path.exists() && !force {
        anyhow::bail!("{} 已存在（--force 覆盖）", path.display());
    }
    let secret = Secret::generate();
    let content = config::init_template(&secret.to_hex());
    // 先生成后写入：模板经 parse+resolve 自校验，坏模板出不了门
    config::parse_toml(&content)?.resolve()?;
    std::fs::write(&path, content)
        .with_context(|| format!("写入 {} 失败", path.display()))?;
    println!("已生成 {}", path.display());
    println!();
    println!("# 后续步骤：");
    println!("  1) 校验：./veloterm_relay -c {} --check", path.display());
    println!("  2) 系统服务：sudo ./veloterm_relay install --config {}（或直接前台运行）", path.display());
    println!("  3) 客户端：veloterm 主机表单勾选「经中继连接」，填地址/端口/传输(tcp) + keyId 1 + secret");
    println!("     （目标无需任何配置——客户端自动携带所连主机地址）");
    Ok(())
}

fn run(path: std::path::PathBuf, check_only: bool) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_timer(LocalTimer)
        .init();

    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("读取配置 {} 失败", path.display()))?;
    let resolved = config::parse_toml(&text)?.resolve()?;

    if check_only {
        println!("配置有效：{} 个 token，{} 个监听器", resolved.auth.len(), resolved.listeners.len());
        for l in &resolved.listeners {
            println!(
                "  [{}] bind={} default={:?} allow={}",
                match l.proto {
                    ListenerProto::Tcp => "tcp",
                    ListenerProto::Ws => "ws",
                    ListenerProto::Udp => "udp",
                    ListenerProto::Quic => "quic",
                },
                l.bind,
                l.default_target,
                l.allowed.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", ")
            );
        }
        return Ok(());
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(serve(resolved))
}

async fn serve(resolved: config::ResolvedConfig) -> anyhow::Result<()> {
    let auth = Arc::new(resolved.auth);
    let conn_limit = Arc::new(Semaphore::new(resolved.max_conns));
    let (shutdown_tx, shutdown_rx) = watch::channel(());

    let mut tasks = tokio::task::JoinSet::new();
    for listener in resolved.listeners {
        // 证书/密钥文件存在性提前校验，避免 quic 监听器启动后才失败
        if let (Some(cert), Some(key)) = (&listener.tls_cert, &listener.tls_key) {
            anyhow::ensure!(cert.is_file(), "证书文件不存在: {}", cert.display());
            anyhow::ensure!(key.is_file(), "私钥文件不存在: {}", key.display());
        }
        match listener.proto {
            ListenerProto::Tcp => tasks.spawn(server::run_tcp(
                listener,
                auth.clone(),
                conn_limit.clone(),
                resolved.handshake_timeout,
                shutdown_rx.clone(),
            )),
            ListenerProto::Ws => tasks.spawn(server::ws::run_ws(
                listener,
                auth.clone(),
                conn_limit.clone(),
                resolved.handshake_timeout,
                shutdown_rx.clone(),
            )),
            ListenerProto::Udp => tasks.spawn(server::udp::run_udp(
                listener,
                auth.clone(),
                conn_limit.clone(),
                resolved.handshake_timeout,
                shutdown_rx.clone(),
            )),
            ListenerProto::Quic => tasks.spawn(server::quic::run_quic(
                listener,
                auth.clone(),
                conn_limit.clone(),
                resolved.handshake_timeout,
                shutdown_rx.clone(),
            )),
        };
    }

    // 任一监听器致命退出（如端口被占）都应终止进程
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("收到退出信号，关闭监听");
            let _ = shutdown_tx.send(());
        }
        result = tasks.join_next() => {
            let e = result
                .and_then(|r| r.err())
                .map(|e| anyhow::anyhow!("监听器任务异常退出: {e}"))
                .unwrap_or_else(|| anyhow::anyhow!("监听器任务意外结束"));
            let _ = shutdown_tx.send(());
            return Err(e);
        }
    }

    while tasks.join_next().await.is_some() {}
    tracing::info!("已退出");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// init 配置模板（共享实现见 config::init_template）。
    fn template_for_test() -> String {
        config::init_template(&"ab".repeat(32))
    }

    #[test]
    fn init_template_parses_resolves_and_defaults_to_tcp_only() {
        let content = template_for_test();
        let resolved = config::parse_toml(&content).unwrap().resolve().unwrap();
        // 只有 tcp 一个生效监听器（ws/quic 处于注释态）
        assert_eq!(resolved.listeners.len(), 1);
        assert_eq!(resolved.listeners[0].proto, ListenerProto::Tcp);
        assert_eq!(resolved.listeners[0].bind.port(), 7300);
        assert_eq!(resolved.auth.len(), 1);
    }

    #[test]
    fn init_template_mentions_commented_ws_and_quic() {
        let content = template_for_test();
        assert!(content.contains("# proto = \"ws\""));
        assert!(content.contains("# proto = \"quic\""));
        // 三个端口互不冲突
        for port in ["7300", "7301", "7302"] {
            assert!(content.contains(port));
        }
    }
}
