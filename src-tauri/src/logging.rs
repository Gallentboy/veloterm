use std::sync::atomic::{AtomicUsize, Ordering};

use log::{Level, LevelFilter};
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

/// 单个日志文件最大体积：20 MB
const MAX_LOG_FILE_SIZE_BYTES: u128 = 20 * 1024 * 1024;
const LOG_FILE_NAME: &str = "veloterm";

/// 全局原子日志等级阈值（默认 `Info` = 3）。
/// 编码映射与 `log::LevelFilter as usize` 保持一致：
/// Off = 0, Error = 1, Warn = 2, Info = 3, Debug = 4, Trace = 5
static CURRENT_LOG_LEVEL: AtomicUsize = AtomicUsize::new(LevelFilter::Info as usize);

fn usize_to_level_filter(raw: usize) -> LevelFilter {
    match raw {
        0 => LevelFilter::Off,
        1 => LevelFilter::Error,
        2 => LevelFilter::Warn,
        3 => LevelFilter::Info,
        4 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    }
}

fn parse_log_level(level: &str) -> Result<LevelFilter, String> {
    match level.trim().to_ascii_lowercase().as_str() {
        "error" => Ok(LevelFilter::Error),
        "warn" | "warning" => Ok(LevelFilter::Warn),
        "info" => Ok(LevelFilter::Info),
        "debug" => Ok(LevelFilter::Debug),
        "trace" => Ok(LevelFilter::Trace),
        other => Err(format!(
            "invalid log level '{other}', expected one of: trace, debug, info, warn, error"
        )),
    }
}

fn level_filter_to_str(filter: LevelFilter) -> &'static str {
    match filter {
        LevelFilter::Error => "error",
        LevelFilter::Warn => "warn",
        LevelFilter::Info | LevelFilter::Off => "info",
        LevelFilter::Debug => "debug",
        LevelFilter::Trace => "trace",
    }
}

pub fn current_level_filter() -> LevelFilter {
    usize_to_level_filter(CURRENT_LOG_LEVEL.load(Ordering::Relaxed))
}

pub fn apply_log_level_filter(filter: LevelFilter) {
    CURRENT_LOG_LEVEL.store(filter as usize, Ordering::SeqCst);
    log::set_max_level(filter);
}

/// 生成本地时区时间戳 `YYYY-MM-DD HH:MM:SS.mmm`（无括号，空格分隔）。
///
/// - 时区取 [`TimezoneStrategy::UseLocal`]（内部为 `OffsetDateTime::now_local()`），
///   避免插件默认的 `UseUtc` 让日志比本地时间少 8 小时；
/// - 逐字段取值走 `OffsetDateTime` 的**固有方法**，因此无需在 `Cargo.toml` 里
///   直接依赖 `time` crate；
/// - 本地时区不可判定时 `UseLocal` 内部会回退 UTC，这里不会 panic。
fn format_local_timestamp() -> String {
    let now = TimezoneStrategy::UseLocal.get_now();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        now.millisecond(),
    )
}

/// 构建 `tauri_plugin_log` 插件实例：
/// - 底层 `fern` 基础通道设为 `LevelFilter::Trace`（放行所有级别进入动态 filter）；
/// - 通过 `.filter(...)` 实时读取 `CURRENT_LOG_LEVEL` 原子变量决定是否输出；
/// - 同时输出至控制台 (`Stdout`) 与本地日志目录 (`LogDir { file_name: Some("veloterm") }`)，单文件上限 20MB 滚动；
/// - 时间戳使用**本地时区 + 毫秒**（插件默认是 UTC 且只到秒）。
pub fn build_log_plugin<R: Runtime>() -> TauriPlugin<R> {
    tauri_plugin_log::Builder::new()
        .level(LevelFilter::Trace)
        .max_file_size(MAX_LOG_FILE_SIZE_BYTES)
        .rotation_strategy(RotationStrategy::KeepOne)
        .targets([
            Target::new(TargetKind::Stdout),
            Target::new(TargetKind::LogDir {
                file_name: Some(LOG_FILE_NAME.into()),
            }),
        ])
        // 顺序要紧：`timezone_strategy()` 内部会重设 fern 的 format（见插件源码
        // `Builder::timezone_strategy` 的注释「overrides the format set in format」），
        // 所以必须先设时区、再用自定义 `format()` 覆盖回来，否则毫秒会被抹掉。
        .timezone_strategy(TimezoneStrategy::UseLocal)
        .format(|out, message, record| {
            // 字段顺序与插件默认格式保持一致：`[时间][target][LEVEL] message`
            out.finish(format_args!(
                "{}[{}][{}] {}",
                format_local_timestamp(),
                record.target(),
                record.level(),
                message,
            ))
        })
        .filter(|metadata| {
            let threshold = current_level_filter();
            let record_level: Level = metadata.level();
            record_level <= threshold
        })
        .build()
}

#[tauri::command]
pub fn set_log_level(level: String) -> Result<(), String> {
    let filter = parse_log_level(&level)?;
    apply_log_level_filter(filter);
    let level_str = level_filter_to_str(filter);

    if let Some(pool) = crate::db::global_pool() {
        let _ = crate::db::set_kv(&pool, "veloterm:settings:log-level", level_str);
    }

    log::info!(
        "[Rust Logger] log level dynamically updated to '{}'",
        level_str
    );
    log::debug!(
        "[Rust Logger] debug probe after switching log level to '{}'",
        level_str
    );

    Ok(())
}

#[tauri::command]
pub fn get_current_log_level() -> Result<String, String> {
    Ok(level_filter_to_str(current_level_filter()).to_string())
}

#[tauri::command]
pub fn get_log_file_path(app: AppHandle) -> Result<String, String> {
    let log_dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    let file_path = log_dir.join(format!("{LOG_FILE_NAME}.log"));
    Ok(file_path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 时间戳形态必须是 `[YYYY-MM-DD][HH:MM:SS.mmm]`（本地时区 + 毫秒）。
    #[test]
    fn local_timestamp_carries_milliseconds() {
        let stamp = format_local_timestamp();

        // `split_once("][")` 会把分隔符本身吃掉，所以 date_part 不含尾部 `]`
        // 注意：本项目 `clippy::panic = "deny"`，测试里也一律用 `expect` 而非 `panic!`
        let (date_part, rest) = stamp
            .split_once("][")
            .expect("timestamp must be `[date][time]`");
        let time_part = rest.strip_suffix(']').expect("timestamp must end with `]`");

        assert!(
            date_part.starts_with('['),
            "date part must start with `[`: {stamp}"
        );
        assert_eq!(
            date_part.len(),
            "[YYYY-MM-DD".len(),
            "unexpected date part: {stamp}"
        );
        assert_eq!(
            time_part.len(),
            "HH:MM:SS.mmm".len(),
            "unexpected time part: {stamp}"
        );

        let (hms, millis) = time_part
            .split_once('.')
            .expect("time part must carry milliseconds");
        assert_eq!(hms.len(), "HH:MM:SS".len(), "unexpected hms: {stamp}");
        assert_eq!(millis.len(), 3, "milliseconds must be 3 digits: {stamp}");
        assert!(
            millis.chars().all(|c| c.is_ascii_digit()),
            "milliseconds must be numeric: {stamp}"
        );

        // 逐段解析并做范围校验，兜住「格式对但值错」的情况
        // （转成定长数组后解构，避免 `indexing_slicing`——本项目 clippy 该项为 deny）
        let numbers = |s: &str, separators: &[char]| -> [u32; 3] {
            s.split(separators)
                .filter(|segment| !segment.is_empty())
                .map(|segment| segment.parse::<u32>().expect("segment must be numeric"))
                .collect::<Vec<_>>()
                .try_into()
                .expect("expected exactly 3 numeric segments")
        };

        let [year, month, day] = numbers(date_part, &['[', '-']);
        assert!((2020..=2100).contains(&year), "implausible year in {stamp}");
        assert!((1..=12).contains(&month), "implausible month in {stamp}");
        assert!((1..=31).contains(&day), "implausible day in {stamp}");

        let [hour, minute, second] = numbers(hms, &[':']);
        assert!(hour <= 23, "implausible hour in {stamp}");
        assert!(minute <= 59, "implausible minute in {stamp}");
        assert!(second <= 59, "implausible second in {stamp}");
    }
}
