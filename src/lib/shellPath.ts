/**
 * 终端路径书写格式化：把文件绝对路径转成「可以直接粘进命令行」的形式，
 * 对齐原生终端的拖拽行为（Windows conhost/Windows Terminal、macOS Terminal.app/zsh）。
 */

/** 路径书写风格：跟随目标 shell 的引用/转义约定 */
export type TerminalPathStyle = 'windows' | 'posix';

/**
 * POSIX shell 元字符集合（zsh/bash 通吃）：空白与通配/引用/控制字符
 * 一律反斜杠转义，与 Terminal.app 把文件拖进 zsh 的产物一致（不加外层引号，
 * 保留单词可继续补全的手感）。
 */
const POSIX_ESCAPE_CHARS = new Set(
  ' \t\n\\&<>()$!*?[]{};\'"`~#|^'.split(''),
);

/** cmd/PowerShell 需要双引号包裹的字符：空白 + cmd 元字符（与原生拖拽的「按需引号」一致） */
const WINDOWS_QUOTE_PATTERN = /[\s&(){}[\]^=;!'+,`~"]/;

/**
 * 单个路径按目标 shell 风格格式化：
 * - `windows`：仅当包含空白或 cmd 元字符时用双引号包裹（conhost/Windows
 *   Terminal 拖拽行为；无特殊字符时保持裸路径，便于 Tab 补全续接）；
 * - `posix`：逐字符反斜杠转义元字符（普通路径零改动）。
 */
export function formatPathForTerminal(path: string, style: TerminalPathStyle): string {
  if (style === 'windows') {
    return WINDOWS_QUOTE_PATTERN.test(path) ? `"${path}"` : path;
  }
  let out = '';
  for (const ch of path) {
    out += POSIX_ESCAPE_CHARS.has(ch) ? `\\${ch}` : ch;
  }
  return out;
}

/**
 * 多个文件路径格式化并拼接（原生终端一次拖多个文件的行为：空格分隔依次写入）。
 */
export function formatPathsForTerminal(
  paths: string[],
  style: TerminalPathStyle,
): string {
  return paths.map((path) => formatPathForTerminal(path, style)).join(' ');
}
