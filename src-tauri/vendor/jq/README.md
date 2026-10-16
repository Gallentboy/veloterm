# vendor/jq —— jq 1.7.1 源码（锁版本子集）

- **上游**： https://github.com/jqlang/jq
- **版本**： 1.7.1（官方 release tarball `jq-1.7.1.tar.gz`，
  SHA-256 `478c9ca129fd2e3443fe27314b455e211e0d8c60bc8ff7df703873deeee580c2`）
- **用途**： 仅 Windows。`vendor/jq-app` 的 build.rs 用 cc 把 jq 程序本体
  （`src/*.c` + `src/decNumber/*.c`）与捆绑的 oniguruma 现场编译并静态编入
  coreutils 多呼叫二进制，`main` 家族入口改名后由 Rust FFI 调用。
  macOS 上 jq-app 是空桩，本目录不参与任何编译。

## 裁剪说明（相对官方 tarball）

仅保留构建与溯源所需文件，其余删除：

- 保留： `src/`（jq 本体，含预生成的 `lexer.c`/`parser.c`/`lexer.h`/`parser.h`、
  `builtin.jq`、`decNumber/`）、`modules/oniguruma/src/` + 其 `COPYING`
  （jq 的 test/match/capture 依赖的正则库，**上游 tarball 即捆绑 v6.9.8**）、
  `Makefile.am`、`configure.ac`（build.rs 推导编译集合与 config 宏的依据）、
  `COPYING`（MIT）、`AUTHORS`。
- 删除： `configure`/`Makefile.in`/`config/`/`m4`/`aclocal.m4`（autotools 生成物，
  Windows 上不能跑 configure，config 全部由 jq-app 的 build.rs 手工渲染）、
  `docs/`、`tests/`、`scripts/`、`jq.1*`、`jq.spec`、`libjq.pc*`、上游 README。
- `modules/oniguruma/src/` 里 `unicode_*_data*.c`、`koi8.c`、`mktable.c` 不单独
  编译（前者被 `unicode.c` 直接 `#include`，后两者是工具/多余编码表），
  但属源码树组成部分，保留。

## 本地修补（升级版本时需复核是否仍需）

`src/jv.c`（WIN32 非 MinGW 的 Heimdal TSD 节）：上游 1.7.1 的
`#ifdef _MSC_VER` 分支误写为 `static __declspec(thread) struct nomem_handler
nomem_handler;`——`nomem_handler` 是 jv_alloc.c 的私有类型，jv.c 并未定义，
MSVC 编译必报 C2079。已改为同节 `#else` 分支同型的
`static __declspec(thread) struct tls_values values;`（该节后续代码使用的
正是 `values`）。

`src/jv_alloc.h`（第 14 行）：`__attribute__((warn_unused_result))` 为 GNU 扩展
且上游未加编译器守卫，MSVC 解析该行崩坏并级联出假错误（C2143/C2091 等，CI
实测）。已用 `#ifdef __GNUC__` 包住，GNU 工具链（含 MinGW）行为不变。

`src/util.h`（MIN/MAX 宏）：原实现为语句表达式 + `__typeof__`（GNU 扩展），
MSVC 不支持。已加 `_MSC_VER` 分支改用等价三元式；当前唯一使用点
（jv.c 的 `MIN(DEC_MAX_DIGITS, …)`）实参无副作用，不涉多次求值风险。

`src/jv_file.c`（第 45-46 行）：`const int max_utf8_len = 4;` 不是 C 的整型
常量表达式，`char buf[4096+max_utf8_len]` 实为变长数组（GCC 支持，MSVC 不支持：
CI 报 C2057/C2466/C2133）。已改 `enum { max_utf8_len = 4 };`，值语义不变
（仅作数值使用，无取址）。

`src/jv_dtoa_tsd.c`（第 39 行）：`inline struct dtoa_context *tsd_dtoa_context_get()`
的裸 inline 定义在 MSVC 下不产出外部符号，jv.c / jv_print.c 引用处链接失败
（CI 报 LNK2019）。已去 `inline` 改普通外部定义；GNU 侧语义不变（本 TU 已
包含 jv_dtoa_tsd.h 的非 inline 声明，按 C99 规则本就编出外部定义）。

`src/util.c`（第 70 行）：上游在 WIN32 下把 `fopen` 覆盖成同名符号（UTF-8
路径转宽字符后调 `_wfopen`）。动态 CRT 下覆盖的是导入桩、无害；本工程 Windows
目标静态链接 CRT（`+crt-static` /MT），`libucrt.lib(fopen.obj)` 是真实定义，
同名即 LNK2005 重定义（CI 实测）。已改名 `veloterm_jq_fopen`（行为不变），
唯一调用点 `jq_util_input_read_more` 在 WIN32 分支随改。

六处补丁均有中文注释标记。

## 构建集合的隐式项（jq-app build.rs 的收录依据）

`Makefile.am` 的 `LIBJQ_SRC` 只列 .c 文件，`lexer.c` / `parser.c`（tarball 自带的
flex 2.6.4 / bison 预生成物）是经 `LIBJQ_INCS` 的 `lexer.l` / `parser.y` 后缀规则
**隐式编入** libjq 的——移植到 cc 构建时必须显式收录，否则缺
`jq_parse` / `jq_parse_library`（CI 实测 LNK2019）。

## 关键构建事实（jq-app build.rs 的依据，均已对 tarball 逐条核实）

1. **预生成 parser/lexer 在包内**： `src/lexer.c`（flex 2.6.4 生成）与
   `src/parser.c`（bison 生成）随 make-dist 产物分发，
   `Makefile.am` 非 maintainer-mode 下明确不重新生成
   （`.y.c`/`.l.c` 规则只回显 "NOT building"）。Windows runner 无需 flex/bison。
2. **builtin.inc 生成规则**（Makefile.am:120-122）：
   `sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/^/"/' -e 's/$$/\\n"/' src/builtin.jq`
   —— 每行：反斜杠翻倍、双引号转义、行首加 `"`、行尾加 `\n"`。
   tarball 不含 builtin.inc；jq-app build.rs 用 Rust 逐行等价复刻
   （Windows runner 无 sed）。`src/builtin.c` 以 `#include "src/builtin.inc"` 引用。
3. **oniguruma 捆绑**： `modules/oniguruma`（v6.9.8）随包分发，无需另行下载。
   其 `src/regenc.h` 无条件 `#include "config.h"`，需提供一份最小 config.h
   （jq-app 生成：HAVE_STDINT_H/HAVE_INTTYPES_H/HAVE_SYS_TYPES_H + SIZEOF_* 系列）。
4. **jq 无 config.h**： configure.ac 无 `AC_CONFIG_HEADERS`，全部 `AC_DEFINE`
   走 DEFS 命令行 `-D`。jq-app 按此直接用 `cc::Build::define` 传宏，
   不渲染 jq 侧 config.h（详见 jq-app build.rs 头注释的宏取舍依据）。
