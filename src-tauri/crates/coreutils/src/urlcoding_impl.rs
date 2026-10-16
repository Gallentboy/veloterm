//! urlencode / urldecode：URL 百分号编解码 CLI（仅进 Windows 命令表）。
//!
//! 能力取自 crates.io 的 `urlencoding` 库（kornelski/rust_urlencoding，MIT，
//! 零依赖纯 Rust，已是 tauri 栈 wry 的传递依赖）：`encode` 按 WHATWG URL 标准
//! 做百分号编码（字母数字与 `-_.~` 保留，空格编为 `%20`，从不用 `+`）；
//! `decode` 按 WHATWG 恢复语义解码（`+` 不当空格，非法转义按字面保留）。
//! 与 openssl/telnet/jq/yq 同为 Windows 专属命令：macOS/Linux 用户可从系统
//! 获得同类工具，不进跨平台清单。
//!
//! 平台形态同 data-encoding 家族（urlcoding 是纯 Rust 库，无 DLL/FFI 约束）：
//! 实现与分发臂跨平台编译，仅 tools-catalog 的 Windows 清单门控——macOS 常规
//! 测试门禁全量覆盖「参数解析 / 行拆分 / 输出」管道与真实库的编解码向量。
//!
//! 行为约定（两个命令一致）：
//! - 有操作数：逐个编码/解码，各占一行，其中操作数 `-` 按出现顺序读 stdin
//!   （GNU 惯例）；无操作数：逐行读 stdin（文本行语义，
//!   剥行尾 `\n` 与其前紧邻的 `\r`），每行产出一条；最后一行无换行也补 `\n`；
//! - stdin 按字节读入再 lossy 转 UTF-8，非 UTF-8 内容不报错（与 yq_impl 的
//!   argv 取舍一致）；urldecode 解码出的非法 UTF-8 字节按库语义成 U+FFFD；
//! - `--help` / `--version` 写 stdout 退出 0；未知选项退出 1（GNU 惯例）；
//!   `--` 终止选项解析，使 `-` 开头的字符串可被编码。

/// 与库解耦的 CLI 管道：参数解析（跳过 argv[0] 程序名）、stdin 行拆分、
/// 逐行输出。编解码经 `transform` 注入，input/out 同样注入——本模块自身
/// 无平台差异，macOS 单测直接构造内存流驱动（见 tests）。
mod pipeline {
    use std::ffi::OsString;
    use std::io::{BufRead, Write};

    /// argv 首元素约定为程序名（与 yq_cmd_main 同一约定），其后为用户参数。
    pub(super) fn run<I, W>(
        input: &mut I,
        out: &mut W,
        transform: fn(&str) -> String,
        bin: &str,
        usage: &str,
        argv: &[OsString],
    ) -> i32
    where
        I: BufRead,
        W: Write,
    {
        let mut operands: Vec<String> = Vec::new();
        let mut end_of_flags = false;
        for arg in argv.iter().skip(1) {
            // CLI 参数场景用 to_string_lossy 折损成 UTF-8（与 yq_impl 同一取舍）
            let s = arg.to_string_lossy().into_owned();
            if !end_of_flags {
                match s.as_str() {
                    "--" => {
                        end_of_flags = true;
                        continue;
                    }
                    "--help" => {
                        // --help/--version 走 stdout、退出 0（GNU 惯例，
                        // version sweep 依赖 --version 返回 0）
                        let _ = writeln!(out, "{usage}\n");
                        return 0;
                    }
                    "--version" => {
                        let _ = writeln!(
                            out,
                            "{bin} (veloterm coreutils) {}",
                            env!("CARGO_PKG_VERSION")
                        );
                        return 0;
                    }
                    s if s.starts_with('-') && s.len() > 1 => {
                        eprintln!("{bin}: unrecognized option '{s}'");
                        eprintln!("Try '{bin} --help' for more information.");
                        return 1;
                    }
                    _ => {}
                }
            }
            operands.push(s);
        }

        if operands.is_empty() {
            let code = pump_stdin(input, out, transform, bin);
            let _ = out.flush();
            return code;
        }
        for op in &operands {
            if op == "-" {
                // `-` 操作数读 stdin（GNU 惯例），按出现顺序与其他操作数穿插
                let code = pump_stdin(input, out, transform, bin);
                if code != 0 {
                    let _ = out.flush();
                    return code;
                }
            } else {
                // 写失败（如管道下游已关闭）静默继续，不 panic（比 which 的
                // println! 更稳：head 截断场景无 panic 噪音）
                let _ = writeln!(out, "{}", transform(op));
            }
        }
        let _ = out.flush();
        0
    }

    /// 逐行读输入并逐行写出变换结果；返回退出码（0 正常 / 1 读失败）。
    /// 无操作数时输入是 stdin；操作数含 `-` 时按出现顺序内联消费。
    fn pump_stdin(
        input: &mut dyn BufRead,
        out: &mut dyn Write,
        transform: fn(&str) -> String,
        bin: &str,
    ) -> i32 {
        // read_until 按字节读，非 UTF-8 行不报错（lossy 转 UTF-8）
        let mut buf: Vec<u8> = Vec::new();
        loop {
            buf.clear();
            match input.read_until(b'\n', &mut buf) {
                Ok(0) => return 0,
                Ok(_) => {}
                Err(e) => {
                    eprintln!("{bin}: failed to read standard input: {e}");
                    return 1;
                }
            }
            // 文本行语义：剥行尾 \n 与其前紧邻的 \r（Windows CRLF 友好）
            let mut line: &[u8] = &buf;
            if line.last() == Some(&b'\n') {
                line = &line[..line.len() - 1];
                if line.last() == Some(&b'\r') {
                    line = &line[..line.len() - 1];
                }
            }
            let _ = writeln!(out, "{}", transform(&String::from_utf8_lossy(line)));
        }
    }
}

mod imp {
    use std::ffi::OsString;

    /// urlencode 入口：`args` 首元素约定为程序名（"urlencode"），其后为用户参数。
    pub fn urlencode_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let argv: Vec<OsString> = args.into_iter().collect();
        super::pipeline::run(
            &mut std::io::stdin().lock(),
            &mut std::io::stdout().lock(),
            encode_str,
            "urlencode",
            USAGE_ENCODE,
            &argv,
        )
    }

    /// urldecode 入口：约定同上。
    pub fn urldecode_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let argv: Vec<OsString> = args.into_iter().collect();
        super::pipeline::run(
            &mut std::io::stdin().lock(),
            &mut std::io::stdout().lock(),
            decode_str,
            "urldecode",
            USAGE_DECODE,
            &argv,
        )
    }

    pub(super) fn encode_str(s: &str) -> String {
        urlencoding::encode(s).into_owned()
    }

    /// urlencoding 2.1.3 的 decode 返回 Result（2.2+ 才是 infallible Cow）：
    /// 解出非法 UTF-8 时按 WHATWG 恢复语义降级为 U+FFFD（lossy），不报错
    pub(super) fn decode_str(s: &str) -> String {
        match urlencoding::decode(s) {
            Ok(cow) => cow.into_owned(),
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        }
    }

    const USAGE_ENCODE: &str = "\
Usage: urlencode [OPTION]... [STRING]...
Percent-encode each STRING, or each line of standard input when no STRING
is given (WHATWG URL standard: alphanumerics and -_.~ are kept as-is,
space becomes %20, '+' is never used; trailing \\n and \\r\\n are stripped).

      --help     display this help and exit
      --version  output version information and exit

An operand '-' reads standard input.
'--' ends the options, so a STRING starting with '-' can be encoded.";

    const USAGE_DECODE: &str = "\
Usage: urldecode [OPTION]... [STRING]...
Decode %XX percent-escapes in each STRING, or each line of standard input
when no STRING is given (WHATWG recovery: '+' is kept as-is, not treated
as a space; malformed escapes are left literal; undecodable bytes become
U+FFFD; trailing \\n and \\r\\n are stripped before decoding).

      --help     display this help and exit
      --version  output version information and exit

An operand '-' reads standard input.
'--' ends the options, so a STRING starting with '-' can be decoded.";
}

// bin crate：分发臂按 crate::urlcoding_impl::urlencode_cmd_main 全路径调用；
// 入口定义在 imp 子模块里，需顶层 re-export（分发臂跨平台编译，无条件导出）
pub use imp::{urldecode_cmd_main, urlencode_cmd_main};

#[cfg(test)]
mod tests {
    use super::pipeline::run;
    use std::ffi::OsString;

    fn ident(s: &str) -> String {
        s.to_string()
    }

    // 真实编解码向量见下方 roundtrip 测试（urlcoding 跨平台编译后 macOS 直接执行）

    const USAGE: &str = "Usage: fake [OPTION]... [STRING]...\n";

    fn s(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    fn output(out: &[u8]) -> String {
        String::from_utf8_lossy(out).into_owned()
    }

    /// 有操作数时不读 stdin，且 argv[0] 程序名不当操作数
    #[test]
    fn operands_bypass_stdin_and_skip_argv0() {
        let mut input = &b"SHOULD NOT APPEAR\n"[..];
        let mut out = Vec::new();
        let code = run(
            &mut input,
            &mut out,
            ident,
            "fake",
            USAGE,
            &s(&["fake", "a b", "中文/路径"]),
        );
        assert_eq!(code, 0);
        assert_eq!(output(&out), "a b\n中文/路径\n");
    }

    /// 无操作数时逐行处理 stdin：CRLF 剥成 LF、末行无换行也补齐
    #[test]
    fn stdin_lines_are_transformed_without_eol() {
        let mut input = &b"a b\r\nplain\nno trailing newline"[..];
        let mut out = Vec::new();
        let code = run(&mut input, &mut out, ident, "fake", USAGE, &s(&["fake"]));
        assert_eq!(code, 0);
        assert_eq!(output(&out), "a b\nplain\nno trailing newline\n");
    }

    /// 选项解析：--help/--version 退出 0 且写 stdout；未知选项退出 1；
    /// `--` 之后 `-x` 是普通操作数；裸 `-` 不当选项
    #[test]
    fn flag_parsing_help_version_and_terminator() {
        // --version
        let mut out = Vec::new();
        assert_eq!(
            run(
                &mut &b""[..],
                &mut out,
                ident,
                "fake",
                USAGE,
                &s(&["fake", "--version"])
            ),
            0
        );
        assert!(
            output(&out).starts_with("fake (veloterm coreutils)"),
            "{}",
            output(&out)
        );

        // --help
        let mut out = Vec::new();
        assert_eq!(
            run(&mut &b""[..], &mut out, ident, "fake", USAGE, &s(&["fake", "--help"])),
            0
        );
        assert!(output(&out).contains("Usage: fake"));

        // 未知选项 → 1（usage 不再写 stdout，提示走 stderr）
        let mut out = Vec::new();
        assert_eq!(
            run(
                &mut &b""[..],
                &mut out,
                ident,
                "fake",
                USAGE,
                &s(&["fake", "-x"])
            ),
            1
        );
        assert!(output(&out).is_empty());

        // `--` 终止后 `-x` 是操作数；裸 `-` 读 stdin（此处 stdin 为空 → 无输出）
        let mut out = Vec::new();
        assert_eq!(
            run(
                &mut &b""[..],
                &mut out,
                ident,
                "fake",
                USAGE,
                &s(&["fake", "--", "-x", "-"])
            ),
            0
        );
        assert_eq!(output(&out), "-x\n");
    }

    /// 操作数 `-` 读 stdin（与无操作数同一逐行路径），可与其他操作数按序穿插
    #[test]
    fn dash_operand_reads_stdin_in_order() {
        let mut input = &b"stdin line\r\nsecond"[..];
        let mut out = Vec::new();
        let code = run(
            &mut input,
            &mut out,
            ident,
            "fake",
            USAGE,
            &s(&["fake", "op1", "-", "op2"]),
        );
        assert_eq!(code, 0);
        assert_eq!(output(&out), "op1\nstdin line\nsecond\nop2\n");
    }

    /// 真实库行为向量（urlcoding 跨平台编译后 macOS 门禁直接执行）：
    /// WHATWG 语义、`+` 保留、非法转义按字面、往返一致
    #[test]
    fn real_library_vectors_and_roundtrip() {
        use super::imp::{decode_str, encode_str};
        assert_eq!(encode_str("a b 中文/~._-"), "a%20b%20%E4%B8%AD%E6%96%87%2F~._-");
        assert_eq!(decode_str("a%20b%20%E4%B8%AD%E6%96%87%2F~._-"), "a b 中文/~._-");
        // '+' 不是空格（百分号解码，非表单解码）
        assert_eq!(decode_str("a+b"), "a+b");
        // WHATWG 恢复：非法转义按字面保留
        assert_eq!(decode_str("100%"), "100%");
        assert_eq!(decode_str("%zz"), "%zz");
        // 往返一致
        let sample = "héllo wörld / ?&=# 中文 🦀";
        assert_eq!(decode_str(&encode_str(sample)), sample);
    }
}
