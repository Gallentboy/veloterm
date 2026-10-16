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
正是 `values`）。补丁处有中文注释标记。

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
