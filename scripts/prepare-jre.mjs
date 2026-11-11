#!/usr/bin/env node
/**
 * 自动为当前打包目标准备裁剪版 JRE 25 (Vineflower 运行环境)
 *
 * 目标映射：
 *  - x86_64-pc-windows-msvc  -> vineflower-runtime-windows-x64.zip
 *  - aarch64-pc-windows-msvc -> vineflower-runtime-windows-arm64.zip
 *  - aarch64-apple-darwin    -> vineflower-runtime-macos-aarch64.tar.gz
 *  - x86_64-apple-darwin     -> vineflower-runtime-macos-x64.tar.gz
 *
 * 用法：
 *  node scripts/prepare-jre.mjs [--target <rust-target>] [--tag <release-tag>] [--force]
 */

import { existsSync, mkdirSync, rmSync, renameSync, chmodSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { execSync } from 'node:child_process';
import { Readable } from 'node:stream';
import { finished } from 'node:stream/promises';
import { createWriteStream } from 'node:fs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const jreDestDir = join(root, 'src-tauri/resources/jre');
const cacheDir = join(root, 'src-tauri/.jre-cache');

// 解析参数
const args = process.argv.slice(2);
let target = '';
let tag = process.env.VELOTERM_RUNTIME_TAG || 'test_runtime';
let force = false;

for (let i = 0; i < args.length; i++) {
  if (args[i] === '--target' && args[i + 1]) {
    target = args[++i];
  } else if (args[i] === '--tag' && args[i + 1]) {
    tag = args[++i];
  } else if (args[i] === '--force') {
    force = true;
  }
}

// 若未指定 target，尝试推断当前主机原生 target
if (!target) {
  const arch = process.arch;
  const platform = process.platform;
  if (platform === 'win32') {
    target = arch === 'arm64' ? 'aarch64-pc-windows-msvc' : 'x86_64-pc-windows-msvc';
  } else if (platform === 'darwin') {
    target = arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin';
  } else {
    console.error(`[prepare-jre] 不支持的宿主平台: ${platform}`);
    process.exit(1);
  }
}

const TARGET_MAP = {
  'x86_64-pc-windows-msvc': {
    asset: 'vineflower-runtime-windows-x64.zip',
    isZip: true,
    exeName: 'java.exe',
  },
  'aarch64-pc-windows-msvc': {
    asset: 'vineflower-runtime-windows-arm64.zip',
    isZip: true,
    exeName: 'java.exe',
  },
  'aarch64-apple-darwin': {
    asset: 'vineflower-runtime-macos-aarch64.tar.gz',
    isZip: false,
    exeName: 'java',
  },
  'x86_64-apple-darwin': {
    asset: 'vineflower-runtime-macos-x64.tar.gz',
    isZip: false,
    exeName: 'java',
  },
};

const targetConfig = TARGET_MAP[target];
if (!targetConfig) {
  console.error(`[prepare-jre] 未知的 Rust target: ${target}`);
  process.exit(1);
}

const javaExePath = join(jreDestDir, 'bin', targetConfig.exeName);

// 检查是否已准备完毕
if (!force && existsSync(javaExePath)) {
  console.log(`[prepare-jre] 目标 JRE 已就绪: ${javaExePath}`);
  try {
    const versionOutput = execSync(`"${javaExePath}" -version 2>&1`, { encoding: 'utf8' });
    console.log(`[prepare-jre] 运行环境验证成功:\n${versionOutput.trim()}`);
    process.exit(0);
  } catch (err) {
    console.warn(`[prepare-jre] 既有 JRE 无法执行，重新拉取准备: ${err.message}`);
  }
}

mkdirSync(cacheDir, { recursive: true });
const cachedAssetPath = join(cacheDir, targetConfig.asset);

console.log(`[prepare-jre] 正在为 target [${target}] 准备 JRE...`);
console.log(`[prepare-jre] 目标资产: ${targetConfig.asset} (Tag: ${tag})`);

// 1. 获取资产文件（优先本地缓存，若无则拉取）
if (!existsSync(cachedAssetPath) || force) {
  let downloaded = false;

  // 方式 A：尝试使用 gh release download
  try {
    console.log(`[prepare-jre] 尝试通过 GitHub CLI 下载...`);
    execSync(`gh release download "${tag}" -p "${targetConfig.asset}" -D "${cacheDir}" --clobber`, {
      stdio: 'inherit',
    });
    if (existsSync(cachedAssetPath)) {
      downloaded = true;
      console.log(`[prepare-jre] gh release download 成功`);
    }
  } catch {
    console.log(`[prepare-jre] gh 命令不可用或下载失败，回退到 HTTP 直链下载...`);
  }

  // 方式 B：回退到 HTTPS 直接下载
  if (!downloaded) {
    const downloadUrl = `https://github.com/Gallentboy/veloterm/releases/download/${tag}/${targetConfig.asset}`;
    console.log(`[prepare-jre] 正在从 ${downloadUrl} 下载...`);

    const resp = await fetch(downloadUrl, {
      headers: process.env.GITHUB_TOKEN ? { Authorization: `Bearer ${process.env.GITHUB_TOKEN}` } : {},
      redirect: 'follow',
    });

    if (!resp.ok) {
      throw new Error(`下载 JRE 资产失败: ${resp.status} ${resp.statusText} (${downloadUrl})`);
    }

    const fileStream = createWriteStream(cachedAssetPath);
    await finished(Readable.fromWeb(resp.body).pipe(fileStream));
    console.log(`[prepare-jre] HTTP 直链下载完成`);
  }
} else {
  console.log(`[prepare-jre] 命中本地缓存: ${cachedAssetPath}`);
}

// 2. 解压到临时目录
const tempExtractDir = join(cacheDir, `extract_${Date.now()}`);
mkdirSync(tempExtractDir, { recursive: true });

try {
  console.log(`[prepare-jre] 正在解压资产...`);
  if (targetConfig.isZip) {
    if (process.platform === 'win32') {
      execSync(`powershell -NoProfile -Command "Expand-Archive -Path '${cachedAssetPath}' -DestinationPath '${tempExtractDir}' -Force"`, {
        stdio: 'inherit',
      });
    } else {
      execSync(`unzip -q "${cachedAssetPath}" -d "${tempExtractDir}"`, { stdio: 'inherit' });
    }
  } else {
    execSync(`tar -xzf "${cachedAssetPath}" -C "${tempExtractDir}"`, { stdio: 'inherit' });
  }

  // 查找解压出的 vineflower-runtime 目录
  let runtimeSrc = join(tempExtractDir, 'vineflower-runtime');
  if (!existsSync(runtimeSrc)) {
    // 兼容平铺解压情况
    if (existsSync(join(tempExtractDir, 'bin'))) {
      runtimeSrc = tempExtractDir;
    } else {
      throw new Error(`解压目录中未找到 vineflower-runtime 或 bin 目录`);
    }
  }

  // 3. 安全更新目标目录 src-tauri/resources/jre
  console.log(`[prepare-jre] 部署到目标目录: ${jreDestDir}`);
  if (existsSync(jreDestDir)) {
    rmSync(jreDestDir, { recursive: true, force: true });
  }
  mkdirSync(dirname(jreDestDir), { recursive: true });
  renameSync(runtimeSrc, jreDestDir);

  // 清理非运行必需且包含只读权限文件的 legal 目录，避免打包时权限冲突
  const legalDir = join(jreDestDir, 'legal');
  if (existsSync(legalDir)) {
    rmSync(legalDir, { recursive: true, force: true });
  }

  // 4. macOS / Linux 赋予可读写与可执行权限
  if (!targetConfig.isZip) {
    try {
      execSync(`chmod -R u+w "${jreDestDir}"`, { stdio: 'ignore' });
      if (process.platform === 'darwin') {
        execSync(`xattr -cr "${jreDestDir}" 2>/dev/null || true`, { stdio: 'ignore' });
      }
    } catch {}

    const javaBin = join(jreDestDir, 'bin', targetConfig.exeName);
    try {
      chmodSync(javaBin, 0o755);
      console.log(`[prepare-jre] 已设置可执行权限 0755: ${javaBin}`);
    } catch (e) {
      console.warn(`[prepare-jre] 设置可执行权限警告: ${e.message}`);
    }
  }

  // 5. 冒烟测试
  if (existsSync(javaExePath)) {
    // 如果宿主与目标架构一致，执行冒烟测试
    const hostMatchesTarget =
      (process.platform === 'darwin' && target.includes('apple-darwin') && process.arch === (target.includes('aarch64') ? 'arm64' : 'x64')) ||
      (process.platform === 'win32' && target.includes('windows-msvc') && process.arch === (target.includes('aarch64') ? 'arm64' : 'x64'));

    if (hostMatchesTarget) {
      const out = execSync(`"${javaExePath}" -version 2>&1`, { encoding: 'utf8' });
      console.log(`[prepare-jre] 裁剪版 JRE 冒烟验证通过:\n${out.trim()}`);
    } else {
      console.log(`[prepare-jre] 目标为交叉/远端平台架构，跳过本机直接执行验证`);
    }
  } else {
    throw new Error(`部署完成后未在 ${javaExePath} 找到 Java 可执行文件`);
  }

  console.log(`[prepare-jre] ✅ 裁剪版 JRE 准备完毕！`);
} finally {
  // 清理临时解压目录
  try {
    if (existsSync(tempExtractDir)) {
      rmSync(tempExtractDir, { recursive: true, force: true });
    }
  } catch {}
}
