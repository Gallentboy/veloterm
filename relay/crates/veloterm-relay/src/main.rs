//! veloterm_relay —— veloterm SSH 中继服务端。
//!
//! 用法：
//! - `veloterm_relay init` 生成配置文件（含新凭证；tcp 默认启用，ws/quic 注释待启）
//! - `veloterm_relay -c relay.toml` 运行（`--check` 仅校验配置）
//! - `veloterm_relay token generate [--key-id N] [--name NAME]` 生成 32 字节 secret

use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use tokio::sync::{Semaphore, watch};
use tracing_subscriber::EnvFilter;
use veloterm_relay::config::{self, ListenerProto};
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
    /// 凭证管理（不带子命令时等同 generate）
    Token {
        #[command(subcommand)]
        action: Option<TokenAction>,
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
        None => run(cli.config, cli.check),
    }
}

/// init 子命令：生成含新凭证的配置模板。
/// 不会被覆盖：目标文件已存在时报错退出（--force 覆盖）。
fn init_config(path: std::path::PathBuf, force: bool) -> anyhow::Result<()> {
    if path.exists() && !force {
        anyhow::bail!("{} 已存在（--force 覆盖）", path.display());
    }
    let secret = Secret::generate();
    let content = init_config_template(&secret.to_hex());
    // 先生成后写入：模板经 parse+resolve 自校验，坏模板出不了门
    config::parse_toml(&content)?.resolve()?;
    std::fs::write(&path, content)
        .with_context(|| format!("写入 {} 失败", path.display()))?;
    println!("已生成 {}", path.display());
    println!();
    println!("# 后续步骤：");
    println!("  1) 校验：./veloterm_relay -c {} --check", path.display());
    println!("  2) 启动：./veloterm_relay -c {}", path.display());
    println!("  3) 客户端：veloterm 主机表单勾选「经中继连接」，填地址/端口/传输(tcp) + keyId 1 + secret");
    println!("     （目标无需任何配置——客户端自动携带所连主机地址）");
    Ok(())
}

/// init 配置模板：tcp 默认启用，ws/quic 注释待启（去掉行首 "# " 即生效）。
fn init_config_template(secret_hex: &str) -> String {
    format!(
        r####"# veloterm_relay 配置（init 生成）｜校验：veloterm_relay --check｜文档：relay/README.md
# 目标无需配置：客户端自动携带所连主机地址；allow_targets 为可选白名单

[relay]
# max_conns = 4096              # 全局并发上限
# handshake_timeout_secs = 10   # VLR2 握手超时（秒）

# 凭证：每台设备一条，泄露单独删段即吊销（veloterm_relay token generate 可再加）
[[token]]
key_id = 1
name  = "device-1"
secret = "{secret_hex}"

# ── 监听器：默认启用 tcp；ws/quic 去掉每行行首 "# " 即启用 ──────────────

[[listener]]
proto = "tcp"
bind = "0.0.0.0:7300"
# allow_targets = ["10.0.0.5:22", "linux1.internal:22"]   # 可选：限制凭证可达目标

# ── WebSocket：出口只放行 HTTP(S)、经 Nginx 等反代转 wss 时启用 ────────
# [[listener]]
# proto = "ws"
# bind = "0.0.0.0:7301"

# ── QUIC：公网推荐（内建 TLS 1.3，未配证书自动自签并把指纹打进日志）────
# [[listener]]
# proto = "quic"
# bind = "0.0.0.0:7302"
# cert = "/path/to/cert.pem"   # 可选；不配则启动自签
# key  = "/path/to/key.pem"
"####
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template_for_test() -> String {
        init_config_template(&"ab".repeat(32))
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

fn run(path: std::path::PathBuf, check_only: bool) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
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
