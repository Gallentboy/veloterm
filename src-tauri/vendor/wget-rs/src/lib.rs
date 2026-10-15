/*!
The library entry point into wget-rs.

VeloTerm vendor 新增：本文件把原 `src/main.rs` 的主逻辑 librarify 到此，
使 wget 命令可通过 `wget_rs::wget_main()` / `wget_rs::wget_main_from()`
内嵌进 VeloTerm `coreutils` 多呼叫二进制；`src/main.rs` 保留为独立
`wget-rs` 二进制的薄壳入口。
*/

pub mod cli;
pub mod download;
pub mod hash;

use std::ffi::OsString;

/// VeloTerm coreutils 入口：执行 wget 并返回进程退出码
/// （0 成功，1 下载/哈希失败；clap 参数错误仍由 clap 直接 exit(2)）。
///
/// 直接读取真实进程 argv（首元素 argv[0] 作为程序名被 clap 跳过），
/// 因此只适合在「以 wget 短名（硬链接）启动」的场景调用；
/// `coreutils wget ...` 子命令分发请改用可注入的 `wget_main_from()`。
pub fn wget_main() -> i32 {
    wget_main_from(std::env::args_os())
}

/// VeloTerm coreutils 可注入入口：以 `args`（首元素为程序名，将被 clap
/// 作为 bin name 跳过）执行 wget 并返回进程退出码
/// （0 成功，1 下载/哈希失败；clap 参数错误仍由 clap 直接 exit(2)）。
///
/// `coreutils wget <URL> ...` 子命令模式下可传入
/// `std::iter::once(OsString::from("wget")).chain(rest_args)`。
pub fn wget_main_from<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut args = cli::parse_args_from(args);

    // 如果没有协议，自动补全为 https://
    if !args.url.starts_with("http://") && !args.url.starts_with("https://") {
        args.url = format!("https://{}", args.url);
    }

    // 执行下载
    if let Err(e) = download::download_file(&args.url, &args.output, args.threads, args.continue_) {
        eprintln!("下载失败: {}", e);
        return 1;
    }

    // 确定下载的文件名
    let filename = match &args.output {
        Some(name) => name.clone(),
        None => {
            // 从URL推断文件名（与download.rs中的逻辑保持一致）
            args.url
                .split('/')
                .last()
                .filter(|s| !s.is_empty())
                .unwrap_or("output")
                .to_string()
        }
    };

    // 处理哈希相关功能
    if args.hash || args.verify_hash.is_some() {
        if let Some(expected_hash) = args.verify_hash {
            // 验证哈希值
            if let Err(e) = hash::verify_and_display(&filename, &expected_hash) {
                eprintln!("哈希验证失败: {}", e);
                return 1;
            }
        } else if args.hash {
            // 计算并显示所有哈希值
            match hash::calculate_all_hashes(&filename) {
                Ok(results) => hash::display_hash_results(&results, &filename),
                Err(e) => {
                    eprintln!("哈希计算失败: {}", e);
                    return 1;
                }
            }
        }
    }

    0
}
