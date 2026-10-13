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
