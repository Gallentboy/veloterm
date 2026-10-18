//! UDP 监听器（骨架，P2 并行实现中——由隔离代理替换本文件）。

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Semaphore, watch};
use veloterm_relay_proto::handshake::ServerAuth;

use crate::config::ResolvedListener;

pub async fn run_udp(
    listener: ResolvedListener,
    auth: Arc<ServerAuth>,
    conn_limit: Arc<Semaphore>,
    _handshake_timeout: Duration,
    shutdown: watch::Receiver<()>,
) -> anyhow::Result<()> {
    run_udp_on(listener, auth, conn_limit, shutdown).await
}

/// 在 listener.bind 上服务（测试注入用变体可自行添加）。
pub async fn run_udp_on(
    _listener: ResolvedListener,
    _auth: Arc<ServerAuth>,
    _conn_limit: Arc<Semaphore>,
    _shutdown: watch::Receiver<()>,
) -> anyhow::Result<()> {
    anyhow::bail!("udp 监听器尚未实现（P2）")
}
