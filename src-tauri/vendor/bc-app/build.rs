//! bc-app 构建脚本：把 bc CLI（vendor/bc，7.1.0）连同 4 个 strgen 生成物
//! 用 cc 现场编译并静态链接。零外部依赖——不需要 vcpkg / cmake / perl /
//! 宿主生成器，Windows runner 无任何额外安装步骤。
//!
//! 仅 `target_os = "windows"` 时工作，其余平台直接 return（macOS 走桩模块，
//! 按 AGENTS.md 约定本机测试通过即视为完成，真实编译由 CI windows runner 验证）。
//!
//! # 依据（从 vendor/bc 的 vs/bc.vcxproj / Makefile.in / 源码逐条核实过的事实）
//!
//! - **上游自带 MSVC 支持**： bc 7.x 官方 tarball 随附 `vs/bc.vcxproj`
//!   （v143 工具集、`/std:c17`、ConformanceMode），全部 POSIX 头包含均有
//!   `#ifndef _WIN32` 守卫——与 jq 1.7.1 的五处修补不同，本地零补丁。
//! - **编译集合**： vcxproj 的 ClCompile 22 个源（args/bc/bc_lex/bc_parse/
//!   data/dc/dc_lex/dc_parse/file/history/lang/lex/library/main/num/opt/
//!   parse/program/rand/read/vector/vm.c）；`bc_fuzzer.c`/`dc_fuzzer.c` 是
//!   BC_ENABLE_OSSFUZZ 专属入口，排除。library.c 在 BC_ENABLE_LIBRARY=0
//!   下整体被预处理器掏空，但上游 vcxproj 照常编译，此处保持一致。
//! - **无 config.h**： bc 的全部构建开关走命令行 `-D` 宏（Makefile.in 的
//!   CPPFLAGS 与 vcxproj 的 PreprocessorDefinitions 均无 AC_CONFIG_HEADERS
//!   一类的产物头），照 vcxproj Release + Makefile.in:218 的
//!   `-DVERSION=$(VERSION)`（值取 vendor/bc/VERSION.txt）直接用 cc 的
//!   define 传宏。不定义 `WIN32`：全树只用 `_WIN32`（MSVC 预定义）。
//! - **strgen 生成物**： tarball 不含 lib.c/lib2.c/bc_help.c/dc_help.c，
//!   上游构建时编译宿主 strgen（gen/strgen.c）现场生成（Makefile.in:247-262
//!   与 vcxproj 的 CustomBuild 参数一致）。Windows runner 无需宿主工具：
//!   本脚本用 Rust 逐字节复刻 strgen（见 `render_strgen`），开发时对 4 个
//!   输入与真实 strgen（macOS clang 编译运行）的输出做过 byte-diff 验证，
//!   逐字节一致。生成物带 `#if BC_ENABLED`/`#if DC_ENABLED` 守卫，与本体
//!   同宏编译。
//! - **入口改名**： `src/main.c` 的 `main(int argc, const char* argv[])` 经
//!   `-Dmain=veloterm_bc_main` 改名后由 FFI 调用。`main` 标识符全树仅此
//!   一处（grep 核实；data.c 的 "(main)" 是字符串字面量内容）。改名比
//!   jq 的 umain 直调更进一步：main.c 含 argv[0] → vm->name 的分发
//!   （`vm->name[0] != 'd'` 走 bc_main）、jmp_bufs 栈初始化与
//!   bc_vm_atexit 收尾，跳过它就要在 C 侧复刻这套宏编排，得不偿失。
//!   上游 vcxproj 的 PostBuildEvent（copy bc.exe dc.exe）即同一二进制
//!   双名字形态；本项目固定传 argv[0]="bc"（tools-catalog 只注册 bc）。
//!
//! # 编译告警
//!
//! cc Build `warnings(false)`：第三方上游 C 源在 MSVC 默认 -W4 下会刷出
//! 大量 C4244/C4100 类告警淹没真实 error（jq-app 的 CI 实测结论，同此
//! 静默）。只作用于 C 编译，不影响 Rust 侧告警。
//!
//! # 链接
//!
//! 仅 `bcrypt`（src/rand.c 的 BCryptGenRandom）；history 的 Win32 Console
//! API（ReadConsoleW 等）走 MSVC 默认引入的 kernel32/UCRT。
//!
//! # 冒烟基准（macOS 本机全宏原生构建实测，Windows 行为基准）
//!
//! - `bc --version` → 打印 "bc 7.1.0" 三行，退出 0（coreutils 的
//!   all_commands_dispatch_version_sweep 依赖此语义）
//! - `echo '2+2' | bc` → 4；`bc -l` 的 e(1)/sqrt(9)/obase=16 均正确

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// 统一日志前缀
const TAG: &str = "[bc-app]";
/// bc 版本宏值（vendor/bc/VERSION.txt 锁定版本）。Makefile.in:218 的
/// `-DVERSION=$(VERSION)` 即此；vm.c 的 `--version` 输出拼成 "bc 7.1.0"。
const BC_VERSION: &str = "7.1.0";

/// bc 程序本体源（vs/bc.vcxproj 的 ClCompile 全集；fuzzer 是
/// BC_ENABLE_OSSFUZZ 专属入口，排除）
const BC_SOURCES: &[&str] = &[
    "args.c", "bc.c", "bc_lex.c", "bc_parse.c", "data.c", "dc.c", "dc_lex.c", "dc_parse.c",
    "file.c", "history.c", "lang.c", "lex.c", "library.c", "main.c", "num.c", "opt.c", "parse.c",
    "program.c", "rand.c", "read.c", "vector.c", "vm.c",
];

/// strgen 生成任务：(输入, 输出文件名, 数组名, label, 宏守卫, remove_tabs)。
/// 参数与 Makefile.in:247-262 / vcxproj CustomBuild 一致；label 取
/// Makefile 形态的相对路径（生成物头注释与 bc_lib_name 的「文件名」即它，
/// 上游 Linux 发布物内嵌的就是 "gen/lib.bc"）。
const STRGEN_JOBS: &[(&str, &str, &str, &str, &str, bool)] = &[
    ("gen/lib.bc", "lib.c", "bc_lib", "bc_lib_name", "BC_ENABLED", true),
    ("gen/lib2.bc", "lib2.c", "bc_lib2", "bc_lib2_name", "BC_ENABLED", true),
    ("gen/bc_help.txt", "bc_help.c", "bc_help", "", "BC_ENABLED", false),
    ("gen/dc_help.txt", "dc_help.c", "dc_help", "", "DC_ENABLED", false),
];

/// strgen 的 `{{ A H N HN }}` 标记：包住仅 BC_ENABLE_EXTRA_MATH 的内容
const EX_START: &str = "{{ A H N HN }}";
/// strgen 的区块结束标记
const EX_END: &str = "{{ end }}";
/// strgen 的行宽阈值（超过即折行）
const MAX_WIDTH: usize = 72;

fn main() {
    // 用 CARGO_CFG_TARGET_OS（而非 HOST）判断，保证 canadian 交叉也正确
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    println!("cargo:rerun-if-changed=build.rs");
    if target_os != "windows" {
        println!("{TAG} 非 Windows target（{target_os}），按约定早退，不编译任何东西");
        return;
    }
    if let Err(msg) = run() {
        panic!("{TAG} 构建失败：{msg}\n{TAG} 提示：bc 源码应为工程内 vendor/bc（7.1.0）；\
                缺文件说明 vendor 树不完整，见 vendor/bc/README.md 的裁剪清单。");
    }
}

fn run() -> Result<(), String> {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap_or_default());
    let bc_root = manifest.join("..").join("bc");
    let bc_src = bc_root.join("src");
    let bc_include = bc_root.join("include");

    // ── 1. 防御式发现：vendor 树的关键文件 ─────────────────────────────────
    for rel in [
        "VERSION.txt",
        "Makefile.in",
        "vs/bc.vcxproj",
        "src/main.c",
        "src/vm.c",
        "include/vm.h",
        "gen/strgen.c",
        "gen/lib.bc",
        "gen/lib2.bc",
        "gen/bc_help.txt",
        "gen/dc_help.txt",
    ] {
        let p = bc_root.join(rel);
        if !p.is_file() {
            return Err(format!("vendor 树缺关键文件：{}（基目录 {}）", p.display(), bc_root.display()));
        }
    }
    // 版本双源一致性：build.rs 的 BC_VERSION 与 vendor/bc/VERSION.txt 必须同步改
    let version_txt = fs::read_to_string(bc_root.join("VERSION.txt"))
        .map_err(|e| format!("读 VERSION.txt 失败：{e}"))?
        .trim()
        .to_string();
    if version_txt != BC_VERSION {
        return Err(format!(
            "版本双源不一致：VERSION.txt={version_txt} 但 build.rs 的 BC_VERSION={BC_VERSION}（升级时两处必须同步改）"
        ));
    }
    println!("{TAG} vendor/bc 源码就位：{}", bc_root.display());
    let bc_sources = resolve_all(&bc_src, BC_SOURCES)?;

    // ── 2. 生成物：OUT_DIR 下渲染 4 个 strgen 产物（上游由宿主 strgen 现场生成）──
    let genout = out_dir.join("strgen");
    fs::create_dir_all(&genout).map_err(|e| format!("创建 {} 失败：{e}", genout.display()))?;
    let mut generated = Vec::with_capacity(STRGEN_JOBS.len());
    for (input, out_name, name, label, define, remove_tabs) in STRGEN_JOBS {
        let text = fs::read_to_string(bc_root.join(input))
            .map_err(|e| format!("读 {} 失败：{e}", bc_root.join(input).display()))?;
        let rendered = render_strgen(&text, input, name, label, define, *remove_tabs)?;
        let path = genout.join(out_name);
        write_file(&path, &rendered)?;
        println!(
            "{TAG} strgen 生成 {}（{} 字节输入 → {} 字节，remove_tabs={}）",
            out_name,
            text.len(),
            rendered.len(),
            remove_tabs
        );
        generated.push(path);
    }

    // ── 3. 编译 bc 本体 + 生成物 ────────────────────────────────────────────
    let mut bc = cc::Build::new();
    // 第三方上游 C 源编译告警静默（MSVC -W0 / GNU -w）：cc-rs 对 MSVC 默认 -W4，
    // 上游源会刷出海量 C4244/C4100 淹没真实 error（jq-app 的 CI 实测结论）
    bc.warnings(false);
    // vcxproj 用 /std:c17（AdditionalOptions）；flag_if_supported 让不认识该
    // 旗标的编译器自动跳过
    bc.flag_if_supported("/std:c17");
    // 入口改名：main.c 的 main → FFI 入口（全树唯一 main 标识符，见头注释）
    bc.define("main", "veloterm_bc_main");
    // ── 宏集合：vs/bc.vcxproj（Release）逐项照抄 + Makefile.in:218 的 VERSION ──
    for (k, v) in bc_defines() {
        bc.define(k, v);
    }
    bc.include(&bc_include);
    for f in bc_sources.iter() {
        bc.file(f);
    }
    for f in &generated {
        bc.file(f);
    }
    println!(
        "{TAG} 编译 bc 本体 {} 源 + strgen 生成物 {} 个（-Dmain=veloterm_bc_main）",
        bc_sources.len(),
        generated.len()
    );
    bc.compile("veloterm_bc_app");

    // ── 4. 链接：仅 bcrypt（rand.c 的 BCryptGenRandom）──────────────────────
    println!("cargo:rustc-link-lib=bcrypt");

    // rerun 挂 build.rs 与整个 vendor 树（gen/ 输入与源码变更都要重编）
    println!("cargo:rerun-if-changed={}", bc_root.display());
    Ok(())
}

/// 宏集合（vcxproj Release 的 PreprocessorDefinitions 去掉 MSVC 自动项
/// NDEBUG 之外全部翻译，加 Makefile.in:218 的 VERSION）。改动前先对照
/// vendor/bc/vs/bc.vcxproj 与 Makefile.in——升级版本时若上游调整了开关，
/// 以新 vcxproj 为准同步这里与 vendor/bc/README.md。
fn bc_defines() -> Vec<(&'static str, &'static str)> {
    vec![
        // 计算器家族：bc 与 dc 双启用（argv[0] 分发，本项目固定走 bc）
        ("BC_ENABLED", "1"),
        ("DC_ENABLED", "1"),
        // 特性开关（上游默认集：extra math 开、history 开、NLS/editline/
        // readline/ossfuzz 关、bcl 库形态关——库形态下连 main 都没有）
        ("BC_ENABLE_EXTRA_MATH", "1"),
        ("BC_ENABLE_HISTORY", "1"),
        ("BC_ENABLE_NLS", "0"),
        ("BC_DEBUG_CODE", "0"),
        ("BC_ENABLE_LIBRARY", "0"),
        ("BC_ENABLE_EDITLINE", "0"),
        ("BC_ENABLE_READLINE", "0"),
        ("BC_ENABLE_OSSFUZZ", "0"),
        // 构建类型（vcxproj 用 N；A=assert heavy，仅 debug 用）
        ("BUILD_TYPE", "N"),
        // 交互模式默认值（TTY banner/prompt/expr 退出码/数字钳制等，
        // 与上游 Linux 发布物一致；非交互管道用法不受这些影响）
        ("BC_DEFAULT_BANNER", "1"),
        ("BC_DEFAULT_SIGINT_RESET", "0"),
        ("DC_DEFAULT_SIGINT_RESET", "0"),
        ("BC_DEFAULT_TTY_MODE", "1"),
        ("DC_DEFAULT_TTY_MODE", "1"),
        ("BC_DEFAULT_PROMPT", "1"),
        ("DC_DEFAULT_PROMPT", "1"),
        ("BC_DEFAULT_EXPR_EXIT", "1"),
        ("DC_DEFAULT_EXPR_EXIT", "1"),
        ("BC_DEFAULT_DIGIT_CLAMP", "1"),
        ("DC_DEFAULT_DIGIT_CLAMP", "1"),
        // Release 工程的 NDEBUG（assert 静默；vcxproj Release 同款）
        ("NDEBUG", "1"),
        // Makefile.in:218 的 -DVERSION=$(VERSION)（vm.c 的 --version 输出）
        ("VERSION", BC_VERSION),
    ]
}

/// 把相对名清单解析为绝对路径；任一缺失则报错并列出全部缺失项
fn resolve_all(base: &Path, names: &[&str]) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::with_capacity(names.len());
    let mut missing = Vec::new();
    for n in names {
        let p = base.join(n);
        if p.is_file() {
            out.push(p);
        } else {
            missing.push(p.display().to_string());
        }
    }
    if !missing.is_empty() {
        return Err(format!("以下源文件缺失（基目录 {}）：{}", base.display(), missing.join("、")));
    }
    Ok(out)
}

fn write_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 {} 失败：{e}", parent.display()))?;
    }
    fs::write(path, content).map_err(|e| format!("写 {} 失败：{e}", path.display()))
}

/// strgen（vendor/bc/gen/strgen.c）的 Rust 逐字节复刻：把 bc 脚本/帮助文本
/// 变成 `const char name[] = {…}` 字节数组 C 源。与 jq-app 的
/// render_builtin_inc 同思路（宿主生成器不可用时在 build.rs 内复刻），
/// 开发期对 4 个真实输入做过与宿主 strgen 输出的 byte-diff 验证。
///
/// 上游 CLI 形态：`strgen <input> <output> <exclude> <name> [label [define
/// [remove_tabs]]]`，本项目固定 exclude=false（extra math 开启，区块标记
/// 只剥离不剔除内容）。逐段对应 strgen.c 的 main()：
/// - 头注释嵌入 input 路径原文（上游 fprintf %s，不做转义）；
/// - label 的字符串值在 Windows 下会把反斜杠翻倍（output_label）——本构建
///   恒用 POSIX 风格相对路径（"gen/lib.bc"），翻倍逻辑保留作语义等价；
/// - 许可头剥离：先扫 `/*`，再扫 `*/` 后跟 \n/\r；上游循环后多跳一字节
///   （输入文件许可头后都有空行，即空行的 \n），再吞全部行首换行；
/// - 正文：跳 \r；remove_tabs 时跳 \t；`{{ A H N HN }}`/`{{ end }}` 标记
///   连同其后一个换行剥离（本项目 exclude=false 恒定——extra math 开启，
///   两标记的剥离动作相同，上游 exclude=true 的整段剔除分支未移植）；
///   行首（count==0）补 \t，字节按 `%d,` 打印，count 累计打印字符数、
///   超 72 折行；
/// - 尾部：行首（count==0）补两个空格再 `0\n};\n`，有 define 则
///   `#endif // <define>\n`。
pub fn render_strgen(
    input: &str,
    input_path: &str,
    name: &str,
    label: &str,
    define: &str,
    remove_tabs: bool,
) -> Result<String, String> {
    let has_label = !label.is_empty();
    let has_define = !define.is_empty();
    if input.trim().is_empty() {
        return Err(format!("{input_path} 内容为空"));
    }
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len() * 4);

    out.push_str(&format!(
        "// Copyright (c) 2018-2025 Gavin D. Howard and contributors.\n\
         // Licensed under the 2-clause BSD license.\n\
         // *** AUTOMATICALLY GENERATED FROM {input_path}. DO NOT MODIFY. ***\n\n"
    ));
    if has_label {
        out.push_str(&format!("extern const char *{label};\n\n"));
    }
    out.push_str(&format!("extern const char {name}[];\n\n"));
    if has_define {
        out.push_str(&format!("#if {define}\n"));
    }
    if has_label {
        let escaped = input_path.replace('\\', "\\\\");
        out.push_str(&format!("const char *{label} = \"{escaped}\";\n\n"));
    }
    out.push_str(&format!("const char {name}[] = {{\n"));

    // 许可头剥离（状态机与 strgen.c 逐句对应）
    let mut i = 0usize;
    let mut slashes = 0u32;
    let at = |i: usize, pat: &str| -> bool {
        let p = pat.as_bytes();
        i + p.len() <= bytes.len() && &bytes[i..i + p.len()] == p
    };
    while slashes < 2 && i < bytes.len() && bytes[i] > 0 {
        if slashes == 1
            && bytes[i] == b'*'
            && i + 2 < bytes.len()
            && bytes[i + 1] == b'/'
            && (bytes[i + 2] == b'\n' || bytes[i + 2] == b'\r')
        {
            slashes += 1;
            i += 2;
        } else if slashes == 0 && bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            slashes += 1;
            i += 1;
        }
        i += 1;
    }
    if i >= bytes.len() || bytes[i] == 0 {
        return Err(format!("{input_path} 找不到许可头注释结尾（strgen 视为非法输入）"));
    }
    // 上游循环退出后无条件再跳一字节（许可头后的空行 \n），再吞换行
    i += 1;
    while i < bytes.len() && (bytes[i] == b'\n' || bytes[i] == b'\r') {
        i += 1;
    }

    let mut count = 0usize;
    while i < bytes.len() && bytes[i] != 0 {
        let b = bytes[i];
        if b == b'\r' {
            i += 1;
            continue;
        }
        if !remove_tabs || b != b'\t' {
            if b == b'{' {
                if at(i, EX_START) || at(i, EX_END) {
                    let marker = if at(i, EX_START) { EX_START } else { EX_END };
                    i += marker.len();
                    if i < bytes.len() && bytes[i] == b'\r' {
                        i += 1;
                    }
                    i += 1;
                    continue;
                }
            }
            if count == 0 {
                out.push('\t');
            }
            // 输入为纯 ASCII（vendor 锁定版本已验证），%d 与 u8 数值一致
            let s = format!("{b},");
            count += s.len();
            out.push_str(&s);
            if count > MAX_WIDTH {
                count = 0;
                out.push('\n');
            }
        }
        i += 1;
    }
    if count == 0 {
        out.push_str("  ");
    }
    out.push_str("0\n};\n");
    if has_define {
        out.push_str(&format!("#endif // {define}\n"));
    }
    Ok(out)
}

// ── 单元测试（跨平台运行；只测纯函数，不触碰文件系统/外部命令）─────────────
#[cfg(test)]
mod tests {
    use super::*;

    /// 许可头剥离 + 基本形状：头注释嵌路径、数组名、NUL 结尾、define 守卫
    #[test]
    fn strgen_strips_license_and_wraps_array() {
        let input = "/*\n * SPDX-License-Identifier: BSD-2-Clause\n */\n\nprint \"hi\\n\"\n";
        let out = render_strgen(input, "gen/x.bc", "x_lib", "x_lib_name", "BC_ENABLED", false)
            .unwrap();
        assert!(
            out.starts_with("// *** AUTOMATICALLY GENERATED FROM gen/x.bc. DO NOT MODIFY. ***"),
            "头注释应嵌输入路径"
        );
        assert!(out.contains("extern const char *x_lib_name;\n\n"));
        assert!(out.contains("#if BC_ENABLED\n"));
        assert!(out.contains("const char *x_lib_name = \"gen/x.bc\";\n\n"));
        assert!(out.contains("const char x_lib[] = {\n"));
        // print "hi\n" 的字节（\n 是源码里的转义两字符 92,110）
        assert!(out.contains("\t112,114,105,110,116,32,34,104,105,92,110,34,10,"));
        assert!(out.ends_with("0\n};\n#endif // BC_ENABLED\n"));
    }

    /// remove_tabs：制表符剔除后不进数组
    #[test]
    fn strgen_removes_tabs_when_asked() {
        let input = "/*\n * x\n */\n\na\tb\n";
        let keep = render_strgen(input, "gen/x.bc", "x", "", "", false).unwrap();
        let strip = render_strgen(input, "gen/x.bc", "x", "", "", true).unwrap();
        assert!(keep.contains("\t97,9,98,10,"));
        assert!(strip.contains("\t97,98,10,"));
        assert!(!strip.contains("9,98"));
    }

    /// extra-math 区块标记剥离（exclude=false 形态：标记剥掉、内容保留）
    #[test]
    fn strgen_strips_extra_math_markers_but_keeps_content() {
        let input = "/*\n * x\n */\n\na\n{{ A H N HN }}\nb\n{{ end }}\nc\n";
        let out = render_strgen(input, "gen/x.bc", "x", "", "", false).unwrap();
        // a(97) b(98) c(99) 连续入数组，标记与其后换行消失
        assert!(out.contains("\t97,10,98,10,99,10,"));
        assert!(!out.contains("123,123")); // '{' = 123，标记不得进数组
    }

    /// 行宽折行：超过 72 列即换行；折行后行首补 \t
    #[test]
    fn strgen_wraps_at_max_width() {
        let input = format!("/*\n * x\n */\n\n{}\n", "1".repeat(200));
        let out = render_strgen(input.as_str(), "gen/x.bc", "x", "", "", false).unwrap();
        for line in out.lines() {
            if line.starts_with('\t') {
                assert!(line.len() <= MAX_WIDTH + 1, "折行超宽：{}", line.len());
            }
        }
        assert!(out.contains("\n\t"));
    }

    /// 非法输入（找不到许可头结尾 / 空输入）必须报错而不是产出坏文件
    #[test]
    fn strgen_rejects_invalid_inputs() {
        assert!(render_strgen("no license here", "gen/x.bc", "x", "", "", false).is_err());
        assert!(render_strgen("", "gen/x.bc", "x", "", "", false).is_err());
        assert!(render_strgen("\n \n", "gen/x.bc", "x", "", "", false).is_err());
    }

    /// 编译集合与宏清单的结构约束（变更 vendor 版本时先改这里）：
    /// 22 源对齐 vcxproj，fuzzer 排除；生成任务 4 项、lib 族带 label 且
    /// remove_tabs、help 族无 label 不折行。
    #[test]
    fn source_lists_match_upstream_vcxproj() {
        // vs/bc.vcxproj 的 ClCompile 22 个（library.c 在 LIBRARY=0 下为空 TU）
        assert_eq!(BC_SOURCES.len(), 22);
        assert!(BC_SOURCES.contains(&"main.c"));
        assert!(!BC_SOURCES.contains(&"bc_fuzzer.c"));
        assert!(!BC_SOURCES.contains(&"dc_fuzzer.c"));
        // 生成任务：Makefile.in:247-262 的四条
        assert_eq!(STRGEN_JOBS.len(), 4);
        for (_, _, name, label, define, remove_tabs) in STRGEN_JOBS {
            let is_lib = name.starts_with("bc_lib");
            assert_eq!(*remove_tabs, is_lib, "{name} 的 remove_tabs 应与上游一致");
            assert_eq!(!label.is_empty(), is_lib, "{name} 的 label 有无不符");
        }
        // 宏集合的关键开关（与 vcxproj 对齐）
        let defines = bc_defines();
        for must in [
            ("BC_ENABLED", "1"),
            ("DC_ENABLED", "1"),
            ("BC_ENABLE_EXTRA_MATH", "1"),
            ("BC_ENABLE_HISTORY", "1"),
            ("BC_ENABLE_LIBRARY", "0"),
            ("BC_ENABLE_NLS", "0"),
            ("NDEBUG", "1"),
            ("VERSION", BC_VERSION),
        ] {
            assert!(defines.contains(&must), "宏缺 {must:?}");
        }
    }
}
