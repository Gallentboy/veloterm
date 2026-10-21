/**
 * Dockerfile 语言支持：指令 / flag / 构建阶段 / 环境变量补全 + hover。
 *
 * 与 nginxLanguage.ts 同型（纯前端、纯离线、数据懒加载）。与 nginx 的差异在
 * 数据来源：docker-language-server（Go LSP，Apache-2.0）对 Dockerfile **没有
 * 补全能力**（其 completion 分发只覆盖 Compose 与 Bake），本模块的数据集
 * `dockerData/dockerfile.json` 按官方 Dockerfile 参考（moby/buildkit
 * frontend/dockerfile/docs/reference.md，docs.docker.com 的上游源）手工整理，
 * 来源与许可记录见 dockerData/README.md。
 *
 * 行为设计（对齐官方语义的轻量行级解析，无 AST）：
 * - 指令名补全：仅当光标处于逻辑行首 token（续行行不给）；
 * - flag 补全：光标 token 以 `--` 开头时按管辖指令过滤（`--from=` 出先前
 *   阶段名与序号、`--mount=type=` 等出枚举值）；
 * - `$` 变量补全：token 内含 `$` 时出先前 ARG/ENV 定义 ∪ 预定义 build ARG
 *   ∪ 代理变量，仅对官方声明做环境替换的指令（FROM/RUN/ADD/COPY/ENV/
 *   EXPOSE/LABEL/STOPSIGNAL/USER/VOLUME/WORKDIR）生效；
 * - 定义收集只取光标之前的行（ARG/ENV 声明后才可用、stage 先声明后可引用）；
 * - 转义符尊重 `# escape=` parser 指令；续行（尾随转义符）与 heredoc
 *   （`<<EOF` 到终止行）内的行不参与解析、heredoc 体内不给补全。
 *
 * 高亮复用 Monaco 内置 `dockerfile` 语言（全量入口自带），本模块只挂
 * provider。数据经 dynamic import 打成独立 async chunk。
 */

import type * as MonacoNs from 'monaco-editor';

// ─── 数据形状（dockerData/dockerfile.json，字段见该目录 README.md）───

interface DockerfileFlag {
  name: string;
  summary: string;
  valueHint?: string;
  values?: string[];
  /** `--from`：值位补先前构建阶段名 */
  stageRef?: boolean;
}

interface DockerfileInstruction {
  name: string;
  summary: string;
  docUrl: string;
  snippet?: string;
  flags?: DockerfileFlag[];
  /** FROM：声明构建阶段（AS name） */
  stageDecl?: boolean;
  /** ARG / ENV：定义变量 */
  definesVar?: 'arg' | 'env';
  /** 官方声明做环境替换的指令 */
  varExpansion?: boolean;
  deprecated?: boolean;
}

interface NamedVar {
  name: string;
  summary: string;
}

interface DockerfileData {
  parserDirectives: { insert: string; summary: string }[];
  predefinedArgs: NamedVar[];
  proxyVars: NamedVar[];
  instructions: DockerfileInstruction[];
}

// ─── 行级解析（引号感知去注释 / 转义符 / 续行 / heredoc）───

function stripDockerfileComment(line: string): string {
  let quote: string | null = null;
  for (let i = 0; i < line.length; i += 1) {
    const ch = line.charAt(i);
    if (quote) {
      if (ch === quote) quote = null;
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
      continue;
    }
    // 官方语义：# 前是行首或空白才构成注释（token 内的 # 不算）
    if (ch === '#' && (i === 0 || /\s/.test(line.charAt(i - 1)))) {
      return line.slice(0, i);
    }
  }
  return line;
}

/** `# escape=` parser 指令（只认首个指令行之前的注释块），缺省 `\` */
function detectEscapeChar(lines: readonly string[]): string {
  for (const raw of lines) {
    const trimmed = raw.trim();
    if (!trimmed) continue;
    if (!trimmed.startsWith('#')) break; // 首个指令行之后不再找（官方要求在任何指令前）
    const m = trimmed.match(/^#\s*escape\s*=\s*(\S)/);
    if (m) return m[1]!;
  }
  return '\\';
}

interface ScanState {
  escapeChar: string;
  /** 光标前出现的 FROM 次数（--from 序号候选上限） */
  fromCount: number;
  stages: string[];
  vars: Map<string, 'arg' | 'env'>;
  /** 当前逻辑行的指令名（大写；逻辑行开始时更新，续行沿用） */
  currentInstr: string | null;
  /** 扫描结束时是否在等续行（下一行属于当前逻辑行） */
  pendingCont: boolean;
  /** 打开中的 heredoc 终止符 */
  heredocs: string[];
}

/**
 * 扫描 [0, uptoLine) 行，产出光标前的文档状态。heredoc 体不参与任何解析；
 * 续行只更新续行状态。注释行不结束也不开启逻辑行。
 */
function scanDockerfile(lines: readonly string[], uptoLine: number): ScanState {
  const escapeChar = detectEscapeChar(lines);
  const state: ScanState = {
    escapeChar,
    fromCount: 0,
    stages: [],
    vars: new Map(),
    currentInstr: null,
    pendingCont: false,
    heredocs: [],
  };
  const limit = Math.min(uptoLine, lines.length);
  for (let i = 0; i < limit; i += 1) {
    const raw = lines[i]!;
    if (state.heredocs.length > 0) {
      const idx = state.heredocs.indexOf(raw.trim());
      if (idx >= 0) state.heredocs.splice(idx, 1);
      continue;
    }
    const code = stripDockerfileComment(raw);
    const trimmed = code.trim();
    if (!trimmed) continue;
    if (trimmed.startsWith('#')) continue;
    if (state.pendingCont) {
      state.pendingCont = trimmed.endsWith(escapeChar);
      continue;
    }
    const m = trimmed.match(/^([A-Za-z]+)(\s|$)/);
    state.currentInstr = m ? m[1]!.toUpperCase() : null;
    const rest = m ? trimmed.slice(m[1]!.length).trim() : '';
    if (state.currentInstr === 'FROM') {
      state.fromCount += 1;
      const stage = rest.match(/(?:^|\s)[Aa][Ss]\s+([^\s#]+)/);
      if (stage) state.stages.push(stage[1]!);
    } else if (state.currentInstr === 'ARG') {
      for (const tok of rest.split(/\s+/)) {
        if (!tok || tok.startsWith('--')) continue;
        const name = tok.split('=')[0]!;
        if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) state.vars.set(name, 'arg');
      }
    } else if (state.currentInstr === 'ENV') {
      if (rest.includes('=')) {
        // k=v 形式（可能一行多对）
        for (const tok of rest.split(/\s+(?=[A-Za-z_][A-Za-z0-9_]*=)/)) {
          const name = tok.split('=')[0]!;
          if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) state.vars.set(name, 'env');
        }
      } else {
        // 旧式空格分隔 `ENV key value`：只有第一个 token 是变量名
        const name = rest.split(/\s+/)[0] ?? '';
        if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) state.vars.set(name, 'env');
      }
    }
    const opensHeredoc =
      state.currentInstr === 'RUN' ||
      state.currentInstr === 'COPY' ||
      state.currentInstr === 'ADD';
    if (opensHeredoc) {
      for (const hm of trimmed.matchAll(/<<-?([A-Za-z0-9_.-]+)/g)) {
        state.heredocs.push(hm[1]!);
      }
      if (state.heredocs.length > 0) continue; // heredoc 打开时续接由终止行决定
    }
    state.pendingCont = trimmed.endsWith(escapeChar);
  }
  return state;
}

/** 光标行的逻辑归类（供补全/hover 判定管辖指令） */
export interface DockerfileCursorContext {
  /** 光标在 heredoc 字面量体内 */
  inHeredoc: boolean;
  /** 光标行是上一逻辑行的续行 */
  isContinuation: boolean;
  /** 管辖指令（大写名）；非续行时取光标行自身首 token（可能正在输入、可为 null） */
  instr: string | null;
}

export function cursorContext(lines: readonly string[], lineNo: number): DockerfileCursorContext {
  const state = scanDockerfile(lines, lineNo);
  if (state.heredocs.length > 0) {
    return { inHeredoc: true, isContinuation: true, instr: state.currentInstr };
  }
  if (state.pendingCont) {
    return { inHeredoc: false, isContinuation: true, instr: state.currentInstr };
  }
  const code = stripDockerfileComment(lines[lineNo] ?? '');
  const trimmed = code.trim();
  const m = trimmed.match(/^([A-Za-z]+)/);
  return { inHeredoc: false, isContinuation: false, instr: m ? m[1]!.toUpperCase() : null };
}

// ─── 数据懒加载 ───

let dataIndex: DockerfileData | null = null;
let dataIndexLoading: Promise<DockerfileData> | null = null;

function ensureDataIndex(): Promise<DockerfileData> {
  if (dataIndex) return Promise.resolve(dataIndex);
  if (!dataIndexLoading) {
    dataIndexLoading = import('./dockerData/dockerfile.json').then((mod) => {
      dataIndex = mod.default as unknown as DockerfileData;
      return dataIndex;
    });
  }
  return dataIndexLoading;
}

// ─── 注册入口（幂等；monaco 实例由宿主传入）───

let registered = false;

export function ensureDockerfileLanguage(monaco: typeof MonacoNs): void {
  if (registered) return;
  registered = true;

  monaco.languages.registerCompletionItemProvider('dockerfile', {
    triggerCharacters: ['-', '$', ' ', '='],
    async provideCompletionItems(model, position) {
      const data = await ensureDataIndex();
      const lines = model.getValue().split('\n');
      const lineNo = position.lineNumber - 1;
      const line = lines[lineNo] ?? '';
      const before = line.slice(0, position.column - 1);
      if (before.includes('\t')) return { suggestions: [] };
      const state = scanDockerfile(lines, lineNo);
      if (state.heredocs.length > 0) return { suggestions: [] };

      const codeBefore = stripDockerfileComment(before);
      const trimmedCode = codeBefore.trimStart();
      const indentLen = codeBefore.length - trimmedCode.length;

      // ① 注释行：仅文件头部指令区之前给 parser 指令，其余注释不给任何补全。
      // 门控用原始 before（codeBefore 已被去注释，检测不到注释）
      const trimmedBefore = before.trimStart();
      if (trimmedBefore.startsWith('#')) {
        if (!state.currentInstr) {
          const afterHash = before.slice(before.indexOf('#') + 1);
          if (/^\s*\S*$/.test(afterHash)) {
            const hashCol = before.indexOf('#') + 1;
            return {
              suggestions: data.parserDirectives.map((d) => ({
                label: d.insert,
                kind: monaco.languages.CompletionItemKind.Snippet,
                detail: 'directive',
                documentation: { value: d.summary },
                insertText: d.insert,
                range: {
                  startLineNumber: position.lineNumber,
                  endLineNumber: position.lineNumber,
                  startColumn: hashCol,
                  endColumn: position.column,
                },
              })),
            };
          }
        }
        return { suggestions: [] };
      }

      // 光标所在 token（空白分词；Dockerfile 无引号边界需求，token 级替换）
      const tokenStartIdx = codeBefore.search(/\s\S*$/) + 1;
      const token = codeBefore.slice(tokenStartIdx);
      const tokenStartCol = indentLen === codeBefore.length ? position.column : tokenStartIdx + 1;

      const ctx = cursorContext(lines, lineNo);

      // ② flag 补全：token 以 -- 开头（含 `--flag=值` 前缀）
      if (token.startsWith('--')) {
        const instr = data.instructions.find((i) => i.name === ctx.instr);
        if (!instr?.flags) return { suggestions: [] };
        const eq = token.indexOf('=');
        const replace = {
          startLineNumber: position.lineNumber,
          endLineNumber: position.lineNumber,
          startColumn: tokenStartCol,
          endColumn: position.column,
        };
        const doc = (summary: string, hint?: string) =>
          hint ? { value: `${summary}\n\n\`${hint}\`` } : { value: summary };
        if (eq > 0) {
          const flagName = token.slice(0, eq);
          const flag = instr.flags.find((f) => f.name === flagName);
          if (!flag) return { suggestions: [] };
          // 值位：--from= 出阶段名/序号；带 values 的出枚举。
          // 只在首个值段（无 '=' 或恰在 type= 段上）给枚举，
          // 避免 `--mount=type=cache,target=` 整段被候选覆盖
          const valPart = token.slice(eq + 1);
          const firstSegment = !valPart.includes('=') || /^type=?$/.test(valPart);
          if (flag.stageRef) {
            const items: MonacoNs.languages.CompletionItem[] = [];
            for (const stage of state.stages) {
              items.push({
                label: `${flag.name}=${stage}`,
                kind: monaco.languages.CompletionItemKind.Reference,
                detail: 'stage',
                insertText: `${flag.name}=${stage}`,
                range: replace,
              });
            }
            for (let n = 0; n < state.fromCount - 1; n += 1) {
              items.push({
                label: `${flag.name}=${n}`,
                kind: monaco.languages.CompletionItemKind.Reference,
                detail: 'stage index',
                insertText: `${flag.name}=${n}`,
                range: replace,
              });
            }
            return { suggestions: items };
          }
          if (flag.values && firstSegment) {
            return {
              suggestions: flag.values.map((v) => ({
                label: `${flag.name}=${v}`,
                kind: monaco.languages.CompletionItemKind.Value,
                detail: flag.valueHint,
                documentation: doc(flag.summary, flag.valueHint),
                insertText: `${flag.name}=${v}`,
                range: replace,
              })),
            };
          }
          return { suggestions: [] };
        }
        return {
          suggestions: instr.flags
            .filter((f) => f.name.startsWith(token))
            .map((f) => ({
              label: f.name,
              kind: monaco.languages.CompletionItemKind.Field,
              detail: f.valueHint,
              documentation: doc(f.summary, f.valueHint),
              insertText: f.name,
              range: replace,
            })),
        };
      }

      // ③ 指令名补全：光标处于本逻辑行首 token（非续行、冒号/空白前的代码无空白）
      const firstTokenZone =
        !ctx.isContinuation && !/\s/.test(codeBefore.slice(indentLen));
      if (firstTokenZone) {
        const prefix = trimmedCode.toUpperCase();
        return {
          suggestions: data.instructions
            .filter((i) => i.name.startsWith(prefix))
            .map((i) => ({
              label: i.name,
              kind: monaco.languages.CompletionItemKind.Keyword,
              detail: i.deprecated ? 'instruction (deprecated)' : 'instruction',
              documentation: {
                value: `${i.summary}\n\n[官方参考](${i.docUrl})`,
              },
              insertText: i.snippet ?? i.name,
              insertTextRules:
                monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
              range: {
                startLineNumber: position.lineNumber,
                endLineNumber: position.lineNumber,
                startColumn: indentLen + 1,
                endColumn: position.column,
              },
            })),
        };
      }

      // ④ `$` 变量补全：token 内含 $，且管辖指令做环境替换
      const dollar = token.lastIndexOf('$');
      if (dollar >= 0 && ctx.instr) {
        const instr = data.instructions.find((i) => i.name === ctx.instr);
        if (!instr?.varExpansion) return { suggestions: [] };
        const after = token.slice(dollar + 1);
        const brace = after.startsWith('{');
        const varPrefix = (brace ? after.slice(1) : after).toUpperCase();
        const match = (name: string) => name.toUpperCase().startsWith(varPrefix);
        const insert = (name: string) => (brace ? `${name}}` : name);
        const items: MonacoNs.languages.CompletionItem[] = [];
        const kind = monaco.languages.CompletionItemKind.Variable;
        for (const [name, kindOfVar] of state.vars) {
          if (!match(name)) continue;
          items.push({
            label: `$${name}`,
            kind,
            detail: kindOfVar === 'arg' ? 'ARG' : 'ENV',
            insertText: insert(name),
            range: {
              startLineNumber: position.lineNumber,
              endLineNumber: position.lineNumber,
              startColumn: tokenStartCol + dollar + (brace ? 2 : 1),
              endColumn: position.column,
            },
          });
        }
        for (const v of [...data.predefinedArgs, ...data.proxyVars]) {
          if (!match(v.name)) continue;
          items.push({
            label: `$${v.name}`,
            kind,
            detail: 'predefined',
            documentation: { value: v.summary },
            insertText: insert(v.name),
            range: {
              startLineNumber: position.lineNumber,
              endLineNumber: position.lineNumber,
              startColumn: tokenStartCol + dollar + (brace ? 2 : 1),
              endColumn: position.column,
            },
          });
        }
        return { suggestions: items };
      }

      // ⑤ 其余（镜像名等值位）：离线无候选
      return { suggestions: [] };
    },
  });

  monaco.languages.registerHoverProvider('dockerfile', {
    async provideHover(model, position) {
      const data = await ensureDataIndex();
      const lines = model.getValue().split('\n');
      const lineNo = position.lineNumber - 1;
      const line = lines[lineNo] ?? '';
      const ctx = cursorContext(lines, lineNo);
      if (ctx.inHeredoc) return null;
      const code = stripDockerfileComment(line);
      // 光标处完整 token（跨光标两侧取空白界）
      const beforePart = code.slice(0, position.column - 1);
      const afterPart = code.slice(position.column - 1);
      const startIdx = beforePart.search(/\s\S*$/) + 1;
      const endRel = afterPart.search(/\s/);
      const token = code.slice(
        startIdx,
        endRel >= 0 ? position.column - 1 + endRel : undefined,
      );

      const mkRange = (startCol: number, endCol: number) => ({
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: startCol,
        endColumn: endCol,
      });

      if (token.startsWith('--') && ctx.instr) {
        const instr = data.instructions.find((i) => i.name === ctx.instr);
        // token 可能带 `=值` 尾巴（--from=x），按 = 截断后查 flag
        const flagName = token.split('=')[0]!;
        const flag = instr?.flags?.find((f) => f.name === flagName);
        if (flag) {
          const body = flag.valueHint
            ? `${flag.summary}\n\n\`${flag.valueHint}\``
            : flag.summary;
          return {
            contents: [{ value: body }],
            range: mkRange(startIdx + 1, startIdx + 1 + token.length),
          };
        }
        return null;
      }
      const instr = data.instructions.find((i) => i.name === token.toUpperCase());
      if (instr) {
        const parts = [instr.summary];
        if (instr.flags?.length) {
          parts.push(
            instr.flags.map((f) => `- \`${f.name}\` — ${f.summary}`).join('\n'),
          );
        }
        parts.push(`[官方参考](${instr.docUrl})`);
        return {
          contents: [{ value: parts.join('\n\n') }],
          range: mkRange(startIdx + 1, startIdx + 1 + token.length),
        };
      }
      // 变量 hover（管辖指令做环境替换时）
      if (token.includes('$') && ctx.instr) {
        const governing = data.instructions.find((i) => i.name === ctx.instr);
        if (!governing?.varExpansion) return null;
        const name = token.replace(/^[$]+/, '').replace(/[}]$/, '');
        const scan = scanDockerfile(lines, lineNo);
        const summary = scan.vars.has(name)
          ? scan.vars.get(name) === 'arg'
            ? 'ARG 变量'
            : 'ENV 变量'
          : [...data.predefinedArgs, ...data.proxyVars].find((v) => v.name === name)
              ?.summary;
        if (summary) {
          return {
            contents: [{ value: summary }],
            range: mkRange(startIdx + 1, startIdx + 1 + token.length),
          };
        }
      }
      return null;
    },
  });
}
