//! VLR3 流式握手驱动（服务端与 veloterm 客户端共用）：**先密钥协商、后加密认证**。
//!
//! ```text
//! ① 密钥协商（明文区，仅含随机字节形态的公钥与 nonce——协议特征被混淆）
//!    C→R: "VLR3" | key_id u8 | client_pub(32) | client_nonce(16)      （53B）
//!    R→C: "VLR3" | server_pub(32) | server_nonce(16)                  （52B）
//!    双方: keys = HKDF-SHA256(凭证 ‖ X25519共享, 双nonce)  ← 从不传输
//! ② 认证+选路（全部进 AEAD 记录层）
//!    C→R: 加密{ flags u8 | [target] | proof(32) }   ← 目标地址也被隐藏
//!    R→C: 加密{ status u8 }
//! ③ 之后双向字节管道整体跑在加密记录层内（SSH 端到端加密嵌套其中）。
//! ```
//!
//! proof = HMAC-SHA256(secret, "veloterm-relay-v3/auth" ‖ key_id ‖ 双公钥 ‖
//! 双nonce ‖ target字节)——绑定完整协商上下文。密钥由凭证参与推导，凭证不匹配
//! 则双方记录层密钥不同、记录校验必败（服务端对未知 key_id 用随机诱饵走同形
//! 流程，防枚举）。
//!
//! 服务端 [`server_handshake`] 成功返回 [`ServerSession`]（含加密流与目标）；
//! 终局状态（Ok/ConnectFailed）由调用方经 [`send_status`] 以加密记录发送。
//! 客户端 [`client_handshake`] 成功返回加密流，之后即为纯字节管道。

use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;

use crate::auth::{self, NONCE_LEN as NonceLen, PROOF_LEN, Secret};
use crate::crypto::{derive_session_keys, EphemeralKey, EncryptedStream};
use crate::frame::{Status, TargetAddr};

/// 服务端失败原因。
#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("握手帧格式非法")]
    Malformed,
    #[error("鉴权失败（凭证不匹配或记录校验失败）")]
    AuthFailed,
    #[error("目标不被允许（未在 allow_targets 白名单且无默认目标）")]
    TargetNotAllowed,
}

/// 客户端失败原因。
#[derive(Debug, thiserror::Error)]
pub enum ClientHandshakeError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("服务端响应帧非法")]
    BadServer,
    #[error("被服务端拒绝: {0:?}")]
    Rejected(Status),
    #[error("凭证不匹配（keyId/secret 与服务端不一致）或链路数据损坏")]
    AuthMismatch,
}

/// 一条客户端凭证条目。
#[derive(Debug, Clone)]
pub struct AuthEntry {
    pub key_id: u8,
    pub name: String,
    pub secret: Secret,
}

#[derive(Debug, Default)]
pub struct ServerAuth {
    entries: Vec<AuthEntry>,
}

impl ServerAuth {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, entry: AuthEntry) {
        self.entries.push(entry);
    }

    pub fn get(&self, key_id: u8) -> Option<&AuthEntry> {
        self.entries.iter().find(|e| e.key_id == key_id)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 目标选路策略：客户端默认携带目标（即其配置的 SSH 主机地址）；
/// `allowed` 为白名单（空 = 不限制，即「令牌鉴权的开放中继」），
/// `default_target` 仅供客户端未携带目标的场景兜底（UDP 模式等）。
#[derive(Debug, Clone, Default)]
pub struct TargetPolicy {
    pub default_target: Option<TargetAddr>,
    pub allowed: Vec<TargetAddr>,
}

/// 服务端握手产物：加密流 + 解析出的目标与凭证名。
pub struct ServerSession<S> {
    pub stream: EncryptedStream<S>,
    pub target: TargetAddr,
    pub principal: String,
}

const MAGIC: [u8; 4] = *b"VLR3";
const CLIENT_HELLO_LEN: usize = 4 + 1 + 32 + 16;
const SERVER_HELLO_LEN: usize = 4 + 32 + 16;
/// 认证证明的域分隔（与 VLR2 的 DOM_STREAM 不同域，防跨版本重放）。
const DOM_AUTH: &[u8] = b"veloterm-relay-v3/auth";

/// 发送终局状态（1 字节加密记录）。握手成功后由服务端调用方使用。
pub async fn send_status<S: AsyncWrite + Unpin>(
    io: &mut EncryptedStream<S>,
    status: Status,
) -> std::io::Result<()> {
    io.write_all(&[status as u8]).await?;
    io.flush().await
}

fn transcript(
    key_id: u8,
    client_pub: &[u8; 32],
    server_pub: &[u8; 32],
    client_nonce: &[u8; NonceLen],
    server_nonce: &[u8; NonceLen],
    target: Option<&TargetAddr>,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(DOM_AUTH.len() + 1 + 64 + 32 + 300);
    buf.extend_from_slice(DOM_AUTH);
    buf.push(key_id);
    buf.extend_from_slice(client_pub);
    buf.extend_from_slice(server_pub);
    buf.extend_from_slice(client_nonce);
    buf.extend_from_slice(server_nonce);
    if let Some(t) = target {
        buf.extend_from_slice(t.host.as_bytes());
        buf.extend_from_slice(&t.port.to_be_bytes());
    }
    buf
}

fn auth_proof(
    secret: &Secret,
    key_id: u8,
    client_pub: &[u8; 32],
    server_pub: &[u8; 32],
    client_nonce: &[u8; NonceLen],
    server_nonce: &[u8; NonceLen],
    target: Option<&TargetAddr>,
) -> [u8; PROOF_LEN] {
    let t = transcript(key_id, client_pub, server_pub, client_nonce, server_nonce, target);
    hmac_sha256(secret.as_bytes(), &[&t])
}

fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; PROOF_LEN] {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC 接受任意长度密钥");
    for p in parts {
        mac.update(p);
    }
    mac.finalize().into_bytes().into()
}

/// 服务端握手。凭证不匹配（含未知 key_id 走诱饵）时记录层校验必败，
/// 返回 [`HandshakeError::AuthFailed`]；协议格式错误返回 `Malformed`。
#[allow(clippy::too_many_arguments)]
pub async fn server_handshake<S>(
    io: S,
    server_auth: &ServerAuth,
    policy: &TargetPolicy,
    hs_timeout: Duration,
) -> Result<ServerSession<S>, HandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    timeout(hs_timeout, async move {
        let mut io = io;
        // ① 客户端 hello（明文）
        let mut c_hello = [0u8; CLIENT_HELLO_LEN];
        io.read_exact(&mut c_hello).await.map_err(map_eof_malformed)?;
        if c_hello[..4] != MAGIC {
            return Err(HandshakeError::Malformed);
        }
        let key_id = c_hello[4];
        let client_pub: [u8; 32] = c_hello[5..37].try_into().unwrap();
        let client_nonce: [u8; NonceLen] = c_hello[37..53].try_into().unwrap();

        // 未知 key_id 用随机诱饵走同形流程（防枚举）：密钥不同 → 记录校验必败
        let entry = server_auth.get(key_id);
        let verify_secret = match entry {
            Some(e) => e.secret.clone(),
            None => Secret::generate(),
        };

        // ② 服务端 hello（明文）：临时密钥对 + nonce
        let eph = EphemeralKey::generate();
        let server_pub = eph.public();
        let server_nonce = auth::random_nonce();
        let mut s_hello = [0u8; SERVER_HELLO_LEN];
        s_hello[..4].copy_from_slice(&MAGIC);
        s_hello[4..36].copy_from_slice(&server_pub);
        s_hello[36..].copy_from_slice(&server_nonce);
        io.write_all(&s_hello).await?;
        io.flush().await?;

        let shared = eph.shared(&client_pub);
        let keys = derive_session_keys(&verify_secret, &shared, &client_nonce, &server_nonce);
        let mut enc = EncryptedStream::new_server(io, &keys);

        // ③ 客户端认证记录（密文）：flags | [target] | proof
        //    凭证不匹配 → 解密失败 → AuthFailed（无法也无需回执）
        let flags = match enc.read_u8().await {
            Ok(f) => f,
            Err(_) => return Err(HandshakeError::AuthFailed),
        };
        let client_target = if flags & 0x01 != 0 {
            let host_len = match enc.read_u8().await {
                Ok(n) => n as usize,
                Err(_) => return Err(HandshakeError::AuthFailed),
            };
            if host_len == 0 || host_len > 255 {
                return Err(HandshakeError::Malformed);
            }
            let mut host = vec![0u8; host_len];
            if enc.read_exact(&mut host).await.is_err() {
                return Err(HandshakeError::AuthFailed);
            }
            let mut port_buf = [0u8; 2];
            if enc.read_exact(&mut port_buf).await.is_err() {
                return Err(HandshakeError::AuthFailed);
            }
            let host = String::from_utf8(host).map_err(|_| HandshakeError::Malformed)?;
            Some(TargetAddr { host, port: u16::from_be_bytes(port_buf) })
        } else {
            None
        };
        let mut proof = [0u8; PROOF_LEN];
        if enc.read_exact(&mut proof).await.is_err() {
            return Err(HandshakeError::AuthFailed);
        }

        let Some(entry) = entry else {
            // 诱饵路径：密钥随机，解密不应成功；兜底按鉴权失败
            return Err(HandshakeError::AuthFailed);
        };
        let expected = auth_proof(
            &entry.secret,
            key_id,
            &client_pub,
            &server_pub,
            &client_nonce,
            &server_nonce,
            client_target.as_ref(),
        );
        if !auth::const_eq(&expected, &proof) {
            let _ = send_status(&mut enc, Status::AuthFailed).await;
            return Err(HandshakeError::AuthFailed);
        }

        let target = match client_target {
            Some(t) => {
                if policy.allowed.is_empty() || policy.allowed.contains(&t) {
                    t
                } else {
                    let _ = send_status(&mut enc, Status::TargetNotAllowed).await;
                    return Err(HandshakeError::TargetNotAllowed);
                }
            }
            None => match &policy.default_target {
                Some(t) => t.clone(),
                None => {
                    let _ = send_status(&mut enc, Status::TargetNotAllowed).await;
                    return Err(HandshakeError::TargetNotAllowed);
                }
            },
        };
        Ok(ServerSession { stream: enc, target, principal: entry.name.clone() })
    })
    .await
    .map_err(|_| {
        HandshakeError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "handshake timeout",
        ))
    })?
}

/// 客户端握手。成功后返回的加密流即为纯字节管道（SSH 直接跑在上面）。
pub async fn client_handshake<S>(
    io: S,
    key_id: u8,
    secret: &Secret,
    target: Option<&TargetAddr>,
    hs_timeout: Duration,
) -> Result<EncryptedStream<S>, ClientHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    timeout(hs_timeout, async move {
        let mut io = io;
        // ① 客户端 hello（明文）
        let eph = EphemeralKey::generate();
        let client_pub = eph.public();
        let client_nonce = auth::random_nonce();
        let mut c_hello = [0u8; CLIENT_HELLO_LEN];
        c_hello[..4].copy_from_slice(&MAGIC);
        c_hello[4] = key_id;
        c_hello[5..37].copy_from_slice(&client_pub);
        c_hello[37..].copy_from_slice(&client_nonce);
        io.write_all(&c_hello).await?;
        io.flush().await?;

        // ② 服务端 hello（明文）
        let mut s_hello = [0u8; SERVER_HELLO_LEN];
        io.read_exact(&mut s_hello).await.map_err(map_eof_bad_server)?;
        if s_hello[..4] != MAGIC {
            return Err(ClientHandshakeError::BadServer);
        }
        let server_pub: [u8; 32] = s_hello[4..36].try_into().unwrap();
        let server_nonce: [u8; NonceLen] = s_hello[36..].try_into().unwrap();

        let shared = eph.shared(&server_pub);
        let keys = derive_session_keys(secret, &shared, &client_nonce, &server_nonce);
        let mut enc = EncryptedStream::new(io, &keys);

        // ③ 认证记录（密文）：flags | [target] | proof
        enc.write_all(&[if target.is_some() { 0x01 } else { 0x00 }]).await?;
        if let Some(t) = target {
            enc.write_all(&[t.host.len() as u8]).await?;
            enc.write_all(t.host.as_bytes()).await?;
            enc.write_all(&t.port.to_be_bytes()).await?;
        }
        let proof = auth_proof(
            secret, key_id, &client_pub, &server_pub, &client_nonce, &server_nonce, target,
        );
        enc.write_all(&proof).await?;
        enc.flush().await?;

        // ④ 终局状态（1 字节加密记录）；解密失败/断开 → 凭证不匹配
        let status = match enc.read_u8().await {
            Ok(s) => s,
            Err(_) => return Err(ClientHandshakeError::AuthMismatch),
        };
        match Status::from_u8(status) {
            Some(Status::Ok) => Ok(enc),
            Some(st) => Err(ClientHandshakeError::Rejected(st)),
            None => Err(ClientHandshakeError::BadServer),
        }
    })
    .await
    .map_err(|_| {
        ClientHandshakeError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "handshake timeout",
        ))
    })?
}

fn map_eof_malformed(e: std::io::Error) -> HandshakeError {
    if e.kind() == std::io::ErrorKind::UnexpectedEof {
        HandshakeError::Malformed
    } else {
        HandshakeError::Io(e)
    }
}

fn map_eof_bad_server(e: std::io::Error) -> ClientHandshakeError {
    if e.kind() == std::io::ErrorKind::UnexpectedEof {
        ClientHandshakeError::BadServer
    } else {
        ClientHandshakeError::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    fn auth_store() -> (ServerAuth, Secret) {
        let secret = Secret::from_bytes([9u8; 32]);
        let mut store = ServerAuth::new();
        store.add(AuthEntry { key_id: 9, name: "test".into(), secret: secret.clone() });
        (store, secret)
    }

    fn policy() -> TargetPolicy {
        TargetPolicy {
            default_target: Some(TargetAddr::parse("127.0.0.1:22").unwrap()),
            allowed: vec![TargetAddr::parse("10.1.1.5:2222").unwrap()],
        }
    }

    #[tokio::test]
    async fn handshake_success_then_encrypted_pipe() {
        let (store, secret) = auth_store();
        let (client_io, server_io) = duplex(64 * 1024);

        let server = tokio::spawn(async move {
            let mut session = server_handshake(server_io, &store, &policy(), TEST_TIMEOUT)
                .await
                .unwrap();
            let (target, principal) = (session.target.clone(), session.principal.clone());
            send_status(&mut session.stream, Status::Ok).await.unwrap();
            // 握手后加密管道回显
            let mut buf = [0u8; 4];
            session.stream.read_exact(&mut buf).await.unwrap();
            session.stream.write_all(&buf).await.unwrap();
            session.stream.flush().await.unwrap();
            (target, principal)
        });

        let mut client =
            client_handshake(client_io, 9, &secret, None, TEST_TIMEOUT).await.unwrap();
        client.write_all(b"ping").await.unwrap();
        client.flush().await.unwrap();
        let mut buf = [0u8; 4];
        client.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");

        let (target, principal) = server.await.unwrap();
        assert_eq!(target, TargetAddr::parse("127.0.0.1:22").unwrap());
        assert_eq!(principal, "test");
    }

    #[tokio::test]
    async fn wrong_secret_yields_auth_mismatch() {
        let (store, _) = auth_store();
        let wrong = Secret::from_bytes([1u8; 32]);
        let (client_io, server_io) = duplex(64 * 1024);

        let server = tokio::spawn(async move {
            server_handshake(server_io, &store, &policy(), TEST_TIMEOUT)
                .await
                .map(|_| ())
                .expect_err("错误 secret 应鉴权失败")
        });
        let err = match client_handshake(client_io, 9, &wrong, None, TEST_TIMEOUT).await {
            Err(e) => e,
            Ok(_) => panic!("错误 secret 应鉴权失败"),
        };
        assert!(matches!(err, ClientHandshakeError::AuthMismatch));
        assert!(matches!(server.await.unwrap(), HandshakeError::AuthFailed));
    }

    #[tokio::test]
    async fn target_policy_enforced() {
        // 白名单外 → 加密回执 TargetNotAllowed
        let (store, secret) = auth_store();
        let denied = TargetAddr::parse("8.8.8.8:53").unwrap();
        let (c, s) = duplex(64 * 1024);
        let server = tokio::spawn(async move {
            server_handshake(s, &store, &policy(), TEST_TIMEOUT).await
        });
        let err = match client_handshake(c, 9, &secret, Some(&denied), TEST_TIMEOUT).await {
            Err(e) => e,
            Ok(_) => panic!("白名单外目标应被拒"),
        };
        assert!(matches!(
            err,
            ClientHandshakeError::Rejected(Status::TargetNotAllowed)
        ));
        let server_result = server.await.unwrap();
        assert!(matches!(server_result.err(), Some(HandshakeError::TargetNotAllowed)));

        // 白名单内 → 放行
        let (store, secret) = auth_store();
        let allowed = TargetAddr::parse("10.1.1.5:2222").unwrap();
        let (c2, s2) = duplex(64 * 1024);
        let server2 = tokio::spawn(async move {
            let mut session = server_handshake(s2, &store, &policy(), TEST_TIMEOUT)
                .await
                .unwrap();
            send_status(&mut session.stream, Status::Ok).await.unwrap();
            session.target
        });
        client_handshake(c2, 9, &secret, Some(&allowed), TEST_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(server2.await.unwrap(), allowed);
    }

    #[tokio::test]
    async fn open_relay_allows_any_client_target() {
        // 无白名单（allowed 空）+ 无默认目标 → 客户端携带任意目标均放行
        let (store, secret) = auth_store();
        let (c, s) = duplex(64 * 1024);
        let server = tokio::spawn(async move {
            let mut session = server_handshake(
                s,
                &store,
                &TargetPolicy { default_target: None, allowed: vec![] },
                TEST_TIMEOUT,
            )
            .await
            .unwrap();
            send_status(&mut session.stream, Status::Ok).await.unwrap();
            session.target
        });
        let anywhere = TargetAddr::parse("203.0.113.7:2222").unwrap();
        client_handshake(c, 9, &secret, Some(&anywhere), TEST_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(server.await.unwrap(), anywhere);
    }

    #[tokio::test]
    async fn unknown_key_id_uniformly_rejected() {
        let (store, some_secret) = auth_store();
        let (c, s) = duplex(64 * 1024);
        let server = tokio::spawn(async move {
            server_handshake(s, &store, &policy(), TEST_TIMEOUT).await
        });
        let err = match client_handshake(c, 200, &some_secret, None, TEST_TIMEOUT).await {
            Err(e) => e,
            Ok(_) => panic!("未知 key_id 应被拒"),
        };
        assert!(matches!(err, ClientHandshakeError::AuthMismatch));
        let server_result = server.await.unwrap();
        assert!(matches!(server_result.err(), Some(HandshakeError::AuthFailed)));
    }

    #[tokio::test]
    async fn handshake_client_timeout_on_silent_peer() {
        let (_unused_tx, mut silent_rx) = tokio::io::duplex(64);
        let secret = Secret::from_bytes([3u8; 32]);
        let err = match client_handshake(&mut silent_rx, 1, &secret, None, Duration::from_millis(80)).await {
            Err(e) => e,
            Ok(_) => panic!("沉默对端应超时"),
        };
        assert!(matches!(err, ClientHandshakeError::Io(ref e) if e.kind() == std::io::ErrorKind::TimedOut));
    }
}
