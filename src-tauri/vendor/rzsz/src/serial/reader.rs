use std::io::{self, Read};

/// VeloTerm vendor 修改：原实现经 nix::poll 对 fd 做读超时（unix-only）。
/// 嵌入形态下内层不是 fd 而是「异步通道桥」，故把「带超时的读/就绪探测」
/// 抽象为 trait 交给内层实现（桥接层用 recv_timeout 实现，全平台可携），
/// ModemReader 自身只保留缓冲、单字节回退（unread）与 purge 语义。
pub trait TimeoutRead: Read {
    /// 带超时的读：超时返回 io::ErrorKind::TimedOut，连接关闭返回
    /// UnexpectedEof（与上游 poll+read 行为对齐）
    fn read_timeout(&mut self, buf: &mut [u8], timeout_tenths: u32) -> io::Result<usize>;

    /// 就绪探测（上游 rdchk()/data_available 语义；纯查询不消费）
    fn ready(&mut self) -> bool;
}

/// Buffered reader for modem I/O with timeout-based reads.
pub struct ModemReader<R: TimeoutRead> {
    inner: R,
    buffer: Vec<u8>,
    pos: usize,
    len: usize,
    pushback: Option<u8>,
}

impl<R: TimeoutRead> ModemReader<R> {
    pub fn new(inner: R, buffer_size: usize) -> Self {
        Self {
            inner,
            buffer: vec![0u8; buffer_size],
            pos: 0,
            len: 0,
            pushback: None,
        }
    }

    /// Push a byte back into the reader (1-byte lookahead).
    pub fn unread_byte(&mut self, b: u8) {
        self.pushback = Some(b);
    }

    /// Read a single byte with timeout (in tenths of seconds).
    pub fn read_byte(&mut self, timeout_tenths: u32) -> io::Result<u8> {
        if let Some(b) = self.pushback.take() {
            return Ok(b);
        }
        if self.pos < self.len {
            let b = self.buffer[self.pos];
            self.pos += 1;
            return Ok(b);
        }

        // Buffer exhausted — refill with timeout via the inner reader
        let bytes_read = self.inner.read_timeout(&mut self.buffer, timeout_tenths)?;
        if bytes_read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed",
            ));
        }

        self.pos = 1;
        self.len = bytes_read;
        Ok(self.buffer[0])
    }

    /// Non-blocking check for pending input. Equivalent to rdchk() in lsz.c.
    pub fn data_available(&mut self) -> bool {
        if self.pushback.is_some() || self.pos < self.len {
            return true;
        }
        self.inner.ready()
    }

    /// Discard any buffered data.
    pub fn purge(&mut self) {
        self.pos = 0;
        self.len = 0;
    }
}
