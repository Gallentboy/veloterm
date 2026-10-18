//! VLR2 流式握手驱动（服务端与 veloterm 客户端共用）。
//!
//! 时序与错误路径约定：
//! - 服务端 [`server_handshake`] 成功返回解析出的目标地址，**不发送** `Ok`/`ConnectFailed`
//!   终局状态——目标是否连得上只有调用方知道，由调用方经 [`send_status`] 收尾；
//!   握手内部的失败（格式/鉴权/选路）由本函数自己回写对应状态后返回 `Err`。
//! - 客户端 [`client_handshake`] 返回 `Ok` 后，流即为纯字节管道（SSH 等直接在其上运行）。

use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;

use crate::auth::{self, PROOF_LEN, Secret};
use crate::frame::{ClientHello, ServerFirst, Status, TargetAddr, HELLO_FIXED_LEN, MAX_HELLO_LEN};

/// 服务端失败原因。Io 里 `TimedOut` 表示握手超时。
#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("握手帧格式非法")]
    Malformed,
    #[error("鉴权失败（证明不匹配）")]
    AuthFailed,
    #[error("目标不被允许（未在 allow_targets 白名单且无默认目标）")]
    TargetNotAllowed,
}

/// 客户端失败原因。`Rejected` 携带服务端终局状态码，供 UI 呈现具体原因。
#[derive(Debug, thiserror::Error)]
pub enum ClientHandshakeError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("服务端响应帧非法")]
    BadServer,
    #[error("被服务端拒绝: {0:?}")]
    Rejected(Status),
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
/// `allowed` 为白名单（空 = 不限制，即「令牌鉴权的开放中继」——
/// 凭证有效即可达任意目标），`default_target` 仅供客户端未携带目标的
/// 场景兜底（UDP 模式等）。白名单存在时客户端目标必须在其中。
#[derive(Debug, Clone, Default)]
pub struct TargetPolicy {
    pub default_target: Option<TargetAddr>,
    pub allowed: Vec<TargetAddr>,
}

/// 发送终局状态帧（`Ok` / `ConnectFailed` 等）。握手成功后由服务端调用方使用。
pub async fn send_status<S: AsyncWrite + Unpin>(io: &mut S, status: Status) -> std::io::Result<()> {
    io.write_all(&status.encode_frame()).await?;
    io.flush().await
}

async fn read_exact_or<S: AsyncRead + Unpin>(io: &mut S, buf: &mut [u8]) -> Result<(), HandshakeError> {
    // 对端中途断开视作格式错误（半个握手帧），与超时一样由调用方记日志
    io.read_exact(buf).await.map_err(map_eof)?;
    Ok(())
}

fn map_eof(e: std::io::Error) -> HandshakeError {
    if e.kind() == std::io::ErrorKind::UnexpectedEof {
        HandshakeError::Malformed
    } else {
        HandshakeError::Io(e)
    }
}

fn map_client_eof(e: std::io::Error) -> ClientHandshakeError {
    if e.kind() == std::io::ErrorKind::UnexpectedEof {
        ClientHandshakeError::BadServer
    } else {
        ClientHandshakeError::Io(e)
    }
}

async fn read_client_hello<S: AsyncRead + Unpin>(io: &mut S) -> Result<ClientHello, HandshakeError> {
    let mut buf = vec![0u8; HELLO_FIXED_LEN];
    read_exact_or(io, &mut buf).await?;
    if buf[..4] != crate::frame::MAGIC {
        return Err(HandshakeError::Malformed);
    }
    if buf[4] & crate::frame::FLAG_HAS_TARGET != 0 {
        buf.push(0);
        read_exact_or(io, &mut buf[HELLO_FIXED_LEN..=HELLO_FIXED_LEN]).await?;
        let host_len = buf[HELLO_FIXED_LEN] as usize;
        if host_len == 0 || buf.len() + host_len + 2 > MAX_HELLO_LEN {
            return Err(HandshakeError::Malformed);
        }
        buf.resize(HELLO_FIXED_LEN + 1 + host_len + 2, 0);
        read_exact_or(io, &mut buf[HELLO_FIXED_LEN + 1..]).await?;
    }
    ClientHello::decode(&buf).map_err(|_| HandshakeError::Malformed)
}

/// 服务端握手。返回（目标地址，凭证名称）。
///
/// 对未知 key_id 也走完全相同的「continue → 读证明」流程再拒绝，
/// 避免通过响应差异枚举有效 key_id。
pub async fn server_handshake<S>(
    io: &mut S,
    server_auth: &ServerAuth,
    policy: &TargetPolicy,
    hs_timeout: Duration,
) -> Result<(TargetAddr, String), HandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    timeout(hs_timeout, async {
        let hello = match read_client_hello(io).await {
            Ok(h) => h,
            Err(e) => {
                let _ = send_status(io, Status::Malformed).await;
                return Err(e);
            }
        };

        // 已知 key_id 用真 secret 验证明；未知的用随机 secret 走同形流程
        // （证明必败），响应路径与已知条目完全一致。
        let entry = match server_auth.get(hello.key_id) {
            Some(entry) => entry,
            None => {
                let decoy = Secret::generate();
                let server_nonce = auth::random_nonce();
                io.write_all(&ServerFirst::encode_continue(server_nonce))
                    .await?;
                io.flush().await?;
                let mut proof = [0u8; PROOF_LEN];
                read_exact_or(io, &mut proof).await?;
                let _ = auth::verify_stream_proof(
                    &decoy,
                    hello.key_id,
                    &hello.client_nonce,
                    &server_nonce,
                    hello.target.as_ref(),
                    &proof,
                );
                let _ = send_status(io, Status::AuthFailed).await;
                return Err(HandshakeError::AuthFailed);
            }
        };

        let server_nonce = auth::random_nonce();
        io.write_all(&ServerFirst::encode_continue(server_nonce))
            .await?;
        io.flush().await?;

        let mut proof = [0u8; PROOF_LEN];
        read_exact_or(io, &mut proof).await?;

        if !auth::verify_stream_proof(
            &entry.secret,
            hello.key_id,
            &hello.client_nonce,
            &server_nonce,
            hello.target.as_ref(),
            &proof,
        ) {
            let _ = send_status(io, Status::AuthFailed).await;
            return Err(HandshakeError::AuthFailed);
        }

        let target = match &hello.target {
            Some(t) => {
                if policy.allowed.is_empty() || policy.allowed.iter().any(|a| a == t) {
                    t.clone()
                } else {
                    let _ = send_status(io, Status::TargetNotAllowed).await;
                    return Err(HandshakeError::TargetNotAllowed);
                }
            }
            None => match &policy.default_target {
                Some(t) => t.clone(),
                None => {
                    let _ = send_status(io, Status::TargetNotAllowed).await;
                    return Err(HandshakeError::TargetNotAllowed);
                }
            },
        };
        Ok((target, entry.name.clone()))
    })
    .await
    .map_err(|_| {
        HandshakeError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "handshake timeout",
        ))
    })?
}

/// 客户端握手。成功后 `io` 即为纯字节管道。
pub async fn client_handshake<S>(
    io: &mut S,
    key_id: u8,
    secret: &Secret,
    target: Option<&TargetAddr>,
    hs_timeout: Duration,
) -> Result<(), ClientHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    timeout(hs_timeout, async {
        let hello = ClientHello {
            key_id,
            client_nonce: auth::random_nonce(),
            target: target.cloned(),
        };
        io.write_all(&hello.encode()).await?;
        io.flush().await?;

        let mut head5 = [0u8; 5];
        io.read_exact(&mut head5).await.map_err(map_client_eof)?;
        let rest16: Vec<u8> = if head5[4] == crate::frame::CONTINUE_MARKER {
            let mut nonce_buf = [0u8; 16];
            io.read_exact(&mut nonce_buf).await.map_err(map_client_eof)?;
            nonce_buf.to_vec()
        } else {
            Vec::new()
        };
        let first =
            ServerFirst::decode(&head5, &rest16).map_err(|_| ClientHandshakeError::BadServer)?;
        let server_nonce = match first {
            ServerFirst::Continue { server_nonce } => server_nonce,
            ServerFirst::Status(status) => return Err(ClientHandshakeError::Rejected(status)),
        };

        let proof = auth::stream_proof(secret, key_id, &hello.client_nonce, &server_nonce, target);
        io.write_all(&proof).await?;
        io.flush().await?;

        let mut status_frame = [0u8; 5];
        io.read_exact(&mut status_frame).await.map_err(map_client_eof)?;
        if status_frame[..4] != crate::frame::MAGIC {
            return Err(ClientHandshakeError::BadServer);
        }
        match Status::from_u8(status_frame[4]) {
            Some(Status::Ok) => Ok(()),
            Some(status) => Err(ClientHandshakeError::Rejected(status)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    fn auth_store() -> (ServerAuth, Secret) {
        let secret = Secret::from_bytes([9u8; 32]);
        let mut store = ServerAuth::new();
        store.add(AuthEntry {
            key_id: 9,
            name: "test".into(),
            secret: secret.clone(),
        });
        (store, secret)
    }

    fn policy() -> TargetPolicy {
        TargetPolicy {
            default_target: Some(TargetAddr::parse("127.0.0.1:22").unwrap()),
            allowed: vec![TargetAddr::parse("10.1.1.5:2222").unwrap()],
        }
    }

    #[tokio::test]
    async fn handshake_success_then_pure_pipe() {
        let (store, secret) = auth_store();
        let (mut client_io, mut server_io) = duplex(4096);

        let server = tokio::spawn(async move {
            let (target, name) = server_handshake(&mut server_io, &store, &policy(), TEST_TIMEOUT)
                .await
                .unwrap();
            send_status(&mut server_io, Status::Ok).await.unwrap();
            // 握手完成后回显：验证管道是纯字节流
            let mut buf = [0u8; 4];
            server_io.read_exact(&mut buf).await.unwrap();
            server_io.write_all(&buf).await.unwrap();
            (target, name)
        });

        client_handshake(&mut client_io, 9, &secret, None, TEST_TIMEOUT)
            .await
            .unwrap();
        client_io.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        client_io.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");

        let (target, name) = server.await.unwrap();
        assert_eq!(target, TargetAddr::parse("127.0.0.1:22").unwrap());
        assert_eq!(name, "test");
    }

    #[tokio::test]
    async fn handshake_reject_wrong_secret() {
        let (store, _) = auth_store();
        let wrong = Secret::from_bytes([1u8; 32]);
        let (mut client_io, mut server_io) = duplex(4096);

        let server = tokio::spawn(async move {
            server_handshake(&mut server_io, &store, &policy(), TEST_TIMEOUT)
                .await
                .unwrap_err()
        });

        let err = client_handshake(&mut client_io, 9, &wrong, None, TEST_TIMEOUT)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ClientHandshakeError::Rejected(Status::AuthFailed)
        ));
        assert!(matches!(server.await.unwrap(), HandshakeError::AuthFailed));
    }

    #[tokio::test]
    async fn handshake_target_policy_enforced() {
        // 白名单之外的显式目标 → 双端各自得到 TargetNotAllowed
        let (store, secret) = auth_store();
        let denied = TargetAddr::parse("8.8.8.8:53").unwrap();
        let (mut c, mut s) = duplex(4096);
        let server = tokio::spawn(async move {
            server_handshake(&mut s, &store, &policy(), TEST_TIMEOUT).await
        });
        let err = client_handshake(&mut c, 9, &secret, Some(&denied), TEST_TIMEOUT)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ClientHandshakeError::Rejected(Status::TargetNotAllowed)
        ));
        assert!(matches!(
            server.await.unwrap().unwrap_err(),
            HandshakeError::TargetNotAllowed
        ));

        // 白名单内的目标 → 放行并回传该目标
        let (store, secret) = auth_store();
        let allowed = TargetAddr::parse("10.1.1.5:2222").unwrap();
        let (mut c2, mut s2) = duplex(4096);
        let server2 = tokio::spawn(async move {
            let (t, _) = server_handshake(&mut s2, &store, &policy(), TEST_TIMEOUT)
                .await
                .unwrap();
            send_status(&mut s2, Status::Ok).await.unwrap();
            t
        });
        client_handshake(&mut c2, 9, &secret, Some(&allowed), TEST_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(server2.await.unwrap(), allowed);
    }

    #[tokio::test]
    async fn handshake_unknown_key_id_uniformly_rejected() {
        let (store, some_secret) = auth_store();
        let (mut c, mut s) = duplex(4096);
        let server = tokio::spawn(async move {
            server_handshake(&mut s, &store, &policy(), TEST_TIMEOUT).await
        });
        let err = client_handshake(&mut c, 200, &some_secret, None, TEST_TIMEOUT)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ClientHandshakeError::Rejected(Status::AuthFailed)
        ));
        assert!(matches!(
            server.await.unwrap().unwrap_err(),
            HandshakeError::AuthFailed
        ));
    }

    #[tokio::test]
    async fn open_relay_allows_any_client_target() {
        // 开放中继：无白名单（allowed 空）+ 无默认目标 → 客户端携带任意目标均放行
        let (store, secret) = auth_store();
        let (mut c, mut s) = duplex(4096);
        let server = tokio::spawn(async move {
            let (t, _) = server_handshake(
                &mut s,
                &store,
                &TargetPolicy { default_target: None, allowed: vec![] },
                TEST_TIMEOUT,
            )
            .await
            .unwrap();
            send_status(&mut s, Status::Ok).await.unwrap();
            t
        });
        let anywhere = TargetAddr::parse("203.0.113.7:2222").unwrap();
        client_handshake(&mut c, 9, &secret, Some(&anywhere), TEST_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(server.await.unwrap(), anywhere);
    }

    #[tokio::test]
    async fn handshake_client_timeout_on_silent_peer() {
        // 服务端一言不发 → 客户端握手超时
        let (_unused_tx, mut silent_rx) = tokio::io::duplex(64);
        let secret = Secret::from_bytes([3u8; 32]);
        let err = client_handshake(&mut silent_rx, 1, &secret, None, Duration::from_millis(80))
            .await
            .unwrap_err();
        assert!(matches!(err, ClientHandshakeError::Io(ref e) if e.kind() == std::io::ErrorKind::TimedOut));
    }
}
