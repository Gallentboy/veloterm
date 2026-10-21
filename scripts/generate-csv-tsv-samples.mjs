#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');
const outDir = path.join(rootDir, 'src-tauri', 'testdata');

if (!fs.existsSync(outDir)) {
  fs.mkdirSync(outDir, { recursive: true });
}

const csvPath = path.join(outDir, 'sample.csv');
const tsvPath = path.join(outDir, 'sample.tsv');

const METHODS = ['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'HEAD'];
const PATHS = [
  '/api/v1/auth/login',
  '/api/v1/users',
  '/api/v1/servers/status',
  '/api/v1/sftp/list',
  '/api/v1/terminal/session',
  '/healthz',
  '/metrics',
  '/index.html',
  '/static/app.js',
];
const STATUSES = [200, 201, 204, 301, 400, 401, 403, 404, 500, 502];
const USER_AGENTS = [
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)',
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64)',
  'curl/8.7.1',
  'VeloTerm-Client/1.0.4',
  'Go-http-client/1.1',
];

const TARGET_SIZE = 2 * 1024 * 1024; // 2MB

function generateData(delimiter) {
  const headers = [
    'id',
    'timestamp',
    'ip_address',
    'method',
    'path',
    'status_code',
    'response_time_ms',
    'bytes_sent',
    'user_agent',
    'description',
  ];

  const lines = [headers.join(delimiter)];
  let totalBytes = lines[0].length + 1;
  let id = 1;
  const baseTime = Date.now() - 86400000;

  while (totalBytes < TARGET_SIZE) {
    const timestamp = new Date(baseTime + id * 1234).toISOString();
    const ip = `${(id % 200) + 10}.${(id % 254) + 1}.${((id * 7) % 254) + 1}.${((id * 13) % 254) + 1}`;
    const method = METHODS[id % METHODS.length];
    const reqPath = PATHS[id % PATHS.length];
    const status = STATUSES[id % STATUSES.length];
    const respTime = ((id * 37) % 1500 + (id % 10) * 0.12).toFixed(2);
    const bytesSent = (id * 239) % 65536;
    const ua = USER_AGENTS[id % USER_AGENTS.length];
    
    // 偶尔包含带逗号/引号的字段，验证 RFC 4180 转义
    let desc = `request #${id} completed`;
    if (id % 7 === 0) {
      desc = `status is "OK", with latency ${respTime}ms`;
    } else if (id % 13 === 0) {
      desc = `client ip: ${ip}; processed, all good`;
    }

    // 格式化单元格，必要时双引号包裹
    const formatCell = (val) => {
      const s = String(val);
      if (delimiter === ',' && (s.includes(',') || s.includes('"') || s.includes('\n'))) {
        return `"${s.replace(/"/g, '""')}"`;
      }
      if (delimiter === '\t' && (s.includes('\t') || s.includes('"') || s.includes('\n'))) {
        return `"${s.replace(/"/g, '""')}"`;
      }
      return s;
    };

    const row = [
      id,
      timestamp,
      ip,
      method,
      reqPath,
      status,
      respTime,
      bytesSent,
      formatCell(ua),
      formatCell(desc),
    ].join(delimiter);

    lines.push(row);
    totalBytes += Buffer.byteLength(row, 'utf8') + 1;
    id++;
  }

  return lines.join('\n');
}

console.log('Generating sample.csv (~2MB)...');
const csvData = generateData(',');
fs.writeFileSync(csvPath, csvData, 'utf8');
const csvSize = fs.statSync(csvPath).size;
console.log(`Created: ${csvPath} (${(csvSize / (1024 * 1024)).toFixed(2)} MB, ${csvData.split('\n').length} rows)`);

console.log('Generating sample.tsv (~2MB)...');
const tsvData = generateData('\t');
fs.writeFileSync(tsvPath, tsvData, 'utf8');
const tsvSize = fs.statSync(tsvPath).size;
console.log(`Created: ${tsvPath} (${(tsvSize / (1024 * 1024)).toFixed(2)} MB, ${tsvData.split('\n').length} rows)`);
