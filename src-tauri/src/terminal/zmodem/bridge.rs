//! ZMODEM 传输桥接：把 rzsz 引擎（阻塞、泛型 Read/Write）接到异步 SSH 数据流。
//!
//! 数据通路（全程在 Rust，不经前端）：
//! ```text
//! SSH ChannelMsg::Data ──► ZmodemBridge::feed ──► std mpsc ──► 引擎 Reader
//! 引擎 Writer ──► std mpsc ──► 异步写回 SSH 通道
//! ```
//! 引擎跑在 `spawn_blocking` 线程（阻塞读等待桥接数据）；宿主侧由
//! `ZmodemBridge` 维护每会话状态与进度事件。

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::mpsc as tokio_mpsc;

use rzsz::serial::reader::TimeoutRead;

/// 引擎 → 通道：写回服务器的协议字节
pub type OutboundTx = std_mpsc::Sender<Vec<u8>>;
/// 通道 → 引擎：服务器来的协议字节
pub type InboundRx = std_mpsc::Receiver<Vec<u8>>;

/// 进度事件（前端只消费这个，不接触协议字节）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZmodemProgress {
    pub session_id: String,
    /// "download"（sz，服务器→本地） | "upload"（rz，本地→服务器）
    pub direction: &'static str,
    pub file_name: String,
    pub transferred: u64,
    pub total: u64,
    /// 即时速度（字节/秒，后端按事件间隔差分计算）
    pub speed_bps: u64,
    /// 传输耗时（毫秒；仅 is_finished=true 时有意义，0 = 未提供）
    pub elapsed_ms: u64,
    pub is_finished: bool,
    /// 失败/取消时的说明（None = 正常）
    pub error: Option<String>,
}

/// 引擎侧 Reader：把桥接字节流包装成 TimeoutRead（recv_timeout 实现，全平台）
pub struct BridgeReader {
    rx: InboundRx,
    /// 当前 chunk 内未消费的剩余字节
    pending: Vec<u8>,
    pos: usize,
}

impl BridgeReader {
    pub fn new(rx: InboundRx) -> Self {
        Self { rx, pending: Vec::new(), pos: 0 }
    }
}

impl Read for BridgeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // 先消费当前 chunk 剩余
        if self.pos < self.pending.len() {
            let n = (self.pending.len() - self.pos).min(buf.len());
            if let (Some(dst), Some(src)) = (
                buf.get_mut(..n),
                self.pending.get(self.pos..self.pos + n),
            ) {
                dst.copy_from_slice(src);
            }
            self.pos += n;
            if self.pos >= self.pending.len() {
                self.pending.clear();
                self.pos = 0;
            }
            return Ok(n);
        }
        match self.rx.recv() {
            Ok(chunk) => {
                self.pending = chunk;
                self.pos = 0;
                self.read(buf)
            }
            Err(_) => Ok(0), // 桥关闭 = EOF
        }
    }
}

impl TimeoutRead for BridgeReader {
    fn read_timeout(&mut self, buf: &mut [u8], timeout_tenths: u32) -> io::Result<usize> {
        if self.pos < self.pending.len() {
            return self.read(buf);
        }
        let timeout = std::time::Duration::from_millis(
            (timeout_tenths as u64).saturating_mul(100).max(100),
        );
        match self.rx.recv_timeout(timeout) {
            Ok(chunk) => {
                self.pending = chunk;
                self.pos = 0;
                self.read(buf)
            }
            Err(std_mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "read timeout",
            )),
            Err(std_mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed",
            )),
        }
    }

    fn ready(&mut self) -> bool {
        self.pos < self.pending.len() || !matches!(self.rx.try_recv(), Err(std_mpsc::TryRecvError::Empty))
    }
}

/// 引擎侧 Writer：协议字节 → 异步通道写回
pub struct BridgeWriter {
    tx: tokio_mpsc::UnboundedSender<Vec<u8>>,
}

impl BridgeWriter {
    pub fn new(tx: tokio_mpsc::UnboundedSender<Vec<u8>>) -> Self {
        Self { tx }
    }
}

impl Write for BridgeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.tx
            .send(buf.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "channel closed"))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// 传输任务句柄：宿主侧持有，负责喂数据与取消（可 Clone：宿主保存一份、
/// 传输任务持一份，共享同一取消/结束标志与入向通道）
#[derive(Clone)]
pub struct ZmodemBridge {
    pub session_id: String,
    pub direction: &'static str,
    inbound_tx: std_mpsc::Sender<Vec<u8>>,
    cancelled: Arc<AtomicBool>,
    /// 引擎结束（成功/失败/取消）置位：宿主据此恢复终端透传
    finished: Arc<AtomicBool>,
    /// 引擎线程存活标志：线程真正退出时置 false（早于 finished 事件，
    /// 宿主据此判定「这块字节引擎已经收不到了」→ 回退透传）
    engine_alive: Arc<AtomicBool>,
}

impl ZmodemBridge {
    /// 新建桥（引擎侧收发端由调用方取用）
    pub fn new(session_id: String, direction: &'static str) -> (Self, InboundRx) {
        let (inbound_tx, inbound_rx) = std_mpsc::channel();
        (
            Self {
                session_id,
                direction,
                inbound_tx,
                cancelled: Arc::new(AtomicBool::new(false)),
                finished: Arc::new(AtomicBool::new(false)),
                engine_alive: Arc::new(AtomicBool::new(true)),
            },
            inbound_rx,
        )
    }

    /// 把 SSH 收到的字节喂给引擎（探测命中后的所有字节都走这里）
    /// 喂数据给引擎。返回 false = 引擎已结束（接收端已 drop），
    /// 调用方应把本块字节回落到终端透传（否则服务器随后输出的
    /// shell 提示符会被吞掉，表现为「传输完成后终端不回车」）。
    pub fn feed(&self, data: &[u8]) -> bool {
        // 引擎线程已退出 → 字节无处可去，调用方必须透传（否则吞掉提示符）
        if !self.engine_alive.load(Ordering::Relaxed) {
            return false;
        }
        self.inbound_tx.send(data.to_vec()).is_ok()
    }

    /// 引擎存活标志（引擎线程退出前必须置 false）
    pub fn engine_alive_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.engine_alive)
    }

    /// 取消检查闭包（交给 Session::is_cancelled）
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    /// 结束标志（传输任务在引擎退出后置位；宿主轮询此值恢复透传）
    #[inline(always)]
    pub fn finished_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.finished)
    }

    /// 请求取消（用户点取消 / 会话关闭）：
    /// 置取消位（引擎下一检查点退出）+ 立即置 finished（宿主恢复透传，
    /// 不等引擎线程），并由调用方通过 `send_abort` 向服务器发中止序列。
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.finished.store(true, Ordering::Relaxed);
    }

    /// 向服务器发标准 ZMODEM 中止序列（CAN×10 + BS×10）：
    /// 让远端 lrz/lsz 立即退出，避免它继续等待把终端卡住
    pub fn send_abort(&self, tx: &tokio::sync::mpsc::UnboundedSender<Vec<u8>>) {
        let mut out = Vec::new();
        let mut enc = rzsz::zmodem::frame::FrameEncoder::new();
        if enc.send_cancel(&mut out).is_ok() {
            let _ = tx.send(out);
        }
    }

    /// 关闭入向通道，让引擎的阻塞读以 EOF 返回
    pub fn close_inbound(&self) {
        // 用一个空 channel 替换不可行（字段不可变）；通过取消标志 + 让
        // 调用方 drop 自身来关闭。这里只置取消位，引擎下一检查点即退出。
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_reader_reassembles_chunks() {
        let (tx, rx) = std_mpsc::channel::<Vec<u8>>();
        tx.send(b"hello ".to_vec()).unwrap();
        tx.send(b"world".to_vec()).unwrap();
        drop(tx);
        let mut r = BridgeReader::new(rx);
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"hello world");
    }

    #[test]
    fn bridge_reader_timeout_and_eof() {
        let (_tx, rx) = std_mpsc::channel::<Vec<u8>>();
        let mut r = BridgeReader::new(rx);
        let mut buf = [0u8; 4];
        let err = r.read_timeout(&mut buf, 1).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        drop(_tx);
        let err = r.read_timeout(&mut buf, 1).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn bridge_writer_forwards_bytes() {
        let (tx, mut rx) = tokio_mpsc::unbounded_channel::<Vec<u8>>();
        let mut w = BridgeWriter::new(tx);
        w.write_all(b"proto").unwrap();
        assert_eq!(rx.try_recv().unwrap(), b"proto");
    }

    #[test]
    fn cancel_flag_is_shared() {
        let (bridge, _rx) = ZmodemBridge::new("s1".into(), "download");
        let flag = bridge.cancel_flag();
        assert!(!flag.load(Ordering::Relaxed));
        bridge.cancel();
        assert!(flag.load(Ordering::Relaxed));
    }
}
