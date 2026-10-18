//! WebSocket 监听器：TCP accept → HTTP 升级（`accept_async`）→ [`WsByteStream`]
//! 字节流适配 → [`serve_stream`](crate::server::serve_stream)。
//! 会话逻辑（VLR2 握手/目标连接/双向搬运/日志）与 tcp 监听器完全同源，
//! 本文件只负责传输层：升级、适配、并发许可与 shutdown。

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{ready, Context, Poll};
use std::time::Duration;

use futures_util::{Sink, Stream};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, Semaphore};
use tokio::task::JoinSet;
use tokio_tungstenite::tungstenite::{Bytes, Error as WsError, Message};
use tracing::{info, warn};
use veloterm_relay_proto::handshake::{ServerAuth, TargetPolicy};

use crate::config::ResolvedListener;
use crate::server::serve_stream;

/// 把 `WebSocketStream<S>` 适配成 `AsyncRead + AsyncWrite` 字节流，
/// 服务端会话与测试客户端（connect_async 后同样包一层）共用。
///
/// 字节流语义约定：
/// - **写侧**：`poll_write` 只缓冲，`poll_flush` 时把整段缓冲作为**一条**
///   binary 消息发出——字节流 = 逐条 binary 消息载荷首尾拼接。上游
///   （握手、copy_bidirectional、russh）都遵循「写一批 → flush」纪律，
///   缓冲峰值即单个写块，不会无界堆积。
/// - **读侧**：逐条接收 binary 消息入内部缓冲后按需拷出。
/// - **text 帧按连接错误处理**：VLR2/SSH 全是二进制，收到 text 说明对端
///   不是 veloterm 客户端（被浏览器/其他 ws 工具误连），报错断开。
/// - **Close 帧按字节流 EOF 处理**：ws 的干净收尾就是字节流的干净收尾，
///   让 copy_bidirectional 走正常的单向关闭流程。
/// - **ping/pong 由 tungstenite 自动应答**，适配层只跳过这两类控制帧。
///
/// 注意：tungstenite 自身给 `WebSocketStream` 实现的 AsyncRead/AsyncWrite
/// 是**帧级**语义（读写 ws 帧而非消息载荷），不能当字节流用，故有本适配器。
pub struct WsByteStream<S> {
    ws: tokio_tungstenite::WebSocketStream<S>,
    /// 已收未读的 binary 载荷
    rx_buf: Bytes,
    /// 已写未发（下一次 poll_flush 作为一条 binary 消息发出）
    tx_buf: Vec<u8>,
    /// 对端已 Close 或底层流终止：poll_read 恒返回 EOF
    eof: bool,
    /// 本端已完成 ws 关闭（poll_shutdown 之后不可再写）
    closed: bool,
}

impl<S> WsByteStream<S> {
    pub fn new(ws: tokio_tungstenite::WebSocketStream<S>) -> Self {
        Self {
            ws,
            rx_buf: Bytes::new(),
            tx_buf: Vec::new(),
            eof: false,
            closed: false,
        }
    }
}

impl<S> AsyncRead for WsByteStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if buf.remaining() == 0 {
                return Poll::Ready(Ok(()));
            }
            if !this.rx_buf.is_empty() {
                let n = buf.remaining().min(this.rx_buf.len());
                let chunk = this.rx_buf.split_to(n);
                buf.put_slice(&chunk);
                return Poll::Ready(Ok(()));
            }
            if this.eof {
                return Poll::Ready(Ok(()));
            }
            match Pin::new(&mut this.ws).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => this.eof = true, // 底层流终止：EOF
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(ws_err_to_io(e))),
                Poll::Ready(Some(Ok(msg))) => match msg {
                    Message::Binary(data) => this.rx_buf = data,
                    Message::Ping(_) | Message::Pong(_) => {} // 自动应答，跳过
                    Message::Close(_) => this.eof = true, // 干净收尾 → EOF
                    Message::Text(_) => {
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "ws 字节流只接受 binary 消息，收到 text 帧",
                        )));
                    }
                    // read 路径不会产出原始帧（上游文档明言），防御性跳过
                    Message::Frame(_) => {}
                },
            }
        }
    }
}

impl<S> AsyncWrite for WsByteStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.closed {
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, "ws 已关闭")));
        }
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        this.tx_buf.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.closed {
            return Poll::Ready(Ok(()));
        }
        if !this.tx_buf.is_empty() {
            match Pin::new(&mut this.ws).poll_ready(cx) {
                Poll::Ready(Ok(())) => {
                    let payload = std::mem::take(&mut this.tx_buf);
                    if let Err(e) =
                        Pin::new(&mut this.ws).start_send(Message::Binary(payload.into()))
                    {
                        return Poll::Ready(Err(ws_err_to_io(e)));
                    }
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(ws_err_to_io(e))),
                Poll::Pending => return Poll::Pending,
            }
        }
        Pin::new(&mut this.ws).poll_flush(cx).map_err(ws_err_to_io)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // 先把残余缓冲作为 binary 消息冲干净，再走 ws 关闭握手（发 Close 帧）
        ready!(self.as_mut().poll_flush(cx))?;
        let this = self.get_mut();
        if this.closed {
            return Poll::Ready(Ok(()));
        }
        match Pin::new(&mut this.ws).poll_close(cx) {
            Poll::Ready(Ok(())) => {
                this.closed = true;
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(ws_err_to_io(e))),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// tungstenite 错误 → io 错误：IO 直通；连接已关按断开语义；
/// 其余（协议/容量/UTF-8 等）归为连接中止。
fn ws_err_to_io(e: WsError) -> io::Error {
    match e {
        WsError::Io(io_err) => io_err,
        WsError::ConnectionClosed => {
            io::Error::new(io::ErrorKind::ConnectionReset, WsError::ConnectionClosed)
        }
        WsError::AlreadyClosed => {
            io::Error::new(io::ErrorKind::NotConnected, WsError::AlreadyClosed)
        }
        other => io::Error::new(io::ErrorKind::ConnectionAborted, other),
    }
}

/// WebSocket 监听器。shutdown 变更即退出并终止在途连接（与 tcp 一致）。
pub async fn run_ws(
    listener: ResolvedListener,
    auth: Arc<ServerAuth>,
    conn_limit: Arc<Semaphore>,
    handshake_timeout: Duration,
    shutdown: watch::Receiver<()>,
) -> anyhow::Result<()> {
    let tcp = TcpListener::bind(listener.bind)
        .await
        .map_err(|e| anyhow::anyhow!("监听 {} 绑定失败: {e}", listener.bind))?;
    run_ws_on(tcp, listener, auth, conn_limit, handshake_timeout, shutdown).await
}

/// 在已绑定的 TcpListener 上服务（测试注入用：绑定时机由调用方掌控，避免竞态）。
pub async fn run_ws_on(
    tcp: TcpListener,
    listener: ResolvedListener,
    auth: Arc<ServerAuth>,
    conn_limit: Arc<Semaphore>,
    handshake_timeout: Duration,
    mut shutdown: watch::Receiver<()>,
) -> anyhow::Result<()> {
    info!(bind = %tcp.local_addr()?, "ws listener ready");

    let policy = TargetPolicy {
        default_target: listener.default_target.clone(),
        allowed: listener.allowed.clone(),
    };
    let mut tasks: JoinSet<()> = JoinSet::new();
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = tcp.accept() => match accepted {
                Ok((stream, peer)) => {
                    // try_acquire：握手洪峰也不能阻塞 accept 循环
                    let permit = match conn_limit.clone().try_acquire_owned() {
                        Ok(p) => p,
                        Err(_) => {
                            warn!(%peer, "并发上限已满，拒绝入连");
                            drop(stream);
                            continue;
                        }
                    };
                    tasks.spawn(handle_ws_conn(
                        stream,
                        peer,
                        auth.clone(),
                        policy.clone(),
                        handshake_timeout,
                        permit,
                    ));
                }
                Err(e) => {
                    warn!(error = %e, "accept 失败");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}

async fn handle_ws_conn(
    stream: TcpStream,
    peer: SocketAddr,
    auth: Arc<ServerAuth>,
    policy: TargetPolicy,
    handshake_timeout: Duration,
    _permit: tokio::sync::OwnedSemaphorePermit,
) {
    let _ = stream.set_nodelay(true);
    // HTTP 升级同样是客户端驱动的握手工作，纳入同一超时预算
    let ws = match tokio::time::timeout(handshake_timeout, tokio_tungstenite::accept_async(stream))
        .await
    {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => {
            warn!(%peer, error = %e, "websocket 升级失败");
            return;
        }
        Err(_) => {
            warn!(%peer, "websocket 升级超时");
            return;
        }
    };
    serve_stream(WsByteStream::new(ws), "ws", &peer.to_string(), &auth, &policy, handshake_timeout)
        .await;
}
