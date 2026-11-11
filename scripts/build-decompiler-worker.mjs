#!/usr/bin/env node
/**
 * 编译并打包 Java 常驻反编译守护 Worker（velo-worker.jar）：
 * - 依赖 Vineflower 官方 jar (src-tauri/resources/decompiler/vineflower.jar)
 * - 源码: src-tauri/resources/decompiler/VeloDecompilerWorker.java
 * - 产物: src-tauri/resources/decompiler/velo-worker.jar
 */

import { execSync } from 'child_process';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const ROOT = path.resolve(__dirname, '..');

const DECOMPILER_DIR = path.join(ROOT, 'src-tauri', 'resources', 'decompiler');
const JAVA_SRC = path.join(DECOMPILER_DIR, 'VeloDecompilerWorker.java');
const VINEFLOWER_JAR = path.join(DECOMPILER_DIR, 'vineflower.jar');
const OUTPUT_JAR = path.join(DECOMPILER_DIR, 'velo-worker.jar');

if (!fs.existsSync(JAVA_SRC)) {
  console.error(`[build-worker] 找不到源码文件: ${JAVA_SRC}`);
  process.exit(1);
}

if (!fs.existsSync(VINEFLOWER_JAR)) {
  console.error(`[build-worker] 找不到 Vineflower jar: ${VINEFLOWER_JAR}`);
  process.exit(1);
}

const tmpDir = path.join(ROOT, '.cache', 'velo-worker-classes');
fs.rmSync(tmpDir, { recursive: true, force: true });
fs.mkdirSync(tmpDir, { recursive: true });

function findJdk17Bin(binName) {
  // 1. macOS java_home 先找 17+ (17, 21, 25)
  if (process.platform === 'darwin') {
    for (const v of ['17', '21', '25', '17+']) {
      try {
        const home = execSync(`/usr/libexec/java_home -v ${v}`, { encoding: 'utf8' }).trim();
        if (home) {
          const candidate = path.join(home, 'bin', binName);
          if (fs.existsSync(candidate)) return candidate;
        }
      } catch (_) {}
    }
  }
  // 2. JAVA_HOME
  if (process.env.JAVA_HOME) {
    const candidate = path.join(process.env.JAVA_HOME, 'bin', binName);
    if (fs.existsSync(candidate)) return candidate;
  }
  // 3. Fallback
  return binName;
}

const javacBin = findJdk17Bin('javac');
const jarBin = findJdk17Bin('jar');

try {
  console.log(`[build-worker] 使用 javac: ${javacBin}`);
  execSync(`"${javacBin}" -encoding UTF-8 -cp "${VINEFLOWER_JAR}" -d "${tmpDir}" "${JAVA_SRC}"`, {
    stdio: 'inherit',
  });

  console.log(`[build-worker] 正在打包: ${OUTPUT_JAR}`);
  execSync(`"${jarBin}" -cfe "${OUTPUT_JAR}" com.veloterm.decompiler.VeloDecompilerWorker -C "${tmpDir}" .`, {
    stdio: 'inherit',
  });

  const stat = fs.statSync(OUTPUT_JAR);
  console.log(`[build-worker] 打包成功: ${OUTPUT_JAR} (${(stat.size / 1024).toFixed(1)} KB)`);
} finally {
  fs.rmSync(tmpDir, { recursive: true, force: true });
}
