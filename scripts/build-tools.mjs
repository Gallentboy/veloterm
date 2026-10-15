#!/usr/bin/env node
// 构建 coreutils 多呼叫二进制并复制到 Tauri externalBin 目录（仅 Windows 需要）。
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

import { execSync } from 'node:child_process';
import { cpSync, mkdirSync, existsSync, rmSync } from 'node:fs';
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
console.log('[build-tools] 完成');
