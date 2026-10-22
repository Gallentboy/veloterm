#!/usr/bin/env node
// obconnector-c（OceanBase libobclient）的 Windows 专用构建脚本——独立于
// build-tools.mjs（后者的尾段 nmap 无条件构建，与 obclient 无关，obclient
// 不经它跑）。只做一件事：vendor/obconnector-c 的 MSVC + CMake Release
// 构建（上游树全外参零侵入）+ 产物断言 + veloterm-win/ 冒烟工程静态链
// 接验证。车道入口：.github/workflows/test-obclient.yml（tag test_obclient）。
//
// 依赖：CMake/VS（runner 自带）+ vcpkg OpenSSL 静态库——OPENSSL_DIR 环境
// 变量指向 x64/arm64-windows-static 安装根（车道由 vcpkg 步骤安装，manifest
// 与 publish.yml 同源；本地请自行安装并导出）。
// macOS 不经本脚本：上游官方路径（deps/3rd + build.sh）已实测编通。
import { execSync } from 'node:child_process';
import { mkdirSync, existsSync, rmSync, readFileSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const srcTauri = join(root, 'src-tauri');
const obSrc = join(srcTauri, 'vendor', 'obconnector-c');

if (process.platform !== 'win32') {
  console.log(
    `[build-obclient] 非 Windows 主机（${process.platform}），跳过——macOS 走上游 build.sh 官方路径`,
  );
  process.exit(0);
}

const isArm64 = process.arch === 'arm64';
const obVersion = readFileSync(join(obSrc, 'rpm', 'libobclient-VER.txt'), 'utf8').trim();
console.log(
  `[build-obclient] obconnector-c ${obVersion}: MSVC + CMake（${isArm64 ? 'arm64' : 'x64'}，Release only）`,
);

// OpenSSL 静态库：vcpkg（车道步骤已在 triplet 文件写 release-only 再 install）
const opensslDir =
  process.env.OPENSSL_DIR ?? `C:\\vcpkg\\installed\\${isArm64 ? 'arm64' : 'x64'}-windows-static`;
if (!existsSync(join(opensslDir, 'lib', 'libcrypto.lib'))) {
  console.error(
    `[build-obclient] 未找到 OpenSSL 静态库：${opensslDir}\\lib\\libcrypto.lib 不存在。\n` +
      `  CI 上由 test-obclient.yml 的 vcpkg 步骤安装并导出 OPENSSL_DIR；本地请设 OPENSSL_DIR 指向 ${isArm64 ? 'arm64' : 'x64'}-windows-static 安装根。`,
  );
  process.exit(1);
}

const obBuild = join(srcTauri, 'obclient-build');
rmSync(obBuild, { recursive: true, force: true });
mkdirSync(obBuild, { recursive: true });

// CMake 4 移除 <3.5 兼容：上游非 APPLE 分支声明 cmake_minimum_required(2.8)
// 会硬报错，CMAKE_POLICY_VERSION_MINIMUM=3.5 是官方逃生口（CMake 3 下仅是
// 多余缓存变量，无害）。OpenSSL 三件套显式传路径（上游 build.sh 同型）；
// WITH_CURL=OFF 跳过 remote_io 插件（plugins/io 的 IF(WITH_CURL) 门）。
//
// env CFLAGS 注入 /DHAVE_BROKEN_INLINE：MSVC 的 C inline（ma_global.h 在
// _MSC_VER 下映射为 __inline）不发射外部符号，ob_serialize.c 的 40 个
// encode_*/decode_* inline 定义被其他编译单元引用即 LNK2019（CI 实测
// encode_i64/decode_i64）。HAVE_BROKEN_INLINE 是上游 ma_global.h 自带的
// 逃生开关（#undef inline / #define inline 空），经 env CFLAGS 注入会追加
// 在 CMake 平台 init 旗标之后，不覆盖 /DWIN32 等默认定义。
// CMake 旗标细节与产物说明见 vendor/obconnector-c/veloterm-win/README.md。
execSync(
  `cmake -S "${obSrc}" -B "${obBuild}" -A ${isArm64 ? 'ARM64' : 'x64'} ` +
    `-DCMAKE_POLICY_VERSION_MINIMUM=3.5 ` +
    `-DWITH_SSL=OPENSSL -DWITH_CURL=OFF ` +
    `-DOPENSSL_ROOT_DIR="${opensslDir}" ` +
    `-DOPENSSL_INCLUDE_DIR="${opensslDir}\\include" ` +
    `-DOPENSSL_SSL_LIBRARY="${opensslDir}\\lib\\libssl.lib" ` +
    `-DOPENSSL_CRYPTO_LIBRARY="${opensslDir}\\lib\\libcrypto.lib"`,
  { stdio: 'inherit', env: { ...process.env, CFLAGS: '/DHAVE_BROKEN_INLINE' } },
);
execSync(`cmake --build "${obBuild}" --config Release --parallel`, { stdio: 'inherit' });

// 产物断言：动态 libobclnt.dll + 导入库 libobclnt.lib（vcpkg OpenSSL 已链入
// DLL）+ 静态 obclnt.lib（仅连接器对象，OpenSSL 不在内——Unix 才走 .a 合并
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

// 冒烟：veloterm-win/ 工程静态链 obclnt.lib + vcpkg ssl/crypto 跑
// mysql_init/close 生命周期（WIN32 初始化路径含 WSAStartup，顺带验证
// ws2_32 接线）
const obSmokeBuild = join(obBuild, 'smoke');
execSync(
  `cmake -S "${join(obSrc, 'veloterm-win')}" -B "${obSmokeBuild}" ` +
    `-A ${isArm64 ? 'ARM64' : 'x64'} -DOBCLIENT_BUILD_DIR="${obBuild}" ` +
    `-DOPENSSL_ROOT_DIR="${opensslDir}"`,
  { stdio: 'inherit' },
);
execSync(`cmake --build "${obSmokeBuild}" --config Release`, { stdio: 'inherit' });
execSync(`"${join(obSmokeBuild, 'Release', 'obclient-smoke.exe')}"`, { stdio: 'inherit' });

console.log(
  `[build-obclient] 产物 (${obVersion}) 留在 ${obBuild}（未接线 app 消费，不拷 binaries/）`,
);
console.log('[build-obclient] 完成');
