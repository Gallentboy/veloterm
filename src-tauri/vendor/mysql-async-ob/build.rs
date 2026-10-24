//! veloterm 分叉新增：把编译时刻（epoch 秒 + UTC 人读形态）注入环境变量，
//! 供 example 启动横幅打印——真机排查时核对手里的 exe 是哪一次构建的产物
//!（分叉的上游没有 build.rs，此文件是本地补丁，升级换树时需保留）。
//!
//! 刻意不发 rerun-if-changed：走默认语义（包内任一文件变化即重跑），
//! 源码一变时间戳就是新的；源码没变时重跑与否不影响版本判定。

fn main() {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=OB_FORK_BUILD_EPOCH={secs}");
    println!("cargo:rustc-env=OB_FORK_BUILD_UTC={}", format_utc(secs));
}

/// epoch 秒 → `YYYY-MM-DD HH:MM:SS UTC`（Howard Hinnant 的 civil_from_days
/// 算法，days→(y,m,d) 纯整数运算，零依赖）
fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
