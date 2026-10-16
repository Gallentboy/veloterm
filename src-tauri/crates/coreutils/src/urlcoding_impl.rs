//! urlencode / urldecode：URL 百分号编解码 CLI（仅进 Windows 命令表）。
//!
//! 能力取自 crates.io 的 `urlencoding` 库（kornelski/rust_urlencoding，MIT，
//! 零依赖纯 Rust，已是 tauri 栈 wry 的传递依赖）：`encode` 按 WHATWG URL 标准
//! 做百分号编码（字母数字与 `-_.~` 保留，空格编为 `%20`，从不用 `+`）；
//! `decode` 按 WHATWG 恢复语义解码（`+` 不当空格，非法转义按字面保留）。
//! 与 openssl/telnet/jq/yq 同为 Windows 专属命令：macOS/Linux 用户可从系统
//! 获得同类工具，不进跨平台清单。
//!
//! 结构参考 yq_impl.rs 的 cfg 分家：`imp`（Windows）持库调用，非 Windows 为
//! 不可达桩。与 yq 的 FFI 不同，全部逻辑都在本文件——故把「参数解析 / 行拆分 /
//! 输出」管道放在无条件编译的 `pipeline` 子模块，由 transform 函数注入编解码，
//! macOS 的常规测试门禁也能覆盖 argv0 剥离、`--` 终止符、CRLF 行尾处理等自研逻辑。
//!
//! 行为约定（两个命令一致）：
//! - 有操作数：逐个编码/解码，各占一行；无操作数：逐行读 stdin（文本行语义，
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
    /// 非 Windows 构建中仅测试引用（命令不进清单，桩不调用），放行 dead_code。
    #[cfg_attr(not(windows), allow(dead_code))]
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
                        // 裸 "-" 是普通操作数（惯例上指 stdin 内容本身），不拦
                        eprintln!("{bin}: unrecognized option '{s}'");
                        eprintln!("Try '{bin} --help' for more information.");
                        return 1;
                    }
                    _ => {}
                }
            }
            operands.push(s);
        }

        if !operands.is_empty() {
            for op in &operands {
                // 写失败（如管道下游已关闭）静默继续读完输入，不 panic（比
                // which 的 println! 更稳：head 截断场景无 panic 噪音）
                let _ = writeln!(out, "{}", transform(op));
            }
            let _ = out.flush();
            return 0;
        }

        // 无操作数：逐行读 stdin。read_until 按字节读，非 UTF-8 行不报错
        let mut buf: Vec<u8> = Vec::new();
        loop {
            buf.clear();
            match input.read_until(b'\n', &mut buf) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    eprintln!("{bin}: 读取标准输入失败：{e}");
                    let _ = out.flush();
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
        let _ = out.flush();
        0
    }
}

#[cfg(windows)]
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

    fn encode_str(s: &str) -> String {
        urlencoding::encode(s).into_owned()
    }

    fn decode_str(s: &str) -> String {
        urlencoding::decode(s).into_owned()
    }

    const USAGE_ENCODE: &str = "\
Usage: urlencode [OPTION]... [STRING]...
Percent-encode each STRING, or each line of standard input when no STRING
is given (WHATWG URL standard: alphanumerics and -_.~ are kept as-is,
space becomes %20, '+' is never used; trailing \\n and \\r\\n are stripped).

      --help     display this help and exit
      --version  output version information and exit

'--' ends the options, so a STRING starting with '-' can be encoded.";

    const USAGE_DECODE: &str = "\
Usage: urldecode [OPTION]... [STRING]...
Decode %XX percent-escapes in each STRING, or each line of standard input
when no STRING is given (WHATWG recovery: '+' is kept as-is, not treated
as a space; malformed escapes are left literal; undecodable bytes become
U+FFFD; trailing \\n and \\r\\n are stripped before decoding).

      --help     display this help and exit
      --version  output version information and exit

'--' ends the options, so a STRING starting with '-' can be decoded.";
}

#[cfg(not(windows))]
mod imp {
    use std::ffi::OsString;

    /// 非 Windows 桩：命令只进 Windows 清单（tools-catalog 的 cfg 门控），
    /// 本桩在 mac 构建中无人调用——允许 dead_code（同 yq_impl 桩）。
    #[allow(dead_code)]
    pub fn urlencode_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let _ = args.into_iter().count();
        eprintln!("urlencode 仅在 Windows 构建");
        2
    }

    #[allow(dead_code)]
    pub fn urldecode_cmd_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
        let _ = args.into_iter().count();
        eprintln!("urldecode 仅在 Windows 构建");
        2
    }
}

// bin crate：分发臂按 crate::urlcoding_impl::urlencode_cmd_main 全路径调用；
// 入口定义在 cfg 分家的 imp 子模块里，需顶层 re-export。cfg 限定 windows：
// 非 Windows 构建没有调用方，无条件 re-export 反而触发 unused_imports。
#[cfg(windows)]
pub use imp::{urldecode_cmd_main, urlencode_cmd_main};

#[cfg(test)]
mod tests {
    use super::pipeline::run;
    use std::ffi::OsString;

    /// 测试用恒等变换：只验证管道自身（argv0 剥离 / 选项解析 / 行拆分），
    /// 真实编解码向量见下方 cfg(windows) 的 roundtrip 测试
    fn ident(s: &str) -> String {
        s.to_string()
    }

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

        // `--` 终止后 `-x` 是操作数；裸 `-` 也是操作数
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
        assert_eq!(output(&out), "-x\n-\n");
    }

    /// 真实库行为向量（仅 Windows 编译侧执行；CI 的 Windows 作业只构建，
    /// 该测试主要供本地 Windows 冒烟与安装包验证）：
    /// WHATWG 语义、`+` 保留、非法转义按字面、往返一致
    #[test]
    #[cfg(windows)]
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
