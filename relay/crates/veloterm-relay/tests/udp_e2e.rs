//! UDP 监听器集成测试：真实回环 socket 跑「时间同步 → 数据报鉴权 → 中继转发 →
//! echo 目标 → 回程」的服务端行为闭环。纯帧编解码由 proto crate 单测覆盖，
//! 这里只验服务端行为：同步应答可验证、时间窗/鉴权错误回执、会话转发与回收。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::net::UdpSocket;
use tokio::sync::{watch, Semaphore};
use tokio::time::timeout;
use veloterm_relay_proto::auth::{random_nonce, Secret};
use veloterm_relay_proto::frame::TargetAddr;
use veloterm_relay_proto::handshake::{AuthEntry, ServerAuth};
use veloterm_relay_proto::udp::{
    decode_error, encode_data, encode_sync_request, verify_sync_response, ClockOffset,
    UdpErrorCode,
};

use veloterm_relay::config::{ListenerProto, ResolvedListener};
use veloterm_relay::server;

const KEY_ID: u8 = 9;
const HS_TIMEOUT: Duration = Duration::from_secs(5);
/// 单步等待上限：回环正常路径毫秒级返回，5s 只兜底防挂死。
const WAIT: Duration = Duration::from_secs(5);

fn test_auth() -> Arc<ServerAuth> {
    let mut store = ServerAuth::new();
    store.add(AuthEntry {
        key_id: KEY_ID,
        name: "itest".into(),
        secret: Secret::from_bytes([9u8; 32]),
    });
    Arc::new(store)
}

fn test_secret() -> Secret {
    Secret::from_bytes([9u8; 32])
}

fn local_now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

/// 起一个 UDP echo 目标（把数据报回显给来源），返回其地址。
async fn spawn_udp_echo_target() -> TargetAddr {
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = sock.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65_535];
        loop {
            let Ok((n, peer)) = sock.recv_from(&mut buf).await else { return };
            if sock.send_to(&buf[..n], peer).await.is_err() {
                return;
            }
        }
    });
    TargetAddr { host: addr.ip().to_string(), port: addr.port() }
}

/// 起中继（先 bind 拿端口再 spawn，避免竞态）。返回（中继地址，shutdown sender）
/// —— sender 必须由调用方持有到测试结束，提前 drop 会关闭 watch 通道令服务退出。
async fn spawn_udp_relay(
    default: TargetAddr,
    conn_limit: Arc<Semaphore>,
    udp_idle_timeout: Duration,
) -> (SocketAddr, watch::Sender<()>) {
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let bind = sock.local_addr().unwrap();
    let listener = ResolvedListener {
        proto: ListenerProto::Udp,
        bind,
        default_target: Some(default),
        allowed: vec![],
        udp_idle_timeout,
        tls_cert: None,
        tls_key: None,
    };
    let (tx, rx) = watch::channel(());
    tokio::spawn(server::udp::run_udp_on(sock, listener, test_auth(), conn_limit, rx));
    (bind, tx)
}

/// 客户端时间同步：发同步请求 → 验 HMAC 应答 → 得钟差。
async fn sync_clock(client: &UdpSocket, relay: SocketAddr) -> ClockOffset {
    let nonce = random_nonce();
    client
        .send_to(&encode_sync_request(KEY_ID, &nonce), relay)
        .await
        .unwrap();
    let mut buf = vec![0u8; 256];
    let (n, _) = timeout(WAIT, client.recv_from(&mut buf))
        .await
        .expect("应收到时间同步应答")
        .unwrap();
    let server_time = verify_sync_response(&test_secret(), KEY_ID, &nonce, &buf[..n])
        .expect("同步应答 HMAC 应验证通过");
    ClockOffset::from_sample(server_time, local_now_ms())
}

/// 带超时的收包：超时/错误返回 None（用于断言「不应有回包」）。
async fn recv_from_or_timeout(sock: &UdpSocket, wait: Duration) -> Option<(Vec<u8>, SocketAddr)> {
    let mut buf = vec![0u8; 65_535];
    match timeout(wait, sock.recv_from(&mut buf)).await {
        Ok(Ok((n, from))) => Some((buf[..n].to_vec(), from)),
        Ok(Err(_)) | Err(_) => None,
    }
}

/// 1. 时间同步 + 数据往返：客户端先对时，再发数据报，经中继到 echo 目标并回显。
#[tokio::test]
async fn udp_sync_then_data_roundtrip_via_echo_target() {
    let echo = spawn_udp_echo_target().await;
    let (relay, _relay_keep) =
        spawn_udp_relay(echo, Arc::new(Semaphore::new(64)), Duration::from_secs(60)).await;

    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let offset = sync_clock(&client, relay).await;
    assert!(offset.offset_ms.unsigned_abs() < 5_000, "同机钟差应可忽略: {offset:?}");

    let dg = encode_data(&test_secret(), KEY_ID, offset.to_server_time(local_now_ms()), b"udp-e2e-query");
    client.send_to(&dg, relay).await.unwrap();
    let (payload, from) = recv_from_or_timeout(&client, WAIT).await.expect("应收到 echo 回程");
    assert_eq!(from, relay, "回程应来自中继地址");
    assert_eq!(payload, b"udp-e2e-query");

    // 第二个数据报走已有会话（复用上游 socket，不新建）
    let dg2 = encode_data(&test_secret(), KEY_ID, offset.to_server_time(local_now_ms()), b"second");
    client.send_to(&dg2, relay).await.unwrap();
    let (payload2, _) = recv_from_or_timeout(&client, WAIT).await.expect("会话复用也应回显");
    assert_eq!(payload2, b"second");
}

/// 2. 时间偏差回执：证明合法但时间戳出窗 → TimeSkew + 服务器时间（供重新对时）。
#[tokio::test]
async fn udp_stale_timestamp_gets_time_skew_receipt() {
    let echo = spawn_udp_echo_target().await;
    let (relay, _relay_keep) =
        spawn_udp_relay(echo, Arc::new(Semaphore::new(64)), Duration::from_secs(60)).await;
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    // 同机时钟下「本地时间 - 60s」必然超出 ±30s 窗口；证明本身按该时间戳合法计算
    let stale = local_now_ms().saturating_sub(60_000);
    let dg = encode_data(&test_secret(), KEY_ID, stale, b"x");
    client.send_to(&dg, relay).await.unwrap();

    let (receipt, _) = recv_from_or_timeout(&client, WAIT).await.expect("应收到错误回执");
    let (code, server_time) = decode_error(&receipt).expect("应为合法错误回执");
    assert_eq!(code, UdpErrorCode::TimeSkew);
    assert!(
        server_time.abs_diff(local_now_ms()) < 5_000,
        "回执应携带当前服务器时间供客户端重新对时"
    );
}

/// 3a. 鉴权失败回执：错误 secret 构造的数据报（证明必不匹配）→ AuthFailed。
#[tokio::test]
async fn udp_wrong_secret_data_gets_auth_failed_receipt() {
    let echo = spawn_udp_echo_target().await;
    let (relay, _relay_keep) =
        spawn_udp_relay(echo, Arc::new(Semaphore::new(64)), Duration::from_secs(60)).await;
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    let wrong = Secret::from_bytes([1u8; 32]);
    let dg = encode_data(&wrong, KEY_ID, local_now_ms(), b"x"); // 同机时间在窗内，仅证明错
    client.send_to(&dg, relay).await.unwrap();

    let (receipt, _) = recv_from_or_timeout(&client, WAIT).await.expect("应收到错误回执");
    let (code, _) = decode_error(&receipt).expect("应为合法错误回执");
    assert_eq!(code, UdpErrorCode::AuthFailed);
}

/// 3b. 未知 key_id：同步请求与数据报都回 AuthFailed。
#[tokio::test]
async fn udp_unknown_key_id_gets_auth_failed_receipt() {
    let echo = spawn_udp_echo_target().await;
    let (relay, _relay_keep) =
        spawn_udp_relay(echo, Arc::new(Semaphore::new(64)), Duration::from_secs(60)).await;
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    let nonce = random_nonce();
    client.send_to(&encode_sync_request(200, &nonce), relay).await.unwrap();
    let (receipt, _) = recv_from_or_timeout(&client, WAIT).await.expect("同步请求应收到回执");
    assert_eq!(decode_error(&receipt).expect("应为合法错误回执").0, UdpErrorCode::AuthFailed);

    let dg = encode_data(&test_secret(), 200, local_now_ms(), b"x");
    client.send_to(&dg, relay).await.unwrap();
    let (receipt, _) = recv_from_or_timeout(&client, WAIT).await.expect("数据报应收到回执");
    assert_eq!(decode_error(&receipt).expect("应为合法错误回执").0, UdpErrorCode::AuthFailed);
}

/// 配置防线：UDP 监听器未配默认目标时启动即失败（V1 固定目标语义）。
#[tokio::test]
async fn udp_run_udp_requires_default_target() {
    let listener = ResolvedListener {
        proto: ListenerProto::Udp,
        bind: "127.0.0.1:17301".parse().unwrap(),
        default_target: None,
        allowed: vec![],
        udp_idle_timeout: Duration::from_secs(60),
        tls_cert: None,
        tls_key: None,
    };
    let (_tx, rx) = watch::channel(());
    let err = server::udp::run_udp(listener, test_auth(), Arc::new(Semaphore::new(4)), HS_TIMEOUT, rx)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("target"), "错误应指向缺少 target: {err}");
}

/// 会话生命周期：并发许可被活跃会话占满时新客户端数据报被静默丢弃；
/// 会话空闲回收后许可归还，同一客户端可重新建会话。
#[tokio::test]
async fn udp_idle_sessions_are_reaped_and_permits_released() {
    let echo = spawn_udp_echo_target().await;
    let idle = Duration::from_millis(800); // 扫描周期 = 200ms
    let (relay, _relay_keep) =
        spawn_udp_relay(echo, Arc::new(Semaphore::new(1)), idle).await;

    // c1 建会话（占住唯一许可）并拿到回显
    let c1 = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let off1 = sync_clock(&c1, relay).await;
    let dg1 = encode_data(&test_secret(), KEY_ID, off1.to_server_time(local_now_ms()), b"c1");
    c1.send_to(&dg1, relay).await.unwrap();
    let (payload, _) = recv_from_or_timeout(&c1, WAIT).await.expect("c1 应收到回显");
    assert_eq!(payload, b"c1");

    // c2 的合法数据报被丢弃：无回显、无错误回执（try_acquire 失败路径）
    let c2 = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let off2 = sync_clock(&c2, relay).await; // 同步不占许可
    let dg2 = encode_data(&test_secret(), KEY_ID, off2.to_server_time(local_now_ms()), b"c2");
    c2.send_to(&dg2, relay).await.unwrap();
    assert!(
        recv_from_or_timeout(&c2, Duration::from_millis(300)).await.is_none(),
        "许可被占时应静默丢弃新会话数据报"
    );

    // 等空闲回收（800ms 超时 + 200ms 扫描周期，2s 余量充足）
    tokio::time::sleep(Duration::from_millis(2_000)).await;
    c2.send_to(&dg2, relay).await.unwrap();
    let (payload, _) = recv_from_or_timeout(&c2, WAIT).await.expect("回收后许可应已释放，c2 可建新会话");
    assert_eq!(payload, b"c2");
}
