# vendor/bc —— gavinhoward/bc 7.1.0 源码（锁版本子集）

- **上游**： https://github.com/gavinhoward/bc
- **版本**： 7.1.0（官方 release tarball `bc-7.1.0.tar.xz`，
  SHA-256 `1f13663ba0f2435b684321714a4d0b9fff32bb951fc78dc7424cd69bba5c0d3a`）
- **用途**： 仅 Windows。`vendor/bc-app` 的 build.rs 用 cc 把 bc 程序本体
  （`src/*.c` 的 22 个编译单元 + 4 个 strgen 生成物）现场编译并静态编入
  coreutils 多呼叫二进制，`main` 入口经 `-Dmain=veloterm_bc_main` 改名后由
  Rust FFI 调用。macOS 上 bc-app 是空桩，本目录不参与任何编译。

## 裁剪说明（相对官方 tarball）

仅保留构建与溯源所需文件，其余删除：

- 保留： `src/`（bc/dc 本体；`bc_fuzzer.c`/`dc_fuzzer.c` 是 OSS-FUZZ 专属
  入口，不编译但属源码树组成部分）、`include/`（全部头文件）、`gen/`
  （strgen 生成器 + 4 个生成输入：`lib.bc`/`lib2.bc`/`bc_help.txt`/
  `dc_help.txt`）、`Makefile.in`（strgen 调用参数与宏集合的依据）、
  `vs/bc.vcxproj`（**上游官方 MSVC 工程**：编译集合、全套 BC_* 宏、
  `/std:c17`、链接 `bcrypt.lib` 的依据）、`VERSION.txt`、`LICENSE.md`
  （BSD-2-Clause）、`NOTICE.md`。
- 删除： `configure`/`configure.sh`（POSIX shell 构建入口，Windows 不能跑；
  宏集合已由 vcxproj 给出）、`build.rig`/`build.pkg.rig`/`build.gaml`/
  `project/`（rig 构建系统）、`compile_flags.txt`、`bcl.pc.in`、`vs/` 其余
  文件（.sln/.filters/bcl.vcxproj/tests）、`manuals/`、`tests/`、`locales/`
  （NLS 目录，BC_ENABLE_NLS=0 不用）、`scripts/`、`MAINTENANCE-TERMS.md`、
  `MEMORY_BUGS.md`、`NEWS.md`、上游 README.md。

## 本地修补

**无**。bc 7.x 上游自带官方 MSVC 支持（`vs/` 工程随 tarball 分发），
全部 POSIX 头包含均有 `#ifndef _WIN32` 守卫，MSVC 编译零补丁。
（与 jq 1.7.1 的五处 MSVC 修补形成对比；升级版本时若上游回归了
MSVC 兼容性，再在此登记。）

## 关键构建事实（bc-app build.rs 的依据，均已对 tarball 逐条核实）

1. **编译集合**（`vs/bc.vcxproj` 的 ClCompile，与 `Makefile.in` 的 BC_OBJ
   一致）： `src/{args,bc,bc_lex,bc_parse,data,dc,dc_lex,dc_parse,file,
   history,lang,lex,library,main,num,opt,parse,program,rand,read,vector,vm}.c`
   共 22 个；`bc_fuzzer.c`/`dc_fuzzer.c` 仅 OSS-FUZZ（BC_ENABLE_OSSFUZZ）。
2. **无 config.h**： bc 全部构建开关走命令行 `-D` 宏（vcxproj 与 Makefile.in
   的 CPPFLAGS 均如此），MSVC 下不需要渲染任何 config 头。
   上游 vcxproj 的全套宏见 bc-app build.rs 的 `defines` 清单；
   另有 `Makefile.in:218` 的 `-DVERSION=$(VERSION)`（值取 VERSION.txt）
   与 `-DNDEBUG`（Release 工程）。
3. **strgen 生成物**（4 个，Makefile.in:247-262 与 vcxproj 的 CustomBuild
   参数一致）：
   - `gen/lib.bc` → lib.c：参数 `0 bc_lib bc_lib_name BC_ENABLED 1`
     （remove_tabs=1）
   - `gen/lib2.bc` → lib2.c：参数 `0 bc_lib2 bc_lib2_name BC_ENABLED 1`
   - `gen/bc_help.txt` → bc_help.c：参数 `0 bc_help "" BC_ENABLED 0`
   - `gen/dc_help.txt` → dc_help.c：参数 `0 dc_help "" DC_ENABLED 0`
   tarball 不含生成物，上游在构建时编译宿主 strgen（`gen/strgen.c`）现场
   生成。Windows runner 无需该宿主工具：bc-app build.rs 用 Rust 逐字节
   复刻 strgen（许可头剥离、`{{ A H N HN }}`/`{{ end }}` extra-math 标记、
   `%d,` 字节数组、72 列折行），对 4 个输入与真实 strgen 输出做过
   byte-diff 验证（见 build.rs 头注释）。生成物带 `#if BC_ENABLED` /
   `#if DC_ENABLED` 守卫，编译时须与本体同宏。
4. **入口分发**： `src/main.c` 的 `main`（`-Dmain` 改名后为 FFI 入口）按
   argv[0] basename 首字符分发：`vm->name[0] != 'd'` → `bc_main`，
   `== 'd'` → `dc_main`（`vm.h` 的 `BC_IS_BC`/`BC_IS_DC`）。本项目固定传
   argv[0]="bc"（只注册 bc 命令；dc 是同一二进制的另一名字，上游 vcxproj
   的 PostBuildEvent `copy bc.exe dc.exe` 即此形态）。参数解析器
   `bc_opt_init` 置 `optind=1`——argv[0] 按程序名跳过，与 jq umain 同约。
5. **链接**： 仅 `bcrypt.lib`（`src/rand.c` 的 `BCryptGenRandom`）；
   history 的 Win32 Console API 走默认引入的 kernel32。
   `_WIN32` 由 MSVC 预定义（全树不使用裸 `WIN32`），无需 -DWIN32。
