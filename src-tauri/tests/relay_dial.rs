//! app 侧中继拨号集成测试：起真实 veloterm_relay（tcp 监听器）+ 回环 echo 目标，
//! 验证 `dial_relay` 的 VLR2 握手与握手后的纯字节管道——即 SSH 连接的载入路径。

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use veloterm_relay::config::{ListenerProto, ResolvedListener};
use veloterm_relay::server;
use veloterm_relay_proto::frame::TargetAddr;
use veloterm_relay_proto::handshake::{AuthEntry, ServerAuth};

use app_lib::terminal::ssh::common::{RelayTransport, SshRelayEndpoint};

async fn spawn_echo_target() -> TargetAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            tokio::spawn(async move {
                let (mut r, mut w) = stream.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    TargetAddr { host: addr.ip().to_string(), port: addr.port() }
}

async fn spawn_relay_tcp(default: TargetAddr) -> (TargetAddr, tokio::sync::watch::Sender<()>) {
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bind = tcp.local_addr().unwrap();
    let mut auth = ServerAuth::new();
    auth.add(AuthEntry {
        key_id: 9,
        name: "itest".into(),
        secret: veloterm_relay_proto::auth::Secret::from_bytes([9u8; 32]),
    });
    let listener = ResolvedListener {
        proto: ListenerProto::Tcp,
        bind,
        default_target: Some(default),
        allowed: vec![],
        udp_idle_timeout: Duration::from_secs(60),
        tls_cert: None,
        tls_key: None,
    };
    let (tx, rx) = tokio::sync::watch::channel(());
    tokio::spawn(server::run_tcp_on(
        tcp,
        listener,
        Arc::new(auth),
        Arc::new(tokio::sync::Semaphore::new(16)),
        Duration::from_secs(5),
        rx,
    ));
    (TargetAddr { host: bind.ip().to_string(), port: bind.port() }, tx)
}

fn relay_endpoint(relay: &TargetAddr) -> SshRelayEndpoint {
    SshRelayEndpoint {
        host: relay.host.clone(),
        port: relay.port,
        transport: RelayTransport::Tcp,
        key_id: 9,
        secret: veloterm_relay_proto::auth::Secret::from_bytes([9u8; 32]).to_hex(),
        cert_fingerprint: None,
    }
}

#[tokio::test]
async fn dial_relay_tcp_handshake_then_pure_pipe() {
    let echo = spawn_echo_target().await;
    let (relay, _relay_keep) = spawn_relay_tcp(echo.clone()).await;

    let dial_result =
        app_lib::terminal::ssh::relay_transport::dial_relay(
            &relay_endpoint(&relay),
            &TargetAddr { host: echo.host.clone(), port: echo.port },
        )
        .await;
    assert!(dial_result.is_ok(), "中继拨号应成功: {:?}", dial_result.as_ref().err());
    let mut stream = dial_result.unwrap();

    stream.write_all(b"ssh-payload").await.unwrap();
    let mut buf = vec![0u8; 11];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(buf, b"ssh-payload");
}

#[tokio::test]
async fn dial_relay_tcp_rejects_bad_secret() {
    let echo = spawn_echo_target().await;
    let (relay, _relay_keep) = spawn_relay_tcp(echo).await;

    let mut endpoint = relay_endpoint(&relay);
    endpoint.secret = veloterm_relay_proto::auth::Secret::from_bytes([1u8; 32]).to_hex();
    let result = app_lib::terminal::ssh::relay_transport::dial_relay(
        &endpoint,
        &TargetAddr { host: "127.0.0.1".into(), port: 22 },
    )
    .await;
    let err = match result {
        Err(e) => e,
        Ok(_) => unreachable!("错误 secret 应被中继拒绝"),
    };
    assert!(err.contains("握手失败"), "{err}");
}
