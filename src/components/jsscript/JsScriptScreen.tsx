/**
 * JS 脚本页：内嵌 V8（rusty_v8）运行时的脚本工作台。
 * 上半 Monaco（javascript，⌘/Ctrl+Enter 运行），下半输出面板——console
 * 逐行流式回传（log/warn/error 三色），终态显示返回值 JSON 或错误横幅
 * （超时/内存超限/脚本错误/已取消）。超时可选（5s..10min），运行中可取消
 * （后端 IsolateHandle 终止，死循环可杀）。草稿经 localStorage 持久化。
 * 脚本按经典脚本执行（非模块）：不支持 import/export 与顶层 await，
 * 定时器/Promise 事件循环不做（微任务在脚本返回后自动清空）。
 */

import * as monaco from 'monaco-editor';
import { CircleStop, Eraser, Play } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { Button } from '@/components/ui/button';
import { SelectMenu } from '@/components/ui/SelectMenu';
import { setupMonacoEnvironment } from '@/editor/monacoEnvironment';
import { initMonacoTheme, VELOTERM_MONACO_THEME } from '@/editor/monacoTheme';
import { cn } from '@/lib/utils';
import { jsRun, jsRunCancel, type JsConsoleLevel, type JsRunFinish, type JsScriptLanguage } from '@/services/jsScriptService';

/** 草稿持久化键（localStorage；读写失败静默降级为不持久化）。
 *  值为 JSON `{ lang, source }`；旧版纯字符串草稿 = JS 源码 */
const DRAFT_KEY = 'veloterm:js-script:draft';

/** console 行数上限（防脚本刷屏拖垮渲染；超出丢最旧的） */
const MAX_CONSOLE_LINES = 2000;

/** 首次打开的示例脚本（⌘/Ctrl+Enter 即跑；未改动时随语言切换换对应示例） */
const DEFAULT_SCRIPTS: Record<JsScriptLanguage, string> = {
  javascript: [
    '// ⌘/Ctrl + Enter 运行',
    'const list = [1, 2, 3].map((x) => x * 2);',
    "console.log('doubled:', list.join(', '));",
    'list;',
  ].join('\n'),
  typescript: [
    '// ⌘/Ctrl + Enter 运行（TS：类型只擦除不检查，红线是编辑器诊断）',
    'interface P { name: string; age: number }',
    'enum Color { Red, Green = 5, Blue }',
    'const greet = (p: P): string => `${p.name}-${p.age}`;',
    "console.log(greet({ name: '中文', age: 1 }), Color.Blue);",
    'Color.Red;',
  ].join('\n'),
};

/** 超时档位（秒） */
const TIMEOUT_OPTIONS = [5, 30, 60, 300, 600];

interface ConsoleLine {
  level: JsConsoleLevel;
  text: string;
  position?: string | null;
}

let runSeq = 0;

const LEVEL_CLASS: Record<JsConsoleLevel, string> = {
  log: 'text-foreground',
  warn: 'text-terminal-yellow',
  error: 'text-terminal-red',
};

/** 终态错误种类的展示键（jsScript.errTimeout 等五键） */
const KIND_KEY: Record<string, string> = {
  timeout: 'jsScript.errTimeout',
  heapLimit: 'jsScript.errHeapLimit',
  script: 'jsScript.errScript',
  cancelled: 'jsScript.errCancelled',
  host: 'jsScript.errHost',
};

/** console 的精确声明：与运行时绑定完全一致（只有 log/warn/error），
 *  覆盖 DOM lib 里的全量 console 成员（info/debug 等运行时并不存在） */
const CONSOLE_DTS = [
  'declare const console: {',
  '  log(...data: unknown[]): void;',
  '  warn(...data: unknown[]): void;',
  '  error(...data: unknown[]): void;',
  '};',
].join('\n');

interface Draft {
  lang: JsScriptLanguage;
  source: string;
}

function loadDraft(): Draft {
  try {
    const raw = localStorage.getItem(DRAFT_KEY);
    if (raw) {
      try {
        const v = JSON.parse(raw) as { lang?: unknown; source?: unknown };
        if (
          (v.lang === 'javascript' || v.lang === 'typescript') &&
          typeof v.source === 'string'
        ) {
          return { lang: v.lang, source: v.source };
        }
      } catch {
        // 旧版纯字符串草稿 = JS 源码
        return { lang: 'javascript', source: raw };
      }
    }
  } catch {
    // localStorage 不可用（隐私模式等）
  }
  return { lang: 'javascript', source: DEFAULT_SCRIPTS.javascript };
}

export function JsScriptScreen() {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const modelRef = useRef<monaco.editor.ITextModel | null>(null);
  const runIdRef = useRef<string | null>(null);

  const [initial] = useState(loadDraft);
  const [lang, setLang] = useState<JsScriptLanguage>(initial.lang);
  const [source, setSource] = useState(initial.source);
  const [timeoutSecs, setTimeoutSecs] = useState('30');
  const [running, setRunning] = useState(false);
  const [lines, setLines] = useState<ConsoleLine[]>([]);
  const [finish, setFinish] = useState<JsRunFinish | null>(null);

  // 草稿持久化（300ms 防抖；JSON 形态携带语言）
  useEffect(() => {
    const id = window.setTimeout(() => {
      try {
        localStorage.setItem(DRAFT_KEY, JSON.stringify({ lang, source }));
      } catch {
        // 隐私模式等场景静默降级
      }
    }, 300);
    return () => window.clearTimeout(id);
  }, [lang, source]);

  /** 切语言：模型语言热切（ts.worker 诊断/补全随之生效）；
 *  示例未改动时换成对应语言的示例 */
  const switchLang = (next: JsScriptLanguage) => {
    if (next === lang || running) return;
    setLang(next);
    if (modelRef.current) {
      monaco.editor.setModelLanguage(
        modelRef.current,
        next === 'typescript' ? 'typescript' : 'javascript',
      );
    }
    if (source === DEFAULT_SCRIPTS[lang]) {
      setSource(DEFAULT_SCRIPTS[next]);
    }
  };

  // 外部 source 变化（切语言换示例）同步进 model，保留 undo 链
  useEffect(() => {
    const model = modelRef.current;
    if (model && source !== model.getValue()) {
      model.pushEditOperations(
        [],
        [{ range: model.getFullModelRange(), text: source }],
        () => null,
      );
    }
  }, [source]);

  const run = async () => {
    if (running || !source.trim()) return;
    const runId = `js-${Date.now()}-${++runSeq}`;
    runIdRef.current = runId;
    setRunning(true);
    setLines([]);
    setFinish(null);
    try {
      await jsRun(
        runId,
        source,
        (event) => {
          if (event.type === 'console') {
            setLines((prev) => {
              const next = [...prev, { level: event.level, text: event.text, position: event.position }];
              return next.length > MAX_CONSOLE_LINES ? next.slice(-MAX_CONSOLE_LINES) : next;
            });
          } else if (event.type === 'done') {
            setFinish({
              ok: true,
              resultJson: event.resultJson,
              errorKind: null,
              errorMessage: null,
              elapsedMs: event.elapsedMs,
            });
            setRunning(false);
          } else {
            setFinish({
              ok: false,
              resultJson: null,
              errorKind: event.kind,
              errorMessage: event.message,
              elapsedMs: event.elapsedMs,
            });
            setRunning(false);
          }
        },
        {timeoutMs: Number(timeoutSecs) * 1000, language: lang},
      );
    } catch (err) {
      // invoke 层失败（通道异常等）：若尚未收到终态事件则兜底落错误横幅
      setFinish((prev) =>
        prev ?? {
          ok: false,
          resultJson: null,
          errorKind: 'host',
          errorMessage: String(err),
          elapsedMs: 0,
        },
      );
      setRunning(false);
    } finally {
      if (runIdRef.current === runId) runIdRef.current = null;
    }
  };

  const runRef = useRef(run);
  runRef.current = run;

  // ts.worker 配置收敛（挂载时应用、卸载时恢复）：lib 只留纯 ECMAScript
  //（去 DOM —— setTimeout/fetch/XMLHttpRequest/WebSocket/requestAnimationFrame
  // /queueMicrotask 等「沙箱不支持」的全局不再被建议，且立即红线「找不到名称」，
  // 与后端禁用拦截同一口径）；console 用精确声明覆盖。typescriptDefaults 是
  // Monaco 全局的，必须恢复原状——不能影响文件仓等其它 JS/TS 编辑场景
  //（页面互斥渲染，时序即作用域）
  useEffect(() => {
    // Monaco 0.57 起 typescriptDefaults 从 languages.typescript 迁到顶层
    // monaco.typescript（monaco.html 同款先例）
    const ts = monaco.typescript;
    const jsPrevious = ts.javascriptDefaults.getCompilerOptions();
    const tsPrevious = ts.typescriptDefaults.getCompilerOptions();
    const tuned = {
      target: ts.ScriptTarget.ESNext,
      lib: ['esnext'],
      allowNonTsExtensions: true,
    };
    const d1 = ts.javascriptDefaults.addExtraLib(CONSOLE_DTS, 'veloterm-console.d.ts');
    const d2 = ts.typescriptDefaults.addExtraLib(CONSOLE_DTS, 'veloterm-console.d.ts');
    ts.javascriptDefaults.setCompilerOptions({ ...jsPrevious, ...tuned });
    ts.typescriptDefaults.setCompilerOptions({ ...tsPrevious, ...tuned });
    return () => {
      d1.dispose();
      d2.dispose();
      ts.javascriptDefaults.setCompilerOptions(jsPrevious);
      ts.typescriptDefaults.setCompilerOptions(tsPrevious);
    };
  }, []);

  // Monaco 编辑器（自持 model，SqlEditor 同型但更简：仅运行命令 + 内容上报）
  useEffect(() => {
    if (!containerRef.current) return;
    setupMonacoEnvironment();
    initMonacoTheme();

    const model = monaco.editor.createModel(
      source,
      initial.lang,
      monaco.Uri.parse(`inmemory://js-script-${crypto.randomUUID()}`),
    );
    model.onDidChangeContent(() => setSource(model.getValue()));
    modelRef.current = model;

    const editor = monaco.editor.create(containerRef.current, {
      model,
      theme: VELOTERM_MONACO_THEME,
      automaticLayout: true,
      fontSize: 13,
      lineHeight: 20,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      wordWrap: 'on',
      glyphMargin: false,
      folding: true,
      overviewRulerLanes: 0,
      hideCursorInOverviewRuler: true,
      overviewRulerBorder: false,
      scrollbar: { verticalScrollbarSize: 8, horizontalScrollbarSize: 8, useShadows: false },
      padding: { top: 8, bottom: 8 },
      tabSize: 2,
      suggestSelection: 'first',
      // 关键字建议全关：Monaco 无法按单个关键字过滤 worker 建议（async/await
      // 无法单独剔除），全关是唯一途径——代价是 const/return 等正常关键字
      // 也不再提示（仅本编辑器实例，不影响其它场景）
      suggest: { showKeywords: false },
    });
    editorRef.current = editor;
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => {
      runRef.current();
    });

    return () => {
      editor.dispose();
      model.dispose();
      editorRef.current = null;
      modelRef.current = null;
    };
    // 编辑器实例只建一次（source 初始值经闭包取一次，后续变更走 onDidChangeContent）
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const clearOutput = () => {
    setLines([]);
    setFinish(null);
  };

  const errBadgeClass =
    finish && !finish.ok
      ? finish.errorKind === 'cancelled' || finish.errorKind === 'timeout'
        ? 'text-terminal-yellow border-terminal-yellow/40 bg-terminal-yellow/10'
        : 'text-terminal-red border-terminal-red/40 bg-terminal-red/10'
      : '';

  return (
    <div className="flex h-full min-w-0 flex-col">
      {/* 工具栏 */}
      <div className="flex items-center gap-2 border-b border-border/60 px-3 py-1.5">
        <div className="flex overflow-hidden rounded border border-border/60">
          {(['javascript', 'typescript'] as const).map((l) => (
            <button
              key={l}
              type="button"
              onClick={() => switchLang(l)}
              disabled={running}
              className={cn(
                'px-2 py-0.5 text-ui-xs font-medium transition-colors',
                lang === l
                  ? 'bg-selected text-foreground'
                  : 'text-foreground-subtle hover:bg-hover',
              )}
            >
              {t(l === 'javascript' ? 'jsScript.langJs' : 'jsScript.langTs')}
            </button>
          ))}
        </div>
        <span className="text-ui-sm text-foreground-subtle">{t('jsScript.timeoutLabel')}</span>
        <SelectMenu
          value={timeoutSecs}
          onChange={setTimeoutSecs}
          options={TIMEOUT_OPTIONS.map((n) => ({
            value: String(n),
            label: t('jsScript.secOption', { n }),
          }))}
          className="w-24"
          disabled={running}
        />
        <div className="flex-1" />
        <span className="hidden text-ui-xs text-foreground-subtlest sm:inline">⌘/Ctrl + Enter</span>
        <Button
          variant="ghost"
          size="sm"
          onClick={clearOutput}
          disabled={running || (lines.length === 0 && !finish)}
        >
          <Eraser className="size-3.5" />
          {t('jsScript.clear')}
        </Button>
        {running ? (
          <Button variant="secondary" size="sm" onClick={() => runIdRef.current && jsRunCancel(runIdRef.current)}>
            <CircleStop className="size-3.5" />
            {t('jsScript.cancel')}
          </Button>
        ) : null}
        <Button size="sm" onClick={run} disabled={running || !source.trim()}>
          <Play className="size-3.5" />
          {t('jsScript.run')}
        </Button>
      </div>

      {/* 编辑器 */}
      <div ref={containerRef} className="min-h-0 flex-1" />

      {/* 输出面板 */}
      <div className="flex h-64 shrink-0 flex-col border-t border-border/60">
        <div className="flex items-center gap-2 border-b border-border/40 px-3 py-1">
          <span className="text-ui-xs font-medium text-foreground-subtle">{t('jsScript.output')}</span>
          {finish ? (
            <span className="text-ui-xs text-foreground-subtlest">({finish.elapsedMs} ms)</span>
          ) : running ? (
            <span className="animate-pulse text-ui-xs text-terminal-blue">{t('jsScript.running')}</span>
          ) : null}
        </div>
        <div className="scrollbar-slim min-h-0 flex-1 overflow-y-auto px-3 py-1.5 font-mono text-ui-sm leading-relaxed">
          {lines.length === 0 && !finish ? (
            <div className="flex h-full items-center justify-center font-sans text-ui-sm text-foreground-subtlest">
              {t('jsScript.outputEmpty')}
            </div>
          ) : null}
          {lines.map((line, i) => (
            <div key={i} className={cn('whitespace-pre-wrap break-all', LEVEL_CLASS[line.level])}>
              {/* 位置列固定宽度右对齐（无位置时同样占位），正文起始列对齐 */}
              <span className="mr-2 inline-block w-14 shrink-0 select-none text-right text-foreground-subtlest">
                {line.position ?? ''}
              </span>
              {line.text}
            </div>
          ))}
          {finish?.ok ? (
            <div className="mt-2 rounded border border-border/60 bg-hover/40 p-2">
              <div className="mb-1 font-sans text-ui-xs font-medium text-foreground-subtle">
                {t('jsScript.result')}
              </div>
              <div className="whitespace-pre-wrap break-all text-terminal-green">
                {finish.resultJson ?? t('jsScript.resultUndefined')}
              </div>
            </div>
          ) : null}
          {finish && !finish.ok ? (
            <div className={cn('mt-2 rounded border p-2 font-sans text-ui-sm', errBadgeClass)}>
              <span className="font-semibold">
                {t(KIND_KEY[finish.errorKind ?? 'host'] ?? 'jsScript.errHost')}
              </span>
              {finish.errorMessage ? (
                <span className="ml-2 opacity-80">{finish.errorMessage}</span>
              ) : null}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}
