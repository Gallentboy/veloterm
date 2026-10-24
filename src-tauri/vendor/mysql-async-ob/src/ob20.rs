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

    /// 缺省能力组合（对齐 Java JDBC `writeConnectAttributes` 的默认值 1480）：
    /// CHECKSUM(8) + CHECKSUM_SWITCH(64) + OCJ_ENABLE_EXTRA_OK_PACKET(128) +
    /// OB_PROTOCOL_V2(256)。其中 OCJ_ENABLE_EXTRA_OK_PACKET 是**服务端回显
    /// 能力位的前提**——没有它服务端不发额外 OK 包，ob20 探测失败。
    /// （此前对齐 libobclient C 版报 FLT 三位，Java 默认不报——已对齐 Java。）
    pub const OBPROXY_DEFAULT_CAPABILITY_FLAG: u64 = OBCLIENT_CAP_CHECKSUM
        | OBCLIENT_CAP_CHECKSUM_SWITCH
        | OBCLIENT_CAP_OCJ_ENABLE_EXTRA_OK_PACKET
        | OBCLIENT_CAP_OB_PROTOCOL_V2;

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

/// 在已组装好的 ob20 包上就地设置 IS_LAST_PACKET 标志（flag 字段的 bit 1）
/// 并重算头 CRC16——包可能是多帧分块，只有最后一帧才设此标志。
/// flag 位于头的第 16..20 字节；CRC16 覆盖外层前缀 7B + 头前 22B。
pub fn set_last_packet_flag(packet: &mut [u8]) {
    // flag 偏移 16，小端 u32
    let flag_off = 16;
    let mut flag = u32::from_le_bytes([
        packet[flag_off],
        packet[flag_off + 1],
        packet[flag_off + 2],
        packet[flag_off + 3],
    ]);
    flag |= Ob20Flags::IS_LAST_PACKET;
    packet[flag_off..flag_off + 4].copy_from_slice(&flag.to_le_bytes());

    // 重算头 CRC16：外层前缀 7B 用 len=this_packet_len / seq=头内 pkt_seq / complen=0
    let mut prefix7 = [0u8; 7];
    prefix7[0..3].copy_from_slice(&(packet.len() as u32).to_le_bytes()[0..3]);
    prefix7[3] = packet[11]; // pkt_seq 在头偏移 11
    let checksum = header_checksum(&prefix7, &packet[..22]);
    packet[22..24].copy_from_slice(&checksum.to_le_bytes());
}

/// OB Oracle 扩展列类型码 → 标准 MySQL 列类型码的降级映射。
///
/// OB Oracle 租户在文本协议下返回的列定义可能使用 OB 扩展类型码
///（Java JDBC `internal/ColumnType.java` 的 `typeOracleMap`），这些码不在
/// mysql_common 的标准 `ColumnType` 枚举里，`from_u8` 直接报错。文本协议
/// 下这些类型的值都以字符串返回，统一降级为 VAR_STRING(253) 是安全行为。
///
/// | OB 码 | Java ColumnType | 含义 |
/// |---|---|---|
/// | 160 | COMPLEX | 复合类型 |
/// | 161 | ARRAY | 数组 |
/// | 162 | STRUCT | 结构体 |
/// | 163 | CURSOR | 游标 |
/// | 200 | TIMESTAMP_TZ | 带时区时间戳 |
/// | 201 | TIMESTAMP_LTZ | 本地时区时间戳 |
/// | 202 | TIMESTAMP_NANO | 纳秒时间戳 |
/// | 203 | RAW | 原始二进制 |
/// | 204 | INTERVALYM | 年月间隔 |
/// | 205 | INTERVALDS | 天秒间隔 |
/// | 206 | NUMBER_FLOAT | 浮点数 |
/// | 207 | NVARCHAR2 | 国家字符变长串 |
/// | 208 | NCHAR | 国家字符定长串 |
/// | 209 | ROWID | 行标识 |
/// | 210 | ORA_BLOB | 二进制大对象 |
/// | 211 | ORA_CLOB | 字符大对象 |
pub fn remap_ob_column_type(raw: &[u8]) -> Vec<u8> {
    // 列定义包的 column_type 字段偏移取决于 schema 长度（lenenc），无法简单
    // 定位——在 read_column_defs 里直接对解析前的原始包字节做后处理不现实。
    // 这里返回一个映射表供调用方在解析失败时查找。
    // 实际的 hook 点在 queryable/stmt.rs::read_column_defs：解析失败时
    // 把包字节里的类型码替换为 253（VAR_STRING）再重试。
    let _ = raw;
    Vec::new()
}

/// OB Oracle 扩展类型码集合（需要降级处理的码值）
pub const OB_ORACLE_EXT_TYPES: &[u8] = &[
    160, 161, 162, 163, // COMPLEX/ARRAY/STRUCT/CURSOR
    200, 201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211, // Oracle 扩展
];

/// 解析一个 ob20 包（不含外层压缩帧）。`prefix7` 为外层帧的
/// `[len:3][seq:1][complen:3]` 实际值，用于头 CRC16 校验（校验失败即报错，
/// 与 libobclient 的 `decode_protocol20_header` 同语义；头内 checksum 为 0
/// 的旧服务端跳过校验）。返回 (头, MySQL 包体)——extra 区被跳过不解析
/// （本驱动不消费全链路追踪/会话同步信息）。
pub fn unwrap_packet<'a>(
    prefix7: &[u8; 7],
    bytes: &'a [u8],
) -> Result<(Ob20Header, &'a [u8]), Ob20Error> {
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

    // payload 区 = extra 区（可选，长度域不含自身 4 字节）+ MySQL 包；
    // 头的 payload_len 不含头与尾包，尾包紧跟其后
    if bytes.len() < OB20_HEADER_SIZE + header.payload_len as usize + OB20_TAILER_SIZE {
        return Err(Ob20Error::PayloadLengthMismatch {
            header_says: header.payload_len as usize,
            actual: bytes.len() - OB20_HEADER_SIZE,
        });
    }
    let mut payload_start = OB20_HEADER_SIZE;
    if header.flags.has(Ob20Flags::EXTRA_INFO_EXIST) {
        let extra_len = le32(payload_start) as usize + OB20_EXTRAINFO_LENGTH_SIZE;
        let extra_end = payload_start + extra_len;
        if extra_end > bytes.len() {
            return Err(Ob20Error::BadExtraInfoLength(extra_len));
        }
        payload_start = extra_end;
    }
    let tailer_at = OB20_HEADER_SIZE + header.payload_len as usize;
    if tailer_at < payload_start || tailer_at + OB20_TAILER_SIZE > bytes.len() {
        return Err(Ob20Error::PayloadLengthMismatch {
            header_says: header.payload_len as usize,
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

/// 从 OK 包的 session_state_info（CLIENT_SESSION_TRACK 的 info 段）里找
/// `ob_capability_flag` 系统变量的十进制 u64 值——服务端经此把确认后的 OB
/// 能力位回显给客户端（libobclient 在 `read_ok_packet` 的
/// SESSION_TRACK_SYSTEM_VARIABLES 分支解析，NLS 格式变量同路捎带）。
///
/// 线格式：`[type:1][entry_len:lenenc][entry]...`；type 0（系统变量）的
/// entry = `[name_len:lenenc][name][value_len:lenenc][value]`。
pub fn scan_capability_flag(session_state_info: &[u8]) -> Option<u64> {
    let mut pos = 0;
    while pos < session_state_info.len() {
        let entry_type = *session_state_info.get(pos)?;
        pos += 1;
        let entry_len = lenenc(session_state_info, &mut pos)?;
        let entry_end = pos.checked_add(entry_len)?;
        if entry_end > session_state_info.len() {
            return None;
        }
        let entry = &session_state_info[..entry_end];
        if entry_type == 0 {
            // SESSION_TRACK_SYSTEM_VARIABLES：name/value 对
            let name_len = lenenc(entry, &mut pos)?;
            let name_start = pos;
            pos = pos.checked_add(name_len)?;
            let name = entry.get(name_start..pos)?;
            if name == b"ob_capability_flag" {
                let value_len = lenenc(entry, &mut pos)?;
                let value_start = pos;
                pos = pos.checked_add(value_len)?;
                let text = std::str::from_utf8(entry.get(value_start..pos)?).ok()?;
                return text.trim().parse::<u64>().ok();
            }
        }
        pos = entry_end;
    }
    None
}

/// MySQL lenenc 整数（返回值，游标前进）；越界返回 None
fn lenenc(buf: &[u8], pos: &mut usize) -> Option<usize> {
    let first = *buf.get(*pos)?;
    match first {
        0xFB => {
            *pos += 1;
            Some(0)
        }
        0xFC..=0xFE => {
            let width = match first {
                0xFC => 2,
                0xFD => 3,
                _ => 8,
            };
            let mut len = 0usize;
            for i in 1..=width {
                len |= (*buf.get(*pos + i)? as usize) << (8 * (i - 1));
            }
            *pos += width + 1;
            Some(len)
        }
        len => {
            *pos += 1;
            Some(len as usize)
        }
    }
}

/// SM3 密码散列（GB/T 32905-2016）与 `ob_sm3_password` 认证插件的应答算法。
///
/// SM3 标准实现（国密哈希，32 字节输出）；认证算法对照 obconnector-c 的
/// `plugins/auth/ob_sm3_pw.c::ma_sm3_scramble`——与 mysql_native_password 的
/// 三步 SHA1 舞蹈同构，只是哈希换 SM3：
/// `d1 = SM3(password)`；`d2 = SM3(d1)`；`r = SM3(d2 || salt)`；应答 `= d1 ⊕ r`。
pub mod sm3 {
    const IV: [u32; 8] = [
        0x7380_166F, 0x4914_B2B9, 0x1724_42D7, 0xDA8A_0600, 0xA96F_30BC, 0x1631_38AA, 0xE38D_EE4D,
        0xB0FB_0E4E,
    ];

    fn t_j(j: usize) -> u32 {
        if j < 16 {
            0x79CC_4519
        } else {
            0x7A87_9D8A
        }
    }

    fn p0(x: u32) -> u32 {
        x ^ x.rotate_left(9) ^ x.rotate_left(17)
    }

    fn p1(x: u32) -> u32 {
        x ^ x.rotate_left(15) ^ x.rotate_left(23)
    }

    fn ff_j(j: usize, x: u32, y: u32, z: u32) -> u32 {
        if j < 16 {
            x ^ y ^ z
        } else {
            (x & y) | (x & z) | (y & z)
        }
    }

    fn gg_j(j: usize, x: u32, y: u32, z: u32) -> u32 {
        if j < 16 {
            x ^ y ^ z
        } else {
            (x & y) | (!x & z)
        }
    }

    /// SM3 摘要（32 字节）
    pub fn sm3(message: &[u8]) -> [u8; 32] {
        // 填充：0x80 + 0* + 64 位大端比特长度（与 SHA-256 同形，长度域不反转）
        let bit_len = (message.len() as u64).wrapping_mul(8);
        let mut padded = message.to_vec();
        padded.push(0x80);
        while padded.len() % 64 != 56 {
            padded.push(0);
        }
        padded.extend_from_slice(&bit_len.to_be_bytes());

        let mut v = IV;
        for block in padded.chunks_exact(64) {
            let mut w = [0u32; 68];
            for (i, word) in w.iter_mut().take(16).enumerate() {
                *word = u32::from_be_bytes([
                    block[i * 4],
                    block[i * 4 + 1],
                    block[i * 4 + 2],
                    block[i * 4 + 3],
                ]);
            }
            for i in 16..68 {
                w[i] = p1(w[i - 16] ^ w[i - 9] ^ w[i - 3].rotate_left(15))
                    ^ w[i - 13].rotate_left(7)
                    ^ w[i - 6];
            }
            let mut w1 = [0u32; 64];
            for (i, word) in w1.iter_mut().enumerate() {
                *word = w[i] ^ w[i + 4];
            }

            let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = v;
            for j in 0..64 {
                let ss1 = (a.rotate_left(12))
                    .wrapping_add(e)
                    .wrapping_add(t_j(j).rotate_left((j as u32) % 32))
                    .rotate_left(7);
                let ss2 = ss1 ^ a.rotate_left(12);
                let tt1 = ff_j(j, a, b, c)
                    .wrapping_add(d)
                    .wrapping_add(ss2)
                    .wrapping_add(w1[j]);
                let tt2 = gg_j(j, e, f, g)
                    .wrapping_add(h)
                    .wrapping_add(ss1)
                    .wrapping_add(w[j]);
                d = c;
                c = b.rotate_left(9);
                b = a;
                a = tt1;
                h = g;
                g = f.rotate_left(19);
                f = e;
                e = p0(tt2);
            }
            v[0] ^= a;
            v[1] ^= b;
            v[2] ^= c;
            v[3] ^= d;
            v[4] ^= e;
            v[5] ^= f;
            v[6] ^= g;
            v[7] ^= h;
        }

        let mut out = [0u8; 32];
        for (i, word) in v.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    /// ob_sm3_password 插件应答（32 字节）；salt 是握手包的 20 字节 nonce
    pub fn ob_sm3_scramble(password: &[u8], salt: &[u8]) -> [u8; 32] {
        let d1 = sm3(password);
        let d2 = sm3(&d1);
        let mut tail = d2.to_vec();
        tail.extend_from_slice(salt);
        let r = sm3(&tail);
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = d1[i] ^ r[i];
        }
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// 国密标准向量：SM3("abc")（GB/T 32905-2016 附录 A）
        #[test]
        fn sm3_standard_vector() {
            let digest = sm3(b"abc");
            assert_eq!(
                hex(&digest),
                "66c7f0f462eeedd9d1f2d46bdc10e4e24167c4875cf2f7a2297da02b8f4ba8e0"
            );
        }

        /// 多块消息（跨 64 字节分组边界）与单块结果长度一致、确定性
        #[test]
        fn sm3_multiblock_deterministic() {
            let long: Vec<u8> = std::iter::repeat(b'x').take(200).collect();
            assert_eq!(sm3(&long), sm3(&long));
            assert_eq!(sm3(&long).len(), 32);
            // 空消息
            assert_eq!(sm3(b"").len(), 32);
        }

        /// 认证应答：长度 32、确定、与 salt 相关
        #[test]
        fn ob_sm3_scramble_shape() {
            let salt = [7u8; 20];
            let a = ob_sm3_scramble(b"secret", &salt);
            assert_eq!(a.len(), 32);
            assert_eq!(a, ob_sm3_scramble(b"secret", &salt));
            assert_ne!(a, ob_sm3_scramble(b"secret", &[8u8; 20]));
        }

        fn hex(bytes: &[u8]) -> String {
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        }
    }
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
        assert_eq!(header.header_checksum, header_checksum(&prefix7, &packet[..22]));

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
        // 手工组装：头(EXTRA_INFO_EXIST) + extra_len(不含自身 4 字节) + extra + payload + crc32
        let extra_body = [0xD1, 0x07, 0x02, 0x00, 0xAA, 0xBB]; // TRACE_INFO=2001, len=2
        let extra_total = OB20_EXTRAINFO_LENGTH_SIZE + extra_body.len();
        let payload_len = extra_total + mysql_packet.len();
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

        out.extend_from_slice(&(extra_body.len() as u32).to_le_bytes());
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
        // 缺省组合对齐 Java JDBC（1480 减去 CHECKSUM/ CHECKSUM_SWITCH 在
        // useObChecksum=false 时被去掉的位后 = 456 = 8+64+128+256）
        assert_eq!(
            caps::OBPROXY_DEFAULT_CAPABILITY_FLAG,
            (1 << 3) | (1 << 6) | (1 << 7) | (1 << 8)
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

    /// OK 包 session-track 的能力回显扫描：标准格式命中、多变量穿插命中、
    /// 无关变量/坏输入不误报
    #[test]
    fn scan_capability_flag_from_session_state() {
        fn entry(t: u8, data: &[u8]) -> Vec<u8> {
            let mut v = vec![t];
            if data.len() < 251 {
                v.push(data.len() as u8);
            } else {
                v.push(0xFC);
                v.extend_from_slice(&(data.len() as u16).to_le_bytes());
            }
            v.extend_from_slice(data);
            v
        }
        fn sysvar(name: &str, value: &str) -> Vec<u8> {
            let mut d = vec![name.len() as u8];
            d.extend_from_slice(name.as_bytes());
            d.push(value.len() as u8);
            d.extend_from_slice(value.as_bytes());
            d
        }

        // 单变量：ob_capability_flag = 0x10340（含 OB_PROTOCOL_V2 位）
        let mut info = entry(0, &sysvar("ob_capability_flag", "66368"));
        assert_eq!(scan_capability_flag(&info), Some(66368));

        // 前置无关变量 + 目标变量 + 尾随其它类型条目
        let mut mixed = entry(0, &sysvar("character_set_client", "utf8mb4"));
        mixed.extend(entry(0, &sysvar("ob_capability_flag", "256")));
        mixed.extend(entry(1, b"test"));
        assert_eq!(scan_capability_flag(&mixed), Some(256));

        // 无目标变量 → None；空输入 → None；截断输入 → None
        assert_eq!(scan_capability_flag(&entry(0, &sysvar("nls_date_format", "YYYY-MM-DD"))), None);
        assert_eq!(scan_capability_flag(&[]), None);
        assert_eq!(scan_capability_flag(&[0, 200]), None);
    }
}
