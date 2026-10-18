//! WebSocket 传输层端到端测试（对照 tests/e2e.rs 的 TCP 版）：
//! 客户端 connect_async + 与服务端同一个 [`WsByteStream`] 适配器 → VLR2 握手
//! → 中继 → 目标；最后一项用 russh 证明完整 SSH 会话能跑在 ws 字节流上。

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{connect_async, MaybeTlsStream};
use veloterm_relay_proto::auth::Secret;
use veloterm_relay_proto::frame::TargetAddr;
use veloterm_relay_proto::handshake::{client_handshake, AuthEntry, ServerAuth};

use veloterm_relay::config::{ListenerProto, ResolvedListener};
use veloterm_relay::server::ws::{run_ws_on, WsByteStream};

const KEY_ID: u8 = 9;
const HS_TIMEOUT: Duration = Duration::from_secs(5);

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

/// 起一个 TCP echo 目标，返回其地址。
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

/// 起 ws 中继（先同步 bind 拿到端口，再 spawn 服务任务，避免连接竞态）。
/// 返回（中继地址，shutdown sender）——sender 必须由调用方持有，
/// 提前 drop 会让 watch 通道关闭、服务任务立即退出。
async fn spawn_ws_relay(
    default: Option<TargetAddr>,
    allowed: Vec<TargetAddr>,
) -> (TargetAddr, tokio::sync::watch::Sender<()>) {
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bind = tcp.local_addr().unwrap();
    let listener = ResolvedListener {
        proto: ListenerProto::Ws,
        bind,
        default_target: default,
        allowed,
        udp_idle_timeout: Duration::from_secs(60),
        tls_cert: None,
        tls_key: None,
    };
    let (tx, rx) = tokio::sync::watch::channel(());
    tokio::spawn(run_ws_on(
        tcp,
        listener,
        test_auth(),
        Arc::new(tokio::sync::Semaphore::new(64)),
        HS_TIMEOUT,
        rx,
    ));
    (TargetAddr { host: bind.ip().to_string(), port: bind.port() }, tx)
}

/// ws 客户端底层形状（升级后的字节流适配器）。
type RawWs = WsByteStream<MaybeTlsStream<TcpStream>>;
/// ws 客户端握手的加密流（VLR3：后续字节全部走加密记录层）。
type ClientWs = veloterm_relay_proto::crypto::EncryptedStream<RawWs>;

/// 建立 ws 连接（不做 VLR2 握手）。
async fn connect_ws(relay: &TargetAddr) -> RawWs {
    let (ws, _resp) = connect_async(format!("ws://{}:{}/", relay.host, relay.port))
        .await
        .expect("ws 连接应成功");
    WsByteStream::new(ws)
}

/// 经 ws 中继建立一条已握手的字节流。
async fn connect_via_relay(
    relay: &TargetAddr,
    secret: &Secret,
    target: Option<&TargetAddr>,
) -> ClientWs {
    let io = connect_ws(relay).await;
    client_handshake(io, KEY_ID, secret, target, HS_TIMEOUT)
        .await
        .expect("握手应成功")
}

#[tokio::test]
async fn ws_relay_pipes_bytes_both_ways() {
    let echo = spawn_echo_target().await;
    let (relay, _relay_keep) = spawn_ws_relay(Some(echo), vec![]).await;

    let mut io = connect_via_relay(&relay, &test_secret(), None).await;
    // 适配器按 AsyncWrite 契约缓冲写入，poll_flush 才把缓冲作为一条 binary 消息发出
    // （TcpStream 无此问题是因为内核套接字写即发送；握手与 russh 内部都会显式 flush）。
    // 所以 write_all 之后必须显式 flush，否则数据滞留本端缓冲、对端永远等不到。
    io.write_all(b"hello over ws").await.unwrap();
    io.flush().await.unwrap();
    let mut buf = vec![0u8; 13];
    tokio::time::timeout(HS_TIMEOUT, io.read_exact(&mut buf))
        .await
        .expect("回显应在一个超时周期内到达")
        .unwrap();
    assert_eq!(buf, b"hello over ws");
}

#[tokio::test]
async fn wrong_secret_is_rejected_with_status_over_ws() {
    let echo = spawn_echo_target().await;
    let (relay, _relay_keep) = spawn_ws_relay(Some(echo), vec![]).await;

    let mut io = connect_ws(&relay).await;
    let wrong = Secret::from_bytes([1u8; 32]);
    let err = match client_handshake(&mut io, KEY_ID, &wrong, None, HS_TIMEOUT).await {
        Err(e) => e,
        Ok(_) => panic!("错误 secret 应鉴权失败"),
    };
    assert!(matches!(err, veloterm_relay_proto::handshake::ClientHandshakeError::AuthMismatch));
}

mod russh_ws_e2e {
    use super::*;

    use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
    use russh::keys::PrivateKey;
    use russh::server::{self, Auth, ChannelOpenHandle, Session};
    use russh::{ChannelId, ChannelMsg, client};

    /// 起一个最小 russh 服务端（接受任意密码认证，exec 回显命令），返回地址。
    async fn spawn_sshd() -> TargetAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // 确定性测试主机密钥（固定种子，避免测试依赖随机与额外 feature）
        let keypair = Ed25519Keypair::from_seed(&[0xED; 32]);
        let host_key = PrivateKey::new(KeypairData::from(keypair), "veloterm-relay-test")
            .unwrap();
        let config = Arc::new(server::Config { keys: vec![host_key], ..Default::default() });
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { return };
                let config = config.clone();
                tokio::spawn(async move {
                    let _ = server::run_stream(config, stream, Sshd).await;
                });
            }
        });
        TargetAddr { host: addr.ip().to_string(), port: addr.port() }
    }

    #[derive(Clone)]
    struct Sshd;

    impl server::Server for Sshd {
        type Handler = Sshd;
        fn new_client(&mut self, _addr: Option<std::net::SocketAddr>) -> Self {
            self.clone()
        }
    }

    impl server::Handler for Sshd {
        type Error = russh::Error;

        async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
            Ok(Auth::Accept)
        }

        async fn channel_open_session(
            &mut self,
            _channel: russh::Channel<server::Msg>,
            reply: ChannelOpenHandle,
            _session: &mut Session,
        ) -> Result<(), Self::Error> {
            reply.accept().await;
            Ok(())
        }

        async fn exec_request(
            &mut self,
            channel: ChannelId,
            request: &[u8],
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            let mut reply = b"echo:".to_vec();
            reply.extend_from_slice(request);
            session.data(channel, reply)?;
            session.close(channel)?;
            Ok(())
        }
    }

    struct SshClient;

    impl client::Handler for SshClient {
        type Error = russh::Error;

        async fn check_server_key(
            &mut self,
            _server_public_key: &russh::keys::PublicKeyOrCertificate,
        ) -> Result<bool, Self::Error> {
            Ok(true) // 测试环境信任回环主机密钥
        }
    }

    #[tokio::test]
    async fn ssh_runs_over_ws_relay_end_to_end() {
        let sshd = spawn_sshd().await;
        let (relay, _relay_keep) = spawn_ws_relay(Some(sshd), vec![]).await;

        // veloterm 未来的接入形态：VLR2 握手后的流直接喂 connect_stream——ws 适配器
        // 提供 AsyncRead+AsyncWrite，russh 感知不到传输层差异。
        let stream = connect_via_relay(&relay, &test_secret(), None).await;

        let config = Arc::new(client::Config::default());
        let mut session =
            tokio::time::timeout(HS_TIMEOUT, client::connect_stream(config, stream, SshClient))
                .await
                .expect("SSH 连接应在超时内建立")
                .unwrap();
        let auth = tokio::time::timeout(HS_TIMEOUT, session.authenticate_password("user", "pw"))
            .await
            .expect("SSH 认证应在超时内完成")
            .unwrap();
        assert!(auth.success(), "SSH 认证应成功（经 ws 中继）");

        let mut channel = tokio::time::timeout(HS_TIMEOUT, session.channel_open_session())
            .await
            .expect("通道应在超时内打开")
            .unwrap();
        channel.exec(true, "ping").await.unwrap();
        let mut got_echo = false;
        loop {
            let msg = tokio::time::timeout(HS_TIMEOUT, channel.wait())
                .await
                .expect("通道消息应在超时内到达");
            let Some(msg) = msg else { break };
            if let ChannelMsg::Data { data } = msg {
                assert_eq!(&data[..], b"echo:ping");
                got_echo = true;
            }
        }
        assert!(got_echo, "应收到 exec 回显");
        session
            .disconnect(russh::Disconnect::ByApplication, "done", "en")
            .await
            .ok();
    }
}
