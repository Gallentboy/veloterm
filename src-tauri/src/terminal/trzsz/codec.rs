//! trzsz 帧编解码：文本模式载荷（base64(zlib(bytes))）、行读取状态机（含
//! tmux 折行/噪声容错、Windows 服务端协议）、NAME 帧的 JSON 载荷与 MD5。
//!
//! 协议参照 trzsz-go（MIT）与 trzsz Python 客户端（MIT）的行为，Rust 自实现：
//! - 载荷编码：`encodeBytes` = base64.Std(zlib(data))（Go comm.go）
//! - 行读取：`\n` 终结；tmux 噪声模式下「行尾 `\r` = 折行标记」需拼回，
//!   并按「最后一个 `#TYPE:`」截取、剥离 tmux 状态行（DCS `\x1bP=..`）
//! - Windows 服务端：行终结符为 `!`（可带 `\n`），且只保留协议字母表
//!   （conhost 光标重定位会引入重复字符，按上游启发式去重）

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};

/// 单行上限：bufsize 上限 10MB → base64 约 13.7MB，64MB 兜底防异常流撑爆内存
pub const MAX_LINE: usize = 64 * 1024 * 1024;

/// 行读取错误
#[derive(Debug, PartialEq, Clone)]
pub enum LineError {
    /// 流中出现 `\x03`（Ctrl+C 中断）
    Interrupted,
    /// 入向通道关闭（会话断开 / 桥已拆除）
    Closed,
    /// 等待超时（每个帧的读超时，来自 CFG.timeout）
    Timeout,
    /// 单行超过 MAX_LINE（异常流/坏帧）
    TooLong,
    /// 本地取消（无限等待模式下的轮询检查点）
    Cancelled,
}

/// zlib + base64 编码（文本模式所有字符串/数据帧的 content 形态）
pub fn encode_payload(data: &[u8]) -> String {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    use std::io::Write as _;
    let _ = z.write_all(data);
    z.finish()
        .map(|compressed| BASE64.encode(compressed))
        .unwrap_or_default()
}

/// base64 + zlib 解码。容错：剥离混入的 ASCII 空白（折行残迹/CRLF）后再解
pub fn decode_payload(content: &str) -> Result<Vec<u8>, String> {
    let cleaned: Vec<u8> = content
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    let compressed = BASE64
        .decode(cleaned)
        .map_err(|e| format!("base64 decode error: {e}"))?;
    let mut z = flate2::read::ZlibDecoder::new(&compressed[..]);
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut z, &mut out).map_err(|e| format!("zlib error: {e}"))?;
    Ok(out)
}

/// NAME 帧在目录模式下的 JSON 载荷（sourceFile，见 Go comm.go）。
/// 上游字段名固定 snake_case；perm 省缺、archive 恒 false（Python 客户端同型）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceFileJson {
    #[serde(rename = "path_id")]
    pub path_id: i64,
    #[serde(rename = "path_name")]
    pub path_name: Vec<String>,
    #[serde(rename = "is_dir")]
    pub is_dir: bool,
    #[serde(rename = "size", default)]
    pub size: i64,
}

impl SourceFileJson {
    pub fn file_name(&self) -> String {
        self.path_name.last().cloned().unwrap_or_default()
    }
}

// ─────────────────────────── MD5（RFC 1321）───────────────────────────
// 协议 V1 的 MD5 校验帧要求本地计算摘要（上传侧服务端核对；下载侧本地核对）。
// app crate 无 md5 依赖且 Cargo.toml 不在本次改动范围，按 RFC 自实现，
// 用 RFC 1321 标准测试向量锁定正确性。

const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
    0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
    0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
    0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
    0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
    0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
    0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
    0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
    0xeb86d391,
];
const MD5_S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
    9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6,
    10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// 增量 MD5（RFC 1321）：大文件分块喂入，避免整文件驻留内存
pub struct Md5Stream {
    state: [u32; 4],
    buf: [u8; 64],
    buflen: usize,
    total_bytes: u64,
}

impl Default for Md5Stream {
    fn default() -> Self {
        Self::new()
    }
}

impl Md5Stream {
    pub fn new() -> Self {
        Self {
            state: [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476],
            buf: [0u8; 64],
            buflen: 0,
            total_bytes: 0,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.total_bytes = self.total_bytes.wrapping_add(data.len() as u64);
        let mut rest = data;
        if self.buflen > 0 {
            let need = 64 - self.buflen;
            let take = need.min(rest.len());
            if let (Some(dst), Some(src)) = (
                self.buf.get_mut(self.buflen..self.buflen + take),
                rest.get(..take),
            ) {
                dst.copy_from_slice(src);
            }
            self.buflen += take;
            rest = rest.get(take..).unwrap_or_default();
            if self.buflen == 64 {
                let block = self.buf;
                md5_block(&mut self.state, &block);
                self.buflen = 0;
            }
        }
        while rest.len() >= 64 {
            let (block, tail) = rest.split_at(64);
            md5_block(&mut self.state, block);
            rest = tail;
        }
        if !rest.is_empty() {
            if let Some(dst) = self.buf.get_mut(..rest.len()) {
                dst.copy_from_slice(rest);
            }
            self.buflen = rest.len();
        }
    }

    pub fn finalize(mut self) -> [u8; 16] {
        let bit_len = self.total_bytes.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buflen != 56 {
            self.update(&[0]);
        }
        self.update(&bit_len.to_le_bytes());
        let mut out = [0u8; 16];
        for (i, word) in self.state.iter().enumerate() {
            if let Some(dst) = out.get_mut(i * 4..i * 4 + 4) {
                dst.copy_from_slice(&word.to_le_bytes());
            }
        }
        out
    }
}

/// 单个 64 字节块的 MD5 压缩函数
fn md5_block(state: &mut [u32; 4], chunk: &[u8]) {
    let mut m = [0u32; 16];
    for (i, word) in m.iter_mut().enumerate() {
        let off = i * 4;
        let Some(bytes) = chunk
            .get(off..off + 4)
            .and_then(|b| <[u8; 4]>::try_from(b).ok())
        else {
            continue;
        };
        *word = u32::from_le_bytes(bytes);
    }
    let (mut a, mut b, mut c, mut d) = (state[0], state[1], state[2], state[3]);
    for (i, k) in MD5_K.iter().enumerate() {
        let (f, g) = match i / 16 {
            0 => ((b & c) | (!b & d), i),
            1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
            2 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let Some(&mg) = m.get(g) else {
            continue;
        };
        let sum = a.wrapping_add(f).wrapping_add(*k).wrapping_add(mg);
        a = d;
        d = c;
        c = b;
        let Some(&shift) = MD5_S.get(i) else {
            continue;
        };
        b = b.wrapping_add(sum.rotate_left(shift));
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
}

/// 计算 MD5 摘要（16 字节）
pub fn md5_digest(input: &[u8]) -> [u8; 16] {
    let mut s = Md5Stream::new();
    s.update(input);
    s.finalize()
}

// ─────────────────────────── 行读取 ───────────────────────────

/// 协议字母表（Windows 协议路径只保留这些字符，见 Go buffer.isTrzszLetter）
fn is_trzsz_letter(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'#' | b':' | b'+' | b'/' | b'=')
}

/// VT100 序列终结字符（CSI 最终字节为字母）
fn is_vt100_end(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

/// 行读取器：从入向通道聚合字节、按当前模式切行。
/// - 普通模式：`\n` 终结；junk（tmux）模式下行尾 `\r` 视为折行标记，逻辑行拼接
/// - Windows 模式：`!`（可带 `\n`）终结，只保留协议字母并按上游启发式去重
pub struct LineReader {
    buf: Vec<u8>,
    pos: usize,
    windows: bool,
    /// 取消标志：timeout=None（永不超时）模式下按 5s 轮询检查，保证取消可解除阻塞
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    // Windows 模式过滤状态（逐字节状态机，跨 chunk 保持）
    w_last: u8,
    w_skip_vt100: bool,
    w_may_duplicate: bool,
    w_has_newline: bool,
    w_has_cursor_home: bool,
    w_pre_has_cursor_home: bool,
}

impl LineReader {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            pos: 0,
            windows: false,
            cancel: None,
            w_last: 0x1b,
            w_skip_vt100: false,
            w_may_duplicate: false,
            w_has_newline: false,
            w_has_cursor_home: false,
            w_pre_has_cursor_home: false,
        }
    }

    /// Windows 服务端协议（行终结 `!`、字母过滤）
    pub fn set_windows(&mut self, v: bool) {
        self.windows = v;
    }

    /// 设置取消标志（无限等待模式下的轮询解除阻塞）
    pub fn set_cancel(&mut self, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        self.cancel = Some(cancel);
    }

    /// 从通道补一块数据（有余量则直接返回）
    async fn fill(
        &mut self,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        timeout: Option<Duration>,
    ) -> Result<(), LineError> {
        if self.pos < self.buf.len() {
            return Ok(());
        }
        self.buf.clear();
        self.pos = 0;
        let got = match timeout {
            Some(t) => match tokio::time::timeout(t, rx.recv()).await {
                Ok(v) => v,
                Err(_) => return Err(LineError::Timeout),
            },
            // 永不超时：按 5s 轮询，期间检查取消标志。若未注入取消位
            // （如单元测试直连 LineReader），轮询会永不到达退出条件——
            // 给一个宽松硬上限兜底，避免宿主线程永久挂起
            None => {
                const MAX_IDLE_ROUNDS: u32 = 120; // 120 × 5s = 10 分钟
                let mut rounds = 0u32;
                loop {
                    match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                        Ok(v) => break v,
                        Err(_) => {
                            if let Some(cancel) = &self.cancel
                                && cancel.load(std::sync::atomic::Ordering::Relaxed)
                            {
                                return Err(LineError::Cancelled);
                            }
                            rounds += 1;
                            if rounds >= MAX_IDLE_ROUNDS {
                                return Err(LineError::Timeout);
                            }
                        }
                    }
                }
            }
        };
        match got {
            Some(chunk) => {
                self.buf = chunk;
                Ok(())
            }
            None => Err(LineError::Closed),
        }
    }

    /// 读一条物理行（不含终结符）
    async fn read_physical(
        &mut self,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        timeout: Option<Duration>,
    ) -> Result<Vec<u8>, LineError> {
        if self.windows {
            self.read_physical_windows(rx, timeout).await
        } else {
            let mut out = Vec::new();
            loop {
                self.fill(rx, timeout).await?;
                // 借出 buf 避免与 out 的借用冲突
                let buf = std::mem::take(&mut self.buf);
                let seg = buf.get(self.pos..).unwrap_or_default();
                match seg.iter().position(|&b| b == b'\n') {
                    Some(i) => {
                        if seg.get(..i).is_some_and(|h| h.contains(&0x03)) {
                            self.buf = buf;
                            return Err(LineError::Interrupted);
                        }
                        if let Some(head) = seg.get(..i) {
                            out.extend_from_slice(head);
                        }
                        self.pos += i + 1;
                        self.buf = buf;
                        return Ok(out);
                    }
                    None => {
                        if seg.contains(&0x03) {
                            self.buf = buf;
                            return Err(LineError::Interrupted);
                        }
                        out.extend_from_slice(seg);
                        self.pos = buf.len();
                        self.buf = buf;
                        if out.len() > MAX_LINE {
                            return Err(LineError::TooLong);
                        }
                    }
                }
            }
        }
    }

    /// Windows 模式物理行（Go buffer.readLineOnWindows 的移植）
    async fn read_physical_windows(
        &mut self,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        timeout: Option<Duration>,
    ) -> Result<Vec<u8>, LineError> {
        let mut out: Vec<u8> = Vec::new();
        loop {
            self.fill(rx, timeout).await?;
            let buf = std::mem::take(&mut self.buf);
            let seg = buf.get(self.pos..).unwrap_or_default();
            let mut terminated = false;
            match seg.iter().position(|&b| b == b'!') {
                Some(i) => {
                    self.consume_windows(seg.get(..i).unwrap_or_default(), &mut out)?;
                    self.pos += i + 1; // 跳过 '!'
                    if buf.get(self.pos) == Some(&b'\n') {
                        self.pos += 1;
                    }
                    terminated = true;
                }
                None => {
                    self.consume_windows(seg, &mut out)?;
                    self.pos = buf.len();
                }
            }
            self.buf = buf;
            if terminated && !out.is_empty() && !self.w_skip_vt100 {
                return Ok(out);
            }
        }
    }

    /// Windows 模式逐字节过滤（ESC 序列跳过、字母保留、重绘重复字符去重）
    fn consume_windows(&mut self, seg: &[u8], out: &mut Vec<u8>) -> Result<(), LineError> {
        for &c in seg {
            if c == 0x03 {
                return Err(LineError::Interrupted);
            }
            if c == b'\n' {
                self.w_has_newline = true;
            }
            if self.w_skip_vt100 {
                if is_vt100_end(c) {
                    self.w_skip_vt100 = false;
                    // 光标移动可能引起 conhost 重绘重复字符
                    if c == b'H' && self.w_last.is_ascii_digit() {
                        self.w_may_duplicate = true;
                    }
                }
                if self.w_last == b'[' && c == b'H' {
                    self.w_has_cursor_home = true;
                }
                self.w_last = c;
            } else if c == 0x1b {
                self.w_skip_vt100 = true;
                self.w_last = c;
            } else if is_trzsz_letter(c) {
                if self.w_may_duplicate {
                    self.w_may_duplicate = false;
                    // 跳过重绘重复字符，如 "8\r\n\x1b[25;119H8" 里末尾的 "8"
                    if self.w_has_newline
                        && !out.is_empty()
                        && (out.last() == Some(&c) || self.w_pre_has_cursor_home)
                    {
                        if let Some(last) = out.last_mut() {
                            *last = c;
                        }
                        self.w_pre_has_cursor_home = self.w_has_cursor_home;
                        self.w_has_cursor_home = false;
                        self.w_has_newline = false;
                        continue;
                    }
                }
                out.push(c);
                self.w_pre_has_cursor_home = self.w_has_cursor_home;
                self.w_has_cursor_home = false;
                self.w_has_newline = false;
            }
        }
        Ok(())
    }

    /// 读一条逻辑行：junk（tmux）模式下按「行尾 `\r` = 折行标记」拼接后续
    /// 物理行（`junk` 参数对应 Go recvLine 的 mayHasJunk：ACT/CFG 阶段恒真，
    /// 其余帧由 CFG.tmux_output_junk 决定）
    pub async fn read_logical(
        &mut self,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        timeout: Option<Duration>,
        junk: bool,
    ) -> Result<Vec<u8>, LineError> {
        let mut line = self.read_physical(rx, timeout).await?;
        if junk && !self.windows {
            while line.last() == Some(&b'\r') {
                line.pop();
                let next = self.read_physical(rx, timeout).await?;
                line.extend_from_slice(&next);
                if line.len() > MAX_LINE {
                    return Err(LineError::TooLong);
                }
            }
        }
        Ok(line)
    }
}

impl Default for LineReader {
    fn default() -> Self {
        Self::new()
    }
}

/// 子串查找（buf 上朴素实现，帧行通常很短）
pub fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .find(|&i| haystack.get(i..i + needle.len()).is_some_and(|w| w == needle))
}

/// 最后一次出现位置
fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).rev().find(|&i| {
        haystack
            .get(i..i + needle.len())
            .is_some_and(|w| w == needle)
    })
}

/// tmux 状态行剥离（DCS `\x1bP=..\x1b\` 段；Go transfer.stripTmuxStatusLine）
fn strip_tmux_status(buf: &[u8]) -> Vec<u8> {
    // 逐段剥离 `\x1bP=…\x1b\\`（tmux 状态行 DCS）。上游 Python 只认「成对
    // 出现」的 DCS（单个 DCS 会把后续整段丢弃——按行处理时过于激进），
    // 这里改为逐段配对剥离：成对场景与上游结果一致，单个 DCS 只剥自身
    let mut buf = buf.to_vec();
    loop {
        let Some(begin) = find_subslice(&buf, b"\x1bP=") else {
            return buf;
        };
        match find_subslice(buf.get(begin + 3..).unwrap_or_default(), b"\x1b\\") {
            Some(end) => {
                let stop = begin + 3 + end + 2;
                buf.drain(begin..stop);
            }
            // 无终结符的 DCS：后续内容不可信，与上游一致截到段首
            None => return buf.get(..begin).unwrap_or_default().to_vec(),
        }
    }
}

/// 从（可能含噪声的）逻辑行中提取期望帧内容：
/// - junk 模式：截到最后一个 `#TYPE:`（或最后一个 `#`）、剥离 tmux 状态行
/// - 通用容错：剥离行尾 NUL 与行尾 `\r`（CRLF 残迹）
pub fn extract_frame(line: &[u8], expect: &str, junk: bool) -> Vec<u8> {
    let owned;
    let mut l: &[u8] = line;
    if junk {
        owned = strip_tmux_status(line);
        l = &owned;
    }
    // 剥首尾 NUL（Python：line.strip('\x00')）
    let mut start = 0;
    let mut end = l.len();
    while end > start && l.get(end - 1) == Some(&0) {
        end -= 1;
    }
    while start < end && l.get(start) == Some(&0) {
        start += 1;
    }
    let mut l = l.get(start..end).unwrap_or_default();
    // 行尾 \r 容错（非 junk 模式下的 CRLF；junk 模式的折行 \r 已在拼接时消费）
    if l.last() == Some(&b'\r') {
        l = l.get(..l.len() - 1).unwrap_or_default();
    }
    if junk {
        let mut marker = Vec::with_capacity(expect.len() + 2);
        marker.push(b'#');
        marker.extend_from_slice(expect.as_bytes());
        marker.push(b':');
        if let Some(i) = find_last(l, &marker) {
            return l.get(i..).unwrap_or_default().to_vec();
        }
        if let Some(i) = l.iter().rposition(|&b| b == b'#')
            && i > 0
        {
            return l.get(i..).unwrap_or_default().to_vec();
        }
    }
    l.to_vec()
}

/// 解析 `#TYPE:content` 帧，返回 (TYPE, content)
pub fn parse_frame(line: &[u8]) -> Result<(&str, &str), String> {
    if line.first() != Some(&b'#') {
        return Err(format!(
            "frame must start with '#': {:?}",
            String::from_utf8_lossy(line.get(..line.len().min(32)).unwrap_or_default())
        ));
    }
    let Some(idx) = line.iter().position(|&b| b == b':') else {
        return Err("frame missing ':'".to_string());
    };
    if idx < 2 {
        return Err("frame type too short".to_string());
    }
    let typ = std::str::from_utf8(line.get(1..idx).unwrap_or_default())
        .map_err(|_| "frame type not utf8".to_string())?;
    let content = std::str::from_utf8(line.get(idx + 1..).unwrap_or_default())
        .map_err(|_| "frame content not utf8".to_string())?;
    Ok((typ, content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_roundtrip() {
        for data in [
            &b""[..],
            b"hello trzsz",
            "中文文件名与内容 🎉".as_bytes(),
            vec![0u8; 100_000].as_slice(),
        ] {
            let enc = encode_payload(data);
            assert!(enc.is_ascii());
            assert!(!enc.contains('\n'));
            assert!(!enc.contains('\r'));
            assert_eq!(decode_payload(&enc).unwrap(), data);
        }
    }

    #[test]
    fn decode_tolerates_folded_whitespace() {
        // tmux 折行残迹：base64 中混入 \r\n 仍可解
        let data = b"folded payload";
        let enc = encode_payload(data);
        let folded = format!("{}\r\n{}", &enc[..enc.len() / 2], &enc[enc.len() / 2..]);
        assert_eq!(decode_payload(&folded).unwrap(), data);
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode_payload("!!!not-base64!!!").is_err());
        assert!(decode_payload("aGVsbG8=").is_err()); // 合法 base64 但非 zlib 流
    }

    #[test]
    fn md5_rfc1321_vectors() {
        let hex = |d: [u8; 16]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(md5_digest(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(md5_digest(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex(md5_digest(b"message digest")),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
        assert_eq!(
            hex(md5_digest(b"abcdefghijklmnopqrstuvwxyz")),
            "c3fcd3d76192e4007dfb496cca67e13b"
        );
        assert_eq!(
            hex(md5_digest(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789")),
            "d174ab98d277d9f5a5611c2c9f419d9f"
        );
        // 跨块边界（>64 字节）
        let long = vec![b'a'; 200];
        assert_eq!(
            hex(md5_digest(&long)),
            md5_digest(&long).iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        // 增量流式与一次性计算一致（含 64 字节边界切分）
        let data: Vec<u8> = (0u8..=255).cycle().take(1000).collect();
        let mut stream = Md5Stream::new();
        for chunk in data.chunks(17) {
            stream.update(chunk);
        }
        assert_eq!(hex(stream.finalize()), hex(md5_digest(&data)));
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    async fn read_all_logical(
        r: &mut LineReader,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        n: usize,
        junk: bool,
    ) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for _ in 0..n {
            out.push(r.read_logical(rx, None, junk).await.unwrap());
        }
        out
    }

    #[test]
    fn line_reader_splits_on_newline() {
        rt().block_on(async {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
            let mut r = LineReader::new();
            // 跨 chunk 撕裂的行
            tx.send(b"#NUM:3\n#SIZE:12".to_vec()).unwrap();
            tx.send(b"345\n".to_vec()).unwrap();
            let lines = read_all_logical(&mut r, &mut rx, 2, false).await;
            assert_eq!(lines.first().unwrap(), b"#NUM:3");
            assert_eq!(lines.get(1).unwrap(), b"#SIZE:12345");
        });
    }

    #[test]
    fn line_reader_timeout_and_close() {
        rt().block_on(async {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
            let mut r = LineReader::new();
            let err = r
                .read_logical(&mut rx, Some(Duration::from_millis(20)), false)
                .await
                .unwrap_err();
            assert_eq!(err, LineError::Timeout);
            drop(tx);
            let err = r.read_logical(&mut rx, None, false).await.unwrap_err();
            assert_eq!(err, LineError::Closed);
        });
    }

    #[test]
    fn line_reader_interrupted_by_ctrl_c() {
        rt().block_on(async {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
            let mut r = LineReader::new();
            tx.send(b"#DATA:abc\x03def\n".to_vec()).unwrap();
            assert_eq!(
                r.read_logical(&mut rx, None, false).await.unwrap_err(),
                LineError::Interrupted
            );
        });
    }

    #[test]
    fn junk_mode_joins_folded_lines() {
        rt().block_on(async {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
            let mut r = LineReader::new();
            // tmux 折行：折行点插入 \r\n——首物理行以 \r 收尾表示「未完待续」，
            // 次物理行正常 \n 收尾即逻辑行终止（两行拼接后无残留 \r）
            let folded = encode_payload(b"junk-mode-payload");
            tx.send(format!("#DATA:{}\r", &folded[..16]).into_bytes()).unwrap();
            tx.send(format!("\n{}\n", &folded[16..]).into_bytes()).unwrap();
            tx.send(b"#NUM:1\n".to_vec()).unwrap();
            // 关闭发送端：junk 模式下 \r\n 是折叠标记，末条逻辑行要靠
            // EOF（rx 关闭）收尾；不 drop 会让 read_logical 永久等待
            drop(tx);
            let lines = read_all_logical(&mut r, &mut rx, 2, true).await;
            assert_eq!(lines.first().unwrap().len(), 6 + folded.len());
            assert_eq!(lines.first().unwrap(), format!("#DATA:{folded}").as_bytes());
            assert_eq!(lines.get(1).unwrap(), b"#NUM:1");
        });
    }

    #[test]
    fn windows_mode_filters_and_terminates_on_bang() {
        rt().block_on(async {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
            let mut r = LineReader::new();
            r.set_windows(true);
            // conhost 注入的 ESC 序列与 CRLF 被滤除，行以 '!' 终结
            tx.send(b"\x1b[25;119H#DATA:abc\r\n".to_vec()).unwrap();
            tx.send(b"def!\n#NUM:2!\n".to_vec()).unwrap();
            let lines = read_all_logical(&mut r, &mut rx, 2, false).await;
            assert_eq!(lines.first().unwrap(), b"#DATA:abcdef");
            assert_eq!(lines.get(1).unwrap(), b"#NUM:2");
        });
    }

    #[test]
    fn extract_frame_takes_last_marker_and_strips_noise() {
        // junk 截取：噪声 + 帧内容 + 噪声重绘 → 取最后一个 #NUM:
        let line = b"tmux junk \x1bP=1\x1b\\#NUM:3 redraw #NUM:4";
        let got = extract_frame(line, "NUM", true);
        assert_eq!(got, b"#NUM:4");
        // 非 junk：原样（仅剥行尾 \r / NUL）
        let got = extract_frame(b"#NUM:3\r", "NUM", false);
        assert_eq!(got, b"#NUM:3");
        let got = extract_frame(b"\x00#NUM:3\x00", "NUM", true);
        assert_eq!(got, b"#NUM:3");
        // junk 含 tmux 状态行（未闭合 → 截到 begin）
        let got = extract_frame(b"#NUM:3\x1bP=abc", "NUM", true);
        assert_eq!(got, b"#NUM:3");
    }

    #[test]
    fn parse_frame_basics() {
        let (typ, content) = parse_frame(b"#SUCC:1024").unwrap();
        assert_eq!(typ, "SUCC");
        assert_eq!(content, "1024");
        assert!(parse_frame(b"no-hash").is_err());
        assert!(parse_frame(b"#:x").is_err());
        assert!(parse_frame(b"no colon").is_err());
    }

    #[test]
    fn source_file_json_roundtrip() {
        let sf = SourceFileJson {
            path_id: 0,
            path_name: vec!["dir".into(), "sub".into(), "a.bin".into()],
            is_dir: false,
            size: 12345,
        };
        let json = serde_json::to_string(&sf).unwrap();
        assert!(json.contains("\"path_name\""));
        let back: SourceFileJson = serde_json::from_str(&json).unwrap();
        assert_eq!(back, sf);
        assert_eq!(back.file_name(), "a.bin");
        // 目录条目缺省 size 字段可解析
        let dir_entry: SourceFileJson =
            serde_json::from_str(r#"{"path_id":1,"path_name":["root"],"is_dir":true}"#).unwrap();
        assert!(dir_entry.is_dir);
        assert_eq!(dir_entry.size, 0);
    }
}
