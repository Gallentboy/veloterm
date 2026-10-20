/**
 * 终端输出智能高亮（纯前端）——ERROR/WARN/成功符/关键词着色 + IP/URL 可点击。
 *
 * 实现方式：**写入前转义注入**而非 xterm Decoration（后者只适合标记行级
 * 着色；逐 token 高亮需要重写缓冲，xterm 没有这个 API）。在字节写入
 * `term.write()` 前按行扫描，对命中的片段包裹 ANSI 颜色转义。
 *
 * 性能策略：
 * - 仅扫描可打印行；超长行只扫前 8KB
 * - 着色规则预编译为单个 RegExp（命名组区分类别，命中后 O(规则数) 查表）
 * - 输出分块写入（按行拆分后逐块 write，xterm 自身异步渲染不受影响）
 * - 大块输出（>512KB）直接透传：尾部几行的价值不值得全量扫描的卡顿
 *
 * 着色语义（成功=绿 / 错误=红 / 警告=黄 / 信息=蓝，色值走 16 色
 * 由主题的 bright 变体映射，暗/亮主题均可读）：
 * - ERROR / Error / error / FATAL / PANIC / CRITICAL / Traceback → 红
 * - failed / FAILED / FAIL / failure / errors → 红
 * - WARN / warn / WARNING / DEPRECATED → 黄
 * - ✓ ✔ ☑ → 绿；✗ ✘ ☒ → 红
 * - diff：+ 开头 → 绿，- 开头 → 红（保留缩进；`+++`/`---` 文件头不着色，
 *   ls -l 的权限位行 `-rw-r--r--` 显式排除）
 * - "ok" / OK / success / Successful / done / PASS → 绿
 * - IPv4（可带 :port）/ `::` 压缩形 IPv6 / 常见端口 → 蓝
 * - 已着色探测：若该行已含 ANSI 前景色码，跳过整行（避免与程序自身
 *   颜色冲突——git diff --color、ls --color 等自带颜色的输出不再二次着色）
 *
 * 可点击链接（URL / IPv4）走 xterm registerLinkProvider——原样写入文本、
 * 悬停下划线 + 点击事件，不注入转义。URL 经合成 anchor(target=_blank)
 * 走系统浏览器（同 MarkdownContent 的既有通路，零后端依赖）；裸 IPv4
 * 点击复制到剪贴板。
 */
import type { IDecoration, IDisposable, ILink, IMarker, Terminal } from '@xterm/xterm';
import { writeClipboardText } from '@/lib/clipboard';

/** 着色规则：全部拼进单个命名组正则，命中类别靠组名查表 */
interface HighlightRule {
  pattern: RegExp;
  /** ANSI 前景色码 */
  color: string;
}

const RULES: HighlightRule[] = [
  // 错误（红）
  { pattern: /\b(ERROR|FATAL|PANIC|CRITICAL|Traceback)\b/g, color: '91' },
  { pattern: /\b(Error|error|errors|Errors|failed|FAILED|failure|Failure|FAIL|Fail|fail)\b/g, color: '91' },
  { pattern: /\b[Ee]xception\b/g, color: '91' },
  // 警告（黄）
  { pattern: /\b(WARN|warn|WARNING|Warning|warning|DEPRECATED|Deprecated|deprecated)\b/g, color: '93' },
  // 成功（绿）
  { pattern: /\b(OK|ok|success|Success|Successful|successful|done|Done|PASS|passed|PASSED)\b/g, color: '92' },
  // 成功/失败符号（非单词字符，无 \b 可言）
  { pattern: /[✓✔☑]/g, color: '92' },
  { pattern: /[✗✘☒]/g, color: '91' },
  // 常见端口（蓝）——负向断言排掉「12:22」这类时间戳（IP:port 形态由 IPv4 规则整体着色）
  { pattern: /(?<![\d:])(:22|:80|:443|:3000|:3306|:5432|:6379|:8080|:8443|:8888|:9090|:27017)\b/g, color: '94' },
  // IPv4（蓝，可带 :port）
  { pattern: /\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}(:\d+)?\b/g, color: '94' },
  // IPv6（蓝）——完整 8 段形或 `::` 压缩形；不收「段:段」短形（会误伤 12:34:56 时间戳与 MAC 地址）
  { pattern: /\b(?:[0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}\b|\b(?:[0-9a-fA-F]{1,4}:)*::(?:[0-9a-fA-F]{1,4}:)*[0-9a-fA-F]{0,4}\b/g, color: '94' },
];

/** 预编译的联合正则：第 i 条规则包进命名组 r{i}，命中后 m.groups 里查类别 */
const COMBINED = new RegExp(
  RULES.map((rule, i) => `(?<r${i}>${rule.pattern.source})`).join('|'),
  'g',
);

/** diff 行前缀着色（独立于联合正则——只对行首生效；`++`/`--` 开头的长选项/分隔线排除） */
const DIFF_ADD = /^(\s*)(\+(?!\+).*)$/;
const DIFF_DEL = /^(\s*)(-(?!-).*)$/;
const DIFF_HUNK = /^(@@.*@@)$/;
/** ls -l 的权限位行（`-rw-r--r-- ...`）——与 diff 删除行同形，显式排除 */
const LS_PERM_LINE = /^\s*[-dl][rwxsStT+-]{9}(\s|$)/;

/** 已含 ANSI 前景色码 → 跳过整行 */
const HAS_ANSI_FG = /\x1b\[(3|4|9|10)[0-9]m/;

/**
 * 扫描单行文本，返回带 ANSI 转义注入的行（原样返回=未命中）。
 * 行内的 CSI/OSC 序列会被当普通文本扫（规则均为可见字符词形，
 * 不会在序列字节内命中；已着色行整体跳过兜底）。
 */
function highlightLine(line: string): string {
  if (line.length > 8192) return line; // 超长行跳过（性能兜底）
  if (HAS_ANSI_FG.test(line)) return line; // 程序已着色，跳过
  if (LS_PERM_LINE.test(line)) return line;

  // diff 行首着色（+/- 前缀 + 缩进原样保留）
  let diffResult: string | null = null;
  const da = DIFF_ADD.exec(line);
  if (da) {
    diffResult = `${da[1]}\x1b[92m${da[2]}\x1b[0m`;
  } else {
    const dd = DIFF_DEL.exec(line);
    if (dd) {
      diffResult = `${dd[1]}\x1b[91m${dd[2]}\x1b[0m`;
    } else if (DIFF_HUNK.test(line)) {
      diffResult = `\x1b[96m${line}\x1b[0m`;
    }
  }
  if (diffResult) return diffResult;

  // 联合正则逐 token 着色，类别经命名组查表（零 RegExp 重建）
  let result = '';
  let lastIdx = 0;
  COMBINED.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = COMBINED.exec(line)) !== null) {
    const hit = m[0];
    const start = m.index;
    result += line.slice(lastIdx, start);
    let color = '94'; // 兜底蓝（未查到组时）
    const groups = m.groups ?? {};
    for (let i = 0; i < RULES.length; i++) {
      if (groups[`r${i}`] !== undefined) {
        color = RULES[i].color;
        break;
      }
    }
    result += `\x1b[${color}m${hit}\x1b[0m`;
    lastIdx = start + hit.length;
    if (COMBINED.lastIndex === m.index) COMBINED.lastIndex++; // 零宽保护
  }
  if (lastIdx === 0) return line; // 无命中
  result += line.slice(lastIdx);
  return result;
}

/**
 * 输出通道的同步处理函数（挂在 createTerminalByteChannel → term.write 之间）：
 * 按行扫描高亮后返回新文本，\r\n / \n 原样保留。
 * 单行且无换行的部分写入不扫（可能是不完整 token，下次写入会补全；
 * 此处扫了下次也不会重扫，宁可漏不可错）。
 */
export function highlightOutput(data: string): string {
  if (data.length > 512 * 1024) return data;
  const lines = data.split('\n');
  if (lines.length === 1 && !data.includes('\r')) return data;
  return lines.map((line) => highlightLine(line)).join('\n');
}

/** 链接识别：带 scheme 的 URL / www. 裸域 / IPv4（可带 :port） */
const LINK_RE = /\b(?:https?:\/\/|www\.)[^\s<>"'`\]]+|(?:\d{1,3}\.){3}\d{1,3}(?::\d{1,5})?/g;
/** 匹配尾部的标点不算链接的一部分（句子里的逗号/右括号等） */
const LINK_TRAILING_PUNCT = /[.,;:!?)]+$/;

function openExternalUrl(url: string): void {
  // 合成 anchor 与 AI 面板 MarkdownContent 同一通路（target=_blank → 系统浏览器）
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.target = '_blank';
  anchor.rel = 'noopener';
  anchor.click();
}

/** 链接悬停色：终端主题蓝（随暗/亮主题切换，逐次解析 CSS 变量） */
function resolveLinkColor(): string {
  const value = getComputedStyle(document.documentElement)
    .getPropertyValue('--color-terminal-blue')
    .trim();
  return /^#[0-9a-fA-F]{6}$/.test(value) ? value : '#4099ff';
}

/**
 * 注册 IP/URL 可点击链接（xterm LinkProvider）：悬停下划线 + 手型光标由
 * xterm 自带；悬停还叠一层前景色 decoration 让链接变蓝（终端主题色，
 * leave/重悬停时撞掉）。点击 URL 开系统浏览器，点击裸 IPv4 复制到剪贴板。
 * 返回 IDisposable，随 term.dispose 一并释放（也可手动 dispose）。
 */
export function registerOutputLinkProvider(term: Terminal): IDisposable {
  // 悬停高亮 decoration 单例：同一时刻只保留一份，悬停新链接时先撞旧的
  let hoverDecoration: IDecoration | null = null;
  let hoverMarker: IMarker | null = null;
  const clearHoverHighlight = () => {
    try {
      hoverDecoration?.dispose();
    } catch {
      // decoration 可能已随 marker 失效被自动释放
    }
    hoverDecoration = null;
    try {
      hoverMarker?.dispose();
    } catch {
      // 同上
    }
    hoverMarker = null;
  };

  return term.registerLinkProvider({
    provideLinks(bufferLineNumber: number, callback: (links: ILink[] | undefined) => void) {
      const line = term.buffer.active.getLine(bufferLineNumber - 1);
      if (!line) {
        callback([]);
        return;
      }
      // 逐 cell 重建行文本并记录「字符下标 → cell 列号」映射：
      // translateToString 对宽字符（占 2 cell）只出 1 字符，直接用字符串
      // 下标当列号会漂移，CJK 行内的链接会错位。
      let text = '';
      const cellOfChar: number[] = [];
      for (let x = 0; x < term.cols; x++) {
        const cell = line.getCell(x);
        if (!cell) break;
        const chars = cell.getChars();
        if (chars) {
          for (const _ch of chars) cellOfChar.push(x);
          text += chars;
        }
      }
      if (!text) {
        callback([]);
        return;
      }
      const links: ILink[] = [];
      LINK_RE.lastIndex = 0;
      let m: RegExpExecArray | null;
      while ((m = LINK_RE.exec(text)) !== null) {
        let token = m[0].replace(LINK_TRAILING_PUNCT, '');
        const start = m.index;
        const end = start + token.length; // [start, end)
        if (!token || cellOfChar[start] === undefined || cellOfChar[end - 1] === undefined) continue;
        const isUrl = /^https?:\/\/|^www\./i.test(token);
        const activateTarget = token;
        links.push({
          range: {
            start: { x: cellOfChar[start] + 1, y: bufferLineNumber },
            end: { x: cellOfChar[end - 1] + 1, y: bufferLineNumber },
          },
          text: token,
          hover: () => {
            clearHoverHighlight();
            const buffer = term.buffer.active;
            // registerMarker 只接受相对光标行的偏移，任意缓冲行（含 scrollback）
            // 换算成 offset 后同样可挂
            const marker = term.registerMarker(bufferLineNumber - (buffer.baseY + buffer.cursorY));
            if (!marker) return;
            hoverMarker = marker;
            hoverDecoration =
              term.registerDecoration({
                marker,
                x: cellOfChar[start],
                width: cellOfChar[end - 1] - cellOfChar[start] + 1,
                foregroundColor: resolveLinkColor(),
              }) ?? null;
          },
          leave: () => clearHoverHighlight(),
          dispose: () => clearHoverHighlight(),
          activate: () => {
            clearHoverHighlight();
            if (isUrl) {
              openExternalUrl(/^https?:\/\//i.test(activateTarget) ? activateTarget : `https://${activateTarget}`);
            } else {
              // 裸 IPv4 / IPv4:port —— 复制比「打开」更常见的需求
              void writeClipboardText(activateTarget);
            }
          },
        });
        if (LINK_RE.lastIndex === m.index) LINK_RE.lastIndex++; // 零宽保护
      }
      callback(links);
    },
  });
}
