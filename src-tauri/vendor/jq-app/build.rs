//! jq-app 构建脚本：把 jq CLI（vendor/jq，1.7.1）连同其捆绑的 oniguruma
//! 用 cc 现场编译并静态链接。零外部依赖——不需要 vcpkg / cmake / perl /
//! flex / bison / sed，Windows runner 无任何额外安装步骤。
//!
//! 仅 `target_os = windows` 时工作，其余平台直接 return（macOS 走桩模块，
//! 按 AGENTS.md 约定本机测试通过即视为完成，真实编译由 CI windows runner 验证）。
//!
//! # 依据（从 vendor/jq 的 Makefile.am / configure.ac / 源码逐条核实过的事实）
//!
//! - **编译集合**：`Makefile.am` 的 `LIBJQ_SRC` + `jq_SOURCES`：
//!   src/{builtin,bytecode,compile,execute,jq_test,jv,jv_alloc,jv_aux,jv_dtoa,
//!   jv_file,jv_parse,jv_print,jv_unicode,linker,locfile,util,jv_dtoa_tsd}.c +
//!   src/main.c + src/decNumber/{decContext,decNumber}.c + 预生成的
//!   src/lexer.c / src/parser.c（tarball 自带的 flex 2.6.4 / bison 产物，
//!   非 maintainer-mode 不重新生成）。lexer/parser 不在 LIBJQ_SRC 的 .c
//!   清单里——automake 经 LIBJQ_INCS 的 lexer.l/parser.y 后缀规则隐式编入
//!   libjq，故 JQ_SOURCES 须显式收录（漏编即缺 jq_parse/jq_parse_library，
//!   CI 实测 LNK2019）。src/inject_errors.c 是 ENABLE_ERROR_INJECTION 的
//!   测试用 LD_PRELOAD 库，不属于 jq 程序本体，排除。
//! - **oniguruma 6.9.8 随 jq tarball 捆绑**（modules/oniguruma）：编译其
//!   `src/Makefile.am` 的 `libonig_la_SOURCES` 全集（regposix/regposerr 是
//!   ENABLE_POSIX_API 可选项、koi8.c/mktable.c/unicode_*_data*.c 是
//!   EXTRA_DIST——data 文件被 unicode.c 直接 `#include`，均不单独编译）。
//! - **jq 1.7.1 没有 config.h**：configure.ac 无 `AC_CONFIG_HEADERS`，全部
//!   `AC_DEFINE` 走 DEFS 命令行 `-D` 宏。本脚本照此直接用 cc 的 define
//!   传宏（取舍见下），不渲染 jq 侧 config.h。
//! - **oniguruma 需要 config.h**：其 `src/regenc.h` 无条件
//!   `#include "config.h"`。MSVC 下最小集 = HAVE_STDINT_H / HAVE_INTTYPES_H /
//!   HAVE_SYS_TYPES_H + SIZEOF_{INT,LONG,LONG_LONG,VOIDP}（regint.h 的
//!   hash_data_type 用），渲染到独立目录避免与 jq 宏互相污染。
//! - **oniguruma 静态链接需 `ONIG_STATIC`**：oniguruma.h 的 `ONIG_EXTERN`
//!   在 MSVC 下默认展开为 `__declspec(dllimport)`（面向 DLL 消费者），静态
//!   编入同一二进制时连锁报 LNK4217。上游 CMake 的静态目标同此做法。
//! - **builtin.inc**：tarball 不带；Makefile.am:120-122 的规则是
//!   `sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/^/"/' -e 's/$$/\\n"/'`，
//!   即每行：反斜杠翻倍 → 双引号转义 → 行首加 `"` → 行尾加 `\n"`。
//!   Windows runner 无 sed，用 Rust 逐行等价实现（生成函数有单测）。
//!   消费方：builtin.c 的 `#include "src/builtin.inc"`、main.c 的
//!   `#include "src/version.h"` / `"src/config_opts.inc"`——统一生成到
//!   OUT_DIR/gen/src/ 下，把 gen 目录挂进 include 路径即可命中。
//! - **入口改名**：jq 在 WIN32 下 main.c 只有 `wmain`（宽字符入口，MinGW
//!   -municode 用）与 `umain`（真实逻辑，UTF-8 char**）；本项目 FFI 直接
//!   调 umain，`-Dumain=veloterm_jq_main` 改名（全树 grep 过：umain 标识符
//!   只在 main.c 出现，改名无副作用；wmain 成为死代码，MSVC 下无人引用）。
//!
//! # config 宏取舍（宁缺毋滥：不确定的一律不定义，jq 源码有 #ifdef 回退）
//!
//! - 定义：`WIN32`（cl 不预定义、autotools 在 shlwapi&&!cygwin 时定义；
//!   jq 的 Windows 行为——_setmode/_O_BINARY、WriteConsoleW、_wfopen、
//!   setenv 回退、_mkgmtime、get_home 的 USERPROFILE 路径——全部挂在它上）；
//!   `USE_DECNUM`；`HAVE_LIBONIG`（正则内建走捆绑 onig）；
//!   `IEEE_8087`（x64 小端，jv_dtoa 需要）；
//!   `HAVE_SETLOCALE` / `HAVE_STRFTIME` / `HAVE_GMTIME` / `HAVE_LOCALTIME`
//!   （UCRT 实有）；`HAVE__ISATTY`（→ main.c 的 USE_ISATTY，`echo {} | jq`
//!   无 filter 时默认 "."；isatty/fileno 的符号映射见垫片）；
//!   `HAVE_GETTIMEOFDAY`（MSVC 没有，由本脚本的 compat .c 用
//!   GetSystemTimeAsFileTime 实现，保住 now/1 的亚秒精度）。
//! - 数学宏 = UCRT 实有的 C99 全集 + POSIX 贝塞尔 j0/j1/jn/y0/y1/yn
//!   （MSDN 浮点支持页确认 UCRT 提供全部 C99 数学函数）。**不**定义：
//!   gamma / scalb（glibc 遗产，UCRT 无，jq 回退为内建不可用报错）、
//!   drem / exp10 / pow10 / significand（libm.h 里本就 `&& !WIN32` 关闭）。
//! - **不**定义：HAVE_PTHREAD* / HAVE___THREAD（MSVC 走 jv_alloc.c 的
//!   `__declspec(thread)` TLS 分支与 jv.c 的 Heimdal TSD 实现）、
//!   HAVE_MEMMEM / HAVE_STRPTIME / HAVE_SETENV（util.c/builtin.c 各有
//!   WIN32 回退实现）、HAVE_TIMEGM / HAVE_TM_TM_GMT_OFF（→ my_mktime 的
//!   `_mkgmtime` 分支）。
//!
//! # MSVC 垫片（jq 源码按 POSIX 假设写，MSVC 缺四样，全部脚本生成）
//!
//! - `/FI` 强制包含 veloterm_jq_shim.h：静默 CRT 弃用告警；
//!   `PATH_MAX`（util.c 用，MSVC limits.h 无）；`pthread_once_t` /
//!   `PTHREAD_ONCE_INIT` / `pthread_once`（jv_thread.h 的 WIN32 分支给了
//!   mutex 与 key 声明却漏了 once，jv.c/jv_dtoa_tsd.c 无条件使用——用
//!   INIT_ONCE 补齐）；`<malloc.h>`（main.c 的 wmain 裸调 alloca）；
//!   `JV_PRINTF_LIKE` / `JV_VPRINTF_LIKE` 空宏（jv.h 只在 `__GNUC__` 下
//!   定义、129-130 行却无条件使用，MSVC 下未定义会崩坏声明解析）；
//!   `S_ISDIR`（POSIX 宏，UCRT <sys/stat.h> 只有 _S_IFMT/_S_IFDIR——
//!   jv_file.c 用它判目录，未定义会被当隐式函数调用 → LNK2019）。
//! - 假 `unistd.h`（main/util/jv_file/lexer 包含；MSVC 无此头）：
//!   含 `<io.h>` + STDIN/STDOUT/STDERR_FILENO 宏 + isatty/fileno 映射到
//!   `_isatty`/`_fileno`（open/close/fstat/fdopen 等 POSIX 名 UCRT 头件
//!   默认 `_CRT_INTERNAL_NONSTDC_NAMES` 下本就声明并导出，无需映射）。
//! - 假 `sys/time.h`（builtin.c 无条件包含）：struct timeval +
//!   gettimeofday 声明（实现放 compat .c）；timeval 按 SDK 的
//!   `_TIMEVAL_DEFINED` 守卫包住——垫片自己先包含 `<windows.h>`，
//!   winsock.h 已定义同布局的 timeval，无守卫必报 C2011 重定义。
//! - 假 `libgen.h`（main.c/linker.c 包含）：dirname 的最小实现（POSIX
//!   语义，就地修改入参，兼容 `/` 与 `\`）。
//! - vendor/jq 六处就地修补（均见 vendor/jq/README.md，带中文注释标记）：
//!   jv.c 上游笔误（`_MSC_VER` 分支误声明 jv_alloc.c 的私有类型
//!   nomem_handler）；jv_alloc.h 无编译器守卫的
//!   `__attribute__((warn_unused_result))`；util.h 的 MIN/MAX 宏用 GNU
//!   语句表达式 + `__typeof__`（MSVC 不支持，加 `_MSC_VER` 三元式分支）；
//!   jv_file.c 的 `const int` 数组尺寸实为 VLA（MSVC 不支持，改 enum 常量）；
//!   jv_dtoa_tsd.c 的裸 `inline` 定义在 MSVC 下不产出外部符号（LNK2019，去 inline）；
//!   util.c 的 WIN32 `fopen` 同名覆盖与静态 CRT 的 fopen.obj 撞名（LNK2005，
//!   改名 veloterm_jq_fopen，调用点随改）。
//!
//! # 编译告警
//!
//! 两个 cc Build（oniguruma 与 jq 本体）均 `warnings(false)`：MSVC 下 cc-rs
//! 默认 -W4、GNU 系 -Wall，第三方上游源码会刷出 2500+ 条告警把真实 error
//! 淹没（CI 实测）。静默只作用于 C 编译，不影响 Rust 侧告警。
//!
//! jq 本体另带 `/Zc:preprocessor`（flag_if_supported 探测后添加）：MSVC 传统
//! 预处理器处理不了 jv.h 的 JV_ARRAY/JV_OBJECT 参数计数宏，各调用点连锁报
//! C2197；标准一致的预处理器与 GCC 语义对齐，jq 源码本就按此编写。
//!
//! # 链接
//!
//! 纯 C 自包含，仅 `shlwapi`（linker.c 的 PathIsRelativeA）；其余
//! kernel32 / UCRT 由 MSVC 默认带。

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// 统一日志前缀
const TAG: &str = "[jq-app]";
/// jq 版本宏值（vendor/jq 锁定版本）。**不带 `jq-` 前缀**：main.c 的
/// `printf("jq-%s\n", JQ_VERSION)` 自加前缀（→ `jq --version` 输出
/// "jq-1.7.1"），且 `$ENV.VERSION_DIR` 按「有无 `-`」分支（带前缀会被
/// 误判为开发版输出 "jq-master"）。与 tarball 实发的 src/version.h 同值。
const JQ_VERSION: &str = "1.7.1";
/// --build-configuration / $JQ_BUILD_CONFIGURATION 输出的构建描述
const JQ_CONFIG: &str = "cc static build for Windows (veloterm vendor/jq 1.7.1, oniguruma 6.9.8 bundled)";

/// jq 程序本体源（Makefile.am LIBJQ_SRC + jq_SOURCES；inject_errors.c 是
/// 测试用错误注入库，排除）。lexer.c/parser.c 不在 LIBJQ_SRC 的 .c 清单里，
/// 但 automake 经 LIBJQ_INCS 的 lexer.l/parser.y 后缀规则把预生成的 .c 隐式
/// 编入 libjq——必须显式收录，否则缺 jq_parse/jq_parse_library（CI LNK2019）。
const JQ_SOURCES: &[&str] = &[
    "builtin.c", "bytecode.c", "compile.c", "execute.c", "jq_test.c", "jv.c", "jv_alloc.c",
    "jv_aux.c", "jv_dtoa.c", "jv_file.c", "jv_parse.c", "jv_print.c", "jv_unicode.c", "linker.c",
    "locfile.c", "util.c", "jv_dtoa_tsd.c", "main.c", "lexer.c", "parser.c",
    "decNumber/decContext.c", "decNumber/decNumber.c",
];

/// 捆绑 oniguruma 源（其 src/Makefile.am 的 libonig_la_SOURCES；
/// regposix/regposerr 为可选 POSIX API、koi8.c/mktable.c 与 unicode_*_data*.c
/// 为 EXTRA_DIST，均不在编译集合）
const ONIG_SOURCES: &[&str] = &[
    "regparse.c", "regcomp.c", "regexec.c", "regenc.c", "regerror.c", "regext.c", "regsyntax.c",
    "regtrav.c", "regversion.c", "st.c", "reggnu.c", "unicode.c", "unicode_unfold_key.c",
    "unicode_fold1_key.c", "unicode_fold2_key.c", "unicode_fold3_key.c", "ascii.c", "utf8.c",
    "utf16_be.c", "utf16_le.c", "utf32_be.c", "utf32_le.c", "euc_jp.c", "euc_jp_prop.c", "sjis.c",
    "sjis_prop.c", "iso8859_1.c", "iso8859_2.c", "iso8859_3.c", "iso8859_4.c", "iso8859_5.c",
    "iso8859_6.c", "iso8859_7.c", "iso8859_8.c", "iso8859_9.c", "iso8859_10.c", "iso8859_11.c",
    "iso8859_13.c", "iso8859_14.c", "iso8859_15.c", "iso8859_16.c", "euc_tw.c", "euc_kr.c",
    "big5.c", "gb18030.c", "koi8_r.c", "cp1251.c", "onig_init.c",
];

/// libm.h 里 UCRT 实有、声明可直接命中的数学函数（MSDN：UCRT 提供全部
/// C99 数学函数；j0/j1/jn/y0/y1/yn 为 UCRT 保留的 POSIX 名）。
/// 不含：gamma/scalb（UCRT 无）与 drem/exp10/pow10/significand
/// （libm.h 在 WIN32 下强制 NO）。缺失时 jq 对应内建报
/// "X/0 is not defined"，行为与上游 configure 探测失败一致。
const JQ_MATH_FUNCS: &[&str] = &[
    "acos", "acosh", "asin", "asinh", "atan", "atan2", "atanh", "cbrt", "ceil", "copysign",
    "cos", "cosh", "erf", "erfc", "exp", "exp2", "expm1", "fabs", "fdim", "floor", "fma",
    "fmax", "fmin", "fmod", "frexp", "hypot", "ilogb", "j0", "j1", "jn", "ldexp", "lgamma",
    "log", "log10", "log1p", "log2", "logb", "nearbyint", "nextafter", "nexttoward", "pow",
    "remainder", "rint", "round", "scalbln", "scalbn", "sin", "sinh", "sqrt", "tan", "tanh",
    "tgamma", "trunc", "y0", "y1", "yn",
];

fn main() {
    // 用 CARGO_CFG_TARGET_OS（而非 HOST）判断，保证 canadian 交叉也正确
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    println!("cargo:rerun-if-changed=build.rs");
    if target_os != "windows" {
        println!("{TAG} 非 Windows target（{target_os}），按约定早退，不编译任何东西");
        return;
    }
    if let Err(msg) = run() {
        panic!("{TAG} 构建失败：{msg}\n{TAG} 提示：jq 源码应为工程内 vendor/jq（1.7.1）；\
                缺文件说明 vendor 树不完整，见 vendor/jq/README.md 的裁剪清单。");
    }
}

fn run() -> Result<(), String> {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap_or_default());
    let jq_root = manifest.join("..").join("jq");
    let jq_src = jq_root.join("src");
    let onig_src = jq_root.join("modules").join("oniguruma").join("src");

    // ── 1. 防御式发现：vendor 树的关键文件 ─────────────────────────────────
    for rel in [
        "Makefile.am",
        "configure.ac",
        "src/main.c",
        "src/builtin.jq",
        "src/lexer.c",
        "src/parser.c",
        "src/decNumber/decNumber.c",
        "modules/oniguruma/src/oniguruma.h",
    ] {
        let p = jq_root.join(rel);
        if !p.is_file() {
            return Err(format!("vendor 树缺关键文件：{}（基目录 {}）", p.display(), jq_root.display()));
        }
    }
    println!("{TAG} vendor/jq 源码就位：{}", jq_root.display());
    let jq_sources = resolve_all(&jq_src, JQ_SOURCES)?;
    let onig_sources = resolve_all(&onig_src, ONIG_SOURCES)?;

    // ── 2. 生成物：OUT_DIR 下三棵（gen=jq 的 include 生成物 / shim=MSVC 垫片
    //        / onigcfg=oniguruma 的 config.h，互不污染）────────────────────
    let genout = out_dir.join("gen");
    let shim = out_dir.join("shim");
    let onigcfg = out_dir.join("onigcfg");
    for d in [&genout, &shim, &onigcfg] {
        fs::create_dir_all(d).map_err(|e| format!("创建 {} 失败：{e}", d.display()))?;
    }
    write_file(&genout.join("src").join("version.h"), &render_version_h())?;
    write_file(&genout.join("src").join("config_opts.inc"), &render_config_opts_inc())?;
    let builtin_inc = render_builtin_inc(
        &fs::read_to_string(jq_src.join("builtin.jq"))
            .map_err(|e| format!("读 {} 失败：{e}", jq_src.join("builtin.jq").display()))?,
    )?;
    write_file(&genout.join("src").join("builtin.inc"), &builtin_inc)?;
    write_file(&shim.join("veloterm_jq_shim.h"), VELOTERM_JQ_SHIM_H)?;
    write_file(&shim.join("unistd.h"), FAKE_UNISTD_H)?;
    write_file(&shim.join("sys").join("time.h"), FAKE_SYS_TIME_H)?;
    write_file(&shim.join("libgen.h"), FAKE_LIBGEN_H)?;
    let compat_c = out_dir.join("veloterm_jq_compat.c");
    write_file(&compat_c, VELOTERM_JQ_COMPAT_C)?;
    write_file(&onigcfg.join("config.h"), &render_onig_config_h())?;
    println!(
        "{TAG} 生成物：builtin.inc（{} 行）/ version.h / config_opts.inc / MSVC 垫片 4 头 + compat.c / onig config.h",
        builtin_inc.lines().count()
    );

    // ── 3. 编译捆绑 oniguruma（独立 cc Build：宏与 include 都不与 jq 侧混）──
    let mut onig = cc::Build::new();
    onig.include(&onig_src).include(&onigcfg);
    // 静态编入同一二进制：oniguruma.h 的 ONIG_EXTERN 在 MSVC 下默认展开为
    // __declspec(dllimport)（那是给 DLL 消费者用的），静态链接时会连锁报
    // LNK4217 "imported by ... in function f_match"。上游 CMake 的静态目标正是
    // 用 ONIG_STATIC 关闭该分支（CMakeLists.txt 的 $<NOT:$<BOOL:${BUILD_SHARED_LIBS}>>）。
    onig.define("ONIG_STATIC", "1");
    // 第三方上游 C 源（oniguruma / jq / decNumber）全部编译告警静默：
    // cc-rs 对 MSVC 默认 -W4（GNU 系 -Wall），实测刷出 2500+ 条
    // C4245/C4244/C4100，把真正的 error 埋得看不见。warnings(false)
    // 由 cc-rs 发对应静默旗标（MSVC -W0 / GNU -w）。
    onig.warnings(false);
    for f in &onig_sources {
        onig.file(f);
    }
    println!("{TAG} 编译 oniguruma 6.9.8（{} 个源）", onig_sources.len());
    onig.compile("veloterm_jq_onig");

    // ── 4. 编译 jq 本体 + compat ────────────────────────────────────────────
    let mut jq = cc::Build::new();
    // 同 onig：第三方源全部告警静默（MSVC -W0 / GNU -w）
    jq.warnings(false);
    // MSVC 传统预处理器无法正确处理 jv.h 的 JV_ARRAY / JV_OBJECT 参数计数宏
    // （“__VA_ARGS__ 后跟候选列表”的 arity 分派被绑定错位，各调用点连锁报
    // C2197 "too many arguments"）。/Zc:preprocessor 切到与标准一致的预处理器，
    // 与 GCC 语义对齐——jq 源码本就按此编写；flag_if_supported 让 clang-cl
    // 等预处理器本就标准一致的编译器自动跳过。
    jq.flag_if_supported("/Zc:preprocessor");
    // 入口改名：WIN32 下 main.c 只有 wmain/umain，FFI 走 umain（UTF-8 char**）；
    // umain 标识符全树仅 main.c 出现（grep 核实），改名无副作用
    jq.define("umain", "veloterm_jq_main");
    // 平台与特性宏（取舍依据见本文件头注释）
    jq.define("WIN32", "1");
    jq.define("USE_DECNUM", "1");
    jq.define("HAVE_LIBONIG", "1");
    // jq 侧（builtin.c 等）include oniguruma.h 时同样要避开 dllimport 分支，
    // 否则 onig_* 调用被当 DLL 导入（LNK4217 的报错方正是 builtin.o）
    jq.define("ONIG_STATIC", "1");
    jq.define("IEEE_8087", "1");
    // UCRT 实有的 libc 探测宏
    jq.define("HAVE_SETLOCALE", "1");
    jq.define("HAVE_STRFTIME", "1");
    jq.define("HAVE_GMTIME", "1");
    jq.define("HAVE_LOCALTIME", "1");
    jq.define("HAVE__ISATTY", "1");
    jq.define("HAVE_GETTIMEOFDAY", "1"); // 实现在 veloterm_jq_compat.c
    for f in JQ_MATH_FUNCS {
        jq.define(&format!("HAVE_{}", f.to_uppercase()), "1");
    }
    for inc in [
        jq_src.clone(),               // jq 头 + decNumber/ 相对包含
        genout.clone(),                // 生成物 "src/builtin.inc" / "src/version.h" ...
        shim.clone(),                 // MSVC 垫片（假 unistd/sys/time/libgen）
        jq_src.join("decNumber"),     // decNumber 自身的同目录包含（保险）
        onig_src.clone(),             // <oniguruma.h>（builtin.c）
    ] {
        println!("{TAG} include += {}", inc.display());
        jq.include(&inc);
    }
    // /FI 强制包含顶层垫片（pthread_once / PATH_MAX / CRT 告警静默 / malloc.h）
    let fi = shim.join("veloterm_jq_shim.h");
    jq.flag(format!("/FI{}", fi.display()));
    println!("{TAG} /FI {}", fi.display());
    for f in jq_sources.iter() {
        jq.file(f);
    }
    jq.file(&compat_c);
    println!("{TAG} 编译 jq 本体 {} 源 + compat（-Dumain=veloterm_jq_main）", jq_sources.len());
    jq.compile("veloterm_jq_app");

    // ── 5. 链接：仅 shlwapi（linker.c 的 PathIsRelativeA）──────────────────
    println!("cargo:rustc-link-lib=shlwapi");

    // rerun 挂 build.rs 与整个 vendor 树（builtin.jq / 源码变更都要重编）
    println!("cargo:rerun-if-changed={}", jq_root.display());
    Ok(())
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

/// main.c 的 `#include "src/version.h"`：上游由 scripts/version 在 dist 时
/// 生成（tarball 内 src/version.h 即其产物，值为裸版本号 `"1.7.1"`）。
/// 本脚本生成到 OUT_DIR/gen/src/version.h —— 该路径是 `"src/version.h"`
/// 经 -I 解析时唯一命中的形态（vendor/jq/src/version.h 需 `"version.h"`
/// 拼写才可达），即 Windows 构建实际生效的就是这里渲染的值。
fn render_version_h() -> String {
    format!("#define JQ_VERSION \"{JQ_VERSION}\"\n")
}

/// main.c 的 `#include "src/config_opts.inc"`：上游取 configure 命令行，
/// 这里写等价的构建来源说明（仅 --build-configuration 展示用）
fn render_config_opts_inc() -> String {
    format!("#define JQ_CONFIG \"{JQ_CONFIG}\"\n")
}

/// 复刻 Makefile.am:120-122 的 builtin.inc 生成规则（每行：反斜杠翻倍、
/// 双引号转义、行首加 `"`、行尾加 `\n"`；sed 逐行处理、行尾无累积；
/// 文件末尾的换行不产生幽灵空行，中间的空行照样产出 `"\n"` 段）。
/// 与 sed 的差异仅限末尾换行的有无（C 拼接语义不受影响）。
/// 入参必须 LF-only：GNU sed 会把 CRLF 的 \r 当行内容拼进字符串，为了不
/// 携带这个隐患，遇 CR 直接报错（当前 vendor 的 builtin.jq 为 ASCII/LF）。
/// （pub 仅为测试/校验工具可直调；build script 根位置无可见性副作用。）
pub fn render_builtin_inc(builtin_jq: &str) -> Result<String, String> {
    if builtin_jq.trim().is_empty() {
        return Err("src/builtin.jq 内容为空".into());
    }
    if builtin_jq.contains('\r') {
        return Err("src/builtin.jq 含 CR（应为 LF-only，CRLF 会把 \\r 拼进内建源码）".into());
    }
    let mut lines: Vec<&str> = builtin_jq.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop(); // 文件以 \n 结尾：最后一“行”是文件终结符，不是空行
    }
    let mut out = String::with_capacity(builtin_jq.len() * 2);
    for line in lines {
        let escaped = line.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!("\"{escaped}\\n\"\n"));
    }
    Ok(out)
}

/// oniguruma 的 config.h（其 regenc.h 无条件包含）：MSVC 最小集。
/// 只列 regint.h 实际探测的 HAVE_ 头存在性 + hash_data_type 用的 SIZEOF。
fn render_onig_config_h() -> String {
    [
        "/* veloterm 生成：MSVC x64 最小 oniguruma config */",
        "#define HAVE_STDINT_H 1",
        "#define HAVE_INTTYPES_H 1",
        "#define HAVE_SYS_TYPES_H 1",
        "#define SIZEOF_INT 4",
        "#define SIZEOF_LONG 4",
        "#define SIZEOF_LONG_LONG 8",
        "#define SIZEOF_VOIDP 8",
        "#define SIZEOF_SIZE_T 8",
        "",
    ]
    .join("\n")
}

// ── MSVC 垫片（内容见 build.rs 头注释「MSVC 垫片」一节）───────────────────

const VELOTERM_JQ_SHIM_H: &str = r#"/* veloterm jq MSVC 顶层垫片（/FI 强制包含到每个 jq 编译单元）
 *
 * jq 源码按 POSIX/autotools 假设编写；MSVC 缺的四样在这里补两样
 * （unistd/sys/time/libgen 以同名假头放本目录，经 -I 命中）：
 *  1. CRT 弃用告警静默（isatty/fileno/fopen 等 POSIX/unsafe 名）；
 *  2. pthread_once：jq 的 jv_thread.h（WIN32 非 MinGW 分支）提供 mutex 与
 *     pthread_key_*（实体在 jv.c 的 Heimdal 节）却漏了 once，而
 *     jv.c / jv_dtoa_tsd.c 无条件使用 —— 用 INIT_ONCE 补齐；
 *  3. PATH_MAX：util.c 用，MSVC <limits.h> 无（只有 _MAX_PATH）；
 *  4. <malloc.h>：main.c 的 wmain 裸调 alloca，MSVC 声明在此头。
 */
#ifndef VELOTERM_JQ_SHIM_H
#define VELOTERM_JQ_SHIM_H

#ifndef _CRT_SECURE_NO_WARNINGS
#define _CRT_SECURE_NO_WARNINGS
#endif
#ifndef _CRT_NONSTDC_NO_DEPRECATE
#define _CRT_NONSTDC_NO_DEPRECATE
#endif

#ifndef PATH_MAX
#define PATH_MAX 4096
#endif

/* jv.h 只在 __GNUC__ 下定义 JV_PRINTF_LIKE / JV_VPRINTF_LIKE，但 129-130 行
 * 无条件用作函数声明的尾巴——MSVC 下未定义会让声明解析崩坏，级联出
 * "assuming extern returning int" / "cannot convert from 'int' to 'jv'" 一串
 * 假错误。MSVC 无 printf format 属性，空宏即正确语义（先于 jv.h 生效）。 */
#ifndef JV_PRINTF_LIKE
#define JV_PRINTF_LIKE(fmt_arg_num, args_num)
#endif
#ifndef JV_VPRINTF_LIKE
#define JV_VPRINTF_LIKE(fmt_arg_num)
#endif

#include <windows.h>   /* INIT_ONCE（pthread_once 用） */
#include <malloc.h>    /* alloca（main.c 的 wmain） */
#include <sys/stat.h>  /* _S_IFMT/_S_IFDIR（S_ISDIR 用） */

/* UCRT 的 <sys/stat.h> 没有 POSIX 的 S_ISDIR；jv_file.c 用它判目录，
 * 未定义会被当隐式函数调用（LNK2019）。语义与 POSIX 一致。 */
#ifndef S_ISDIR
#define S_ISDIR(m) (((m) & _S_IFMT) == _S_IFDIR)
#endif

typedef INIT_ONCE pthread_once_t;
#define PTHREAD_ONCE_INIT INIT_ONCE_STATIC_INIT

static BOOL CALLBACK veloterm_jq_once_thunk(PINIT_ONCE once, PVOID param, PVOID *ctx) {
    (void)once;
    (void)ctx;
    ((void (*)(void))param)();
    return TRUE;
}

__inline int pthread_once(pthread_once_t *ctl, void (*init)(void)) {
    if (!InitOnceExecuteOnce(ctl, veloterm_jq_once_thunk, (PVOID)init, NULL))
        return -1; /* jq 调用方均注释 "cannot fail" 并忽略返回值 */
    return 0;
}

#endif /* VELOTERM_JQ_SHIM_H */
"#;

const FAKE_UNISTD_H: &str = r#"/* veloterm 生成：MSVC 无 <unistd.h>；jq 的 main/util/jv_file/lexer 包含之。
 * isatty/fileno 映射到 UCRT 的下划线名；open/close/fstat/fdopen 等 POSIX
 * 名 UCRT <io.h> 在默认 _CRT_INTERNAL_NONSTDC_NAMES 下本就声明并导出。
 */
#ifndef VELOTERM_JQ_UNISTD_H
#define VELOTERM_JQ_UNISTD_H

#include <io.h>

#define isatty _isatty
#define fileno _fileno

#define STDIN_FILENO  0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2

#endif /* VELOTERM_JQ_UNISTD_H */
"#;

const FAKE_SYS_TIME_H: &str = r#"/* veloterm 生成：MSVC 无 <sys/time.h>；jq 的 builtin.c 无条件包含之。
 * gettimeofday 的实现见 veloterm_jq_compat.c。
 */
#ifndef VELOTERM_JQ_SYS_TIME_H
#define VELOTERM_JQ_SYS_TIME_H

#include <time.h> /* time_t */

/* struct timeval 与 Windows SDK 冲突（C2011 重定义）：顶层垫片先包含
 * <windows.h>，其中的 winsock.h 已定义同布局的 timeval——按 SDK 自己的
 * _TIMEVAL_DEFINED 守卫（并防御 winsock 的包含守卫）包住，谁先定义都只留
 * 一份；字段布局两者一致（long tv_sec/tv_usec）。 */
#if !defined(_TIMEVAL_DEFINED) && !defined(_WINSOCKAPI_) && !defined(_WINSOCK2API_)
#define _TIMEVAL_DEFINED
struct timeval {
    long tv_sec;
    long tv_usec;
};
#endif

int gettimeofday(struct timeval *tv, void *tz);

#endif /* VELOTERM_JQ_SYS_TIME_H */
"#;

const FAKE_LIBGEN_H: &str = r#"/* veloterm 生成：MSVC 无 <libgen.h>；jq 的 main.c/linker.c 用 dirname 取
 * JQ_ORIGIN / PROGRAM_ORIGIN。POSIX dirname 允许就地修改入参；分隔符同时
 * 接受 '/' 与 '\'（Windows 路径）。jq 不使用 basename（自有 jv_basename）。
 */
#ifndef VELOTERM_JQ_LIBGEN_H
#define VELOTERM_JQ_LIBGEN_H

#include <string.h>

__inline char *dirname(char *path) {
    char *p;
    if (path == 0 || *path == '\0')
        return ".";
    p = path + strlen(path) - 1;
    while (p > path && (*p == '/' || *p == '\\'))
        *p-- = '\0';                       /* 去掉尾部斜杠 */
    while (p > path && *p != '/' && *p != '\\')
        p--;                               /* 回退到最后一个分隔符 */
    if (p == path && *p != '/' && *p != '\\')
        return ".";                        /* 纯文件名，无目录部分 */
    if (p == path) {
        *(p + 1) = '\0';                   /* 根："/" 或 "C:\" */
        return path;
    }
    *p = '\0';
    return path;
}

#endif /* VELOTERM_JQ_LIBGEN_H */
"#;

const VELOTERM_JQ_COMPAT_C: &str = r#"/* veloterm 生成：MSVC 无 gettimeofday；jq 的 now/1（HAVE_GETTIMEOFDAY
 * 分支）需要亚秒精度。1601-01-01 → 1970-01-01 的 100ns 间隔数为
 * 116444736000000000。
 */
#include <windows.h>
#include <sys/time.h>

int gettimeofday(struct timeval *tv, void *tz) {
    FILETIME ft;
    ULARGE_INTEGER u;
    (void)tz;
    if (tv == 0)
        return -1;
    GetSystemTimeAsFileTime(&ft);
    u.LowPart = ft.dwLowDateTime;
    u.HighPart = ft.dwHighDateTime;
    u.QuadPart -= 116444736000000000ULL;
    tv->tv_sec = (long)(u.QuadPart / 10000000ULL);
    tv->tv_usec = (long)((u.QuadPart % 10000000ULL) / 10ULL);
    return 0;
}
"#;

// ── 单元测试（跨平台运行；只测纯函数，不触碰文件系统/外部命令）─────────────
#[cfg(test)]
mod tests {
    use super::*;

    /// builtin.inc 生成规则与 Makefile.am 的 sed 逐条等价：
    /// 反斜杠翻倍、双引号转义、行首 `"`、行尾 `\n"`。
    #[test]
    fn builtin_inc_matches_makefile_sed_rule() {
        let jq_src = "line1\nhas \"quotes\" and \\backslash\nlast";
        let inc = render_builtin_inc(jq_src).unwrap();
        assert_eq!(
            inc,
            "\"line1\\n\"\n\"has \\\"quotes\\\" and \\\\backslash\\n\"\n\"last\\n\"\n"
        );
    }

    /// C 语义抽查：拼出来的字符串常量逐行以 `\n` 结尾、首尾引号配对
    #[test]
    fn builtin_inc_lines_are_c_string_segments() {
        let inc = render_builtin_inc("a\nb\n").unwrap();
        for line in inc.lines() {
            assert!(line.starts_with('"') && line.ends_with("\\n\""), "坏行：{line}");
        }
    }

    /// 空输入是 vendor 树损坏，必须报错而不是产出空 builtin.inc
    #[test]
    fn builtin_inc_rejects_empty_input() {
        assert!(render_builtin_inc("").is_err());
        assert!(render_builtin_inc("\n \n").is_err());
    }

    /// 末尾换行不产生幽灵空行；中间空行产出 `"\n"` 段；含 CR 直接报错
    #[test]
    fn builtin_inc_handles_trailing_newline_and_blank_lines() {
        assert_eq!(render_builtin_inc("a\nb\n").unwrap(), "\"a\\n\"\n\"b\\n\"\n");
        assert_eq!(render_builtin_inc("a\nb").unwrap(), "\"a\\n\"\n\"b\\n\"\n");
        assert_eq!(render_builtin_inc("a\n\nb").unwrap(), "\"a\\n\"\n\"\\n\"\n\"b\\n\"\n");
        assert!(render_builtin_inc("a\r\nb\r\n").is_err());
    }

    /// version.h / config_opts.inc / onig config.h 的形状
    #[test]
    fn generated_headers_have_expected_shape() {
        // 值必须与 tarball 实发的 src/version.h 一致（裸版本号）；
        // 带 "jq-" 前缀会被 main.c 的 printf 二次加前缀成 jq-jq-1.7.1
        assert_eq!(render_version_h(), "#define JQ_VERSION \"1.7.1\"\n");
        assert!(!render_version_h().contains("\"jq-"), "版本宏不应带 jq- 前缀");
        assert!(render_config_opts_inc().starts_with("#define JQ_CONFIG \""));
        let onig = render_onig_config_h();
        for needle in [
            "#define HAVE_STDINT_H 1",
            "#define SIZEOF_VOIDP 8",
            "#define SIZEOF_LONG 4",
        ] {
            assert!(onig.contains(needle), "onig config.h 缺 {needle}");
        }
        assert!(!onig.contains("HAVE_UNISTD_H"), "MSVC 不应声明 HAVE_UNISTD_H");
    }

    /// MSVC 兼容垫片的关键不变量：timeval 与 SDK 同守卫避免 C2011；
    /// jv.h 只在 __GNUC__ 下定义的 JV_*_LIKE 宏需垫片补空定义
    #[test]
    fn shim_headers_keep_msvc_compat_invariants() {
        assert!(
            FAKE_SYS_TIME_H.contains("#if !defined(_TIMEVAL_DEFINED)"),
            "timeval 必须与 winsock 的 SDK 守卫对齐，避免 C2011 重定义"
        );
        assert!(
            VELOTERM_JQ_SHIM_H.contains("#define JV_PRINTF_LIKE(fmt_arg_num, args_num)"),
            "JV_PRINTF_LIKE 需在垫片里定义为空宏"
        );
        assert!(
            VELOTERM_JQ_SHIM_H.contains("#define JV_VPRINTF_LIKE(fmt_arg_num)"),
            "JV_VPRINTF_LIKE 需在垫片里定义为空宏"
        );
        assert!(
            VELOTERM_JQ_SHIM_H.contains("#define S_ISDIR(m)"),
            "S_ISDIR 需在垫片里补上（UCRT 无该 POSIX 宏，缺则 LNK2019）"
        );
    }

    /// 编译集合与上游 Makefile.am 的清单条数（变更 vendor 版本时先改这里）
    #[test]
    fn source_lists_match_upstream_makefile_am() {
        // LIBJQ_SRC 的 .c（17）+ decNumber 2 + main.c + 预生成 lexer.c/parser.c = 22
        assert_eq!(JQ_SOURCES.len(), 22);
        assert!(!JQ_SOURCES.contains(&"inject_errors.c"));
        assert!(JQ_SOURCES.contains(&"main.c"));
        // lexer/parser 经 LIBJQ_INCS 的 .l/.y 后缀规则隐式编入上游 libjq，
        // 本清单必须显式列（漏编缺 jq_parse/jq_parse_library，CI 实测 LNK2019）
        assert!(JQ_SOURCES.contains(&"lexer.c"));
        assert!(JQ_SOURCES.contains(&"parser.c"));
        // onig libonig_la_SOURCES 全集（无 iso8859_12，上游就没有）
        assert_eq!(ONIG_SOURCES.len(), 48);
        assert!(!ONIG_SOURCES.contains(&"iso8859_12.c"));
        for not_in in ["regposix.c", "regposerr.c", "koi8.c", "mktable.c", "unicode_fold_data.c"] {
            assert!(!ONIG_SOURCES.contains(&not_in), "{not_in} 不应单独编译");
        }
    }
}
