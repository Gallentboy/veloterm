//! trzsz 传输的终端流入口探测（纯状态机，无 IO，可单测）。
//!
//! 服务端（trz / tsz）在终端输出的触发串（Go tsz.go / trz.go）：
//! ```text
//! \x1b[s::TRZSZ:TRANSFER:{S|R|D|F}:{version}[:{013dUniqueId}][:{port}]\r\n
//! ```
//! - `S` = tsz 下载（服务器发文件）；`R` = trz 上传；`D` = trz -d（目录上传，
//!   选目录）；`F` = 服务端指定客户端文件（client_files，MVP 不支持）
//! - tmux 普通模式下前缀可能是 `\n\x1b[1A\x1b[0J`；保存光标前缀也可能以
//!   `\x1b7`（DECSC）出现，两种都认
//! - uniqueId 末位编码服务端环境：`..10` = Windows 服务端（行协议切 `!`
//!   终结、禁二进制）；`..20` = tmux 内。tmux 重绘会**重放**同 id 的触发
//!   串，必须按 id 去重防重复触发
//!
//! 状态机两级：
//! - **Scanning**：缓存可能构成触发串前缀的尾部（≤20 字节），其余字节透传；
//! - **Completing**：命中 `::TRZSZ:TRANSFER:` 后积累到行尾（`\n`）再校验，
//!   非法（解析失败/超长/重复 id）→ 缓冲作普通输出放行、回到 Scanning。
//!
//! 命中后本行（含 `\x1b[s` 前缀与 `\r\n`）整体吞掉不进终端；行后的字节
//! 以 `rest` 交给传输引擎。

use std::collections::VecDeque;

use super::codec::find_subslice;

/// 触发串固定标记
const MARKER: &[u8] = b"::TRZSZ:TRANSFER:";
/// 尾部缓存上限：`\x1b[s`（3）+ 标记（17）
const MAX_TAIL: usize = MARKER.len() + 3;
/// Completing 缓冲上限（标记 + 模式 + 版本 + id + 端口 + CRLF，128 足够）
const MAX_TRIGGER_LINE: usize = 128;
/// unique id 去重容量（tmux 重绘重放窗口）
const SEEN_IDS_CAP: usize = 64;

/// 触发串解析结果
#[derive(Debug, Clone, PartialEq)]
pub struct TrszszTrigger {
    /// b'S' | b'R' | b'D' | b'F'
    pub mode: u8,
    /// 服务端版本串（如 "1.2.0"；仅记录，协议按客户端声明的 v1 走）
    pub version: String,
    /// 13 位毫秒时间戳式唯一 id（旧服务端可能为空）
    pub unique_id: String,
    /// Windows 服务端（id == "1" 或 13 位且末尾 "10"）
    pub win_server: bool,
}

impl TrszszTrigger {
    /// 传输方向：S = download；R/D/F = upload
    pub fn direction(&self) -> &'static str {
        if self.mode == b'S' {
            "download"
        } else {
            "upload"
        }
    }
}

/// 喂入一段终端输出的结果
#[derive(Debug, PartialEq)]
pub enum FeedOutcome {
    /// 未命中：应显示到终端的字节（可能少于输入——疑似触发串前缀被暂扣）
    Passthrough(Vec<u8>),
    /// 命中触发串：`display` 先显示（触发行之前的正常输出），随后启动传输，
    /// `rest`（触发行之后未消费的字节）喂给引擎
    Triggered {
        display: Vec<u8>,
        info: TrszszTrigger,
        rest: Vec<u8>,
    },
}

/// trzsz 触发串探测器（跨 chunk 状态机）
pub struct TrszszDetector {
    /// Scanning 态尾部缓存（≤ MAX_TAIL）
    tail: Vec<u8>,
    /// Completing 态缓冲（自吞咽起点开始，含标记本身）
    completing: Option<Vec<u8>>,
    /// 已见 unique id（tmux 重绘重放去重；reset 不清空）
    seen_ids: VecDeque<String>,
}

impl Default for TrszszDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl TrszszDetector {
    pub fn new() -> Self {
        Self {
            tail: Vec::new(),
            completing: None,
            seen_ids: VecDeque::new(),
        }
    }

    /// 喂入一段终端输出（检测串可能跨 chunk 断开）
    pub fn feed(&mut self, chunk: &[u8]) -> FeedOutcome {
        let mut passthrough = Vec::new();
        for (i, &b) in chunk.iter().enumerate() {
            if self.completing.is_none() {
                self.tail.push(b);
                if self.tail.len() > MAX_TAIL {
                    let overflow = self.tail.len() - MAX_TAIL;
                    self.tail.drain(..overflow);
                }
                if let Some(mpos) = find_subslice(&self.tail, MARKER) {
                    // 吞咽起点回溯：紧邻的保存光标前缀一并吞掉。三种形态：
                    // `\x1b[s`（ANSI.SYS / trzsz-go）、`\x1b7\x07`（Python 服务端：DECSC+BEL）
                    // 、`\x1b7` 裸 DECSC——按长度降序检查避免短前缀漏字节
                    let mut swallow_from = mpos;
                    let three = mpos
                        .checked_sub(3)
                        .and_then(|s| self.tail.get(s..mpos));
                    if three.is_some_and(|w| w == b"\x1b7\x07" || w == b"\x1b[s") {
                        swallow_from = mpos - 3;
                    } else if mpos >= 2 && self.tail.get(mpos - 2..mpos).is_some_and(|w| w == b"\x1b7") {
                        swallow_from = mpos - 2;
                    }
                    if let Some(head) = self.tail.get(..swallow_from) {
                        passthrough.extend_from_slice(head);
                    }
                    if let Some(rest) = self.tail.get(swallow_from..) {
                        self.completing = Some(rest.to_vec());
                    }
                    self.tail.clear();
                } else {
                    // 尾部只保留「仍可能构成触发串前缀」的最长后缀
                    let keep = holdback_len(&self.tail);
                    let flush_to = self.tail.len() - keep;
                    if let Some(head) = self.tail.get(..flush_to) {
                        passthrough.extend_from_slice(head);
                    }
                    self.tail.drain(..flush_to);
                }
            } else {
                let buf = self.completing.as_mut().expect("completing checked");
                buf.push(b);
                let done = b == b'\n';
                let invalid = buf.len() > MAX_TRIGGER_LINE;
                if done || invalid {
                    let buf = self.completing.take().unwrap_or_default();
                    match parse_trigger(&buf) {
                        Ok(info) => {
                            if info.unique_id.is_empty() || !self.is_seen_id(&info.unique_id) {
                                if !info.unique_id.is_empty() {
                                    self.remember_id(&info.unique_id);
                                }
                                return FeedOutcome::Triggered {
                                    display: passthrough,
                                    info,
                                    rest: chunk.get(i + 1..).unwrap_or_default().to_vec(),
                                };
                            }
                            // tmux 重绘重放的同 id 触发串：当普通输出放行
                            passthrough.extend_from_slice(&buf);
                        }
                        Err(_) => {
                            // 非法形态（如 cat 含触发串文本的文件）：放行
                            passthrough.extend_from_slice(&buf);
                        }
                    }
                }
            }
        }
        FeedOutcome::Passthrough(passthrough)
    }

    /// 传输结束/取消后复位（seen_ids 保留：重绘重放的去重必须跨会话生效）
    pub fn reset(&mut self) {
        self.tail.clear();
        self.completing = None;
    }

    fn is_seen_id(&self, id: &str) -> bool {
        self.seen_ids.iter().any(|seen| seen == id)
    }

    fn remember_id(&mut self, id: &str) {
        if self.seen_ids.len() >= SEEN_IDS_CAP {
            self.seen_ids.pop_front();
        }
        self.seen_ids.push_back(id.to_string());
    }
}

/// 尾部缓存里「仍可能构成触发串前缀」的最长后缀长度。
/// 候选是**完整前缀串**（保存光标变体 + 标记的拼接）：只有后缀整体是某个
/// 候选的前缀才保留——分立候选（只查 `\x1b[s` 或标记各自的前缀）会把
/// `\x1b[s:` 这种「变体已现、标记未完」的中间态错判为无潜力而提前冲刷
/// （实测：`\x1b[s` 被单独冲走、后续标记命中时回溯失败，前缀泄漏到终端）。
fn holdback_len(tail: &[u8]) -> usize {
    // \x1b[s + 标记；\x1b7\x07 + 标记（Python 服务端）；\x1b7 + 标记；裸标记
    const CANDIDATES: [&[u8]; 4] = [
        b"\x1b[s::TRZSZ:TRANSFER:",
        b"\x1b7\x07::TRZSZ:TRANSFER:",
        b"\x1b7::TRZSZ:TRANSFER:",
        b"::TRZSZ:TRANSFER:",
    ];
    let max_check = MAX_TAIL.min(tail.len());
    for k in (1..=max_check).rev() {
        let suffix = tail.get(tail.len() - k..).unwrap_or_default();
        if CANDIDATES.iter().any(|c| starts_with(c, suffix)) {
            return k;
        }
    }
    0
}

/// candidate 是否以 prefix 开头（prefix 是 candidate 的真前缀或相等）
fn starts_with(candidate: &[u8], prefix: &[u8]) -> bool {
    candidate.len() >= prefix.len() && candidate.get(..prefix.len()).is_some_and(|w| w == prefix)
}

/// 校验并解析完整触发行：
/// `[\x1b[s|\x1b7]::TRZSZ:TRANSFER:{mode}:{ver}[:{id}][:{port}]\r?\n`
fn parse_trigger(buf: &[u8]) -> Result<TrszszTrigger, ()> {
    let mut s: &[u8] = buf;
    // 剥保存光标前缀
    if s.starts_with(b"\x1b[s") {
        s = s.get(3..).ok_or(())?;
    } else if s.starts_with(b"\x1b7") {
        s = s.get(2..).ok_or(())?;
    }
    if !s.starts_with(MARKER) {
        return Err(());
    }
    let mut rest = s.get(MARKER.len()..).ok_or(())?;
    // 行尾必须是 \r\n 或 \n
    if rest.ends_with(b"\r\n") {
        rest = rest.get(..rest.len() - 2).ok_or(())?;
    } else if rest.ends_with(b"\n") {
        rest = rest.get(..rest.len() - 1).ok_or(())?;
    } else {
        return Err(());
    }
    // mode
    let mode = *rest.first().ok_or(())?;
    if !matches!(mode, b'S' | b'R' | b'D' | b'F') {
        return Err(());
    }
    rest = rest.get(1..).ok_or(())?;
    if rest.first() != Some(&b':') {
        return Err(());
    }
    rest = rest.get(1..).ok_or(())?;
    // version：\d+\.\d+\.\d+
    let mut version = String::new();
    for part in 0..3 {
        let digits = take_digits(&mut rest)?;
        if part < 2 {
            if rest.first() != Some(&b'.') {
                return Err(());
            }
            rest = rest.get(1..).ok_or(())?;
        }
        version.push_str(&digits);
        if part < 2 {
            version.push('.');
        }
    }
    // 可选 :uniqueId（数字串）
    let mut unique_id = String::new();
    if rest.first() == Some(&b':') {
        rest = rest.get(1..).ok_or(())?;
        unique_id = take_digits(&mut rest)?;
    }
    // 可选 :port（数字，忽略内容）
    if rest.first() == Some(&b':') {
        rest = rest.get(1..).ok_or(())?;
        let _ = take_digits(&mut rest)?;
    }
    if !rest.is_empty() {
        return Err(());
    }
    let win_server = unique_id == "1"
        || (unique_id.len() == 13 && unique_id.ends_with("10"));
    Ok(TrszszTrigger {
        mode,
        version,
        unique_id,
        win_server,
    })
}

/// 从头取一段十进制数字（至少一位），前进切片
fn take_digits(rest: &mut &[u8]) -> Result<String, ()> {
    let n = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    if n == 0 {
        return Err(());
    }
    let digits = String::from_utf8_lossy(rest.get(..n).ok_or(())?).into_owned();
    *rest = rest.get(n..).ok_or(())?;
    Ok(digits)
}

/// 传输结束后的残尾过滤器（照 zmodem/detector.rs 的 TailFrameFilter 形态）：
/// 服务端退出序列（`\x1b8\x1b[0J` 复位光标 + 清屏、`\x1b[?25h` 显光标）与
/// 迟到的协议帧（`#TYPE:base64` 形态）会被吞掉；「Saved N files..」等正常
/// 文本与 shell 提示符放行。字节预算耗尽即自动退出静默。
pub struct TrszszTailFilter {
    remaining: usize,
}

impl Default for TrszszTailFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl TrszszTailFilter {
    /// 静默窗口字节预算
    const BUDGET: usize = 4096;

    pub fn new() -> Self {
        Self { remaining: 0 }
    }

    /// 传输结束后开启静默窗口
    pub fn arm(&mut self) {
        self.remaining = Self::BUDGET;
    }

    pub fn is_armed(&self) -> bool {
        self.remaining > 0
    }

    /// 过滤一段输出，返回应透传给终端的字节。
    ///
    /// 逐行判定：`#TYPE:base64` 形态的迟到协议帧与空白残行吞掉；「Saved N
    /// files..」、shell 提示符等真实文本放行并立即退出静默。
    pub fn filter(&mut self, data: &[u8]) -> Vec<u8> {
        if self.remaining == 0 {
            return data.to_vec();
        }
        self.remaining = self.remaining.saturating_sub(data.len());
        let cleaned = remove_sequences(data);
        let mut filtered = Vec::with_capacity(cleaned.len());
        let mut passed_text = false;
        let mut start = 0;
        let mut i = 0;
        while i < cleaned.len() {
            if cleaned.get(i) == Some(&b'\n') {
                if let Some(line) = cleaned.get(start..=i) {
                    let swallable = !passed_text && (is_protocol_frame_line(line) || is_blank(line));
                    if !swallable {
                        filtered.extend_from_slice(line);
                        if !is_blank(line) {
                            passed_text = true;
                        }
                    }
                }
                start = i + 1;
            }
            i += 1;
        }
        if start < cleaned.len()
            && let Some(line) = cleaned.get(start..)
        {
            if passed_text {
                filtered.extend_from_slice(line);
            } else if !is_protocol_frame_line(line) && !is_blank(line) {
                // 无行尾的真实文本（如 shell 提示符）：放行并退出静默
                filtered.extend_from_slice(line);
                passed_text = true;
            }
            // 悬空的空白/协议帧残段：保持吞咽，等下一段判完
        }
        if passed_text {
            self.remaining = 0;
        }
        filtered
    }
}

/// 删除服务端退出序列字节（ESC 8 + CSI 0J、CSI ?25h/l）
fn remove_sequences(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data.get(i) == Some(&0x1b) {
            let rest = data.get(i..).unwrap_or_default();
            if rest.starts_with(b"\x1b8\x1b[0J") {
                i += 6;
                continue;
            }
            if rest.starts_with(b"\x1b[?25h") || rest.starts_with(b"\x1b[?25l") {
                i += 6;
                continue;
            }
        }
        if let Some(&b) = data.get(i) {
            out.push(b);
        }
        i += 1;
    }
    out
}

/// `#TYPE:base64...` 形态判定（TYPE 为 2-6 位大小写字母）
fn is_protocol_frame_line(line: &[u8]) -> bool {
    if line.first() != Some(&b'#') || line.len() < 4 {
        return false;
    }
    let colon = match line.iter().position(|&b| b == b':') {
        Some(c) if (2..=7).contains(&c) => c,
        _ => return false,
    };
    if !line
        .get(1..colon)
        .is_some_and(|seg| seg.iter().all(|b| b.is_ascii_alphabetic()))
    {
        return false;
    }
    line.get(colon + 1..).is_some_and(|rest| {
        rest.iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b' ' | b'\r' | b'\n'))
    })
}

/// 空白残行（仅 CR/LF）：退出序列剥离后的换行残迹
fn is_blank(line: &[u8]) -> bool {
    line.iter().all(|b| matches!(b, b'\r' | b'\n'))
}

#[cfg(test)]
#[allow(clippy::panic)] // 断言失败分支以 panic! 报告（restriction lint 经 [lints] deny，对测试同样生效）
mod tests {
    use super::*;

    /// 服务端真实形态（tsz.go）：`\x1b[s::TRZSZ:TRANSFER:S:1.2.0:{id}:{port}\r\n`
    const REAL_TRIGGER: &[u8] =
        b"\x1b[s::TRZSZ:TRANSFER:S:1.2.0:1730100000000:39877\r\n";

    #[test]
    fn detects_real_trigger_and_strips_line() {
        let mut d = TrszszDetector::new();
        let out = d.feed(b"[root@host ~]# tsz demo.zip\r\n");
        assert!(matches!(out, FeedOutcome::Passthrough(p) if p == b"[root@host ~]# tsz demo.zip\r\n"));
        match d.feed(REAL_TRIGGER) {
            FeedOutcome::Triggered { display, info, rest } => {
                assert!(display.is_empty());
                assert_eq!(info.mode, b'S');
                assert_eq!(info.version, "1.2.0");
                assert_eq!(info.unique_id, "1730100000000");
                assert!(!info.win_server);
                assert!(rest.is_empty());
                assert_eq!(info.direction(), "download");
            }
            other => panic!("expected trigger, got {other:?}"),
        }
    }

    #[test]
    fn trigger_followed_by_more_bytes_in_same_chunk() {
        let mut d = TrszszDetector::new();
        let mut chunk = REAL_TRIGGER.to_vec();
        chunk.extend_from_slice(b"#CFG:eHg=\n");
        match d.feed(&chunk) {
            FeedOutcome::Triggered { rest, .. } => {
                assert_eq!(rest, b"#CFG:eHg=\n");
            }
            other => panic!("expected trigger, got {other:?}"),
        }
    }

    #[test]
    fn trigger_torn_across_chunks() {
        let mut d = TrszszDetector::new();
        // 逐字节撕裂
        let mut got: Option<FeedOutcome> = None;
        for &b in REAL_TRIGGER {
            let out = d.feed(&[b]);
            if !matches!(out, FeedOutcome::Passthrough(_)) {
                got = Some(out);
            }
        }
        match got.expect("should trigger") {
            FeedOutcome::Triggered { info, rest, .. } => {
                assert_eq!(info.mode, b'S');
                assert!(rest.is_empty());
            }
            other => panic!("expected trigger, got {other:?}"),
        }
    }

    #[test]
    fn torn_at_marker_boundary() {
        let mut d = TrszszDetector::new();
        assert!(matches!(
            d.feed(b"noise ::TRZSZ:TRANSFER"),
            FeedOutcome::Passthrough(_)
        ));
        match d.feed(b":R:1.1.6:1730100000000\r\n") {
            FeedOutcome::Triggered { info, .. } => {
                assert_eq!(info.mode, b'R');
                assert_eq!(info.direction(), "upload");
            }
            other => panic!("expected trigger, got {other:?}"),
        }
    }

    #[test]
    fn tmux_prefix_and_decsc_variant() {
        let mut d = TrszszDetector::new();
        // tmux 普通模式：\n\x1b[1A\x1b[0J 前缀（来自服务端）；id 末尾 20 = tmux
        let tmux_trigger = b"\n\x1b[1A\x1b[0J\x1b[s::TRZSZ:TRANSFER:S:1.2.0:1730100000020\r\n";
        match d.feed(tmux_trigger) {
            FeedOutcome::Triggered { display, info, .. } => {
                assert_eq!(display, b"\n\x1b[1A\x1b[0J");
                assert!(!info.win_server);
            }
            other => panic!("expected trigger, got {other:?}"),
        }
        // DECSC 形态 \x1b7
        let mut d2 = TrszszDetector::new();
        match d2.feed(b"\x1b7::TRZSZ:TRANSFER:R:1.2.0:1730100000000:1234\r\n") {
            FeedOutcome::Triggered { display, info, .. } => {
                assert!(display.is_empty());
                assert_eq!(info.mode, b'R');
            }
            other => panic!("expected trigger, got {other:?}"),
        }
    }

    #[test]
    fn win_server_id_suffix() {
        let mut d = TrszszDetector::new();
        match d.feed(b"\x1b[s::TRZSZ:TRANSFER:S:1.2.0:1730100000010\r\n") {
            FeedOutcome::Triggered { info, .. } => assert!(info.win_server),
            other => panic!("expected trigger, got {other:?}"),
        }
        // id == "1" 也算 Windows 服务端（旧约定）
        let mut d2 = TrszszDetector::new();
        match d2.feed(b"::TRZSZ:TRANSFER:R:0.2.0:1\r\n") {
            FeedOutcome::Triggered { info, .. } => {
                assert!(info.win_server);
                assert_eq!(info.unique_id, "1");
            }
            other => panic!("expected trigger, got {other:?}"),
        }
    }

    #[test]
    fn repeated_unique_id_passes_through() {
        let mut d = TrszszDetector::new();
        match d.feed(REAL_TRIGGER) {
            FeedOutcome::Triggered { .. } => {}
            other => panic!("expected trigger, got {other:?}"),
        }
        d.reset();
        // tmux 重绘重放同一 id：不再触发，作为普通输出放行
        match d.feed(REAL_TRIGGER) {
            FeedOutcome::Passthrough(p) => assert_eq!(p, REAL_TRIGGER),
            other => panic!("expected passthrough, got {other:?}"),
        }
        // 新 id 正常触发
        let fresh = b"\x1b[s::TRZSZ:TRANSFER:S:1.2.0:1799999999999\r\n";
        assert!(matches!(d.feed(fresh), FeedOutcome::Triggered { .. }));
    }

    #[test]
    fn plain_text_and_partial_marker_pass_through() {
        let mut d = TrszszDetector::new();
        // 普通输出 + 冒号结尾（疑似前缀被暂扣后放行）
        match d.feed(b"hello: world\n::TRZSZ:") {
            FeedOutcome::Passthrough(p) => {
                assert_eq!(p, b"hello: world\n");
            }
            other => panic!("expected passthrough, got {other:?}"),
        }
        // 后续字节证明不是触发串 → 暂扣部分放行
        match d.feed(b"TRANSFER:not-a-trigger\n") {
            FeedOutcome::Passthrough(p) => {
                assert_eq!(p, b"::TRZSZ:TRANSFER:not-a-trigger\n");
            }
            other => panic!("expected passthrough, got {other:?}"),
        }
    }

    #[test]
    fn catting_trigger_like_text_does_not_fire() {
        let mut d = TrszszDetector::new();
        // grep TRZSZ 文件：触发串后没有合法版本号
        let line = b"::TRZSZ:TRANSFER:S:nightly:0000000000000\r\n";
        match d.feed(line) {
            FeedOutcome::Passthrough(p) => assert_eq!(p, line),
            other => panic!("expected passthrough, got {other:?}"),
        }
        // 模式非法
        let line2 = b"::TRZSZ:TRANSFER:X:1.2.0:1730100000000\r\n";
        assert!(matches!(d.feed(line2), FeedOutcome::Passthrough(_)));
        // 超长 completing 缓冲（无行尾的连续垃圾）
        let mut d2 = TrszszDetector::new();
        let mut junk = b"::TRZSZ:TRANSFER:S:".to_vec();
        junk.extend(std::iter::repeat_n(b'a', 200));
        match d2.feed(&junk) {
            FeedOutcome::Passthrough(_) => {}
            other => panic!("expected passthrough, got {other:?}"),
        }
    }

    #[test]
    fn reset_clears_partial_state() {
        let mut d = TrszszDetector::new();
        d.feed(b"::TRZSZ:TRA");
        d.reset();
        match d.feed(b"NSFER:S:1.2.0:1730100000000\r\n") {
            FeedOutcome::Passthrough(p) => {
                assert_eq!(p, b"NSFER:S:1.2.0:1730100000000\r\n");
            }
            other => panic!("expected passthrough, got {other:?}"),
        }
    }

    #[test]
    fn holdback_keeps_only_prefix_potential() {
        assert_eq!(holdback_len(b"abc"), 0);
        assert_eq!(holdback_len(b"abc:"), 1);
        assert_eq!(holdback_len(b"abc::"), 2);
        // x 不属于任何候选前缀；后 7 字节 \x1b[s::TR 是「\x1b[s+标记」完整候选的前缀
        assert_eq!(holdback_len(b"x\x1b[s::TR"), 7);
        // 完整标记会在 feed 中命中，不会出现在 holdback 判定里
        assert_eq!(holdback_len(b"\x1b7::"), 4);
    }

    #[test]
    fn tail_filter_swallows_exit_sequence_and_frames() {
        let mut f = TrszszTailFilter::new();
        f.arm();
        let out = f.filter(b"\x1b8\x1b[0JSaved 1 file to /tmp\r\n");
        assert_eq!(out, b"Saved 1 file to /tmp\r\n");
        // 已放行文本 → 退出静默
        let out = f.filter(b"#SUCC:1024\r\n");
        assert_eq!(out, b"#SUCC:1024\r\n");

        // 反例：纯协议残尾全吞
        let mut f2 = TrszszTailFilter::new();
        f2.arm();
        assert!(f2.filter(b"\x1b[?25h\r\n").is_empty());
        assert!(f2.filter(b"#fail:eHg=\r\n").is_empty());
        assert!(f2.filter(b"[user@host ~]$ ").starts_with(b"[user"));
    }
}
