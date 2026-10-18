//! TCP 监听器：accept → VLR2 握手 → 连接目标 → 纯字节搬运。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;
use tracing::{info, warn};
use veloterm_relay_proto::frame::Status;
use veloterm_relay_proto::handshake::{send_status, server_handshake, ServerAuth, TargetPolicy};

use crate::config::ResolvedListener;

/// 监听器任务。shutdown 变更即退出并终止在途连接（中继重启本就断会话）。
pub async fn run_tcp(
    listener: ResolvedListener,
    auth: Arc<ServerAuth>,
    conn_limit: Arc<Semaphore>,
    handshake_timeout: std::time::Duration,
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
    handshake_timeout: std::time::Duration,
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
                    tasks.spawn(handle_conn(
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
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            },
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}

async fn handle_conn(
    mut inbound: TcpStream,
    peer: SocketAddr,
    auth: Arc<ServerAuth>,
    policy: TargetPolicy,
    handshake_timeout: std::time::Duration,
    _permit: tokio::sync::OwnedSemaphorePermit,
) {
    let started = Instant::now();
    let (target, principal) = match server_handshake(&mut inbound, &auth, &policy, handshake_timeout).await {
        Ok(v) => v,
        Err(e) => {
            warn!(%peer, error = %e, "握手拒绝");
            return;
        }
    };

    let mut outbound = match TcpStream::connect((target.host.as_str(), target.port)).await {
        Ok(s) => s,
        Err(e) => {
            warn!(%peer, %target, error = %e, "目标连接失败");
            let _ = send_status(&mut inbound, Status::ConnectFailed).await;
            return;
        }
    };
    if send_status(&mut inbound, Status::Ok).await.is_err() {
        return; // 客户端在终局状态前跑了
    }

    match relay_bytes(&mut inbound, &mut outbound).await {
        Ok((up, down)) => info!(
            %peer, %target, principal,
            up_bytes = up, down_bytes = down,
            duration_ms = started.elapsed().as_millis() as u64,
            "relay session closed"
        ),
        Err(e) => warn!(%peer, %target, error = %e, "搬运中断"),
    }
}

async fn relay_bytes(
    inbound: &mut TcpStream,
    outbound: &mut TcpStream,
) -> std::io::Result<(u64, u64)> {
    // half-close 透传：客户端 FIN 之后仍把目标侧剩余响应搬完（SSH 会正常收尾）
    let _ = inbound.set_nodelay(true);
    let _ = outbound.set_nodelay(true);
    tokio::io::copy_bidirectional(inbound, outbound).await
}
