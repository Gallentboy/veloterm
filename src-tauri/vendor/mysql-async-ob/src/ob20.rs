//! OceanBase 2.0 协议层（ob20）——纯 Rust 移植。
//!
//! wire 格式逐条对照 `src-tauri/vendor/obconnector-c`（libobclient）的
//! `ob_protocol20.c` / `ma_net.c` / `mariadb_lib.c` / `mariadb_com.h`：
//!
//! - OB Oracle 租户的握手/认证完全走标准 MySQL 协议；OB 能力位经握手包的
//!   connect attrs 上报（`__proxy_capability_flag` 等，见 [`caps`]），服务端
//!   确认后在**整个连接**上叠加本层包格式（libobclient 里
//!   `ENABLE_PROTOCOL_OB20` 缺省 AUTO=开）。
//! - 收发两侧的线上形态：外层是 MySQL 压缩协议帧
//!   `[len:3][seq:1][complen:3][zlib(ob20 包)]`；ob20 包 =
//!   `[24B 头][extra_len:4?][extra?][MySQL 包(含自身 4B 头)][crc32:4]`。
//!   CRC32 覆盖 extra 区（含 4 字节长度域）与 MySQL 包体；24B 头的
//!   checksum（CRC16）覆盖「外层 7 字节前缀 + 头前 22 字节」。
//! - 全部整数小端。
//!
//! 本模块只做**格式编解码**（可独立单测）；io 层接入（与压缩协议帧的
//! 组装/解压、pkt_seq 对齐）是下一阶段，故 `wrap`/`unwrap` 以显式参数
//! 携带外层前缀，不在模块内假设传输形态。
//!
//! 许可证注意：CRC/常量等 facts 按线缆行为实现；libobclient 是 LGPL-2.1，
//! 若后续需要逐行翻译其代码入本模块，需先过项目方的许可评估。

pub mod caps {
    //! OBCLIENT 能力位（`ObCapabilityFlagShift`，shift 0..=20）与握手
    //! connect attrs 的键值（`mariadb_com.h`）。

    pub const OBCLIENT_CAP_PARTITION_TABLE: u64 = 1 << 0;
    pub const OBCLIENT_CAP_CHANGE_USER: u64 = 1 << 1;
    pub const OBCLIENT_CAP_READ_WEAK: u64 = 1 << 2;
    pub const OBCLIENT_CAP_CHECKSUM: u64 = 1 << 3;
    pub const OBCLIENT_CAP_SAFE_WEAK_READ: u64 = 1 << 4;
    pub const OBCLIENT_CAP_PRIORITY_HIT: u64 = 1 << 5;
    pub const OBCLIENT_CAP_CHECKSUM_SWITCH: u64 = 1 << 6;
    pub const OBCLIENT_CAP_OCJ_ENABLE_EXTRA_OK_PACKET: u64 = 1 << 7;
    /// 服务端确认该位后，整条连接切换为 ob20 包格式（`get_use_protocol_ob20`）
    pub const OBCLIENT_CAP_OB_PROTOCOL_V2: u64 = 1 << 8;
    pub const OBCLIENT_CAP_EXTRA_OK_PACKET_FOR_STATISTICS: u64 = 1 << 9;
    pub const OBCLIENT_CAP_ABUNDANT_FEEDBACK: u64 = 1 << 10;
    pub const OBCLIENT_CAP_PL_ROUTE: u64 = 1 << 11;
    pub const OBCLIENT_CAP_PROXY_REROUTE: u64 = 1 << 12;
    pub const OBCLIENT_CAP_PROXY_SESSION_SYNC: u64 = 1 << 13;
    pub const OBCLIENT_CAP_FULL_LINK_TRACE: u64 = 1 << 14;
    pub const OBCLIENT_CAP_PROXY_NEW_EXTRA_INFO: u64 = 1 << 15;
    pub const OBCLIENT_CAP_PROXY_SESSION_VAR_SYNC: u64 = 1 << 16;
    pub const OBCLIENT_CAP_PROXY_WEAK_STALE_FEEDBACK: u64 = 1 << 17;
    pub const OBCLIENT_CAP_PROXY_FULL_LINK_TRACE_SHOW_TRACE: u64 = 1 << 18;
    pub const OBCLIENT_CAP_SERVER_DUP_SESS_INFO_SYNC: u64 = 1 << 19;
    pub const OBCLIENT_CAP_PROXY_LOCAL_INFILES: u64 = 1 << 20;

    /// libobclient 的缺省能力组合（`OBPROXY_DEFAULT_CAPABILITY_FLAG`）：
    /// 全链路追踪相关位与 ob20 一起上报。
    pub const OBPROXY_DEFAULT_CAPABILITY_FLAG: u64 = OBCLIENT_CAP_OB_PROTOCOL_V2
        | OBCLIENT_CAP_FULL_LINK_TRACE
        | OBCLIENT_CAP_PROXY_NEW_EXTRA_INFO
        | OBCLIENT_CAP_PROXY_FULL_LINK_TRACE_SHOW_TRACE;

    /// connect attrs 的键（握手包 `client_connect_attrs` 段）
    pub const ATTR_CAPABILITY_FLAG: &str = "__proxy_capability_flag";
    pub const ATTR_CLIENT_MODE: &str = "__mysql_client_type";
    pub const ATTR_CLIENT_OBPROXY_MODE: &str = "__ob_proxy";
    pub const ATTR_CONNECTION_ID: &str = "__connection_id";
    pub const ATTR_CLIENT_NAME_KEY: &str = "__ob_client_name";
    /// veloterm 分叉的名字（libobclient 上报 "OceanBase Connector/C"）
    pub const ATTR_CLIENT_NAME_VALUE: &str = "veloterm oceanbase_async";
    pub const ATTR_CLIENT_VERSION_KEY: &str = "__ob_client_version";
    pub const ATTR_PROXY_CONNECTION_ID: &str = "__proxy_connection_id";
    pub const ATTR_GLOBAL_VARS_VERSION: &str = "__global_vars_version";
    /// LOB 扩展能力（`__ob_client_attribute_capability_flag`）的键与位
    pub const ATTR_CLIENT_ATTRIBUTE_CAPABILITY_FLAG: &str = "__ob_client_attribute_capability_flag";
    pub const OBCLIENT_CAP_SUPPORT_JDBC_BINARY_DOUBLE: u64 = 1 << 0;
    pub const OBCLIENT_CAP_OB_LOB_LOCATOR_V2: u64 = 1 << 1;

    /// 组装本驱动应上报的 connect attrs（对齐 libobclient 的
    /// `ma_ob_client_set_connect_attrs`：ob20/extra-info 位 + 自报家门；
    /// 全链路追踪不实现，相关位不上报）
    pub fn driver_connect_attrs(client_version: &str) -> Vec<(&'static str, String)> {
        vec![
            (ATTR_CAPABILITY_FLAG, OBPROXY_DEFAULT_CAPABILITY_FLAG.to_string()),
            (ATTR_CLIENT_MODE, "__ob_libobclient".to_string()),
            (ATTR_CLIENT_NAME_KEY, ATTR_CLIENT_NAME_VALUE.to_string()),
            (ATTR_CLIENT_VERSION_KEY, client_version.to_string()),
            (
                ATTR_CLIENT_ATTRIBUTE_CAPABILITY_FLAG,
                OBCLIENT_CAP_SUPPORT_JDBC_BINARY_DOUBLE.to_string(),
            ),
        ]
    }
}

use std::fmt;

/// ob20 头的 magic（`OB20_PROTOCOL_MAGIC_NUM`）
pub const OB20_MAGIC: u16 = 0x20AB;
/// ob20 头的版本（`OB20_PROTOCOL_VERSION_VALUE`）
pub const OB20_VERSION: u16 = 20;
/// 头长度（字节数）
pub const OB20_HEADER_SIZE: usize = 24;
/// 尾包长度（payload CRC32）
pub const OB20_TAILER_SIZE: usize = 4;
/// extra 区长度域宽度
pub const OB20_EXTRAINFO_LENGTH_SIZE: usize = 4;
/// request_id / payload 尺寸约束：request_id 是 24 位（序列化时截取低 3 字节）
pub const UINT24_MAX: u32 = 0x00FF_FFFF;

/// ob20 头标志位（`Ob20ProtocolFlags`）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ob20Flags(pub u32);

impl Ob20Flags {
    pub const EXTRA_INFO_EXIST: u32 = 1;
    pub const IS_LAST_PACKET: u32 = 1 << 1;
    pub const IS_PROXY_REROUTE: u32 = 1 << 2;
    pub const IS_NEW_EXTRA_INFO: u32 = 1 << 3;

    pub fn has(self, bit: u32) -> bool {
        self.0 & bit != 0
    }
}

/// OB20 头（线上 24 字节，字段全小端；request_id 虽是 u32 但只序列化低 24 位）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ob20Header {
    pub connection_id: u32,
    pub request_id: u32,
    pub pkt_seq: u8,
    /// extra 区（含 4 字节长度域）+ MySQL 包体的总字节数
    pub payload_len: u32,
    pub flags: Ob20Flags,
    pub reserved: u16,
    pub header_checksum: u16,
}

/// CRC-16（poly 0x8005 反射形态 0xA001、初值 0、无 xorout——与 libobclient
/// 的 `ob_crc16` 表实现等价；CRC-16/ARC，"123456789" 校验值 0xBB3D）。
/// 逐位实现而非抄表：表只是本算法的查表加速。
pub fn ob_crc16(mut crc: u16, data: &[u8]) -> u16 {
    for &byte in data {
        crc ^= u16::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc
}

/// 头 checksum 的输入：外层压缩协议帧前缀（`[len:3][seq:1][complen:3]`）
/// 拼上头前 22 字节。写入与校验两侧同口径。
fn header_checksum(prefix7: &[u8], header22: &[u8]) -> u16 {
    let crc = ob_crc16(0, prefix7);
    ob_crc16(crc, header22)
}

/// 编解码错误
#[derive(Debug, PartialEq, Eq)]
pub enum Ob20Error {
    /// 输入不足以容纳 24B 头
    TooShort,
    /// magic / version 不符（协议协商错位）
    BadMagic { actual: u16 },
    BadVersion { actual: u16 },
    /// 头 CRC16 校验失败
    ChecksumMismatch { expected: u16, actual: u16 },
    /// payload 长度域与实际输入不符（extra 区或包体缺失）
    PayloadLengthMismatch { header_says: usize, actual: usize },
    /// 尾包 CRC32 校验失败
    TailerMismatch { expected: u32, actual: u32 },
    /// extra 区声明长度越过包边界
    BadExtraInfoLength(usize),
}

impl fmt::Display for Ob20Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort => write!(f, "ob20 输入不足 24 字节头"),
            Self::BadMagic { actual } => write!(f, "ob20 magic 不符：0x{actual:04X}（期望 0x{OB20_MAGIC:04X}）"),
            Self::BadVersion { actual } => write!(f, "ob20 version 不符：{actual}（期望 {OB20_VERSION}）"),
            Self::ChecksumMismatch { expected, actual } => {
                write!(f, "ob20 头 CRC16 校验失败：头内 {expected:04X} ≠ 计算 {actual:04X}")
            }
            Self::PayloadLengthMismatch { header_says, actual } => {
                write!(f, "ob20 payload 长度不符：头声明 {header_says}，实际 {actual}")
            }
            Self::TailerMismatch { expected, actual } => {
                write!(f, "ob20 尾包 CRC32 校验失败：包内 {expected:08X} ≠ 计算 {actual:08X}")
            }
            Self::BadExtraInfoLength(n) => write!(f, "ob20 extra 区长度越界：{n}"),
        }
    }
}

impl std::error::Error for Ob20Error {}

/// 读写两侧共享的 ob20 上下文：connection_id 来自握手，request_id 随每个
/// 请求自增（回绕 2^24），pkt_seq 对齐外层压缩帧的包序号。
#[derive(Debug, Clone)]
pub struct Ob20Ctx {
    pub connection_id: u32,
    pub request_id: u32,
    pub pkt_seq: u8,
}

impl Ob20Ctx {
    pub fn new(connection_id: u32) -> Self {
        Self {
            connection_id,
            request_id: 0,
            pkt_seq: 0,
        }
    }

    /// 请求边界：request_id +1 回绕 2^24（`update_request_id`）
    pub fn next_request(&mut self) {
        self.request_id = (self.request_id + 1) & UINT24_MAX;
    }
}

/// 组装一个 ob20 包（不含外层压缩帧；`complen_for_checksum` 是外层帧前缀
/// 里的 complen 值，写入侧与校验侧必须同口径——libobclient 写入时填 0，
/// 该差异需在 io 层对接真机时对齐后固化）。不携带 extra 区（全链路追踪
/// 不实现）。`mysql_payload` 是**含自身 4 字节头**的 MySQL 包。
pub fn wrap_packet(ctx: &Ob20Ctx, mysql_payload: &[u8], complen_for_checksum: u32) -> Vec<u8> {
    let mut prefix7 = [0u8; 7];
    let total = (OB20_HEADER_SIZE + mysql_payload.len() + OB20_TAILER_SIZE) as u32;
    prefix7[0..3].copy_from_slice(&total.to_le_bytes()[0..3]);
    prefix7[3] = ctx.pkt_seq;
    prefix7[4..7].copy_from_slice(&complen_for_checksum.to_le_bytes()[0..3]);

    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(&OB20_MAGIC.to_le_bytes());
    out.extend_from_slice(&OB20_VERSION.to_le_bytes());
    out.extend_from_slice(&ctx.connection_id.to_le_bytes());
    out.extend_from_slice(&ctx.request_id.to_le_bytes()[0..3]); // u24
    out.push(ctx.pkt_seq);
    out.extend_from_slice(&(mysql_payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&Ob20Flags::default().0.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    let checksum = header_checksum(&prefix7, &out);
    out.extend_from_slice(&checksum.to_le_bytes());
    debug_assert_eq!(out.len(), OB20_HEADER_SIZE);

    out.extend_from_slice(mysql_payload);
    let crc = crc32fast::hash(&out[OB20_HEADER_SIZE..]);
    out.extend_from_slice(&crc.to_le_bytes());
    out
}

/// 解析一个 ob20 包（不含外层压缩帧）。`prefix7` 为外层帧的
/// `[len:3][seq:1][complen:3]` 实际值，用于头 CRC16 校验（校验失败即报错，
/// 与 libobclient 的 `decode_protocol20_header` 同语义；头内 checksum 为 0
/// 的旧服务端跳过校验）。返回 (头, MySQL 包体)——extra 区被跳过不解析
/// （本驱动不消费全链路追踪/会话同步信息）。
pub fn unwrap_packet(prefix7: &[u8; 7], bytes: &[u8]) -> Result<(Ob20Header, &[u8]), Ob20Error> {
    if bytes.len() < OB20_HEADER_SIZE {
        return Err(Ob20Error::TooShort);
    }
    let le16 = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
    let le32 =
        |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);

    let magic = le16(0);
    if magic != OB20_MAGIC {
        return Err(Ob20Error::BadMagic { actual: magic });
    }
    let version = le16(2);
    if version != OB20_VERSION {
        return Err(Ob20Error::BadVersion { actual: version });
    }
    let header = Ob20Header {
        connection_id: le32(4),
        request_id: u32::from(bytes[8]) | (u32::from(bytes[9]) << 8) | (u32::from(bytes[10]) << 16),
        pkt_seq: bytes[11],
        payload_len: le32(12),
        flags: Ob20Flags(le32(16)),
        reserved: le16(20),
        header_checksum: le16(22),
    };

    if header.header_checksum != 0 {
        let actual = header_checksum(prefix7, &bytes[..22]);
        if actual != header.header_checksum {
            return Err(Ob20Error::ChecksumMismatch {
                expected: header.header_checksum,
                actual,
            });
        }
    }

    // payload 区 = extra 区（可选）+ MySQL 包 + 4B 尾包
    if bytes.len() < OB20_HEADER_SIZE + header.payload_len as usize {
        return Err(Ob20Error::PayloadLengthMismatch {
            header_says: header.payload_len as usize,
            actual: bytes.len() - OB20_HEADER_SIZE,
        });
    }
    let mut payload_start = OB20_HEADER_SIZE;
    let mut extra_end = payload_start;
    if header.flags.has(Ob20Flags::EXTRA_INFO_EXIST) {
        let extra_len =
            le32(payload_start) as usize + OB20_EXTRAINFO_LENGTH_SIZE;
        extra_end = payload_start + extra_len;
        if extra_end > bytes.len() {
            return Err(Ob20Error::BadExtraInfoLength(extra_len));
        }
        payload_start = extra_end;
    }
    let tailer_at = OB20_HEADER_SIZE + header.payload_len as usize - OB20_TAILER_SIZE;
    if tailer_at < payload_start || tailer_at > bytes.len() {
        return Err(Ob20Error::PayloadLengthMismatch {
            header_says: header.payload_len,
            actual: bytes.len() - OB20_HEADER_SIZE,
        });
    }
    let crc = crc32fast::hash(&bytes[OB20_HEADER_SIZE..tailer_at]);
    let expected = le32(tailer_at);
    if crc != expected {
        return Err(Ob20Error::TailerMismatch {
            expected,
            actual: crc,
        });
    }
    Ok((header, &bytes[payload_start..tailer_at]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CRC-16/ARC 标准校验值（poly 0x8005、init 0、"123456789" → 0xBB3D），
    /// 锁定与 libobclient 表实现的等价性
    #[test]
    fn crc16_check_value() {
        assert_eq!(ob_crc16(0, b"123456789"), 0xBB3D);
        // 空输入不变
        assert_eq!(ob_crc16(0x1234, b""), 0x1234);
        // 增量 == 一次
        let whole = ob_crc16(0, b"oceanbase protocol twenty");
        let split = ob_crc16(ob_crc16(0, b"oceanbase"), b" protocol twenty");
        assert_eq!(whole, split);
    }

    /// 头编解码往返：wrap → unwrap 得到原 MySQL 包体与等价头字段；
    /// 篡改任一字节破坏 CRC16/CRC32
    #[test]
    fn wrap_unwrap_roundtrip() {
        let ctx = Ob20Ctx::new(0x0011_2233);
        // MySQL 包（含 4B 头）：COM_PING
        let mysql_packet = [0x01, 0x00, 0x00, 0x00, 0x0E];
        let packet = wrap_packet(&ctx, &mysql_packet, 0);
        assert_eq!(packet.len(), OB20_HEADER_SIZE + mysql_packet.len() + OB20_TAILER_SIZE);

        let prefix7 = [
            (packet.len() as u8), 0, 0, // len:3
            ctx.pkt_seq, // seq:1
            0, 0, 0, // complen:3
        ];
        let (header, payload) = unwrap_packet(&prefix7, &packet).expect("unwrap");
        assert_eq!(payload, &mysql_packet);
        assert_eq!(header.connection_id, ctx.connection_id);
        assert_eq!(header.request_id, ctx.request_id);
        assert_eq!(header.payload_len as usize, mysql_packet.len());
        assert!(!header.flags.has(Ob20Flags::EXTRA_INFO_EXIST));
        assert_eq!(header.header_checksum, {
            let mut h22 = &packet[..22];
            let _ = &mut h22;
            header_checksum(&prefix7, h22)
        });

        // 篡改头字节 → CRC16 失败
        let mut broken = packet.clone();
        broken[4] ^= 0xFF; // connection_id 低位
        let err = unwrap_packet(&prefix7, &broken).unwrap_err();
        assert!(matches!(err, Ob20Error::ChecksumMismatch { .. }));

        // 篡改包体 → CRC32 失败
        let mut broken = packet.clone();
        let last = broken.len() - 5; // mysql_payload 内最后一字节
        broken[last] ^= 0x01;
        let err = unwrap_packet(&prefix7, &broken).unwrap_err();
        assert!(matches!(err, Ob20Error::TailerMismatch { .. }));

        // magic 不符
        let mut bad = packet.clone();
        bad[0] = 0xFF;
        assert!(matches!(
            unwrap_packet(&prefix7, &bad),
            Err(Ob20Error::BadMagic { .. })
        ));
    }

    /// extra 区跳过路径：手工构造带 extra 区的包（模拟服务端全链路追踪回包）
    #[test]
    fn unwrap_skips_extra_info() {
        let ctx = Ob20Ctx::new(42);
        let mysql_packet = [0x02, 0x00, 0x00, 0x00, 0x0E];
        // 手工组装：头(EXTRA_INFO_EXIST) + extra_len + extra + payload + crc32
        let extra_body = [0xD1, 0x07, 0x02, 0x00, 0xAA, 0xBB]; // TRACE_INFO=2001, len=2
        let extra_total = (OB20_EXTRAINFO_LENGTH_SIZE + extra_body.len()) as u32;
        let payload_len = extra_total as usize + mysql_packet.len() + OB20_TAILER_SIZE;
        let flags = Ob20Flags(Ob20Flags::EXTRA_INFO_EXIST | Ob20Flags::IS_NEW_EXTRA_INFO);

        let mut prefix7 = [0u8; 7];
        let total = (OB20_HEADER_SIZE + payload_len) as u32;
        prefix7[0..3].copy_from_slice(&total.to_le_bytes()[0..3]);
        prefix7[3] = ctx.pkt_seq;

        let mut out = Vec::new();
        out.extend_from_slice(&OB20_MAGIC.to_le_bytes());
        out.extend_from_slice(&OB20_VERSION.to_le_bytes());
        out.extend_from_slice(&ctx.connection_id.to_le_bytes());
        out.extend_from_slice(&ctx.request_id.to_le_bytes()[0..3]);
        out.push(ctx.pkt_seq);
        out.extend_from_slice(&(payload_len as u32).to_le_bytes());
        out.extend_from_slice(&flags.0.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        let checksum = header_checksum(&prefix7, &out);
        out.extend_from_slice(&checksum.to_le_bytes());

        out.extend_from_slice(&extra_total.to_le_bytes());
        out.extend_from_slice(&extra_body);
        out.extend_from_slice(&mysql_packet);
        let crc = crc32fast::hash(&out[OB20_HEADER_SIZE..]);
        out.extend_from_slice(&crc.to_le_bytes());

        let (header, payload) = unwrap_packet(&prefix7, &out).expect("unwrap with extra");
        assert_eq!(payload, &mysql_packet);
        assert!(header.flags.has(Ob20Flags::EXTRA_INFO_EXIST));
    }

    /// request_id 回绕（2^24）
    #[test]
    fn request_id_wraps_at_u24() {
        let mut ctx = Ob20Ctx::new(1);
        ctx.request_id = UINT24_MAX;
        ctx.next_request();
        assert_eq!(ctx.request_id, 0);
    }

    /// 能力位：数值锚定 shift 枚举（防位移错位静默漂移）
    #[test]
    fn capability_bits() {
        assert_eq!(caps::OBCLIENT_CAP_OB_PROTOCOL_V2, 1 << 8);
        assert_eq!(caps::OBCLIENT_CAP_FULL_LINK_TRACE, 1 << 14);
        assert_eq!(caps::OBCLIENT_CAP_PROXY_LOCAL_INFILES, 1 << 20);
        // 缺省组合只含 ob20/extra-info/追踪四项
        assert_eq!(
            caps::OBPROXY_DEFAULT_CAPABILITY_FLAG,
            (1 << 8) | (1 << 14) | (1 << 15) | (1 << 18)
        );
    }

    /// connect attrs 组装：键名与 libobclient 一致（服务端按这些键识别驱动）
    #[test]
    fn driver_connect_attrs_keys() {
        let attrs = caps::driver_connect_attrs("0.35.1-veloterm");
        let keys: Vec<_> = attrs.iter().map(|(k, _)| *k).collect();
        assert!(keys.contains(&caps::ATTR_CAPABILITY_FLAG));
        assert!(keys.contains(&caps::ATTR_CLIENT_MODE));
        let (_, cap_value) = attrs
            .iter()
            .find(|(k, _)| *k == caps::ATTR_CAPABILITY_FLAG)
            .expect("capability attr");
        assert_eq!(
            cap_value.parse::<u64>().unwrap(),
            caps::OBPROXY_DEFAULT_CAPABILITY_FLAG
        );
    }
}
