/**
 * crontab 语言支持：Monarch 高亮 + 纯函数 cron 解析器 + 校验 markers +
 * 「下次运行」hover 预览 + 调度模板补全。
 *
 * 纯手写实现（无上游移植）：cron 表达式语法与语义对齐 vixie cron /
 * cronie 的 man 5 crontab——5 字段（minute hour dom month dow）、`*`、`,` 列表、
 * `-` 范围、`/` 步长、JAN-DEC / SUN-SAT 名字（大小写不敏感）、dow 的 0 与 7
 * 都是 Sunday、@宏展开；dom/dow 的 OR 语义按 vixie 规则实现：**两字段都不是
 * `*` 时任一匹配即命中**（`0 0 1 * 1` = 每月 1 号或每周一，经典坑）。
 *
 * `parseCronExpr` / `nextCronRuns` 是零依赖纯函数，可脱离 Monaco 独立测试：
 *   node --experimental-strip-types --input-type=module -e '...'（inline 驱动，
 *   参照 htmlAutoInsert 的纯函数冒烟惯例；命令与用例清单见本文件头注与
 *   systemdData/README.md 的冒烟段）。
 *
 * markers：ensure 时对所有（及后续创建的）model 挂 onDidChangeLanguage /
 * onWillDispose 追踪，语言变为 crontab 时挂 onDidChangeContent 逐行校验——
 * 非注释行的时间表达式跑 parseCronExpr，错误处 setModelMarkers（owner
 * 'cron'，Error 级），合法/离开 crontab 语言时清空；dispose 全程解绑。
 *
 * hover：光标落在表达式区（宏 token 或前 5 个时间字段）时预览未来 3 次运行
 * 时间（本地时区、分钟对齐）；补全是纯文本模板（@宏 / 环境变量 / 常用调度
 * 片段，尾部带空格便于直接续写命令），不设 InsertAsSnippet（仓库惯例，
 * 见 nginxLanguage.ts 的 `$` snippet 坑注释）。
 *
 * 本模块只做 type-only 的 monaco 导入（运行期实例由宿主传入）；无 enum /
 * namespace 等 strip-types 不可擦除语法。
 */

import type * as MonacoNs from 'monaco-editor';

// ─── cron 字段与宏定义（vixie cron 语义）───

interface CronFieldSpec {
  name: string;
  min: number;
  max: number;
  /** 名称表（3 字母、小写比较）：month 与 dow 专属 */
  names?: string[];
}

const CRON_FIELDS: readonly CronFieldSpec[] = [
  { name: 'minute', min: 0, max: 59 },
  { name: 'hour', min: 0, max: 23 },
  { name: 'day of month', min: 1, max: 31 },
  { name: 'month', min: 1, max: 12, names: ['jan', 'feb', 'mar', 'apr', 'may', 'jun', 'jul', 'aug', 'sep', 'oct', 'nov', 'dec'] },
  { name: 'day of week', min: 0, max: 7, names: ['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat'] },
];

const CRON_MACROS: Readonly<Record<string, string>> = {
  '@yearly': '0 0 1 1 *',
  '@annually': '0 0 1 1 *',
  '@monthly': '0 0 1 * *',
  '@weekly': '0 0 * * 0',
  '@daily': '0 0 * * *',
  '@midnight': '0 0 * * *',
  '@hourly': '0 * * * *',
};

// ─── 纯函数解析器（parseCronExpr / nextCronRuns 独立导出，零依赖）───

export interface CronParseError {
  /** 出错字段序号（0=minute…4=dow）；-1 为整体错误（字段数/宏） */
  field: number;
  message: string;
}

export interface CronParseResult {
  ok: boolean;
  error?: CronParseError;
  /** dom 与 dow 都非 `*` → vixie OR 语义生效 */
  domDowOrSemantics?: boolean;
  /** @reboot：开机运行一次，无固定时刻表 */
  reboot?: boolean;
}

interface CronSchedule {
  minutes: number[];
  hours: number[];
  doms: number[];
  months: number[];
  dows: number[];
  domRestricted: boolean;
  dowRestricted: boolean;
  reboot: boolean;
}

type CronFieldResult =
  | { set: Set<number>; restricted: boolean }
  | { err: string };

/** dow 的 7 归一为 0（都是 Sunday） */
function normalizeDow(value: number, fieldIdx: number): number {
  return fieldIdx === 4 && value === 7 ? 0 : value;
}

/** token（数字或本字段名称表中的名字）→ 数值；不属于本字段返回 null */
function resolveCronToken(tok: string, spec: CronFieldSpec): number | null {
  if (/^\d+$/.test(tok)) return parseInt(tok, 10);
  if (spec.names) {
    const idx = spec.names.indexOf(tok.toLowerCase());
    if (idx >= 0) return spec.min + idx;
  }
  return null;
}

/** 单字段解析：`*` | 列表 | 范围 | 步长及组合（`5/15` 按 cronie 语义 = 5-max/15） */
function parseCronField(text: string, fieldIdx: number): CronFieldResult {
  const spec = CRON_FIELDS[fieldIdx]!;
  if (text === '*') {
    const set = new Set<number>();
    for (let v = spec.min; v <= spec.max; v += 1) set.add(normalizeDow(v, fieldIdx));
    return { set, restricted: false };
  }
  const set = new Set<number>();
  for (const item of text.split(',')) {
    if (!item) return { err: `empty list item in '${text}'` };
    let rangePart = item;
    let stepText: string | null = null;
    const slash = item.indexOf('/');
    if (slash >= 0) {
      if (item.indexOf('/', slash + 1) >= 0) {
        return { err: `multiple '/' separators in '${item}'` };
      }
      rangePart = item.slice(0, slash);
      stepText = item.slice(slash + 1);
      if (!stepText || !/^\d+$/.test(stepText)) {
        return { err: `step must be a positive integer in '${item}'` };
      }
    }
    const step = stepText ? parseInt(stepText, 10) : 1;
    if (step < 1) return { err: `step must be >= 1 in '${item}'` };
    let lo: number;
    let hi: number;
    if (rangePart === '*') {
      lo = spec.min;
      hi = spec.max;
    } else {
      const dash = rangePart.indexOf('-');
      if (dash >= 0) {
        if (rangePart.indexOf('-', dash + 1) >= 0) {
          return { err: `multiple '-' separators in '${item}'` };
        }
        const loTok = rangePart.slice(0, dash);
        const hiTok = rangePart.slice(dash + 1);
        if (!loTok || !hiTok) return { err: `malformed range '${item}'` };
        const loV = resolveCronToken(loTok, spec);
        const hiV = resolveCronToken(hiTok, spec);
        if (loV === null) return { err: `invalid value '${loTok}' for ${spec.name}` };
        if (hiV === null) return { err: `invalid value '${hiTok}' for ${spec.name}` };
        lo = loV;
        hi = hiV;
      } else {
        const v = resolveCronToken(rangePart, spec);
        if (v === null) return { err: `invalid value '${rangePart}' for ${spec.name}` };
        lo = v;
        hi = stepText ? spec.max : v;
      }
    }
    if (lo < spec.min || lo > spec.max) {
      return { err: `${spec.name} value ${lo} out of range ${spec.min}-${spec.max}` };
    }
    if (hi < spec.min || hi > spec.max) {
      return { err: `${spec.name} value ${hi} out of range ${spec.min}-${spec.max}` };
    }
    if (lo > hi) return { err: `inverted range in '${item}' (${lo} > ${hi})` };
    for (let v = lo; v <= hi; v += step) set.add(normalizeDow(v, fieldIdx));
  }
  if (set.size === 0) return { err: `empty field '${text}'` };
  return { set, restricted: true };
}

function parseCronDetailed(
  expr: string,
): { schedule: CronSchedule | null; error: CronParseError | null } {
  const trimmed = expr.trim();
  if (!trimmed) {
    return { schedule: null, error: { field: -1, message: 'empty cron expression' } };
  }
  let expr5 = trimmed;
  let reboot = false;
  if (trimmed.startsWith('@')) {
    const parts = trimmed.split(/\s+/);
    const macro = parts[0]!.toLowerCase();
    if (parts.length > 1) {
      return {
        schedule: null,
        error: { field: -1, message: `unexpected text after macro '${parts[0]}'` },
      };
    }
    if (macro === '@reboot') {
      reboot = true;
    } else {
      const expansion = CRON_MACROS[macro];
      if (!expansion) {
        return {
          schedule: null,
          error: {
            field: -1,
            message: `unknown macro '${parts[0]}' (expected @reboot @yearly @annually @monthly @weekly @daily @midnight @hourly)`,
          },
        };
      }
      expr5 = expansion;
    }
  }
  if (reboot) {
    return {
      schedule: {
        minutes: [], hours: [], doms: [], months: [], dows: [],
        domRestricted: false, dowRestricted: false, reboot: true,
      },
      error: null,
    };
  }
  const parts = expr5.split(/\s+/);
  if (parts.length !== 5) {
    return {
      schedule: null,
      error: {
        field: Math.min(parts.length, 5),
        message: `expected 5 time fields (minute hour day-of-month month day-of-week), got ${parts.length}`,
      },
    };
  }
  const fields: { set: Set<number>; restricted: boolean }[] = [];
  for (let i = 0; i < 5; i += 1) {
    const r = parseCronField(parts[i]!, i);
    if ('err' in r) return { schedule: null, error: { field: i, message: r.err } };
    fields.push(r);
  }
  const sorted = (s: Set<number>) => [...s].sort((a, b) => a - b);
  return {
    schedule: {
      minutes: sorted(fields[0]!.set),
      hours: sorted(fields[1]!.set),
      doms: sorted(fields[2]!.set),
      months: sorted(fields[3]!.set),
      dows: sorted(fields[4]!.set),
      domRestricted: fields[2]!.restricted,
      dowRestricted: fields[4]!.restricted,
      reboot: false,
    },
    error: null,
  };
}

/**
 * 校验 5 字段 cron 表达式或 @宏。合法时返回 ok:true（附 dom/dow OR 语义标记
 * 与 @reboot 标记），非法时返回错误所在字段序号与信息。
 */
export function parseCronExpr(expr: string): CronParseResult {
  const { schedule, error } = parseCronDetailed(expr);
  if (!schedule) {
    return { ok: false, error: error ?? { field: -1, message: 'invalid cron expression' } };
  }
  return {
    ok: true,
    domDowOrSemantics: schedule.domRestricted && schedule.dowRestricted,
    reboot: schedule.reboot,
  };
}

/**
 * 计算 `from` 之后（严格大于，分钟对齐、秒/毫秒丢弃）的下 count 次运行时刻，
 * 本地时区。非法表达式与 @reboot 返回 []；极稀疏表达式（如 2 月 29 日）以
 * 8 年为搜索上限，超窗不足 count 次时返回已找到的部分。
 *
 * dom/dow 采用 vixie OR 语义：两字段都非 `*` 时任一匹配即命中；只有一方
 * 受限时按该方匹配；都是 `*` 时每天命中。
 */
export function nextCronRuns(expr: string, from: Date, count: number): Date[] {
  if (count <= 0) return [];
  const { schedule } = parseCronDetailed(expr);
  if (!schedule || schedule.reboot) return [];
  const results: Date[] = [];
  const start = new Date(
    from.getFullYear(), from.getMonth(), from.getDate(),
    from.getHours(), from.getMinutes() + 1, 0, 0,
  );
  let day = new Date(start.getFullYear(), start.getMonth(), start.getDate());
  const dayLimit = 366 * 8 + 2;
  for (let guard = 0; guard < dayLimit && results.length < count; guard += 1) {
    if (schedule.months.includes(day.getMonth() + 1)) {
      const dom = day.getDate();
      const dow = day.getDay();
      const dayMatches =
        schedule.domRestricted && schedule.dowRestricted
          ? schedule.doms.includes(dom) || schedule.dows.includes(dow)
          : (!schedule.domRestricted || schedule.doms.includes(dom)) &&
            (!schedule.dowRestricted || schedule.dows.includes(dow));
      if (dayMatches) {
        for (const h of schedule.hours) {
          for (const m of schedule.minutes) {
            const t = new Date(
              day.getFullYear(), day.getMonth(), day.getDate(), h, m, 0, 0,
            );
            if (t >= start) {
              results.push(t);
              if (results.length >= count) return results;
            }
          }
        }
      }
    }
    day = new Date(day.getFullYear(), day.getMonth(), day.getDate() + 1);
  }
  return results;
}

// ─── 行级工具（markers / hover 共用）───

const ENV_ASSIGN_RE = /^[A-Za-z_][A-Za-z0-9_]*[ \t]*=/;

interface CronLineToken {
  text: string;
  /** 0 基字符偏移 */
  start: number;
}

function tokenizeCronLine(line: string): CronLineToken[] {
  const tokens: CronLineToken[] = [];
  let i = 0;
  while (i < line.length) {
    while (i < line.length && /\s/.test(line.charAt(i))) i += 1;
    if (i >= line.length) break;
    const start = i;
    while (i < line.length && !/\s/.test(line.charAt(i))) i += 1;
    tokens.push({ text: line.slice(start, i), start });
  }
  return tokens;
}

function formatCronDateTime(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

// ─── Monarch 高亮 ───

const CRONTAB_MONARCH_LANGUAGE: MonacoNs.languages.IMonarchLanguage = {
  defaultToken: '',
  tokenPostfix: '.crontab',
  tokenizer: {
    // 行首分流：注释 / @宏 / 特殊与一般环境变量赋值 / 时间表达式（五个字段态）
    root: [
      [/^[ \t]*#.*$/, 'comment'],
      [/^[ \t]*$/, ''],
      [/@(reboot|yearly|annually|monthly|weekly|daily|midnight|hourly)(?=$|[ \t])/, 'keyword', '@command'],
      [/^[ \t]*(PATH|SHELL|MAILTO|MAILFROM|CRON_TZ|HOME|LOGNAME|USER)[ \t]*(?==)/, 'type'],
      [/^[ \t]*[A-Za-z_][A-Za-z0-9_]*[ \t]*(?==)/, 'variable.name'],
      [/[=]/, 'delimiter', '@envvalue'],
      [/^[ \t]*(?=\S)/, '', '@field1'],
      [/./, ''],
    ],
    // 五个时间字段态：数字/名称/`*`/步进与范围算符；空白推进到下一字段，
    // 第 5 字段后进入命令态；行尾（可带尾随空白）弹回 root
    field1: [
      [/[ \t]*$/, '', '@pop'],
      [/[A-Za-z]{3}(?![A-Za-z])/, 'type'],
      [/\d+/, 'number'],
      [/[*]/, 'keyword'],
      [/[\/\-,]/, 'operator'],
      [/[ \t]+/, '', '@field2'],
      [/./, ''],
    ],
    field2: [
      [/[ \t]*$/, '', '@pop'],
      [/[A-Za-z]{3}(?![A-Za-z])/, 'type'],
      [/\d+/, 'number'],
      [/[*]/, 'keyword'],
      [/[\/\-,]/, 'operator'],
      [/[ \t]+/, '', '@field3'],
      [/./, ''],
    ],
    field3: [
      [/[ \t]*$/, '', '@pop'],
      [/[A-Za-z]{3}(?![A-Za-z])/, 'type'],
      [/\d+/, 'number'],
      [/[*]/, 'keyword'],
      [/[\/\-,]/, 'operator'],
      [/[ \t]+/, '', '@field4'],
      [/./, ''],
    ],
    field4: [
      [/[ \t]*$/, '', '@pop'],
      [/[A-Za-z]{3}(?![A-Za-z])/, 'type'],
      [/\d+/, 'number'],
      [/[*]/, 'keyword'],
      [/[\/\-,]/, 'operator'],
      [/[ \t]+/, '', '@field5'],
      [/./, ''],
    ],
    field5: [
      [/[ \t]*$/, '', '@pop'],
      [/[A-Za-z]{3}(?![A-Za-z])/, 'type'],
      [/\d+/, 'number'],
      [/[*]/, 'keyword'],
      [/[\/\-,]/, 'operator'],
      [/[ \t]+/, '', '@command'],
      [/./, ''],
    ],
    // 命令部分：简单 shell 词法（字符串 / $变量 / 重定向与管道算符）
    command: [
      [/[ \t]*$/, '', '@pop'],
      [/"([^"\\]|\\.)*"?/, 'string'],
      [/'([^'\\]|\\.)*'?/, 'string'],
      [/\$\{?[A-Za-z_][A-Za-z0-9_]*\}?/, 'variable'],
      [/[|>&;<()]/, 'operator'],
      [/\d+/, 'number'],
      [/\s/, ''],
      [/./, ''],
    ],
    // 环境变量值：简单字符串着色，行尾弹回 root
    envvalue: [
      [/[ \t]*$/, '', '@pop'],
      [/"([^"\\]|\\.)*"?/, 'string'],
      [/'([^'\\]|\\.)*'?/, 'string'],
      [/\$\{?[A-Za-z_][A-Za-z0-9_]*\}?/, 'variable'],
      [/[^\s]/, 'string'],
      [/\s/, ''],
    ],
  },
};

const CRONTAB_LANGUAGE_CONFIGURATION: MonacoNs.languages.LanguageConfiguration = {
  comments: { lineComment: '#' },
};

// ─── 校验 markers（crontab 语言 model 专属，owner 'cron'）───

const CRON_MARKER_OWNER = 'cron';

interface CrontabModelTracking {
  disposables: MonacoNs.IDisposable[];
  contentListener: MonacoNs.IDisposable | null;
}

const trackedCrontabModels = new WeakMap<
  MonacoNs.editor.ITextModel,
  CrontabModelTracking
>();

function validateCrontabModel(
  monaco: typeof MonacoNs,
  model: MonacoNs.editor.ITextModel,
): void {
  const markers: MonacoNs.editor.IMarkerData[] = [];
  const lines = model.getLinesContent();
  for (let i = 0; i < lines.length; i += 1) {
    const raw = lines[i]!;
    const trimmed = raw.trim();
    if (!trimmed || trimmed.startsWith('#')) continue;
    if (ENV_ASSIGN_RE.test(trimmed)) continue;
    if (trimmed.startsWith('@')) {
      const res = parseCronExpr(trimmed.split(/\s+/)[0]!);
      if (!res.ok) {
        markers.push({
          severity: monaco.MarkerSeverity.Error,
          message: `cron: ${res.error!.message}`,
          startLineNumber: i + 1,
          endLineNumber: i + 1,
          startColumn: 1,
          endColumn: raw.length + 1,
        });
      }
      continue;
    }
    const tokens = tokenizeCronLine(raw);
    if (tokens.length < 5) {
      markers.push({
        severity: monaco.MarkerSeverity.Error,
        message: `cron: expected 5 time fields (minute hour day-of-month month day-of-week), got ${tokens.length}`,
        startLineNumber: i + 1,
        endLineNumber: i + 1,
        startColumn: 1,
        endColumn: raw.length + 1,
      });
      continue;
    }
    const expr = tokens.slice(0, 5).map((t) => t.text).join(' ');
    const res = parseCronExpr(expr);
    if (!res.ok) {
      const err = res.error!;
      const tok = err.field >= 0 && err.field < 5 ? tokens[err.field] : undefined;
      markers.push({
        severity: monaco.MarkerSeverity.Error,
        message: `cron: ${err.message}`,
        startLineNumber: i + 1,
        endLineNumber: i + 1,
        startColumn: tok ? tok.start + 1 : 1,
        endColumn: tok ? tok.start + 1 + tok.text.length : raw.length + 1,
      });
    }
  }
  monaco.editor.setModelMarkers(model, CRON_MARKER_OWNER, markers);
}

/** 对所有 model 挂语言切换/销毁追踪，语言为 crontab 时挂内容监听并即时校验 */
function trackCrontabModel(
  monaco: typeof MonacoNs,
  model: MonacoNs.editor.ITextModel,
): void {
  if (trackedCrontabModels.has(model)) return;
  const tracking: CrontabModelTracking = { disposables: [], contentListener: null };
  trackedCrontabModels.set(model, tracking);
  tracking.disposables.push(
    model.onDidChangeLanguage(() => syncCrontabModel(monaco, model)),
    model.onWillDispose(() => untrackCrontabModel(monaco, model)),
  );
  syncCrontabModel(monaco, model);
}

function syncCrontabModel(
  monaco: typeof MonacoNs,
  model: MonacoNs.editor.ITextModel,
): void {
  const tracking = trackedCrontabModels.get(model);
  if (!tracking) return;
  if (model.getLanguageId() === 'crontab') {
    if (!tracking.contentListener) {
      tracking.contentListener = model.onDidChangeContent(() =>
        validateCrontabModel(monaco, model),
      );
    }
    validateCrontabModel(monaco, model);
  } else if (tracking.contentListener) {
    tracking.contentListener.dispose();
    tracking.contentListener = null;
    monaco.editor.setModelMarkers(model, CRON_MARKER_OWNER, []);
  }
}

function untrackCrontabModel(
  monaco: typeof MonacoNs,
  model: MonacoNs.editor.ITextModel,
): void {
  const tracking = trackedCrontabModels.get(model);
  if (!tracking) return;
  tracking.contentListener?.dispose();
  for (const d of tracking.disposables) d.dispose();
  monaco.editor.setModelMarkers(model, CRON_MARKER_OWNER, []);
  trackedCrontabModels.delete(model);
}

// ─── 补全数据（纯文本模板，见文件头注释）───

const CRON_MACROS_DOC: { macro: string; desc: string }[] = [
  { macro: '@reboot', desc: 'run once at system boot' },
  { macro: '@yearly', desc: 'run once a year, midnight Jan 1 (0 0 1 1 *)' },
  { macro: '@annually', desc: 'same as @yearly (0 0 1 1 *)' },
  { macro: '@monthly', desc: 'run once a month, midnight the 1st (0 0 1 * *)' },
  { macro: '@weekly', desc: 'run once a week, midnight Sunday (0 0 * * 0)' },
  { macro: '@daily', desc: 'run once a day at midnight (0 0 * * *)' },
  { macro: '@midnight', desc: 'same as @daily (0 0 * * *)' },
  { macro: '@hourly', desc: 'run once an hour at minute 0 (0 * * * *)' },
];

const CRON_TEMPLATES: { label: string; insert: string; detail: string }[] = [
  { label: '* * * * *', insert: '* * * * * ', detail: 'every minute' },
  { label: '*/5 * * * *', insert: '*/5 * * * * ', detail: 'every 5 minutes' },
  { label: '*/15 * * * *', insert: '*/15 * * * * ', detail: 'every 15 minutes' },
  { label: '*/30 * * * *', insert: '*/30 * * * * ', detail: 'every 30 minutes' },
  { label: '0 * * * *', insert: '0 * * * * ', detail: 'hourly at minute 0' },
  { label: '0 3 * * *', insert: '0 3 * * * ', detail: 'daily at 03:00' },
  { label: '0 3 * * 1', insert: '0 3 * * 1 ', detail: 'Mondays at 03:00' },
  { label: '0 3 1 * *', insert: '0 3 1 * * ', detail: '1st of every month at 03:00' },
  { label: '0 9 * * 1-5', insert: '0 9 * * 1-5 ', detail: 'weekdays at 09:00' },
];

const CRON_ENV_VARS: { name: string; desc: string }[] = [
  { name: 'MAILTO', desc: 'Mail command output to this address (empty value disables mail).' },
  { name: 'MAILFROM', desc: 'From address used for cron mails.' },
  { name: 'SHELL', desc: 'Shell used to run commands (default /bin/sh).' },
  { name: 'PATH', desc: 'Search path for commands (system dependent default).' },
  { name: 'CRON_TZ', desc: 'Timezone the schedule of entries in this crontab is interpreted in.' },
  { name: 'HOME', desc: 'Home directory passed to commands.' },
  { name: 'LOGNAME', desc: 'Login name passed to commands.' },
];

// ─── 注册入口（幂等；monaco 实例由宿主传入）───

let registered = false;

export function ensureCrontabLanguage(monaco: typeof MonacoNs): void {
  if (registered) return;
  registered = true;

  monaco.languages.register({
    id: 'crontab',
    extensions: ['.crontab'],
    filenames: ['crontab'],
    aliases: ['crontab'],
  });
  monaco.languages.setLanguageConfiguration('crontab', CRONTAB_LANGUAGE_CONFIGURATION);
  monaco.languages.setMonarchTokensProvider('crontab', CRONTAB_MONARCH_LANGUAGE);

  // 校验 markers：存量 + 新建 model 全量追踪，语言切到 crontab 才激活
  monaco.editor.onDidCreateModel((model) => trackCrontabModel(monaco, model));
  for (const model of monaco.editor.getModels()) trackCrontabModel(monaco, model);

  monaco.languages.registerCompletionItemProvider('crontab', {
    triggerCharacters: ['@', '*', '/'],
    provideCompletionItems(model, position) {
      const before = model.getValueInRange({
        startLineNumber: position.lineNumber,
        startColumn: 1,
        endLineNumber: position.lineNumber,
        endColumn: position.column,
      });
      const beforeTrim = before.replace(/^[ \t]+/, '');
      // 仅行首 token 区给候选（时间表达式/宏/环境变量名）；命令区离线无候选
      if (/\s/.test(beforeTrim)) return { suggestions: [] };

      const lineContent = model.getLineContent(position.lineNumber);
      const word = model.getWordUntilPosition(position);
      const charBeforeWord =
        word.startColumn > 1 ? lineContent.charAt(word.startColumn - 2) : '';
      const macroMode = charBeforeWord === '@' || beforeTrim.startsWith('@');
      const replace: MonacoNs.IRange = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: macroMode ? word.startColumn - 1 : word.startColumn || position.column,
        endColumn: word.endColumn || position.column,
      };
      const items: MonacoNs.languages.CompletionItem[] = [];
      for (const t of CRON_TEMPLATES) {
        items.push({
          label: t.label,
          kind: monaco.languages.CompletionItemKind.Snippet,
          detail: t.detail,
          documentation: { value: `Schedule template — ${t.detail}` },
          insertText: t.insert,
          range: replace,
        });
      }
      for (const m of CRON_MACROS_DOC) {
        items.push({
          label: m.macro,
          kind: monaco.languages.CompletionItemKind.Keyword,
          detail: 'macro',
          documentation: { value: `${m.desc} (expands to \`${CRON_MACROS[m.macro] ?? 'no fixed schedule'}\`)` },
          insertText: m.macro,
          range: replace,
        });
      }
      for (const v of CRON_ENV_VARS) {
        items.push({
          label: `${v.name}=`,
          kind: monaco.languages.CompletionItemKind.Variable,
          detail: 'environment variable',
          documentation: { value: v.desc },
          insertText: `${v.name}=`,
          range: replace,
        });
      }
      return { suggestions: items };
    },
  });

  monaco.languages.registerHoverProvider('crontab', {
    provideHover(model, position) {
      const raw = model.getLineContent(position.lineNumber);
      const trimmed = raw.trim();
      if (!trimmed || trimmed.startsWith('#') || ENV_ASSIGN_RE.test(trimmed)) return null;
      const offset = position.column - 1;
      let expr: string;
      let range: MonacoNs.IRange;
      let macro = false;
      if (trimmed.startsWith('@')) {
        const start = raw.indexOf('@');
        const macroTok = trimmed.split(/\s+/)[0]!;
        const end = start + macroTok.length;
        if (offset < start || offset > end) return null;
        expr = macroTok;
        macro = true;
        range = {
          startLineNumber: position.lineNumber,
          endLineNumber: position.lineNumber,
          startColumn: start + 1,
          endColumn: end + 1,
        };
      } else {
        const tokens = tokenizeCronLine(raw);
        if (tokens.length < 5) return null;
        const fifth = tokens[4]!;
        if (offset > fifth.start + fifth.text.length) return null;
        expr = tokens.slice(0, 5).map((t) => t.text).join(' ');
        range = {
          startLineNumber: position.lineNumber,
          endLineNumber: position.lineNumber,
          startColumn: tokens[0]!.start + 1,
          endColumn: fifth.start + fifth.text.length + 1,
        };
      }
      const res = parseCronExpr(expr);
      if (!res.ok) {
        return {
          contents: [{ value: `**Invalid cron expression**\n\n${res.error!.message}` }],
          range,
        };
      }
      if (res.reboot) {
        return {
          contents: [{ value: '**@reboot** — runs once at system boot (no fixed schedule)' }],
          range,
        };
      }
      const parts: string[] = [];
      const runs = nextCronRuns(expr, new Date(), 3).map(formatCronDateTime);
      parts.push(
        runs.length
          ? `**Next runs:** ${runs.join(' / ')}`
          : '**Next runs:** no match within the search window',
      );
      if (macro) {
        parts.push(`Expands to \`${CRON_MACROS[expr.toLowerCase()]}\``);
      }
      if (res.domDowOrSemantics) {
        parts.push(
          'Day-of-month and day-of-week are both restricted — the command runs when **either** matches (vixie OR semantics).',
        );
      }
      return { contents: [{ value: parts.join('\n\n') }], range };
    },
  });
}
