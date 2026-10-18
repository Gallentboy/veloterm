//! SSH 经 veloterm_relay 中继的传输拨号：按 `RelayTransport` 建立 tcp/ws/quic
//! 通道，完成 VLR2 客户端握手（双 nonce 挑战响应），返回纯字节流供
//! `russh::client::connect_stream` 使用。握手后的流与直连 TCP 无差别。

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::stream::StreamExt;
use futures_util::SinkExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use veloterm_relay_proto::auth::Secret;
use veloterm_relay_proto::frame::TargetAddr;
use veloterm_relay_proto::handshake::client_handshake;

use super::common::{RelayTransport, SshRelayEndpoint};

/// 握手后的中继字节流（SSH 直接跑在上面）。
pub type RelayStream = Box<dyn AsyncReadWrite>;

pub trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const QUIC_ALPN: &[u8] = b"veloterm-relay";

/// 经中继建立到 `ssh_endpoint` 的已握手流。
/// 目标选择：`relay.target` 显式指定（需在中继 allow_targets 白名单内）；
/// 不指定则客户端不带 target，由中继按其默认目标路由（服务端 target 配置）。
pub async fn dial_relay(
    relay: &SshRelayEndpoint,
    ssh_endpoint: &TargetAddr,
) -> Result<RelayStream, String> {
    let secret = Secret::from_hex(relay.secret.trim()).map_err(|e| {
        format!("中继 secret 非法（{e}）：需 veloterm_relay token generate 生成的 64 位十六进制")
    })?;
    let target: Option<TargetAddr> = match &relay.target {
        Some(t) => Some(TargetAddr::parse(t).map_err(|e| format!("中继 target \"{t}\" 非法: {e}"))?),
        None => None,
    };
    // ssh_endpoint 仅用于日志与校验链首存在性；显式 target 缺省时不发送
    let _ = ssh_endpoint;

    match relay.transport {
        RelayTransport::Tcp => {
            let mut stream = TcpStream::connect((relay.host.as_str(), relay.port))
                .await
                .map_err(|e| format!("连接中继 {}:{} 失败: {e}", relay.host, relay.port))?;
            client_handshake(&mut stream, relay.key_id, &secret, target.as_ref(), HANDSHAKE_TIMEOUT)
                .await
                .map_err(|e| format!("中继握手失败（{}:{}）: {e}", relay.host, relay.port))?;
            Ok(Box::new(stream))
        }
        RelayTransport::Ws => {
            let url = format!("ws://{}:{}/", relay.host, relay.port);
            let (ws, _resp) = tokio_tungstenite::connect_async(url)
                .await
                .map_err(|e| format!("WebSocket 连接中继 {}:{} 失败: {e}", relay.host, relay.port))?;
            let mut io = WsIo::new(ws);
            client_handshake(&mut io, relay.key_id, &secret, target.as_ref(), HANDSHAKE_TIMEOUT)
                .await
                .map_err(|e| format!("中继握手失败（{}:{}）: {e}", relay.host, relay.port))?;
            Ok(Box::new(io))
        }
        RelayTransport::Quic => {
            let (conn, endpoint) = quic_connect(&relay.host, relay.port)
                .await
                .map_err(|e| format!("QUIC 连接中继 {}:{} 失败: {e}", relay.host, relay.port))?;
            let (send, recv) = conn
                .open_bi()
                .await
                .map_err(|e| format!("QUIC 打开双向流失败: {e}"))?;
            // conn 与 endpoint 比流活得久（字段持有），否则流立即失效
            let mut io = BiIo { send, recv, _conn: conn, _endpoint: endpoint };
            client_handshake(&mut io, relay.key_id, &secret, target.as_ref(), HANDSHAKE_TIMEOUT)
                .await
                .map_err(|e| format!("中继握手失败（{}:{}）: {e}", relay.host, relay.port))?;
            Ok(Box::new(io))
        }
    }
}

/// quinn 客户端连接。跳过证书校验是有意设计：中继证书通常自签，且 SSH 层
/// 有自己的端到端主机密钥校验，QUIC 在此仅作传输加密。
async fn quic_connect(host: &str, port: u16) -> Result<(quinn::Connection, quinn::Endpoint), String> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut client_config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("rustls 协议版本协商失败: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerify))
        .with_no_client_auth();
    client_config.alpn_protocols = vec![QUIC_ALPN.to_vec()];

    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(Duration::from_secs(5)));
    let quic_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(client_config)
        .map_err(|e| format!("QUIC 加密套件协商失败: {e}"))?;
    let mut qconfig = quinn::ClientConfig::new(Arc::new(quic_crypto));
    qconfig.transport_config(Arc::new(transport));

    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse().map_err(|e| format!("本地 QUIC 端口绑定失败: {e}"))?)
        .map_err(|e| format!("本地 QUIC endpoint 创建失败: {e}"))?;
    endpoint.set_default_client_config(qconfig);

    let remote = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("解析中继地址 {host}:{port} 失败: {e}"))?
        .next()
        .ok_or_else(|| format!("解析中继地址 {host}:{port} 失败：无结果"))?;
    let conn = endpoint
        .connect(remote, "veloterm-relay")
        .map_err(|e| format!("QUIC 连接发起失败: {e}"))?
        .await
        .map_err(|e| format!("QUIC 握手失败: {e}"))?;
    Ok((conn, endpoint))
}

#[derive(Debug)]
struct NoVerify;

impl rustls::client::danger::ServerCertVerifier for NoVerify {
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
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
        ]
    }
}

/// QUIC 双向流组合成单一流；连接与 endpoint 由字段持有保活（下划线字段
/// 仅为存活语义，不读取）。
struct BiIo {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    _conn: quinn::Connection,
    _endpoint: quinn::Endpoint,
}

impl AsyncRead for BiIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // 显式走 tokio 的 AsyncRead：quinn 流同时实现了 futures 版（错误类型不同）
        tokio::io::AsyncRead::poll_read(Pin::new(&mut self.recv), cx, buf)
    }
}

impl AsyncWrite for BiIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        tokio::io::AsyncWrite::poll_write(Pin::new(&mut self.send), cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        tokio::io::AsyncWrite::poll_flush(Pin::new(&mut self.send), cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let _ = tokio::io::AsyncWrite::poll_flush(Pin::new(&mut self.send), cx);
        Pin::new(&mut self.send)
            .finish()
            .map(|_| Poll::Ready(Ok(())))
            .unwrap_or_else(|_| Poll::Ready(Err(std::io::Error::other("quinn finish 失败"))))
    }
}

/// WebSocket 字节流适配器：binary 消息载荷拼接为字节流；写侧缓冲到 flush 时
/// 作为一条 binary 消息发出。text 等非 binary 帧按协议错误处理（协议规定只有
/// binary 载荷；ping/pong 由 tungstenite 自动处理，不会出现在 poll_next）。
struct WsIo<S> {
    ws: tokio_tungstenite::WebSocketStream<S>,
    rbuf: Vec<u8>,
    wbuf: Vec<u8>,
    eof: bool,
}

impl<S> WsIo<S> {
    fn new(ws: tokio_tungstenite::WebSocketStream<S>) -> Self {
        Self { ws, rbuf: Vec::new(), wbuf: Vec::new(), eof: false }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for WsIo<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        loop {
            if !self.rbuf.is_empty() {
                let n = buf.remaining().min(self.rbuf.len());
                let chunk: Vec<u8> = self.rbuf.drain(..n).collect();
                buf.put_slice(&chunk);
                return Poll::Ready(Ok(()));
            }
            if self.eof {
                return Poll::Ready(Ok(()));
            }
            match self.ws.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok(msg))) => match msg {
                    tokio_tungstenite::tungstenite::Message::Binary(data) => {
                        self.rbuf = data.to_vec();
                    }
                    tokio_tungstenite::tungstenite::Message::Close(_) => {
                        self.eof = true;
                    }
                    other => {
                        return Poll::Ready(Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("ws 中继收到非 binary 帧: {other:?}"),
                        )));
                    }
                },
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("ws 中继流错误: {e}"),
                    )));
                }
                Poll::Ready(None) => {
                    self.eof = true;
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for WsIo<S> {
    fn poll_write(mut self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        self.wbuf.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if !self.wbuf.is_empty() {
            match self.ws.poll_ready_unpin(cx) {
                Poll::Ready(Ok(())) => {
                    let payload = std::mem::take(&mut self.wbuf);
                    if let Err(e) = self
                        .ws
                        .start_send_unpin(tokio_tungstenite::tungstenite::Message::Binary(payload.into()))
                    {
                        return Poll::Ready(Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("ws 中继发送失败: {e}"),
                        )));
                    }
                }
                Poll::Ready(Err(e)) => {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("ws 中继发送就绪失败: {e}"),
                    )));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        self.ws
            .poll_flush_unpin(cx)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("ws flush 失败: {e}")))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        // 先把缓冲数据冲出去，再发送 Close 帧（tungstenite 的 close 是 async，
        // 这里用 Sink 的 poll 语义等价完成）
        match self.as_mut().poll_flush(cx) {
            Poll::Ready(Ok(())) => {}
            other => return other,
        }
        match self.ws.poll_ready_unpin(cx) {
            Poll::Ready(Ok(())) => {
                let _ = self.ws.start_send_unpin(tokio_tungstenite::tungstenite::Message::Close(None));
                self.ws.poll_flush_unpin(cx).map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("ws 关闭失败: {e}"),
                    )
                })
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("ws 关闭就绪失败: {e}"),
            ))),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapters_implement_duplex_traits() {
        fn assert_impls<T: AsyncRead + AsyncWrite + Unpin>() {}
        assert_impls::<WsIo<TcpStream>>();
        assert_impls::<BiIo>();
    }
}
