/**
 * systemd 单元文件语言支持：Monarch 高亮 + 节感知指令补全 + 枚举值补全 + hover。
 *
 * 与 nginxLanguage.ts / dockerfileLanguage.ts 同型（纯前端、纯离线、数据懒加载）。
 * 数据 `systemdData/directives.json` 按 systemd 官方 man 手册手工整理（事实性
 * 指令清单，来源与口径见 systemdData/README.md），经 dynamic import 打成独立
 * async chunk，不打开单元文件的会话零加载成本。
 *
 * 行为设计（行级扫描无 parser，惯例同 nginx 的 computeContexts）：
 * - 节上下文：光标行之前最近的 `[Section]` 标题（光标行自身的标题不并入）；
 * - 节标题补全：光标行是 `[XXX` 半成品标题时，替换范围盖住 `[`、insertText
 *   为完整 `[Unit]`；还没有任何节标题时顺带在指令候选里混入节标题（引导
 *   正确的文件结构）；
 * - 指令补全：仅光标处于本行首 token（前面只有空白）时给，按节过滤；
 *   值续行（上方行以 `\` 结尾且可追到指令行）时降级为该指令的枚举值补全；
 * - 枚举值补全：行内已有 `=` 且光标在值位时，按「节 + 指令」查 values 索引；
 * - insertText 一律纯文本（不设 insertTextRules——仓库踩过 `$`/`%` 被 snippet
 *   语法解析的坑，见 nginxLanguage.ts 同款注释）；
 * - hover：节标题词 → 节说明；指令名 → 说明 + 所属节（含跨节归属提示）；
 *   枚举值 → 值说明。
 *
 * 高亮：`[Section]` 节标题、`Key=` 指令名、`#`/`;` 注释、布尔/数字/路径值、
 * `%h %i %n …` 说明符（keyword 特殊色）、行尾 `\` 续行（值态跨行保持）。
 *
 * 本模块只做 type-only 的 monaco 导入（运行期实例由宿主传入）。
 */

import type * as MonacoNs from 'monaco-editor';

// ─── 数据形状（systemdData/directives.json，字段见该目录 README.md）───

interface RawSystemdData {
  /** 节名 → 节说明（节标题补全 / hover） */
  sections: Record<string, string>;
  /** 节名 → 指令名 → 一句话说明（节内指令补全 / hover） */
  directives: Record<string, Record<string, string>>;
  /** 节名 → 指令名 → 值 → 值说明（`=` 后的值补全 / 值 hover） */
  values: Record<string, Record<string, Record<string, string>>>;
}

interface SystemdIndex {
  sections: Map<string, string>;
  /** 节 → 指令 → 说明 */
  directives: Map<string, Map<string, string>>;
  /** 节 → 指令 → 值 → 说明 */
  values: Map<string, Map<string, Map<string, string>>>;
  /** 指令 → 出现的节列表（hover 跨节归属） */
  directiveOwners: Map<string, string[]>;
}

// ─── 节上下文（行级扫描：光标行之前最近的 [Section]）───

const SYSTEMD_SECTION_RE = /^[ \t]*\[([^\]]*)\]/;

/**
 * 光标行之前最近出现的节标题名（0 基行号；光标行自身的标题不并入，
 * 对齐 nginx computeContexts 的「本行符号取祖先链」语义）。
 */
export function currentSystemdSection(
  lines: readonly string[],
  cursorLine: number,
): string | null {
  let section: string | null = null;
  const limit = Math.min(cursorLine, lines.length);
  for (let i = 0; i < limit; i += 1) {
    const m = SYSTEMD_SECTION_RE.exec(lines[i] ?? '');
    if (m) section = m[1]!;
  }
  return section;
}

/** 指令名（首 token，后随 `=`） */
const SYSTEMD_KEY_RE = /^[ \t]*([A-Za-z][A-Za-z0-9_]*)[ \t]*=/;

/**
 * 值续行归因：光标行上方若以 `\` 结尾，沿续行链向上找指令行，返回其指令名
 * （补全据此降级为枚举值位）；不构成续行链时返回 null。
 */
function continuationDirective(lines: readonly string[], lineNo: number): string | null {
  let i = lineNo - 1;
  while (i >= 0) {
    const prev = (lines[i] ?? '').replace(/[ \t]+$/, '');
    if (!prev.endsWith('\\')) return null;
    const m = SYSTEMD_KEY_RE.exec(prev);
    if (m) return m[1]!;
    i -= 1;
  }
  return null;
}

// ─── 索引懒加载 ───

let dataIndex: SystemdIndex | null = null;
let dataIndexLoading: Promise<SystemdIndex> | null = null;

function buildIndex(raw: RawSystemdData): SystemdIndex {
  const sections = new Map(Object.entries(raw.sections));
  const directives = new Map<string, Map<string, string>>();
  const directiveOwners = new Map<string, string[]>();
  for (const [section, entries] of Object.entries(raw.directives)) {
    const bucket = new Map<string, string>(Object.entries(entries));
    directives.set(section, bucket);
    for (const name of bucket.keys()) {
      const owners = directiveOwners.get(name);
      if (owners) owners.push(section);
      else directiveOwners.set(name, [section]);
    }
  }
  const values = new Map<string, Map<string, Map<string, string>>>();
  for (const [section, perDirective] of Object.entries(raw.values)) {
    const bucket = new Map<string, Map<string, string>>();
    for (const [directive, valueMap] of Object.entries(perDirective)) {
      bucket.set(directive, new Map(Object.entries(valueMap)));
    }
    values.set(section, bucket);
  }
  return { sections, directives, values, directiveOwners };
}

function ensureDataIndex(): Promise<SystemdIndex> {
  if (dataIndex) return Promise.resolve(dataIndex);
  if (!dataIndexLoading) {
    dataIndexLoading = import('./systemdData/directives.json').then((mod) => {
      dataIndex = buildIndex(mod.default as unknown as RawSystemdData);
      return dataIndex;
    });
  }
  return dataIndexLoading;
}

// ─── Monarch 高亮 ───

const SYSTEMD_MONARCH_LANGUAGE: MonacoNs.languages.IMonarchLanguage = {
  defaultToken: '',
  tokenPostfix: '.systemd',
  tokenizer: {
    root: [
      [/^[ \t]*[;#].*$/, 'comment'],
      [/^[ \t]*\[[^\]\n]*\][ \t]*$/, 'type.identifier'],
      // 行首指令名（带 `=` lookahead 优先着色；未打完 `=` 的行退化为次条规则）
      [/^[ \t]*[A-Za-z][A-Za-z0-9_]*[ \t]*(?==)/, 'variable.name'],
      [/[=]/, 'delimiter', '@value'],
      [/^[ \t]*[A-Za-z][A-Za-z0-9_]*/, 'variable.name'],
      [/./, ''],
    ],
    // 值态：说明符/布尔/数字特殊着色，其余字符按值（绿）；行尾 `\` 保持值态
    // （systemd 值续行），未续行则在行尾弹回 root（零长 `$` 匹配 + @pop 的
    // 形态与 monaco 内置 shell Monarch 的 heredoc 收尾同款）
    value: [
      [/[ \t]*\\$/, 'string'],
      [/[ \t]*$/, '', '@pop'],
      [/%[%A-Za-z]/, 'keyword'],
      [/\b(true|false|yes|no|on|off)\b/, 'keyword'],
      [/\d+/, 'number'],
      [/[^\s]/, 'string'],
      [/\s/, ''],
    ],
  },
};

const SYSTEMD_LANGUAGE_CONFIGURATION: MonacoNs.languages.LanguageConfiguration = {
  comments: { lineComment: '#' },
};

// ─── 补全候选构造 ───

interface CompletionContext2 {
  monaco: typeof MonacoNs;
  position: MonacoNs.Position;
}

function lineRange(ctx: CompletionContext2, startCol: number): MonacoNs.IRange {
  return {
    startLineNumber: ctx.position.lineNumber,
    endLineNumber: ctx.position.lineNumber,
    startColumn: startCol,
    endColumn: ctx.position.column,
  };
}

/** 节标题候选：替换范围盖住已打的 `[`，insertText 为完整 `[Unit]` */
function sectionSuggestions(
  ctx: CompletionContext2,
  index: SystemdIndex,
  replace: MonacoNs.IRange,
): MonacoNs.languages.CompletionItem[] {
  const items: MonacoNs.languages.CompletionItem[] = [];
  for (const [name, desc] of index.sections) {
    items.push({
      label: `[${name}]`,
      kind: ctx.monaco.languages.CompletionItemKind.Module,
      detail: 'section',
      documentation: { value: desc },
      insertText: `[${name}]`,
      // 纯文本插入（见文件头注释）
      range: replace,
    });
  }
  return items;
}

function directiveSuggestions(
  ctx: CompletionContext2,
  index: SystemdIndex,
  section: string,
  replace: MonacoNs.IRange,
): MonacoNs.languages.CompletionItem[] {
  const items: MonacoNs.languages.CompletionItem[] = [];
  const bucket = index.directives.get(section);
  if (!bucket) return items;
  for (const [name, desc] of bucket) {
    items.push({
      label: name,
      kind: ctx.monaco.languages.CompletionItemKind.Property,
      detail: `${section} section`,
      documentation: { value: desc },
      insertText: name,
      range: replace,
    });
  }
  return items;
}

function valueSuggestions(
  ctx: CompletionContext2,
  index: SystemdIndex,
  section: string,
  directive: string,
  replace: MonacoNs.IRange,
): MonacoNs.languages.CompletionItem[] {
  const valueMap = index.values.get(section)?.get(directive);
  if (!valueMap) return [];
  const items: MonacoNs.languages.CompletionItem[] = [];
  for (const [value, desc] of valueMap) {
    items.push({
      label: value,
      kind: ctx.monaco.languages.CompletionItemKind.Value,
      detail: `${directive} value`,
      documentation: { value: desc },
      insertText: value,
      range: replace,
    });
  }
  return items;
}

// ─── 注册入口（幂等；monaco 实例由宿主传入）───

let registered = false;

export function ensureSystemdLanguage(monaco: typeof MonacoNs): void {
  if (registered) return;
  registered = true;

  monaco.languages.register({
    id: 'systemd',
    extensions: [
      '.service',
      '.socket',
      '.timer',
      '.target',
      '.mount',
      '.automount',
      '.path',
      '.slice',
      '.scope',
      '.device',
      '.swap',
    ],
    aliases: ['systemd unit'],
  });
  monaco.languages.setLanguageConfiguration('systemd', SYSTEMD_LANGUAGE_CONFIGURATION);
  monaco.languages.setMonarchTokensProvider('systemd', SYSTEMD_MONARCH_LANGUAGE);

  monaco.languages.registerCompletionItemProvider('systemd', {
    triggerCharacters: ['[', '='],
    async provideCompletionItems(model, position) {
      const index = await ensureDataIndex();
      const ctx: CompletionContext2 = { monaco, position };
      const lines = model.getValue().split('\n');
      const lineNo = position.lineNumber - 1;
      const before = model.getValueInRange({
        startLineNumber: position.lineNumber,
        startColumn: 1,
        endLineNumber: position.lineNumber,
        endColumn: position.column,
      });

      // ① 节标题位：行是 `[XXX` 半成品标题（或仅有 `[`）
      const bracketIdx = before.indexOf('[');
      if (bracketIdx >= 0 && /^\s*\[[^\]]*$/.test(before)) {
        return {
          suggestions: sectionSuggestions(ctx, index, lineRange(ctx, bracketIdx + 1)),
        };
      }

      const section = currentSystemdSection(lines, lineNo) ?? 'Unit';

      // ② 值位：本行已有 `=` 且光标在 `=` 之后
      const eq = before.indexOf('=');
      if (eq >= 0 && !/^[ \t]*[;#]/.test(before)) {
        const keyMatch = SYSTEMD_KEY_RE.exec(before);
        const key = keyMatch?.[1];
        if (!key) return { suggestions: [] };
        // 值 token 起点：`=` 后最后一个空白之后（未打值时光标即起点）
        const valPart = before.slice(eq + 1);
        const lastWs = valPart.match(/\s\S*$/);
        const startCol = lastWs ? before.length - lastWs[0].length + 2 : eq + 2;
        return {
          suggestions: valueSuggestions(ctx, index, section, key, lineRange(ctx, startCol)),
        };
      }

      const word = model.getWordUntilPosition(position);
      // ③ 续行降级：上方以 `\` 结尾时可追到指令行 → 枚举值位
      const contDirective = continuationDirective(lines, lineNo);
      if (contDirective) {
        return {
          suggestions: valueSuggestions(
            ctx,
            index,
            section,
            contDirective,
            lineRange(ctx, word.startColumn || position.column),
          ),
        };
      }

      // ④ 指令位：仅当光标处于本行首 token（word 之前只有空白）
      const beforeWord = before.slice(0, (word.startColumn || position.column) - 1);
      if (/^\s*$/.test(beforeWord)) {
        const replace: MonacoNs.IRange = {
          startLineNumber: position.lineNumber,
          endLineNumber: position.lineNumber,
          startColumn: word.startColumn || position.column,
          endColumn: word.endColumn || position.column,
        };
        const suggestions = directiveSuggestions(ctx, index, section, replace);
        // 文件还没有任何节标题时混入节标题候选（引导先写 [Unit]）
        if (!currentSystemdSection(lines, lineNo)) {
          suggestions.push(...sectionSuggestions(ctx, index, replace));
        }
        return { suggestions };
      }
      return { suggestions: [] };
    },
  });

  monaco.languages.registerHoverProvider('systemd', {
    async provideHover(model, position) {
      const index = await ensureDataIndex();
      const word = model.getWordAtPosition(position);
      if (!word) return null;
      const lineContent = model.getLineContent(position.lineNumber);
      const lines = model.getValue().split('\n');
      const lineNo = position.lineNumber - 1;
      const section = currentSystemdSection(lines, lineNo) ?? 'Unit';
      const range: MonacoNs.IRange = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      };

      // ① 节标题词（[Unit] 内的 Unit）
      const isHeaderLine = /^[ \t]*\[[^\]]*\][ \t]*$/.test(lineContent);
      if (isHeaderLine) {
        const desc = index.sections.get(word.word);
        if (desc) return { contents: [{ value: `**[${word.word}]**\n\n${desc}` }], range };
        return null;
      }

      // ② 值位：`=` 之后 → 枚举值说明
      const eq = lineContent.indexOf('=');
      const keyMatch = SYSTEMD_KEY_RE.exec(lineContent);
      if (keyMatch && eq >= 0 && position.column > eq + 1) {
        const valueDoc = index.values.get(section)?.get(keyMatch[1]!)?.get(word.word);
        if (valueDoc) {
          return {
            contents: [
              { value: `**${word.word}** (${keyMatch[1]}= value)\n\n${valueDoc}` },
            ],
            range,
          };
        }
        // 值位查不到枚举不再落回指令 hover
        return null;
      }

      // ③ 指令名：当前节优先，其次跨节归属
      const own = index.directives.get(section)?.get(word.word);
      if (own) {
        return {
          contents: [{ value: `**${word.word}** (${section} section)\n\n${own}` }],
          range,
        };
      }
      const owners = index.directiveOwners.get(word.word);
      if (owners?.length) {
        const desc = index.directives.get(owners[0]!)?.get(word.word) ?? '';
        const ownerNote =
          owners.length > 1
            ? `Valid in sections: ${owners.join(', ')}`
            : `Belongs to the [${owners[0]}] section`;
        return {
          contents: [{ value: `**${word.word}**\n\n${desc}\n\n${ownerNote}` }],
          range,
        };
      }
      return null;
    },
  });
}
