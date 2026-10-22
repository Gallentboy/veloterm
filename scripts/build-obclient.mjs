#!/usr/bin/env node
// obconnector-c（OceanBase libobclient）的 mac/win 四平台静态库构建脚本
// ——独立于 build-tools.mjs（后者的尾段 nmap 无条件构建，与 obclient
// 无关）。产四平台的 obclnt-sys 链接档案（vendor/obclient-lib/<平台>/）：
//   mac-aarch64 / mac-x86_64 → libobclnt.a（自包含：CMake 的 .a 合并分支
//     并入 zlib + OpenSSL——brew openssl@3 的静态档案）
//   win-x64 / win-arm64      → obclnt.lib（仅连接器对象，OpenSSL 不在内，
//     消费方经 OPENSSL_DIR 补链 vcpkg 静态 ssl/crypto）
// 车道入口：.github/workflows/test-obclient.yml（tag test_obclient，四腿
// 矩阵：windows-latest / windows-11-arm / macos-26 / macos-26-intel——
// 全原生 runner，无交叉编译）。
//
// OpenSSL 来源（两平台同代 3.x，ABI 兼容；补丁位不强求一致——win 侧
// vcpkg manifest 锁 3.6.4#1，mac 侧 brew openssl@3 公式跟上游发布走，
// 当前 3.5.9，无 3.6.4 配方）：
//   - Windows：vcpkg 静态库——OPENSSL_DIR 环境变量指向
//     x64/arm64-windows-static 安装根（车道 vcpkg 步骤安装，manifest 与
//     publish.yml 同源；本地请自行安装并导出）
//   - macOS：brew openssl@3（keg 自带 libssl.a/libcrypto.a 静态档案——
//     这是合并分支的硬前提；dylib 路径不会触发合并）。前缀两代 brew 不同
//     （ARM /opt/homebrew、Intel /usr/local），经 `brew --prefix openssl@3`
//     动态解析；OPENSSL_DIR 环境变量可覆盖
// CMake 旗标细节与产物说明见 vendor/obconnector-c/veloterm-win/README.md。
import { execSync } from 'node:child_process';
import { mkdirSync, existsSync, rmSync, readFileSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const srcTauri = join(root, 'src-tauri');
const obSrc = join(srcTauri, 'vendor', 'obconnector-c');

const isMac = process.platform === 'darwin';
const isWin = process.platform === 'win32';
if (!isMac && !isWin) {
  console.log(
    `[build-obclient] 未支持的主机（${process.platform}），跳过——仅 mac/win 四平台`,
  );
  process.exit(0);
}

const isArm64 = process.arch === 'arm64';
const platformName = isMac
  ? `mac-${isArm64 ? 'aarch64' : 'x86_64'}`
  : `win-${isArm64 ? 'arm64' : 'x64'}`;
const obVersion = readFileSync(join(obSrc, 'rpm', 'libobclient-VER.txt'), 'utf8').trim();
console.log(`[build-obclient] obconnector-c ${obVersion}: ${isMac ? 'clang' : 'MSVC'} + CMake（${platformName}，Release only）`);

// ── OpenSSL 静态库定位 ────────────────────────────────────────────────────
let opensslDir;
if (isWin) {
  opensslDir =
    process.env.OPENSSL_DIR ?? `C:\\vcpkg\\installed\\${isArm64 ? 'arm64' : 'x64'}-windows-static`;
} else {
  opensslDir =
    process.env.OPENSSL_DIR ??
    execSync('brew --prefix openssl@3', { encoding: 'utf8' }).trim();
  if (!existsSync(opensslDir)) {
    console.error(
      `[build-obclient] brew openssl@3 未安装（brew --prefix 解析到 ${opensslDir} 但目录不存在）。请先 brew install openssl@3，或用 OPENSSL_DIR 指向带静态 .a 的 OpenSSL 安装根。`,
    );
    process.exit(1);
  }
}
const libExt = isWin ? '.lib' : '.a';
const sslLibName = `libssl${libExt}`;
const cryptoLibName = `libcrypto${libExt}`;
for (const f of [sslLibName, cryptoLibName]) {
  if (!existsSync(join(opensslDir, 'lib', f))) {
    console.error(
      `[build-obclient] 未找到 OpenSSL 静态库：${join(opensslDir, 'lib', f)} 不存在。\n` +
        (isWin
          ? `  CI 上由 test-obclient.yml 的 vcpkg 步骤安装并导出 OPENSSL_DIR；本地请设 OPENSSL_DIR 指向 ${isArm64 ? 'arm64' : 'x64'}-windows-static 安装根。`
          : `  brew openssl@3 的 keg 应自带静态 .a；若用 OPENSSL_DIR 覆盖，目录须含 lib/${sslLibName} 与 lib/${cryptoLibName}（.a 是 mac 合并分支的硬前提）。`),
    );
    process.exit(1);
  }
}

const obBuild = join(srcTauri, 'obclient-build');
rmSync(obBuild, { recursive: true, force: true });
mkdirSync(obBuild, { recursive: true });

// CMake 4 移除 <3.5 兼容：上游非 APPLE 分支声明 cmake_minimum_required(2.8)
// 会硬报错，CMAKE_POLICY_VERSION_MINIMUM=3.5 是官方逃生口（CMake 3 下仅是
// 多余缓存变量，无害；mac 的 APPLE 分支本就是 3.5 下限，旗标同样无害）。
// OpenSSL 三件套显式传路径（上游 build.sh 同型，绕开 FindOpenSSL 探测不
// 确定性）；WITH_CURL=OFF 跳过 remote_io 插件（plugins/io 的 IF(WITH_CURL)
// 门）。
//
// env CFLAGS 注入 /DHAVE_BROKEN_INLINE（仅 Windows 需要；mac 的 clang 按
// C99 inline 语义发射外部定义）：MSVC 的 C inline（ma_global.h 在 _MSC_VER
// 下映射为 __inline）不发射外部符号，ob_serialize.c 的 40 个 encode_*/
// decode_* inline 定义被其他编译单元引用即 LNK2019（CI 实测
// encode_i64/decode_i64）。HAVE_BROKEN_INLINE 是上游 ma_global.h 自带的
// 逃生开关（#undef inline / #define inline 空）。经 env CFLAGS 注入会追加
// 在 CMake 平台 init 旗标之后，不覆盖 /DWIN32 等默认定义。
const configureEnv = isWin
  ? { ...process.env, CFLAGS: '/DHAVE_BROKEN_INLINE' }
  : { ...process.env };
execSync(
  `cmake -S "${obSrc}" -B "${obBuild}" ${isWin ? `-A ${isArm64 ? 'ARM64' : 'x64'} ` : '-DCMAKE_BUILD_TYPE=Release '}` +
    `-DCMAKE_POLICY_VERSION_MINIMUM=3.5 ` +
    `-DWITH_SSL=OPENSSL -DWITH_CURL=OFF ` +
    `-DOPENSSL_ROOT_DIR="${opensslDir}" ` +
    `-DOPENSSL_INCLUDE_DIR="${join(opensslDir, 'include')}" ` +
    `-DOPENSSL_SSL_LIBRARY="${join(opensslDir, 'lib', sslLibName)}" ` +
    `-DOPENSSL_CRYPTO_LIBRARY="${join(opensslDir, 'lib', cryptoLibName)}"`,
  { stdio: 'inherit', env: configureEnv },
);
execSync(
  `cmake --build "${obBuild}" ${isWin ? '--config Release' : ''} --parallel`,
  { stdio: 'inherit' },
);

// ── 产物断言 ─────────────────────────────────────────────────────────────
if (isWin) {
  // 动态 libobclnt.dll + 导入库 libobclnt.lib（vcpkg OpenSSL 已链入 DLL）+
  // 静态 obclnt.lib（仅连接器对象，OpenSSL 不在内——Unix 才走 .a 合并
  // 分支，冒烟工程自带）。VS 多配置生成器默认输出到 libmariadb/Release/。
  const obLibDir = join(obBuild, 'libmariadb', 'Release');
  for (const f of ['libobclnt.dll', 'libobclnt.lib', 'obclnt.lib']) {
    if (!existsSync(join(obLibDir, f))) {
      console.error(`[build-obclient] 缺少构建产物: ${join(obLibDir, f)}`);
      const probe = join(obBuild, 'libmariadb');
      if (existsSync(probe)) {
        console.error(`  libmariadb/ 实际内容：${readdirSync(probe).join(', ')}`);
      }
      process.exit(1);
    }
  }
} else {
  // mac 的合并静态库在 archive_output_directory/（MERGE_CONVENIENCE_
  // LIBRARIES2 的 ARCHIVE_OUTPUT_DIRECTORY，libutils.cmake）——自包含
  // （OpenSSL/zlib 并入），消费方零外部依赖。
  const macArchive = join(obBuild, 'archive_output_directory', 'libobclnt.a');
  if (!existsSync(macArchive)) {
    console.error(`[build-obclient] 缺少构建产物: ${macArchive}`);
    const probe = join(obBuild, 'archive_output_directory');
    if (existsSync(probe)) {
      console.error(`  archive_output_directory/ 实际内容：${readdirSync(probe).join(', ')}`);
    }
    process.exit(1);
  }
}

// ── 冒烟：验证产物可链可跑 ───────────────────────────────────────────────
if (isWin) {
  // veloterm-win/ 工程静态链 obclnt.lib + vcpkg ssl/crypto 跑
  // mysql_init/close 生命周期（WIN32 初始化路径含 WSAStartup，顺带验证
  // ws2_32/bcrypt 接线）
  const obSmokeBuild = join(obBuild, 'smoke');
  execSync(
    `cmake -S "${join(obSrc, 'veloterm-win')}" -B "${obSmokeBuild}" ` +
      `-A ${isArm64 ? 'ARM64' : 'x64'} -DOBCLIENT_BUILD_DIR="${obBuild}" ` +
      `-DOPENSSL_ROOT_DIR="${opensslDir}"`,
    { stdio: 'inherit' },
  );
  execSync(`cmake --build "${obSmokeBuild}" --config Release`, { stdio: 'inherit' });
  execSync(`"${join(obSmokeBuild, 'Release', 'obclient-smoke.exe')}"`, { stdio: 'inherit' });
} else {
  // mac：合并 .a 直链即可（自包含，本机实测无需额外框架/库）——veloterm-win/
  // 的 smoke.c 是纯 C 的引擎臂子集冒烟，两平台共用同一源码
  const smokeBin = join(obBuild, 'obclient-smoke');
  execSync(
    `cc "${join(obSrc, 'veloterm-win', 'smoke.c')}" ` +
      `-I"${join(obSrc, 'include')}" -I"${join(obBuild, 'include')}" ` +
      `"${join(obBuild, 'archive_output_directory', 'libobclnt.a')}" -o "${smokeBin}"`,
    { stdio: 'inherit' },
  );
  execSync(`"${smokeBin}"`, { stdio: 'inherit' });
}

console.log(`[build-obclient] ${platformName} 产物 (${obVersion}) 留在 ${obBuild}（未接线 app 消费，不拷 binaries/）`);
console.log('[build-obclient] 完成');
