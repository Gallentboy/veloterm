/**
 * nginx 语言支持：语言注册 + Monarch 高亮 + 上下文感知补全 + hover。
 *
 * 移植自 pappasam/nginx-language-server v0.9.0（GPL-3.0，数据与许可见
 * ./nginxData/README.md）——只移植前端可独立完成的 completion/hover 两个
 * 能力，语义对齐其 server.py / parser/data.py：
 *
 * - 索引：directives.json 按 contexts[] 展开成「上下文 → 指令名 → 条目」
 *   双层索引（对齐 data.py::get_directives）；
 * - 补全候选：当前上下文指令 ∪ any 上下文指令 ∪ 全部 $ 变量（对齐
 *   server.py::completion 的候选并集语义）；
 * - contexts 解析：逐行扫描维护块指令栈（替代上游的 crossplane + 50 行
 *   回溯近似——栈扫描对「块开行之后的空行」直接给出块内上下文，比上游
 *   更精确）；光标行自身的块开头不并入（对齐上游「本行符号取祖先链」）；
 * - 两处有意偏离上游：① 光标前是 `$` 时 range 覆盖 `$` 且只出变量
 *   （Monaco word 不含 `$`，不处理会替换残留 `$`；`$` 触发意图明确是
 *   变量）；② 未知上下文（如 map 块）上游整轮返回空，这里退化为只出
 *   变量（`$` 补全在 map 块内仍可用）。
 *
 * 本模块只做 type-only 的 monaco 导入（运行期实例由宿主传入），纯逻辑
 * （computeContexts / 构建索引）可脱离 bundler 单独验证。数据经 dynamic
 * import 打成独立 async chunk，不打开 nginx 文件的会话零加载成本。
 */

import type * as MonacoNs from 'monaco-editor';

// ─── 数据形状（directives.json / variables.json，字段见 nginxData/README.md）───

interface RawDirective {
  name: string;
  syntax: string[];
  def?: string | null;
  contexts: string[];
  desc: string;
  notes?: string[];
  since?: string | null;
  module: string;
}

interface RawVariable {
  name: string;
  desc: string;
  module: string;
}

/** 补全/hover 共用的条目（文档串在索引构建时一次性生成，对齐上游预构建） */
interface NginxEntry {
  name: string;
  detail: string;
  documentation: string;
}

interface NginxDataIndex {
  /** 上下文 → 指令名 → 条目 */
  directives: Map<string, Map<string, NginxEntry>>;
  /** 变量名（含 $ 前缀）→ 条目 */
  variables: Map<string, NginxEntry>;
}

// ─── Markdown 文档生成（对齐 data.py 的 ls_documentation；72 列强制换行是
// LSP 弹窗宽度适配，Monaco 文档浮层自适应宽度，原文更自然——不换行）───

function escapeMarkdown(str: string): string {
  return str.replace(/_/g, '\\_');
}

function normalizeDesc(desc: string): string {
  const trimmed = desc.trim();
  if (!trimmed) return '';
  // 上游：句尾冒号收为句号（desc 截自文档标题行时常以冒号结尾）
  const body = trimmed.endsWith(':') ? `${trimmed.slice(0, -1)}.` : trimmed;
  return escapeMarkdown(body);
}

function directiveDoc(d: RawDirective): string {
  const parts: string[] = [];
  if (d.def) {
    // 上游把 default 按 ";" 拆开后每段补换行（每条指令一行）
    const formatted = d.def
      .split(';')
      .map((s) => s.trim())
      .filter(Boolean)
      .map((s) => `${s};`)
      .join('\n');
    if (formatted) parts.push(`\`\`\`nginx\n${formatted}\n\`\`\``);
  }
  const desc = normalizeDesc(d.desc);
  if (desc) parts.push(desc);
  if (d.syntax?.length) {
    parts.push(`\`\`\`nginx\n${d.syntax.join('\n')}\n\`\`\``);
  }
  if (d.contexts?.length) {
    parts.push(`**Contexts:** \`${d.contexts.join(', ')}\``);
  }
  if (d.module) parts.push(escapeMarkdown(`**Module:** ${d.module}`));
  if (d.since) parts.push(escapeMarkdown(`**Since:** ${d.since}`));
  if (d.notes?.length) {
    const notes = d.notes.map((n) => escapeMarkdown(n.trim())).filter(Boolean);
    if (notes.length === 1) {
      parts.push(`*Notes:* ${notes[0]}`);
    } else if (notes.length > 1) {
      parts.push(`*Notes:*\n${notes.map((n) => `- ${n}`).join('\n')}`);
    }
  }
  // 上游把弯引号归一为直引号
  return parts.join('\n\n').replace(/[“”]/g, '"').trim();
}

function variableDoc(v: RawVariable): string {
  const parts: string[] = [];
  const desc = normalizeDesc(v.desc);
  if (desc) parts.push(desc);
  if (v.module) parts.push(escapeMarkdown(`**Module:** ${v.module}`));
  return parts.join('\n\n').replace(/[“”]/g, '"').trim();
}

// ─── contexts 解析（替代 crossplane：光标行的祖先块指令链）───

/**
 * 把一行按引号感知的方式切成 token（供栈扫描判断块开/块闭）。
 * 引号内的 `{` `}` `#` `;` 都不参与结构判定；nginx 字符串不跨行。
 */
function tokenizeNginxLine(line: string): string[] {
  const tokens: string[] = [];
  let current = '';
  let quote: '"' | "'" | null = null;
  for (const ch of line) {
    if (quote) {
      current += ch;
      if (ch === quote) quote = null;
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
      current += ch;
      continue;
    }
    if (ch === '#') break; // 行内注释，其后不再参与
    if (/\s/.test(ch)) {
      if (current) tokens.push(current);
      current = '';
      continue;
    }
    if (ch === '{' || ch === '}' || ch === ';') {
      if (current) tokens.push(current);
      current = '';
      tokens.push(ch);
      continue;
    }
    current += ch;
  }
  if (current) tokens.push(current);
  return tokens;
}

/**
 * 计算光标行的祖先块指令链（contexts）。只处理 cursorLine 之前的行——
 * 光标行自身的块开头不并入（`server {` 行上的符号取的是 ['http'] 而非
 * ['http','server']，对齐上游 _convert 记录祖先链的语义）。空栈表示顶层，
 * 调用方兜底为 ['main']。
 */
export function computeContexts(lines: readonly string[], cursorLine: number): string[] {
  const stack: string[] = [];
  const limit = Math.min(cursorLine, lines.length);
  for (let i = 0; i < limit; i += 1) {
    let directive = '';
    for (const token of tokenizeNginxLine(lines[i])) {
      if (token === '{') {
        if (directive) stack.push(directive);
        directive = '';
      } else if (token === '}') {
        // 与最近的 { 配对弹出；对不配对的孤立 } 容错弹出
        stack.pop();
        directive = '';
      } else if (token === ';') {
        directive = '';
      } else if (!directive) {
        directive = token;
      }
    }
  }
  return stack;
}

// ─── 索引懒加载 ───

let dataIndex: NginxDataIndex | null = null;
let dataIndexLoading: Promise<NginxDataIndex> | null = null;

function buildIndex(raw: RawDirective[], rawVars: RawVariable[]): NginxDataIndex {
  const directives = new Map<string, Map<string, NginxEntry>>();
  for (const d of raw) {
    const entry: NginxEntry = {
      name: d.name,
      detail: `dir: ${d.name}`,
      documentation: directiveDoc(d),
    };
    for (const ctx of d.contexts) {
      let bucket = directives.get(ctx);
      if (!bucket) {
        bucket = new Map();
        directives.set(ctx, bucket);
      }
      bucket.set(d.name, entry);
    }
  }
  const variables = new Map<string, NginxEntry>();
  for (const v of rawVars) {
    variables.set(v.name, {
      name: v.name,
      detail: `var: ${v.name}`,
      documentation: variableDoc(v),
    });
  }
  return { directives, variables };
}

function ensureDataIndex(): Promise<NginxDataIndex> {
  if (dataIndex) return Promise.resolve(dataIndex);
  if (!dataIndexLoading) {
    dataIndexLoading = Promise.all([
      import('./nginxData/directives.json'),
      import('./nginxData/variables.json'),
    ]).then(([directivesMod, variablesMod]) => {
      dataIndex = buildIndex(
        directivesMod.default as unknown as RawDirective[],
        variablesMod.default as unknown as RawVariable[],
      );
      return dataIndex;
    });
  }
  return dataIndexLoading;
}

// ─── 候选集合（对齐 server.py：当前上下文 ∪ any ∪ 变量；hover 额外合并
// if-in-location 特例上下文 ifinlocation）───

function candidatesFor(
  index: NginxDataIndex,
  contexts: string[],
  options: { mergeIfInLocation?: boolean } = {},
): Map<string, NginxEntry> {
  const chain = contexts.length > 0 ? contexts : ['main'];
  const last = chain[chain.length - 1]!;
  const merged = new Map<string, NginxEntry>();
  const addContext = (ctx: string) => {
    for (const entry of index.directives.get(ctx)?.values() ?? []) {
      merged.set(entry.name, entry);
    }
  };
  addContext(last);
  addContext('any');
  if (
    options.mergeIfInLocation &&
    chain.length >= 2 &&
    chain[chain.length - 1] === 'if' &&
    chain[chain.length - 2] === 'location'
  ) {
    addContext('ifinlocation');
  }
  return merged;
}

// ─── Monarch 高亮 ───

const NGINX_MONARCH_LANGUAGE: MonacoNs.languages.IMonarchLanguage = {
  defaultToken: '',
  tokenPostfix: '.nginx',
  brackets: [
    { token: 'delimiter.bracket', open: '{', close: '}' },
  ],
  tokenizer: {
    root: [
      [/#.*$/, 'comment'],
      [/"([^"\\]|\\.)*"?/, 'string'],
      [/'([^'\\]|\\.)*'?/, 'string'],
      [/\$[\w]+/, 'variable'],
      [/[{}]/, 'delimiter.bracket'],
      [/[;]/, 'delimiter'],
      [/[=~^]/, 'operator'],
    ],
  },
};

// ─── 注册入口（幂等；monaco 实例由宿主传入）───

let registered = false;

export function ensureNginxLanguage(monaco: typeof MonacoNs): void {
  if (registered) return;
  registered = true;

  monaco.languages.register({
    id: 'nginx',
    extensions: [],
    filenames: ['nginx.conf'],
  });
  monaco.languages.setMonarchTokensProvider('nginx', NGINX_MONARCH_LANGUAGE);

  monaco.languages.registerCompletionItemProvider('nginx', {
    triggerCharacters: ['$'],
    async provideCompletionItems(model, position) {
      const index = await ensureDataIndex();
      const lineContent = model.getValueInRange({
        startLineNumber: position.lineNumber,
        startColumn: 1,
        endLineNumber: position.lineNumber,
        endColumn: position.column,
      });
      const word = model.getWordUntilPosition(position);
      // Monaco word 不含 `$`：光标前一个字符是 `$` 时进入变量模式，
      // 替换范围覆盖 `$`、候选只出变量（见文件头「有意偏离」①）
      const charBeforeWord =
        word.startColumn > 1
          ? lineContent.charAt(word.startColumn - 2)
          : '';
      const variableMode = charBeforeWord === '$';
      const replace = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        // 变量模式起点前移一位盖住 `$`；无 word 时（光标紧跟空格）起于光标处
        startColumn: variableMode ? word.startColumn - 1 : word.startColumn || position.column,
        endColumn: word.endColumn || position.column,
      };
      const kind = monaco.languages.CompletionItemKind.Property;
      const toItems = (entries: Iterable<NginxEntry>) => {
        const items: MonacoNs.languages.CompletionItem[] = [];
        for (const entry of entries) {
          items.push({
            label: entry.name,
            kind,
            detail: entry.detail,
            documentation: { value: entry.documentation },
            insertText: entry.name,
            // 不设 insertTextRules：保持纯文本插入（对齐上游 PlainText）。
            // InsertAsSnippet 会把 $arg_name 的 $ 当 snippet 变量占位符，
            // 未定义变量解析为空串 → 选中变量回车后什么都不插入
            range: replace,
          });
        }
        return items;
      };
      if (variableMode) {
        return { suggestions: toItems(index.variables.values()) };
      }
      const contexts = computeContexts(
        model.getValue().split('\n'),
        position.lineNumber - 1,
      );
      const last = contexts[contexts.length - 1] ?? 'main';
      const bucket = index.directives.get(last);
      if (!bucket) {
        // 未知上下文（如 map 块）：上游整轮返回空，这里保留变量补全（偏离②）
        return { suggestions: toItems(index.variables.values()) };
      }
      const merged = candidatesFor(index, contexts);
      for (const entry of index.variables.values()) {
        if (!merged.has(entry.name)) merged.set(entry.name, entry);
      }
      return { suggestions: toItems(merged.values()) };
    },
  });

  monaco.languages.registerHoverProvider('nginx', {
    async provideHover(model, position) {
      const index = await ensureDataIndex();
      const word = model.getWordAtPosition(position);
      if (!word) return null;
      const lineContent = model.getLineContent(position.lineNumber);
      const charBeforeWord =
        word.startColumn > 1 ? lineContent.charAt(word.startColumn - 2) : '';
      const contexts = computeContexts(
        model.getValue().split('\n'),
        position.lineNumber - 1,
      );
      const merged = candidatesFor(index, contexts, { mergeIfInLocation: true });
      // 查找序：$word（变量）→ word（指令/变量）→ word 去尾段 + "_name"
      // （上游的 _name 后缀启发：`fastcgi_script` 命中 `fastcgi_script_name`）
      const candidates: (string | undefined)[] = [];
      if (charBeforeWord === '$') candidates.push(`$${word.word}`);
      candidates.push(word.word);
      const lastUnderscore = word.word.lastIndexOf('_');
      if (lastUnderscore > 0) {
        candidates.push(`${word.word.slice(0, lastUnderscore)}_name`);
      }
      for (const key of candidates) {
        const found = key != null ? merged.get(key) ?? index.variables.get(key) : undefined;
        if (found) {
          return {
            contents: [{ value: found.documentation }],
            range: {
              startLineNumber: position.lineNumber,
              endLineNumber: position.lineNumber,
              startColumn: charBeforeWord === '$' ? word.startColumn - 1 : word.startColumn,
              endColumn: word.endColumn,
            },
          };
        }
      }
      return null;
    },
  });
}
