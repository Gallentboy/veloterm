//! 版本字符串检测 + 最小版本范围解析（`[4.0,5.0)` 取满足范围的最高版本）。
//!
//! 调解按 (depth, seq) 不比较版本；比较仅用于范围选择与 LATEST/RELEASE，
//! 因此这里用简化版 Maven 语义（数字段数值比较 + qualifier 档位），
//! 覆盖 `4.2.3 < 5.0`、`1.0-SNAPSHOT < 1.0`、`1.0-alpha < 1.0` 等常见形态。

/// SNAPSHOT 版本（对 baseVersion 而言，`-SNAPSHOT` 结尾）。
#[inline]
pub fn is_snapshot(version: &str) -> bool {
    version.ends_with("-SNAPSHOT")
}

/// 版本范围（`[1.0,2.0)`、`(,1.0]` 等以 `[` 或 `(` 开头）
/// 或元版本（LATEST/RELEASE）。
#[inline]
pub fn is_range_or_meta(version: &str) -> bool {
    let v = version.trim();
    v.starts_with('[')
        || v.starts_with('(')
        || v.eq_ignore_ascii_case("LATEST")
        || v.eq_ignore_ascii_case("RELEASE")
}

// ---------- 简化版本比较（仅用于范围/LATEST 选版） ----------

/// qualifier 档位：alpha/beta/milestone/rc/cr/snapshot < 无后缀(release) < sp < 其他未知（字典序组内比）。
/// 支持限定词后接数字（alpha1 / rc2），数字后缀零填充 4 位保证字典序。
fn qualifier_rank(s: &str) -> (u8, String) {
    let l = s.to_ascii_lowercase();
    let base = l.trim_end_matches(|c: char| c.is_ascii_digit());
    let suffix = &l[base.len()..];
    let rank = match base {
        "alpha" | "a" => 0,
        "beta" | "b" => 1,
        "milestone" | "m" => 2,
        "rc" | "cr" => 3,
        "snapshot" => 4,
        "" | "ga" | "final" | "release" | "stable" => 5,
        "sp" => 6,
        _ => 7,
    };
    if rank == 7 {
        return (7, l);
    }
    // 数字后缀零填充（alpha9 < alpha10）
    let padded = if suffix.is_empty() {
        String::new()
    } else {
        format!("{:0>4}", suffix)
    };
    (rank, padded)
}

/// 版本切分为 (数字, qualifier) 段序列；`.`/`-`/数字与非数字边界均为分隔。
fn version_items(v: &str) -> Vec<(u64, (u8, String))> {
    let mut items = Vec::new();
    for part in v.split(['.', '-']) {
        if part.is_empty() {
            continue;
        }
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        let rest = &part[digits.len()..];
        if digits.is_empty() {
            items.push((0, qualifier_rank(part)));
        } else {
            let n: u64 = digits.parse().unwrap_or(0);
            if rest.is_empty() {
                items.push((n, (5, String::new()))); // 纯数字段视作 release 档
            } else {
                items.push((n, qualifier_rank(rest)));
            }
        }
    }
    // 尾部零段修剪（1.0.0 == 1）
    while items.last().is_some_and(|(n, (r, _))| *n == 0 && *r == 5) {
        items.pop();
    }
    items
}

/// 简化版 Maven 版本比较（满足范围选版需要，不追求 ComparableVersion 完全等价）。
pub fn cmp_version(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let ia = version_items(a);
    let ib = version_items(b);
    let n = ia.len().max(ib.len());
    // 缺失段视作 (0, release 档)
    let default_item: (u64, (u8, String)) = (0, (5, String::new()));
    for i in 0..n {
        let xa = ia.get(i).unwrap_or(&default_item);
        let xb = ib.get(i).unwrap_or(&default_item);
        match xa.0.cmp(&xb.0) {
            Ordering::Equal => {}
            o => return o,
        }
        let ord = match (xa.1 .0, xb.1 .0) {
            (ra, rb) => match ra.cmp(&rb) {
                Ordering::Equal => xa.1 .1.cmp(&xb.1 .1),
                o => o,
            },
        };
        match ord {
            Ordering::Equal => {}
            o => return o,
        }
    }
    ia.len().cmp(&ib.len())
}

// ---------- 范围解析 ----------

#[derive(Debug, Clone, PartialEq)]
pub enum Bound {
    Inclusive(String),
    Exclusive(String),
    Unbounded,
}

#[derive(Debug, Clone)]
pub struct VersionRange {
    pub low: Bound,
    pub high: Bound,
}

impl Bound {
    /// 值是否满足该下界
    fn allows_low(&self, v: &str) -> bool {
        match self {
            Bound::Unbounded => true,
            Bound::Inclusive(b) => cmp_version(v, b) != std::cmp::Ordering::Less,
            Bound::Exclusive(b) => cmp_version(v, b) == std::cmp::Ordering::Greater,
        }
    }

    /// 值是否满足该上界
    fn allows_high(&self, v: &str) -> bool {
        match self {
            Bound::Unbounded => true,
            Bound::Inclusive(b) => cmp_version(v, b) != std::cmp::Ordering::Greater,
            Bound::Exclusive(b) => cmp_version(v, b) == std::cmp::Ordering::Less,
        }
    }
}

/// 解析 Maven 范围表达式：`[a,b]` `(a,b)` `[a,b)` `(a,b]` `[a,)` `(,b]` 或单值 `1.0`（最小版本）。
/// 输入须已是范围（以 `[`/`(` 开头）；无法解析返回 None。
pub fn parse_range(s: &str) -> Option<VersionRange> {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let (low_incl, high_incl) = match (bytes[0], bytes[s.len() - 1]) {
        (b'[', b']') => (true, true),
        (b'(', b')') => (false, false),
        (b'[', b')') => (true, false),
        (b'(', b']') => (false, true),
        _ => return None,
    };
    let inner = &s[1..s.len() - 1];
    if inner.contains(",") {
        let (low, high) = inner.split_once(',')?;
        let low_b = if low.trim().is_empty() {
            Bound::Unbounded
        } else if low_incl {
            Bound::Inclusive(low.trim().to_string())
        } else {
            Bound::Exclusive(low.trim().to_string())
        };
        let high_b = if high.trim().is_empty() {
            Bound::Unbounded
        } else if high_incl {
            Bound::Inclusive(high.trim().to_string())
        } else {
            Bound::Exclusive(high.trim().to_string())
        };
        Some(VersionRange { low: low_b, high: high_b })
    } else {
        // 单值 = [v,v]（最小版本硬要求）
        let v = inner.trim().to_string();
        Some(VersionRange {
            low: Bound::Inclusive(v.clone()),
            high: Bound::Inclusive(v),
        })
    }
}

impl VersionRange {
    pub fn contains(&self, v: &str) -> bool {
        self.low.allows_low(v) && self.high.allows_high(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    fn assert_ord(a: &str, b: &str) {
        assert_eq!(cmp_version(a, b), Ordering::Less, "{a} should < {b}");
        assert_eq!(cmp_version(b, a), Ordering::Greater);
    }

    #[test]
    fn detect_snapshot() {
        assert!(is_snapshot("1.0-SNAPSHOT"));
        assert!(!is_snapshot("1.0"));
        assert!(!is_snapshot("1.0-20240101.123456-3"));
    }

    #[test]
    fn detect_range_and_meta() {
        assert!(is_range_or_meta("[1.0,2.0)"));
        assert!(is_range_or_meta("(,1.0]"));
        assert!(is_range_or_meta("LATEST"));
        assert!(is_range_or_meta("release"));
        assert!(!is_range_or_meta("1.0.1"));
        assert!(!is_range_or_meta("1.0-SNAPSHOT"));
    }

    #[test]
    fn version_ordering() {
        assert_ord("4.0", "4.2.3");
        assert_ord("4.2.3", "5.0");
        assert_ord("1.0-SNAPSHOT", "1.0");
        assert_ord("1.0-alpha1", "1.0");
        assert_ord("1.0-beta", "1.0-rc1");
        assert_ord("9.4.48.v20220622", "9.4.49.v20220914");
        assert_eq!(cmp_version("1.0.0", "1"), Ordering::Equal);
        assert_eq!(cmp_version("1.0", "1.0"), Ordering::Equal);
    }

    #[test]
    fn range_parsing() {
        let r = parse_range("[4.0,5.0)").unwrap();
        assert!(r.contains("4.0"));
        assert!(r.contains("4.2.3"));
        assert!(!r.contains("5.0"));
        assert!(!r.contains("3.9"));

        let r = parse_range("(,5.0]").unwrap();
        assert!(r.contains("4.9"));
        assert!(r.contains("5.0"));
        assert!(!r.contains("5.0.1"));

        let r = parse_range("[4.0,)").unwrap();
        assert!(r.contains("99.0"));
        assert!(!r.contains("3.9"));

        // 单值
        let r = parse_range("[4.2.2]").unwrap();
        assert!(r.contains("4.2.2"));
        assert!(!r.contains("4.2.3"));
        assert!(parse_range("4.0").is_none()); // 不以 [/( 开头不是范围
    }
}
