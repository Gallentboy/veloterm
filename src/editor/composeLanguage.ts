/**
 * docker-compose 语言支持：schema 驱动的键/枚举/文件内依赖补全 + hover。
 *
 * 移植自 docker/docker-language-server（Go LSP，Apache-2.0）的
 * internal/compose/completion.go 语义，数据为其 vendored 的
 * compose-spec.json（draft-07，byte-copy 于 dockerData/，来源 commit 见该目录
 * README.md）。与 nginx 模块同型：纯前端、纯离线、数据懒加载独立 chunk。
 *
 * 移植范围与上游的差异：
 * - 路径构造：上游用 goccy YAML AST，这里用缩进回溯扫描（nginx computeContexts
 *   同型手法）。flow 风格（{a: b}）、锚点/别名、多文档 `---` 之后的内容降级为
 *   「不给补全」（安全失败）；块风格（真实 compose 文件的绝大多数）全覆盖。
 * - oneOf 消歧：上游按已存在兄弟键选分支，这里取唯一 object 分支、否则合并全部
 *   object 分支的 properties 并集（如 volumes 项的 bind/volume/tmpfs 变体属性
 *   全部给出）。
 * - insertText/detail 语义对齐上游（object/array 型键补 `key:
  `，
 *   boolean 型出 ${1|true,false|} 选择片段，detail 为类型列表）。
 * - 不移植（网络/文件系统依赖，SFTP 远端场景不成立）：Docker Hub 镜像 tag、
 *   目录路径、跨文件（extends.file / build.target 读兄弟文件）。
 * - 额外移植（上游同有、纯文件内）：depends_on/networks/configs/secrets 的
 *   已定义名字补全、extends.service、volume 项 source 的顶层卷名补全。
 *
 * 注册在内置 `yaml` 语言上（保留 Monaco 原生 YAML 高亮），provider 内按
 * 「文件名 compose 系 ∪ 结构嗅探（顶层 services 为 mapping 等）」门控并按
 * model+version 缓存——普通 yaml / .gitlab-ci.yml（services 是列表）不触发。
 */

import type * as MonacoNs from 'monaco-editor';

// ─── 最小 draft-07 schema 类型（本 schema 的 $ref 全为 #/definitions/ 内部引用）───

interface SchemaNode {
  $ref?: string;
  type?: string | string[];
  description?: string;
  properties?: Record<string, SchemaNode>;
  patternProperties?: Record<string, SchemaNode>;
  additionalProperties?: boolean | SchemaNode;
  items?: SchemaNode;
  oneOf?: SchemaNode[];
  anyOf?: SchemaNode[];
  enum?: unknown[];
  deprecated?: boolean;
  required?: string[];
}

interface ComposeSchemaRoot extends SchemaNode {
  definitions?: Record<string, SchemaNode>;
}

type DataIndex = { root: ComposeSchemaRoot };

let dataIndex: DataIndex | null = null;
let dataIndexLoading: Promise<DataIndex> | null = null;

function ensureDataIndex(): Promise<DataIndex> {
  if (dataIndex) return Promise.resolve(dataIndex);
  if (!dataIndexLoading) {
    dataIndexLoading = import('./dockerData/compose-spec.json').then((mod) => {
      const root = mod.default as unknown as ComposeSchemaRoot;
      dataIndex = { root };
      return dataIndex;
    });
  }
  return dataIndexLoading;
}

function deref(root: ComposeSchemaRoot, s: SchemaNode): SchemaNode {
  let cur = s;
  let guard = 0;
  while (cur.$ref && guard < 10) {
    const m = cur.$ref.match(/^#\/definitions\/(.+)$/);
    if (!m) break;
    cur = root.definitions?.[m[1]!] ?? {};
    guard += 1;
  }
  return cur;
}

/** 引用类型集合（对齐上游 referencedTypes：type 优先，oneOf 取各分支首类型） */
function typesOf(root: ComposeSchemaRoot, s: SchemaNode): string[] {
  if (s.type) return Array.isArray(s.type) ? s.type : [s.type];
  if (s.$ref) return typesOf(root, deref(root, s));
  const out: string[] = [];
  for (const b of s.oneOf ?? s.anyOf ?? []) {
    const t = typesOf(root, b);
    if (t[0]) out.push(t[0]);
  }
  return out;
}

/**
 * 容器化：deref + oneOf 收敛。唯一 object 分支直接取；多个 object 分支合并
 * properties 并集（有意偏离上游的按键消歧，见文件头）。无 object 分支（值
 * 联合，如 string|number）原样返回——lookupChild 自然失败。
 */
function collapseContainer(root: ComposeSchemaRoot, s: SchemaNode): SchemaNode {
  const d = deref(root, s);
  const branches = [...(d.oneOf ?? []), ...(d.anyOf ?? [])].map((b) => deref(root, b));
  if (branches.length === 0) return d;
  const objectBranches = branches.filter((b) => typesOf(root, b).includes('object'));
  if (objectBranches.length === 1) return objectBranches[0]!;
  if (objectBranches.length > 1) {
    const merged: Record<string, SchemaNode> = {};
    for (const b of objectBranches) Object.assign(merged, b.properties ?? {});
    return { type: 'object', properties: merged, description: d.description };
  }
  return d;
}

/** 从容器 schema 取子节点：properties 精确命中，否则 patternProperties 正则 */
function lookupChild(
  root: ComposeSchemaRoot,
  container: SchemaNode,
  key: string,
): SchemaNode | null {
  const c = collapseContainer(root, container);
  const exact = c.properties?.[key];
  if (exact) return exact;
  for (const [pat, sub] of Object.entries(c.patternProperties ?? {})) {
    try {
      if (new RegExp(pat).test(key)) return sub;
    } catch {
      // schema 带非 JS 兼容正则时按不命中处理
    }
  }
  return null;
}

/** 数组分支（oneOf 里挑 array 型，如 depends_on 的列表形态） */
function arrayBranch(root: ComposeSchemaRoot, s: SchemaNode): SchemaNode | null {
  const d = deref(root, s);
  if (typesOf(root, d).includes('array')) return d;
  for (const b of [...(d.oneOf ?? []), ...(d.anyOf ?? [])]) {
    const db = deref(root, b);
    if (typesOf(root, db).includes('array')) return db;
  }
  return null;
}

// ─── YAML 行解析（缩进 / `- ` 列表项 / 键:值 / 引号感知去注释）───

function stripYamlComment(line: string): string {
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
    if (ch === '#' && (i === 0 || /\s/.test(line.charAt(i - 1)))) {
      return line.slice(0, i);
    }
  }
  return line;
}

interface YamlLineInfo {
  /** 有效缩进（`- ` 折算进缩进） */
  indent: number;
  /** 本行以 `- ` 开头（列表项） */
  hasDash: boolean;
  /** 键名（非键行为 null；含 bare dash 行） */
  key: string | null;
  /** 冒号后为空 → 开块（mapping 或 sequence） */
  opensBlock: boolean;
  /** 行内标量值（开块行为 null） */
  scalarValue: string | null;
  /** 文档分隔行 --- / ... */
  docSep: boolean;
}

function parseYamlLine(line: string): YamlLineInfo {
  const code = stripYamlComment(line);
  const trimmedAll = code.trim();
  if (trimmedAll === '---' || trimmedAll === '...') {
    return { indent: 0, hasDash: false, key: null, opensBlock: false, scalarValue: null, docSep: true };
  }
  const indentMatch = code.match(/^[ \t]*/)!;
  let indent = indentMatch[0].length;
  let content = code.slice(indent);
  if (content.startsWith('\t')) content = content.slice(1); // tab 容错
  let hasDash = false;
  const dash = content.match(/^-(\s+|$)/);
  if (dash) {
    hasDash = true;
    indent += dash[0].length;
    content = content.slice(dash[0].length);
  }
  const m = content.match(/^("[^"]*"|'[^']*'|[^\s:#]+)\s*:(.*)$/);
  if (m) {
    let key = m[1]!;
    if ((key.startsWith('"') || key.startsWith("'")) && key.length >= 2) {
      key = key.slice(1, -1);
    }
    const value = m[2]!.trim();
    return {
      indent,
      hasDash,
      key,
      opensBlock: value === '',
      scalarValue: value === '' ? null : value,
      docSep: false,
    };
  }
  return {
    indent,
    hasDash,
    key: null,
    opensBlock: false,
    scalarValue: content.trim() === '' ? null : content.trim(),
    docSep: false,
  };
}

interface PathEntry {
  indent: number;
  key: string;
  /** 该键的块是序列（后续首个有效行是 `- ` 且更深） */
  opensArray: boolean;
}

function isBlankOrComment(line: string): boolean {
  const t = line.trim();
  return t === '' || t.startsWith('#');
}

/**
 * 构造光标行之前的祖先块键链（只收「冒号后为空」的开块键；标量键不入栈——
 * 有效 YAML 中标量键之后不可能有更深行）。`opensArray` 由后续首个有效行
 * 是否为更深缩进的 `- ` 决定。
 */
export function buildYamlPath(lines: readonly string[], uptoLine: number): PathEntry[] {
  const stack: PathEntry[] = [];
  const limit = Math.min(uptoLine, lines.length);
  for (let i = 0; i < limit; i += 1) {
    const info = parseYamlLine(lines[i]!);
    if (info.docSep) {
      stack.length = 0;
      continue;
    }
    if (info.key === null || !info.opensBlock) continue;
    while (stack.length > 0 && stack[stack.length - 1]!.indent >= info.indent) {
      stack.pop();
    }
    // 向后窥视首个有效行，判定开的块是否为序列（窗口限 50 行防大文件 O(n²)）
    let opensArray = false;
    const peekLimit = Math.min(lines.length, i + 50);
    for (let j = i + 1; j < peekLimit; j += 1) {
      const raw = lines[j]!;
      if (isBlankOrComment(raw)) continue;
      const peek = parseYamlLine(raw);
      const rawIndent = stripYamlComment(raw).match(/^[ \t]*/)![0].length;
      if (rawIndent > info.indent && peek.hasDash) opensArray = true;
      break;
    }
    stack.push({ indent: info.indent, key: info.key, opensArray });
  }
  return stack;
}

/** 沿祖先键链解析 schema；失败（未知键/类型不符）返回 null */
function walkPath(root: ComposeSchemaRoot, entries: readonly PathEntry[]): SchemaNode | null {
  let s: SchemaNode = root;
  for (const e of entries) {
    const child = lookupChild(root, s, e.key);
    if (!child) return null;
    s = child;
    if (e.opensArray) {
      const arr = arrayBranch(root, s);
      if (!arr?.items) return null;
      s = arr.items;
    }
  }
  return s;
}

/** 顶层某区块（services / networks / volumes / configs / secrets）下已定义的名字 */
function sectionNames(lines: readonly string[], section: string): string[] {
  const names: string[] = [];
  let inSection = false;
  // 名字只在区块的首个缩进层级（services 的直属子键）；更深层是属性不算名字
  let nameIndent = -1;
  for (const raw of lines) {
    if (isBlankOrComment(raw)) continue;
    const info = parseYamlLine(raw);
    const rawIndent = stripYamlComment(raw).match(/^[ \t]*/)![0].length;
    if (rawIndent === 0) {
      inSection = info.key === section && info.opensBlock;
      nameIndent = -1;
      continue;
    }
    if (inSection && info.key && !info.hasDash) {
      if (nameIndent === -1) nameIndent = info.indent;
      if (info.indent === nameIndent) names.push(info.key);
    }
  }
  return names;
}

/** 当前列表项内已出现的键（值模式下用于 volumes 项的 type 判定） */
function itemSiblingScalar(lines: readonly string[], lineNo: number, indent: number, key: string): string | null {
  for (let i = lineNo - 1; i >= 0; i -= 1) {
    const raw = lines[i]!;
    if (isBlankOrComment(raw)) continue;
    const info = parseYamlLine(raw);
    if (info.indent < indent) return null;
    if (info.indent === indent && info.key === key && info.scalarValue != null) {
      return info.scalarValue.replace(/^['"]|['"]$/g, '');
    }
  }
  return null;
}

// ─── compose 文件判定（文件名 ∪ 结构嗅探，按 model+version 缓存）───

const COMPOSE_NAME_RE = /^(docker-)?compose([.-].*)?\.ya?ml$/i;

export function isLikelyComposeContent(content: string): boolean {
  const lines = content.split('\n', 300);
  const topForms = new Map<string, string>();
  for (let i = 0; i < lines.length; i += 1) {
    const raw = lines[i]!;
    if (isBlankOrComment(raw)) continue;
    const info = parseYamlLine(raw);
    const rawIndent = stripYamlComment(raw).match(/^[ \t]*/)![0].length;
    if (rawIndent !== 0 || !info.key) continue;
    let form = 'scalar';
    if (info.opensBlock) {
      form = 'block';
      for (let j = i + 1; j < lines.length; j += 1) {
        if (isBlankOrComment(lines[j]!)) continue;
        const peek = parseYamlLine(lines[j]!);
        const pj = stripYamlComment(lines[j]!).match(/^[ \t]*/)![0].length;
        if (pj > 0 && peek.hasDash) form = 'list';
        break;
      }
    }
    topForms.set(info.key, form);
  }
  // gitlab-ci（stages/jobs 列表形态）、GH Actions（jobs）、k8s（apiVersion）排除
  if (topForms.has('stages') || topForms.has('jobs') || topForms.has('pages') || topForms.has('apiVersion')) {
    return false;
  }
  if (topForms.get('services') === 'block') return true;
  if (['volumes', 'networks', 'configs', 'secrets'].some((k) => topForms.get(k) === 'block')) {
    return true;
  }
  return false;
}

const sniffCache = new WeakMap<MonacoNs.editor.ITextModel, { version: number; ok: boolean }>();

function isComposeModel(model: MonacoNs.editor.ITextModel): boolean {
  const uriPath = model.uri?.path ?? '';
  const base = decodeURIComponent(uriPath.slice(uriPath.lastIndexOf('/') + 1));
  if (COMPOSE_NAME_RE.test(base)) return true;
  const cached = sniffCache.get(model);
  const version = model.getVersionId();
  if (cached && cached.version === version) return cached.ok;
  const ok = isLikelyComposeContent(model.getValue());
  sniffCache.set(model, { version, ok });
  return ok;
}

// ─── 补全项生成（insertText/detail 语义对齐上游 insertText/extractDetail）───

/** 数组项是 object 且带 required 时，首个 required 键随 `- ` 一并给出 */
function arrayItemRequiredKey(root: ComposeSchemaRoot, prop: SchemaNode): string | null {
  const arr = arrayBranch(root, prop);
  if (!arr?.items) return null;
  const item = collapseContainer(root, arr.items);
  if (typesOf(root, item).includes('object') && item.required?.length) {
    return item.required[0]!;
  }
  return null;
}

interface KeyItemOptions {
  effIndent: number;
  /** 光标行在替换词后已跟冒号 → 只插裸键名 */
  colonAlready: boolean;
  /** 替换范围 */
  range: MonacoNs.languages.CompletionItem['range'];
}

function makeKeyItems(
  monaco: typeof MonacoNs,
  root: ComposeSchemaRoot,
  container: SchemaNode,
  opts: KeyItemOptions,
): MonacoNs.languages.CompletionItem[] {
  const collapsed = collapseContainer(root, container);
  const spacing = ' '.repeat(opts.effIndent + 2);
  const kind = monaco.languages.CompletionItemKind.Property;
  const items: MonacoNs.languages.CompletionItem[] = [];
  for (const [name, prop] of Object.entries(collapsed.properties ?? {})) {
    const types = typesOf(root, prop);
    const detailTypes = [...types].sort().join(' or ');
    const deprecated = prop.deprecated === true;
    const docSource = prop.description ?? deref(root, prop).description ?? '';
    let insert: string;
    if (opts.colonAlready) {
      insert = name;
    } else if (types.length === 1 && types[0] === 'array') {
      const req = arrayItemRequiredKey(root, prop);
      insert = req != null ? `${name}:\n${spacing}- ${req}: ` : `${name}:\n${spacing}- `;
    } else if (types.includes('array') && types.includes('object')) {
      insert = `${name}:\n${spacing}`;
    } else if (types.length === 1 && types[0] === 'object') {
      insert = `${name}:\n${spacing}`;
    } else if (types.includes('boolean')) {
      insert = `${name}: \${1|true,false|}`;
    } else {
      insert = `${name}: `;
    }
    items.push({
      label: name,
      kind,
      detail: deprecated ? `${detailTypes} (deprecated)` : detailTypes,
      documentation: docSource ? { value: docSource } : undefined,
      insertText: insert,
      insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
      sortText: deprecated ? `1${name}` : `0${name}`,
      range: opts.range,
    });
  }
  return items;
}

// ─── 注册入口（幂等；monaco 实例由宿主传入）───

let registered = false;

export function ensureComposeLanguage(monaco: typeof MonacoNs): void {
  if (registered) return;
  registered = true;

  monaco.languages.registerCompletionItemProvider('yaml', {
    triggerCharacters: [':', '-'],
    async provideCompletionItems(model, position) {
      if (!isComposeModel(model)) return { suggestions: [] };
      const data = await ensureDataIndex();
      const root = data.root;
      const lines = model.getValue().split('\n');
      const lineNo = position.lineNumber - 1;
      const line = lines[lineNo] ?? '';
      const before = line.slice(0, position.column - 1);
      if (before.trimStart().startsWith('#')) return { suggestions: [] };

      // 光标行信息（dash 折算进缩进）；contentPrefix 里含冒号 → 值模式
      const code = stripYamlComment(before);
      let effIndent = code.match(/^[ \t]*/)![0].length;
      let content = code.slice(effIndent);
      const dash = content.match(/^-(\s+|$)/);
      if (dash) {
        effIndent += dash[0].length;
        content = content.slice(dash[0].length);
      }
      const word = model.getWordUntilPosition(position);
      const replace = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn || position.column,
        endColumn: word.endColumn || position.column,
      };
      const restOfLine = line.slice(word.endColumn > 0 ? word.endColumn - 1 : 0);
      const colonAlready = /^\s*:/.test(restOfLine);

      const stack = buildYamlPath(lines, lineNo);
      while (stack.length > 0 && stack[stack.length - 1]!.indent >= effIndent) {
        stack.pop();
      }
      const lastKey = stack.length > 0 ? stack[stack.length - 1]!.key : null;
      const inService =
        stack.length >= 2 && stack[0]!.key === 'services' && !stack[0]!.opensArray;

      // —— 文件内依赖名补全（键模式：容器是依赖集合；或列表项首词）——
      const depInsertSuffix = (name: string, asMappingKey: boolean) =>
        asMappingKey ? `${name}:` : name;
      if (!content.includes(':')) {
        const asMapping = !dash;
        if (inService && lastKey) {
          const self = stack[1]!.key;
          if (lastKey === 'depends_on') {
            return {
              suggestions: sectionNames(lines, 'services')
                .filter((n) => n !== self)
                .map((n) => depItem(monaco, depInsertSuffix(n, asMapping), n, 'service', replace)),
            };
          }
          if (lastKey === 'networks') {
            return {
              suggestions: sectionNames(lines, 'networks').map((n) =>
                depItem(monaco, depInsertSuffix(n, asMapping), n, 'network', replace),
              ),
            };
          }
          if (lastKey === 'configs' || lastKey === 'secrets') {
            return {
              suggestions: sectionNames(lines, lastKey).map((n) =>
                depItem(monaco, depInsertSuffix(n, asMapping), n, lastKey.slice(0, -1), replace),
              ),
            };
          }
        }
      }

      // —— 值模式：枚举 / boolean / extends.service / volume 项 source ——
      if (content.includes(':')) {
        const colonIdx = content.indexOf(':');
        const key = content.slice(0, colonIdx).trim().replace(/^['"]|['"]$/g, '');
        const container = walkPath(root, stack);
        if (container) {
          if (
            key === 'service' &&
            inService &&
            stack.length === 3 &&
            stack[2]!.key === 'extends'
          ) {
            const self = stack[1]!.key;
            return {
              suggestions: sectionNames(lines, 'services')
                .filter((n) => n !== self)
                .map((n) => depItem(monaco, n, n, 'service', replace)),
            };
          }
          if (
            key === 'source' &&
            inService &&
            stack.length === 3 &&
            stack[2]!.key === 'volumes' &&
            stack[2]!.opensArray
          ) {
            const type = itemSiblingScalar(lines, lineNo, effIndent, 'type');
            if (type === 'volume' || type === 'cluster') {
              return {
                suggestions: sectionNames(lines, 'volumes').map((n) =>
                  depItem(monaco, n, n, 'volume', replace),
                ),
              };
            }
          }
          const value = lookupChild(root, container, key);
          if (value) {
            const dv = deref(root, value);
            if (dv.enum?.length) {
              return {
                suggestions: dv.enum
                  .filter((v): v is string => typeof v === 'string')
                  .map((v) => ({
                    label: v,
                    kind: monaco.languages.CompletionItemKind.Value,
                    detail: typesOf(root, value).join(' or '),
                    documentation: dv.description ? { value: dv.description } : undefined,
                    insertText: v,
                    range: replace,
                  })),
              };
            }
            if (typesOf(root, value).includes('boolean')) {
              return {
                suggestions: ['true', 'false'].map((v) => ({
                  label: v,
                  kind: monaco.languages.CompletionItemKind.Value,
                  detail: 'boolean',
                  documentation: dv.description ? { value: dv.description } : undefined,
                  insertText: v,
                  range: replace,
                })),
              };
            }
          }
        }
        return { suggestions: [] };
      }

      // —— 键模式：schema properties（顶层空栈 → 根属性）——
      const container = walkPath(root, stack);
      if (!container) return { suggestions: [] };
      const items = makeKeyItems(monaco, root, container, {
        effIndent,
        colonAlready,
        range: replace,
      });
      return { suggestions: items };
    },
  });

  monaco.languages.registerHoverProvider('yaml', {
    async provideHover(model, position) {
      if (!isComposeModel(model)) return null;
      const data = await ensureDataIndex();
      const root = data.root;
      const lines = model.getValue().split('\n');
      const lineNo = position.lineNumber - 1;
      const word = model.getWordAtPosition(position);
      if (!word) return null;

      const info = parseYamlLine(lines[lineNo] ?? '');
      const stack = buildYamlPath(lines, lineNo);
      const container = walkPath(root, stack);
      if (!container) return null;

      const mkRange = (startCol: number, endCol: number) => ({
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: startCol,
        endColumn: endCol,
      });

      // 查找序：光标处词作为容器键（正在看的键）→ 本行键（值位 hover 看键的 schema）
      for (const key of [word.word, info.key ?? '']) {
        if (!key) continue;
        const node = lookupChild(root, container, key);
        if (!node) continue;
        const d = deref(root, node);
        const types = typesOf(root, node);
        const parts: string[] = [];
        if (d.description) parts.push(d.description);
        if (d.enum?.length) {
          const vals = d.enum.filter((v): v is string => typeof v === 'string');
          if (vals.length) parts.push(`Allowed values:\n${vals.map((v) => `- \`${v}\``).join('\n')}`);
        }
        if (types.length) parts.push(`**Type:** \`${[...types].sort().join(' or ')}\``);
        if (d.required?.length) parts.push(`**Required:** \`${d.required.join(', ')}\``);
        if (node.deprecated || d.deprecated) parts.push('**Deprecated**');
        parts.push('[Compose 文件参考](https://docs.docker.com/reference/compose-file/)');
        return {
          contents: [{ value: parts.join('\n\n') }],
          range: mkRange(word.startColumn, word.endColumn),
        };
      }
      return null;
    },
  });
}

function depItem(
  monaco: typeof MonacoNs,
  insertText: string,
  label: string,
  kindLabel: string,
  range: MonacoNs.languages.CompletionItem['range'],
): MonacoNs.languages.CompletionItem {
  return {
    label,
    kind: monaco.languages.CompletionItemKind.Reference,
    detail: kindLabel,
    insertText,
    range,
  };
}
