#!/usr/bin/env node
// 构建 coreutils 多呼叫二进制并复制到 Tauri externalBin 目录。
// Tauri v2 的 externalBin 要求文件名带 target triple 后缀：
//   src-tauri/binaries/coreutils-x86_64-pc-windows-msvc.exe
// 安装后 Tauri 会去掉 triple 后缀放到主程序同级目录（coreutils.exe），
// 短名命令入口（ls.exe/grep.exe/…）由应用启动钩子以 NTFS 硬链接创建。
// macOS（通用开发/打包）同样按本机 triple 复制，保持配置跨平台一致。
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

// 编译 coreutils 多呼叫二进制（固定 release 供打包）
execSync(`cargo build --release --bin ${TOOL}`, { cwd: srcTauri, stdio: 'inherit' });

// 清空并重建 externalBin 目录
rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });

// cargo 未传 --target 时输出到 target/release（无 triple 子目录）；
// 传了 --target 时输出到 target/<triple>/release。
// 本脚本不传 --target 编译，但复制时兼容两种目录形态。
const exeExt = target.includes('windows') ? '.exe' : '';
const plainOut = join(srcTauri, 'target', 'release');
const tripleOut = join(srcTauri, 'target', target, 'release');
const cargoOut = existsSync(join(tripleOut, TOOL + exeExt))
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
