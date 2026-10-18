//! QUIC 监听器（quinn + rustls/aws-lc-rs）。
//!
//! 会话模型：**单条 QUIC 双向流 = 一个中继会话**。每条流各自跑 VLR2 握手
//! （鉴权内建于 proto 层），包成 [`BiStream`] 后交给传输无关的
//! [`serve_stream`](crate::server::serve_stream)；连接可复用多流。
//! 并发上限按「流」计——每条流对应一条到目标 TCP 的连接，这才是真实资源。

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{Context as _, bail};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;
use tracing::{info, warn};
use veloterm_relay_proto::handshake::{ServerAuth, TargetPolicy};

use crate::config::ResolvedListener;
use crate::server::serve_stream;

/// ALPN 协议标识（客户端与服务端必须一致）。
pub const ALPN_RELAY: &[u8] = b"veloterm-relay";

pub async fn run_quic(
    listener: ResolvedListener,
    auth: Arc<ServerAuth>,
    conn_limit: Arc<Semaphore>,
    handshake_timeout: Duration,
    shutdown: watch::Receiver<()>,
) -> anyhow::Result<()> {
    let endpoint = build_quic_endpoint(&listener)?;
    serve_quic(endpoint, listener, auth, conn_limit, handshake_timeout, shutdown).await
}

/// 构建 TLS 服务端配置并绑定 UDP endpoint。
/// 配置了 cert/key 则从 PEM 加载；否则 rcgen 生成 Ed25519 自签证书，
/// 并把证书 DER 的 SHA-256 指纹写入日志（客户端可 pin）。
pub fn build_quic_endpoint(listener: &ResolvedListener) -> anyhow::Result<quinn::Endpoint> {
    let tls = build_server_tls(listener)?;
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls)
        .context("rustls 配置转 QUIC 加密层失败")?;
    let config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    quinn::Endpoint::server(config, listener.bind)
        .with_context(|| format!("QUIC 监听 {} UDP 绑定失败", listener.bind))
}

/// 在已构建的 endpoint 上服务（测试注入用变体：绑定与端口获取由调用方掌控，避免竞态）。
pub async fn serve_quic(
    endpoint: quinn::Endpoint,
    listener: ResolvedListener,
    auth: Arc<ServerAuth>,
    conn_limit: Arc<Semaphore>,
    handshake_timeout: Duration,
    mut shutdown: watch::Receiver<()>,
) -> anyhow::Result<()> {
    let bind = endpoint.local_addr().context("获取 QUIC 本地地址失败")?;
    info!(%bind, "quic listener ready");

    let policy = TargetPolicy {
        default_target: listener.default_target.clone(),
        allowed: listener.allowed.clone(),
    };
    let mut tasks: JoinSet<()> = JoinSet::new();
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            incoming = endpoint.accept() => match incoming {
                Some(incoming) => match incoming.accept() {
                    Ok(connecting) => match connecting.await {
                        Ok(conn) => {
                            tasks.spawn(handle_quic_conn(
                                conn,
                                auth.clone(),
                                policy.clone(),
                                conn_limit.clone(),
                                handshake_timeout,
                            ));
                        }
                        Err(e) => warn!(error = %e, "QUIC 连接握手失败"),
                    },
                    Err(e) => warn!(error = %e, "QUIC 入连接受阻"),
                },
                // endpoint 未 close 时 accept 不会返回 None；到这儿说明 UDP driver 意外退出
                None => bail!("QUIC endpoint 已停止接受连接"),
            },
        }
    }
    endpoint.close(quinn::VarInt::from(0u32), b"relay shutdown");
    // 与 tcp 监听器同语义：shutdown 即终止在途会话（中继重启本就断会话）
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}

/// 单条 QUIC 连接：循环接受双向流，每条流独立鉴权、独立选路。
/// 连接内并发同样受 conn_limit 约束（try_acquire，不阻塞连接循环）。
async fn handle_quic_conn(
    conn: quinn::Connection,
    auth: Arc<ServerAuth>,
    policy: TargetPolicy,
    conn_limit: Arc<Semaphore>,
    handshake_timeout: Duration,
) {
    let peer = conn.remote_address().to_string();
    let mut streams: JoinSet<()> = JoinSet::new();
    loop {
        match conn.accept_bi().await {
            Ok((mut send, mut recv)) => {
                // try_acquire：流洪峰也不能阻塞连接循环（与 tcp 监听器同策略）
                let permit = match conn_limit.clone().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        warn!(%peer, "并发上限已满，拒绝新流");
                        send.reset(0u32.into()).ok();
                        let _ = recv.stop(0u32.into());
                        continue;
                    }
                };
                streams.spawn({
                    let peer = peer.clone();
                    let auth = auth.clone();
                    let policy = policy.clone();
                    async move {
                        // permit 持有到会话结束
                        let _permit = permit;
                        serve_stream(
                            BiStream::new(send, recv),
                            &peer,
                            &auth,
                            &policy,
                            handshake_timeout,
                        )
                        .await;
                    }
                });
            }
            Err(e) => {
                info!(%peer, error = %e, "QUIC 连接结束");
                break;
            }
        }
        // 惰性回收已结束的流任务，避免长连接上任务句柄累积
        while streams.try_join_next().is_some() {}
    }
    streams.abort_all();
    while streams.join_next().await.is_some() {}
}

/// QUIC 双向流适配器：把 quinn 的 (SendStream, RecvStream) 拼成单个
/// AsyncRead + AsyncWrite 对象，喂给传输无关的 [`serve_stream`]。
/// `poll_shutdown` = `SendStream::finish`（半关闭：已发数据送达后向对端发 FIN）。
pub struct BiStream {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}

impl BiStream {
    pub fn new(send: quinn::SendStream, recv: quinn::RecvStream) -> Self {
        Self { send, recv }
    }
}

impl AsyncRead for BiStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // 显式经 trait 调用：绕过 quinn RecvStream 的同名固有方法（签名不同）
        AsyncRead::poll_read(Pin::new(&mut self.recv), cx, buf)
    }
}

impl AsyncWrite for BiStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        // 同上：绕过 SendStream 固有的 poll_write（其错误类型是 WriteError 非 io::Error）
        AsyncWrite::poll_write(Pin::new(&mut self.send), cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.send), cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.send), cx)
    }
}

/// TLS 服务端配置：证书来源二选一（配置 PEM / 启动自签），统一挂 ALPN。
fn build_server_tls(listener: &ResolvedListener) -> anyhow::Result<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let builder = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .context("协商 TLS 协议版本失败")?
        .with_no_client_auth();

    let mut tls = match (&listener.tls_cert, &listener.tls_key) {
        (Some(cert_path), Some(key_path)) => builder
            .with_single_cert(load_certs(cert_path)?, load_key(key_path)?)
            .context("配置的 TLS 证书/私钥装载失败")?,
        (None, None) => {
            let (cert, key) = self_signed_ed25519()?;
            let fingerprint = fingerprint_hex(&sha256(cert.as_ref()));
            info!(
                fingerprint,
                "quic listener 未配置证书，使用启动自签（Ed25519）；客户端可 pin 此 SHA-256 指纹"
            );
            builder
                .with_single_cert(vec![cert], key)
                .context("自签证书装载失败")?
        }
        // config.rs 已校验 cert/key 成对，这里防御不对称配置
        _ => bail!("quic 监听器 cert 与 key 必须同时配置"),
    };
    tls.alpn_protocols = vec![ALPN_RELAY.to_vec()];
    Ok(tls)
}

/// 从 PEM 文件加载证书链。
fn load_certs(path: &Path) -> anyhow::Result<Vec<CertificateDer<'static>>> {
    let file =
        File::open(path).with_context(|| format!("打开证书文件 {} 失败", path.display()))?;
    let certs: Result<Vec<_>, _> = rustls_pemfile::certs(&mut BufReader::new(file)).collect();
    let certs =
        certs.with_context(|| format!("解析证书文件 {} 失败", path.display()))?;
    if certs.is_empty() {
        bail!("证书文件 {} 中未找到 PEM 证书", path.display());
    }
    Ok(certs)
}

/// 从 PEM 文件加载私钥（自动识别 PKCS#8 / RSA / SEC1）。
fn load_key(path: &Path) -> anyhow::Result<PrivateKeyDer<'static>> {
    let file =
        File::open(path).with_context(|| format!("打开私钥文件 {} 失败", path.display()))?;
    rustls_pemfile::private_key(&mut BufReader::new(file))
        .with_context(|| format!("解析私钥文件 {} 失败", path.display()))?
        .ok_or_else(|| anyhow::anyhow!("私钥文件 {} 中未找到 PEM 私钥", path.display()))
}

/// 生成 Ed25519 自签证书（SAN=localhost，仅作传输加密 + 客户端 pin 指纹用）。
fn self_signed_ed25519() -> anyhow::Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
        .context("生成 Ed25519 密钥对失败")?;
    let params = rcgen::CertificateParams::new(vec!["localhost".to_string()])
        .context("构造自签证书参数失败")?;
    let cert = params.self_signed(&key_pair).context("自签证书签发失败")?;
    Ok((
        cert.der().clone(),
        PrivateKeyDer::Pkcs8(key_pair.serialize_der().into()),
    ))
}

/// SHA-256 指纹的展示形态：冒号分隔的大写十六进制（对齐 `openssl x509 -fingerprint`）。
fn fingerprint_hex(digest: &[u8; 32]) -> String {
    digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// 内置 SHA-256（仅用于自签证书指纹日志）。proto 的 sha2 未在本 crate 直接依赖，
/// 为不扩 manifest 而自带最小实现；标准测试向量锁定正确性（见 tests）。
fn sha256(msg: &[u8]) -> [u8; 32] {
    #[rustfmt::skip]
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    // 补位：0x80 + 零填充到 56 (mod 64)，再附 64 位大端比特长度
    let bit_len = (msg.len() as u64).wrapping_mul(8);
    let mut data = msg.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());

    let mut w = [0u32; 64];
    let (chunks, _) = data.as_chunks::<64>();
    for chunk in chunks {
        for (i, word) in w.iter_mut().enumerate().take(16) {
            *word = u32::from_be_bytes(chunk[4 * i..4 * i + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::ListenerProto;
    use veloterm_relay_proto::frame::TargetAddr;

    fn quic_listener(
        tls_cert: Option<std::path::PathBuf>,
        tls_key: Option<std::path::PathBuf>,
    ) -> ResolvedListener {
        ResolvedListener {
            proto: ListenerProto::Quic,
            bind: "127.0.0.1:0".parse().unwrap(),
            default_target: Some(TargetAddr { host: "127.0.0.1".into(), port: 22 }),
            allowed: vec![],
            udp_idle_timeout: Duration::from_secs(60),
            tls_cert,
            tls_key,
        }
    }

    #[test]
    fn sha256_matches_standard_vectors() {
        let hex = |d: &[u8; 32]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // 56 字节消息：跨进第二个分组，覆盖多分组与补位路径
        assert_eq!(
            hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn fingerprint_hex_is_colon_separated_uppercase() {
        assert_eq!(
            fingerprint_hex(&sha256(b"abc")),
            "BA:78:16:BF:8F:01:CF:EA:41:41:40:DE:5D:AE:22:23:B0:03:61:A3:96:17:7A:9C:B4:10:FF:61:F2:00:15:AD"
        );
    }

    #[tokio::test]
    async fn self_signed_endpoint_binds_ephemeral_port() {
        let endpoint = build_quic_endpoint(&quic_listener(None, None)).unwrap();
        let local = endpoint.local_addr().unwrap();
        assert_eq!(local.ip().to_string(), "127.0.0.1");
        assert_ne!(local.port(), 0);
    }

    #[tokio::test]
    async fn endpoint_loads_configured_pem_pair() {
        // 覆盖 tls_cert/tls_key 的 PEM 加载路径（rcgen 生成 → 落盘 → 加载建端点）
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
        let params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
        let cert = params.self_signed(&key_pair).unwrap();

        let dir = std::env::temp_dir().join(format!("veloterm-relay-quic-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cert.pem"), cert.pem()).unwrap();
        std::fs::write(dir.join("key.pem"), key_pair.serialize_pem()).unwrap();

        let listener = quic_listener(Some(dir.join("cert.pem")), Some(dir.join("key.pem")));
        let endpoint = build_quic_endpoint(&listener);
        let _ = std::fs::remove_dir_all(&dir);
        let local = endpoint.unwrap().local_addr().unwrap();
        assert_ne!(local.port(), 0);
    }
}
