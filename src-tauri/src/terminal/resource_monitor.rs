//! SSH 远程主机的资源监控（延迟 / CPU / 内存 / SWAP / 网络速率 / 磁盘）。
//!
//! 取值方案是 mobaxtauri 与 winssh 两家参考实现的并集：
//! - **延迟**：整条采集命令的往返耗时（客户端掐表，含一次远端 shell 执行，
//!   与 winssh 的 uname 掐表同口径）；
//! - **CPU%**：`/proc/stat` 聚合行的**客户端差分**（per-会话基线 + 轮询间隔即
//!   采样窗，winssh 取法——省掉远端 sleep 1，首拍无基线返回 null）；
//! - **内存 / SWAP**：`/proc/meminfo` 的 `MemTotal − MemAvailable` /
//!   `SwapTotal − SwapFree`（字节精度、内核口径，优于 `free` 命令）；
//! - **网络 ↓↑**：`/proc/net/dev` 全网卡汇总（排除 `lo`）客户端差分 → B/s；
//! - **磁盘 /**：`df -P -B1 /`（POSIX 列位、字节单位）；
//! - **平台 / 发行版**：`uname -s` 定平台（Linux 才出资源段），
//!   `/etc/os-release` 的 `PRETTY_NAME`（回退 `ID`）作发行版徽标。
//!
//! 输出用 marker 分段（winssh 形态），局部解析失败仅置空对应指标（partial
//! 降级），非 Linux 主机只返回延迟与平台。解析与差分均为纯函数，macOS 单测
//! 覆盖（Windows 车道不跑 cargo test 的既定约定）。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// marker 段标识（远端输出按行匹配这些字面量切分）
const MARKER_OS: &str = "__VELOTERM_OS__";
const MARKER_DISTRO: &str = "__VELOTERM_DISTRO__";
const MARKER_STAT: &str = "__VELOTERM_STAT__";
const MARKER_MEM: &str = "__VELOTERM_MEMINFO__";
const MARKER_NET: &str = "__VELOTERM_NET_DEV__";
const MARKER_DF: &str = "__VELOTERM_DF__";

/// 远端采集命令：单次 exec 拿全部指标（POSIX sh 兼容，Linux 段才输出资源）。
/// 只取聚合 cpu 行与 meminfo 的四个键，省流量；`df -P -B1` 保证列位与字节单位
pub const RESOURCE_COMMAND: &str = r#"printf '%s\n' __VELOTERM_OS__
uname -s
printf '%s\n' __VELOTERM_DISTRO__
D=$(sed -n 's/^PRETTY_NAME="\(.*\)"$/\1/p' /etc/os-release 2>/dev/null | head -n1)
[ -n "$D" ] || D=$(sed -n 's/^ID=//p' /etc/os-release 2>/dev/null | head -n1)
printf '%s\n' "$D"
if [ "$(uname -s)" = Linux ]; then
  printf '%s\n' __VELOTERM_STAT__
  sed -n 's/^cpu //p' /proc/stat 2>/dev/null || true
  printf '%s\n' __VELOTERM_MEMINFO__
  grep -E '^(MemTotal|MemAvailable|SwapTotal|SwapFree):' /proc/meminfo 2>/dev/null || true
  printf '%s\n' __VELOTERM_NET_DEV__
  cat /proc/net/dev 2>/dev/null || true
  printf '%s\n' __VELOTERM_DF__
  df -P -B1 / 2>/dev/null || true
fi"#;

/// 字节用量（内存 / SWAP / 磁盘共用）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ByteUsage {
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub usage_percent: f32,
}

/// 网络上下行速率（B/s；首拍无基线为 None）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkRate {
    pub rx_bytes_per_sec: Option<f64>,
    pub tx_bytes_per_sec: Option<f64>,
}

/// 一次采集的完整快照（serde camelCase 直达前端）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSnapshot {
    pub sampled_at: String,
    /// 采集命令往返耗时（毫秒，含远端 shell 执行）
    pub rtt_ms: u64,
    /// 远端平台：linux / darwin / windows / unknown
    pub platform: String,
    /// 发行版展示名（/etc/os-release PRETTY_NAME，无则 None）
    pub distro: Option<String>,
    /// CPU 使用率%（客户端差分，首拍 None）
    pub cpu: Option<f32>,
    pub memory: Option<ByteUsage>,
    pub swap: Option<ByteUsage>,
    pub network: Option<NetworkRate>,
    pub disk: Option<ByteUsage>,
}

/// 差分基线（CPU 计数器 + 网卡字节数 + 采样时刻）
#[derive(Debug, Clone, Copy)]
struct Baseline {
    cpu_idle: u64,
    cpu_total: u64,
    rx_bytes: u64,
    tx_bytes: u64,
    sampled_ms: u64,
}

/// 基线表：terminal_id → 上次采样。标准库 Mutex（临界区纯内存读写）；
/// dispose 时由 manager 调 forget 清理，另有「超过 60s 视为首次」的兜底
/// （会话长时间隐藏后恢复轮询时不产生失真的速率跳变）
fn baselines() -> &'static Mutex<HashMap<usize, Baseline>> {
    static BASELINES: OnceLock<Mutex<HashMap<usize, Baseline>>> = OnceLock::new();
    BASELINES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 会话销毁时清除差分基线（manager.dispose 调用）
pub fn forget_baseline(terminal_id: usize) {
    baselines().lock().unwrap().remove(&terminal_id);
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// 把 marker 分段的远端输出切成 (marker → 内容) 映射
fn split_sections(output: &str) -> HashMap<&'static str, Vec<&str>> {
    let mut sections: HashMap<&'static str, Vec<&str>> = HashMap::new();
    let mut current: Option<&'static str> = None;
    for line in output.lines() {
        let marker = match line.trim() {
            MARKER_OS => Some(MARKER_OS),
            MARKER_DISTRO => Some(MARKER_DISTRO),
            MARKER_STAT => Some(MARKER_STAT),
            MARKER_MEM => Some(MARKER_MEM),
            MARKER_NET => Some(MARKER_NET),
            MARKER_DF => Some(MARKER_DF),
            _ => None,
        };
        if let Some(m) = marker {
            sections.entry(m).or_default();
            current = Some(m);
            continue;
        }
        if let Some(m) = current {
            sections.entry(m).or_default().push(line);
        }
    }
    sections
}

/// `/proc/stat` 聚合行（已去掉 "cpu " 前缀）→ (idle, total)。idle 含 iowait
fn parse_cpu_counters(line: &str) -> Option<(u64, u64)> {
    let values: Vec<u64> = line
        .split_whitespace()
        .filter_map(|v| v.parse::<u64>().ok())
        .collect();
    if values.len() < 4 {
        return None;
    }
    let idle = values.get(3)? + values.get(4).copied().unwrap_or(0);
    let total: u64 = values.iter().sum();
    (total > 0).then_some((idle, total))
}

/// meminfo 键值（kB）→ 字节映射，取指定键
fn meminfo_bytes(lines: &[&str], key: &str) -> Option<u64> {
    lines
        .iter()
        .find_map(|l| l.trim().strip_prefix(key).and_then(|rest| {
            let kb = rest
                .trim()
                .trim_end_matches("kB")
                .trim()
                .parse::<u64>()
                .ok()?;
            kb.checked_mul(1024)
        }))
}

/// `/proc/net/dev` → 全网卡（排除 lo）汇总 (rx_bytes, tx_bytes)
fn parse_net_dev(lines: &[&str]) -> Option<(u64, u64)> {
    let mut rx = 0u64;
    let mut tx = 0u64;
    let mut seen = false;
    for line in lines {
        let Some((ifname_raw, counters_raw)) = line.split_once(':') else {
            continue;
        };
        let ifname = ifname_raw.trim();
        if ifname.is_empty() || ifname == "lo" {
            continue;
        }
        let counters: Vec<u64> = counters_raw
            .split_whitespace()
            .filter_map(|v| v.parse::<u64>().ok())
            .collect();
        if counters.len() < 16 {
            continue;
        }
        seen = true;
        rx = rx.saturating_add(counters.first().copied().unwrap_or(0));
        tx = tx.saturating_add(counter_at(&counters, 8));
    }
    seen.then_some((rx, tx))
}

/// clippy indexing-slicing 禁令下的受控取位（返回 0 而非 panic）
fn counter_at(counters: &[u64], index: usize) -> u64 {
    counters.get(index).copied().unwrap_or(0)
}

/// `df -P -B1 /` 正文 → (used, total, percent)。表头（首字段 Filesystem）跳过：
/// POSIX -P 格式保证设备名不折行，正文恒为表头后的第一行
fn parse_df(lines: &[&str]) -> Option<(u64, u64, u32)> {
    let body = lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .find(|l| !l.starts_with("Filesystem"))?;
    let fields: Vec<&str> = body.split_whitespace().collect();
    if fields.len() < 6 {
        return None;
    }
    let total = fields.get(1)?.parse::<u64>().ok()?;
    let used = fields.get(2)?.parse::<u64>().ok()?;
    let percent = fields.get(4)?.trim_end_matches('%').parse::<u32>().ok()?;
    (total > 0).then_some((used, total, percent))
}

fn byte_usage(used: u64, total: u64) -> ByteUsage {
    let percent = if total > 0 {
        (used as f64 / total as f64) * 100.0
    } else {
        0.0
    };
    ByteUsage {
        used_bytes: used,
        total_bytes: total,
        usage_percent: ((percent * 10.0).round() / 10.0) as f32,
    }
}

/// 解析远端输出 + 差分计算，产出快照（纯函数 + 基线表读写）。rtt_ms 由
/// 调用方掐表传入。任何一段失败只置空该指标，绝不让整次采集失败
pub fn build_snapshot(terminal_id: usize, output: &str, rtt_ms: u64) -> ResourceSnapshot {
    let sections = split_sections(output);
    let platform_raw = sections
        .get(MARKER_OS)
        .and_then(|lines| lines.first())
        .map(|line| line.trim())
        .unwrap_or("");
    let platform = match platform_raw {
        "Linux" => "linux",
        "Darwin" => "darwin",
        s if s.starts_with("Windows") || s == "CYGWIN_NT" || s == "MSYS_NT" => "windows",
        _ => "unknown",
    }
    .to_string();

    // trim_matches 剥首尾引号：os-release 的值形如 PRETTY_NAME="CentOS ... (Core)"，
    // 历版采集脚本只剥了前引号，尾引号一路漏到前端徽标（实测 CentOS 7 出现
    // `CentOS ... (Core)"`）；脚本侧已同步修正，解析侧再兑一层底。
    let distro = sections
        .get(MARKER_DISTRO)
        .and_then(|lines| lines.first())
        .map(|line| line.trim().trim_matches('"').trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let mut snapshot = ResourceSnapshot {
        sampled_at: unix_ms_string(),
        rtt_ms,
        platform,
        distro,
        cpu: None,
        memory: None,
        swap: None,
        network: None,
        disk: None,
    };

    if snapshot.platform != "linux" {
        // 非 Linux：只保留延迟与平台（前端展示「仅 Linux」提示）
        return snapshot;
    }

    // 内存 / SWAP（meminfo，内核口径）
    if let Some(mem_lines) = sections.get(MARKER_MEM) {
        if let (Some(total), Some(available)) = (
            meminfo_bytes(mem_lines, "MemTotal:"),
            meminfo_bytes(mem_lines, "MemAvailable:"),
        ) {
            snapshot.memory = Some(byte_usage(total.saturating_sub(available), total));
        }
        if let (Some(total), Some(free)) = (
            meminfo_bytes(mem_lines, "SwapTotal:"),
            meminfo_bytes(mem_lines, "SwapFree:"),
        ) {
            snapshot.swap = Some(byte_usage(total.saturating_sub(free), total));
        }
    }

    // 磁盘（df -P -B1 /）
    if let Some(df_lines) = sections.get(MARKER_DF)
        && let Some((used, total, _percent)) = parse_df(df_lines)
    {
        snapshot.disk = Some(byte_usage(used, total));
    }

    // CPU% 与网络速率：客户端差分（首拍 None；基线超过 60s 视为重新起测）
    let cpu_now = sections
        .get(MARKER_STAT)
        .and_then(|lines| lines.first())
        .and_then(|l| parse_cpu_counters(l.trim()));
    let net_now = sections.get(MARKER_NET).and_then(|lines| parse_net_dev(lines));

    let sampled_ms = now_unix_ms();
    let mut baselines = baselines().lock().unwrap();
    let previous = baselines
        .get(&terminal_id)
        .copied()
        .filter(|b| sampled_ms.saturating_sub(b.sampled_ms) <= 60_000);

    if let (Some(prev), Some((idle, total))) = (previous, cpu_now) {
        let dt = total.saturating_sub(prev.cpu_total);
        let di = idle.saturating_sub(prev.cpu_idle);
        if dt > 0 {
            snapshot.cpu = Some((((dt - di) as f64 / dt as f64) * 100.0) as f32);
        }
    }
    if let (Some(prev), Some((rx, tx))) = (previous, net_now) {
        let elapsed_s = (sampled_ms.saturating_sub(prev.sampled_ms)) as f64 / 1000.0;
        if elapsed_s > 0.0 {
            snapshot.network = Some(NetworkRate {
                rx_bytes_per_sec: Some(rx.saturating_sub(prev.rx_bytes) as f64 / elapsed_s),
                tx_bytes_per_sec: Some(tx.saturating_sub(prev.tx_bytes) as f64 / elapsed_s),
            });
        }
    }

    if let (Some((idle, total)), Some((rx, tx))) = (cpu_now, net_now) {
        baselines.insert(
            terminal_id,
            Baseline {
                cpu_idle: idle,
                cpu_total: total,
                rx_bytes: rx,
                tx_bytes: tx,
                sampled_ms,
            },
        );
    }
    snapshot
}

fn unix_ms_string() -> String {
    // 无 chrono 依赖：unix 毫秒字符串即满足前端「最近一次采样」的展示需要
    now_unix_ms().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINUX_OUTPUT: &str = "\
__VELOTERM_OS__
Linux
__VELOTERM_DISTRO__
Ubuntu 24.04.1 LTS
__VELOTERM_STAT__
136754 1217 96332 4215892 17392 0 1688 0 0 0
__VELOTERM_MEMINFO__
MemTotal:       16093124 kB
MemAvailable:    8412396 kB
SwapTotal:       2097148 kB
SwapFree:        1997148 kB
__VELOTERM_NET_DEV__
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 1234567    9876    0    0    0     0          0         0  1234567     9876    0    0    0     0       0          0         0
  eth0: 987654321   12345    0    0    0     0          0         0  123456789   11111    0    0    0     0       0          0         0
__VELOTERM_DF__
Filesystem     1B-blocks       Used   Available Use% Mounted on
/dev/sda2     524288000  367001600   157286400  70% /
";

    #[test]
    fn parses_linux_sections_and_bytes() {
        let snap = build_snapshot(1, LINUX_OUTPUT, 63);
        assert_eq!(snap.platform, "linux");
        assert_eq!(snap.distro.as_deref(), Some("Ubuntu 24.04.1 LTS"));
        assert_eq!(snap.rtt_ms, 63);
        // 首拍无基线：CPU 与网络为 None
        assert_eq!(snap.cpu, None);
        assert_eq!(snap.network, None);
        // 内存：MemTotal − MemAvailable（kB → 字节）
        let mem = snap.memory.expect("memory");
        assert_eq!(mem.total_bytes, 16_093_124 * 1024);
        assert_eq!(mem.used_bytes, (16_093_124 - 8_412_396) * 1024);
        // SWAP：SwapTotal − SwapFree
        let swap = snap.swap.expect("swap");
        assert_eq!(swap.used_bytes, (2_097_148 - 1_997_148) * 1024);
        assert_eq!(swap.total_bytes, 2_097_148 * 1024);
        // 磁盘：df -P -B1 第二行
        let disk = snap.disk.expect("disk");
        assert_eq!(disk.used_bytes, 367_001_600);
        assert_eq!(disk.total_bytes, 524_288_000);
        // 基线已登记
        assert!(baselines().lock().unwrap().contains_key(&1));
        forget_baseline(1);
    }

    #[test]
    fn strips_stray_quotes_from_distro_line() {
        // 历版采集脚本只剥 PRETTY_NAME 的前引号，尾引号一路漏到前端徽标
        // （CentOS 7 实测出现 `CentOS Linux 7 (Core)"`）。脚本侧已修，解析侧
        // 兜底：首尾引号剥掉、正文中段引号不受影响。
        let output = "__VELOTERM_OS__\nLinux\n__VELOTERM_DISTRO__\nCentOS Linux 7 (Core)\"\n";
        let snap = build_snapshot(7, output, 10);
        assert_eq!(snap.distro.as_deref(), Some("CentOS Linux 7 (Core)"));
        let quoted_both = "__VELOTERM_OS__\nLinux\n__VELOTERM_DISTRO__\n\"Ubuntu 24.04 LTS\"\n";
        assert_eq!(
            build_snapshot(8, quoted_both, 10).distro.as_deref(),
            Some("Ubuntu 24.04 LTS")
        );
        // 无引号值原样保留
        let plain = "__VELOTERM_OS__\nLinux\n__VELOTERM_DISTRO__\ndebian\n";
        assert_eq!(build_snapshot(9, plain, 10).distro.as_deref(), Some("debian"));
        forget_baseline(7);
        forget_baseline(8);
        forget_baseline(9);
    }

    #[test]
    fn diffs_cpu_and_network_against_baseline() {
        // 首拍登记基线，次拍 user +1000 / idle +100：dt=1100, di=100 → 90.9%。
        // 网络差分依赖采样间隔（同毫秒内两次调用 elapsed=0 会跳过），
        // 其速率路径由 parse_net_dev 的汇总断言覆盖
        let _ = build_snapshot(2, LINUX_OUTPUT, 10);
        let second =
            LINUX_OUTPUT.replace("136754 1217 96332 4215892", "137754 1217 96332 4215992");
        let snap = build_snapshot(2, &second, 11);
        let cpu = snap.cpu.expect("cpu diff");
        assert!((cpu - 90.9).abs() < 0.2, "cpu={cpu}");
        forget_baseline(2);
    }

    #[test]
    fn net_dev_sums_physical_interfaces_excluding_lo() {
        let lines: Vec<&str> = LINUX_OUTPUT
            .lines()
            .skip_while(|l| !l.contains("Inter-|"))
            .skip(2)
            .collect();
        let (rx, tx) = parse_net_dev(&lines).expect("net dev");
        assert_eq!(rx, 987_654_321);
        assert_eq!(tx, 123_456_789);
    }

    #[test]
    fn non_linux_returns_latency_only() {
        let out = "__VELOTERM_OS__\nDarwin\n__VELOTERM_DISTRO__\n\n";
        let snap = build_snapshot(3, out, 41);
        assert_eq!(snap.platform, "darwin");
        assert_eq!(snap.distro, None);
        assert_eq!(snap.rtt_ms, 41);
        assert!(snap.cpu.is_none() && snap.memory.is_none() && snap.disk.is_none());
    }

    #[test]
    fn garbage_output_degrades_to_unknown() {
        let snap = build_snapshot(4, "total chaos\nno markers\n", 5);
        assert_eq!(snap.platform, "unknown");
        assert_eq!(snap.rtt_ms, 5);
    }
}
