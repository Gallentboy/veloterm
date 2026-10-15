//! top 命令实现（Windows 与跨平台终端实时系统与进程监控）
//!
//! 依托 `sysinfo` 收集系统及进程指标，依托 `crossterm` 进行终端实时交互渲染。
//!
//! 支持功能：
//! - 实时刷新 CPU、内存、交换分区与进程列表
//! - 快捷键交互：
//!   - `q` / `Esc`：退出
//!   - `c`：按 CPU 使用率排序
//!   - `m`：按 内存使用率排序
//!   - `p`：按 PID 排序
//!   - `↑` / `↓` / `k` / `j`：上下滚动进程列表
//!   - `PageUp` / `PageDown`：翻页滚动
//!   - `Space`：立即刷新
//! - 命令行参数：
//!   - `-d <SECS>`：刷新间隔（默认 2.0 秒）
//!   - `-n <COUNT>`：运行指定轮数后退出
//!   - `-b`：批处理模式（无交互，单次或多次文本输出）
//!   - `-p <PID>`：仅监控指定 PID
//!   - `-h, --help`：帮助
//!   - `-V, --version`：版本信息

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use std::ffi::OsString;
use std::io::{IsTerminal, Write, stdout};
use std::time::{Duration, Instant};
use sysinfo::{ProcessesToUpdate, System};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Cpu,
    Mem,
    Pid,
}

#[derive(Debug)]
struct TopOptions {
    delay: Duration,
    max_iterations: Option<usize>,
    batch_mode: bool,
    filter_pid: Option<u32>,
}

impl Default for TopOptions {
    fn default() -> Self {
        Self {
            delay: Duration::from_millis(2000),
            max_iterations: None,
            batch_mode: false,
            filter_pid: None,
        }
    }
}

fn print_help() {
    println!(
        "Usage: top [options]\n\n\
        Options:\n  \
          -d <seconds>   Specifies the delay between screen updates (default: 2.0)\n  \
          -n <number>    Specifies the maximum number of iterations before ending\n  \
          -b             Batch mode operation (suitable for logging/piping)\n  \
          -p <PID>       Monitor only the specified process ID\n  \
          -h, --help     Display this help and exit\n  \
          -V, --version  Output version information and exit\n\n\
        Interactive Keys:\n  \
          q, Esc         Quit\n  \
          c              Sort by CPU%\n  \
          m              Sort by MEM%\n  \
          p              Sort by PID\n  \
          Up/Down, k/j   Scroll process list\n  \
          Space          Refresh display immediately"
    );
}

fn print_version() {
    println!("top (veloterm) 0.1.0");
}

/// 刷新间隔的合法区间（秒）：下限避免忙循环，上限避免 `Duration` 溢出。
const MIN_DELAY_SECS: f64 = 0.1;
const MAX_DELAY_SECS: f64 = 3600.0;

fn parse_args(args: &[OsString]) -> Option<TopOptions> {
    let mut opts = TopOptions::default();
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
            "-b" => {
                opts.batch_mode = true;
            }
            "-d" => {
                if let Some(val) = iter.next()
                    && let Ok(secs) = val.parse::<f64>()
                    // inf / NaN 会让 from_secs_f64 panic；非有限值直接忽略，保留默认间隔
                    && secs.is_finite()
                {
                    opts.delay =
                        Duration::from_secs_f64(secs.clamp(MIN_DELAY_SECS, MAX_DELAY_SECS));
                }
            }
            "-n" => {
                if let Some(val) = iter.next()
                    && let Ok(n) = val.parse::<usize>()
                {
                    opts.max_iterations = Some(n);
                }
            }
            "-p" => {
                if let Some(val) = iter.next()
                    && let Ok(pid) = val.parse::<u32>()
                {
                    opts.filter_pid = Some(pid);
                }
            }
            _ => {}
        }
    }

    Some(opts)
}

struct ProcessInfo {
    pid: u32,
    ppid: u32,
    cpu_usage: f32,
    mem_bytes: u64,
    mem_percent: f32,
    status: &'static str,
    run_time_secs: u64,
    command: String,
}

/// 只刷新 top 实际用到的指标：内存（含 swap）与 CPU 使用率，外加进程表。
///
/// 相比 `System::refresh_all()`，省去磁盘 / 网络 / 组件等无关采集——交互模式下
/// 每轮刷新都会调用，这些采集纯属浪费。首次采样须由 `System::new_all()` 完成，
/// 之后两次采样之间的时间差才让 sysinfo 算得出 CPU 使用率增量。
fn refresh_metrics(sys: &mut System) {
    sys.refresh_memory();
    sys.refresh_cpu_usage();
    sys.refresh_processes(ProcessesToUpdate::All);
}

/// RAII 守卫：确保退出时恢复终端状态（退出 AlternateScreen 与 RawMode，恢复光标）
struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    fn new() -> Self {
        let _ = terminal::enable_raw_mode();
        let mut out = stdout();
        let _ = execute!(out, terminal::EnterAlternateScreen, cursor::Hide);
        Self { active: true }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.active {
            let mut out = stdout();
            let _ = execute!(out, cursor::Show, terminal::LeaveAlternateScreen);
            let _ = terminal::disable_raw_mode();
        }
    }
}

pub fn run(args: &[OsString]) -> i32 {
    let Some(opts) = parse_args(args) else {
        return 0;
    };

    let is_term = stdout().is_terminal();
    if opts.batch_mode || !is_term {
        return run_batch(opts);
    }

    run_interactive(opts)
}

fn run_batch(opts: TopOptions) -> i32 {
    let mut sys = System::new_all();
    let max_iter = opts.max_iterations.unwrap_or(1);

    for i in 0..max_iter {
        // 首次采样已由 new_all() 完成；间隔后再采一次，sysinfo 才能算出 CPU 使用率增量
        std::thread::sleep(Duration::from_millis(150));
        refresh_metrics(&mut sys);

        let rows = collect_processes(&sys, opts.filter_pid, SortKey::Cpu);

        println!(
            "top - iteration {}/{} | Mem: {}/{} MB | Swap: {}/{} MB",
            i + 1,
            max_iter,
            sys.used_memory() / (1024 * 1024),
            sys.total_memory() / (1024 * 1024),
            sys.used_swap() / (1024 * 1024),
            sys.total_swap() / (1024 * 1024)
        );
        println!(
            "{:>7} {:>7} {:>6} {:>6} {:>9} {:<8} COMMAND",
            "PID", "PPID", "%CPU", "%MEM", "RSS(MB)", "STAT"
        );
        for r in rows.iter().take(20) {
            let rss_mb = r.mem_bytes as f64 / (1024.0 * 1024.0);
            println!(
                "{:>7} {:>7} {:>6.1} {:>6.1} {:>9.1} {:<8} {}",
                r.pid, r.ppid, r.cpu_usage, r.mem_percent, rss_mb, r.status, r.command
            );
        }
        println!();

        if i + 1 < max_iter {
            std::thread::sleep(opts.delay);
        }
    }

    0
}

fn run_interactive(opts: TopOptions) -> i32 {
    let _guard = TerminalGuard::new();

    let mut sys = System::new_all();
    let mut sort_key = SortKey::Cpu;
    let mut scroll_offset: usize = 0;
    let mut iteration: usize = 0;

    // 初始采样：new_all() 已完成首次采集，间隔后再采一次才能得到 CPU 使用率增量
    std::thread::sleep(Duration::from_millis(100));
    refresh_metrics(&mut sys);

    let mut last_refresh = Instant::now();
    loop {
        if let Some(max) = opts.max_iterations
            && iteration >= max
        {
            break;
        }

        let term_size = terminal::size().unwrap_or((80, 24));
        let width = term_size.0 as usize;
        let height = term_size.1 as usize;

        let rows = collect_processes(&sys, opts.filter_pid, sort_key);
        render_screen(&sys, &rows, sort_key, scroll_offset, width, height);

        iteration += 1;

        // 等待键盘输入或超时刷新
        let elapsed = last_refresh.elapsed();
        let remaining_delay = if elapsed < opts.delay {
            opts.delay - elapsed
        } else {
            Duration::from_millis(10)
        };

        // `Space` 置位后本轮立即重新采样并刷新，不等下一个周期
        let mut force_refresh = false;
        if event::poll(remaining_delay).unwrap_or(false)
            && let Ok(Event::Key(key)) = event::read()
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => break,
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    sort_key = SortKey::Cpu;
                    scroll_offset = 0;
                }
                KeyCode::Char('m') | KeyCode::Char('M') => {
                    sort_key = SortKey::Mem;
                    scroll_offset = 0;
                }
                KeyCode::Char('p') | KeyCode::Char('P') => {
                    sort_key = SortKey::Pid;
                    scroll_offset = 0;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    scroll_offset = scroll_offset.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if scroll_offset + 5 < rows.len() {
                        scroll_offset += 1;
                    }
                }
                KeyCode::PageUp => {
                    scroll_offset = scroll_offset.saturating_sub(10);
                }
                KeyCode::PageDown => {
                    if scroll_offset + 10 < rows.len() {
                        scroll_offset += 10;
                    }
                }
                KeyCode::Char(' ') => force_refresh = true,
                _ => {}
            }
        }

        if force_refresh || last_refresh.elapsed() >= opts.delay {
            refresh_metrics(&mut sys);
            last_refresh = Instant::now();
        }
    }

    0
}

fn collect_processes(sys: &System, filter_pid: Option<u32>, sort_key: SortKey) -> Vec<ProcessInfo> {
    let total_mem = sys.total_memory();

    let mut rows: Vec<ProcessInfo> = sys
        .processes()
        .iter()
        .filter_map(|(&pid, p)| {
            let pid_u32 = pid.as_u32();
            if let Some(target) = filter_pid
                && pid_u32 != target
            {
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
                name.into_owned()
            };

            let mem_bytes = p.memory();
            let mem_percent = if total_mem > 0 {
                (mem_bytes as f64 / total_mem as f64 * 100.0) as f32
            } else {
                0.0
            };

            let ppid_u32 = p.parent().map(|p| p.as_u32()).unwrap_or(0);

            Some(ProcessInfo {
                pid: pid_u32,
                ppid: ppid_u32,
                cpu_usage: p.cpu_usage(),
                mem_bytes,
                mem_percent,
                status: crate::status_code(p.status()),
                run_time_secs: p.run_time(),
                command,
            })
        })
        .collect();

    rows.sort_by(|a, b| match sort_key {
        SortKey::Cpu => b
            .cpu_usage
            .partial_cmp(&a.cpu_usage)
            .unwrap_or(std::cmp::Ordering::Equal),
        SortKey::Mem => b.mem_bytes.cmp(&a.mem_bytes),
        SortKey::Pid => a.pid.cmp(&b.pid),
    });

    rows
}

fn render_screen(
    sys: &System,
    rows: &[ProcessInfo],
    sort_key: SortKey,
    scroll_offset: usize,
    width: usize,
    height: usize,
) {
    let mut out = stdout();
    let _ = execute!(out, cursor::MoveTo(0, 0), Clear(ClearType::All));

    let uptime = System::uptime();
    let up_hours = uptime / 3600;
    let up_mins = (uptime % 3600) / 60;

    let sort_label = match sort_key {
        SortKey::Cpu => "%CPU",
        SortKey::Mem => "%MEM",
        SortKey::Pid => "PID",
    };

    // 第一行：系统概览
    let line1 = format!(
        "top - up {:02}:{:02},  {} tasks,  Sort: {}",
        up_hours,
        up_mins,
        rows.len(),
        sort_label
    );
    let _ = execute!(
        out,
        Print(truncate_line(&line1, width)),
        cursor::MoveToNextLine(1)
    );

    // 第二行：内存与交换分区
    let total_ram_mb = sys.total_memory() / (1024 * 1024);
    let used_ram_mb = sys.used_memory() / (1024 * 1024);
    let total_swap_mb = sys.total_swap() / (1024 * 1024);
    let used_swap_mb = sys.used_swap() / (1024 * 1024);

    let line2 = format!(
        "MiB Mem : {:>6} total, {:>6} used | Swap: {:>6} total, {:>6} used",
        total_ram_mb, used_ram_mb, total_swap_mb, used_swap_mb
    );
    let _ = execute!(
        out,
        Print(truncate_line(&line2, width)),
        cursor::MoveToNextLine(1)
    );

    // 第三行：快捷键提示
    let line3 = "[q]: Quit | [c]: Sort CPU | [m]: Sort MEM | [p]: Sort PID | [↑/↓]: Scroll";
    let _ = execute!(
        out,
        SetForegroundColor(Color::DarkGrey),
        Print(truncate_line(line3, width)),
        ResetColor,
        cursor::MoveToNextLine(1)
    );

    // 第四行：表头（高亮背景）
    let header = format!(
        "{:>7} {:>7} {:>6} {:>6} {:>9} {:<8} {:>8} {}",
        "PID", "PPID", "%CPU", "%MEM", "RSS(MB)", "STAT", "TIME", "COMMAND"
    );
    let _ = execute!(
        out,
        SetBackgroundColor(Color::DarkBlue),
        SetForegroundColor(Color::White),
        Print(pad_or_truncate(&header, width)),
        ResetColor,
        cursor::MoveToNextLine(1)
    );

    // 进程数据行
    let header_lines = 4;
    // 再留一行空白：每行末尾都会 MoveToNextLine，铺满 height 行会把光标顶出屏幕触发滚动
    let available_rows = height.saturating_sub(header_lines + 1);

    for r in rows.iter().skip(scroll_offset).take(available_rows) {
        let rss_mb = r.mem_bytes as f64 / (1024.0 * 1024.0);
        let time_str = format_time(r.run_time_secs);
        let line = format!(
            "{:>7} {:>7} {:>6.1} {:>6.1} {:>9.1} {:<8} {:>8} {}",
            r.pid, r.ppid, r.cpu_usage, r.mem_percent, rss_mb, r.status, time_str, r.command
        );
        let _ = execute!(
            out,
            Print(truncate_line(&line, width)),
            cursor::MoveToNextLine(1)
        );
    }

    let _ = out.flush();
}

/// 按字符边界安全截断：`max_len` 是字节上限，若恰好落在多字节字符中间则回退到
/// 最近的前一个边界。直接 `&s[..max_len]` 会在 CJK 等非 ASCII 内容上 panic。
fn truncate_line(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn pad_or_truncate(s: &str, width: usize) -> String {
    let truncated = truncate_line(s, width);
    let mut out = String::with_capacity(width);
    out.push_str(truncated);
    out.extend(std::iter::repeat_n(' ', width - truncated.len()));
    out
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

    fn parse(v: &[&str]) -> Option<TopOptions> {
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
    fn defaults_match_documented_values() {
        let opts = parse(&[]).unwrap();
        assert_eq!(opts.delay, Duration::from_millis(2000));
        assert_eq!(opts.max_iterations, None);
        assert!(!opts.batch_mode);
        assert_eq!(opts.filter_pid, None);
    }

    #[test]
    fn delay_never_panics_on_non_finite_or_extreme() {
        // inf / NaN 必须被忽略（from_secs_f64 对它们会 panic），保留默认值
        assert_eq!(
            parse(&["-d", "inf"]).unwrap().delay,
            Duration::from_millis(2000)
        );
        assert_eq!(
            parse(&["-d", "-inf"]).unwrap().delay,
            Duration::from_millis(2000)
        );
        assert_eq!(
            parse(&["-d", "NaN"]).unwrap().delay,
            Duration::from_millis(2000)
        );
        // 下溢与负数夹到下限
        assert_eq!(
            parse(&["-d", "0"]).unwrap().delay,
            Duration::from_millis(100)
        );
        assert_eq!(
            parse(&["-d", "-5"]).unwrap().delay,
            Duration::from_millis(100)
        );
        // 天文数字夹到上限
        assert_eq!(
            parse(&["-d", "1e300"]).unwrap().delay,
            Duration::from_secs(3600)
        );
        // 正常值原样保留
        assert_eq!(
            parse(&["-d", "0.5"]).unwrap().delay,
            Duration::from_millis(500)
        );
    }

    #[test]
    fn truncate_line_never_splits_multibyte_char() {
        // 每个汉字 3 字节：任何字节上限都不得 panic，且结果必须是原串前缀
        let s = "中文命令名称abc";
        for max in 0..=s.len() + 2 {
            let cut = truncate_line(s, max);
            assert!(s.starts_with(cut), "max={max} cut={cut:?}");
            assert!(cut.len() <= max);
        }
        assert_eq!(truncate_line(s, 3), "中");
        assert_eq!(truncate_line(s, 4), "中"); // 4 落在「文」内部 → 回退到边界
        assert_eq!(truncate_line("ascii", 100), "ascii");
        assert_eq!(truncate_line("ascii", 2), "as");
    }

    #[test]
    fn pad_or_truncate_never_splits_multibyte_char() {
        for w in 0..40 {
            let out = pad_or_truncate("中文abc", w);
            assert!(out.len() <= w, "width={w} out={out:?}");
        }
        assert_eq!(pad_or_truncate("ab", 4), "ab  ");
        assert_eq!(pad_or_truncate("abcd", 2), "ab");
    }
}
