/**
 * 自建 Monaco 语言支持（nginx / dockerfile / docker-compose / batch / sql）
 * 的统一幂等注册入口，顺带一次性对齐内置 html 语言的 worker 侧格式化缺省
 * （htmlDefaults.format 缺省 insertSpaces=false 出 TAB，与编辑器空格缩进
 * 约定相悖）。各语言模块自身另有幂等与数据懒加载（独立 async chunk，不
 * 打开对应文件的会话零数据成本）；宿主（编辑器面板 / SFTP 预览 / diff
 * 弹窗）只需调用本函数一次。bash 补全因需宿主模型关联
 * （registerBashHostModel）不并入，仍由宿主各自调用。
 *
 * html/css 的补全/hover 等智能走 Monaco 内置的 html/css worker
 * （vscode-html/css-languageservice 引擎，接线见 monacoEnvironment.ts），
 * 不属于「自建语言」、无需在此注册。
 */

import type * as MonacoNs from 'monaco-editor';

import { ensureBatchLanguage } from './batchLanguage';
import { ensureComposeLanguage } from './composeLanguage';
import { ensureDockerfileLanguage } from './dockerfileLanguage';
import { ensureNginxLanguage } from './nginxLanguage';
import { ensureSqlLanguage } from './sqlLanguage';

let htmlDefaultsAligned = false;

function alignHtmlDefaults(monaco: typeof MonacoNs): void {
  if (htmlDefaultsAligned) return;
  htmlDefaultsAligned = true;
  // Monaco 0.57 起 htmlDefaults 从 languages.html 迁到顶层 monaco.html；
  // HTMLFormatConfiguration 字段必填，逐字段显式回填（current 来自缺省值，
  // 兜底仅满足类型）
  const current = monaco.html.htmlDefaults.options;
  const f: Partial<MonacoNs.html.HTMLFormatConfiguration> = current.format ?? {};
  monaco.html.htmlDefaults.setOptions({
    ...current,
    format: {
      tabSize: f.tabSize ?? 4,
      insertSpaces: true,
      wrapLineLength: f.wrapLineLength ?? 120,
      unformatted: f.unformatted ?? '',
      contentUnformatted: f.contentUnformatted ?? 'pre',
      indentInnerHtml: f.indentInnerHtml ?? false,
      preserveNewLines: f.preserveNewLines ?? true,
      maxPreserveNewLines: undefined,
      indentHandlebars: f.indentHandlebars ?? false,
      endWithNewline: f.endWithNewline ?? false,
      extraLiners: f.extraLiners ?? 'head, body, /html',
      wrapAttributes: f.wrapAttributes ?? 'auto',
    },
  });
}

export function ensureExtendedLanguages(monaco: typeof MonacoNs): void {
  ensureNginxLanguage(monaco);
  ensureDockerfileLanguage(monaco);
  ensureComposeLanguage(monaco);
  ensureBatchLanguage(monaco);
  ensureSqlLanguage(monaco);
  alignHtmlDefaults(monaco);
}
