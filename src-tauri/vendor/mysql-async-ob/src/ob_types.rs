//! OB Oracle 扩展类型的客户端解码与语义解析（veloterm 分叉新增）。
//! 参照 Java JDBC（oceanbase-client 2.4.19 反编译源）：
//! - N 列（NVARCHAR2/NCHAR，类型码 207/208）：`AbstractConnectProtocol::
//!   sendCharsetVariables` 在 NLS_NCHAR_CHARACTERSET 含 UTF16 时置
//!   `NCHARACTER_SET_CONNECTION='UTF16'` 且随后把客户端 nCharacterEncoding
//!   设为 "UTF-16BE"——即 wire 上 N 列是**大端 UTF-16 字节**，解码按列
//!   定义包的字符集编号分派（本驱动连接初始化已同款置 UTF16）。
//! - INTERVAL（YEAR TO MONTH / DAY TO SECOND，类型码 204/205）：
//!   `ColumnType.java` 按 String 语义映射；此处提供结构化拆解供校验与展示。

/// 大端 UTF-16 字符集编号：ucs2（35/90）与 utf16（54/55/101）族
///（MySQL collation id，编号表对齐 mysql_common collations.rs）
pub const UTF16BE_CHARSET_IDS: &[u16] = &[35, 90, 54, 55, 101];
/// 小端 UTF-16 编号（utf16le 56/62）——N 列理论上不会出现，防御性收录
pub const UTF16LE_CHARSET_IDS: &[u16] = &[56, 62];

/// 按列字符集编号解码文本协议的字节值：双字节族走对应端序，其余按 UTF-8。
/// 奇数字节长度不是合法 UTF-16，回落 UTF-8 lossy（乱码可见、可诊断）。
#[inline]
pub fn decode_nchar_bytes(charset_id: u16, bytes: &[u8]) -> String {
    let try_utf16 = |little: bool| -> Option<String> {
        if bytes.len() % 2 != 0 {
            return None;
        }
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| {
                if little {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        String::from_utf16(&units).ok()
    };
    if UTF16BE_CHARSET_IDS.contains(&charset_id) {
        if let Some(s) = try_utf16(false) {
            return s;
        }
    } else if UTF16LE_CHARSET_IDS.contains(&charset_id) {
        if let Some(s) = try_utf16(true) {
            return s;
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// 就地解码一行值里的 N 列（字符集编号驱动——语义诚实的服务器可用，
/// 兑底形态；真机实测 OB 虚报 utf8mb4 时用 decode_row_by_flags）。
#[inline]
pub fn decode_row_nchar(charsets: &[u16], values: &mut [crate::Value]) {
    for (cs, val) in charsets.iter().zip(values.iter_mut()) {
        let utf16 = UTF16BE_CHARSET_IDS.contains(cs) || UTF16LE_CHARSET_IDS.contains(cs);
        if !utf16 {
            continue;
        }
        if let crate::Value::Bytes(b) = val {
            *val = crate::Value::Bytes(decode_nchar_bytes(*cs, b).into());
        }
    }
}

/// 按列级 N 类型标记就地解码（真机实测修正：OB 把 NVARCHAR2/NCHAR 列的
/// 字符集编号虚报为 utf8mb4（cs=46，与 VARCHAR2 同号）但字节是大端
/// UTF-16——字符集编号不可依据；JDBC 按 typeOracleMap 的类型码 207/208
/// 分派 nCharacterEncoding=UTF-16BE 解码，本函数同语义，标记由
/// read_column_defs 重映射时从原始类型字节记录）。非 N 列不动。
#[inline]
pub fn decode_row_by_flags(flags: &[bool], values: &mut [crate::Value]) {
    for (is_n, val) in flags.iter().zip(values.iter_mut()) {
        if !*is_n {
            continue;
        }
        if let crate::Value::Bytes(b) = val {
            // 54 = utf16_general_ci，即大端解码；奇数字节/非法序列时
            // decode_nchar_bytes 内部回落 lossy UTF-8（可诊断）
            *val = crate::Value::Bytes(decode_nchar_bytes(54, b).into());
        }
    }
}

/// INTERVAL 值的结构化拆解（JDBC 语义为 String，此解析供校验与展示）
#[derive(Debug, PartialEq, Eq)]
pub struct ObInterval {
    pub negative: bool,
    /// YEAR TO MONTH 的年、月；DAY TO SECOND 形态下恒 0
    pub years: i64,
    pub months: i64,
    /// DAY TO SECOND 的日、时、分、秒、微秒；YEAR TO MONTH 形态下恒 0
    pub days: i64,
    pub hours: i64,
    pub minutes: i64,
    pub seconds: i64,
    pub micros: i64,
}

impl ObInterval {
    fn zero(negative: bool) -> Self {
        Self {
            negative,
            years: 0,
            months: 0,
            days: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
            micros: 0,
        }
    }
}

/// OB 在**文本协议**里对 INTERVAL 回传的其实是结构化二进制（真机 hex
/// 实证，编码对齐 Java `DataTypeUtilities.intervalYmToBytes/
/// intervalDsToBytes/intervalDsToString`）：
/// - YEAR TO MONTH（7B）：`[is_negative 0/1][years LE u32][months u8]
///   [yearStr.length u8]`——例 `00 01 00 00 00 02 03` = +001-02（YEAR(3)）
/// - DAY TO SECOND（14B）：`[is_negative][days LE u32][时][分][秒]
///   [fraction LE u32（9 位定点）][dayStr.length][scale]`
///   例 `00 02 00 00 00 03 04 05 70 17 00 00 03 06` = +002 03:04:05.000006
///   （fraction=6000、scale=6：真实小数 = 6000/10^3 = 6µs；JDBC 原文
///   fractional_second / 10^(9-scale)——不是纳秒，scale≠6 时直接除 1000
///   会错）
pub fn parse_ob_interval_binary(bytes: &[u8]) -> Option<ObInterval> {
    let le32 = |o: usize| -> Option<u32> {
        let b = bytes.get(o..o + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let negative = bytes.first().is_some_and(|b| *b != 0);
    let mut out = ObInterval::zero(negative);
    if bytes.len() == 7 {
        // YEAR TO MONTH（JDBC INTERVALYM_BYTE_SIZE = 7，严格长度）；
        // 尾字节是 yearStr.length 元数据，忽略
        out.years = le32(1)? as i64;
        out.months = *bytes.get(5)? as i64;
        Some(out)
    } else if bytes.len() == 14 {
        // DAY TO SECOND（JDBC intervalDsToString 强制 14）；scale 取尾字节，
        // 真实小数秒（微秒）= fraction / 10^(9-scale)
        out.days = le32(1)? as i64;
        out.hours = *bytes.get(5)? as i64;
        out.minutes = *bytes.get(6)? as i64;
        out.seconds = *bytes.get(7)? as i64;
        let fraction = le32(8)? as i64;
        let scale = (*bytes.get(13)? as i32).clamp(0, 9);
        out.micros = fraction / 10_i64.pow((9 - scale) as u32);
        Some(out)
    } else {
        None
    }
}

/// INTERVAL 的 wire 解析总入口：字节首字节非 ASCII 符号/数字即二进制形态
///（真机实证 OB 文本协议发二进制），否则按文本形态解析（lossy）。
/// JDBC 的 ColumnType 对 204/205 映射 String，但实测字节是结构化二进制
/// ——本解析器以真机字节为准。
pub fn parse_ob_interval_wire(bytes: &[u8]) -> Option<ObInterval> {
    let looks_binary = !bytes
        .first()
        .is_some_and(|b| b.is_ascii_digit() || *b == b'+' || *b == b'-');
    if looks_binary {
        parse_ob_interval_binary(bytes)
    } else {
        parse_ob_interval(&String::from_utf8_lossy(bytes))
    }
}

/// 解析 OB 文本协议回传的 INTERVAL 值（真机回传形态实证后锁定）：
/// - YEAR TO MONTH：`[-]YY[-MM]`，如 `+001-02`、`-03-04`
/// - DAY TO SECOND：`[-]DD HH:MM:SS[.FFFFFF]`，如 `+002 03:04:05.000006`
pub fn parse_ob_interval(input: &str) -> Option<ObInterval> {
    let s = input.trim();
    let (negative, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let mut out = ObInterval::zero(negative);

    let (hms_parts, frac) = if let Some((day_part, time_part)) = body.split_once(' ') {
        // DAY TO SECOND："002 03:04:05.000006"
        out.days = day_part.parse().ok()?;
        split_hms(time_part)?
    } else if body.contains(':') {
        // 无日段的纯时间形态（防御性支持）
        split_hms(body)?
    } else {
        // YEAR TO MONTH："001-02"
        let segs: Vec<&str> = body.split('-').collect();
        match segs.as_slice() {
            [y] => out.years = y.parse().ok()?,
            [y, m] => {
                out.years = y.parse().ok()?;
                out.months = m.parse().ok()?;
            }
            _ => return None,
        }
        return Some(out);
    };

    out.hours = hms_parts.first()?.parse().ok()?;
    out.minutes = hms_parts.get(1)?.parse().ok()?;
    out.seconds = hms_parts.get(2)?.parse().ok()?;
    if !frac.is_empty() {
        // 微秒域右补零到 6 位（".5" = 500000us）；超 6 位截断
        let padded: String = frac.chars().chain(std::iter::repeat('0')).take(6).collect();
        out.micros = padded.parse().ok()?;
    }
    Some(out)
}

/// 拆 `HH:MM:SS[.FFFFFF]` → (三段, 小数秒)
fn split_hms(time: &str) -> Option<(Vec<&str>, &str)> {
    let (hms, frac) = match time.split_once('.') {
        Some((a, b)) => (a, b),
        None => (time, ""),
    };
    let parts: Vec<&str> = hms.split(':').collect();
    if parts.len() == 3 {
        Some((parts, frac))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// N 列解码：UTF-16BE 族按大端解（JDBC nCharacterEncoding="UTF-16BE"）
    #[test]
    fn decode_utf16be_columns() {
        let text = "NVARCHAR中文字符串";
        let bytes: Vec<u8> = text
            .encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect();
        assert_eq!(decode_nchar_bytes(54, &bytes), text);
        assert_eq!(decode_nchar_bytes(55, &bytes), text);
        // LE 族按小端解
        let le: Vec<u8> = text
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(decode_nchar_bytes(56, &le), text);
        // 常规列（utf8mb4 等）原样
        assert_eq!(decode_nchar_bytes(45, "普通UTF8列".as_bytes()), "普通UTF8列");
        // 奇数字节 + 双字节编号 → 回落 lossy UTF-8（可诊断乱码）
        assert_eq!(decode_nchar_bytes(54, &[0xe4, 0xb8, 0xad]), "中");
    }

    /// 行级就地解码（类型标记驱动——真机实测 cs 虚报 utf8mb4 不可依据）
    #[test]
    fn decode_row_by_flags_inplace() {
        let text = "NVARCHAR中文字符串";
        let be: Vec<u8> = text
            .encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect();
        let mut values = vec![
            crate::Value::Bytes(be.into()),
            crate::Value::Bytes("普通列".as_bytes().to_vec().into()),
            crate::Value::Int(42),
        ];
        decode_row_by_flags(&[true, false, false], &mut values);
        match values.first() {
            Some(crate::Value::Bytes(b)) => {
                assert_eq!(String::from_utf8_lossy(b), text)
            }
            _ => panic!("形态不符"),
        }
        match values.get(1) {
            Some(crate::Value::Bytes(b)) => {
                assert_eq!(String::from_utf8_lossy(b), "普通列")
            }
            _ => panic!("形态不符"),
        }
    }

    /// 行级就地解码（字符集编号驱动兑底）
    #[test]
    fn decode_row_nchar_by_charset_inplace() {
        let be: Vec<u8> = "N列".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        let mut values = vec![crate::Value::Bytes(be.into())];
        decode_row_nchar(&[54], &mut values);
        match values.first() {
            Some(crate::Value::Bytes(b)) => assert_eq!(String::from_utf8_lossy(b), "N列"),
            _ => panic!("形态不符"),
        }
    }

    /// INTERVAL 语义解析：真机回传形态的期望值（+001-02 / +002 03:04:05.000006）
    #[test]
    fn parse_interval_forms() {
        let ym = parse_ob_interval("+001-02").unwrap();
        assert_eq!(
            (ym.negative, ym.years, ym.months, ym.days),
            (false, 1, 2, 0)
        );
        let ds = parse_ob_interval("+002 03:04:05.000006").unwrap();
        assert_eq!(
            (ds.days, ds.hours, ds.minutes, ds.seconds, ds.micros),
            (2, 3, 4, 5, 6)
        );
        let neg = parse_ob_interval("-03-04").unwrap();
        assert!(neg.negative && neg.years == 3 && neg.months == 4);
        // 无符号、无小数、短小数
        let plain = parse_ob_interval("1 02:03:04").unwrap();
        assert_eq!((plain.days, plain.seconds, plain.micros), (1, 4, 0));
        let short_frac = parse_ob_interval("+0 00:00:01.5").unwrap();
        assert_eq!(short_frac.micros, 500_000);
        // 非法输入
        assert!(parse_ob_interval("abc").is_none());
        assert!(parse_ob_interval("1-2-3-4").is_none());
        assert!(parse_ob_interval("+1 02:03").is_none());
    }

    /// INTERVAL 二进制 wire 形态（真机 hex fixture 锁定）：
    /// `00 01 00 00 00 02 03` = +001-02（YEAR(3)）、
    /// `00 02 00 00 00 03 04 05 70 17 00 00 03 06` = +002 03:04:05.000006
    #[test]
    fn parse_interval_binary_wire() {
        let ym = parse_ob_interval_binary(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x03]).unwrap();
        assert_eq!((ym.negative, ym.years, ym.months, ym.days), (false, 1, 2, 0));
        let ds = parse_ob_interval_binary(&[
            0x00, 0x02, 0x00, 0x00, 0x00, 0x03, 0x04, 0x05, 0x70, 0x17, 0x00, 0x00, 0x03, 0x06,
        ])
        .unwrap();
        assert_eq!(
            (ds.negative, ds.days, ds.hours, ds.minutes, ds.seconds, ds.micros),
            (false, 2, 3, 4, 5, 6)
        );
        // 负值（is_negative 0/1，JDBC 同款）与 scale≠6 的换算（9 位定点）
        let neg = parse_ob_interval_binary(&[0x01, 0x01, 0x00, 0x00, 0x00, 0x02, 0x03]).unwrap();
        assert!(neg.negative && neg.years == 1 && neg.months == 2);
        // SECOND(3)：0.006 秒 → fraction = 6_000_000（9 位定点，0x5B8D80），scale=3
        let ds3 = parse_ob_interval_binary(&[
            0x00, 0x02, 0x00, 0x00, 0x00, 0x03, 0x04, 0x05, 0x80, 0x8d, 0x5b, 0x00, 0x03, 0x03,
        ])
        .unwrap();
        assert_eq!(ds3.micros, 6, "scale=3 时按 fraction/10^6 换算，不能固定除 1000");
        // wire 总入口的字节/文本分派
        assert_eq!(
            parse_ob_interval_wire(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x03])
                .map(|v| (v.years, v.months)),
            Some((1, 2))
        );
        assert_eq!(
            parse_ob_interval_wire(b"+001-02").map(|v| (v.years, v.months)),
            Some((1, 2))
        );
        assert!(parse_ob_interval_binary(&[0x00, 0x01]).is_none());
    }
}
