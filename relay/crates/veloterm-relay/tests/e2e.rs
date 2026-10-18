//! 集成测试：真实 socket 跑通「客户端 → VLR2 握手 → 中继 → 目标」全链路，
//! 最后一项用 russh 服务端/客户端证明 SSH 协议能完整跑在中继管道上
//! （这正是 veloterm 主程序将来的接入形态：connect_stream over 握手后的流）。

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use veloterm_relay_proto::auth::Secret;
use veloterm_relay_proto::frame::{Status, TargetAddr};
use veloterm_relay_proto::handshake::{client_handshake, AuthEntry, ServerAuth};

use veloterm_relay::config::{ListenerProto, ResolvedListener};
use veloterm_relay::server;

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

/// 起中继（先同步 bind 拿到端口，再 spawn 服务任务，避免连接竞态）。
/// 返回（中继地址，shutdown sender）——sender 必须由调用方持有，
/// 提前 drop 会让 watch 通道关闭、服务任务立即退出。
async fn spawn_relay(default: Option<TargetAddr>, allowed: Vec<TargetAddr>) -> (TargetAddr, tokio::sync::watch::Sender<()>) {
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bind = tcp.local_addr().unwrap();
    let listener = ResolvedListener {
        proto: ListenerProto::Tcp,
        bind,
        default_target: default,
        allowed,
    };
    let (tx, rx) = tokio::sync::watch::channel(());
    tokio::spawn(server::run_tcp_on(
        tcp,
        listener,
        test_auth(),
        Arc::new(tokio::sync::Semaphore::new(64)),
        HS_TIMEOUT,
        rx,
    ));
    (TargetAddr { host: bind.ip().to_string(), port: bind.port() }, tx)
}

/// 经中继建立一条已握手的流。
async fn connect_via_relay(relay: &TargetAddr, secret: &Secret, target: Option<&TargetAddr>) -> TcpStream {
    let mut stream = TcpStream::connect((relay.host.as_str(), relay.port)).await.unwrap();
    client_handshake(&mut stream, KEY_ID, secret, target, HS_TIMEOUT)
        .await
        .expect("握手应成功");
    stream
}

#[tokio::test]
async fn tcp_relay_pipes_bytes_both_ways() {
    let echo = spawn_echo_target().await;
    let (relay, _relay_keep) = spawn_relay(Some(echo.clone()), vec![]).await;

    let mut stream = connect_via_relay(&relay, &test_secret(), None).await;
    stream.write_all(b"hello relay").await.unwrap();
    let mut buf = vec![0u8; 11];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(buf, b"hello relay");
}

#[tokio::test]
async fn wrong_secret_is_rejected_with_status() {
    let echo = spawn_echo_target().await;
    let (relay, _relay_keep) = spawn_relay(Some(echo), vec![]).await;

    let mut stream = TcpStream::connect((relay.host.as_str(), relay.port)).await.unwrap();
    let wrong = Secret::from_bytes([1u8; 32]);
    let err = client_handshake(&mut stream, KEY_ID, &wrong, None, HS_TIMEOUT)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        veloterm_relay_proto::handshake::ClientHandshakeError::Rejected(Status::AuthFailed)
    ));
}

#[tokio::test]
async fn target_allowlist_gates_client_supplied_targets() {
    let echo1 = spawn_echo_target().await;
    let echo2 = spawn_echo_target().await;
    let stranger = spawn_echo_target().await;
    let (relay, _relay_keep) = spawn_relay(Some(echo1.clone()), vec![echo2.clone()]).await;

    // 白名单内：连 echo2 并确认数据走的是 echo2（回显即可确认链路通）
    let mut s = connect_via_relay(&relay, &test_secret(), Some(&echo2)).await;
    s.write_all(b"to-echo2").await.unwrap();
    let mut buf = vec![0u8; 8];
    s.read_exact(&mut buf).await.unwrap();
    assert_eq!(buf, b"to-echo2");

    // 白名单外：拒绝
    let mut s = TcpStream::connect((relay.host.as_str(), relay.port)).await.unwrap();
    let err = client_handshake(&mut s, KEY_ID, &test_secret(), Some(&stranger), HS_TIMEOUT)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        veloterm_relay_proto::handshake::ClientHandshakeError::Rejected(Status::TargetNotAllowed)
    ));
}

#[tokio::test]
async fn unreachable_target_reports_connect_failed() {
    let dead = TargetAddr { host: "127.0.0.1".into(), port: 1 }; // 回环 1 口无监听
    let (relay, _relay_keep) = spawn_relay(Some(dead), vec![]).await;

    let mut s = TcpStream::connect((relay.host.as_str(), relay.port)).await.unwrap();
    let err = client_handshake(&mut s, KEY_ID, &test_secret(), None, HS_TIMEOUT)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        veloterm_relay_proto::handshake::ClientHandshakeError::Rejected(Status::ConnectFailed)
    ));
}

#[cfg(test)]
mod russh_e2e {
    use super::*;

    use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
    use russh::keys::PrivateKey;
    use russh::server::{self, Auth, ChannelOpenHandle, Session};
    use russh::{Channel, ChannelId, ChannelMsg, client};

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
            _channel: Channel<server::Msg>,
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
    async fn ssh_runs_over_relay_end_to_end() {
        let sshd = spawn_sshd().await;
        let (relay, _relay_keep) = spawn_relay(Some(sshd), vec![]).await;

        // veloterm 未来的接入形态：VLR2 握手后的 TcpStream 直接喂 connect_stream
        let mut stream = TcpStream::connect((relay.host.as_str(), relay.port)).await.unwrap();
        client_handshake(&mut stream, KEY_ID, &test_secret(), None, HS_TIMEOUT)
            .await
            .unwrap();

        let config = Arc::new(client::Config::default());
        let mut session = client::connect_stream(config, stream, SshClient)
            .await
            .unwrap();
        let auth = session.authenticate_password("user", "pw").await.unwrap();
        assert!(auth.success(), "SSH 认证应成功（经中继）");

        let mut channel = session.channel_open_session().await.unwrap();
        channel.exec(true, "ping").await.unwrap();
        let mut got_echo = false;
        while let Some(msg) = channel.wait().await {
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

    // 断言密钥算法形状（防 API 漂移时静默通过）
    #[test]
    fn host_key_shape_is_ed25519() {
        let keypair = Ed25519Keypair::from_seed(&[0xED; 32]);
        let key = PrivateKey::new(KeypairData::from(keypair), "t").unwrap();
        assert_eq!(key.algorithm(), russh::keys::ssh_key::Algorithm::Ed25519);
    }
}
