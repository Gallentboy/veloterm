//! WebSocket 监听器（骨架，P2 并行实现中——由隔离代理替换本文件）。

use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};
use veloterm_relay_proto::handshake::ServerAuth;

use crate::config::ResolvedListener;

pub async fn run_ws(
    listener: ResolvedListener,
    auth: Arc<ServerAuth>,
    conn_limit: Arc<Semaphore>,
    handshake_timeout: Duration,
    shutdown: watch::Receiver<()>,
) -> anyhow::Result<()> {
    let tcp = TcpListener::bind(listener.bind)
        .await
        .map_err(|e| anyhow::anyhow!("监听 {} 绑定失败: {e}", listener.bind))?;
    run_ws_on(tcp, listener, auth, conn_limit, handshake_timeout, shutdown).await
}

/// 在已绑定的 TcpListener 上服务（测试注入用，与 run_tcp_on 同约定）。
pub async fn run_ws_on(
    _tcp: TcpListener,
    _listener: ResolvedListener,
    _auth: Arc<ServerAuth>,
    _conn_limit: Arc<Semaphore>,
    _handshake_timeout: Duration,
    _shutdown: watch::Receiver<()>,
) -> anyhow::Result<()> {
    anyhow::bail!("ws 监听器尚未实现（P2）")
}
