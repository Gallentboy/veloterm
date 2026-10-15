// VeloTerm vendor 新增文件：上游 uutils/diffutils v0.5.0 只内置 diff 与 cmp 两个
// 命令（side-by-side 输出经 `diff -y` 触发），没有独立的 sdiff CLI。本文件为
// VeloTerm coreutils 多呼叫二进制补齐 sdiff：把 sdiff 风格的参数翻译成
// `diff --side-by-side` 风格后，复用 `diff::main` 的全部逻辑（同文件判断、
// 目录参数拼接、错误处理、退出码 0=相同 / 1=有差异 / 2=出错，与 GNU 一致）。
//
// 参数翻译表（GNU sdiff -> 本引擎支持的 diff 参数）：
//   -w N / -wN / --width=N   输出总宽度（GNU sdiff 默认 130，与引擎默认一致）
//   -t / --expand-tabs       制表符展开为空格（与 diff 语义完全一致，直接透传）
//   --tabsize=N              透传（diff 参数）
//   --help / --version       简要输出后以 0 退出
//
// 引擎不支持的 GNU sdiff 参数的处理：
//   -o FILE（交互式合并输出）/ -d（最小差异）等：透传给 diff 解析器，统一报
//   "Unknown option" 后以 2 退出；
//   -s / -l（抑制公共行）：必须在此显式拦截——diff 的 `-s` 语义是
//   --report-identical-files，直接透传会产生静默的语义错位，而非报错。

use crate::diff;
use std::ffi::OsString;
use std::iter::Peekable;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn usage() {
    println!("Usage: sdiff [OPTION]... FILE1 FILE2");
    println!();
    println!("Options:");
    println!("  -w, --width=NUM    output at most NUM print columns (default 130)");
    println!("  -t, --expand-tabs  expand tabs to spaces in output");
    println!("      --tabsize=NUM  set tab stop size (default 8)");
    println!("      --help         display this help and exit");
    println!("      --version      output version information and exit");
    println!();
    println!("(-o interactive merge and -s/-l common-line suppression are not");
    println!("supported by this vendored build.)");
}

pub fn main<I: Iterator<Item = OsString>>(mut opts: Peekable<I>) -> ExitCode {
    // 迭代器首元素是命令名（诊断信息里的 executable），原样保留。
    let Some(exe) = opts.next() else {
        eprintln!("Usage: sdiff [OPTION]... FILE1 FILE2");
        return ExitCode::from(2);
    };

    // 强制 side-by-side 输出；放在最前，用户再传 -u/-c 等格式选项时由 diff
    // 解析器报 "Conflicting output style option"。
    let mut translated: Vec<OsString> = vec![exe, OsString::from("-y")];

    while let Some(arg) = opts.next() {
        let s = arg.to_string_lossy().into_owned();
        match s.as_str() {
            "--help" => {
                usage();
                return ExitCode::SUCCESS;
            }
            "--version" | "-V" => {
                println!("sdiff (uutils diffutils {VERSION}, VeloTerm vendor)");
                return ExitCode::SUCCESS;
            }
            // 显式拦截（原因见文件头注释）：透传会给出错误语义而非报错。
            "-s" | "--suppress-common-lines" => {
                eprintln!("sdiff: option '{s}' is not supported by this vendored build");
                return ExitCode::from(2);
            }
            "-l" | "--left-column" => {
                eprintln!("sdiff: option '{s}' is not supported by this vendored build");
                return ExitCode::from(2);
            }
            "-o" | "--output" => {
                eprintln!("sdiff: interactive merge (-o) is not supported by this vendored build");
                return ExitCode::from(2);
            }
            // sdiff 的 -w 是宽度；diff 引擎只有长格式 --width=N，做等价翻译。
            "-w" | "--width" => match opts.next() {
                Some(n) => {
                    translated.push(OsString::from(format!(
                        "--width={}",
                        n.to_string_lossy()
                    )));
                }
                None => {
                    eprintln!("sdiff: option '-w' requires an argument");
                    return ExitCode::from(2);
                }
            },
            _ => {
                if let Some(rest) = s.strip_prefix("-w") {
                    // GNU 风格的粘附形式 `-w80`；其余以 -w 开头的串（如 -width）
                    // 不是合法选项，透传给 diff 解析器统一报错。
                    if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
                        translated.push(OsString::from(format!("--width={rest}")));
                        continue;
                    }
                }
                translated.push(arg);
            }
        }
    }

    diff::main(translated.into_iter().peekable())
}
