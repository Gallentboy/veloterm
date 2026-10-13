#!/usr/bin/env node
// 版本号同步：package.json 是唯一版本源，同步到 src-tauri/Cargo.toml。
// tauri.conf.json 不写 version 字段，Tauri 会自动回退到 Cargo.toml 的包版本，
// 因此只需维护 package.json 一处（用 `pnpm version patch/minor/major` 提升）。
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const pkgPath = join(root, 'package.json');
const cargoPath = join(root, 'src-tauri/Cargo.toml');

const pkg = JSON.parse(readFileSync(pkgPath, 'utf8'));
const version = pkg.version;
if (!/^\d+\.\d+\.\d+(-[\w.]+)?$/.test(version)) {
  console.error(`[sync-version] package.json version 非法: ${version}`);
  process.exit(1);
}

const cargo = readFileSync(cargoPath, 'utf8');
const next = cargo.replace(/^version\s*=\s*"[^"]*"/m, `version = "${version}"`);
if (next === cargo && !new RegExp(`^version\\s*=\\s*"${version}"`, 'm').test(cargo)) {
  console.error('[sync-version] Cargo.toml 中未找到 [package] version 字段');
  process.exit(1);
}
if (next !== cargo) {
  writeFileSync(cargoPath, next);
  console.log(`[sync-version] Cargo.toml -> ${version}`);
} else {
  console.log(`[sync-version] 已是 ${version}，无需同步`);
}
