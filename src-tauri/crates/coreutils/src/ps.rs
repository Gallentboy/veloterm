//! ps 命令实现（Windows 与跨平台进程状态查看器）
//!
//! 支持参数：
//! - `ps` / `ps -ef` / `ps aux`：列出进程
//! - `-p, --pid <PID>`：指定 PID 过滤
//! - `-C, --command <NAME>`：按进程名或命令行过滤（子串、忽略大小写）
//! - `--sort <[-]cpu|[-]mem|[-]pid>`：排序
//! - `-h, --help`：帮助
//! - `-V, --version`：版本信息

use std::ffi::OsString;
use sysinfo::System;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Default,
    Full, // -f / -ef
    Aux,  // aux / -aux
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Pid,
    Cpu,
    Mem,
}

#[derive(Debug)]
struct PsOptions {
    format: OutputFormat,
    filter_pids: Vec<u32>,
    filter_comm: Option<String>,
    sort_key: SortKey,
    sort_descending: bool,
}

impl Default for PsOptions {
    fn default() -> Self {
        Self {
            format: OutputFormat::Default,
            filter_pids: Vec::new(),
            filter_comm: None,
            sort_key: SortKey::Pid,
            sort_descending: false,
        }
    }
}

fn print_help() {
    println!(
        "Usage: ps [options]\n\n\
        Options:\n  \
          -A, -e         Select all processes\n  \
          -a             Select all with a tty, except session leaders\n  \
          -f             Do full-format listing\n  \
          aux, -aux      BSD-style process listing\n  \
          -p, --pid <PID>   Filter by process ID (comma-separated or repeated)\n  \
          -C <name>      Filter by process name or command line (substring)\n  \
          --sort <field> Sort by field: [-]cpu, [-]mem, [-]pid\n  \
          -h, --help     Display this help and exit\n  \
          -V, --version  Output version information and exit"
    );
}

fn print_version() {
    println!("ps (veloterm) 0.1.0");
}

fn parse_args(args: &[OsString]) -> Option<PsOptions> {
    let mut opts = PsOptions::default();
    let mut iter = args.iter().map(|s| s.to_string_lossy().into_owned());

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return None;
            }
            "-V" | "--version" => {
                print_version();
                return None;
            }
            "-e" | "-A" | "-a" => {
                // Windows 下已默认展示全部进程
            }
            "-f" => {
                opts.format = OutputFormat::Full;
            }
            "-ef" => {
                opts.format = OutputFormat::Full;
            }
            "aux" | "-aux" => {
                opts.format = OutputFormat::Aux;
                opts.sort_key = SortKey::Cpu;
                opts.sort_descending = true;
            }
            "-p" | "--pid" => match iter.next() {
                Some(val) => collect_pids(&val, &mut opts),
                None => eprintln!("ps: option -p requires an argument"),
            },
            "-C" => match iter.next() {
                Some(val) => opts.filter_comm = Some(val.to_lowercase()),
                None => eprintln!("ps: option -C requires an argument"),
            },
            arg if arg.starts_with("--sort=") => {
                let val = &arg["--sort=".len()..];
                parse_sort_spec(val, &mut opts);
            }
            "--sort" => {
                if let Some(val) = iter.next() {
                    parse_sort_spec(&val, &mut opts);
                }
            }
            // GNU 附值形态：--pid=123 / --command=xxx
            arg if arg.starts_with("--pid=") => collect_pids(&arg["--pid=".len()..], &mut opts),
            arg if arg.starts_with("--command=") => {
                opts.filter_comm = Some(arg["--command=".len()..].to_lowercase());
            }
            // GNU 附着短形态：-p1234 / -Cnginx（-p、-C 的分离形态已在上方精确匹配）。
            // 这两臂必须存在：旧版落到 `_ => {}` 被静默吞掉，筛选未生效却直接输出
            // 全量列表，用户表现为「搜了等于没搜」
            arg if arg.len() > 2 && arg.starts_with("-p") => collect_pids(&arg[2..], &mut opts),
            arg if arg.len() > 2 && arg.starts_with("-C") => {
                opts.filter_comm = Some(arg[2..].to_lowercase());
            }
            _ => {}
        }
    }

    Some(opts)
}

/// 解析逗号分隔的 PID 列表（-p / --pid 的各类形态共用）；非法项打警告不静默，
/// 否则「写了筛选却输出全量列表」会被误认为「搜不到」
fn collect_pids(spec: &str, opts: &mut PsOptions) {
    for part in spec.split(',') {
        match part.trim().parse::<u32>() {
            Ok(pid) => opts.filter_pids.push(pid),
            Err(_) => eprintln!("ps: invalid process id: {part}"),
        }
    }
}

/// -C 过滤语义：进程名（Windows 下为 exe 文件名，如 WeChat.exe）**或**完整命令行
/// 任一子串命中即可（忽略大小写）。只匹配进程名会让「按参数搜进程」落空
/// （如 java -jar server.jar 里搜 server.jar），这正是 Windows 下最常见的搜不到
#[inline(always)]
fn matches_filter(name: &str, command: &str, filter: &str) -> bool {
    name.to_lowercase().contains(filter) || command.to_lowercase().contains(filter)
}

fn parse_sort_spec(val: &str, opts: &mut PsOptions) {
    let (desc, key) = if let Some(stripped) = val.strip_prefix('-') {
        (true, stripped)
    } else if let Some(stripped) = val.strip_prefix('+') {
        (false, stripped)
    } else {
        (false, val)
    };

    match key.to_lowercase().as_str() {
        "cpu" | "%cpu" => {
            opts.sort_key = SortKey::Cpu;
            opts.sort_descending = desc;
        }
        "mem" | "%mem" | "rss" | "vsz" => {
            opts.sort_key = SortKey::Mem;
            opts.sort_descending = desc;
        }
        "pid" => {
            opts.sort_key = SortKey::Pid;
            opts.sort_descending = desc;
        }
        _ => {}
    }
}

struct ProcessRow {
    pid: u32,
    ppid: u32,
    cpu_usage: f32,
    mem_bytes: u64,
    mem_percent: f32,
    vsz_bytes: u64,
    status: &'static str,
    run_time_secs: u64,
    command: String,
}

pub fn run(args: &[OsString]) -> i32 {
    let Some(opts) = parse_args(args) else {
        return 0;
    };

    let mut sys = System::new();
    // 首次刷新后微延时并再次刷新，使 sysinfo 能够正确算出 CPU 使用率增量。
    // 刷新走 crate 级 helper：默认 `refresh_processes` 不含 cmd，不刷新则
    // p.cmd() 恒为空，COMMAND 列会退化为进程名、-C 无法按参数匹配
    crate::refresh_all_processes(&mut sys);
    std::thread::sleep(std::time::Duration::from_millis(120));
    crate::refresh_all_processes(&mut sys);

    let total_mem = sys.total_memory();

    let mut rows: Vec<ProcessRow> = sys
        .processes()
        .iter()
        .filter_map(|(&pid, p)| {
            let pid_u32 = pid.as_u32();
            if !opts.filter_pids.is_empty() && !opts.filter_pids.contains(&pid_u32) {
                return None;
            }

            let name = p.name().to_string_lossy();
            let cmd_parts = p.cmd();
            let command = if !cmd_parts.is_empty() {
                cmd_parts
                    .iter()
                    .map(|s| s.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                name.to_string()
            };

            if let Some(ref filter) = opts.filter_comm
                && !matches_filter(&name, &command, filter)
            {
                return None;
            }

            let mem_bytes = p.memory();
            let mem_percent = if total_mem > 0 {
                (mem_bytes as f64 / total_mem as f64 * 100.0) as f32
            } else {
                0.0
            };

            let ppid_u32 = p.parent().map(|p| p.as_u32()).unwrap_or(0);

            Some(ProcessRow {
                pid: pid_u32,
                ppid: ppid_u32,
                cpu_usage: p.cpu_usage(),
                mem_bytes,
                mem_percent,
                vsz_bytes: p.virtual_memory(),
                status: crate::status_code(p.status()),
                run_time_secs: p.run_time(),
                command,
            })
        })
        .collect();

    // 排序
    rows.sort_by(|a, b| {
        let ordering = match opts.sort_key {
            SortKey::Pid => a.pid.cmp(&b.pid),
            SortKey::Cpu => a
                .cpu_usage
                .partial_cmp(&b.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal),
            SortKey::Mem => a.mem_bytes.cmp(&b.mem_bytes),
        };
        if opts.sort_descending {
            ordering.reverse()
        } else {
            ordering
        }
    });

    // 格式化输出
    match opts.format {
        OutputFormat::Aux => {
            println!(
                "{:<8} {:>7} {:>6} {:>6} {:>10} {:>10} {:<6} {:>8} COMMAND",
                "USER", "PID", "%CPU", "%MEM", "VSZ(KB)", "RSS(KB)", "STAT", "TIME"
            );
            for r in rows {
                let time_str = format_time(r.run_time_secs);
                let vsz_kb = r.vsz_bytes / 1024;
                let rss_kb = r.mem_bytes / 1024;
                println!(
                    "{:<8} {:>7} {:>6.1} {:>6.1} {:>10} {:>10} {:<6} {:>8} {}",
                    "-",
                    r.pid,
                    r.cpu_usage,
                    r.mem_percent,
                    vsz_kb,
                    rss_kb,
                    r.status,
                    time_str,
                    r.command
                );
            }
        }
        OutputFormat::Full => {
            println!(
                "{:<8} {:>7} {:>7} {:>6} {:>8} CMD",
                "UID", "PID", "PPID", "%CPU", "TIME"
            );
            for r in rows {
                let time_str = format_time(r.run_time_secs);
                println!(
                    "{:<8} {:>7} {:>7} {:>6.1} {:>8} {}",
                    "-", r.pid, r.ppid, r.cpu_usage, time_str, r.command
                );
            }
        }
        OutputFormat::Default => {
            println!(
                "{:>7} {:>7} {:>6} {:>6} {:>9} {:<8} COMMAND",
                "PID", "PPID", "%CPU", "%MEM", "RSS(MB)", "STAT"
            );
            for r in rows {
                let rss_mb = r.mem_bytes as f64 / (1024.0 * 1024.0);
                println!(
                    "{:>7} {:>7} {:>6.1} {:>6.1} {:>9.1} {:<8} {}",
                    r.pid, r.ppid, r.cpu_usage, r.mem_percent, rss_mb, r.status, r.command
                );
            }
        }
    }

    0
}

fn format_time(secs: u64) -> String {
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;
    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, mins, s)
    } else {
        format!("{:02}:{:02}", mins, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysinfo::ProcessStatus;

    fn parse(v: &[&str]) -> Option<PsOptions> {
        let args: Vec<OsString> = v.iter().map(OsString::from).collect();
        parse_args(&args)
    }

    #[test]
    fn help_and_version_short_circuit() {
        assert!(parse(&["-h"]).is_none());
        assert!(parse(&["--help"]).is_none());
        assert!(parse(&["-V"]).is_none());
        assert!(parse(&["--version"]).is_none());
    }

    #[test]
    fn defaults_are_pid_ascending() {
        let opts = parse(&[]).unwrap();
        assert_eq!(opts.format, OutputFormat::Default);
        assert_eq!(opts.sort_key, SortKey::Pid);
        assert!(!opts.sort_descending);
        assert!(opts.filter_pids.is_empty());
        assert!(opts.filter_comm.is_none());
    }

    #[test]
    fn aux_implies_cpu_descending() {
        let opts = parse(&["aux"]).unwrap();
        assert_eq!(opts.format, OutputFormat::Aux);
        assert_eq!(opts.sort_key, SortKey::Cpu);
        assert!(opts.sort_descending);
    }

    #[test]
    fn pid_filter_accepts_comma_and_repeat() {
        let opts = parse(&["-p", "1,2", "--pid", "3"]).unwrap();
        assert_eq!(opts.filter_pids, vec![1, 2, 3]);
    }

    /// GNU 附值形态：-p1234 / -Cnginx / --pid=123 / --command=xxx
    /// 旧版这些形态会被解析器静默吞掉，筛选不生效却输出全量列表
    #[test]
    fn attached_and_equals_value_forms() {
        let opts = parse(&["-p1,2", "--pid=3"]).unwrap();
        assert_eq!(opts.filter_pids, vec![1, 2, 3]);
        let opts = parse(&["-Cnginx"]).unwrap();
        assert_eq!(opts.filter_comm.as_deref(), Some("nginx"));
        let opts = parse(&["--command=Node"]).unwrap();
        assert_eq!(opts.filter_comm.as_deref(), Some("node"));
    }

    /// -C 匹配语义：进程名或命令行任一子串命中（忽略大小写）
    #[test]
    fn filter_matches_name_or_command_line() {
        // Windows 常见：搜 exe 名或命令行参数
        assert!(matches_filter("WeChat.exe", r"C:\Apps\WeChat.exe", "wechat"));
        assert!(matches_filter("java.exe", "java -jar server.jar", "server.jar"));
        assert!(!matches_filter("bash", "bash -l", "python"));
    }

    #[test]
    fn sort_spec_direction_prefix() {
        let opts = parse(&["--sort=-mem"]).unwrap();
        assert_eq!(opts.sort_key, SortKey::Mem);
        assert!(opts.sort_descending);
        let opts = parse(&["--sort", "+cpu"]).unwrap();
        assert_eq!(opts.sort_key, SortKey::Cpu);
        assert!(!opts.sort_descending);
    }

    #[test]
    fn status_codes_are_short_and_nonempty() {
        // STAT 是定宽列，任何状态都必须映射为 1–2 字符的短码
        let all = [
            ProcessStatus::Idle,
            ProcessStatus::Run,
            ProcessStatus::Sleep,
            ProcessStatus::Stop,
            ProcessStatus::Zombie,
            ProcessStatus::Tracing,
            ProcessStatus::Dead,
            ProcessStatus::Wakekill,
            ProcessStatus::Waking,
            ProcessStatus::Parked,
            ProcessStatus::LockBlocked,
            ProcessStatus::UninterruptibleDiskSleep,
            ProcessStatus::Unknown(0),
        ];
        for st in all {
            let code = crate::status_code(st);
            assert!(!code.is_empty() && code.len() <= 2, "{st:?} -> {code:?}");
        }
    }
}
