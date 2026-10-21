/**
 * 扩展名 → Monaco language id 映射（离线内置语言，与 text_detect 白名单同源扩展）。
 * 未知后缀回退 plaintext。
 */

const EXT_LANGUAGE_MAP: Record<string, string> = {
  rs: 'rust',
  ts: 'typescript',
  tsx: 'typescript',
  js: 'javascript',
  jsx: 'javascript',
  mjs: 'javascript',
  cjs: 'javascript',
  py: 'python',
  go: 'go',
  c: 'c',
  h: 'cpp',
  cpp: 'cpp',
  cc: 'cpp',
  hpp: 'cpp',
  java: 'java',
  kt: 'kotlin',
  swift: 'swift',
  php: 'php',
  rb: 'ruby',
  cs: 'csharp',
  sh: 'shell',
  bash: 'shell',
  zsh: 'shell',
  // Windows Batch（自建 batch 语言：Monarch + 补全 + goto 标签跳转，
  // 见 editor/batchLanguage.ts）
  bat: 'batch',
  cmd: 'batch',
  sql: 'sql',
  json: 'json',
  yaml: 'yaml',
  yml: 'yaml',
  toml: 'ini',
  xml: 'xml',
  html: 'html',
  css: 'css',
  scss: 'scss',
  less: 'less',
  ini: 'ini',
  conf: 'ini',
  cfg: 'ini',
  env: 'ini',
  md: 'markdown',
  mdx: 'markdown',
  txt: 'plaintext',
  log: 'plaintext',
  csv: 'plaintext',
  // *.dockerfile（Dockerfile 精确名与 Dockerfile.dev 基名剥离已覆盖）
  dockerfile: 'dockerfile',
};

/** 无后缀的常见文本文件名 → language id */
const EXACT_NAME_LANGUAGE_MAP: Record<string, string> = {
  dockerfile: 'dockerfile',
  makefile: 'makefile',
  cmakelists: 'cmake',
  'cmakelists.txt': 'cmake',
  gemfile: 'ruby',
  procfile: 'plaintext',
  vagrantfile: 'ruby',
  readme: 'markdown',
  // nginx 主配置名精确命中（其余 .conf 大多是普通 ini 系配置，
  // 是否 nginx 由 resolveLanguageIdWithContent 按内容嗅探判定）
  'nginx.conf': 'nginx',
};

export function resolveLanguageId(fileName: string): string {
  const lower = fileName.toLowerCase();
  const exact = EXACT_NAME_LANGUAGE_MAP[lower];
  if (exact) {
    return exact;
  }
  const base = lower.replace(/\.[^.]*$/, '');
  if (EXACT_NAME_LANGUAGE_MAP[base]) {
    return EXACT_NAME_LANGUAGE_MAP[base]!;
  }
  const ext = lower.includes('.') ? lower.slice(lower.lastIndexOf('.') + 1) : '';
  return EXT_LANGUAGE_MAP[ext] ?? 'plaintext';
}

/**
 * 内容嗅探：conf 系文件（conf/cfg/ini 等映射为 ini 的）是否实为 nginx 配置。
 * nginx 的站点配置常不带 nginx 字样（sites-available/default、xxx.conf），
 * 判据分两档：
 * - 强信号（命中即判）：顶层块——无参块 events/http/mail/stream/server/types
 *   与带参块 upstream/map/location（map $a $b { / upstream backend { 形态）；
 * - 弱信号（≥2 个不同信号命中才判）：listen 端口;/server_name/proxy_pass/
 *   fastcgi_param/include 的「指令 + 分号」形态——单个巧合不判，防误报。
 * 反例对照：php.ini/my.cnf/smb.conf 是 [节] 形态、apache 用 <VirtualHost>、
 * haproxy 无花括号块、caddy 顶层关键字不同、php-fpm 与 dovecot 的
 * listen 是等号赋值（listen = 9000），均不命中。
 */
const NGINX_TOP_BLOCK_RE =
  /^(events|http|mail|stream|server|types)\s*\{|^(upstream|map|location)\s+[^{]*\{/;

/** 弱信号表：指令 + 分号形态（按不同信号去重计数，阈值 ≥2） */
const NGINX_WEAK_SIGNAL_RES: readonly RegExp[] = [
  /^listen\s+\d+\s*;$/,
  /^server_name\s+[^;]+;$/,
  /^proxy_pass\s+[^;]+;$/,
  /^fastcgi_param\s+[^;]+;$/,
  /^include\s+[^;]+;$/,
];

const NGINX_WEAK_SIGNAL_THRESHOLD = 2;

export function isLikelyNginxContent(content: string): boolean {
  const limit = Math.min(content.length, 64 * 1024);
  const head = content.slice(0, limit);
  const lines = head.split('\n', 200);
  let weakHits = 0;
  const seenSignals = new Set<number>();
  for (const line of lines) {
    const trimmed = line.trim();
    if (NGINX_TOP_BLOCK_RE.test(trimmed)) return true;
    for (let i = 0; i < NGINX_WEAK_SIGNAL_RES.length; i += 1) {
      const re = NGINX_WEAK_SIGNAL_RES[i]!;
      if (!seenSignals.has(i) && re.test(trimmed)) {
        seenSignals.add(i);
        weakHits += 1;
        if (weakHits >= NGINX_WEAK_SIGNAL_THRESHOLD) return true;
      }
    }
  }
  return false;
}

/** 路径含 /nginx/ 段（/etc/nginx/... 形态；相对路径 nginx/ 开头同认） */
const NGINX_DIR_RE = /(?:^|\/)nginx\//i;
/** 目录名启发（sites-available/sites-enabled/conf.d）——Apache2 有同名目录，
 * 只与内容嗅探叠加（Apache 的 <VirtualHost> 形态会被嗅探正确拒绝） */
const NGINX_LIKE_DIR_RE = /\/(?:sites-available|sites-enabled|conf\.d)\//i;

/** 名字是否「已知」：精确名（含基名剥离）或已知扩展名命中——已知名字不被
 * 路径/嗅探劫持（readme→markdown、dockerfile 等无扩展名精确名同受保护） */
function isUnknownLanguageName(lowerFileName: string): boolean {
  if (EXACT_NAME_LANGUAGE_MAP[lowerFileName]) {
    return false;
  }
  const base = lowerFileName.replace(/\.[^.]*$/, '');
  if (EXACT_NAME_LANGUAGE_MAP[base]) {
    return false;
  }
  if (!lowerFileName.includes('.')) {
    return true;
  }
  const ext = lowerFileName.slice(lowerFileName.lastIndexOf('.') + 1);
  return !Object.prototype.hasOwnProperty.call(EXT_LANGUAGE_MAP, ext);
}

/**
 * 带内容/路径的语言判定（nginx 三级识别的完整入口）：
 * - 名字精确命中（nginx.conf）优先；
 * - 路径启发：conf 族（conf/cfg/ini→ini）或无/未知扩展名的文件，路径含
 *   /nginx/ → 直接判 nginx（覆盖 mime.types / fastcgi_params 这类无顶层块
 *   的文件；已知扩展名如 .txt/.sh 不被劫持）；
 * - 内容嗅探：conf 族 / 无（未知）扩展名 / sites 族目录的文件，内容命中
 *   强/弱信号 → nginx；sites 族目录单列是因为 Apache2 同名目录存在，路径
 *   只提升候选、内容确认后才判。
 * 内容不可用时退化为纯名字判定（路径启发仍生效）。
 */
export function resolveLanguageIdWithContent(
  fileName: string,
  content: string | null | undefined,
  remotePath?: string,
): string {
  const base = resolveLanguageId(fileName);
  if (base === 'nginx') return base;
  const lowerName = fileName.toLowerCase();
  const lowerPath = (remotePath ?? '').toLowerCase();
  const isConfFamily = base === 'ini';
  // 无扩展名/未知扩展名/未知名字（EXT_LANGUAGE_MAP 与精确名表都未收录）——
  // resolveLanguageId 对这类回退 plaintext，这里需与 txt/log 等已知名区分
  const isUnknown = isUnknownLanguageName(lowerName);
  if (NGINX_DIR_RE.test(lowerPath) && (isConfFamily || isUnknown)) {
    return 'nginx';
  }
  const sniffCandidate = isConfFamily || isUnknown || NGINX_LIKE_DIR_RE.test(lowerPath);
  if (sniffCandidate && content && isLikelyNginxContent(content)) {
    return 'nginx';
  }
  return base;
}
