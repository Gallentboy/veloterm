//! dog, the command-line DNS client.

#![warn(deprecated_in_future)]
#![warn(future_incompatible)]
#![warn(missing_copy_implementations)]
#![warn(missing_docs)]
#![warn(nonstandard_style)]
#![warn(rust_2018_compatibility)]
#![warn(rust_2018_idioms)]
#![warn(single_use_lifetimes)]
#![warn(trivial_casts, trivial_numeric_casts)]
#![warn(unused)]

#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::enum_glob_use)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::upper_case_acronyms)]
#![allow(clippy::wildcard_imports)]

#![deny(unsafe_code)]

/*!
VeloTerm vendor 新增：上游本 crate 仅有 bin 入口（`src/main.rs` 持有全部模块
声明与 `main()` 主逻辑）。为把 dog 内嵌进 VeloTerm `coreutils` 多呼叫二进制，
新增本文件：承接原 `main.rs` 的模块声明与主逻辑，暴露 `dog_main()` /
`dog_main_from()` 两个入口（同 vendor/eza、vendor/zoxide 的约定）；
`main.rs` 保留为独立 dog 二进制的薄壳入口。所有 `std::process::exit` 改为
返回退出码，避免嵌入式多呼叫二进制与单测进程被直接杀死。
*/

mod colours;
mod connect;
mod hints;
mod logger;
mod output;
mod requests;
mod resolve;
mod table;
mod txid;

mod options;
use self::options::*;

use std::ffi::OsStr;

use log::*;

/// VeloTerm coreutils 入口：以真实进程 argv 执行 dog 并返回进程退出码。
///
/// 注意：本函数直接读取真实进程 argv，`dog_main_from` 内部会跳过首元素
/// （argv[0] 程序名），适合「以 dog 短名（硬链接）启动」的场景；
/// `coreutils dog ...` 子命令模式同样适用（首元素恰为命令名）。
pub fn dog_main() -> i32 {
    dog_main_from(std::env::args_os())
}

/// VeloTerm coreutils 可注入入口：以 `args`（首元素为程序名，将被跳过——
/// 上游 `Options::getopts` 期望的是不含 argv[0] 的参数表）执行 dog 并返回
/// 进程退出码，任何路径都不调用 `std::process::exit`。
pub fn dog_main_from<I>(args: I) -> i32
where
    I: IntoIterator,
    I::Item: AsRef<OsStr>,
{
    logger::configure(std::env::var_os("DOG_DEBUG"));

    #[cfg(windows)]
    if let Err(e) = ansi_term::enable_ansi_support() {
        warn!("Failed to enable ANSI support: {}", e);
    }

    match Options::getopts(args.into_iter().skip(1)) {
        OptionsResult::Ok(options) => {
            info!("Running with options -> {:#?}", options);
            match disabled_feature_check(&options) {
                Some(code) => code,
                None => run(options),
            }
        }

        OptionsResult::Help(help_reason, use_colours) => {
            if use_colours.should_use_colours() {
                print!("{}", include_str!(concat!(env!("OUT_DIR"), "/usage.pretty.txt")));
            }
            else {
                print!("{}", include_str!(concat!(env!("OUT_DIR"), "/usage.bland.txt")));
            }

            if help_reason == HelpReason::NoDomains {
                exits::OPTIONS_ERROR
            }
            else {
                exits::SUCCESS
            }
        }

        OptionsResult::Version(use_colours) => {
            if use_colours.should_use_colours() {
                print!("{}", include_str!(concat!(env!("OUT_DIR"), "/version.pretty.txt")));
            }
            else {
                print!("{}", include_str!(concat!(env!("OUT_DIR"), "/version.bland.txt")));
            }

            exits::SUCCESS
        }

        OptionsResult::InvalidOptionsFormat(oe) => {
            eprintln!("dog: Invalid options: {}", oe);
            exits::OPTIONS_ERROR
        }

        OptionsResult::InvalidOptions(why) => {
            eprintln!("dog: Invalid options: {}", why);
            exits::OPTIONS_ERROR
        }
    }
}


/// Runs dog with some options, returning the status to exit with.
fn run(Options { requests, format, measure_time }: Options) -> i32 {
    use std::time::Instant;

    let should_show_opt = requests.edns.should_show();

    let mut responses = Vec::new();
    let timer = if measure_time { Some(Instant::now()) } else { None };

    let mut errored = false;

    let local_host_hints = match hints::LocalHosts::load() {
        Ok(lh) => lh,
        Err(e) => {
            warn!("Error loading local host hints: {}", e);
            hints::LocalHosts::default()
        }
    };

    for hostname_in_query in &requests.inputs.domains {
        if local_host_hints.contains(hostname_in_query) {
            eprintln!("warning: domain '{}' also exists in hosts file", hostname_in_query);
        }
    }

    let request_tuples = match requests.generate() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Unable to obtain resolver: {}", e);
            return exits::SYSTEM_ERROR;
        }
    };

    for (transport, request_list) in request_tuples {
        let request_list_len = request_list.len();
        for (i, request) in request_list.into_iter().enumerate() {
            let result = transport.send(&request);

            match result {
                Ok(mut response) => {
                    if response.flags.error_code.is_some() && i != request_list_len - 1 {
                        continue;
                    }

                    if ! should_show_opt {
                        response.answers.retain(dns::Answer::is_standard);
                        response.authorities.retain(dns::Answer::is_standard);
                        response.additionals.retain(dns::Answer::is_standard);
                    }

                    responses.push(response);
                    break;
                }
                Err(e) => {
                    format.print_error(e);
                    errored = true;
                    break;
                }
            }
        }
    }

    let duration = timer.map(|t| t.elapsed());
    if format.print(responses, duration) {
        if errored {
            exits::NETWORK_ERROR
        }
        else {
            exits::SUCCESS
        }
    }
    else {
        exits::NO_SHORT_RESULTS
    }
}


/// Checks whether the options contain parameters that will cause dog to fail
/// because the feature is disabled, returning the exit code to use if so.
#[allow(unused)]
fn disabled_feature_check(options: &Options) -> Option<i32> {
    use crate::connect::TransportType;

    #[cfg(all(not(feature = "with_tls"), not(feature = "with_rustls_tls")))]
    if options.requests.inputs.transport_types.contains(&TransportType::TLS) {
        eprintln!("dog: Cannot use '--tls': This version of dog has been compiled without TLS support");
        return Some(exits::OPTIONS_ERROR);
    }

    #[cfg(all(not(feature = "with_https"), not(feature = "with_rustls_https")))]
    if options.requests.inputs.transport_types.contains(&TransportType::HTTPS) {
        eprintln!("dog: Cannot use '--https': This version of dog has been compiled without HTTPS support");
        return Some(exits::OPTIONS_ERROR);
    }

    None
}


/// The possible status numbers dog can exit with.
mod exits {

    /// Exit code for when everything turns out OK.
    pub const SUCCESS: i32 = 0;

    /// Exit code for when there was at least one network error during execution.
    pub const NETWORK_ERROR: i32 = 1;

    /// Exit code for when there is no result from the server when running in
    /// short mode. This can be any received server error, not just `NXDOMAIN`.
    pub const NO_SHORT_RESULTS: i32 = 2;

    /// Exit code for when the command-line options are invalid.
    pub const OPTIONS_ERROR: i32 = 3;

    /// Exit code for when the system network configuration could not be determined.
    pub const SYSTEM_ERROR: i32 = 4;
}
