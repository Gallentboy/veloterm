//! 监听器实现：tcp（本文件）+ ws/quic/udp（各自子模块，共享会话逻辑 `serve_stream`）。

pub mod quic;
pub mod udp;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;
use tracing::{info, warn};
use veloterm_relay_proto::frame::Status;
use veloterm_relay_proto::handshake::{send_status, server_handshake, ServerAuth, TargetPolicy};

use crate::config::ResolvedListener;

/// 传输无关的会话逻辑：VLR2 握手 → 连接目标 → 终局状态 → 双向搬运 → 日志。
/// tcp/ws/quic 的每连接任务都以各自的（AsyncRead+AsyncWrite）流调用本函数；
/// 并发上限由各传输层在调用前用信号量控制。
pub async fn serve_stream<S>(
    mut io: S,
    peer: &str,
    auth: &ServerAuth,
    policy: &TargetPolicy,
    hs_timeout: Duration,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let started = Instant::now();
    let (target, principal) = match server_handshake(&mut io, auth, policy, hs_timeout).await {
        Ok(v) => v,
        Err(e) => {
            warn!(peer, error = %e, "握手拒绝");
            return;
        }
    };

    let mut outbound = match TcpStream::connect((target.host.as_str(), target.port)).await {
        Ok(s) => s,
        Err(e) => {
            warn!(peer, %target, error = %e, "目标连接失败");
            let _ = send_status(&mut io, Status::ConnectFailed).await;
            return;
        }
    };
    if send_status(&mut io, Status::Ok).await.is_err() {
        return; // 客户端在终局状态前跑了
    }

    let _ = outbound.set_nodelay(true);
    match tokio::io::copy_bidirectional(&mut io, &mut outbound).await {
        Ok((up, down)) => info!(
            peer, %target, principal,
            up_bytes = up, down_bytes = down,
            duration_ms = started.elapsed().as_millis() as u64,
            "relay session closed"
        ),
        Err(e) => warn!(peer, %target, error = %e, "搬运中断"),
    }
}

/// TCP 监听器。shutdown 变更即退出并终止在途连接（中继重启本就断会话）。
pub async fn run_tcp(
    listener: ResolvedListener,
    auth: Arc<ServerAuth>,
    conn_limit: Arc<Semaphore>,
    handshake_timeout: Duration,
    shutdown: watch::Receiver<()>,
) -> anyhow::Result<()> {
    let tcp = TcpListener::bind(listener.bind)
        .await
        .map_err(|e| anyhow::anyhow!("监听 {} 绑定失败: {e}", listener.bind))?;
    run_tcp_on(tcp, listener, auth, conn_limit, handshake_timeout, shutdown).await
}

/// 在已绑定的 TcpListener 上服务（测试注入用：绑定时机由调用方掌控，避免竞态）。
pub async fn run_tcp_on(
    tcp: TcpListener,
    listener: ResolvedListener,
    auth: Arc<ServerAuth>,
    conn_limit: Arc<Semaphore>,
    handshake_timeout: Duration,
    mut shutdown: watch::Receiver<()>,
) -> anyhow::Result<()> {
    info!(bind = %tcp.local_addr()?, 默认目标 = ?listener.default_target, "tcp listener ready");

    let policy = TargetPolicy {
        default_target: listener.default_target.clone(),
        allowed: listener.allowed.clone(),
    };
    let mut tasks: JoinSet<()> = JoinSet::new();
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = tcp.accept() => match accepted {
                Ok((stream, peer)) => {
                    // try_acquire：握手洪峰也不能阻塞 accept 循环
                    let permit = match conn_limit.clone().try_acquire_owned() {
                        Ok(p) => p,
                        Err(_) => {
                            warn!(%peer, "并发上限已满，拒绝入连");
                            drop(stream);
                            continue;
                        }
                    };
                    tasks.spawn(handle_tcp_conn(
                        stream,
                        peer,
                        auth.clone(),
                        policy.clone(),
                        handshake_timeout,
                        permit,
                    ));
                }
                Err(e) => {
                    warn!(error = %e, "accept 失败");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}

async fn handle_tcp_conn(
    stream: TcpStream,
    peer: SocketAddr,
    auth: Arc<ServerAuth>,
    policy: TargetPolicy,
    handshake_timeout: Duration,
    _permit: tokio::sync::OwnedSemaphorePermit,
) {
    let _ = stream.set_nodelay(true);
    serve_stream(stream, &peer.to_string(), &auth, &policy, handshake_timeout).await;
}
