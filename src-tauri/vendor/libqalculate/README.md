# vendor/libqalculate（qalc 计算器 CLI）

上游：https://github.com/Qalculate/libqalculate （GPL-2.0-or-later）
版本：**5.13.1**（锁在 `VELOTERM_VERSION`；5.13.2 只发了 Windows 安装包资产、
未附 `make dist` 源码包，故取最后一个附源码包的 5.13.1）
来源：官方 release 资产 `libqalculate-5.13.1.tar.gz`（autotools 已预生成
`configure`，CI 无需 autoconf/automake），整目录解压收入，未做删减。

## 为什么是 MinGW 而不是 MSVC

libqalculate 是 autotools 体系的 C++ 库（GMP + MPFR + libxml2 硬依赖），
上游无 CMake / MSVC 工程，官方 Windows 发行版本身就是 MSYS2/MinGW 构建
（`util.cc` 的 `_WIN32` 分支按 MinGW 移植）。在 MSVC 下从源码复刻构建
需手写 config.h、移植 libxml2/gettext 静态库并处理 GNU 扩展，成本远超
收益——jq（纯 C、零外部依赖）能走 cc 现场编译，libqalculate 不具备同等
条件。故沿用 yq 的例外形态：**源码 vendor + Windows 上经 MinGW 工具链
构建 + `qalcffi.dll` 运行时 LoadLibrary**（分发臂在
`crates/coreutils/src/qalc_impl.rs`）。

## 构建形态（`scripts/build-tools.mjs`，仅 Windows）

在 MSYS2 MinGW64 环境（windows-latest 预装 `C:\msys64`，依赖由 CI 的
pacman 步骤安装：gcc / make / pkg-config / libxml2 / gmp / mpfr）：

1. **VPATH 构建静态库**：构建目录 `src-tauri/qalc-build/`（已 gitignore，
   不污染 vendor 源码树），`configure --disable-shared --enable-static
   --enable-compiled-definitions --without-libcurl --without-icu
   --disable-nls --disable-unittests && make`。
   - `--enable-compiled-definitions`：把全部定义 XML（函数/单位/变量/
     货币/数据集 + 汇率快照）在构建期编进二进制，**运行时零数据文件**，
     不需要随安装包分发 share/qalculate。
   - `--without-libcurl`：去掉汇率在线更新（去 curl 依赖；汇率用内嵌
     快照）。`--without-icu`：去掉 Unicode 大小写折叠（去 ICU 依赖）。
   - `--disable-nls`：CLI 消息不本地化（去 gettext/intl 依赖）。
2. **链接 qalcffi.dll**：`src/qalc.cc` 以 `-Dmain=qalc_main` 单独编译
   （BusyBox 多呼叫二进制的入口改名手法），与本仓库新增的
   `veloterm_ffi.cc`（导出 `veloterm_qalc_main`）连同静态库一起
   `g++ -shared -static -static-libgcc -static-libstdc++` 链接——
   libqalculate / libxml2 / gmp / mpfr / winpthread / C++ 运行时全部
   静态打入，DLL 除系统库外零外部依赖（脚本内 objdump 依赖断言——MSYS2
   基础包不保证有 ldd，objdump 随 binutils 必在）。
3. 产物复制到 `src-tauri/binaries/qalcffi.dll`，经
   `tauri.windows.conf.json` 的 resources 放安装根目录。

## 运行时行为

- coreutils.exe 的 `qalc` 分发臂（硬链接短名 `tools\qalc.exe` 或子命令
  模式）按 exe 同目录 → 上级目录搜索 `qalcffi.dll` 后调
  `veloterm_qalc_main`，与 yqffi 完全同一套约定。
- 用户数据（qalc.cfg / 历史 / 自定义变量）落在
  `%LOCALAPPDATA%\Qalculate`（上游 `getLocalDir()` 的 Windows 语义，
  exe 目录无 `user\` 子目录时的回退），不写安装目录。

## 本地补丁（升级时需复核重放）

仅 `src/qalc.cc` 两处，均只影响 `_WIN32` 分支，搜索 `VeloTerm patch`：

1. `PUTS_UNICODE` / `FPUTS_UNICODE`：改为直接 `puts` / `fputs` 按字节
   写出。上游在 Windows 下对含 Unicode 的输出走 `fputws` 宽字符通道
   （CRT 在重定向到管道时按本地代码页转码）或 iconv 转 ANSI 代码页，
   两者在 VeloTerm 的 UTF-8 终端（xterm/ConPTY）里都会乱码。宏体必须
   保持上游「带 else 的完整 if 语句」形态（`if(true) {puts(x);} else {}`）
   ——qalc.cc 全文件 345 个调用点混用带/不带分号两种写法，裸表达式形态
   `puts(x)` 会在无分号调用点报 `expected ';'`（CI 实测 qalc.o 编译失败）。
2. `utf8_encoding` 探测：跳过 locale 探测恒为 true，让输出使用
   Unicode 数学符号（×、÷、−、≈ …）。

## 升级流程

换上游 dist tarball 解压替换本目录（保留 `VELOTERM_VERSION` /
`veloterm_ffi.cc` / 本 README）→ 改 `VELOTERM_VERSION` → 复核上述两处
补丁是否仍适用/需重放 → push 后由 CI（publish.yml）实际构建验证。
