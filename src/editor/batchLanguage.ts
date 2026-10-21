/**
 * Windows Batch 语言支持：语言注册 + Monarch 高亮 + 补全 + goto 标签定义/
 * 引用/重命名。
 *
 * 移植自 rech-editor-batch 0.0.26（MIT，数据来源/许可/升级见
 * ./batchData/README.md）。上游是 VS Code 扩展 + Node LSP，但其逻辑全部是
 * 正则/静态数据（无 parser），故整体以前端 TS 落地，零 Rust 改动、零进程。
 *
 * 移植对应关系：
 * - 片段补全 ← 上游 SnippetsGenerator（上游唯一的补全来源，31 条 Community 集）；
 * - Monarch 高亮 ← 上游 batchfile.tmLanguage.json 的人工移植子集（命令表
 *   316 条、:: 与 REM 注释、:label、goto/call 引用、%var%/!var!/%%i/%~f1、
 *   set 行变量名与赋值算子、字符串；TextMate 的嵌套与多行规则形态不照搬，
 *   变量优先于字符串着色是 Monarch 无嵌套状态下的取舍）；
 * - 定义跳转 ← 上游 LSP BatchDeclarationFinder（`^:label$` 精确匹配）；
 * - 引用/重命名 ← 上游 LSP BatchReferencesProvider（最小词长 3、双引号内
 *   忽略但 %var%/!var! 变量引用豁免的过滤语义原样保留）。
 *
 * 两类有意增强（上游没有，数据取自上游文件）：
 * ① 命令/内建关键字补全（上游只有片段 + word-based；词表取自其语法文件）；
 * ② `%` 触发的变量补全（扫描文档 set 声明）与 goto/call 后的标签补全
 *   （扫描文档 :label 行）——与 bash 的 `$` 模式 UX 一致。
 *
 * 偏离上游实现的两处（语义不变）：引用/定义的位置由正则索引直接计算，
 * 不移植上游 Scan 迭代器的 column+1 偏移修正；多词命令（net computer 等）
 * 只进补全候选、不进 Monarch 词表（Monarch cases 是整 token 精确匹配，
 * 'net' 单词条目已覆盖高亮）。
 *
 * 不移植：TabStopper 列停靠（劫持 Tab 的内部编辑约定）、Alt+N 插入注释行
 * （Monaco 内置注释命令已覆盖）、windows1252 编码默认（文件加载层范畴，
 * 见 batchData/README.md 已知边界）、ConflictingExtensionsChecker（VS Code
 * 专属）。
 *
 * 语言用自建 id `batch` 而非 Monaco 内置 `bat`：内置 bat 是懒加载 Monarch，
 * 覆盖高亮会与其打「后注册者赢」的时序战（nginx 自建 id 同因）。本模块
 * 只做 type-only 的 monaco 导入（运行期实例由宿主传入），数据经 dynamic
 * import 独立 chunk，不打开 bat 文件的会话零数据成本。
 */

import type * as MonacoNs from 'monaco-editor';

import type { BatchSnippet } from './batchData/snippets';

// ─── 静态数据懒加载 ───

interface BatchStaticData {
  snippets: readonly BatchSnippet[];
  commands: readonly string[];
  keywords: readonly string[];
  /** Monarch 词表属性（多词命令剔除——cases 是整 token 精确匹配） */
  monarchCommands: string[];
  /** 片段展示文档（描述 + body 代码块），一次生成 */
  snippetDocs: Map<string, string>;
}

let staticDataPromise: Promise<BatchStaticData> | null = null;

function ensureBatchData(): Promise<BatchStaticData> {
  if (!staticDataPromise) {
    staticDataPromise = Promise.all([
      import('./batchData/snippets'),
      import('./batchData/keywords'),
    ]).then(([snippets, keywords]) => {
      const all = snippets.BATCH_SNIPPETS;
      const snippetDocs = new Map<string, string>();
      for (const s of all) {
        snippetDocs.set(s.prefix, `${s.description}\n\n\`\`\`bat\n${s.body}\n\`\`\``);
      }
      return {
        snippets: all,
        commands: keywords.BATCH_COMMANDS,
        keywords: keywords.BATCH_KEYWORDS,
        monarchCommands: keywords.BATCH_COMMANDS.filter((c) => !c.includes(' ')),
        snippetDocs,
      };
    });
  }
  return staticDataPromise;
}

// ─── 文档扫描（补全的文档内数据源；纯函数便于验证） ───

/** goto/call 可跳的标签清单：行首 `:name` 行（名字不含空白与 + = , ; :） */
export function scanBatchLabels(text: string): string[] {
  const labels: string[] = [];
  const seen = new Set<string>();
  for (const line of text.split('\n')) {
    const m = /^[ \t]*:([^\s+=,;:][^\r\n]*)$/.exec(line);
    if (!m) continue;
    const name = m[1]!.trim();
    if (name && !seen.has(name)) {
      seen.add(name);
      labels.push(name);
    }
  }
  return labels;
}

/** set 声明的变量名清单（`set VAR=` / `set /a X+=` / `set "VAR=..."`，按首次出现排序） */
export function scanBatchVariables(text: string): string[] {
  const vars: string[] = [];
  const seen = new Set<string>();
  const re =
    /\bset\s+(?:\/[ap]\s+)?["']?([A-Za-z_][\w\-.]*)['"]?\s*(?:\+=|-=|\*=|\/=|%%=|&=|\|=|\^=|<<=|>>=|=)/gi;
  for (const m of text.matchAll(re)) {
    const name = m[1]!;
    if (!seen.has(name)) {
      seen.add(name);
      vars.push(name);
    }
  }
  return vars;
}

// ─── 引用查找（上游 BatchReferencesProvider 的移植；位置由正则索引直接计算） ───

/** 上游 MIN_WORD_SIZE：小于 3 字符的词不做引用（误报太多） */
const BATCH_MIN_WORD_SIZE = 3;

interface BatchReference {
  /** 0-based 行号 */
  line: number;
  /** 0-based 行内列（词首） */
  column: number;
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * 词级引用扫描：可选定界符包裹的 term 全文匹配；双引号内的命中忽略，
 * 但 `%term%` / `!term!` 变量引用豁免（引号内的变量是真实引用）。
 */
export function findBatchReferences(text: string, term: string): BatchReference[] {
  if (term.length < BATCH_MIN_WORD_SIZE) return [];
  const re = new RegExp(
    `([\\s.!%:,()])?(${escapeRegExp(term)})(?=[\\s\\t\\n\\r.!%:,=()]|$)`,
    'gi',
  );
  const lines = text.split('\n');
  const results: BatchReference[] = [];
  for (const m of text.matchAll(re)) {
    const lead = m[1] ?? '';
    const termStart = m.index + lead.length;
    const before = text.slice(0, termStart);
    const line = before.split('\n').length - 1;
    const column = termStart - (before.lastIndexOf('\n') + 1);
    const lineText = lines[line] ?? '';
    if (!shouldIgnoreReference(lineText, column, term)) {
      results.push({ line, column });
    }
  }
  return results;
}

function shouldIgnoreReference(lineText: string, termStart: number, term: string): boolean {
  const enclosed = isEnclosedInQuotes(lineText, termStart);
  const variable = isVariableReference(lineText, termStart, term);
  return enclosed && !variable;
}

function isEnclosedInQuotes(lineText: string, column: number): boolean {
  let insideQuotes = false;
  for (let i = 0; i < column && i < lineText.length; i += 1) {
    if (lineText[i] === '"') insideQuotes = !insideQuotes;
  }
  return insideQuotes;
}

function isVariableReference(lineText: string, termStart: number, term: string): boolean {
  if (termStart === 0 || termStart >= lineText.length) return false;
  return ['%', '!'].some(
    (sym) => lineText[termStart - 1] === sym && lineText[termStart + term.length] === sym,
  );
}

// ─── 语言配置 + Monarch（上游 bat.configuration.json + tmLanguage 的移植子集） ───

const BATCH_LANGUAGE_CONFIGURATION: MonacoNs.languages.LanguageConfiguration = {
  comments: {
    // 上游 lineComment 为 rem；:: 也是合法行注释但不作注释前缀（插入用 rem）
    lineComment: 'rem',
  },
  brackets: [['(', ')']],
  autoClosingPairs: [
    { open: '(', close: ')' },
    { open: '"', close: '"' },
    { open: "'", close: "'" },
    // 上游特色：成对的变量定界符（%var% / !var!）
    { open: '%', close: '%' },
    { open: '!', close: '!' },
  ],
  surroundingPairs: [
    { open: '{', close: '}' },
    { open: '[', close: ']' },
    { open: '(', close: ')' },
    { open: '"', close: '"' },
    { open: "'", close: "'" },
    { open: '%', close: '%' },
    { open: '!', close: '!' },
  ],
  // 上游 wordPattern（含 CP1252 重音字符）
  wordPattern:
    /[a-zA-Z0-9ÇéâäàçêëèïîÄôöòûùÖÜáíóúñÑÁÂÀãÃÊËÈÍÎÏÌÓÔÒõÕÚÛÙüÉì_\-]+/,
};

const BATCH_MONARCH_LANGUAGE: MonacoNs.languages.IMonarchLanguage = {
  defaultToken: '',
  ignoreCase: true,
  tokenPostfix: '.batch',
  brackets: [{ token: 'delimiter.parenthesis', open: '(', close: ')' }],
  // 命令/内建词表（Monarch cases 按整 token 精确匹配、ignoreCase 生效）；
  // 注册期由 ensureBatchLanguage 注入数据后整体挂载（见文件头懒加载说明）
  commands: [],
  keywords: [],
  tokenizer: {
    root: [
      // 行注释：:: 整行与 REM 整行（上游 comments 规则）
      [/^\s*::.*$/, 'comment'],
      [/^\s*rem(\s.*)?$/, 'comment'],
      // 行首标签 :name（名字首字符排除 + = , ; : ——与上游 labels 规则一致）
      [/^\s*:[^\s+=,;:][^\s]*/, 'metatag'],
      // goto/call 的 :label 引用
      [/\b(goto|call)(\s+)(:\S+)/, ['keyword', '', 'metatag']],
      // @ 前缀（@echo off）
      [/@/, 'annotation'],
      // for 循环变量 %%i 与位置参数 %1 / %~f1 / %*
      [/%%[a-zA-Z_](?![a-zA-Z0-9_])/, 'variable'],
      [/%(~[fdpnxsatz]+)?\d(?![0-9])/, 'variable'],
      [/%\*/, 'variable'],
      // 环境变量 %var% 与延迟展开 !var!（优先于字符串：Monarch 无嵌套，
      // 引号内变量保色、裸引号由字符串规则兜底）
      [/%[^%\n]+%/, 'variable'],
      [/![^!\n]+!/, 'variable'],
      // 双引号字符串（含不成对的裸 "）
      [/"([^"\n])*"?/, 'string'],
      // set 行：推入专门状态着色变量名与赋值算子（上游 command_set 的简化移植）
      [/^(\s*)(set)(?=[\s/])/, ['', { token: 'keyword', next: '@setline' }]],
      // 数字
      [/\d+/, 'number'],
      // 运算符与括号
      [/&&|\|\||[&|<>]/, 'operator'],
      [/==|<<=|>>=|\+=|-=|\*=|\/=|%%=|&=|\|=|\^=/, 'operator'],
      [/=/, 'operator'],
      [/[()]/, '@brackets'],
      [/[+\-*/^~,.!;:]/, 'delimiter'],
      // 词元：命令表/内建命中（cases 整 token 匹配，兜 @default 放行）
      [/[a-zA-Z_][\w\-]*/, { cases: { '@commands': 'keyword', '@keywords': 'keyword', '@default': '' } }],
      [/[ \t\r\n]+/, ''],
    ],
    // set VAR=value：变量名 → 赋值算子 → 弹回 root（值侧由 root 规则着色）
    setline: [
      [/(\s*)(\/[ap])(?=\s)/, ['', 'keyword']],
      [/([a-zA-Z_][\w\-.]*)/, 'variable.name'],
      [/(<<=|>>=|\+=|-=|\*=|\/=|%%=|&=|\|=|\^=|=)/, { token: 'operator', next: '@pop' }],
      [/%%[a-zA-Z_](?![a-zA-Z0-9_])/, 'variable'],
      [/%[^%\n]+%/, 'variable'],
      [/![^!\n]+!/, 'variable'],
      [/"([^"\n])*"?/, 'string'],
      [/\d+/, 'number'],
      [/[()]/, '@brackets'],
      [/[^\s]/, ''],
      [/\s/, ''],
    ],
  },
};

// ─── 注册入口（幂等；monaco 实例由宿主传入） ───

let registered = false;

export function ensureBatchLanguage(monaco: typeof MonacoNs): void {
  if (registered) return;
  registered = true;

  monaco.languages.register({
    id: 'batch',
    extensions: ['.bat', '.cmd'],
    aliases: ['Batch', 'bat'],
  });
  monaco.languages.setLanguageConfiguration('batch', BATCH_LANGUAGE_CONFIGURATION);

  // Monarch 词表来自懒加载数据 chunk：到达后注入属性并整体挂载（Monaco 会
  // 对既有模型重新着色；不打开 bat 文件的会话零数据成本）
  void ensureBatchData().then((data) => {
    BATCH_MONARCH_LANGUAGE.commands = data.monarchCommands;
    BATCH_MONARCH_LANGUAGE.keywords = [...data.keywords];
    monaco.languages.setMonarchTokensProvider('batch', BATCH_MONARCH_LANGUAGE);
  });

  registerBatchProviders(monaco);
}

// ─── providers（补全 / 定义 / 引用 / 重命名） ───

function registerBatchProviders(monaco: typeof MonacoNs): void {
  const kind = monaco.languages.CompletionItemKind;

  monaco.languages.registerCompletionItemProvider('batch', {
    triggerCharacters: ['%', ':'],
    async provideCompletionItems(model, position) {
      const data = await ensureBatchData();
      const lineContent = model.getLineContent(position.lineNumber);
      const word = model.getWordUntilPosition(position);
      const beforeWord =
        word.startColumn > 1 ? lineContent.slice(0, word.startColumn - 1) : '';

      const makeRange = (): MonacoNs.IRange => ({
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn || position.column,
        endColumn: word.endColumn || position.column,
      });

      // % 变量模式：词前单个 %（%% 是 for 循环变量，排除）——有意增强②
      if (/%$/.test(beforeWord) && !/%%$/.test(beforeWord)) {
        const range = makeRange();
        return {
          suggestions: scanBatchVariables(model.getValue()).map((name) => ({
            label: name,
            kind: kind.Variable,
            detail: 'variable',
            // 补右定界 %：替换词或光标处插入后成对闭合
            insertText: `${name}%`,
            range,
          })),
        };
      }

      // goto/call 标签模式：行内光标前是（链首的）goto/call——有意增强②
      if (/(?:^|&&|\|\||[&(])\s*@?\s*(?:goto|call)\s+:?$/.test(beforeWord)) {
        const range = makeRange();
        return {
          suggestions: scanBatchLabels(model.getValue()).map((name) => ({
            label: name,
            kind: kind.Reference,
            detail: 'label',
            insertText: name,
            range,
          })),
        };
      }

      // 普通模式：片段（上游 SnippetsGenerator）+ 命令/内建（有意增强①）
      const items: MonacoNs.languages.CompletionItem[] = [];
      const seen = new Set<string>();
      const push = (item: MonacoNs.languages.CompletionItem & { label: string }) => {
        if (seen.has(item.label)) return;
        seen.add(item.label);
        items.push(item);
      };

      const range = makeRange();
      // 命令位判定：行首 / @ 前缀 / 链接算子（&、&&、||、(）之后
      const trimmed = beforeWord.replace(/[ \t]+$/, '');
      const atCommandPosition = trimmed === '' || /(?:&&|\|\||&|\||\(|@)$/.test(trimmed);
      if (atCommandPosition) {
        for (const cmd of data.commands) {
          push({
            label: cmd,
            kind: kind.Function,
            detail: 'Windows command',
            insertText: cmd,
            range,
          });
        }
        for (const kw of data.keywords) {
          push({
            label: kw,
            kind: kind.Keyword,
            detail: 'cmd builtin',
            insertText: kw,
            range,
          });
        }
      }
      for (const s of data.snippets) {
        push({
          label: s.prefix,
          kind: kind.Snippet,
          detail: 'snippet',
          documentation: { value: data.snippetDocs.get(s.prefix) ?? '' },
          insertText: s.body,
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          range,
        });
      }
      return { suggestions: items };
    },
  });

  // 定义跳转：goto 目标标签（上游 BatchDeclarationFinder 的 ^:label$ 精确匹配）
  monaco.languages.registerDefinitionProvider('batch', {
    provideDefinition(model, position) {
      const word = model.getWordAtPosition(position);
      if (!word) return null;
      const text = model.getValue();
      const re = new RegExp(`^:${escapeRegExp(word.word)}$`, 'im');
      const m = re.exec(text);
      if (!m) return null;
      const line = m.index === 0 ? 1 : text.slice(0, m.index).split('\n').length;
      return {
        uri: model.uri,
        range: {
          startLineNumber: line,
          endLineNumber: line,
          startColumn: 1,
          endColumn: 1,
        },
      };
    },
  });

  monaco.languages.registerReferenceProvider('batch', {
    provideReferences(model, position) {
      const word = model.getWordAtPosition(position);
      if (!word) return [];
      return findBatchReferences(model.getValue(), word.word).map((ref) => ({
        uri: model.uri,
        range: {
          startLineNumber: ref.line + 1,
          endLineNumber: ref.line + 1,
          startColumn: ref.column + 1,
          endColumn: ref.column + 1,
        },
      }));
    },
  });

  monaco.languages.registerRenameProvider('batch', {
    provideRenameEdits(model, position, newName) {
      const word = model.getWordAtPosition(position);
      if (!word) return null;
      const refs = findBatchReferences(model.getValue(), word.word);
      return {
        edits: refs.map((ref) => ({
          resource: model.uri,
          versionId: undefined,
          textEdit: {
            range: {
              startLineNumber: ref.line + 1,
              endLineNumber: ref.line + 1,
              startColumn: ref.column + 1,
              endColumn: ref.column + 1 + word.word.length,
            },
            text: newName,
          },
        })),
      };
    },
  });
}
