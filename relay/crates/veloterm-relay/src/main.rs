//! veloterm_relay —— veloterm SSH 中继服务端。
//!
//! 用法：
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
