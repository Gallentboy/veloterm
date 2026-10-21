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
 * 判据取前 200 行内出现顶层块特征：events/http/mail/stream 块，或
 * server 块（.conf 文件里 server { 是 nginx 的决定性信号，apache 用
 * <VirtualHost>、caddy 语法不同，无冲突）。
 */
const NGINX_TOP_BLOCK_RE = /^(events|http|mail|stream|server)\s*\{/;

export function isLikelyNginxContent(content: string): boolean {
  const limit = Math.min(content.length, 64 * 1024);
  const head = content.slice(0, limit);
  const lines = head.split('\n', 200);
  for (const line of lines) {
    if (NGINX_TOP_BLOCK_RE.test(line.trim())) return true;
  }
  return false;
}

/**
 * 带内容的语言判定：仅在名字判为 conf 系（ini）时做 nginx 内容嗅探，
 * 其余结果不变。内容不可用时退化为纯名字判定。
 */
export function resolveLanguageIdWithContent(
  fileName: string,
  content: string | null | undefined,
): string {
  const base = resolveLanguageId(fileName);
  if (base === 'ini' && content && isLikelyNginxContent(content)) {
    return 'nginx';
  }
  return base;
}
