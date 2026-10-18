//! QUIC 监听器（骨架，P2 并行实现中——由隔离代理替换本文件）。

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Semaphore, watch};
use veloterm_relay_proto::handshake::ServerAuth;

use crate::config::ResolvedListener;

pub async fn run_quic(
    listener: ResolvedListener,
    auth: Arc<ServerAuth>,
    conn_limit: Arc<Semaphore>,
    handshake_timeout: Duration,
    shutdown: watch::Receiver<()>,
) -> anyhow::Result<()> {
    run_quic_on(listener, auth, conn_limit, handshake_timeout, shutdown).await
}

/// 在 listener.bind 上服务（测试注入用变体可自行添加）。
pub async fn run_quic_on(
    _listener: ResolvedListener,
    _auth: Arc<ServerAuth>,
    _conn_limit: Arc<Semaphore>,
    _handshake_timeout: Duration,
    _shutdown: watch::Receiver<()>,
) -> anyhow::Result<()> {
    anyhow::bail!("quic 监听器尚未实现（P2）")
}
