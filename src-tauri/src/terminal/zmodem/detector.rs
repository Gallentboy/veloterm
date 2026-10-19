//! ZMODEM 传输的终端流入口探测（纯状态机，无 IO，可单测）。
//!
//! 两种入口：
//! - **sz（服务器 → 本地，下载）**：服务器 sz 输出 ZRQINIT 头。hex 编码形态为
//!   `**\x18B00` + 8 个十六进制字符（ZPAD×2 + ZDLE + 'B' + 00 头类型 + 4 位
//!   帧类型 + 4 位 CRC），bin16/bin32 形态为 `**\x18A`/`**\x18C`。这些字节
//!   含 `\x18` 控制字符，文本日志不可能凑出——误触发面为零。
//! - **rz（服务器等待接收，上传）**：lrz 启动时输出字面 `rz\r`（部分实现为
//!   `\x18rz\r`），这是给终端模拟器的「请发文件」公告。纯 ASCII，故只认
//!   **行首**出现（上一字节是 \n 或流开头）；误触发时后续握手无对端响应、
//!   超时干净退出，不传数据。
//!
//! 状态机在命中前缓存最多 `MAX_PREFIX` 字节（用于把 `**\x18B0` 之前的 ZPAD
//! 等前缀完整交回，命中后不再向前端转发任何字节）。

/// 探测结果
#[derive(Debug, PartialEq)]
pub enum Detection {
    /// 未命中，透传
    None,
    /// 命中协议帧但非入口类型（ZFIN/ZFILE 等收尾/进行中信号）：
    /// 吞掉不透传（防进 bash 被当命令执行）、不启动新传输
    Swallow,
    /// 命中 sz 下载（服务器要发文件，帧为 ZFILE 等）
    Sz { prefix: Vec<u8> },
    /// 命中 rz 上传（服务器在等文件：ZRQINIT 帧，或 `rz\r` 公告）
    Rz { prefix: Vec<u8> },
}

/// 探测器：跨 chunk 维护最多 12 字节尾部 + 「上一字节是否行首」状态
pub struct ZmodemDetector {
    /// 尚未判定的尾部缓存（上限 MAX_PREFIX，超出则丢弃最旧——只需足够判定）
    tail: Vec<u8>,
    /// 上一个已确定透传的字节是否为换行（rz 公告行首判定）
    at_line_start: bool,
}

/// 协议头前缀：`**` + ZDLE(0x18) + 编码字节
/// - 'B' = hex 编码（帧类型在后续 hex 字符里）
/// - 'A' = bin16 / 'C' = bin32（帧类型在紧随的 1 字节）
const HEX_PREFIX: &[u8] = b"**\x18B";
const BIN16_PREFIX: &[u8] = b"**\x18A";
const BIN32_PREFIX: &[u8] = b"**\x18C";
/// 前缀缓存上限：hex 形态需要 `**\x18B` + 2 个 hex 字符（帧类型）+ 余量
const MAX_PREFIX: usize = 16;

impl Default for ZmodemDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl ZmodemDetector {
    pub fn new() -> Self {
        Self {
            tail: Vec::new(),
            at_line_start: true,
        }
    }

    /// 喂入一段终端输出，返回探测结果。
    ///
    /// - `Detection::None`：本段全部字节应透传前端（调用方照旧 send）；
    /// - 命中：返回的 `prefix` 必须交给 ZMODEM 引擎（其中包含本段已缓存字节），
    ///   本段**其余未消费字节由调用方一并转交引擎**（实现上调用方在命中后把
    ///   整个本段交给引擎即可——引擎会按协议自行跳过前导垃圾）。
    pub fn feed(&mut self, chunk: &[u8]) -> Detection {
        for &b in chunk {
            // 本字节是否位于行首（用 push 之前的状态判定；rz 公告的判定
            // 必须基于「r 所在位置是行首」，而 tail 匹配发生在后续字节上）
            let was_line_start = self.at_line_start;
            self.tail.push(b);
            if self.tail.len() > MAX_PREFIX {
                let overflow = self.tail.len() - MAX_PREFIX;
                self.tail.drain(..overflow);
            }

            // 协议帧：`**\x18B` + 2 个 hex 字符（帧类型字节）→ 判方向。
            // 方向语义（与 zmodem2-js 的探测逻辑一致，实测校验）：
            //   ZRQINIT(0x00) = sz 发起「我要发文件」 → 本地接收 = download
            //   ZRINIT (0x01) = rz 发起「我准备好收」 → 本地上传 = upload
            //   其余帧（ZFILE 等）= 传输已在进行 → 按 download 兜底
            // 关键：**不能用 `rz\r` 公告判方向**——lrzsz 的 sz 也会先输出
            // `rz\r`（给传统终端模拟器的「准备接收」提示），实测 head=
            // `rz\r**\x18B000000000`：公告在前、ZRQINIT 帧在后。若在公告上
            // 抢先命中就会把 sz 误判成上传（实测踩坑）。方向只认协议帧类型。
            if let Some(pos) = find_subslice(&self.tail, HEX_PREFIX) {
                let after = &self.tail[pos + HEX_PREFIX.len()..];
                if after.len() >= 2 {
                    let hi = hex_val(after[0]);
                    let lo = hex_val(after[1]);
                    if let (Some(hi), Some(lo)) = (hi, lo) {
                        let frame_type = (hi << 4) | lo;
                        let prefix = self.tail.clone();
                        self.tail.clear();
                        // 只认入口帧：0 = ZRQINIT（sz 下载）、1 = ZRINIT（rz 上传）。
                        // 其他类型（ZFIN=08 收尾、ZFILE=04 传输中等）是既有会话
                        // 的信号，不触发新传输——实测上传完成后服务器 lrz 发的
                        // ZFIN 被「非 01 即 Sz」误判为下载，弹了假保存框。
                        return match frame_type {
                            0 => Detection::Sz { prefix },
                            1 => Detection::Rz { prefix },
                            // 收尾/进行中帧：吞掉（透传会进 bash 被当命令执行）
                            _ => Detection::Swallow,
                        };
                    }
                }
            }

            // bin16 / bin32：`**\x18A` / `**\x18C` + 1 字节帧类型
            for pfx in [BIN16_PREFIX, BIN32_PREFIX] {
                if let Some(pos) = find_subslice(&self.tail, pfx)
                    && let Some(&frame_type) = self.tail.get(pos + pfx.len())
                {
                    let prefix = self.tail.clone();
                    self.tail.clear();
                    // 只认入口帧：0 = ZRQINIT（sz 下载）、1 = ZRINIT（rz 上传）。
                    // 其他类型（ZFIN=08 收尾、ZFILE=04 传输中等）是既有会话
                    // 的信号，不触发新传输——实测上传完成后服务器 lrz 发的
                    // ZFIN 被「非 01 即 Sz」误判为下载，弹了假保存框。
                    return match frame_type {
                        0 => Detection::Sz { prefix },
                        1 => Detection::Rz { prefix },
                        // 收尾/进行中帧：吞掉（透传会进 bash 被当命令执行）
                        _ => Detection::Swallow,
                    };
                }
            }


            // 行首状态推进：只有 \n 之后才是行首（\r 不算——CRLF 的 \r 后
            // 紧跟 \n；旧式 \r 结尾的行不视为新行）
            self.at_line_start = b == b'\n';
            let _ = was_line_start;
        }
        Detection::None
    }

    /// 传输结束/取消后复位（行首状态重置）
    pub fn reset(&mut self) {
        self.tail.clear();
        self.at_line_start = true;
    }
}

/// 子串查找（tail 很小，朴素实现足够）
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// 单个 hex 字符值
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// 传输结束后的协议残尾过滤器：引擎退出后服务器可能还会补发 ZFIN/ZFIN-OO
/// 等收尾帧（含 `\x18` 控制字节）。这些字节若透传到终端会被 shell 当命令
/// 执行（实测：`**0800000000022d` → `-bash: **0800000000022d: 未找到命令`）。
///
/// 策略：结束后进入「静默窗口」，丢弃仍然像协议数据的输出（含 ZPAD/ZDLE
/// 或整段无换行的短二进制），一旦遇到明显是 shell 输出的文本（含换行或
/// 可打印 ASCII 且长度足够）即退出静默、恢复正常透传。
pub struct TailFrameFilter {
    /// 剩余需要检查的字节数（0 = 已退出静默）
    remaining: usize,
}

impl Default for TailFrameFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl TailFrameFilter {
    /// 静默窗口：最多吞 256 字节（足够覆盖收尾帧，不会误伤正常输出）
    const BUDGET: usize = 256;

    pub fn new() -> Self {
        Self { remaining: 0 }
    }

    /// 传输结束后调用：开启静默窗口
    pub fn arm(&mut self) {
        self.remaining = Self::BUDGET;
    }

    pub fn is_armed(&self) -> bool {
        self.remaining > 0
    }

    /// 过滤一段输出：返回应当透传给终端的字节。
    ///
    /// 只丢弃**明确是协议残尾**的块（含 ZDLE 0x18，或纯 ZPAD/hex/换行的帧
    /// 片段）；任何带 shell 特征的内容（提示符 `]# `、字母、空格、方括号）
    /// 立即放行并退出静默。宁可漏掉一个残帧，也不能吞掉提示符。
    pub fn filter(&mut self, data: &[u8]) -> Vec<u8> {
        if self.remaining == 0 {
            return data.to_vec();
        }
        self.remaining = self.remaining.saturating_sub(data.len());
        // 含 ZDLE → 一定是协议帧（ZMODEM 用 0x18 做转义，文本输出不会出现）
        if data.contains(&0x18) {
            return Vec::new();
        }
        // 无 ZDLE 时仅当「整块都是 ZPAD/hex/CR/LF」才视为残帧；但要求长度
        // >= 8 且不含空格、方括号、小写字母等 shell 特征
        let pure_frame = data.len() >= 8
            && data.iter().all(|b| {
                *b == b'*' || *b == b'\r' || *b == b'\n' || b.is_ascii_hexdigit()
            });
        if pure_frame {
            return Vec::new();
        }
        // 其他一律视为正常输出
        self.remaining = 0;
        data.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 实测帧（CentOS lrz 执行 rz 后的真实输出）：
    /// `**` + ZDLE + 'B' + "01"（帧类型 ZRINIT = 服务器准备好接收）+ CRC
    const REAL_RZ_FRAME: &[u8] = b"**\x18B0100000023be50";

    #[test]
    fn real_rz_frame_detected_as_upload() {
        let mut d = ZmodemDetector::new();
        // 终端回显命令 + 服务器帧（实测形态）
        assert_eq!(d.feed(b"[root@CPZX-WH zy]# rz\r\n"), Detection::None);
        match d.feed(REAL_RZ_FRAME) {
            Detection::Rz { prefix } => {
                // prefix 是命中瞬间的 tail（含 `**\x18B01` 判定前缀）
                assert!(prefix.windows(4).any(|w| w == b"**\x18B"), "prefix: {prefix:?}");
            }
            other => panic!("expected Rz (ZRQINIT=upload), got {other:?}"),
        }
    }

    #[test]
    fn sz_zrqinit_frame_detected_as_download() {
        // ZRQINIT = 帧类型 0 → hex "00"（sz 发起：服务器要发文件）
        let mut d = ZmodemDetector::new();
        assert!(matches!(
            d.feed(b"**\x18B0000000023be50"),
            Detection::Sz { .. }
        ));
    }

    #[test]
    fn frame_split_across_chunks_still_detected() {
        let mut d = ZmodemDetector::new();
        assert_eq!(d.feed(b"**\x18B0"), Detection::None); // 帧类型尚未到齐
        assert!(matches!(d.feed(b"100000023be50"), Detection::Rz { .. }));
    }

    #[test]
    fn bin16_zrinit_detected_as_upload() {
        let mut d = ZmodemDetector::new();
        // bin16: `**` + ZDLE + 'A' + 帧类型字节 1（ZRINIT = 上传）
        assert!(matches!(d.feed(b"**\x18A\x01"), Detection::Rz { .. }));
    }

    /// 实测日志（sz 场景）：
    /// `[ZMODEM] session 3 detected upload (24 bytes chunk,
    ///  head="rz\r**\u{18}B000000000")`
    /// lrzsz 的 sz 先输出 `rz\r` 公告，再发 ZRQINIT 帧。必须在公告上
    /// **不**命中，继续等到协议帧才判方向（ZRQINIT=00 → download）。
    #[test]
    fn sz_with_leading_rz_announce_is_download() {
        let mut d = ZmodemDetector::new();
        // 公告 + 帧在同一 chunk（真实形态）
        match d.feed(b"rz\r**\x18B000000000") {
            Detection::Sz { .. } => {}
            other => panic!("expected Sz (ZRQINIT=download), got {other:?}"),
        }
    }

    /// 公告单独到达时不得命中（否则会把随后的 sz 误判为上传）
    #[test]
    fn bare_rz_announce_does_not_trigger() {
        let mut d = ZmodemDetector::new();
        assert_eq!(d.feed(b"rz\r"), Detection::None);
        assert_eq!(d.feed(b"more text\n"), Detection::None);
    }

    #[test]
    fn ignores_plain_text_without_control_byte() {
        let mut d = ZmodemDetector::new();
        assert_eq!(d.feed(b"**B00 some log\n"), Detection::None);
        assert_eq!(d.feed(b"**\x18"), Detection::None);
    }

    #[test]
    fn tail_is_bounded() {
        let mut d = ZmodemDetector::new();
        d.feed(b"0123456789abcdef");
        assert!(d.tail.len() <= MAX_PREFIX);
    }

    #[test]
    fn reset_clears_partial_frame() {
        let mut d = ZmodemDetector::new();
        d.feed(b"**\x18B0");
        d.reset();
        // 复位后残留的半帧不得参与判定；新帧正常识别
        assert_eq!(d.feed(b"junk"), Detection::None);
        assert!(matches!(d.feed(b"**\x18B01"), Detection::Rz { .. }));
    }

    #[test]
    fn hex_helpers() {
        assert_eq!(hex_val(b'0'), Some(0));
        assert_eq!(hex_val(b'f'), Some(15));
        assert_eq!(hex_val(b'F'), Some(15));
        assert_eq!(hex_val(b'z'), None);
        assert_eq!(find_subslice(b"abc**\x18B01", b"**\x18B"), Some(3));
        assert_eq!(find_subslice(b"short", b"**\x18B"), None);
    }
}
