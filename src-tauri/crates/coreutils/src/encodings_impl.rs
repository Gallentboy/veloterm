//! data-encoding 家族命令（base32hex / base64url / hex / hexupper）的 CLI 壳，
//! 编解码内核来自 crates.io 的 data-encoding 2.11.1（ia0/data-encoding 仓库
//! tag v3.0.0-1.2 的 lib/，MIT，零依赖纯 Rust）。版本由 Cargo.lock 锁定，且
//! 与 uutils 家族的既有依赖共享同一份（path 依赖会造成同版本双份编译，故
//! 不走 vendor 树，同 flate2/crossterm 的先例）。
//!
//! 上游自带 CLI（bin/，data-encoding-bin）是单命令 getopts 形态（-m/-b），
//! 与本工程 BusyBox 短名模式不匹配，故按 tar_impl 的先例自研 GNU 风格壳，
//! 与已有 base32/base64（uutils，GNU 兼容）保持同构：`-d` 解码、`-i` 忽略
//! 非字母表字符、`-w COLS` 折行（默认 76，0 不折行）、单个 `[FILE]` 操作数
//! （缺省或 `-` 读标准输入）。
//!
//! 命令与库常量的映射（RFC 4648 §10 官方测试向量见单测）：
//!
//! | 命令      | 常量                  | 语义                          |
//! | --------- | --------------------- | ----------------------------- |
//! | base32hex | `BASE32HEX`           | RFC 4648 base32hex（带填充）  |
//! | base64url | `BASE64URL`           | RFC 4648 base64url（带填充）  |
//! | hex       | `HEXLOWER_PERMISSIVE` | 小写输出，解码大小写不敏感    |
//! | hexupper  | `HEXUPPER_PERMISSIVE` | 大写输出，解码大小写不敏感    |
//!
//! 为何只有四个：库的 base 家族中 base32/base64 已由 uutils 提供（base32 /
//! base64 / basenc 三个命令），按「不重复实现」原则排除；其余变体（nopad /
//! MIME / DNSSEC / DNSCurve）上游 CLI 也未命名为独立 base（经选项组合可达），
//! 不设短名命令。
//!
//! 解码换行容忍对齐 GNU/uutils（base_common 同款语义）：`\r` 与 `\n` 无条件
//! 忽略（`base64 f | base64 -d` 回环必须成立），其余非字母表字节默认报错，
//! `-i` 时静默丢弃。
//!
//! 平台门控：命令名只进 tools-catalog 的 Windows 清单；实现与依赖跨平台
//! 编译（分发臂不设 cfg）——macOS 本机单测可全量覆盖行为，Windows 侧由 CI
//! 构建覆盖编译。
//!
//! 流式算法：每轮读入按编码块对齐截断，跨块余数滚入下一轮；折行在输出侧
//! 进行——GNU 允许任意列宽，而 data-encoding 的 Wrap 要求列宽与位宽对齐
//! （base32hex 的 5 位宽在 76 列不满足），故不走库内 wrap。

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read, Write};

/// data-encoding 依赖版本（升级依赖时同步改，--version 输出用；与
/// crates/coreutils/Cargo.toml 的版本要求保持一致）
const DATA_ENCODING_VERSION: &str = "2.11.1";

/// 编码输出默认折行宽度（GNU base32/base64/basenc 同款默认）
const WRAP_DEFAULT: usize = 76;

/// 流式读写块大小
const IO_CHUNK: usize = 32 * 1024;

/// 一个短名命令：命令名 + 库编码常量
struct EncSpec {
    name: &'static str,
    about: &'static str,
    base: &'static data_encoding::Encoding,
}

/// data-encoding 家族的全部命令（分发臂按 name 逐个接入 enc_main）
const SPECS: &[EncSpec] = &[
    EncSpec {
        name: "base32hex",
        about: "Base32hex encode/decode data (RFC 4648) and print to standard output.",
        base: &data_encoding::BASE32HEX,
    },
    EncSpec {
        name: "base64url",
        about: "Base64url encode/decode data (RFC 4648) and print to standard output.",
        base: &data_encoding::BASE64URL,
    },
    EncSpec {
        name: "hex",
        about: "Hexadecimal encode/decode data and print to standard output.",
        base: &data_encoding::HEXLOWER_PERMISSIVE,
    },
    EncSpec {
        name: "hexupper",
        about: "Uppercase hexadecimal encode/decode data and print to standard output.",
        base: &data_encoding::HEXUPPER_PERMISSIVE,
    },
];

/// 执行失败：`Msg` 携带 stderr 文案；`Pipe` 表示下游管道已关闭（Rust 无
/// SIGPIPE，需自行识别后静默退出，对齐 GNU 被 SIGPIPE 中断的行为）。
enum Fail {
    Msg(String),
    Pipe,
}

/// 入口（run_uutils 通用臂约定：迭代器首元素为命令名，用于选择编码）
pub fn enc_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
    let mut it = args.into_iter();
    let name = it
        .next()
        .map(|a| a.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Some(spec) = SPECS.iter().find(|s| s.name == name) else {
        eprintln!("enc_main: unknown command '{name}'");
        return 1;
    };
    let argv: Vec<String> = it.map(|a| a.to_string_lossy().into_owned()).collect();
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    run_spec(spec, &argv, &mut stdin, &mut stdout)
}

/// 解析参数并执行（生产走 stdin/stdout；单测注入游标）。
/// 返回进程退出码，错误信息自行打印到 stderr。
fn run_spec(spec: &EncSpec, argv: &[String], stdin: &mut dyn Read, out: &mut dyn Write) -> i32 {
    let mut decode = false;
    let mut ignore_garbage = false;
    let mut wrap: Option<usize> = None;
    let mut operands: Vec<&str> = Vec::new();
    let mut no_more_flags = false;

    let mut i = 0;
    while let Some(arg) = argv.get(i) {
        let arg = arg.as_str();
        i += 1;
        if no_more_flags || !arg.starts_with('-') || arg == "-" {
            operands.push(arg);
            continue;
        }
        if arg == "--" {
            no_more_flags = true;
            continue;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (key, inline) = match long.split_once('=') {
                Some((k, v)) => (k, Some(v)),
                None => (long, None),
            };
            match key {
                "decode" => decode = true,
                "ignore-garbage" => ignore_garbage = true,
                "wrap" => {
                    let value = match inline {
                        Some(v) => v.to_string(),
                        None => match argv.get(i) {
                            Some(v) => {
                                i += 1;
                                v.clone()
                            }
                            None => {
                                eprintln!("{}: option '--wrap' requires an argument", spec.name);
                                return 1;
                            }
                        },
                    };
                    match value.parse::<usize>() {
                        Ok(w) => wrap = Some(w),
                        Err(_) => {
                            eprintln!("{}: invalid wrap size: '{value}'", spec.name);
                            return 1;
                        }
                    }
                }
                "help" => {
                    let _ = print_help(spec, out);
                    return 0;
                }
                "version" => {
                    let _ = writeln!(
                        out,
                        "{} (veloterm data-encoding {DATA_ENCODING_VERSION})",
                        spec.name
                    );
                    return 0;
                }
                _ => {
                    eprintln!("{}: unrecognized option '{arg}'", spec.name);
                    return 1;
                }
            }
            continue;
        }
        // 短参数（可捆绑；-w 可带内联值或取下一参数）
        let flags = &arg[1..];
        for (ci, c) in flags.char_indices() {
            match c {
                'd' => decode = true,
                'i' => ignore_garbage = true,
                'w' => {
                    let inline = &flags[ci + c.len_utf8()..];
                    let value = if !inline.is_empty() {
                        inline.to_string()
                    } else {
                        match argv.get(i) {
                            Some(v) => {
                                i += 1;
                                v.clone()
                            }
                            None => {
                                eprintln!("{}: option requires an argument -- 'w'", spec.name);
                                return 1;
                            }
                        }
                    };
                    match value.parse::<usize>() {
                        Ok(w) => wrap = Some(w),
                        Err(_) => {
                            eprintln!("{}: invalid wrap size: '{value}'", spec.name);
                            return 1;
                        }
                    }
                    break; // 内联值消费完剩余字符
                }
                _ => {
                    eprintln!("{}: invalid option -- '{c}'", spec.name);
                    return 1;
                }
            }
        }
    }

    // GNU base32/base64 仅接受单个 FILE 操作数
    let file = match operands.as_slice() {
        [] => None,
        [one] => Some(*one),
        [_, extra, ..] => {
            eprintln!("{}: extra operand '{extra}'", spec.name);
            return 1;
        }
    };

    let mut file_in;
    let input: &mut dyn Read = match file {
        None | Some("-") => stdin,
        Some(path) => match File::open(path) {
            Ok(f) => {
                file_in = f;
                &mut file_in
            }
            Err(e) => {
                eprintln!("{}: {path}: {e}", spec.name);
                return 1;
            }
        },
    };

    let result = if decode {
        decode_stream(spec.base, ignore_garbage, input, out)
    } else {
        encode_stream(spec.base, wrap.unwrap_or(WRAP_DEFAULT), input, out)
    };
    match result {
        Ok(()) => 0,
        Err(Fail::Pipe) => 0,
        Err(Fail::Msg(m)) => {
            eprintln!("{}: {m}", spec.name);
            1
        }
    }
}

fn print_help(spec: &EncSpec, out: &mut dyn Write) -> io::Result<()> {
    write!(
        out,
        "Usage: {name} [OPTION]... [FILE]\n\
         {about}\n\
         With no FILE, or when FILE is -, read standard input.\n\
         \n\
         \x20 -d, --decode          decode data\n\
         \x20 -i, --ignore-garbage  when decoding, ignore non-alphabet characters\n\
         \x20 -w, --wrap=COLS       wrap encoded lines after COLS characters \
         (default {default}, 0 disables wrapping)\n\
         \x20     --help            display this help and exit\n\
         \x20     --version         output version information and exit\n",
        name = spec.name,
        about = spec.about,
        default = WRAP_DEFAULT,
    )
}

/// 明文侧编码块大小（字节）：hex 1 / base64 3 / base32 5。
/// 与上游 bin/src/main.rs 的 encode_block 同式。
fn encode_block(base: &data_encoding::Encoding) -> usize {
    match base.bit_width() {
        1 | 2 | 4 => 1,
        3 | 6 => 3,
        5 => 5,
        _ => unreachable!(),
    }
}

/// 编码侧字符块大小：hex 2 / base64 4 / base32 8。
fn decode_block(base: &data_encoding::Encoding) -> usize {
    encode_block(base) * 8 / base.bit_width()
}

/// 字母表 + 填充 + translate.from 的合法输入字节表（解码过滤用）。
/// translate.from 是「用户实际敲的字符」一侧（如 permissive hex 的大写）。
fn canonical_table(base: &data_encoding::Encoding) -> [bool; 256] {
    let spec = base.specification();
    let mut table = [false; 256];
    for &b in spec.symbols.as_bytes() {
        table[b as usize] = true;
    }
    // 填充字符按库约束必为 ASCII
    if let Some(p) = spec.padding {
        table[p as u8 as usize] = true;
    }
    for &b in spec.translate.from.as_bytes() {
        table[b as usize] = true;
    }
    table
}

fn write_out(out: &mut dyn Write, buf: &[u8]) -> Result<(), Fail> {
    out.write_all(buf)
        .map_err(|e| match e.kind() {
            io::ErrorKind::BrokenPipe => Fail::Pipe,
            _ => Fail::Msg(format!("write error: {e}")),
        })
}

fn read_err(e: io::Error) -> Fail {
    Fail::Msg(format!("read error: {e}"))
}

/// 流式编码：按明文块对齐截断每轮读入（余数滚入下一轮，避免中途补填充），
/// 折行在输出侧按任意列宽进行。
fn encode_stream(
    base: &data_encoding::Encoding,
    wrap: usize,
    input: &mut dyn Read,
    out: &mut dyn Write,
) -> Result<(), Fail> {
    let block = encode_block(base);
    let mut buf = vec![0u8; IO_CHUNK + block];
    let mut enc = Vec::new();
    let mut line = Vec::new();
    let mut rest = 0usize;
    loop {
        let ilen = input.read(&mut buf[rest..]).map_err(read_err)?;
        let next = if ilen == 0 {
            rest
        } else {
            (rest + ilen) / block * block
        };
        if next > 0 {
            enc.clear();
            enc.resize(base.encode_len(next), 0);
            base.encode_mut(&buf[..next], &mut enc);
            if wrap == 0 {
                write_out(out, &enc)?;
            } else {
                line.extend_from_slice(&enc);
                let mut start = 0usize;
                while line.len() - start >= wrap {
                    let end = start + wrap;
                    write_out(out, &line[start..end])?;
                    write_out(out, b"\n")?;
                    start = end;
                }
                line.drain(..start);
            }
        }
        if ilen == 0 {
            break;
        }
        // 左结合写法（先 rest+ilen 再减）：next 可大于本轮 ilen（跨块余数归队），
        // 写成 `rest += ilen - next` 会在该场景下溢
        rest = rest + ilen - next;
        if rest > 0 {
            buf.copy_within(next..next + rest, 0);
        }
    }
    // 折行开启时收尾：剩余半行 + 换行（空输入不输出任何内容，同 GNU）
    if wrap > 0 && !line.is_empty() {
        write_out(out, &line)?;
        write_out(out, b"\n")?;
    }
    Ok(())
}

/// 流式解码：先按合法字节表过滤（\r\n 无条件忽略，其余非字母表字节默认
/// 报错、-i 时丢弃），再按编码字符块对齐解码；EOF 的尾块可短于整块，
/// 长度合法性（填充完整性等）交给库判定。
fn decode_stream(
    base: &data_encoding::Encoding,
    ignore_garbage: bool,
    input: &mut dyn Read,
    out: &mut dyn Write,
) -> Result<(), Fail> {
    let canonical = canonical_table(base);
    let block = decode_block(base);
    let mut raw = vec![0u8; IO_CHUNK];
    let mut pending: Vec<u8> = Vec::with_capacity(IO_CHUNK + block);
    let mut decoded = Vec::new();
    loop {
        let ilen = input.read(&mut raw).map_err(read_err)?;
        if ilen == 0 {
            break;
        }
        for &b in &raw[..ilen] {
            if canonical[b as usize] {
                pending.push(b);
            } else if b != b'\r' && b != b'\n' && !ignore_garbage {
                return Err(Fail::Msg("invalid input".into()));
            }
        }
        let next = pending.len() / block * block;
        if next > 0 {
            let mlen = base
                .decode_len(next)
                .map_err(|_| Fail::Msg("invalid input".into()))?;
            decoded.clear();
            decoded.resize(mlen, 0);
            // decode_len 按字符数上限预分配（填充字符也计入），实际输出
            // 长度以 decode_mut 的返回值为准
            let olen = base
                .decode_mut(&pending[..next], &mut decoded)
                .map_err(|_| Fail::Msg("invalid input".into()))?;
            write_out(out, &decoded[..olen])?;
            pending.drain(..next);
        }
    }
    if !pending.is_empty() {
        let mlen = base
            .decode_len(pending.len())
            .map_err(|_| Fail::Msg("invalid input".into()))?;
        decoded.clear();
        decoded.resize(mlen, 0);
        let olen = base
            .decode_mut(&pending, &mut decoded)
            .map_err(|_| Fail::Msg("invalid input".into()))?;
        write_out(out, &decoded[..olen])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;

    /// 经 run_spec 直跑（绕过 run_command 的平台清单——命令仅注册进 Windows
    /// 清单，macOS 侧经本函数全量覆盖行为）
    fn run(name: &str, args: &[&str], input: &[u8]) -> (i32, Vec<u8>) {
        let spec = SPECS.iter().find(|s| s.name == name).expect(name);
        let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let mut stdin = Cursor::new(input.to_vec());
        let mut out = Vec::new();
        let code = run_spec(spec, &argv, &mut stdin, &mut out);
        (code, out)
    }

    fn encode_ok(name: &str, args: &[&str], input: &[u8]) -> Vec<u8> {
        let (code, out) = run(name, args, input);
        assert_eq!(code, 0, "{name} {args:?} 应成功");
        out
    }

    fn decode_ok(name: &str, args: &[&str], input: &[u8]) -> Vec<u8> {
        let (code, out) = run(name, args, input);
        assert_eq!(code, 0, "{name} {args:?} 应成功");
        out
    }

    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "veloterm-encodings-test-{}-{}",
            tag,
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// RFC 4648 §10 官方向量（base32hex 与 base64url 段）
    #[test]
    fn rfc4648_vectors() {
        let cases: &[(&str, &[(&str, &str)])] = &[
            (
                "base32hex",
                &[
                    ("f", "CO======"),
                    ("fo", "CPNG===="),
                    ("foo", "CPNMU==="),
                    ("foob", "CPNMUOG="),
                    ("fooba", "CPNMUOJ1"),
                    ("foobar", "CPNMUOJ1E8======"),
                ],
            ),
            (
                "base64url",
                &[
                    ("f", "Zg=="),
                    ("fo", "Zm8="),
                    ("foo", "Zm9v"),
                    ("foob", "Zm9vYg=="),
                    ("fooba", "Zm9vYmE="),
                    ("foobar", "Zm9vYmFy"),
                ],
            ),
            ("hex", &[("foobar", "666f6f626172")]),
            ("hexupper", &[("foobar", "666F6F626172")]),
        ];
        for (name, vectors) in cases {
            for (plain, encoded) in *vectors {
                assert_eq!(
                    &encode_ok(name, &["-w0"], plain.as_bytes()),
                    encoded.as_bytes(),
                    "{name}({plain:?})"
                );
                // 向量本身回解码（hex 家族缺 permissive 大写用例，见 case_sensitivity）
                assert_eq!(
                    &decode_ok(name, &["-d"], encoded.as_bytes()),
                    plain.as_bytes(),
                    "{name} -d {encoded:?}"
                );
            }
        }
    }

    /// 各命令的字母表差异点：base64url 用 `-_`（base64 是 `+/`）、hex 大小写
    #[test]
    fn alphabet_specifics() {
        // 0xFF×3 → 6bit 全 1：base64 为 "////"，base64url 为 "____"
        assert_eq!(&encode_ok("base64url", &["-w0"], &[0xFF; 3]), b"____");
        assert_eq!(decode_ok("base64url", &["-d"], b"____"), vec![0xFF; 3]);
        assert_eq!(&encode_ok("hex", &["-w0"], &[0xAB; 2]), b"abab");
        assert_eq!(&encode_ok("hexupper", &["-w0"], &[0xAB; 2]), b"ABAB");
        // base64url 字母表不含 +/（base64 的输出，反向亦然）；
        // 0xFB 0xFF 0xBF 恰 24 位 → 4 字符无填充
        assert_eq!(&encode_ok("base64url", &["-w0"], &[0xFB, 0xFF, 0xBF]), b"-_-_");
        assert_eq!(run("base64url", &["-d"], b"/v/=").0, 1);
    }

    /// 全命令 × 多长度回环（含跨读块边界与空输入），默认折行与 -w0 均回解码
    #[test]
    fn roundtrip_all_commands_and_lengths() {
        let lengths = [0usize, 1, 2, 3, 4, 5, 6, 7, 57, 153, 1000, 32 * 1024 + 7];
        let mut input = Vec::new();
        for i in 0..*lengths.last().unwrap() {
            input.push((i * 7 + 13) as u8);
        }
        for spec in SPECS {
            for &len in &lengths {
                let plain = &input[..len];
                let flat = encode_ok(spec.name, &["-w0"], plain);
                assert_eq!(decode_ok(spec.name, &["-d"], &flat), plain);
                let wrapped = encode_ok(spec.name, &[], plain);
                assert_eq!(decode_ok(spec.name, &["-d"], &wrapped), plain);
            }
        }
    }

    /// 折行：默认 76（尾行 + 换行）、-w0 不折不换行、任意列宽（含非块对齐）
    #[test]
    fn wrap_widths() {
        // 60 字节 → 80 个 base64url 字符，76 列下断成 76 + 4
        let plain = vec![b'A'; 60];
        let flat = encode_ok("base64url", &["-w0"], &plain);
        assert_eq!(flat.len(), 80);
        assert!(!flat.contains(&b'\n'));

        let default = encode_ok("base64url", &[], &plain);
        let mut expect = flat[..76].to_vec();
        expect.push(b'\n');
        expect.extend_from_slice(&flat[76..]);
        expect.push(b'\n');
        assert_eq!(default, expect);

        // -w10：8 行 × (10 + \n)
        let w10 = encode_ok("base64url", &["-w10"], &plain);
        let mut expect10 = Vec::new();
        for chunk in flat.chunks(10) {
            expect10.extend_from_slice(chunk);
            expect10.push(b'\n');
        }
        assert_eq!(w10, expect10);

        // 长参数等价形态
        assert_eq!(encode_ok("base64url", &["--wrap=10"], &plain), w10);
        assert_eq!(encode_ok("base64url", &["--wrap", "10"], &plain), w10);
        assert_eq!(encode_ok("base64url", &["-w", "10"], &plain), w10);

        // 空输入：折行开启也不产生输出
        assert!(encode_ok("base64url", &[], b"").is_empty());
    }

    /// 解码换行容忍（GNU/uutils 语义）：\n 与 \r\n 无条件忽略
    #[test]
    fn decode_newline_tolerance() {
        let wrapped = encode_ok("base64url", &[], b"hello world, roundtrip!");
        let crlf: Vec<u8> = wrapped
            .split(|&b| b == b'\n')
            .flat_map(|line| [line, b"\r\n"].concat())
            .collect();
        assert_eq!(decode_ok("base64url", &["-d"], &crlf), b"hello world, roundtrip!");
    }

    /// -i 与大小写：非字母表字节默认报错、-i 丢弃；permissive hex 双向大小写；
    /// base32hex 解码按 RFC 严格区分大小写
    #[test]
    fn garbage_and_case_sensitivity() {
        // 夹带垃圾字节
        assert_eq!(run("base64url", &["-d"], b"Zm9v!Yg==\n").0, 1);
        assert_eq!(
            decode_ok("base64url", &["-d", "-i"], b"Zm9v!Yg==\n"),
            b"foob"
        );
        // 长短参数等价
        assert_eq!(
            decode_ok("base64url", &["--decode", "--ignore-garbage"], b"Z!m9vYg=="),
            b"foob"
        );
        // hex 接受大写输入，hexupper 接受小写输入（permissive 家族）
        assert_eq!(decode_ok("hex", &["-d"], b"666F6F626172"), b"foobar");
        assert_eq!(decode_ok("hexupper", &["-d"], b"666f6f626172"), b"foobar");
        // base32hex 字母表为大写 + 数字：小写输入是非法字符
        assert_eq!(run("base32hex", &["-d"], b"cpnmuog=").0, 1);
    }

    /// 解码要求规范填充（GNU base64 -d 同款严格性）
    #[test]
    fn decode_requires_padding() {
        assert_eq!(run("base64url", &["-d"], b"Zm9vYg").0, 1);
        assert_eq!(run("base32hex", &["-d"], b"CPNMUOJ1E8").0, 1);
        // 长度不合法（base64 字符数 mod 4 == 1）
        assert_eq!(run("base64url", &["-d"], b"Z").0, 1);
    }

    /// CLI 面：--help/--version 退出 0，错误参数退出 1
    #[test]
    fn cli_surface() {
        for spec in SPECS {
            let (code, out) = run(spec.name, &["--version"], b"");
            assert_eq!(code, 0);
            let text = String::from_utf8(out).unwrap();
            assert!(
                text.contains(spec.name)
                    && text.contains(DATA_ENCODING_VERSION),
                "{name} --version 输出异常: {text}",
                name = spec.name
            );

            let (code, out) = run(spec.name, &["--help"], b"");
            assert_eq!(code, 0);
            let text = String::from_utf8(out).unwrap();
            assert!(text.contains("Usage:"), "{} --help 缺 Usage", spec.name);
            assert!(text.contains("--ignore-garbage"));

            // 捆绑短参数 + 未知参数
            assert_eq!(run(spec.name, &["--veloterm-invalid"], b"").0, 1);
            assert_eq!(run(spec.name, &["-x"], b"").0, 1);
            assert_eq!(run(spec.name, &["-w", "abc"], b"").0, 1);
            assert_eq!(run(spec.name, &["-w", "-1"], b"").0, 1);
            assert_eq!(run(spec.name, &["a.txt", "b.txt"], b"").0, 1);
            assert_eq!(run(spec.name, &["/no/such/file"], b"").0, 1);
        }
    }

    /// 文件操作数与 `-`（stdin 显式指定）等价
    #[test]
    fn file_operand_matches_stdin() {
        let dir = tempdir("file");
        let path = dir.join("data.bin");
        fs::write(&path, b"foobar").unwrap();
        let p = path.to_string_lossy().into_owned();

        let via_file = encode_ok("base64url", &["-w0", &p], b"");
        assert_eq!(via_file, b"Zm9vYmFy");
        let via_dash = encode_ok("base64url", &["-w0", "-"], b"foobar");
        assert_eq!(via_dash, via_file);

        let _ = fs::remove_dir_all(&dir);
    }
}
