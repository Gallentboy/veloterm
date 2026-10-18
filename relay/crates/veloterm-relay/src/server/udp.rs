//! UDP 监听器：单 socket 收包 + 按客户端地址的会话表（P2 三传输层之一）。
//!
//! UDP 无连接，做不了 VLR2 的 nonce 问答握手，鉴权改用 proto `udp` 模块的
//! 「时间同步 + 时间窗证明」（协议细节见 proto crate 文档）。与 tcp 监听器的差异：
//! - **固定默认目标**：数据报里没有目标字段，V1 不支持客户端选路；监听器未配
//!   `target` 则启动即失败（校验在本模块，config 层不感知 proto 语义）。
//! - **单 socket 会话表**：一个 UdpSocket 收一切包，按 magic 分流（`VLTS` 时间
//!   同步 / `VLRD` 数据报 / 其余丢弃计数）。每个客户端地址对应一个会话：上游
//!   socket `connect` 到固定目标，pump 任务把上游回包发回客户端地址，客户端
//!   方向的数据报经 mpsc 队列交给 pump 发上游。
//! - **空闲回收**：周期扫表（idle_timeout/4，封顶 5s），超时会话移除——移除即
//!   关闭转发通道，pump 收到 `None` 自然退出、上游 socket 随之释放，会话持有的
//!   并发许可同时归还。
//! - **并发上限**：会话数受 `conn_limit` 约束（`try_acquire`，收包循环不可阻塞）；
//!   满了丢弃新客户端的数据报并记日志（UDP 语义下丢包由客户端上层重试）。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch, Semaphore};
use tokio::task::JoinSet;
use tracing::{debug, info, warn};
use veloterm_relay_proto::frame::TargetAddr;
use veloterm_relay_proto::handshake::ServerAuth;
use veloterm_relay_proto::udp as wire;

use crate::config::ResolvedListener;

/// 单个 UDP 数据报的最大字节数（UDP 载荷理论上限，收发缓冲都按此分配）。
const MAX_DATAGRAM: usize = 65_535;
/// 每会话「客户端 → 目标」方向的转发队列深度，满则丢该包。
const SESSION_BACKLOG: usize = 64;

/// 单监听器共享状态：主收包循环持有，pump 任务克隆其中的 Arc 字段。
struct UdpState {
    sock: Arc<UdpSocket>,
    auth: Arc<ServerAuth>,
    sessions: Arc<Mutex<HashMap<SocketAddr, Session>>>,
    conn_limit: Arc<Semaphore>,
    default_target: TargetAddr,
}

/// 一个客户端地址对应的转发会话。
struct Session {
    /// 客户端 → 目标方向的入队口（pump 任务持有对端）。
    tx: mpsc::Sender<Vec<u8>>,
    last_seen: Instant,
    /// 会话存续期间占用一个并发许可；Session drop 即释放。
    _permit: tokio::sync::OwnedSemaphorePermit,
}

pub async fn run_udp(
    listener: ResolvedListener,
    auth: Arc<ServerAuth>,
    conn_limit: Arc<Semaphore>,
    _handshake_timeout: Duration,
    shutdown: watch::Receiver<()>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        listener.default_target.is_some(),
        "udp 监听器 {} 未配置 target：UDP 模式 V1 只支持固定默认目标（数据报里没有目标字段）",
        listener.bind
    );
    let sock = UdpSocket::bind(listener.bind)
        .await
        .map_err(|e| anyhow::anyhow!("监听 {} 绑定失败: {e}", listener.bind))?;
    run_udp_on(sock, listener, auth, conn_limit, shutdown).await
}

/// 在已绑定的 UdpSocket 上服务（测试注入用：绑定时机由调用方掌控，避免竞态）。
pub async fn run_udp_on(
    sock: UdpSocket,
    listener: ResolvedListener,
    auth: Arc<ServerAuth>,
    conn_limit: Arc<Semaphore>,
    mut shutdown: watch::Receiver<()>,
) -> anyhow::Result<()> {
    let default_target = listener
        .default_target
        .clone()
        .ok_or_else(|| anyhow::anyhow!("udp 监听器未配置 target：UDP 模式 V1 只支持固定默认目标"))?;
    let state = UdpState {
        sock: Arc::new(sock),
        auth,
        sessions: Arc::new(Mutex::new(HashMap::new())),
        conn_limit,
        default_target,
    };

    info!(bind = %state.sock.local_addr()?, target = %state.default_target, "udp listener ready");

    let idle_timeout = listener.udp_idle_timeout;
    let mut tasks: JoinSet<()> = JoinSet::new();
    let mut recv_buf = vec![0u8; MAX_DATAGRAM];
    let mut sweep = tokio::time::interval(sweep_period(idle_timeout));
    let mut dropped_unknown: u64 = 0;

    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = sweep.tick() => state.reap_idle(idle_timeout),
            received = state.sock.recv_from(&mut recv_buf) => match received {
                Ok((n, peer)) => {
                    state.handle_client_datagram(peer, &recv_buf[..n], &mut tasks, &mut dropped_unknown).await;
                }
                Err(e) => {
                    warn!(error = %e, "udp 收包失败");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
        }
    }

    // 退出路径：清空会话表 → 全部转发通道关闭 → pump 的 rx 返回 None 退出
    // （上游 socket 随之释放），会话持有的并发许可随 Session drop 归还。
    state.sessions.lock().unwrap().clear();
    while tasks.join_next().await.is_some() {}
    info!("udp listener closed");
    Ok(())
}

/// 空闲扫描周期：idle_timeout 的 1/4，封顶 5s；保底 100ms（interval 零周期会 panic）。
fn sweep_period(idle_timeout: Duration) -> Duration {
    (idle_timeout / 4).min(Duration::from_secs(5)).max(Duration::from_millis(100))
}

/// 服务器当前时间（毫秒，UNIX 纪元起）：验窗与错误回执的时间基准。
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

/// 建立到固定目标的上游 socket：解析目标 → 按地址族绑本地任意端口 → connect
/// （connect 固定默认对端并过滤来源，回程包只来自目标本身）。
async fn connect_upstream(target: &TargetAddr) -> std::io::Result<UdpSocket> {
    let peer = tokio::net::lookup_host((target.host.as_str(), target.port))
        .await?
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "目标地址无法解析"))?;
    let local: SocketAddr = if peer.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }
        .parse()
        .expect("常量地址解析不会失败");
    let sock = UdpSocket::bind(local).await?;
    sock.connect(peer).await?;
    Ok(sock)
}

impl UdpState {
    /// 按头 4 字节 magic 分流客户端数据报。
    async fn handle_client_datagram(
        &self,
        peer: SocketAddr,
        buf: &[u8],
        tasks: &mut JoinSet<()>,
        dropped_unknown: &mut u64,
    ) {
        if buf.len() < 4 {
            *dropped_unknown += 1;
            debug!(%peer, count = *dropped_unknown, "过短数据报，丢弃");
            return;
        }
        if buf[..4] == wire::SYNC_MAGIC {
            self.handle_sync(peer, buf).await;
        } else if buf[..4] == wire::DATA_MAGIC {
            self.handle_data(peer, buf, tasks).await;
        } else {
            *dropped_unknown += 1;
            debug!(%peer, count = *dropped_unknown, "非 veloterm-relay UDP 包，丢弃");
        }
    }

    /// 时间同步请求：已知 key_id 回 HMAC 应答（客户端据此算钟差）；
    /// 未知 key_id 回 AuthFailed 错误回执。
    async fn handle_sync(&self, peer: SocketAddr, buf: &[u8]) {
        let Some((key_id, client_nonce)) = wire::decode_sync_request(buf) else {
            debug!(%peer, "格式非法的时间同步请求，丢弃");
            return;
        };
        let now = now_ms();
        match self.auth.get(key_id) {
            Some(entry) => {
                let resp = wire::encode_sync_response(&entry.secret, key_id, &client_nonce, now);
                if let Err(e) = self.sock.send_to(&resp, peer).await {
                    debug!(%peer, error = %e, "同步应答发送失败");
                }
            }
            None => {
                warn!(%peer, key_id = key_id, "时间同步请求携带未知 key_id，回执 AuthFailed");
                let _ = self
                    .sock
                    .send_to(&wire::encode_error(wire::UdpErrorCode::AuthFailed, now), peer)
                    .await;
            }
        }
    }

    /// 数据报：解析 → 查密 → 验证明 → 验窗 → 转发。
    /// 鉴权失败回 AuthFailed、出窗回 TimeSkew（附服务器时间供客户端重新对时）。
    async fn handle_data(&self, peer: SocketAddr, buf: &[u8], tasks: &mut JoinSet<()>) {
        let dg = match wire::decode_data(buf) {
            Ok(dg) => dg,
            Err(e) => {
                debug!(%peer, error = %e, "格式非法的数据报，丢弃");
                return;
            }
        };
        let now = now_ms();
        let Some(entry) = self.auth.get(dg.key_id) else {
            warn!(%peer, key_id = dg.key_id, "数据报携带未知 key_id，回执 AuthFailed");
            let _ = self
                .sock
                .send_to(&wire::encode_error(wire::UdpErrorCode::AuthFailed, now), peer)
                .await;
            return;
        };
        if !wire::verify_data_proof(&entry.secret, dg.key_id, dg.time_ms, &dg.proof) {
            warn!(%peer, principal = %entry.name, "数据报证明不匹配，回执 AuthFailed");
            let _ = self
                .sock
                .send_to(&wire::encode_error(wire::UdpErrorCode::AuthFailed, now), peer)
                .await;
            return;
        }
        if !wire::time_within_window(dg.time_ms, now) {
            warn!(
                %peer, principal = %entry.name,
                skew_ms = dg.time_ms.abs_diff(now),
                "数据报时间出窗，回执 TimeSkew"
            );
            let _ = self
                .sock
                .send_to(&wire::encode_error(wire::UdpErrorCode::TimeSkew, now), peer)
                .await;
            return;
        }
        self.forward(peer, &entry.name, dg.payload, tasks).await;
    }

    /// 经会话转发载荷：已有会话则入队（顺手续活）；否则在并发许可内新建会话。
    async fn forward(&self, peer: SocketAddr, principal: &str, payload: &[u8], tasks: &mut JoinSet<()>) {
        let mut stale_session = false;
        {
            let mut sessions = self.sessions.lock().unwrap();
            if let Some(session) = sessions.get_mut(&peer) {
                session.last_seen = Instant::now();
                match session.tx.try_send(payload.to_vec()) {
                    Ok(()) => return,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        warn!(%peer, "会话转发队列满，丢弃数据报");
                        return;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => stale_session = true,
                }
            }
        }
        if stale_session {
            // pump 已退出（上游故障），旧会话成僵尸：移除后走下方重建
            warn!(%peer, "会话上游通道已关闭，重建会话");
            self.sessions.lock().unwrap().remove(&peer);
        }

        // 新客户端：try_acquire——收包循环不能被并发上限阻塞
        let permit = match self.conn_limit.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                warn!(%peer, "并发上限已满，丢弃新 UDP 会话的数据报");
                return;
            }
        };
        let upstream = match connect_upstream(&self.default_target).await {
            Ok(s) => s,
            Err(e) => {
                warn!(%peer, target = %self.default_target, error = %e, "UDP 上游 connect 失败，丢弃数据报");
                return;
            }
        };
        let upstream = Arc::new(upstream);
        let (tx, rx) = mpsc::channel::<Vec<u8>>(SESSION_BACKLOG);
        self.sessions.lock().unwrap().insert(
            peer,
            Session { tx: tx.clone(), last_seen: Instant::now(), _permit: permit },
        );
        info!(%peer, principal = %principal, target = %self.default_target, "udp session open");
        self.spawn_pump(peer, upstream, rx, tasks);
        // 首包不能依赖 pump 已被轮询：直接经通道缓冲投递
        if let Err(e) = tx.try_send(payload.to_vec()) {
            warn!(%peer, error = %e, "新建会话首包入队失败");
        }
    }

    /// 会话 pump：上游回包 → 客户端；转发队列 → 上游。
    /// 任一方向 IO 失败或通道关闭（会话被回收/监听器退出）即结束。
    fn spawn_pump(
        &self,
        peer: SocketAddr,
        upstream: Arc<UdpSocket>,
        mut rx: mpsc::Receiver<Vec<u8>>,
        tasks: &mut JoinSet<()>,
    ) {
        let sock = self.sock.clone();
        let sessions = self.sessions.clone();
        tasks.spawn(async move {
            let mut up_buf = vec![0u8; MAX_DATAGRAM];
            loop {
                tokio::select! {
                    queued = rx.recv() => match queued {
                        Some(payload) => {
                            if let Err(e) = upstream.send(&payload).await {
                                debug!(%peer, error = %e, "上游发送失败，会话 pump 退出");
                                break;
                            }
                        }
                        None => break, // 会话被回收或监听器关闭
                    },
                    received = upstream.recv(&mut up_buf) => match received {
                        Ok(n) => {
                            if let Some(session) = sessions.lock().unwrap().get_mut(&peer) {
                                session.last_seen = Instant::now();
                            }
                            if let Err(e) = sock.send_to(&up_buf[..n], peer).await {
                                debug!(%peer, error = %e, "回程发送失败，会话 pump 退出");
                                break;
                            }
                        }
                        Err(e) => {
                            debug!(%peer, error = %e, "上游接收失败，会话 pump 退出");
                            break;
                        }
                    },
                }
            }
            debug!(%peer, "udp session pump 结束");
        });
    }

    /// 扫除超时未活跃的会话（客户端与上游两个方向任一有流量都会续活）。
    fn reap_idle(&self, idle_timeout: Duration) {
        let mut sessions = self.sessions.lock().unwrap();
        let before = sessions.len();
        sessions.retain(|_, s| s.last_seen.elapsed() < idle_timeout);
        let reaped = before - sessions.len();
        drop(sessions);
        if reaped > 0 {
            info!(reaped, active = before - reaped, "回收空闲 UDP 会话");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_period_bounds() {
        // 常规 60s 空闲 → 15s 被 5s 封顶
        assert_eq!(sweep_period(Duration::from_secs(60)), Duration::from_secs(5));
        // 短空闲 → 1/4
        assert_eq!(sweep_period(Duration::from_secs(8)), Duration::from_secs(2));
        // 零空闲也要有保底周期（interval 零周期会 panic）
        assert_eq!(sweep_period(Duration::ZERO), Duration::from_millis(100));
    }
}
