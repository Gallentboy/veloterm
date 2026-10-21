/**
 * HTML 键入辅助：`>` 自动闭合标签、`</` 自动补结束标签名、`=` 自动补属性值引号。
 *
 * 行为等价移植自 VS Code html-language-features 扩展的 client/autoInsertion.ts
 * + 引擎 doTagComplete / doQuoteComplete（MIT，microsoft/vscode 与
 * microsoft/vscode-html-languageservice）——上游由客户端监听键入、经自定义
 * LSP 请求问引擎；Monaco 的 html worker 不暴露这两个方法，故语义以前端
 * 启发式复刻（无引擎 parser/scanner 的简化，见各函数注释）：
 *
 * - `>`：光标前的开标签（非 void、非自闭合、注释/doctype 除外）→ 追加
 *   `</tag>`，光标保持在标签内容起始处（上游 `$0</tag>` 同位）；
 * - `/`（即键入 `</`）：扫描全文标签栈，最内层未闭合标签 → 补标签名与 `>`
 *   （下一个字符已是 `>` 时只补名，对齐上游 EndTagOpen 分支）；
 * - `=`：开标签内的属性名后 → 插 `""` 并把光标置于引号内（上游
 *   attributeDefaultValue 缺省 doublequotes）。
 *
 * 与上游的差异：引擎的 token 级校验（属性值内 `>`、扫描器状态）退化为
 * 正则保守判定——宁可漏补不可错补；上游 100ms 防抖因 LSP 异步往返而设，
 * 本实现同步执行无此需求。
 *
 * 本模块只做 type-only 的 monaco 导入（编辑器实例由宿主传入）；启发式纯
 * 函数独立导出，便于 node strip-types 冒烟。挂接：attachHtmlAutoInsert(
 * editor)，仅对 languageId === 'html' 的模型生效。
 */

import type * as MonacoNs from 'monaco-editor';

/** 引擎 dataProvider 的 void 元素缺省清单（area…wbr） */
const VOID_ELEMENTS = new Set([
  'area',
  'base',
  'br',
  'col',
  'embed',
  'hr',
  'img',
  'input',
  'keygen',
  'link',
  'menuitem',
  'meta',
  'param',
  'source',
  'track',
  'wbr',
]);

// ─── 启发式纯函数（移植核心） ───

/**
 * `>` 触发：textBefore 以开标签收尾时返回 `</tag>` 插入体。
 * 保守判定：取最后一个 `<`（其后无 `>` —— 引号内含 `>` 的属性会因此漏补，
 * 可接受）；段必须整体是 `<name ...>` 形态且非 `</`/`<!` 开头、非 `/>` 收尾。
 */
export function htmlTagCloseSuggestion(textBefore: string): string | null {
  if (!textBefore.endsWith('>')) return null;
  const lt = textBefore.lastIndexOf('<');
  if (lt < 0) return null;
  const segment = textBefore.slice(lt);
  const m = /^<([a-zA-Z][\w:.-]*)([\s\S]*)$/.exec(segment);
  if (!m) return null; // </、<!、<? 形态（闭标签/注释/doctype/PI）
  if (/\/>$/.test(segment)) return null; // 自闭合 <br/>
  const tag = m[1]!;
  if (VOID_ELEMENTS.has(tag.toLowerCase())) return null;
  return `</${tag}>`;
}

/**
 * `/` 触发（键入 `</`）：全文标签栈扫描，返回最内层未闭合标签的补全体。
 * 栈扫描忽略注释与不配对的闭标签（容错畸形文档）；下一个字符已是 `>`
 * 时只补名（对齐上游）。nextChar 为光标处现有字符（无则 ''）。
 */
export function htmlEndTagSuggestion(
  textBefore: string,
  nextChar: string,
): string | null {
  if (!textBefore.endsWith('</')) return null;
  const body = textBefore.slice(0, -2);
  // 剥注释（未闭合的注释尾部内容不参与标签配对）
  const cleaned = body.replace(/<!--[\s\S]*?(?:-->|$)/g, '');
  const tagRe = /<(\/?)([a-zA-Z][\w:.-]*)([^>]*?)(\/?)>/g;
  const stack: string[] = [];
  for (const m of cleaned.matchAll(tagRe)) {
    const isClose = m[1] === '/';
    const name = m[2]!.toLowerCase();
    const selfClosed = m[4] === '/';
    if (!isClose && !selfClosed && !VOID_ELEMENTS.has(name)) {
      stack.push(name);
    } else if (isClose) {
      // 弹出最近同名（畸形文档：弹到匹配或放弃）
      for (let i = stack.length - 1; i >= 0; i -= 1) {
        if (stack[i] === name) {
          stack.length = i;
          break;
        }
      }
    }
  }
  const top = stack[stack.length - 1];
  if (!top) return null;
  return nextChar === '>' ? top : `${top}>`;
}

/**
 * `=` 触发：开标签内属性名后的等号 → 插入 `""`（光标置于引号内）。
 * 返回 null 表示不插。tag 内判定：最后一个 `<` 晚于最后一个 `>`；
 * 等号前必须是独立属性名（非标签名本身）；光标处已是引号/等号（已有值
 * 的形态）不插。
 */
export function htmlAttrQuoteSuggestion(
  textBefore: string,
  nextChar: string,
): string | null {
  if (!textBefore.endsWith('=')) return null;
  const lt = textBefore.lastIndexOf('<');
  if (lt < 0 || lt < textBefore.lastIndexOf('>')) return null; // 不在开标签内
  const m = /^<([a-zA-Z][\w:.-]*)([\s\S]+)$/.exec(textBefore.slice(lt));
  if (!m) return null;
  const attrZone = m[2]!;
  // 属性名（非标签名——attrZone 至少含 名字+空格+名= 两段）收在等号前
  if (!/(?:^|\s)([^\s<>=]+)\s*$/.test(attrZone.slice(0, -1))) return null;
  if (attrZone.length <= 1) return null; // `<div=`：等号贴标签名，是畸形而非属性
  if (nextChar === '"' || nextChar === "'" || nextChar === '=') return null;
  return '""';
}

// ─── 编辑器挂接 ───

/**
 * 把键入辅助挂到指定编辑器（幂等性由宿主控制：编辑器销毁即解绑）。
 * 仅对 languageId === 'html' 的模型生效；撤销/重做与模块自身写入不触发。
 */
export function attachHtmlAutoInsert(
  editor: MonacoNs.editor.IStandaloneCodeEditor,
): MonacoNs.IDisposable {
  let applying = false;
  return editor.onDidChangeModelContent((e) => {
    if (applying || e.isUndoing || e.isRedoing) return;
    const model = editor.getModel();
    if (!model || model.getLanguageId() !== 'html') return;
    const change = e.changes[e.changes.length - 1];
    if (!change || change.rangeLength !== 0 || change.text.length === 0) return;
    if (change.text.includes('\n')) return; // 上游 isSingleLine
    const lastChar = change.text[change.text.length - 1]!;
    if (lastChar !== '>' && lastChar !== '/' && lastChar !== '=') return;

    // 键入后的光标位（单行插入：起点列 + 文本长）
    const position = {
      lineNumber: change.range.startLineNumber,
      column: change.range.startColumn + change.text.length,
    };
    const offset = model.getOffsetAt(position);
    const full = model.getValue();
    const textBefore = full.slice(0, offset);
    const nextChar = full[offset] ?? '';

    let insert: string | null = null;
    // 光标落点相对插入体的偏移（`</tag>`：光标留在插入体之前；`""`：引号之间）
    let cursorBack: number;
    if (lastChar === '>') {
      insert = htmlTagCloseSuggestion(textBefore);
      cursorBack = insert === null ? 0 : insert.length;
    } else if (lastChar === '/') {
      insert = htmlEndTagSuggestion(textBefore, nextChar);
      cursorBack = 0; // `</tag>`：光标跟在补全体之后
    } else {
      insert = htmlAttrQuoteSuggestion(textBefore, nextChar);
      cursorBack = insert === null ? 0 : 1; // `""`：光标落在两个引号之间
    }
    if (insert === null) return;

    const insertPos = model.getPositionAt(offset);
    applying = true;
    try {
      const applied = editor.executeEdits('veloterm-html-auto-insert', [
        {
          range: {
            startLineNumber: insertPos.lineNumber,
            startColumn: insertPos.column,
            endLineNumber: insertPos.lineNumber,
            endColumn: insertPos.column,
          },
          text: insert,
        },
      ]);
      if (applied) {
        editor.setPosition({
          lineNumber: insertPos.lineNumber,
          column: insertPos.column + (insert.length - cursorBack),
        });
      }
    } finally {
      applying = false;
    }
  });
}
