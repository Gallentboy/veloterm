//! VLR3 传输加密：临时 X25519 密钥协商 + HKDF-SHA256 密钥推导 +
//! ChaCha20-Poly1305 AEAD 记录层。
//!
//! 安全模型：
//! - 每连接临时密钥对，公钥随握手明文传输（随机字节形态，顺带混淆协议特征）；
//! - 对称密钥 = HKDF(凭证 secret ‖ X25519 共享秘密, 双方 nonce)——**从不传输**，
//!   没有凭证的观察者（含中间人）推导不出，录制流量事后泄露凭证也解不开
//!   （临时 DH 前向保密）；
//! - 记录层每帧独立认证，篡改/重排/重放任一帧立即断连。
//!
//! 记录帧格式（握手完成后双向一致）：
//! `长度 u16 BE（密文长） ‖ 密文（明文 ‖ 16B Poly1305 tag）`
//! nonce = `0x00000000 ‖ 方向计数器 u64 BE`，每方向独立计数。

use std::pin::Pin;
use std::task::{Context, Poll};

use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, Tag};
use hkdf::Hkdf;
use sha2::Sha256;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::auth::Secret;

pub const EPHEMERAL_PUB_LEN: usize = 32;
pub const SESSION_KEY_LEN: usize = 32;
/// 单条记录明文上限（密文 = 明文 + 16B tag ≤ 65535 → 明文 ≤ 65519）。
pub const RECORD_PLAINTEXT_MAX: usize = 65535 - 16;
/// HKDF info 域（协议域分隔）。
const HKDF_INFO: &[u8] = b"veloterm-relay-v3";

/// 一次性密钥协商材料（客户端/服务端同构：生成临时密钥对）。
pub struct EphemeralKey {
    secret: StaticSecret,
}

impl EphemeralKey {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("系统熵源不可用");
        Self { secret: StaticSecret::from(bytes) }
    }

    pub fn public(&self) -> [u8; EPHEMERAL_PUB_LEN] {
        *PublicKey::from(&self.secret).as_bytes()
    }

    pub fn shared(&self, peer_pub: &[u8; EPHEMERAL_PUB_LEN]) -> [u8; 32] {
        *self.secret.diffie_hellman(&PublicKey::from(*peer_pub)).as_bytes()
    }
}

/// 双向会话密钥（协商产物，两端一致）。
#[derive(Clone)]
pub struct SessionKeys {
    pub client_to_server: [u8; SESSION_KEY_LEN],
    pub server_to_client: [u8; SESSION_KEY_LEN],
}

/// 密钥推导：`HKDF-SHA256(ikm = X25519共享 ‖ 凭证, salt = 客户端nonce ‖ 服务端nonce)`。
/// 凭证参与推导实现通道绑定：无凭证者即使完成 DH（中间人）也得不到密钥。
pub fn derive_session_keys(
    credential: &Secret,
    shared: &[u8; 32],
    client_nonce: &[u8; 16],
    server_nonce: &[u8; 16],
) -> SessionKeys {
    let mut ikm = Vec::with_capacity(64);
    ikm.extend_from_slice(shared);
    ikm.extend_from_slice(credential.as_bytes());
    let mut salt = [0u8; 32];
    salt[..16].copy_from_slice(client_nonce);
    salt[16..].copy_from_slice(server_nonce);
    let hk = Hkdf::<Sha256>::new(Some(&salt), &ikm);
    let mut okm = [0u8; 2 * SESSION_KEY_LEN];
    hk.expand(HKDF_INFO, &mut okm).expect("HKDF 输出长度合法");
    let (c2s, s2c) = okm.split_at(SESSION_KEY_LEN);
    SessionKeys {
        client_to_server: c2s.try_into().unwrap(),
        server_to_client: s2c.try_into().unwrap(),
    }
}

/// AEAD 单向加密器（计数器 nonce）。
struct DirectionCipher {
    cipher: ChaCha20Poly1305,
    counter: u64,
}

impl DirectionCipher {
    fn new(key: &[u8; SESSION_KEY_LEN]) -> Self {
        Self { cipher: ChaCha20Poly1305::new(Key::from_slice(key)), counter: 0 }
    }

    #[inline(always)]
    fn nonce(&self) -> Nonce {
        let mut n = [0u8; 12];
        n[4..].copy_from_slice(&self.counter.to_be_bytes());
        Nonce::from(n)
    }

    #[inline]
    fn seal(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let mut buf = plaintext.to_vec();
        let tag: Tag = self
            .cipher
            .encrypt_in_place_detached(&self.nonce(), &[], &mut buf)
            .expect("加密不失败（内存操作）");
        self.counter = self.counter.checked_add(1).expect("记录计数器溢出（2^64 条，不可达）");
        let mut out = Vec::with_capacity(buf.len() + 16);
        out.extend_from_slice(&buf);
        out.extend_from_slice(&tag);
        out
    }

    #[inline]
    fn open(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, ()> {
        if ciphertext.len() < 16 {
            return Err(());
        }
        let (body, tag) = ciphertext.split_at(ciphertext.len() - 16);
        let mut buf = body.to_vec();
        self.cipher
            .decrypt_in_place_detached(&self.nonce(), &[], &mut buf, tag.into())
            .map_err(|_| ())?;
        self.counter = self.counter.checked_add(1).ok_or(())?;
        Ok(buf)
    }
}

fn record_error(msg: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg)
}

/// 加密字节流包装器：内层任意 AsyncRead+AsyncWrite，外层呈现为普通字节流。
/// 写侧缓冲到 flush，按 ≤RECORD_PLAINTEXT_MAX 分条加密（copy_bidirectional 的
/// 64KB 写缓冲自动拆多条记录）；读侧从内层裸读累积，按「长度头 ‖ 密文」
/// 前缀解析逐条解密。任一记录校验失败 → InvalidData（连接立即作废）。
pub struct EncryptedStream<S> {
    inner: S,
    tx: DirectionCipher,
    rx: DirectionCipher,
    tx_buf: Vec<u8>,
    rx_buf: Vec<u8>,
    /// 内层裸读累积（完整/半条记录的密文字节）
    rx_raw: Vec<u8>,
    /// 正在写出的当前帧（密文）与未写完的偏移
    tx_frame: Option<(Vec<u8>, usize)>,
}

impl<S> EncryptedStream<S> {
    /// 客户端方向：tx 用 c2s、rx 用 s2c。
    pub fn new(inner: S, keys: &SessionKeys) -> Self {
        Self::with_directions(
            inner,
            keys.client_to_server,
            keys.server_to_client,
        )
    }

    /// 服务端方向：tx 用 s2c、rx 用 c2s。
    pub fn new_server(inner: S, keys: &SessionKeys) -> Self {
        Self::with_directions(
            inner,
            keys.server_to_client,
            keys.client_to_server,
        )
    }

    fn with_directions(inner: S, tx_key: [u8; SESSION_KEY_LEN], rx_key: [u8; SESSION_KEY_LEN]) -> Self {
        Self {
            inner,
            tx: DirectionCipher::new(&tx_key),
            rx: DirectionCipher::new(&rx_key),
            tx_buf: Vec::new(),
            rx_buf: Vec::new(),
            rx_raw: Vec::new(),
            tx_frame: None,
        }
    }

    pub fn into_inner(self) -> S {
        self.inner
    }

    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for EncryptedStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        loop {
            // 1. 有解密好的明文 → 直接交付
            if !this.rx_buf.is_empty() {
                let n = out.remaining().min(this.rx_buf.len());
                let chunk: Vec<u8> = this.rx_buf.drain(..n).collect();
                out.put_slice(&chunk);
                return Poll::Ready(Ok(()));
            }
            if out.remaining() == 0 {
                return Poll::Ready(Ok(()));
            }

            // 2. 从累积字节里解析尽可能多的完整记录（前缀消费）
            while this.rx_raw.len() >= 2 {
                let expect = u16::from_be_bytes([this.rx_raw[0], this.rx_raw[1]]) as usize;
                if expect < 16 {
                    return Poll::Ready(Err(record_error("记录长度非法（小于认证标签）")));
                }
                if this.rx_raw.len() < 2 + expect {
                    break; // 半条记录，继续从内层读
                }
                let ciphertext: Vec<u8> = this.rx_raw.drain(..2 + expect).skip(2).collect();
                match this.rx.open(&ciphertext) {
                    Ok(plain) => this.rx_buf = plain,
                    Err(()) => {
                        return Poll::Ready(Err(record_error(
                            "记录校验失败（数据被篡改或密钥不一致）",
                        )))
                    }
                }
                if !this.rx_buf.is_empty() {
                    break; // 回循环开头交付，避免单次 poll 无限解记录
                }
            }
            if !this.rx_buf.is_empty() {
                continue;
            }

            // 3. 内层裸读累积（64KB 暂存 → rx_raw）
            let mut scratch = [0u8; 64 * 1024];
            let mut rb = ReadBuf::new(&mut scratch);
            match Pin::new(&mut this.inner).poll_read(cx, &mut rb) {
                Poll::Ready(Ok(())) => {
                    let got = rb.filled().len();
                    if got == 0 {
                        // 内层 EOF：无半条记录 → 干净 EOF；有 → 截断错误
                        if this.rx_raw.is_empty() {
                            return Poll::Ready(Ok(()));
                        }
                        return Poll::Ready(Err(record_error("记录被截断")));
                    }
                    this.rx_raw.extend_from_slice(rb.filled());
                    // 回循环开头尝试解析
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for EncryptedStream<S> {
    fn poll_write(self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        this.tx_buf.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        loop {
            if let Some((frame, offset)) = &mut this.tx_frame {
                while *offset < frame.len() {
                    match Pin::new(&mut this.inner).poll_write(cx, &frame[*offset..]) {
                        Poll::Ready(Ok(n)) => *offset += n,
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Pending => return Poll::Pending,
                    }
                }
                this.tx_frame = None;
            }
            if this.tx_buf.is_empty() {
                return Pin::new(&mut this.inner).poll_flush(cx);
            }
            // 取 ≤RECORD_PLAINTEXT_MAX 一段封帧（长度头 + 密文）
            let take = this.tx_buf.len().min(RECORD_PLAINTEXT_MAX);
            let chunk: Vec<u8> = this.tx_buf.drain(..take).collect();
            let ciphertext = this.tx.seal(&chunk);
            let mut frame = Vec::with_capacity(2 + ciphertext.len());
            frame.extend_from_slice(&(ciphertext.len() as u16).to_be_bytes());
            frame.extend_from_slice(&ciphertext);
            this.tx_frame = Some((frame, 0));
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.as_mut().poll_flush(cx) {
            Poll::Ready(Ok(())) => {
                let this = self.get_mut();
                Pin::new(&mut this.inner).poll_shutdown(cx)
            }
            other => other,
        }
    }
}

/// 便捷：把一段明文作为记录写出（测试与小负载用；大块写自动分帧）。
pub async fn write_record<S: AsyncWrite + Unpin>(
    io: &mut EncryptedStream<S>,
    plaintext: &[u8],
) -> std::io::Result<()> {
    io.write_all(plaintext).await?;
    io.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt as _;
    use tokio::io::duplex;

    fn test_keys() -> (SessionKeys, SessionKeys) {
        let cred = Secret::from_bytes([7u8; 32]);
        let client = EphemeralKey::generate();
        let server = EphemeralKey::generate();
        let c_shared = client.shared(&server.public());
        let s_shared = server.shared(&client.public());
        assert_eq!(c_shared, s_shared, "X25519 双方共享秘密应一致");
        let cn = [1u8; 16];
        let sn = [2u8; 16];
        (
            derive_session_keys(&cred, &c_shared, &cn, &sn),
            derive_session_keys(&cred, &s_shared, &cn, &sn),
        )
    }

    #[tokio::test]
    async fn encrypted_roundtrip_both_directions() {
        let (client_keys, server_keys) = test_keys();
        let (a, b) = duplex(64 * 1024);
        let mut client = EncryptedStream::new(a, &client_keys);
        let mut server = EncryptedStream::new_server(b, &server_keys);

        write_record(&mut client, b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        server.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");

        write_record(&mut server, b"pong!").await.unwrap();
        let mut buf = [0u8; 5];
        client.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"pong!");
    }

    #[tokio::test]
    async fn wrong_keys_fail_to_decrypt() {
        let (client_keys, _) = test_keys();
        let (_, other_server_keys) = test_keys(); // 不同临时密钥对
        let (a, b) = duplex(64 * 1024);
        let mut client = EncryptedStream::new(a, &client_keys);
        let mut server = EncryptedStream::new_server(b, &other_server_keys);

        write_record(&mut client, b"secret").await.unwrap();
        let mut buf = [0u8; 6];
        assert!(server.read_exact(&mut buf).await.is_err(), "密钥不一致应解密失败");
    }

    #[tokio::test]
    async fn tampered_record_is_detected() {
        let (client_keys, server_keys) = test_keys();
        let (a, b) = duplex(64 * 1024);
        let client = EncryptedStream::new(a, &client_keys);
        let mut server = EncryptedStream::new_server(b, &server_keys);

        // 从内层裸流注入伪造记录（长度 32 密文）：服务端解密必然校验失败
        let mut raw = client.into_inner();
        let tampered = [0u8; 34];
        raw.write_all(&tampered).await.unwrap();
        raw.flush().await.unwrap();
        drop(raw);
        let mut buf = [0u8; 8];
        assert!(server.read_exact(&mut buf).await.is_err(), "篡改记录应校验失败");
    }

    #[tokio::test]
    async fn large_write_splits_into_multiple_records() {
        let (client_keys, server_keys) = test_keys();
        let (a, b) = duplex(256 * 1024);
        let mut client = EncryptedStream::new(a, &client_keys);
        let mut server = EncryptedStream::new_server(b, &server_keys);

        // 超过单条记录上限的整块写 + 读回（覆盖分帧与重组）
        let payload: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        client.write_all(&payload).await.unwrap();
        client.flush().await.unwrap();
        let mut got = vec![0u8; payload.len()];
        server.read_exact(&mut got).await.unwrap();
        assert_eq!(got, payload);
    }

    #[test]
    fn directional_keys_differ() {
        let (k1, k2) = test_keys();
        assert_eq!(k1.client_to_server, k2.client_to_server);
        assert_ne!(k1.client_to_server, k1.server_to_client, "双向密钥应不同");
    }
}
