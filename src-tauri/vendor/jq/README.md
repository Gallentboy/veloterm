# vendor/jq —— jq 1.8.2 源码（锁版本子集）

- **上游**： https://github.com/jqlang/jq
- **版本**： 1.8.2（官方 release tarball `jq-1.8.2.tar.gz`，
  SHA-256 `71b8d6e8f5fe81f6c6d0d110e3892251f6ce76ed095abd315e26e6e1193af3af`）
- **用途**： 仅 Windows。`vendor/jq-app` 的 build.rs 用 cc 把 jq 程序本体
  （`src/*.c` + `vendor/decNumber/*.c`）与捆绑的 oniguruma 现场编译并静态编入
  coreutils 多呼叫二进制，`main` 家族入口改名后由 Rust FFI 调用。
  macOS 上 jq-app 是空桩，本目录不参与任何编译。
- **升级动机**： 1.8.2 是安全修复版（修 18 个 CVE：jvp_string_append 堆溢出、
  路径深度栈溢出、解析器 NUL 截断、hash 种子随机化等）。

## 裁剪说明（相对官方 tarball）

仅保留构建与溯源所需文件，其余删除：

- 保留： `src/`（jq 本体，含预生成的 `lexer.c`/`parser.c`/`lexer.h`/`parser.h`、
  `builtin.jq`、tarball 实发的 `version.h`）、`vendor/decNumber/`（1.8 起
  decNumber 从 `src/decNumber/` 移到此处）、`vendor/oniguruma/src/` + 其
  `COPYING`（jq 的 test/match/capture 依赖的正则库，**上游 tarball 捆绑
  v6.9.10**；src 同目录的 config.h.{win32,win64,windows.in}/Makefile.windows
  等上游自带模板保留但不用，config 由 jq-app 的 build.rs 手工渲染）、
  `Makefile.am`、`configure.ac`（build.rs 推导编译集合与 config 宏的依据）、
  `COPYING`（MIT）、`AUTHORS`。
- 删除： `configure`/`Makefile.in`/`config/`/`m4`/`aclocal.m4`（autotools 生成物，
  Windows 上不能跑 configure，config 全部由 jq-app 的 build.rs 手工渲染）、
  `docs/`、`tests/`、`scripts/`、`jq.1*`、`jq.spec`、`libjq.pc*`、上游
  README/NEWS/ChangeLog、oniguruma 目录里除 src/ 与 COPYING 外的全部内容。
- `vendor/oniguruma/src/` 里 `unicode_*_data*.c`、`koi8.c`、`mktable.c` 不单独
  编译（data 文件被 `unicode.c` 直接 `#include`，后两者是工具/多余编码表），
  但属源码树组成部分，保留。

## 本地修补（升级版本时需复核是否仍需；当前 10 处）

`src/jv.c`（文件头，第 46 行）：上游 1.8.2 无条件 `#include <pthread.h>`
（1.7.1 无此行；MSVC 无该头，CI 实测 C1083）。已按 jv_thread.h 同样的
门控哲学包为 `#if !defined(WIN32) || defined(__MINGW32__)`——WIN32 非
MinGW 走文件内 Heimdal TSD 节 + jv_thread.h 的自有声明，真 pthread
平台照旧。

`src/jv.c`（WIN32 非 MinGW 的 Heimdal TSD 节）：上游的
`#ifdef _MSC_VER` 分支误写为
`static __declspec(thread) struct nomem_handler nomem_handler;`——
`nomem_handler` 是 jv_alloc.c 的私有类型，jv.c 并未定义，
MSVC 编译必报 C2079。已改为同节 `#else` 分支同型的
`static __declspec(thread) struct tls_values values;`（该节后续代码使用的
正是 `values`）。

`src/jv_alloc.h`（`jv_mem_realloc` 声明）：`__attribute__((warn_unused_result))`
为 GNU 扩展且上游未加编译器守卫，MSVC 解析该行崩坏并级联出假错误（C2143/C2091
等，CI 实测）。已用 `#ifdef __GNUC__` 包住，GNU 工具链（含 MinGW）行为不变。

`src/util.h`（MIN/MAX 宏）：原实现为语句表达式 + `__typeof__`（GNU 扩展），
MSVC 不支持。已加 `_MSC_VER` 分支改用等价三元式；1.8.2 的全部使用点
（jv.c/jv_print.c/locfile.c/compile.c/builtin.c）实参均为纯表达式、无副作用，
不涉多次求值风险。

`src/jv_dtoa_tsd.c`（`tsd_dtoa_context_get`）：裸 `inline` 定义在 MSVC 下
不产出外部符号，jv.c / jv_print.c 引用处链接失败（CI 报 LNK2019）。已去
`inline` 改普通外部定义；GNU 侧语义不变（本 TU 已包含 jv_dtoa_tsd.h 的
非 inline 声明，按 C99 规则本就编出外部定义）。

`src/util.c`（WIN32 的 `fopen` 覆盖）：上游在 WIN32 下把 `fopen` 覆盖成
同名符号（UTF-8 路径转宽字符后调 `_wfopen`）。动态 CRT 下覆盖的是导入桩、
无害；本工程 Windows 目标静态链接 CRT（`+crt-static` /MT），
`libucrt.lib(fopen.obj)` 是真实定义，同名即 LNK2005 重定义（CI 实测）。
已改名 `veloterm_jq_fopen`（行为不变），唯一调用点 `open_file` 的
`fopen(f, "r")` 在 WIN32 分支随改（非 WIN32 保持原样 `fopen`，
POSIX 路径无该定义）。

`src/main.c`（umain 开头的 WIN32 初始化节）：上游把 stdout/stderr 设为
`_O_TEXT | _O_U8TEXT`——UCRT 的宽字符 UTF-8 模式下，窄字符输出函数
（fprintf/fwrite，jq 的全部错误输出与「重定向到管道/文件」的输出即此类）
属非法操作，release 版 CRT 经 invalid parameter handler 静默丢弃：坏输入时
stderr 的 parse error 被吞（终端表现为完全无输出）、`jq ... > file` 产出
空文件。上游官方构建是 MinGW 不触发；本工程 MSVC/UCRT 静态链接实测触发。
已删去两行 `_setmode`（同节 `jv_tsd_dtoa_ctx_init` 与 `fflush` 保留；
tty 正常输出走 jv_print.c 的 `WriteConsoleW` 不经 stdio 不受影响；
`-b/--binary` 的 `_O_BINARY` 分支不变）。

`src/main.c`（诊断插桩 + 退出路径回归门禁，保留）：`VELOTERM_JQ_TIMING=1`
时在退出前向 stderr 输出分层数据。第一版只打 in-main 分段
（init/argparse/compile/exec+output/cleanup，QueryPerformanceCounter）；实测
`-n 1+1` 的 in-main 仅 ~2ms，而进程真实寿命（内核 StartTime→ExitTime，三种
计时口径一致）~450ms、`--version` 全程仅 ~33ms——缺口在 umain 之外，故扩展为：
- `pre_main`（进程创建→umain 入口，GetProcessTimes 创建时刻 + 精确系统时间差）
  与 `uptime`（进程创建→探针点），二者与脚本侧进程寿命对照即可拆出退出段；
- 内存/CPU 快照（ws / peak_ws / private / cpu / handles，K32GetProcessMemoryInfo
  动态解析，不给链接器引入 psapi.lib）；
- 退出路径二分：`VELOTERM_JQ_EXIT_MODE=quick`（`_exit`，跳过 atexit 与 stdio
  冲刷）/ `terminate`（`TerminateProcess`，再跳过 CRT 与加载器收尾）；
- `VELOTERM_JQ_EXIT_AFTER=compile`（编译后即退，隔离 exec 段）。
定位成果：正是靠它发现 `-n 1+1` 的退出码恒为 0xC0000374（见下一处补丁）。
外层驱动见 `scripts/diagnose-jq-timing.ps1`（test-jq.yml 车道自动执行，并据此
做退出路径回归门禁：出现崩溃退出码或正常退出比 TerminateProcess 慢 50ms 以上即
失败）。默认不设环境变量时探针完全静默，不影响正常使用。

`src/builtin.c`（f_strflocaltime 的 tm 字段写入）：上游 1.8.2 新代码用
`*(int *)((void *)tm + offsets[i])`——void* 指针算术是 GNU 扩展（按 1 字节
步长），MSVC 报 C2036 "void *: unknown size"（CI 实测）。已换 `(char *)tm`
承载同样字节偏移，与 GCC 扩展语义逐字节等价。

`src/jv.c`（退出期堆损坏根因，长期保留）：上游 Heimdal 风格的
WIN32 TSD shim（`pthread_setspecific`）在「替换旧值」前会调用旧值的析构器，
这不是 POSIX 语义（真 pthread 只在「键值随线程退出」时调用析构器）。jq 的退出
清理恰是「自己先 free、再 setspecific(key, NULL) 清槽」——`jv_tsd_dec_ctx_fini`
（本文件）与 `jv_tsd_dtoa_ctx_fini`（jv_dtoa_tsd.c）——于是同一指针被二次 free：
MSVC 静态 CRT 下进程退出期以 0xC0000374（STATUS_HEAP_CORRUPTION）终止，Windows
错误报告（WER）随后做崩溃事后处理（用户机实测每次 `jq -n 1+1` 退出多耗 ~450ms；
`--version` 不建 dec/dtoa 上下文故不受影响）。已删除该析构调用（线程退出路径的
`w32_service_thread_detach` 不受影响，仍按键表调用析构器）。MinGW 构建走真
pthread 分支，不触发本问题。

### 已被上游修复、随 1.8.2 升级移除的补丁

- `src/jv_file.c`（1.7.1 时的 VLA 补丁）：上游 1.8 已把
  `const int max_utf8_len = 4;` 的变长数组写法改为字面量 `char buf[4096 + 4]`，
  本地补丁不再需要。

10 处补丁均有中文注释标记。

## 构建集合的隐式项（jq-app build.rs 的收录依据）

`Makefile.am` 的 `LIBJQ_SRC` 只列 .c 文件，`lexer.c` / `parser.c`（tarball 自带
的 flex / bison 预生成物）是经 `LIBJQ_INCS` 的 `lexer.l` / `parser.y` 后缀规则
**隐式编入** libjq 的——移植到 cc 构建时必须显式收录，否则缺
`jq_parse` / `jq_parse_library`（CI 实测 LNK2019）。decNumber 的两个 .c 在
`vendor/decNumber/`（1.8 起从 src/ 移入），经 `AM_CPPFLAGS` 的 `-I vendor`
被 `#include "decNumber/decNumber.h"` 命中。

## 关键构建事实（jq-app build.rs 的依据，均已对 tarball 逐条核实）

1. **预生成 parser/lexer 在包内**： `src/lexer.c`（flex 生成）与
   `src/parser.c`（bison 生成）随 make-dist 产物分发，
   `Makefile.am` 非 maintainer-mode 下明确不重新生成
   （`.y.c`/`.l.c` 规则只回显 "NOT building"）。Windows runner 无需 flex/bison。
2. **builtin.inc 生成规则**（Makefile.am:121-129，1.8 起从 sed 字符串转义改为
   八进制字节 dump）：`od -v -A n -t o1 | sed` 管道把 `src/builtin.jq`
   逐字节转成逗号分隔的八进制整数表，消费方是 builtin.c 的
   `static const char jq_builtins[] = { #include "src/builtin.inc" '\0', }`。
   **语义关键**：sed 两条补零规则把不以 0 开头的 token（字节 64~255）补成
   前导 0 形态（144→0144）——C 整数字面量只有带前导 0 才按八进制解析，裸
   144 会被当十进制 144=0x90（非法 UTF-8，升级时实踩：内建库全损、
   `jq -n 1+1` 报 INVALID_CHARACTER，而退出码/耗时门禁全绿）。tarball 不含
   builtin.inc；jq-app build.rs 用 Rust 逐字节等价复刻（Windows runner 无
   od/sed），含全字节 0~255 的 C 八进制字面量不变量单测。
3. **version.h 随 tarball 实发**：1.8 起 tarball 自带 `src/version.h`
   （值为裸版本号 `"1.8.2"`）。build.rs 仍按 JQ_VERSION 常量在 OUT_DIR 生成
   （include 路径优先级上生成版生效），并在构建启动时与 tarball 实发值校验
   一致（双源防漂移，同 bc-app 校验 VERSION.txt 的约定）。
4. **oniguruma 捆绑**： `vendor/oniguruma`（v6.9.10，1.8.2 起从 modules/ 移到
   vendor/）随包分发，无需另行下载。其 `src/regenc.h` 无条件
   `#include "config.h"`，需提供一份最小 config.h（jq-app 生成：
   HAVE_STDINT_H/HAVE_INTTYPES_H/HAVE_SYS_TYPES_H + SIZEOF_* 系列）。6.9.10 新增
   的 `unicode_egcb_data.c`/`unicode_wb_data.c`/`unicode_property_data*.c` 与
   既有 `unicode_fold_data.c` 一样是 EXTRA_DIST（被 unicode.c 直接 #include），
   不单独编译，编译集合与 6.9.8 相同。
5. **jq 无 config.h**： configure.ac 无 `AC_CONFIG_HEADERS`，全部 `AC_DEFINE`
   走 DEFS 命令行 `-D`。jq-app 按此直接用 `cc::Build::define` 传宏，
   不渲染 jq 侧 config.h（详见 jq-app build.rs 头注释的宏取舍依据）。
6. **libm.h 变化**（1.8.2 vs 1.7.1）：新增 `modf`/`frexp`/`lgamma_r` 的
   HAVE_ 门控（前两者 UCRT 实有需定义；lgamma_r 是 glibc 扩展、UCRT 无，
   不定义、jq 回退内建不可用）；pow10 条目已删。
