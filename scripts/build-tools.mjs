#!/usr/bin/env node
// 构建 coreutils 多呼叫二进制并复制到 Tauri externalBin 目录（仅 Windows 需要），
// 同时产出随包 resources 分发的两个 FFI DLL：yqffi.dll（Go）与
// qalcffi.dll（libqalculate，MSYS2 MinGW64 构建）。
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
execSync(
  `cargo build --release ${cargoTargetArgs}-p ${TOOL} --bin ${TOOL}`,
  { cwd: srcTauri, stdio: 'inherit' },
);

// 清空并重建 externalBin 目录
rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });

// cargo 未传 --target 时输出到 target/release（无 triple 子目录）；
// 传了 --target 时输出到 target/<triple>/release，与上方构建参数对应取用；
// 本地未传时兼容两处都存在的历史产物
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

// ── yq（Go）：coreutils 的 yq 命令经 yqffi.dll 动态导入 ────────────────────
// Go 官方不支持 MSVC 链接 c-archive（golang/go#74189），静态链入 coreutils.exe
// 不可能；buildmode=c-shared 的 DLL 是唯一形态（分发臂见 crates/coreutils
// 的 yq_impl.rs，运行时按 exe 同目录/上级目录定位）。产物为普通文件名
// yqffi.dll，经 tauri.windows.conf.json 的 resources 随包放在安装根目录。
// 版本锁在 vendor/yq/VELOTERM_VERSION（与 openssl-src 的双源纪律一致），
// 通过 ldflags 注入 cmd.GitDescribe 让 `yq --version` 打出真实版本。
const yqSrc = join(srcTauri, 'vendor', 'yq');
const yqVersion = readFileSync(join(yqSrc, 'VELOTERM_VERSION'), 'utf8').trim();
console.log(`[build-tools] yq ${yqVersion}: go build -buildmode=c-shared`);
try {
  execSync('go version', { stdio: 'ignore' });
} catch {
  console.error('[build-tools] 未找到 go 工具链（yqffi.dll 构建必需）：请安装 Go ≥ 1.24 或将其加入 PATH');
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
    '[build-tools] 未找到 gcc（cgo 编译 yqffi.dll 必需）：windows-latest 通常自带 MinGW，' +
      '若镜像变化请安装 MinGW-w64 或把其 bin 目录加入 PATH',
  );
  process.exit(1);
}
execSync(
  `go build -buildmode=c-shared -tags veloterm_ffi -trimpath ` +
    `-ldflags "-s -w -X github.com/mikefarah/yq/v4/cmd.GitDescribe=${yqVersion}" ` +
    `-o "${join(outDir, 'yqffi.dll')}" .`,
  { cwd: yqSrc, stdio: 'inherit', env: { ...process.env, CGO_ENABLED: '1' } },
);
console.log(`[build-tools] yqffi.dll (${yqVersion}) -> ${join(outDir, 'yqffi.dll')}`);

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
    '[build-tools] 未找到 MSYS2（qalcffi.dll 构建必需，需 usr/bin/bash.exe）：' +
      '请安装 MSYS2（默认 C:\\msys64）或设 MSYS2_ROOT 指向其根目录',
  );
  process.exit(1);
}
const bashExe = join(msysRoot, 'usr', 'bin', 'bash.exe');
// MSYSTEM=MINGW64 + 登录 shell：/etc/profile 按该变量前置 /mingw64/bin，
// bash 内的 g++/make/pkg-config 即 x86_64-w64-mingw32 工具链
const mingwEnv = { ...process.env, MSYSTEM: 'MINGW64' };

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
# 增量构建：Makefile 已存在（上次构建残留）则跳过 configure
if [ ! -f Makefile ]; then
  sh "$SRC/configure" --disable-shared --enable-static --enable-compiled-definitions \
    --without-libcurl --without-icu --disable-nls --disable-unittests
fi
make -j"$(nproc)"
# qalc 入口改名编译：-Dmain=qalc_main（仅本翻译单元），编译参数对齐
# src/Makefile.am 的 AM_CPPFLAGS（config.h 在 builddir 根，support.h 经
# HAVE_CONFIG_H 引入）
g++ -c -O2 -DHAVE_CONFIG_H -Dmain=qalc_main \
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

console.log('[build-tools] 完成');
