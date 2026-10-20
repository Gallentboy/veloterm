#!/usr/bin/env node
// 构建 coreutils 多呼叫二进制并复制到 Tauri externalBin 目录（仅 Windows 需要），
// 同时产出随包 resources 分发的五个 FFI DLL：veloterm_goffi.dll（yq+fzf，Go）、
// qalcffi.dll（libqalculate，MSYS2 MinGW64）、nmapffi/ncatffi/npingffi.dll（nmap
// 三件套，MSVC+CMake）与 vim64.dll（vim，官方 MSVC nmake VIMDLL 形态）。
// nmap 段额外随包分发 nmap-data/ 数据文件树，vim 段随包分发 vim-runtime/。
// Tauri v2 的 externalBin 要求文件名带 target triple 后缀：
//   src-tauri/binaries/coreutils-x86_64-pc-windows-msvc.exe
// 安装后 Tauri 会去掉 triple 后缀放到主程序同级目录（coreutils.exe），
// 短名命令入口（ls.exe/grep.exe/…）由应用启动钩子以 NTFS 硬链接创建。
// macOS 打包不消费该产物（externalBin 只在 tauri.windows.conf.json 声明，
// 硬链接安装钩子也是 cfg(windows)），故非 Windows target 直接跳过，
// 避免在 mac 上无谓编译整套 vendor 工具链依赖。
//
// 用法：node scripts/build-tools.mjs [--target <triple>]
// 未指定 target 时取 rustc 默认 host triple。

import { execSync, execFileSync, spawnSync } from 'node:child_process';
import { cpSync, mkdirSync, existsSync, rmSync, readFileSync, writeFileSync, copyFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const srcTauri = join(root, 'src-tauri');
const outDir = join(srcTauri, 'binaries');

const TOOL = 'coreutils';

const args = process.argv.slice(2);
let target = '';
const targetFlag = args.indexOf('--target');
const explicitTarget = targetFlag !== -1;
if (targetFlag !== -1 && args[targetFlag + 1]) {
  target = args[targetFlag + 1];
} else {
  target = execSync('rustc -vV', { encoding: 'utf8' })
    .split('\n')
    .find((line) => line.startsWith('host:'))
    ?.slice('host:'.length)
    .trim();
}

if (!target) {
  console.error('[build-tools] 无法解析 rustc host triple');
  process.exit(1);
}

console.log(`[build-tools] target: ${target}`);

// arm64 分支：目标为 aarch64 时工具链假设切换——MSYS2 走 CLANGARM64 环境
// （编译器为 clang，依赖 mingw-w64-clang-aarch64-gcc-compat 提供 gcc/g++ 兼容名）、
// MSVC/cmake/nmake 走 ARM64、GOAMD64 仅对 x86_64 有意义
const isArm64 = target.startsWith('aarch64-');

// --only <段>：局部构建（coreutils / goffi / qalc / vim / nmap / ncat / nping）。
// tag 触发的 test-*.yml 快速反馈车道用
// `--only qalc` 只依赖 MSYS2；`--only vim` 只依赖 MSVC nmake（零外部包，最轻）；
// `--only nmap|ncat|nping` 走 MSVC+CMake——三者同属
// Nmap 项目、同工程（veloterm-win/CMakeLists.txt），`--only nmap` 出全部三个
// DLL，`--only ncat` / `--only nping` 只出对应那一个（cmake --target 单目标，
// 用于增量验证）；goffi 段（yq+fzf 合并 DLL）只依赖 Go。不传时全量构建
// （publish 路径），行为不变
const onlyFlag = args.indexOf('--only');
const only = onlyFlag !== -1 && args[onlyFlag + 1] ? args[onlyFlag + 1] : '';
if (only && !['coreutils', 'goffi', 'qalc', 'vim', 'nmap', 'ncat', 'nping'].includes(only)) {
  console.error(
    `[build-tools] 未知 --only 段：${only}（可选 coreutils/goffi/qalc/vim/nmap/ncat/nping）`,
  );
  process.exit(1);
}
if (only) {
  console.log(`[build-tools] 局部构建：${only}`);
}

// 工具集仅随 Windows 安装包分发：macOS/Linux 打包链路无 externalBin 消费方，
// 跳过以免在 mac 上整套编译 uutils/ripgrep/eza 等 vendor 依赖（本地跑工具
// 集测试用 cargo test -p coreutils，与本打包脚本无关）
if (!target.includes('windows')) {
  console.log(`[build-tools] ${target} 非 Windows 目标，无需 coreutils，跳过`);
  process.exit(0);
}

// 工程约定：Windows 包只能由 Windows 主机/runner 出，禁止在 mac 上交叉编译
// Windows 目标（既慢且从未验证过，openssl-app/putty-telnet 的 vcpkg 与 MSVC
// 工具链在 mac 上也不可用）
if (target.includes('windows') && process.platform !== 'win32') {
  console.error(
    '[build-tools] 严禁在非 Windows 主机上编译 Windows 目标，请交给 Windows runner（CI）或 Windows 机器',
  );
  process.exit(1);
}

const exeExt = target.includes('windows') ? '.exe' : '';

// 编译 coreutils 多呼叫二进制（固定 release 供打包）。
// coreutils 已拆为独立 workspace member（crates/coreutils）：根包 app 的默认
// bin 选择不再包含它，必须显式 -p。该 crate 无 build.rs，构建不会触及 app 的
// tauri_build（externalBin 存在性检查），不存在鸡生蛋问题——若历史上曾见
// binaries/ 下残留空占位文件，那是旧版单包时代的工作区，现已不再需要。
// 关键：显式传了 --target 时透传给 cargo —— cargo 对带 --target 的构建使用
// 独立的 target/<triple>/ 缓存目录，与不带 --target 的 target/release 完全不共享；
// CI 里 tauri build 带 --target x86_64-pc-windows-msvc，这里保持一致才能命中同一份缓存，
// 否则整套依赖会被重编两遍。本地未传 --target 时与本地 `tauri build`（也不带）同样一致
const cargoTargetArgs = explicitTarget ? `--target ${target} ` : '';
if (!only || only === 'coreutils') {
  execSync(
    `cargo build --release ${cargoTargetArgs}-p ${TOOL} --bin ${TOOL}`,
    { cwd: srcTauri, stdio: 'inherit' },
  );
}

// 清空并重建 externalBin 目录（--only 局部构建不清空：避免误删其他段的产物）
if (!only) {
  rmSync(outDir, { recursive: true, force: true });
}
mkdirSync(outDir, { recursive: true });

// cargo 未传 --target 时输出到 target/release（无 triple 子目录）；
// 传了 --target 时输出到 target/<triple>/release，与上方构建参数对应取用；
// 本地未传时兼容两处都存在的历史产物
if (!only || only === 'coreutils') {
  const plainOut = join(srcTauri, 'target', 'release');
  const tripleOut = join(srcTauri, 'target', target, 'release');
  const cargoOut = explicitTarget
    ? tripleOut
    : existsSync(join(tripleOut, TOOL + exeExt))
      ? tripleOut
      : plainOut;

  const src = join(cargoOut, TOOL + exeExt);
  if (!existsSync(src)) {
    console.error(`[build-tools] 缺少编译产物: ${src}`);
    process.exit(1);
  }

  const dest = join(outDir, `${TOOL}-${target}${exeExt}`);
  cpSync(src, dest);
  console.log(`[build-tools] ${TOOL} -> ${dest}`);
}
if (only === 'coreutils') {
  console.log('[build-tools] 完成');
  process.exit(0);
}

// ── yq + fzf（Go）→ 单一 veloterm_goffi.dll：coreutils 的 yq、fzf 命令经
// 合并 DLL 按符号动态导入（veloterm_yq_main / veloterm_fzf_main，分发臂见
// crates/coreutils 的 goffi_impl.rs，运行时按 exe 同目录/上级目录定位）。
// Go 官方不支持 MSVC 链接 c-archive（golang/go#74189），静态链入 coreutils.exe
// 不可能；buildmode=c-shared 的 DLL 是唯一形态。合并架构：宿主取 vendor/fzf
// 的 package main（自带 FFI 入口与 shell 资源 embed，上游逻辑在根 main 包、
// 无法被外部 import），经 vendor/go.work 跨模块引 yq 的 cmd 包，fzf 树内
// veloterm_goffi.go（tag veloterm_goffi）追加导出 yq 入口并在 init 关闭常规
// GC（单次 CLI 语义，1GB 软上限兜底）。两份 Go runtime 合一，体积省约三成。
// 版本锁在各 vendor 目录的 VELOTERM_VERSION，经 ldflags 注入让两个
// --version 都打出真实版本。
// GOAMD64=v3：以 AVX2 为基线（2013+ Intel/AMD）换取向量化性能，基线以下的
// CPU 会非法指令崩溃——性能与兼容的权衡，回退只需删掉该环境变量。
if (!only || only === 'goffi') {
  try {
    execSync('go version', { stdio: 'ignore' });
  } catch {
    console.error('[build-tools] 未找到 go 工具链（veloterm_goffi.dll 构建必需）：请安装 Go ≥ 1.26 或将其加入 PATH');
    process.exit(1);
  }
  // c-shared 需要 cgo：CGO_ENABLED=1 + gcc。windows-latest 镜像通常自带 MinGW
  // 且在 PATH（GitHub 文档不保证，实例间可能变化），找不到时按常见安装位置
  // 探测（choco mingw / msys2 / 手动安装）并前置 PATH，全找不到才报错退出。
  function ensureGccOnPath() {
    try {
      execSync('gcc --version', { stdio: 'ignore' });
      return 'PATH';
    } catch {
      /* 继续探测 */
    }
    const candidates = [
      'C:\\mingw64\\bin',
      'C:\\tools\\mingw64\\bin',
      'C:\\ProgramData\\mingw64\\bin',
      'C:\\ProgramData\\chocolatey\\lib\\mingw\\tools\\install\\mingw64\\bin',
      'C:\\ProgramData\\chocolatey\\lib\\mingw\\tools\\install\\mingw32\\bin',
      'C:\\msys64\\mingw64\\bin',
      'C:\\tools\\msys64\\mingw64\\bin',
      // arm64：MSYS2 CLANGARM64 环境（gcc.exe 由 gcc-compat 包提供，
      // CI 的 pacman 步骤负责安装）
      'C:\\msys64\\clangarm64\\bin',
    ];
    for (const dir of candidates) {
      if (existsSync(join(dir, 'gcc.exe'))) {
        process.env.PATH = `${dir};${process.env.PATH}`;
        console.log(`[build-tools] PATH 未含 gcc，探测到 ${dir}\\gcc.exe，已前置到 PATH`);
        return dir;
      }
    }
    return null;
  }
  if (!ensureGccOnPath()) {
    console.error(
      '[build-tools] 未找到 gcc（cgo 编译 veloterm_goffi.dll 必需）：windows-latest 通常自带 MinGW，' +
        '若镜像变化请安装 MinGW-w64 或把其 bin 目录加入 PATH',
    );
    process.exit(1);
  }
}

// 合并构建：宿主 = vendor/fzf（package main，veloterm_ffi.go 导出 fzf 入口），
// vendor/go.work 跨模块引 yq 的 cmd 包，fzf 树内 veloterm_goffi.go 导出 yq
// 入口 + GC 关闭；双 tag 同时激活两个入口文件，单 DLL 双符号
if (!only || only === 'goffi') {
  const goffiHost = join(srcTauri, 'vendor', 'fzf');
  const yqVersion = readFileSync(join(srcTauri, 'vendor', 'yq', 'VELOTERM_VERSION'), 'utf8').trim();
  const fzfVersion = readFileSync(join(goffiHost, 'VELOTERM_VERSION'), 'utf8').trim();
  console.log(`[build-tools] goffi: yq ${yqVersion} + fzf ${fzfVersion} -> veloterm_goffi.dll (${isArm64 ? 'arm64' : 'GOAMD64=v3'})`);
  execSync(
    `go build -buildmode=c-shared -tags "veloterm_ffi veloterm_goffi" -trimpath -buildvcs=false ` +
      `-ldflags "-s -w -X github.com/mikefarah/yq/v4/cmd.GitDescribe=${yqVersion} -X main.version=${fzfVersion} -X main.revision=" ` +
      `-o "${join(outDir, 'veloterm_goffi.dll')}" .`,
    { cwd: goffiHost, stdio: 'inherit',
      env: { ...process.env, CGO_ENABLED: '1', ...(isArm64 ? {} : { GOAMD64: 'v3' }) } },
  );
  console.log(`[build-tools] veloterm_goffi.dll (yq ${yqVersion} + fzf ${fzfVersion}) -> ${join(outDir, 'veloterm_goffi.dll')}`);
}
if (only === 'goffi') {
  console.log('[build-tools] 完成');
  process.exit(0);
}

// ── qalc / nmap（MSYS2 MinGW64 段共用）：环境定位 ──────────────────────
// 两段同形态（vendor 源码 autotools 构建 + FFI DLL，见各自 vendor 树 README）；
// windows-latest 预装 C:\msys64，依赖由 CI 的 pacman 步骤按段安装（探测不同包集）
const msysNeeded = !only || only === 'qalc';
let bashExe = '';
let mingwEnv = null;
if (msysNeeded) {
  // 定位 MSYS2（windows-latest 预装 C:\msys64；MSYS2_ROOT 可覆盖）
  function locateMsys2() {
    const roots = [];
    if (process.env.MSYS2_ROOT) roots.push(process.env.MSYS2_ROOT);
    roots.push('C:\\msys64', 'C:\\tools\\msys64');
    for (const dir of roots) {
      if (existsSync(join(dir, 'usr', 'bin', 'bash.exe'))) return dir;
    }
    return null;
  }
  const msysRoot = locateMsys2();
  if (!msysRoot) {
    console.error(
      '[build-tools] 未找到 MSYS2（qalcffi.dll/nmapffi.dll 构建必需，需 usr/bin/bash.exe）：' +
        '请安装 MSYS2（默认 C:\\msys64）或设 MSYS2_ROOT 指向其根目录',
    );
    process.exit(1);
  }
  bashExe = join(msysRoot, 'usr', 'bin', 'bash.exe');
  // 登录 shell 里 /etc/profile 按 MSYSTEM 前置对应工具链到 PATH：
  // x86_64 目标用 MINGW64（g++/make/pkg-config 即 x86_64-w64-mingw32 工具链），
  // aarch64 目标用 CLANGARM64（clang 工具链，gcc/g++ 由 gcc-compat 包提供兼容名）
  mingwEnv = { ...process.env, MSYSTEM: isArm64 ? 'CLANGARM64' : 'MINGW64' };
}

if (!only || only === 'qalc') {
// ── qalc（libqalculate C++）：coreutils 的 qalc 命令经 qalcffi.dll 动态导入 ───
// libqalculate 是 autotools 体系的 C++ 库（GMP/MPFR/libxml2 硬依赖），上游
// 无 MSVC 工程且官方 Windows 版即 MinGW 构建——不像 jq（纯 C 自包含）能
// cc 现场编译链入，只能走 MSYS2 MinGW64 工具链（windows-latest 预装
// C:\msys64，依赖由 CI 的 pacman 步骤安装）。构建分两步：
//   1. VPATH 静态构建：src-tauri/qalc-build/（gitignore，不污染 vendor 树）
//      configure --enable-compiled-definitions 把全部定义 XML 编进二进制
//      （运行时零数据文件），--without-libcurl/--without-icu/--disable-nls
//      去掉可选重依赖；
//   2. 链接 DLL：src/qalc.cc 以 -Dmain=qalc_main 改名单独编译（BusyBox
//      多呼叫二进制的入口改名手法），与 veloterm_ffi.cc（导出
//      veloterm_qalc_main）连同静态库一起 -static 链成自包含 qalcffi.dll，
//      库依赖清单取 configure 生成的 libqalculate.la 的 dependency_libs
//      （-lgmp -lmpfr -lbcrypt …），libxml2 传递依赖由 pkg-config --static 补齐。
// 分发臂见 crates/coreutils 的 qalc_impl.rs（与 yqffi 同一套运行时定位约定）。
const qalcSrc = join(srcTauri, 'vendor', 'libqalculate');
const qalcVersion = readFileSync(join(qalcSrc, 'VELOTERM_VERSION'), 'utf8').trim();
console.log(`[build-tools] qalc ${qalcVersion}: MSYS2 ${isArm64 ? 'CLANGARM64' : 'MinGW64'} autotools 构建`);

// 工具链与依赖探测：编译器 + libxml2/gmp/mpfr 开发包（.pc），缺什么给出
// 对应 pacman 安装命令（CI 的 Install MSYS2 packages for qalc 步骤负责装）
const PACMAN_QALC = isArm64
  ? 'pacman -S --needed --noconfirm make gettext mingw-w64-clang-aarch64-gcc-compat \
' +
    'mingw-w64-clang-aarch64-clang mingw-w64-clang-aarch64-pkg-config \
' +
    'mingw-w64-clang-aarch64-libxml2 mingw-w64-clang-aarch64-gmp mingw-w64-clang-aarch64-mpfr'
  : 'pacman -S --needed --noconfirm make gettext mingw-w64-x86_64-gcc ' +
    'mingw-w64-x86_64-pkg-config mingw-w64-x86_64-libxml2 ' +
    'mingw-w64-x86_64-gmp mingw-w64-x86_64-mpfr';
try {
  execFileSync(
    bashExe,
    [
      '-lc',
      'set -e; command -v g++ make pkg-config >/dev/null; ' +
        'pkg-config --exists libxml-2.0 gmp mpfr',
    ],
    { env: mingwEnv, stdio: 'ignore' },
  );
} catch {
  console.error('[build-tools] MSYS2 MINGW64 环境缺少 qalc 构建依赖，请在 MSYS2 中执行：');
  console.error(`  ${PACMAN_QALC}`);
  process.exit(1);
}

const qalcBuild = join(srcTauri, 'qalc-build');
// String.raw：脚本里的 sed 转义（\( \1）需原样到达 bash，不能被 JS 模板
// 字符串的转义处理吃掉
const qalcScript = String.raw`
set -e
SRC=$(cygpath -u '${qalcSrc}')
BUILD=$(cygpath -u '${qalcBuild}')
mkdir -p "$BUILD"
cd "$BUILD"
# 增量构建 + 陈旧配置防护：Makefile 已存在（上次构建残留）且 configure 入参
# 未变时跳过 configure；升级 vendor/libqalculate 或改参数后必须清目录重配——
# 沿用旧 Makefile/config.h 会静默产出与预期配置不符的 DLL（CI 是 fresh
# runner 不受影响，本地 Windows 开发机续跑会踩）
# 构建标志（test-*.yml 快速车道传 -O0 -g 的 debug 配置）参与陈旧配置指纹：
# 切换 flags 自动清目录重配；configure 自动读取环境里的 CFLAGS/CXXFLAGS
#（指纹取值由 JS 侧读 process.env 插入——本脚本是 String.raw 模板，shell 的
# 参数展开写法不能直接写进模板，会被当成 JS 插值解析）
STAMP_HASH=$(printf '%s' "$SRC/configure@--disable-shared@--enable-static@--enable-compiled-definitions@--without-libcurl@--without-icu@--disable-nls@--disable-unittests@CFLAGS=${process.env.CFLAGS ?? ''}@CXXFLAGS=${process.env.CXXFLAGS ?? ''}" | cksum)
if [ ! -f Makefile ] || [ "$(cat .veloterm-configure-stamp 2>/dev/null || true)" != "$STAMP_HASH" ]; then
  find . -mindepth 1 -maxdepth 1 -exec rm -rf {} +
  # 静态链 libxml2 的关键：编译期定义 LIBXML_STATIC，让 libqalculate 的目标
  # 文件引用普通 xml* 符号而非 __imp_xml* dllimport 桩——静态 libxml2.a 只有
  # 普通符号，缺此定义 -static 链接报上千处 undefined reference（CI 实测，
  # 上游 make 能过是因为 libtool 默认链的是动态导入库）。CFLAGS/CXXFLAGS
  # 未设时给 -g -O2（configure 语义：显式设置即全量取代默认）；本脚本是
  # String.raw 模板，只能用无花括号的 shell 写法
  if [ -z "$CFLAGS" ]; then CFLAGS="-g -O2"; fi
  if [ -z "$CXXFLAGS" ]; then CXXFLAGS="-g -O2"; fi
  CFLAGS="$CFLAGS -DLIBXML_STATIC"
  CXXFLAGS="$CXXFLAGS -DLIBXML_STATIC"
  export CFLAGS CXXFLAGS
  sh "$SRC/configure" --disable-shared --enable-static --enable-compiled-definitions \
    --without-libcurl --without-icu --disable-nls --disable-unittests
  printf '%s\n' "$STAMP_HASH" > .veloterm-configure-stamp
fi
make -j"$(nproc)"
# qalc 入口改名编译：-Dmain=qalc_main（仅本翻译单元），编译参数对齐
# src/Makefile.am 的 AM_CPPFLAGS（config.h 在 builddir 根，support.h 经
# HAVE_CONFIG_H 引入）
g++ -c -O2 -DHAVE_CONFIG_H -Dmain=qalc_main -DLIBXML_STATIC \
  -I"$BUILD" -I"$SRC" -I"$SRC/libqalculate" \
  -DPACKAGE_DATA_DIR='"/qalculate-data"' \
  $(pkg-config --cflags libxml-2.0) \
  "$SRC/src/qalc.cc" -o qalc_main.o
# 静态库产物路径：libtool 版本差异可能在 .libs/ 或目录直下，取实际存在者
QALIB=libqalculate/.libs/libqalculate.a
[ -f "$QALIB" ] || QALIB=libqalculate/libqalculate.a
# 静态库的链接期依赖取 libtool 记录（剥掉 -R/-rpath 运行时路径项），
# libxml2 的传递依赖（zlib/lzma/iconv…）由 pkg-config --static 补齐
DEPS=$(sed -n "s/^dependency_libs='\(.*\)'$/\1/p" libqalculate/libqalculate.la \
  | tr ' ' '\n' | grep -v -e '^-R' -e '^-rpath' | tr '\n' ' ')
g++ -shared -static -static-libgcc -static-libstdc++ -o qalcffi.dll \
  "$SRC/veloterm_ffi.cc" qalc_main.o "$QALIB" \
  $DEPS $(pkg-config --static --libs libxml-2.0)
# 自包含断言：objdump 列出 DLL 依赖（ldd 在 MSYS2 基础包不保证存在，
# objdump 随 binutils 必在），除 Windows 系统库外不得有 MinGW/MSYS 动态库
DLLS=$(objdump -p qalcffi.dll | sed -n 's/\s*DLL Name: //p')
echo "$DLLS"
if echo "$DLLS" | grep -Eiq 'mingw|msys-|libstdc|libgcc|winpthread|xml2|gmp|mpfr|zlib|lzma|iconv|zstd'; then
  echo '[build-tools] qalcffi.dll 存在未静态化的 MinGW/MSYS 依赖（见上）' >&2
  exit 1
fi
`;
execFileSync(bashExe, ['-lc', qalcScript], { env: mingwEnv, stdio: 'inherit' });
cpSync(join(qalcBuild, 'qalcffi.dll'), join(outDir, 'qalcffi.dll'));
console.log(`[build-tools] qalcffi.dll (${qalcVersion}) -> ${join(outDir, 'qalcffi.dll')}`);
}
if (only === 'qalc') {
  console.log('[build-tools] 完成');
  process.exit(0);
}

// ── vim（MSVC nmake，官方路径）：coreutils 的 vim 命令经 vim64.dll 动态导入 ──
// 走上游官方 VIMDLL 拆分（src/INSTALLpc.txt 的 MSVC 构建方式，见
// vendor/vim/README.md）：nmake VIMDLL=yes 产出瘦启动器 vim.exe + 主体
// vim64.dll（导出 VimMain，MSWIN 下无条件用 GetCommandLineW 自解析进程命令行，
// 故 vim 属 OWN_ARGV 家族——仅硬链接短名模式可用）。随包只带 DLL 与
// vim-runtime/ 数据树（上游仓库自带 doc/tags，无需构建期 helptags）。
// 分发臂见 crates/coreutils 的 vim_impl.rs（DLL 随包放安装根 lib/ 子目录，
// 候选解析走共享模块 crate::dll_paths，同其他 *ffi）。
if (!only || only === 'vim') {
  const vimSrc = join(srcTauri, 'vendor', 'vim');
  const vimVersion = readFileSync(join(vimSrc, 'VELOTERM_VERSION'), 'utf8').trim();
  console.log(`[build-tools] vim ${vimVersion}: MSVC nmake VIMDLL 构建（${isArm64 ? 'arm64' : 'x64'}）`);

  // nmake 需 VS 环境：vswhere 定位最新含 VC 工具链的安装 → vcvarsall.bat
  // 带架构参数（x64 / arm64，与 relay 车道同一手法；windows-latest 与
  // windows-11-arm 各自预装对应 VS。找不到时给出可诊断提示。
  const vswhereCandidates = [
    process.env['ProgramFiles(x86)']
      ? join(process.env['ProgramFiles(x86)'], 'Microsoft Visual Studio', 'Installer', 'vswhere.exe')
      : '',
    'C:\\Program Files (x86)\\Microsoft Visual Studio\\Installer\\vswhere.exe',
  ].filter(Boolean);
  let vcvars = '';
  for (const vswhere of vswhereCandidates) {
    if (!existsSync(vswhere)) continue;
    // 架构对应的 VC 工具集组件：x64 目标为 VC.Tools.x86.x64，arm64 为 VC.Tools.ARM64
    const component = isArm64
      ? 'Microsoft.VisualStudio.Component.VC.Tools.ARM64'
      : 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64';
    const vsPath = execSync(
      `"${vswhere}" -latest -products * -requires ${component} -property installationPath`,
      { encoding: 'utf8' },
    ).trim();
    if (vsPath) {
      const cand = join(vsPath, 'VC', 'Auxiliary', 'Build', 'vcvarsall.bat');
      if (existsSync(cand)) vcvars = cand;
      break;
    }
  }
  if (!vcvars) {
    console.error(
      `[build-tools] 未找到 Visual Studio ${isArm64 ? 'ARM64' : 'x64'} 环境（vcvarsall.bat，vim 构建必需）：` +
        '请安装含 “使用 C++ 的桌面开发” 的 VS 2022，或确认 vswhere.exe 存在',
    );
    process.exit(1);
  }

  // nmake 需在源内构建（Make_mvc.mak 全相对路径，生成 pathdef.c/ObjX/）。
  // 为不写脏 vendor 树，先整目录拷贝 src/ 到 src-tauri/vim-build/（与
  // qalc-build/nmap-build 同一约定），在副本里构建
  const vimBuild = join(srcTauri, 'vim-build');
  rmSync(vimBuild, { recursive: true, force: true });
  const buildSrc = join(vimBuild, 'src');
  cpSync(join(vimSrc, 'src'), buildSrc, { recursive: true });

  // 官方形态：VIMDLL=yes（隐含 GUI=yes，DLL 同时编入 GUI/控制台代码，运行期
  // 按宿主 exe 的 PE subsystem 选择）+ CPU=AMD64/ARM64 + 静态 CRT（未设 USE_MSVCRT
  // 时 Make_mvc 自动 /MT）。目标只要 vim.exe（依赖链自动先出 vim64.dll），
  // gvim.exe/xxd/install 等不构建。vcvarsall 带目标架构，arm64 宿主上即原生 ARM64 工具链
  const msvcArch = isArm64 ? 'arm64' : 'x64';
  const nmakeCpu = isArm64 ? 'ARM64' : 'AMD64';
  execSync(
    `call "${vcvars}" ${msvcArch} >nul && cd /d "${buildSrc}" && nmake /nologo -f Make_mvc.mak VIMDLL=yes CPU=${nmakeCpu} vim.exe`,
    { stdio: 'inherit' },
  );

  const vimDll = join(buildSrc, 'vim64.dll');
  const vimRunner = join(buildSrc, 'vim.exe');
  for (const p of [vimDll, vimRunner]) {
    if (!existsSync(p)) {
      console.error(`[build-tools] 缺少构建产物: ${p}`);
      process.exit(1);
    }
  }

  // runtime 数据树 staging（vendor 树已裁剪 GUI 专属内容，直接整目录拷贝）
  rmSync(join(outDir, 'vim-runtime'), { recursive: true, force: true });
  cpSync(join(vimSrc, 'runtime'), join(outDir, 'vim-runtime'), { recursive: true });

  // 构建期冒烟：--version + 无终端 headless 编辑（-es 静默 ex 模式，替换并写回
  // 后校验内容）——启动器与 vim64.dll 同目录加载，冒烟通过即证明 DLL 导出与
  // runtime 树可用
  const rtEnv = { ...process.env, VIMRUNTIME: join(outDir, 'vim-runtime') };
  console.log('[build-tools] 冒烟：vim --version / -es 编辑替换');
  execSync(`"${vimRunner}" --version`, { stdio: 'inherit', env: rtEnv });
  const smokeFile = join(vimBuild, 'smoke.txt');
  writeFileSync(smokeFile, 'hello veloterm\n');
  execSync(
    `"${vimRunner}" -u NONE -es -c "%s/hello/world/" -c "wq" "${smokeFile}" < NUL`,
    { stdio: 'inherit', env: rtEnv },
  );
  if (!readFileSync(smokeFile, 'utf8').includes('world')) {
    console.error('[build-tools] vim -es 冒烟未产生预期替换结果');
    process.exit(1);
  }

  // filetype 加载链冒烟：不带 -u NONE（无用户 vimrc 时加载 defaults.vim →
  // `filetype on` → filetype.vim 的 BufRead 自动命令链）。上游 9.1 起
  // filetype 检测走 autoload/dist/ 的 dist#ft#* / dist#script#* 函数——
  // runtime 树缺件（首次 vendor 曾丢 autoload/dist/ 整目录）时该链报
  // E117，而 -u NONE 冒烟恰好绕开 defaults.vim、车道全绿（真机开文件
  // 才弹错）。此档断言 stderr 无 E1xx 错误码且替换仍然生效。
  const ftSmokeFile = join(vimBuild, 'ft-smoke.txt');
  writeFileSync(ftSmokeFile, 'hello filetype\n');
  const ftSmoke = spawnSync(
    vimRunner,
    ['-es', '-c', '%s/filetype/ftloaded/', '-c', 'wq', ftSmokeFile],
    { env: rtEnv, encoding: 'utf8', stdin: 'ignore' },
  );
  const ftSmokeErr = String(ftSmoke.stderr ?? '');
  if (/\bE\d{3}/.test(ftSmokeErr) || ftSmoke.status !== 0) {
    console.error(
      `[build-tools] vim filetype 链冒烟报错（runtime 缺件？autoload/dist/ 必须保留）：\n${ftSmokeErr}`,
    );
    process.exit(1);
  }
  if (!readFileSync(ftSmokeFile, 'utf8').includes('ftloaded')) {
    console.error('[build-tools] vim filetype 链冒烟未产生预期替换结果');
    process.exit(1);
  }

  cpSync(vimDll, join(outDir, 'vim64.dll'));
  console.log(`[build-tools] vim64.dll + vim-runtime/ (${vimVersion}) -> ${join(outDir)}`);
}
if (only === 'vim') {
  console.log('[build-tools] 完成');
  process.exit(0);
}

// ── nmap（MSVC + CMake）：coreutils 的 nmap 命令经 nmapffi.dll 动态导入 ──────
// 走**上游官方 Windows 路径**（mswin32 的 MSVC 工程）的等价实现：直接链
// Npcap SDK 的 wpcap.lib（pcap_*）+ packet.lib（Packet*）+ MSVC 原生
// /DELAYLOAD:packet.dll;wpcap.dll——缺 Npcap 的机器 DLL 仍可加载，运行期由
// winfix.cc 的 SEH 探测降级 connect 扫描。故**不编译 bundled libpcap**
// （MinGW 路线下踩的 flex/bison/asprintf/man 页/probe 全部不存在）。
// 官方 sln 只有 Win32 平台，故用自建 CMake（vendor/nmap/veloterm-win/
// CMakeLists.txt，源清单/宏/include/库清单复刻官方 vcxproj）出 x64 DLL。
// 构建期另产 runner nmap.exe（不随包分发）：生成 scripts/script.db 与无
// Npcap 冒烟。数据文件细节见 vendor/nmap/README.md。
const nmapSrc = join(srcTauri, 'vendor', 'nmap');
const nmapVersion = readFileSync(join(nmapSrc, 'VELOTERM_VERSION'), 'utf8').trim();
console.log(`[build-tools] nmap ${nmapVersion}: MSVC + CMake 构建（${isArm64 ? 'arm64' : 'x64'}）`);

// 依赖探测：cmake（runner 自带）+ OpenSSL（CI 的 vcpkg 安装步骤提供，
// 与 SQLCipher / openssl-app 同源，triplet 随目标架构：x64/arm64-windows-static）
const opensslDir = process.env.OPENSSL_DIR ?? `C:\\vcpkg\\installed\\${isArm64 ? 'arm64' : 'x64'}-windows-static`;
if (!existsSync(join(opensslDir, 'lib', 'libcrypto.lib'))) {
  console.error(
    `[build-tools] 未找到 OpenSSL 静态库（nmapffi.dll 构建必需）：${opensslDir}\\lib\\libcrypto.lib 不存在。\n` +
      `  CI 上由 publish.yml 的 vcpkg 步骤安装并导出 OPENSSL_DIR；本地请设 OPENSSL_DIR 指向 ${isArm64 ? 'arm64' : 'x64'}-windows-static 安装根。`,
  );
  process.exit(1);
}

const nmapBuild = join(srcTauri, 'nmap-build');
rmSync(nmapBuild, { recursive: true, force: true });
mkdirSync(nmapBuild, { recursive: true });

// 1. Npcap SDK（版本与上游 mswin32/Build.bat 同源：它下载 npcap-sdk-1.16.zip；
// 解到 nmap-build 内，缓存复用）。
// 注意：SDK 只提供**导入库**（wpcap.lib/packet.lib 仅数十 KB，内容是跳转到
// DLL 的桩），Npcap 官方从不发布静态库；且真正抓包的内核驱动 npf.sys 只能由
// 安装程序装入系统——故 Npcap 无法静态链接进本 DLL（PE 导入表已证实只延迟
// 导入 wpcap.dll/packet.dll，无 Npcap 代码）。SDK 版本仅决定编译期期望的
// 符号集（即用户所需的最低 Npcap 版本）。
const sdkDir = join(nmapBuild, 'npcap-sdk');
const sdkInclude = join(sdkDir, 'Include', 'pcap.h');
if (!existsSync(sdkInclude)) {
  const NPCAP_SDK_VER = '1.16';
  // SDK zip 随仓库 vendor（337KB）：npcap.com 的服务器证书屡次过期（官方续证
  // 延迟，CI 实测 SEC_E_CERT_EXPIRED 挂过整条 publish），构建不再依赖该站点。
  const vendoredZip = join(root, 'src-tauri', 'vendor', 'nmap', `npcap-sdk-${NPCAP_SDK_VER}.zip`);
  if (!existsSync(vendoredZip)) {
    console.error(`[build-tools] 缺少 vendor 内的 Npcap SDK：${vendoredZip}`);
    process.exit(1);
  }
  const zip = join(nmapBuild, 'npcap-sdk.zip');
  console.log(`[build-tools] 解包仓库内 Npcap SDK ${NPCAP_SDK_VER}`);
  copyFileSync(vendoredZip, zip);
  mkdirSync(sdkDir, { recursive: true });
  execSync(
    `powershell -NoProfile -Command "Expand-Archive -LiteralPath '${zip}' -DestinationPath '${sdkDir}' -Force"`,
    { stdio: 'inherit' },
  );
  if (!existsSync(sdkInclude)) {
    console.error(`[build-tools] Npcap SDK 解压后仍缺 ${sdkInclude}`);
    process.exit(1);
  }
}

// 2. CMake 配置 + 构建（生成器用 VS 多配置；-A x64 出 64 位）
// --only ncat / --only nping 时只构建对应目标（cmake --target，用于增量验证）；
// 否则全量（nmap 段含三个 DLL，publish 与 --only nmap 都走这里）。
const cmakeSrc = join(nmapSrc, 'veloterm-win');
const buildDir = join(nmapBuild, 'cmake');
execSync(
  `cmake -S "${cmakeSrc}" -B "${buildDir}" -A ${isArm64 ? 'ARM64' : 'x64'} ` +
    `-DNPCAP_SDK_DIR="${sdkDir}" -DOPENSSL_DIR="${opensslDir}"`,
  { stdio: 'inherit' },
);
const cmakeTargets =
  only === 'ncat' ? '--target ncatffi ncat-runner' : only === 'nping' ? '--target npingffi nping-runner' : '';
execSync(`cmake --build "${buildDir}" --config Release --parallel ${cmakeTargets}`, {
  stdio: 'inherit',
});

// 需要的产物：--only ncat/nping 只要对应一组；否则三组齐全
const wantNmap = only !== 'ncat' && only !== 'nping';
const wantNcat = !only || only === 'nmap' || only === 'ncat';
const wantNping = !only || only === 'nmap' || only === 'nping';

const builtDll = join(buildDir, 'out', 'nmapffi.dll');
const builtRunner = join(buildDir, 'out', 'nmap.exe');
const builtNcatDll = join(buildDir, 'out', 'ncatffi.dll');
const builtNcatRunner = join(buildDir, 'out', 'ncat.exe');
const builtNpingDll = join(buildDir, 'out', 'npingffi.dll');
const builtNpingRunner = join(buildDir, 'out', 'nping.exe');
const required = [
  ...(wantNmap ? [builtDll, builtRunner] : []),
  ...(wantNcat ? [builtNcatDll, builtNcatRunner] : []),
  ...(wantNping ? [builtNpingDll, builtNpingRunner] : []),
];
for (const p of required) {
  if (!existsSync(p)) {
    console.error(`[build-tools] 缺少构建产物: ${p}`);
    process.exit(1);
  }
}

// 3. 数据文件 staging + script.db 预生成 + 无 Npcap 冒烟（仅 nmap 段需要）
// 数据文件清单取上游 Makefile.in 的 install-nmap/install-nse 目标
// （docs/nmap.xsl、docs/nmap.dtd、nmap-services/-os-db/-service-probes/
// -protocols/-rpc/-mac-prefixes、nse_main.lua、scripts/、nselib/）。
// 注：旧版的 nmap-payloads 在 7.992 已不存在（payload 数据内置在 payload.cc）。
const dataStage = join(nmapBuild, 'nmap-data');
if (wantNmap) {
  rmSync(dataStage, { recursive: true, force: true });
  mkdirSync(join(dataStage, 'scripts'), { recursive: true });
  mkdirSync(join(dataStage, 'nselib'), { recursive: true });
  for (const f of [
    'nmap-services',
    'nmap-os-db',
    'nmap-service-probes',
    'nmap-protocols',
    'nmap-rpc',
    'nmap-mac-prefixes',
    'nse_main.lua',
  ]) {
    cpSync(join(nmapSrc, f), join(dataStage, f));
  }
  cpSync(join(nmapSrc, 'docs', 'nmap.xsl'), join(dataStage, 'nmap.xsl'));
  cpSync(join(nmapSrc, 'docs', 'nmap.dtd'), join(dataStage, 'nmap.dtd'));
  cpSync(join(nmapSrc, 'scripts'), join(dataStage, 'scripts'), { recursive: true });
  cpSync(join(nmapSrc, 'nselib'), join(dataStage, 'nselib'), { recursive: true });

  // script.db 预生成：NSE 运行期索引，安装目录通常只读、用户跑
  // --script-updatedb 会失败，故构建期生成随包分发。--script-updatedb 会先过
  // win_init（无 Npcap → 降级）再扫 scripts/ 写 script.db。
  console.log('[build-tools] 生成 scripts/script.db（nmap --script-updatedb）');
  execSync(`"${builtRunner}" --script-updatedb`, {
    stdio: 'inherit',
    env: { ...process.env, NMAPDIR: dataStage },
  });
  if (!existsSync(join(dataStage, 'scripts', 'script.db'))) {
    console.error('[build-tools] --script-updatedb 未生成 scripts/script.db');
    process.exit(1);
  }

  // 构建期冒烟（CI 无 Npcap：正好回归 /DELAYLOAD + winfix 降级路径）
  console.log('[build-tools] 冒烟：nmap --version / -sT 127.0.0.1');
  execSync(`"${builtRunner}" --version`, {
    stdio: 'inherit',
    env: { ...process.env, NMAPDIR: dataStage },
  });
  execSync(`"${builtRunner}" -n -sT -p 1 -T5 --max-retries 1 127.0.0.1`, {
    stdio: 'inherit',
    env: { ...process.env, NMAPDIR: dataStage },
  });
}

// 4. ncat / nping 构建期冒烟（同 nmap：CI 无 Npcap，正好回归 /DELAYLOAD 降级）
// ncat --version 打印版本后退出；-z 是「零 I/O 扫描」模式，对回环做一次连接
// 尝试即可验证运行期可用（不依赖 Npcap 的原始包能力）。nping 的 --version
// 与一次本地 TCP 探测同理。
if (wantNcat) {
  console.log('[build-tools] 冒烟：ncat --version');
  execSync(`"${builtNcatRunner}" --version`, { stdio: 'inherit' });
  // ncat 对回环做一次 connect（-z 零 I/O 模式；退出码 0=端口开放，非 0 亦可
  // 接受，关键是进程能正常启动与退出）——用 try 包住，只要求不因缺 DLL 而无法启动。
  try {
    execSync(`"${builtNcatRunner}" -z -w 1 127.0.0.1 1`, { stdio: 'inherit' });
  } catch (e) {
    console.log(`[build-tools] ncat -z 冒烟返回非 0（端口关闭属正常）：${e.status}`);
  }
}
if (wantNping) {
  console.log('[build-tools] 冒烟：nping --version');
  execSync(`"${builtNpingRunner}" --version`, { stdio: 'inherit' });
  // nping 对回环发一个 TCP 探测包（--tcp -p 1，count=1）
  try {
    execSync(`"${builtNpingRunner}" --tcp -p 1 -c 1 --delay 10ms 127.0.0.1`, { stdio: 'inherit' });
  } catch (e) {
    console.log(`[build-tools] nping 探测冒烟返回非 0（无 Npcap 时的降级行为）：${e.status}`);
  }
}

// 5. 复制产物到 externalBin 目录（按 only 分段；--only ncat/nping 只拷对应产物）
if (wantNmap) {
  cpSync(builtDll, join(outDir, 'nmapffi.dll'));
  rmSync(join(outDir, 'nmap-data'), { recursive: true, force: true });
  cpSync(dataStage, join(outDir, 'nmap-data'), { recursive: true });
}
if (wantNcat) {
  cpSync(builtNcatDll, join(outDir, 'ncatffi.dll'));
  // ncat 的 SSL 客户端证书束：ncat 在 exe 同目录查找 ca-bundle.crt（缺失仅
  // debug 告警、不致命）；DLL 形态下 GetModuleHandle(0) 取到宿主 coreutils.exe
  // 路径，故随包放安装根即可命中。
  cpSync(join(nmapSrc, 'ncat', 'certs', 'ca-bundle.crt'), join(outDir, 'ca-bundle.crt'));
}
if (wantNping) {
  cpSync(builtNpingDll, join(outDir, 'npingffi.dll'));
}
console.log(`[build-tools] nmap 段产物 (${nmapVersion}) -> ${join(outDir)}`);

console.log('[build-tools] 完成');
