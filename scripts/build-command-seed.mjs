// build-command-seed.mjs -- 合并命令库种子分片并强校验，产出最终播种 JSON。
//
// 输入：src-tauri/seed-parts/part1.json … part5.json（内容分片，人工/agent 产出）
// 输出：src-tauri/assets/builtin-commands-seed.json（Rust include_str! 播种用）
//
// 职责（本脚本是内容质量的机器门禁，产出的 JSON 才能进二进制）：
// 1. schema 强校验：字段齐全、category/platforms/dangerLevel 取值合法、
//    description 达到「详细」下限（>=100 字符）、source 必须是 https 官方文档链接；
// 2. 占位符契约：command 中每个 {{name}} 必须有同名 param，反之亦然；
// 3. 与 tools-catalog 核对：Windows 全命令集（跨平台 102 + 自研 9 + Windows 专属）
//    每个命令必须有 >=1 条分片条目（按 tool 字段匹配），缺一即失败；
// 4. uuid 确定性生成（uuid v5，namespace + `${tool}#${序号}`）——重新生成结果稳定，
//    种子数据升级时同命令 uuid 不变，用户库判重/差量插入语义才成立；
// 5. 排序固化（tool → title）+ 固定 createdAt + isPinned=false。
//
// 用法：node scripts/build-command-seed.mjs（仓库根执行）

import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const partsDir = join(root, 'src-tauri', 'seed-parts');
const outFile = join(root, 'src-tauri', 'assets', 'builtin-commands-seed.json');
const catalogFile = join(root, 'src-tauri', 'crates', 'tools-catalog', 'src', 'lib.rs');

const CATEGORIES = new Set([
  '', 'file', 'text', 'archive', 'package', 'permission', 'shell', 'schedule',
  'network', 'disk', 'process', 'system', 'docker', 'gpu', 'ssh', 'other',
]);
const PLATFORMS = new Set(['win', 'linux', 'mac']);
const DANGERS = new Set(['safe', 'confirm', 'destructive']);
const MIN_DESC_LEN = 100;

// uuid v5（SHA-1 版）：与 RFC 4122 一致的位布局，node 无内置故手写
const NAMESPACE = '9e1f2a34-7c5d-4a1b-8f2e-5eedc0a10001';
function uuidV5(name) {
  const hexToBytes = (s) => s.replace(/-/g, '').match(/../g).map((b) => parseInt(b, 16));
  const hash = createHash('sha1')
    .update(Buffer.from(hexToBytes(NAMESPACE)))
    .update(Buffer.from(name, 'utf8'))
    .digest();
  hash[6] = (hash[6] & 0x0f) | 0x50; // version 5
  hash[8] = (hash[8] & 0x3f) | 0x80; // variant
  const hex = hash.subarray(0, 16).toString('hex');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/** 从 tools-catalog/lib.rs 提取 Windows 全命令集（跨平台 + EXTRA + windows 专属） */
function windowsCommandSet() {
  const src = readFileSync(catalogFile, 'utf8');
  const extract = (anchor) => {
    const idx = src.indexOf(anchor);
    if (idx === -1) throw new Error(`catalog anchor not found: ${anchor}`);
    const start = src.indexOf('&[', idx);
    const end = src.indexOf('];', start);
    const body = src.slice(start, end);
    return [...body.matchAll(/"([a-z0-9]+)"/g)].map((m) => m[1]);
  };
  const winPlatformIdx = src.indexOf('#[cfg(target_os = "windows")]');
  const winSection = src.slice(winPlatformIdx);
  const cross = extract('pub const CROSS_PLATFORM_COMMANDS');
  const extra = extract('pub const EXTRA_COMMANDS');
  const winPlatStart = winSection.indexOf('pub const PLATFORM_COMMANDS');
  const ws = winSection.indexOf('&[', winPlatStart);
  const we = winSection.indexOf('];', ws);
  const winOnly = [...winSection.slice(ws, we).matchAll(/"([a-z0-9]+)"/g)].map((m) => m[1]);
  return new Set([...cross, ...extra, ...winOnly]);
}

const errors = [];
const entries = [];

for (const part of ['part1', 'part2', 'part3', 'part4', 'part5']) {
  const path = join(partsDir, `${part}.json`);
  let arr;
  try {
    arr = JSON.parse(readFileSync(path, 'utf8'));
  } catch (e) {
    errors.push(`${part}: 读取/解析失败：${e.message}`);
    continue;
  }
  if (!Array.isArray(arr) || arr.length === 0) {
    errors.push(`${part}: 不是非空数组`);
    continue;
  }
  for (const [i, e] of arr.entries()) {
    const at = `${part}[${i}]`;
    const str = (k) => (typeof e[k] === 'string' ? e[k].trim() : '');
    if (!str('tool')) errors.push(`${at}: 缺 tool`);
    if (!str('title')) errors.push(`${at}: 缺 title`);
    if (!str('command')) errors.push(`${at}: 缺 command`);
    if (str('description').length < MIN_DESC_LEN) {
      errors.push(`${at}: description 过短（${str('description').length} < ${MIN_DESC_LEN}，要求详细说明）`);
    }
    if (!str('purpose')) errors.push(`${at}: 缺 purpose`);
    if (!CATEGORIES.has(str('category'))) errors.push(`${at}: category 非法：${e.category}`);
    if (!DANGERS.has(str('dangerLevel'))) errors.push(`${at}: dangerLevel 非法：${e.dangerLevel}`);
    if (!/^https:\/\//.test(str('source'))) errors.push(`${at}: source 必须是 https 官方文档链接`);
    if (!Array.isArray(e.platforms) || !e.platforms.length ||
        e.platforms.some((p) => !PLATFORMS.has(p))) {
      errors.push(`${at}: platforms 非法：${JSON.stringify(e.platforms)}`);
    }
    if (!Array.isArray(e.tags) || e.tags.length === 0 || e.tags.some((t) => typeof t !== 'string')) {
      errors.push(`${at}: tags 须为非空字符串数组`);
    }
    // 占位符契约
    const ph = [...new Set([...str('command').matchAll(/\{\{([^}]+)\}\}/g)].map((m) => m[1].trim()))];
    const params = Array.isArray(e.params) ? e.params : [];
    const paramNames = params.map((p) => (typeof p?.name === 'string' ? p.name.trim() : ''));
    for (const p of ph) {
      if (!paramNames.includes(p)) errors.push(`${at}: 占位符 {{${p}}} 无对应 param`);
    }
    for (const n of paramNames) {
      if (n && !ph.includes(n)) errors.push(`${at}: param "${n}" 未出现在 command 中`);
    }
    for (const [j, p] of params.entries()) {
      const pat = `${at}.params[${j}]`;
      if (!p?.name || !p?.label) errors.push(`${pat}: 缺 name/label`);
      for (const k of ['description', 'default', 'example']) {
        if (typeof p?.[k] !== 'string') errors.push(`${pat}: ${k} 缺失或非字符串`);
      }
      if (typeof p?.required !== 'boolean') errors.push(`${pat}: required 非布尔`);
    }
    if (typeof e.exampleOutput !== 'string') errors.push(`${at}: exampleOutput 缺失或非字符串`);
    entries.push(e);
  }
}

// 命令库全集核对：part1-4 覆盖 tools-catalog 的 Windows 全集
const catalogSet = windowsCommandSet();
const covered = new Set(entries.filter((e) => e.tool).map((e) => e.tool));
const missing = [...catalogSet].filter((c) => !covered.has(c)).sort();
if (missing.length) {
  errors.push(`tools-catalog 命令未被分片覆盖（${missing.length} 个）：${missing.join(', ')}`);
}
// title+command 全局去重
const seen = new Map();
for (const e of entries) {
  const k = `${e.title}|${e.command}`;
  if (seen.has(k)) errors.push(`重复条目：${k}（${seen.get(k)} 与后续分片）`);
  seen.set(k, 'dup');
}

if (errors.length) {
  console.error(`[build-command-seed] 校验失败 ${errors.length} 处：`);
  for (const err of errors) console.error(`  - ${err}`);
  process.exit(1);
}

// 排序固化 + 确定性 uuid + 固定 createdAt
entries.sort((a, b) => (a.tool + a.title).localeCompare(b.tool + b.title, 'en'));
const ordinal = new Map();
const finalEntries = entries.map((e) => {
  const n = (ordinal.get(e.tool) ?? 0) + 1;
  ordinal.set(e.tool, n);
  return {
    uuid: uuidV5(`${e.tool}#${n}`),
    title: e.title.trim(),
    command: e.command.trim(),
    description: e.description.trim(),
    purpose: e.purpose.trim(),
    category: e.category.trim(),
    tags: e.tags,
    platforms: e.platforms,
    params: e.params.map((p) => ({
      name: p.name.trim(),
      label: p.label.trim(),
      description: p.description ?? '',
      required: !!p.required,
      default: p.default ?? '',
      example: p.example ?? '',
    })),
    dangerLevel: e.dangerLevel,
    exampleOutput: e.exampleOutput ?? '',
    isPinned: false,
    createdAt: 1782460800_000, // 2026-06-26T00:00:00Z 固定值（种子数据无本机时间语义）
    source: e.source.trim(),
  };
});

mkdirSync(dirname(outFile), { recursive: true });
writeFileSync(outFile, JSON.stringify(finalEntries, null, 2) + '\n');
const byDanger = finalEntries.reduce((acc, e) => { acc[e.dangerLevel] = (acc[e.dangerLevel] ?? 0) + 1; return acc; }, {});
console.log(`[build-command-seed] OK：${finalEntries.length} 条（catalog ${catalogSet.size} 命令全覆盖）`);
console.log(`[build-command-seed] danger 分布：${JSON.stringify(byDanger)}`);
console.log(`[build-command-seed] -> ${outFile}`);
