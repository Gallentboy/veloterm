/**
 * bash/shell 补全：静态词表 + Rust tree-sitter 文档符号 + 远端命令全集。
 *
 * 移植自 bash-language-server 5.7.0（MIT，数据来源见 ./bashData/README.md）
 * 的 server.ts::onCompletion 可内嵌子集，挂接在 Monaco 内置的 `shell` 语言上：
 *
 * - 保留字 / 内建 / 片段：静态词表（bashData，dynamic import 独立 chunk）；
 * - 文档符号（函数/变量，`$`/`${` 变量补全）：后端 `analyze_bash_document`
 *   （tree-sitter 下沉 Rust，src-tauri/src/bash_analysis.rs——移植与偏离记录
 *   见该文件头）；
 * - 远端命令全集（方案 B）：`sftp_bash_remote_commands` 在远端跑
 *   `bash -c 'compgen -c' | sort -u`（与远端 Tab 补全同源；编辑的都是
 *   SFTP 远端文件，本地 PATH 语义不成立）。失败/不可达静默降级为
 *   「无远端命令」，主补全（静态词表 + 文档符号）不受影响；
 * - 不移植（Node 子进程专属，或宿主 bash-completion 专属）：`man`/`help`
 *   文档补全、`-` 前缀命令选项补全、跨文件 sourcing / workspace 索引。
 *
 * 对齐上游 onCompletion 的行为：注释内不出补全（in_comment 由 Rust 侧
 * tree-sitter 判定，heredoc 正文里的 `#` 不算）；`$`/`${` 前缀只出变量
 * （替换范围不盖 `$`——bash 变量名不含 `$`，与 nginx 的 `$` 变量名内嵌
 * 前缀不同）；孤立 `{`（非 `${`）不触发；候选不过滤前缀（Monaco 自行按
 * 词前缀过滤）。
 *
 * 本模块只做 type-only 的 monaco 导入（运行期实例由宿主传入）。宿主模型
 * 需远端命令补全时调 registerBashHostModel 把模型与 hostId 关联（模型
 * dispose 自动解绑）。
 */

import type * as MonacoNs from 'monaco-editor';
import { invoke } from '@tauri-apps/api/core';

import type { BashSnippet } from './bashData/snippets';

// ─── 后端结果形状（bash_analysis.rs 的 serde camelCase 输出） ───

interface BashSymbolDto {
  name: string;
  /** 0-based 声明行 */
  line: number;
  /** 声明是否位于函数体内 */
  local: boolean;
}

interface BashAnalysisResultDto {
  inComment: boolean;
  variables: BashSymbolDto[];
  functions: BashSymbolDto[];
  commandName: string | null;
}

// ─── host 模型注册：模型 URI → hostId（远端命令补全的宿主定位） ───

const hostModelMap = new Map<string, number>();

/** 把 Monaco 模型关联到远端主机（幂等；模型 dispose 自动解绑）。 */
export function registerBashHostModel(
  model: MonacoNs.editor.ITextModel,
  hostId: number,
): void {
  const key = model.uri.toString();
  if (hostModelMap.has(key)) return;
  hostModelMap.set(key, hostId);
  model.onWillDispose(() => {
    hostModelMap.delete(key);
  });
}

// ─── 远端命令缓存：按 hostId 会话级缓存（上游 Executables 亦为 server
// 生命周期一次加载）；in-flight 去重防连打；失败短冷却后重试 ───

const REMOTE_COMMANDS_TTL_MS = 10 * 60 * 1000;
const REMOTE_COMMANDS_FAIL_RETRY_MS = 60 * 1000;

interface RemoteCommandsEntry {
  at: number;
  failed: boolean;
  promise: Promise<string[]>;
}

const remoteCommandsCache = new Map<number, RemoteCommandsEntry>();

function fetchRemoteCommands(hostId: number): Promise<string[]> {
  const cached = remoteCommandsCache.get(hostId);
  if (cached) {
    const ttl = cached.failed ? REMOTE_COMMANDS_FAIL_RETRY_MS : REMOTE_COMMANDS_TTL_MS;
    if (Date.now() - cached.at < ttl) return cached.promise;
  }
  const entry: RemoteCommandsEntry = {
    at: Date.now(),
    failed: false,
    promise: null as unknown as Promise<string[]>,
  };
  // 失败静默降级（方案 B 约定）：resolve 空数组，主补全不受影响
  entry.promise = invoke<string[]>('sftp_bash_remote_commands', {
    sessionId: String(hostId),
  })
    .then((list) => (Array.isArray(list) ? list : []))
    .catch(() => {
      entry.failed = true;
      return [];
    });
  remoteCommandsCache.set(hostId, entry);
  return entry.promise;
}

// ─── 静态词表懒加载（独立 async chunk，不开 shell 文件的会话零成本） ───

interface BashStaticData {
  reservedWords: readonly string[];
  builtins: readonly string[];
  snippets: readonly BashSnippet[];
  /** snippet 展示文档（用途 + insertText 的 bash 块），注册期一次生成 */
  snippetDocs: Map<string, string>;
}

let staticDataPromise: Promise<BashStaticData> | null = null;

function snippetDocumentation(snippet: BashSnippet): string {
  return `${snippet.documentation}\n\n\`\`\`bash\n${snippet.insertText}\n\`\`\``;
}

function ensureBashData(): Promise<BashStaticData> {
  if (!staticDataPromise) {
    staticDataPromise = Promise.all([
      import('./bashData/reservedWords'),
      import('./bashData/builtins'),
      import('./bashData/snippets'),
    ]).then(([reserved, builtins, snippets]) => {
      const all = snippets.BASH_SNIPPETS;
      const snippetDocs = new Map<string, string>();
      for (const s of all) snippetDocs.set(s.label, snippetDocumentation(s));
      return {
        reservedWords: reserved.BASH_RESERVED_WORDS,
        builtins: builtins.BASH_BUILTINS,
        snippets: all,
        snippetDocs,
      };
    });
  }
  return staticDataPromise;
}

// ─── 前端侧注释粗判（无 Tauri 环境 / invoke 失败时的兜底） ───

function isLikelyInComment(lineContent: string, wordStartColumn: number): boolean {
  // 行内裸 `#` 即注释起点（粗判够用，精确判定以 Rust 侧 tree-sitter
  // in_comment 为准——heredoc 正文里的 `#` 不是注释）
  return lineContent.slice(0, Math.max(wordStartColumn - 1, 0)).includes('#');
}

// ─── 注册入口（幂等；monaco 实例由宿主传入） ───

let registered = false;

export function ensureBashCompletion(monaco: typeof MonacoNs): void {
  if (registered) return;
  registered = true;

  monaco.languages.registerCompletionItemProvider('shell', {
    triggerCharacters: ['$'],
    async provideCompletionItems(model, position) {
      const version = model.getVersionId();
      const data = await ensureBashData();
      if (model.getVersionId() !== version) return { suggestions: [] };

      const word = model.getWordUntilPosition(position);
      const lineContent = model.getLineContent(position.lineNumber);
      // word 前的行内容：判定 `$` / `${` 变量前缀与孤立 `{`
      const beforeWord =
        word.startColumn > 1 ? lineContent.slice(0, word.startColumn - 1) : '';

      // 并行取文档符号与远端命令（互不阻塞；各自失败各自降级）
      const hostId = hostModelMap.get(model.uri.toString());
      const [analysis, remoteCommands] = await Promise.all([
        invoke<BashAnalysisResultDto>('analyze_bash_document', {
          content: model.getValue(),
          line: position.lineNumber - 1,
          column: position.column - 1,
        }).catch(() => null),
        hostId != null ? fetchRemoteCommands(hostId) : Promise.resolve([]),
      ]);
      if (model.getVersionId() !== version) return { suggestions: [] };

      // 注释熔断：Rust 侧 tree-sitter 判定优先，无结果时前端粗判兜底
      const inComment = analysis
        ? analysis.inComment
        : isLikelyInComment(lineContent, word.startColumn || position.column);
      if (inComment) return { suggestions: [] };

      // 替换范围：只覆盖 word 本身（bash 变量名不含 `$`，不前移盖 `$`）
      const range: MonacoNs.IRange = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn || position.column,
        endColumn: word.endColumn || position.column,
      };

      const kind = monaco.languages.CompletionItemKind;
      const items: MonacoNs.languages.CompletionItem[] = [];
      const seen = new Set<string>();
      // 本 provider 构造的候选 label 恒为 string（收窄掉联合类型便于去重）
      const push = (item: MonacoNs.languages.CompletionItem & { label: string }) => {
        if (seen.has(item.label)) return;
        seen.add(item.label);
        items.push(item);
      };

      // 变量模式：$ / ${ 前缀只出变量（对齐上游 PARAMETER_EXPANSION_PREFIXES）
      if (/\$\{?$/.test(beforeWord)) {
        for (const v of analysis?.variables ?? []) {
          push({
            label: v.name,
            kind: kind.Variable,
            detail: v.local ? 'local variable' : 'variable',
            insertText: v.name,
            range,
          });
        }
        return { suggestions: items };
      }

      // 孤立 `{`（非 `${`）不触发（对齐上游：`{` 是其触发字符但非补全位）
      if (/\{$/.test(beforeWord)) return { suggestions: [] };

      // 普通模式：保留字 ∪ 内建 ∪ 文档函数 ∪ 文档变量 ∪ 远端命令 ∪ 片段
      for (const label of data.reservedWords) {
        push({ label, kind: kind.Keyword, detail: 'reserved word', insertText: label, range });
      }
      for (const label of data.builtins) {
        push({ label, kind: kind.Function, detail: 'bash builtin', insertText: label, range });
      }
      for (const f of analysis?.functions ?? []) {
        push({
          label: f.name,
          kind: kind.Function,
          detail: 'function',
          insertText: f.name,
          range,
        });
      }
      for (const v of analysis?.variables ?? []) {
        push({
          label: v.name,
          kind: kind.Variable,
          detail: v.local ? 'local variable' : 'variable',
          insertText: v.name,
          range,
        });
      }
      for (const cmd of remoteCommands) {
        push({
          label: cmd,
          kind: kind.Function,
          detail: 'remote command',
          insertText: cmd,
          range,
        });
      }
      for (const s of data.snippets) {
        push({
          label: s.label,
          kind: kind.Snippet,
          detail: 'snippet',
          documentation: { value: data.snippetDocs.get(s.label) ?? '' },
          insertText: s.insertText,
          insertTextRules:
            monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          range,
        });
      }
      return { suggestions: items };
    },
  });
}
