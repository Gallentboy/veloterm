//! QUIC 集成测试（与 e2e.rs 的 TCP 版同构对照）：真实 QUIC socket 跑通
//! 「客户端 → VLR2 握手 → 中继 → 目标」全链路，并用 russh 服务端/客户端
//! 证明 SSH 协议能完整跑在 QUIC 双向流管道上；另验证单连接多流复用
//! （QUIC 相对 TCP 的本质差异：一条连接并行承载多个独立中继会话）。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};
use veloterm_relay_proto::auth::Secret;
use veloterm_relay_proto::frame::TargetAddr;
use veloterm_relay_proto::handshake::{client_handshake, AuthEntry, ServerAuth};

use veloterm_relay::config::{ListenerProto, ResolvedListener};
use veloterm_relay::server::quic::{self, BiStream};

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

/// 起 QUIC 中继（先同步 build endpoint 拿到端口，再 spawn 服务任务，避免连接竞态）。
/// 返回（中继地址，shutdown sender）——sender 必须由调用方持有，
/// 提前 drop 会让 watch 通道关闭、服务任务立即退出。
async fn spawn_quic_relay(
    default: Option<TargetAddr>,
    allowed: Vec<TargetAddr>,
) -> (SocketAddr, watch::Sender<()>) {
    let listener = ResolvedListener {
        proto: ListenerProto::Quic,
        bind: "127.0.0.1:0".parse().unwrap(),
        default_target: default,
        allowed,
        udp_idle_timeout: Duration::from_secs(60),
        tls_cert: None,
        tls_key: None,
    };
    let endpoint = quic::build_quic_endpoint(&listener).unwrap();
    let addr = endpoint.local_addr().unwrap();
    let (tx, rx) = watch::channel(());
    tokio::spawn(quic::serve_quic(
        endpoint,
        listener,
        test_auth(),
        Arc::new(Semaphore::new(64)),
        HS_TIMEOUT,
        rx,
    ));
    (addr, tx)
}

/// 回环测试客户端 endpoint：rustls dangerous() 自定义验证器全接受证书 + 相同 ALPN。
fn test_client_endpoint() -> quinn::Endpoint {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert))
        .with_no_client_auth();
    tls.alpn_protocols = vec![quic::ALPN_RELAY.to_vec()];
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(Arc::new(tls)).unwrap();
    let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
    endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(crypto)));
    endpoint
}

/// 连中继并开一条双向流（未握手；调用方在其上跑 VLR2 握手后即为纯字节管道）。
async fn open_relayed_stream(relay: SocketAddr) -> BiStream {
    let endpoint = test_client_endpoint();
    let conn = endpoint
        .connect(relay, "localhost")
        .unwrap()
        .await
        .expect("QUIC 连接应建立");
    let (send, recv) = conn.open_bi().await.expect("应能打开双向流");
    BiStream::new(send, recv)
}

#[derive(Debug)]
struct AcceptAnyServerCert;

impl rustls::client::danger::ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[tokio::test]
async fn quic_relay_pipes_bytes_both_ways() {
    let echo = spawn_echo_target().await;
    let (relay, _relay_keep) = spawn_quic_relay(Some(echo), vec![]).await;

    let mut stream = client_handshake(open_relayed_stream(relay).await, KEY_ID, &test_secret(), None, HS_TIMEOUT)
        .await
        .expect("握手应成功");
    let payload = b"hello quic relay";
    // 加密流写侧缓冲到 flush 才成帧发出（VLR3），写后必须显式 flush
    stream.write_all(payload).await.unwrap();
    stream.flush().await.unwrap();
    let mut buf = vec![0u8; payload.len()];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(buf, payload);
}

#[tokio::test]
async fn wrong_secret_is_rejected_with_status() {
    let echo = spawn_echo_target().await;
    let (relay, _relay_keep) = spawn_quic_relay(Some(echo), vec![]).await;

    let mut stream = open_relayed_stream(relay).await;
    let wrong = Secret::from_bytes([1u8; 32]);
    let err = match client_handshake(&mut stream, KEY_ID, &wrong, None, HS_TIMEOUT).await {
        Err(e) => e,
        Ok(_) => panic!("错误 secret 应鉴权失败"),
    };
    assert!(matches!(err, veloterm_relay_proto::handshake::ClientHandshakeError::AuthMismatch));
}

/// QUIC 的本质增益：一条连接并行承载多个独立中继会话（流间互不阻塞）。
#[tokio::test]
async fn one_connection_multiplexes_independent_sessions() {
    let echo1 = spawn_echo_target().await;
    let echo2 = spawn_echo_target().await;
    let (relay, _relay_keep) = spawn_quic_relay(Some(echo1), vec![echo2.clone()]).await;

    let endpoint = test_client_endpoint();
    let conn = endpoint
        .connect(relay, "localhost")
        .unwrap()
        .await
        .expect("QUIC 连接应建立");

    // 流 1：默认目标
    let (send1, recv1) = conn.open_bi().await.unwrap();
    let mut s1 = client_handshake(BiStream::new(send1, recv1), KEY_ID, &test_secret(), None, HS_TIMEOUT)
        .await
        .unwrap();

    // 流 2：同连接、白名单内的另一目标——独立握手、独立选路
    let (send2, recv2) = conn.open_bi().await.unwrap();
    let mut s2 = client_handshake(BiStream::new(send2, recv2), KEY_ID, &test_secret(), Some(&echo2), HS_TIMEOUT)
        .await
        .unwrap();

    // 两条流交错读写互不干扰
    let p1 = b"via-default";
    let p2 = b"via-allowlist";
    // 加密流写侧缓冲到 flush 才成帧发出（VLR3），写后必须显式 flush
    s1.write_all(p1).await.unwrap();
    s1.flush().await.unwrap();
    s2.write_all(p2).await.unwrap();
    s2.flush().await.unwrap();
    let mut b1 = vec![0u8; p1.len()];
    let mut b2 = vec![0u8; p2.len()];
    s1.read_exact(&mut b1).await.unwrap();
    s2.read_exact(&mut b2).await.unwrap();
    assert_eq!(b1, p1);
    assert_eq!(b2, p2);
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
    async fn ssh_runs_over_quic_relay_end_to_end() {
        let sshd = spawn_sshd().await;
        let (relay, _relay_keep) = spawn_quic_relay(Some(sshd), vec![]).await;

        // VLR3：握手后返回加密流，直接喂 connect_stream（SSH 嵌套在加密记录层内）
        let stream = client_handshake(open_relayed_stream(relay).await, KEY_ID, &test_secret(), None, HS_TIMEOUT)
            .await
            .unwrap();

        let config = Arc::new(client::Config::default());
        let mut session = client::connect_stream(config, stream, SshClient)
            .await
            .unwrap();
        let auth = session.authenticate_password("user", "pw").await.unwrap();
        assert!(auth.success(), "SSH 认证应成功（经 QUIC 中继）");

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
}
