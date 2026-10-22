//! free 命令实现（GNU procps free 兼容子集：内存/交换查看）。
//!
//! 数据源是 crate 已依赖的 sysinfo 0.31（内存 API 返回**字节**，见 ps.rs /
//! top.rs 的既有用法）；输出布局与单位换算逐字对齐 procps-ng `free.c`：
//! 表头 15 列起，行 label 占 9 列、数值列宽 11 右对齐；Mem 行六列
//! （total/used/free/shared/buff/cache/available），Swap 行三列，`-t` 追加
//! Total 行（三列，= Mem + Swap 逐列和）。
//!
//! 支持参数（GNU 兼容子集）：
//! - `-b` / `-k` / `-m` / `-g`：字节 / KiB（默认）/ MiB / GiB（整数截断除法）；
//! - `-h`：人类可读——**GNU free 的 -h 是 --human 而非 help**（procps 家族
//!   惯例，与本工程 ps/top 的 -h 含义不同），复刻其 scale_size 非 SI 分支
//!   （1024 底，先一位小数后三位整数形态，见 scale_human 注释）；
//! - `-t`：追加 Total 行；
//! - `--help` / `-V, --version`；
//! - 多个单位选项同时给出 → 报错退出 1（GNU check_unit_set 同款）；
//! - 任何非选项操作数 → 用法错误退出 1（GNU free 不接受操作数）；
//! - 不支持：`--si` 十进制族（--kilo/--mega 等）、`-s/-c` 连续刷新、
//!   `-l/-L/-v/-w`。
//!
//! 与 procps（读 /proc/meminfo）的口径差异——sysinfo 采不到的列以近似或
//! 0 呈现：
//! - **shared**（procps = Shmem）：sysinfo 不提供 → 恒 0；
//! - **buff/cache**（procps = Buffers + Cached）：sysinfo 不提供 → 以
//!   `total - used - free` 近似（Windows 上 sysinfo 的 free ==
//!   available、used == total - available，该式恰为 0；Linux/macOS 为
//!   used/free 两口径之外的余量，即内核可回收页的近似）；
//! - **used**：sysinfo 在 Linux/Windows 均为 `total - available`，procps
//!   在 Linux 为 `total - free - buffers - cached`，两者数值略有出入；
//! - **available**：sysinfo available_memory（Linux = MemAvailable，
//!   Windows = GlobalMemoryStatusEx 的 ullAvailPhys，与 free 同值）。
//!
//! 平台门控：命令名只进 tools-catalog 的 Windows 清单；实现与分发臂跨平台
//! 编译（无 cfg 门控）——macOS 本机单测全量覆盖行为（同 data-encoding
//! 家族的门控形态），Windows 侧由 CI 构建与 test-commands 车道覆盖。

use std::ffi::OsString;
use sysinfo::System;

/// 版本串（对齐本工程自研 ps/top 的 `xxx (veloterm) 0.1.0` 惯例）
const VERSION: &str = "0.1.0";

/// 表头行：与 procps free.c 的字面量逐字符一致（label 9 列 + 6 个 11 宽
/// 右对齐列的等价形态，见 default_report_layout 的再生成断言）
const HEADER_LINE: &str =
    "               total        used        free      shared  buff/cache   available";

/// 显示单位
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    /// KiB（GNU 默认：free 读 /proc/meminfo 天然以 KB 呈现）
    Kibi,
    /// `-b`
    Bytes,
    /// `-m`
    Mebi,
    /// `-g`
    Gibi,
    /// `-h`（1024 底人类可读）
    Human,
}

#[derive(Debug)]
struct FreeOptions {
    unit: Unit,
    total: bool,
}

impl FreeOptions {
    fn new() -> Self {
        Self {
            unit: Unit::Kibi,
            total: false,
        }
    }
}

/// 一行内存指标（全部字节）
#[derive(Debug, Clone, Copy, Default)]
struct MemRow {
    total: u64,
    used: u64,
    free: u64,
    shared: u64,
    buff_cache: u64,
    available: u64,
}

/// 一次采样的全部数据（Mem + Swap；格式化与采样分离，单测用假数据直测）
#[derive(Debug, Clone, Copy, Default)]
struct FreeSample {
    mem: MemRow,
    swap: MemRow,
}

/// 帮助文案（选项行按 procps free.c usage 的文案裁剪到本实现支持的子集）
const HELP_TEXT: &str = "Usage:
 \
 free [options]

\
 Options:
 \
 \x20 -b, --bytes         show output in bytes
 \
 \x20 -k, --kibi          show output in kibibytes
 \
 \x20 -m, --mebi          show output in mebibytes
 \
 \x20 -g, --gibi          show output in gibibytes
 \
 \x20 -h, --human         show human-readable output
 \
 \x20 -t, --total         show total for RAM + swap
 \
 \x20     --help     display this help and exit
 \
 \x20 -V, --version  output version information and exit
";

/// GNU 对单位互斥的处理：第二个单位选项直接 errx 退出
/// （"Multiple unit options don't make sense."，退出码 1）
fn set_unit(opts: &mut FreeOptions, unit_set: &mut bool, unit: Unit) -> Result<(), i32> {
    if *unit_set {
        eprintln!("free: Multiple unit options don't make sense.");
        eprint!("{HELP_TEXT}");
        return Err(1);
    }
    *unit_set = true;
    opts.unit = unit;
    Ok(())
}

/// 解析参数：Ok(opts) 正常执行；Err(code) 表示帮助/版本/错误信息已打印，
/// 直接以该码退出（0 = --help/--version，1 = 用法错误）。
fn parse_args(args: &[OsString]) -> Result<FreeOptions, i32> {
    let mut opts = FreeOptions::new();
    let mut unit_set = false;
    let mut no_more_flags = false;

    for arg in args.iter().map(|s| s.to_string_lossy().into_owned()) {
        if no_more_flags || !arg.starts_with('-') || arg == "-" {
            // GNU free 不接受任何操作数（optind != argc → usage 错误）
            eprintln!("free: unexpected operand '{arg}'");
            eprint!("{HELP_TEXT}");
            return Err(1);
        }
        if arg == "--" {
            no_more_flags = true;
            continue;
        }
        if let Some(long) = arg.strip_prefix("--") {
            match long {
                "bytes" => set_unit(&mut opts, &mut unit_set, Unit::Bytes)?,
                "kibi" => set_unit(&mut opts, &mut unit_set, Unit::Kibi)?,
                "mebi" => set_unit(&mut opts, &mut unit_set, Unit::Mebi)?,
                "gibi" => set_unit(&mut opts, &mut unit_set, Unit::Gibi)?,
                "human" => set_unit(&mut opts, &mut unit_set, Unit::Human)?,
                "total" => opts.total = true,
                "help" => {
                    print!("{HELP_TEXT}");
                    return Err(0);
                }
                "version" => {
                    println!("free (veloterm) {VERSION}");
                    return Err(0);
                }
                _ => {
                    eprintln!("free: unrecognized option '{arg}'");
                    eprint!("{HELP_TEXT}");
                    return Err(1);
                }
            }
            continue;
        }
        // 短参数可捆绑（GNU getopt 语义，如 `-th`）
        for c in arg[1..].chars() {
            match c {
                'b' => set_unit(&mut opts, &mut unit_set, Unit::Bytes)?,
                'k' => set_unit(&mut opts, &mut unit_set, Unit::Kibi)?,
                'm' => set_unit(&mut opts, &mut unit_set, Unit::Mebi)?,
                'g' => set_unit(&mut opts, &mut unit_set, Unit::Gibi)?,
                'h' => set_unit(&mut opts, &mut unit_set, Unit::Human)?,
                't' => opts.total = true,
                'V' => {
                    println!("free (veloterm) {VERSION}");
                    return Err(0);
                }
                _ => {
                    eprintln!("free: invalid option -- '{c}'");
                    eprint!("{HELP_TEXT}");
                    return Err(1);
                }
            }
        }
    }

    Ok(opts)
}

/// 采样：只采内存与 swap——`refresh_memory` 一次刷新两者，无需 `new_all`
/// 的 CPU/进程/磁盘/网络采集（free 不展示任何进程信息；sysinfo 0.31 的
/// 内存 API 返回字节数）。
fn collect_sample() -> FreeSample {
    let mut sys = System::new();
    sys.refresh_memory();

    let total = sys.total_memory();
    let used = sys.used_memory();
    let free = sys.free_memory();
    FreeSample {
        mem: MemRow {
            total,
            used,
            free,
            shared: 0, // sysinfo 无 Shmem 口径，见模块头差异说明
            buff_cache: total.saturating_sub(used + free), // 近似口径，见模块头
            available: sys.available_memory(),
        },
        swap: MemRow {
            total: sys.total_swap(),
            used: sys.used_swap(),
            free: sys.free_swap(),
            ..MemRow::default()
        },
    }
}

/// GNU `free -h` 的缩放（procps local/units.c `scale_size` 非 SI 分支的
/// 复刻，输入输出均为字节）：
/// - ≤9999 字节直接 `"1234B"`；
/// - 自 Ki 起逐级（1024 底）先试一位小数（十进制舍入），整串 ≤5 字符即
///   采用——仅单位数整形态放得下（如 `"9.8Ki"` / `"8.0Gi"`）；再试整数
///   形态（如 `"512Mi"` / `"15Gi"`），同样 ≤5 字符即采用；都放不下升级
///   下一级——故四位整数（`"1023Ki"`）与两位数带小数（`"15.6Gi"`）形态
///   都不会出现，分别进位为 `"1.0Mi"` / 取整为 `"15Gi"`（实测 GNU
///   `free -h` 对 16GB 整机即显示 `15Gi`）。
///
/// 一位小数用 f64 + `{:.1}`：GNU 用 float + `%.1f`（同为十进制舍入），
/// 边界差异只出现在第 8 位有效数字，对物理内存量级无实际影响。
fn scale_human(bytes: u64) -> String {
    if bytes <= 9999 {
        return format!("{bytes}B");
    }
    const KIB: u64 = 1024;
    const SUFFIXES: [(&str, u64); 5] = [
        ("Ki", KIB),
        ("Mi", KIB * KIB),
        ("Gi", KIB * KIB * KIB),
        ("Ti", KIB * KIB * KIB * KIB),
        ("Pi", KIB * KIB * KIB * KIB * KIB),
    ];
    for (suffix, div) in SUFFIXES {
        let frac = format!("{:.1}{suffix}", bytes as f64 / div as f64);
        if frac.len() <= 5 {
            return frac;
        }
        let int_form = format!("{}{suffix}", bytes / div);
        if int_form.len() <= 5 {
            return int_form;
        }
    }
    // 超过 Pi：GNU 也会撑爆列宽（其源码注释自嘲到 2015 年），这里以 Pi
    // 整数形态兜底
    format!("{}Pi", bytes / (KIB * KIB * KIB * KIB * KIB))
}

/// 按单位缩放一个字节数（固定单位为整数截断除法，同 GNU）
fn scale_value(bytes: u64, unit: Unit) -> String {
    match unit {
        Unit::Bytes => bytes.to_string(),
        Unit::Kibi => (bytes / 1024).to_string(),
        Unit::Mebi => (bytes / (1024 * 1024)).to_string(),
        Unit::Gibi => (bytes / (1024 * 1024 * 1024)).to_string(),
        Unit::Human => scale_human(bytes),
    }
}

/// 三值行（`Swap:` / `Total:` 的形态：label + total/used/free 三列）。
/// 布局对齐 procps：`%-9s` + `%11s` + 重复 `" %11s"`。
fn format_row3(label: &str, total: u64, used: u64, free: u64, unit: Unit) -> String {
    format!(
        "{:<9}{:>11} {:>11} {:>11}\n",
        label,
        scale_value(total, unit),
        scale_value(used, unit),
        scale_value(free, unit),
    )
}

/// 六值行（`Mem:` 的形态：label + total/used/free/shared/buff_cache/available）
fn format_row6(label: &str, row: &MemRow, unit: Unit) -> String {
    format!(
        "{:<9}{:>11} {:>11} {:>11} {:>11} {:>11} {:>11}\n",
        label,
        scale_value(row.total, unit),
        scale_value(row.used, unit),
        scale_value(row.free, unit),
        scale_value(row.shared, unit),
        scale_value(row.buff_cache, unit),
        scale_value(row.available, unit),
    )
}

/// 格式化整份报告（纯函数：单测以假数据直测，与采样解耦）
fn format_report(sample: &FreeSample, unit: Unit, total: bool) -> String {
    let mut out = String::new();
    out.push_str(HEADER_LINE);
    out.push('\n');
    out.push_str(&format_row6("Mem:", &sample.mem, unit));
    out.push_str(&format_row3(
        "Swap:",
        sample.swap.total,
        sample.swap.used,
        sample.swap.free,
        unit,
    ));
    if total {
        out.push_str(&format_row3(
            "Total:",
            sample.mem.total + sample.swap.total,
            sample.mem.used + sample.swap.used,
            sample.mem.free + sample.swap.free,
            unit,
        ));
    }
    out
}

/// 入口（coreutils 分发臂约定：args 不含命令名）
pub fn run(args: &[OsString]) -> i32 {
    let opts = match parse_args(args) {
        Ok(opts) => opts,
        Err(code) => return code,
    };
    let sample = collect_sample();
    print!("{}", format_report(&sample, opts.unit, opts.total));
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: &[&str]) -> Result<FreeOptions, i32> {
        let args: Vec<OsString> = v.iter().map(OsString::from).collect();
        parse_args(&args)
    }

    const GIB: u64 = 1024 * 1024 * 1024;

    /// 构造假数据：8GiB 物理内存（used 3GiB / free 2GiB / shared 256MiB /
    /// buff_cache 1GiB / available 4GiB）+ 2GiB swap（used 512MiB / free 1.5GiB）
    fn sample() -> FreeSample {
        FreeSample {
            mem: MemRow {
                total: 8 * GIB,
                used: 3 * GIB,
                free: 2 * GIB,
                shared: GIB / 4,
                buff_cache: GIB,
                available: 4 * GIB,
            },
            swap: MemRow {
                total: 2 * GIB,
                used: GIB / 2,
                free: GIB + GIB / 2,
                ..MemRow::default()
            },
        }
    }

    /// 取报告第 n 数据行的数值列（跳过 label），按 u64 解析
    fn numeric_fields(line: &str, label: &str) -> Vec<u64> {
        line[label.len()..]
            .split_whitespace()
            .map(|v| v.parse().expect("应为纯数字"))
            .collect()
    }

    #[test]
    fn help_and_version_short_circuit() {
        assert_eq!(parse(&["--help"]).unwrap_err(), 0);
        assert_eq!(parse(&["-V"]).unwrap_err(), 0);
        assert_eq!(parse(&["--version"]).unwrap_err(), 0);
        // 帮助文案形态：无前导空行，选项行齐全（锁定 HELP_TEXT 常量的
        // 行接续转义不被写坏）
        assert!(HELP_TEXT.starts_with("Usage:\n free [options]"));
        for opt in [
            "-b, --bytes",
            "-k, --kibi",
            "-m, --mebi",
            "-g, --gibi",
            "-h, --human",
            "-t, --total",
            "--help",
            "-V, --version",
        ] {
            assert!(HELP_TEXT.contains(opt), "帮助缺 {opt}");
        }
    }

    /// GNU free 的 -h 是 --human（procps 家族惯例），不是 help——与 ps/top 相反
    #[test]
    fn short_h_is_human_not_help() {
        let opts = parse(&["-h"]).unwrap();
        assert_eq!(opts.unit, Unit::Human);
        assert!(!opts.total);
    }

    #[test]
    fn defaults_and_unit_flags() {
        let opts = parse(&[]).unwrap();
        assert_eq!(opts.unit, Unit::Kibi); // GNU 默认 KiB
        assert!(!opts.total);

        for (flag, unit) in [
            ("-b", Unit::Bytes),
            ("-k", Unit::Kibi),
            ("-m", Unit::Mebi),
            ("-g", Unit::Gibi),
            ("-h", Unit::Human),
            ("--bytes", Unit::Bytes),
            ("--kibi", Unit::Kibi),
            ("--mebi", Unit::Mebi),
            ("--gibi", Unit::Gibi),
            ("--human", Unit::Human),
        ] {
            assert_eq!(parse(&[flag]).unwrap().unit, unit, "{flag}");
        }
        assert!(parse(&["-t"]).unwrap().total);
        assert!(parse(&["--total"]).unwrap().total);
        // GNU getopt 捆绑形态
        let opts = parse(&["-th"]).unwrap();
        assert!(opts.total);
        assert_eq!(opts.unit, Unit::Human);
        // `--` 结束选项后无操作数 → 正常
        assert!(parse(&["--"]).is_ok());
    }

    /// GNU check_unit_set：第二个单位选项即报错退出 1（含捆绑形态）
    #[test]
    fn multiple_unit_options_rejected() {
        assert_eq!(parse(&["-m", "-g"]).unwrap_err(), 1);
        assert_eq!(parse(&["-h", "-b"]).unwrap_err(), 1);
        assert_eq!(parse(&["-mg"]).unwrap_err(), 1);
    }

    /// 非法选项与非选项操作数均按 GNU 退出 1（free 不接受任何操作数）
    #[test]
    fn unknown_option_and_operand_rejected() {
        assert_eq!(parse(&["-z"]).unwrap_err(), 1);
        assert_eq!(parse(&["--veloterm-invalid"]).unwrap_err(), 1);
        assert_eq!(parse(&["/tmp"]).unwrap_err(), 1);
        assert_eq!(parse(&["-"]).unwrap_err(), 1);
    }

    /// 默认（KiB）报告布局：GNU 表头字面量 + 行构成 + 列对齐
    #[test]
    fn default_report_layout() {
        let report = format_report(&sample(), Unit::Kibi, false);
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(lines.len(), 3);

        // 表头与 procps free.c 的字面量逐字符一致
        assert_eq!(
            lines[0],
            "               total        used        free      shared  buff/cache   available"
        );
        // 用 procps 的列宽规约（label 9 列 + 11 宽右对齐列）再生成一遍，
        // 应得到同一字面量——锁定字面量与行格式永不漂移
        assert_eq!(
            lines[0],
            format!(
                "{:<9}{:>11} {:>11} {:>11} {:>11} {:>11} {:>11}",
                "", "total", "used", "free", "shared", "buff/cache", "available"
            )
        );

        // Mem 行六列、Swap 行三列（Swap 不占 shared/buff/cache/available）
        assert!(lines[1].starts_with("Mem:"));
        assert!(lines[2].starts_with("Swap:"));
        assert_eq!(
            numeric_fields(lines[1], "Mem:"),
            [8388608, 3145728, 2097152, 262144, 1048576, 4194304]
        );
        assert_eq!(numeric_fields(lines[2], "Swap:"), [2097152, 524288, 1572864]);
        // 整行精确快照（锁定列间距）
        assert_eq!(
            lines[1],
            "Mem:         8388608     3145728     2097152      262144     1048576     4194304"
        );
        assert_eq!(lines[2], "Swap:        2097152      524288     1572864");
    }

    /// 固定单位换算：整数截断除法（-g 下 256MiB → 0、512MiB → 0）
    #[test]
    fn unit_conversions() {
        // -b 原始字节
        let report = format_report(&sample(), Unit::Bytes, false);
        assert!(report.contains("8589934592"));

        let mebi = format_report(&sample(), Unit::Mebi, false);
        let lines: Vec<&str> = mebi.lines().collect();
        assert_eq!(
            numeric_fields(lines[1], "Mem:"),
            [8192, 3072, 2048, 256, 1024, 4096]
        );

        let gibi = format_report(&sample(), Unit::Gibi, false);
        let lines: Vec<&str> = gibi.lines().collect();
        assert_eq!(numeric_fields(lines[1], "Mem:"), [8, 3, 2, 0, 1, 4]);
        assert_eq!(numeric_fields(lines[2], "Swap:"), [2, 0, 1]);
    }

    /// GNU scale_size（非 SI 分支）逐条对齐：字节直出、一位小数优先、三位
    /// 整数形态、四位整数进位下一级
    #[test]
    fn human_scaling_vectors() {
        let cases: &[(u64, &str)] = &[
            (0, "0B"),
            (512, "512B"),
            (9999, "9999B"),
            (10000, "9.8Ki"), // 9.765625 → 一位小数四舍五入
            (1024, "1024B"),  // 仍属 ≤9999 字节直出区间
            (1536, "1536B"),
            (1024 * 1024, "1.0Mi"), // Ki 级 1024 整数/小数形态均超 5 字符 → 进位
            (1023 * 1024, "1.0Mi"),
            (100 * 1024 * 1024, "100Mi"), // "100.0Mi" 7 字符 → 三位整数形态
            (999 * 1024 * 1024, "999Mi"),
            (GIB, "1.0Gi"),
            (8 * GIB, "8.0Gi"),
            (GIB / 2, "512Mi"), // "512.0Mi" 超长 → 整数形态
            (102376, "99Ki"), // 99.97Ki：一位小数舍成 "100.0Ki" 超 5 字符 → 整数截断 "99Ki"（GNU 同款怪癖）
            (16716081280, "15Gi"), // 16316220KiB 整机经典值（15.578GiB）："15.6Gi" 6 字符放不下 → 取整
            (1024 * GIB, "1.0Ti"),
            (1024u64 * 1024 * GIB, "1.0Pi"),
        ];
        for (bytes, expect) in cases {
            assert_eq!(scale_human(*bytes), *expect, "{bytes} 字节");
        }
    }

    /// -h 整行输出：每个值带单位后缀（与 GNU 的混级呈现一致）
    #[test]
    fn human_report_units() {
        let report = format_report(&sample(), Unit::Human, false);
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(
            lines[1],
            "Mem:           8.0Gi       3.0Gi       2.0Gi       256Mi       1.0Gi       4.0Gi"
        );
        assert_eq!(lines[2], "Swap:          2.0Gi       512Mi       1.5Gi");
    }

    /// -t 追加 Total 行（= Mem + Swap 逐列和，三列形态）
    #[test]
    fn total_line_appended() {
        let report = format_report(&sample(), Unit::Kibi, true);
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[3].starts_with("Total:"));
        assert_eq!(
            numeric_fields(lines[3], "Total:"),
            [10485760, 3670016, 3670016]
        );
        // 不带 -t 时无 Total 行
        assert!(!format_report(&sample(), Unit::Kibi, false).contains("Total:"));
    }

    /// -m 输出数字全部可解析且 total ≥ used / free
    #[test]
    fn megabyte_report_parses_and_ordered() {
        let report = format_report(&sample(), Unit::Mebi, false);
        let fields = numeric_fields(report.lines().nth(1).unwrap(), "Mem:");
        assert_eq!(fields.len(), 6);
        assert!(fields[0] >= fields[1], "total ≥ used");
        assert!(fields[0] >= fields[2], "total ≥ free");
    }

    /// 真实采样冒烟（macOS 单测车道直跑）：物理内存非零，默认输出含完整
    /// 表头与 Mem:/Swap: 行
    #[test]
    fn real_sample_smoke() {
        let sample = collect_sample();
        assert!(sample.mem.total > 0, "物理内存总量应大于 0");
        let report = format_report(&sample, Unit::Kibi, true);
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(lines.len(), 4);
        for name in ["total", "used", "free", "shared", "buff/cache", "available"] {
            assert!(lines[0].contains(name), "表头缺 {name}");
        }
        assert!(lines[1].starts_with("Mem:"));
        assert!(lines[2].starts_with("Swap:"));
        assert!(lines[3].starts_with("Total:"));
    }
}
