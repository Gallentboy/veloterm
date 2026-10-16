#!/usr/bin/env node
// 构建 coreutils 多呼叫二进制并复制到 Tauri externalBin 目录（仅 Windows 需要），
// 同时产出随包 resources 分发的四个 FFI DLL：yqffi.dll 与 fzfffi.dll（Go）、
// qalcffi.dll（libqalculate）与 nmapffi.dll（nmap，均 MSYS2 MinGW64 构建）。
// nmap 段额外随包分发 nmap-data/ 数据文件树（scripts/nselib/服务与版本库）。
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

import { execSync, execFileSync } from 'node:child_process';
import { cpSync, mkdirSync, existsSync, rmSync, readFileSync } from 'node:fs';
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

// --only <段>：局部构建（coreutils / goffi / qalc / nmap）。tag 触发的
// test-*.yml 快速反馈车道用
// `--only qalc` / `--only nmap` 只产对应 DLL——该段只依赖 MSYS2（不需要 cargo
// 全量编译、Go 与 vcpkg OpenSSL），反馈环最短；goffi 段（yq+fzf 合并 DLL）
// 只依赖 Go。不传时全量构建（publish 路径），行为不变
const onlyFlag = args.indexOf('--only');
const only = onlyFlag !== -1 && args[onlyFlag + 1] ? args[onlyFlag + 1] : '';
if (only && !['coreutils', 'goffi', 'qalc', 'nmap'].includes(only)) {
  console.error(`[build-tools] 未知 --only 段：${only}（可选 coreutils/goffi/qalc/nmap）`);
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
  console.log(`[build-tools] goffi: yq ${yqVersion} + fzf ${fzfVersion} -> veloterm_goffi.dll (GOAMD64=v3)`);
  execSync(
    `go build -buildmode=c-shared -tags "veloterm_ffi veloterm_goffi" -trimpath -buildvcs=false ` +
      `-ldflags "-s -w -X github.com/mikefarah/yq/v4/cmd.GitDescribe=${yqVersion} -X main.version=${fzfVersion} -X main.revision=" ` +
      `-o "${join(outDir, 'veloterm_goffi.dll')}" .`,
    { cwd: goffiHost, stdio: 'inherit',
      env: { ...process.env, CGO_ENABLED: '1', GOAMD64: 'v3' } },
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
const msysNeeded = !only || only === 'qalc' || only === 'nmap';
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
  // MSYSTEM=MINGW64 + 登录 shell：/etc/profile 按该变量前置 /mingw64/bin，
  // bash 内的 g++/make/pkg-config 即 x86_64-w64-mingw32 工具链
  mingwEnv = { ...process.env, MSYSTEM: 'MINGW64' };
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
console.log(`[build-tools] qalc ${qalcVersion}: MSYS2 MinGW64 autotools 构建`);

// 工具链与依赖探测：编译器 + libxml2/gmp/mpfr 开发包（.pc），缺什么给出
// 对应 pacman 安装命令（CI 的 Install MSYS2 packages for qalc 步骤负责装）
const PACMAN_QALC =
  'pacman -S --needed --noconfirm make gettext mingw-w64-x86_64-gcc ' +
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

// ── nmap（C/C++ autotools）：coreutils 的 nmap 命令经 nmapffi.dll 动态导入 ──
// 形态同 qalc 的 MSYS2 MinGW64 段（构建目录 src-tauri/nmap-build/，gitignore），
// 但多三件事（细节与依据见 vendor/nmap/README.md）：
//   1. Npcap SDK（npcap-sdk-1.13.zip，与上游 mswin32/Build.bat 同源）下载 +
//      shim：MSYS2 的 libpcap 包是空后端不能用，bundled libpcap 走 CMake
//      （-DPacket_ROOT=SDK -DBUILD_SHARED_LIBS=OFF）出静态库——pcap_* 全编
//      进 DLL，外部导入收敛为 Packet.dll 一个；libdnet 的 --with-wpdpack 只
//      认 include/lib 平铺布局，故另出 shim 目录；
//   2. Packet.dll 以 -Wl,--delayload 延迟导入 + winfix.cc 的 MinGW 降级补丁
//      （上游 SEH/delayimp 降级被 _MSC_VER 门控）：未装 Npcap 的机器
//      LoadLibrary(nmapffi.dll) 仍成功，nmap 降级 connect() 扫描；
//   3. 数据文件：nmap 在 Windows 下默认找 exe 同目录，DLL 形态下取到的是宿
//      主 coreutils.exe 路径，故运行期由 nmap_impl.rs 设 NMAPDIR 指向随包的
//      nmap-data/。构建期产出 runner nmap.exe（= 上游链接行 + winfix.o，仅
//      构建期用、不随包分发）：生成 scripts/script.db（make install-nse 依赖）、
//      make install-nmap/install-nse staging 数据树，并跑 --version 与
//      -sT 127.0.0.1 冒烟——CI 无 Npcap，正好回归优雅降级路径。
const nmapSrc = join(srcTauri, 'vendor', 'nmap');
const nmapVersion = readFileSync(join(nmapSrc, 'VELOTERM_VERSION'), 'utf8').trim();
console.log(`[build-tools] nmap ${nmapVersion}: MSYS2 MinGW64 autotools 构建`);

// 工具链与依赖探测：编译器 + cmake/ninja（libpcap 用）+ unzip（解 Npcap SDK）
// + openssl 开发包（.pc），缺什么给出对应 pacman 安装命令（CI 的
// Install MSYS2 packages 步骤负责装）
const PACMAN_NMAP =
  'pacman -S --needed --noconfirm make gettext unzip mingw-w64-x86_64-gcc ' +
  'mingw-w64-x86_64-pkg-config mingw-w64-x86_64-cmake mingw-w64-x86_64-ninja ' +
  'mingw-w64-x86_64-openssl';
try {
  execFileSync(
    bashExe,
    [
      '-lc',
      'set -e; command -v g++ make cmake ninja pkg-config unzip >/dev/null; ' +
        'pkg-config --exists openssl',
    ],
    { env: mingwEnv, stdio: 'ignore' },
  );
} catch {
  console.error('[build-tools] MSYS2 MINGW64 环境缺少 nmap 构建依赖，请在 MSYS2 中执行：');
  console.error(`  ${PACMAN_NMAP}`);
  process.exit(1);
}

const nmapBuild = join(srcTauri, 'nmap-build');
// String.raw：脚本内的 bash 参数展开（$(...)、${#var%pattern}）需原样到达
// bash，不能被 JS 模板字符串的转义/插值处理吃掉（同 qalc）
const nmapScript = String.raw`
set -e
SRC=$(cygpath -u '${nmapSrc}')
BUILD=$(cygpath -u '${nmapBuild}')
mkdir -p "$BUILD"
cd "$BUILD"
CURL=$(command -v curl || echo /c/Windows/System32/curl.exe)

# ── 1. Npcap SDK（版本锁；缓存到 stamp 指向的目录，重复构建免下载） ──
NPCAP_SDK_VER=1.13
SDK_DIR=$(cat npcap-sdk.stamp 2>/dev/null || true)
if [ -z "$SDK_DIR" ] || [ ! -d "$SDK_DIR" ]; then
  rm -rf sdk-extract npcap-sdk.zip
  "$CURL" -fsSL -o npcap-sdk.zip "https://npcap.com/dist/npcap-sdk-$NPCAP_SDK_VER.zip"
  unzip -q npcap-sdk.zip -d sdk-extract
  SDK_DIR=$(cd "$(dirname "$(find sdk-extract -maxdepth 2 -type d -name Include | head -1)")" && pwd)
  printf '%s\n' "$SDK_DIR" > npcap-sdk.stamp
fi
# libdnet --with-wpdpack 只认 <dir>/include + <dir>/lib 平铺布局（SDK 的 Lib/x64 提一级）
rm -rf npcap-shim
mkdir -p npcap-shim/include npcap-shim/lib
cp "$SDK_DIR"/Include/* npcap-shim/include/
cp "$SDK_DIR"/Lib/x64/* npcap-shim/lib/

# ── 2. bundled libpcap → 静态库（CMake 官方 Windows 路线，1.10.7 预生成
#    scanner/grammar，无需 flex/bison；D-Bus/RDMA 在 MinGW 不可用，显式关掉
#    免探测噪声） ──
PCAP_STAGE=$(pwd)/pcap-stage
if [ ! -f "$PCAP_STAGE/lib/libpcap.a" ]; then
  rm -rf libpcap-build pcap-stage
  cmake -G Ninja -S "$SRC/libpcap" -B libpcap-build \
    -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
    -DDISABLE_DBUS=ON -DDISABLE_RDMA=ON -DPacket_ROOT="$SDK_DIR"
  cmake --build libpcap-build
  cmake --install libpcap-build --prefix "$PCAP_STAGE"
  # SDK 头一并并入 stage/include：nmap 侧 winfix.cc 要 Packet32.h
  cp "$SDK_DIR"/Include/* "$PCAP_STAGE/include/"
fi

# ── 3. nmap 主体：整树拷贝 in-tree 构建（七个子 configure，VPATH 未经
#    验证）；陈旧配置指纹同 qalc 约定，CFLAGS/CXXFLAGS 变化自动清目录重配
#    （test-nmap 车道传 -O0 -g） ──
STAMP_HASH=$(printf '%s' "$SRC/configure@--with-libpcap=stage@--with-wpdpack=shim@--with-openssl=/mingw64@--with-liblua=included@--with-libpcre=included@--with-libz=included@--with-liblinear=included@--without-libssh2@--without-ncat@--without-nping@--without-zenmap@--without-ndiff@LIBS=-lpacket@CFLAGS=${process.env.CFLAGS ?? ''}@CXXFLAGS=${process.env.CXXFLAGS ?? ''}" | cksum)
if [ ! -f nmap-src/Makefile ] || [ "$(cat .veloterm-nmap-stamp 2>/dev/null || true)" != "$STAMP_HASH" ]; then
  rm -rf nmap-src
  cp -a "$SRC" nmap-src
  cd nmap-src
  if [ -z "$CFLAGS" ]; then CFLAGS="-g -O2"; fi
  if [ -z "$CXXFLAGS" ]; then CXXFLAGS="-g -O2"; fi
  export CFLAGS CXXFLAGS
  # LIBS=-lpacket + LDFLAGS=-Lshim/lib：libdnet.a 与 winfix.o 的 Packet* 符号
  # 经 MSVC 导入库 Packet.lib 进最终链接（GNU ld 的 -l 搜索含 packet.lib；
  # NTFS 大小写不敏感命中 Packet.lib）
  ./configure --with-libpcap="$PCAP_STAGE" --with-wpdpack="$BUILD/npcap-shim" \
    --with-openssl=/mingw64 --with-liblua=included --with-libpcre=included \
    --with-libz=included --with-liblinear=included --without-libssh2 \
    --without-ncat --without-nping --without-zenmap --without-ndiff \
    LIBS="-lpacket" LDFLAGS="-L$BUILD/npcap-shim/lib"
  printf '%s\n' "$STAMP_HASH" > "$BUILD/.veloterm-nmap-stamp"
else
  cd nmap-src
fi
make -j"$(nproc)"

# winfix.cc 不在 autotools 构建清单（上游只进 mswin32 的 MSVC 工程），
# 单独编译链入（含树内 VeloTerm 补丁：MinGW 优雅降级路径）；增量重跑时
# 环境里可能没有 CXXFLAGS，免得无优化编译
[ -n "$CXXFLAGS" ] || CXXFLAGS="-g -O2"
g++ -c $CXXFLAGS -DHAVE_CONFIG_H -I. -Imswin32 -I"$PCAP_STAGE/include" \
  mswin32/winfix.cc -o mswin32-winfix.o

# ── 4. runner nmap.exe：上游链接行 + winfix.o（位置在对象区、库之前），
#    同样 delayload——CI/无 Npcap 机器上 runner 才能启动跑冒烟。
#    链接行的改写用 sed 而非 bash 的花括号模式替换：本脚本是 JS String.raw
#    模板，花括号展开会被当成 JS 插值解析（qalc 段同一约束） ──
rm -f nmap
LINK=$(make -n nmap | grep -m1 -- '-o nmap')
LINK_ARGS=$(printf '%s\n' "$LINK" | sed 's/^g++ //')
RUNNER_ARGS=$(printf '%s\n' "$LINK_ARGS" | sed 's/ main\.o / main.o mswin32-winfix.o /')
eval g++ -Wl,--delayload,Packet.dll -ldelayimp $RUNNER_ARGS

# ── 5. script.db 预生成 + 数据文件 staging + 冒烟 ──
# --script-updatedb 会先过 win_init（降级路径）再扫 scripts/ 写 script.db
NMAPDIR="$PWD" ./nmap --script-updatedb
STAGE="$BUILD/nmap-data-stage"
rm -rf "$STAGE"
make install-nmap install-nse DESTDIR= bindir="$STAGE/bin" mandir="$STAGE/man" \
  nmapdatadir="$STAGE/nmap-data"
# 上游安装清单遗漏 nmap-payloads（运行期 payload.cc 会找的 UDP payload 库）
cp nmap-payloads "$STAGE/nmap-data/"

# 构建期冒烟（CI 无 Npcap：正好回归 delayload + winfix 降级路径；
# -n 免反向 DNS，-p 1 单端口 connect 扫描，退出码应为 0）
NMAPDIR="$STAGE/nmap-data" "$STAGE/bin/nmap" --version
NMAPDIR="$STAGE/nmap-data" "$STAGE/bin/nmap" -n -sT -p 1 -T5 --max-retries 1 127.0.0.1

# ── 6. nmapffi.dll：上游链接行去 main.o（veloterm_ffi.cc 以 #define main
#    改名包含 main.cc 替代）、加 winfix.o，全静态 + Packet.dll 延迟导入 ──
# 编译参数复用 main.o 的规则（含 configure 探明的 include 集与 NMAPDATADIR）
rm -f main.o
MAIN_CMD=$(make -n main.o | grep -m1 -- '-o main.o')
FFI_CMD=$(printf '%s\n' "$MAIN_CMD" | sed -e 's|-o main\.o|-o veloterm_ffi.o|' -e 's|main\.cc|veloterm_ffi.cc|g')
eval "$FFI_CMD -Dmain=veloterm_nmap_main_impl"
DLL_ARGS=$(printf '%s\n' "$LINK_ARGS" | sed -e "s|-o nmap|-o $BUILD/nmapffi.dll|" -e 's| main\.o | veloterm_ffi.o mswin32-winfix.o |')
# eval（而非直接展开）：链接行里的 -D...="..." 引号需重新按 shell 语义解析
# （与第 4 步 runner 同一处理；make -n 输出的是原始配方文本）
eval g++ -shared -static -static-libgcc -static-libstdc++ \
  -Wl,--delayload,Packet.dll -ldelayimp $DLL_ARGS

# 自包含断言（同 qalc；另容忍 Packet.dll——延迟导入也会被 objdump 列出。
# wpcap 不应出现：pcap_* 已静态编入；^lib 涵盖 libgcc/libstdc++/libssl 等
# MinGW 动态库命名）
DLLS=$(objdump -p "$BUILD/nmapffi.dll" | sed -n 's/\s*DLL Name: //p')
echo "$DLLS"
if echo "$DLLS" | grep -Eiq 'mingw|msys-|^lib|winpthread|lua|pcre|zlib1|wpcap'; then
  echo '[build-tools] nmapffi.dll 存在未静态化的 MinGW/MSYS 依赖（见上）' >&2
  exit 1
fi
`;
execFileSync(bashExe, ['-lc', nmapScript], { env: mingwEnv, stdio: 'inherit' });
cpSync(join(nmapBuild, 'nmapffi.dll'), join(outDir, 'nmapffi.dll'));
rmSync(join(outDir, 'nmap-data'), { recursive: true, force: true });
cpSync(join(nmapBuild, 'nmap-data-stage', 'nmap-data'), join(outDir, 'nmap-data'), {
  recursive: true,
});
console.log(`[build-tools] nmapffi.dll + nmap-data/ (${nmapVersion}) -> ${join(outDir)}`);

console.log('[build-tools] 完成');
