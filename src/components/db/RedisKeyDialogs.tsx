/**
 * Redis 键管理与诊断对话框组（DbSessionView 消费）：
 * - RedisKeyEditorModal：按类型的键值整体编辑（结构化读 → 表单 → 原子覆写；
 *   成员超上限 truncated 时只读——DEL+重写语义下截断保存会丢数据）
 * - RedisTtlModal：TTL 查看/修改（EXPIRE）/移除过期（PERSIST），经 dbQuery 组行命令
 * - RedisKeyDeleteModal：删除确认（DEL）
 * - RedisSlowlogModal / RedisBigkeysModal：慢日志与大 key 分析（DataTable 呈现）
 * 写命令的 key 一律经 redisQuote 转义（含空白/引号也能正确组行）。
 */

import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ArrowDown, ArrowUp, Loader2, Plus, Trash2 } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { DataTable } from '@/components/ui/DataTable';
import { FormField, TextInput } from '@/components/hosts/formFields';
import {
  dbQuery,
  redisBigkeys,
  redisReadKey,
  redisSlowlog,
  redisWriteKey,
  type RedisBigkeysData,
  type RedisKeyData,
  type RedisKeyWritePayload,
  type RedisTableData,
} from '@/services/dbConnectionsService';
import { cn } from '@/lib/utils';

/** Redis 命令参数转义：含空白/引号/反斜杠时用双引号包裹并转义（与后端 redis_quote_arg 同规则） */
export function redisQuote(value: string): string {
  return /[\s"'\\]/.test(value) ? `"${value.replace(/(["\\])/g, '\\$1')}"` : value;
}

/** 单值命令执行（TTL/PERSIST/DEL/EXPIRE 这类一次往返）：返回首行首列文本 */
function runScalarCommand(sessionId: string, command: string): Promise<string> {
  return runScalarStatement(sessionId, command, 0);
}

/** 跨库单值命令：目标库与会话当前库不同时用 SELECT 夹心（切库 → 命令 → 切回），
 *  取中间语句（index=1）的结果行 */
function runDbAwareCommand(
  sessionId: string,
  keyDb: string | undefined,
  currentDb: string | null,
  command: string,
): Promise<string> {
  if (!keyDb || keyDb === currentDb) return runScalarCommand(sessionId, command);
  const n = Number.parseInt(keyDb.replace(/^db/i, ''), 10) || 0;
  const back = Number.parseInt((currentDb ?? 'db0').replace(/^db/i, ''), 10) || 0;
  return runScalarStatement(sessionId, `SELECT ${n}\n${command}\nSELECT ${back}`, 1);
}

function runScalarStatement(
  sessionId: string,
  command: string,
  wantIndex: number,
): Promise<string> {
  return new Promise((resolve, reject) => {
    let out = '';
    void dbQuery(sessionId, command, (ev) => {
      if (ev.type === 'rows' && ev.index === wantIndex) out = ev.rows[0]?.[0] ?? '';
      if (ev.type === 'statementError' && ev.index === wantIndex) reject(new Error(ev.message));
    })
      .then(() => resolve(out))
      .catch(reject);
  });
}

/** 行编辑组的行内按钮（增/删/上下移）共用样式 */
const rowButtonClass =
  'flex size-6 shrink-0 items-center justify-center rounded-md border border-border/60 text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground disabled:opacity-30';

// ── 键编辑 ─────────────────────────────────────────────────────────────────

export function RedisKeyEditorModal({
  sessionId,
  keyName,
  keyDb,
  onClose,
  onSaved,
}: {
  sessionId: string;
  keyName: string;
  /** 键所属逻辑库（树节点携带的 dbN；undefined = 会话当前库） */
  keyDb: string | undefined;
  onClose: () => void;
  /** 保存成功（父层刷新对象树 + 填入查看命令） */
  onSaved: () => void;
}) {
  const { t } = useTranslation();
  const [data, setData] = useState<RedisKeyData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  // 各类型编辑态（按加载到的 kind 只用一个）
  const [stringValue, setStringValue] = useState('');
  const [hashRows, setHashRows] = useState<{ field: string; value: string }[]>([]);
  const [listItems, setListItems] = useState<string[]>([]);
  const [setMembers, setSetMembers] = useState<string[]>([]);
  const [zsetRows, setZsetRows] = useState<{ member: string; score: string }[]>([]);

  useEffect(() => {
    let alive = true;
    redisReadKey(sessionId, keyName, keyDb)
      .then((loaded) => {
        if (!alive) return;
        setData(loaded);
        setStringValue(loaded.value ?? '');
        setHashRows((loaded.entries as { field: string; value: string }[] | undefined) ?? []);
        setListItems(loaded.items ?? []);
        setSetMembers(loaded.members ?? []);
        setZsetRows((loaded.entries as { member: string; score: string }[] | undefined) ?? []);
      })
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, [sessionId, keyName, keyDb]);

  const handleSave = async () => {
    if (!data || saving || data.truncated) return;
    let payload: RedisKeyWritePayload | null = null;
    switch (data.kind) {
      case 'string':
        payload = { kind: 'string', value: stringValue };
        break;
      case 'hash':
        payload = { kind: 'hash', entries: hashRows.filter((r) => r.field.length > 0) };
        break;
      case 'list':
        payload = { kind: 'list', items: listItems };
        break;
      case 'set':
        payload = { kind: 'set', members: setMembers.filter((m) => m.length > 0) };
        break;
      case 'zset':
        payload = { kind: 'zset', entries: zsetRows.filter((r) => r.member.length > 0) };
        break;
      default:
        return;
    }
    setSaving(true);
    setError(null);
    try {
      // keepTtl：覆写后恢复原过期时间（后端原子管道内完成）
      await redisWriteKey(sessionId, keyName, payload, true, keyDb);
      onSaved();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const editable = data != null && !data.truncated;
  const kindEditable =
    data?.kind === 'string' ||
    data?.kind === 'hash' ||
    data?.kind === 'list' ||
    data?.kind === 'set' ||
    data?.kind === 'zset';

  return (
    <Modal
      title={`${t('db.redisEditKey')} · ${keyName}`}
      onClose={onClose}
      className="w-[520px]"
      footer={
        <>
          <span className="min-w-0 flex-1 truncate text-ui-xs text-foreground-subtlest">
            {data ? `TYPE ${data.kind} · TTL ${data.ttl}` : ''}
          </span>
          <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={onClose}>
            {t('common.close', '关闭')}
          </Button>
          {kindEditable ? (
            <Button
              size="lg"
              className="h-8 px-4 text-ui-base"
              disabled={!editable || saving}
              onClick={() => void handleSave()}
            >
              {saving ? <Loader2 className="size-3.5 animate-spin" /> : null}
              {t('common.save')}
            </Button>
          ) : null}
        </>
      }
    >
      <div className="flex max-h-[440px] flex-col gap-2">
        {error ? (
          <p className="break-all font-mono text-ui-xs text-destructive">{error}</p>
        ) : null}
        {data == null ? (
          <div className="flex h-24 items-center justify-center gap-2 text-ui-sm text-foreground-subtle select-none">
            {!error ? <Loader2 className="size-4 animate-spin" /> : null}
            {t('db.loadingDefinition')}
          </div>
        ) : data.truncated ? (
          <p className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-ui-xs text-amber-500">
            {t('db.redisTruncatedReadonly')}
          </p>
        ) : !kindEditable ? (
          <p className="px-1 text-ui-xs text-foreground-subtle">{t('db.redisEditUnsupported')}</p>
        ) : data.kind === 'string' ? (
          <textarea
            value={stringValue}
            onChange={(e) => setStringValue(e.target.value)}
            spellCheck={false}
            rows={12}
            className="scrollbar-slim w-full resize-y rounded-md border border-border/60 bg-background px-2 py-1.5 font-mono text-ui-xs text-foreground outline-none focus:border-border"
          />
        ) : data.kind === 'hash' ? (
          <PairRowsEditor
            firstLabel={t('db.redisFieldCol')}
            secondLabel={t('db.redisValueCol')}
            rows={hashRows}
            onChange={setHashRows}
            addLabel={t('db.redisAddRow')}
          />
        ) : data.kind === 'zset' ? (
          <PairRowsEditor
            firstLabel={t('db.redisMemberCol')}
            secondLabel={t('db.redisScoreCol')}
            rows={zsetRows.map((r) => ({ field: r.member, value: r.score }))}
            onChange={(rows) => setZsetRows(rows.map((r) => ({ member: r.field, score: r.value })))}
            addLabel={t('db.redisAddRow')}
            secondMono={false}
          />
        ) : data.kind === 'list' ? (
          <ListRowsEditor
            label={t('db.redisItemCol')}
            rows={listItems}
            onChange={setListItems}
            addLabel={t('db.redisAddRow')}
            reorder
          />
        ) : (
          <ListRowsEditor
            label={t('db.redisMemberCol')}
            rows={setMembers}
            onChange={setSetMembers}
            addLabel={t('db.redisAddRow')}
          />
        )}
      </div>
    </Modal>
  );
}

/** 两列行编辑（hash 的 field/value、zset 的 member/score）：增删行 */
function PairRowsEditor({
  firstLabel,
  secondLabel,
  secondMono = true,
  rows,
  onChange,
  addLabel,
}: {
  firstLabel: string;
  secondLabel: string;
  secondMono?: boolean;
  rows: { field: string; value: string }[];
  onChange: (rows: { field: string; value: string }[]) => void;
  addLabel: string;
}) {
  const update = (i: number, patch: Partial<{ field: string; value: string }>) => {
    onChange(rows.map((r, idx) => (idx === i ? { ...r, ...patch } : r)));
  };
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex gap-1.5 px-0.5 text-[10px] font-semibold uppercase tracking-wide text-foreground-subtlest select-none">
        <span className="w-40 shrink-0">{firstLabel}</span>
        <span className="min-w-0 flex-1">{secondLabel}</span>
        <span className="w-6 shrink-0" />
      </div>
      <div className="scrollbar-slim flex max-h-72 flex-col gap-1 overflow-y-auto pr-0.5">
        {rows.map((row, i) => (
          <div key={i} className="flex items-center gap-1.5">
            <TextInput
              value={row.field}
              onChange={(e) => update(i, { field: e.target.value })}
              className="w-40 shrink-0 font-mono"
            />
            <TextInput
              value={row.value}
              onChange={(e) => update(i, { value: e.target.value })}
              className={cn('min-w-0 flex-1', secondMono && 'font-mono')}
            />
            <button
              type="button"
              className={rowButtonClass}
              onClick={() => onChange(rows.filter((_, idx) => idx !== i))}
              aria-label="remove row"
            >
              <Trash2 className="size-3" />
            </button>
          </div>
        ))}
      </div>
      <button
        type="button"
        className="flex items-center gap-1 self-start rounded-md border border-border/60 px-2 py-1 text-ui-xs text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
        onClick={() => onChange([...rows, { field: '', value: '' }])}
      >
        <Plus className="size-3" />
        {addLabel}
      </button>
    </div>
  );
}

/** 单列行编辑（list 元素 / set 成员）；reorder 提供 list 的上移/下移 */
function ListRowsEditor({
  label,
  rows,
  onChange,
  addLabel,
  reorder = false,
}: {
  label: string;
  rows: string[];
  onChange: (rows: string[]) => void;
  addLabel: string;
  reorder?: boolean;
}) {
  const update = (i: number, v: string) => onChange(rows.map((r, idx) => (idx === i ? v : r)));
  const move = (i: number, delta: -1 | 1) => {
    const j = i + delta;
    if (j < 0 || j >= rows.length) return;
    const next = [...rows];
    [next[i], next[j]] = [next[j]!, next[i]!];
    onChange(next);
  };
  return (
    <div className="flex flex-col gap-1.5">
      <div className="px-0.5 text-[10px] font-semibold uppercase tracking-wide text-foreground-subtlest select-none">
        {label}
      </div>
      <div className="scrollbar-slim flex max-h-72 flex-col gap-1 overflow-y-auto pr-0.5">
        {rows.map((row, i) => (
          <div key={i} className="flex items-center gap-1.5">
            {reorder ? (
              <span className="w-6 shrink-0 text-center font-mono text-[10px] text-foreground-subtlest select-none">
                {i}
              </span>
            ) : null}
            <TextInput
              value={row}
              onChange={(e) => update(i, e.target.value)}
              className="min-w-0 flex-1 font-mono"
            />
            {reorder ? (
              <>
                <button
                  type="button"
                  className={rowButtonClass}
                  disabled={i === 0}
                  onClick={() => move(i, -1)}
                  aria-label="move up"
                >
                  <ArrowUp className="size-3" />
                </button>
                <button
                  type="button"
                  className={rowButtonClass}
                  disabled={i === rows.length - 1}
                  onClick={() => move(i, 1)}
                  aria-label="move down"
                >
                  <ArrowDown className="size-3" />
                </button>
              </>
            ) : null}
            <button
              type="button"
              className={rowButtonClass}
              onClick={() => onChange(rows.filter((_, idx) => idx !== i))}
              aria-label="remove row"
            >
              <Trash2 className="size-3" />
            </button>
          </div>
        ))}
      </div>
      <button
        type="button"
        className="flex items-center gap-1 self-start rounded-md border border-border/60 px-2 py-1 text-ui-xs text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
        onClick={() => onChange([...rows, ''])}
      >
        <Plus className="size-3" />
        {addLabel}
      </button>
    </div>
  );
}

// ── TTL 修改 ───────────────────────────────────────────────────────────────

export function RedisTtlModal({
  sessionId,
  keyName,
  keyDb,
  currentDb,
  onClose,
  onChanged,
}: {
  sessionId: string;
  keyName: string;
  keyDb: string | undefined;
  /** 会话当前逻辑库（跨库时命令用 SELECT 夹心包裹） */
  currentDb: string | null;
  onClose: () => void;
  /** 任一修改成功后回调（父层刷新对象树） */
  onChanged: () => void;
}) {
  const { t } = useTranslation();
  const [ttl, setTtl] = useState<number | null>(null);
  const [input, setInput] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const quoted = redisQuote(keyName);

  const run = useCallback(
    (command: string) => runDbAwareCommand(sessionId, keyDb, currentDb, command),
    [sessionId, keyDb, currentDb],
  );

  const reload = useCallback(() => {
    setBusy(true);
    setError(null);
    run(`TTL ${quoted}`)
      .then((v) => setTtl(Number.parseInt(v, 10)))
      .catch((e) => setError(String(e)))
      .finally(() => setBusy(false));
  }, [run, quoted]);

  useEffect(() => {
    reload();
  }, [reload]);

  const apply = async (command: string) => {
    setBusy(true);
    setError(null);
    try {
      await run(command);
      onChanged();
      reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const seconds = Number.parseInt(input, 10);
  const inputValid = Number.isInteger(seconds) && seconds > 0;

  return (
    <Modal
      title={`${t('db.redisTtlTitle')} · ${keyName}`}
      onClose={onClose}
      className="w-[420px]"
      footer={
        <>
          <Button
            variant="ghost"
            size="lg"
            className="h-8 px-3 text-ui-base"
            disabled={busy || ttl === -1 || ttl === -2}
            onClick={() => void apply(`PERSIST ${quoted}`)}
          >
            {t('db.redisRemoveTtl')}
          </Button>
          <div className="flex-1" />
          <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={onClose}>
            {t('common.close', '关闭')}
          </Button>
          <Button
            size="lg"
            className="h-8 px-4 text-ui-base"
            disabled={busy || !inputValid}
            onClick={() => void apply(`EXPIRE ${quoted} ${seconds}`)}
          >
            {t('db.redisApplyTtl')}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        {error ? (
          <p className="break-all font-mono text-ui-xs text-destructive">{error}</p>
        ) : null}
        <div className="flex items-center gap-2 text-ui-xs text-foreground-subtle select-none">
          {t('db.redisCurrentTtl')}：
          {busy ? (
            <Loader2 className="size-3 animate-spin" />
          ) : ttl === -1 ? (
            <span className="font-mono text-foreground">{t('db.redisTtlForever')}</span>
          ) : (
            <span className="font-mono text-foreground">{ttl} s</span>
          )}
        </div>
        <FormField label={t('db.redisTtlSecondsLabel')} hint={t('db.redisTtlHint')}>
          <TextInput
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="3600"
            className="font-mono"
          />
        </FormField>
      </div>
    </Modal>
  );
}

// ── 删除键 ─────────────────────────────────────────────────────────────────

export function RedisKeyDeleteModal({
  sessionId,
  keyName,
  keyDb,
  currentDb,
  onClose,
  onDeleted,
}: {
  sessionId: string;
  keyName: string;
  keyDb: string | undefined;
  /** 会话当前逻辑库（跨库时命令用 SELECT 夹心包裹） */
  currentDb: string | null;
  onClose: () => void;
  onDeleted: () => void;
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleDelete = async () => {
    setBusy(true);
    setError(null);
    try {
      await runDbAwareCommand(sessionId, keyDb, currentDb, `DEL ${redisQuote(keyName)}`);
      onDeleted();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t('db.redisDeleteKeyTitle')}
      onClose={onClose}
      className="w-[400px]"
      footer={
        <>
          <div className="flex-1" />
          <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={onClose}>
            {t('common.cancel')}
          </Button>
          <Button
            variant="destructive"
            size="lg"
            className="h-8 px-4 text-ui-base"
            disabled={busy}
            onClick={() => void handleDelete()}
          >
            {busy ? <Loader2 className="size-3.5 animate-spin" /> : null}
            {t('db.redisDeleteKey')}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-2">
        <p className="text-ui-sm text-foreground">{t('db.redisDeleteConfirm')}</p>
        <p className="break-all rounded-md border border-border/60 bg-surface/40 px-2 py-1.5 font-mono text-ui-xs text-foreground">
          {keyName}
        </p>
        {error ? <p className="break-all font-mono text-ui-xs text-destructive">{error}</p> : null}
      </div>
    </Modal>
  );
}

// ── 慢日志 / 大 Key 分析 ───────────────────────────────────────────────────

export function RedisSlowlogModal({
  sessionId,
  onClose,
}: {
  sessionId: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [data, setData] = useState<RedisTableData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reloadTick, setReloadTick] = useState(0);

  useEffect(() => {
    let alive = true;
    redisSlowlog(sessionId, 200)
      .then((d) => alive && setData(d))
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, [sessionId, reloadTick]);

  return (
    <Modal
      title={t('db.redisSlowlogTitle')}
      onClose={onClose}
      className="w-[680px]"
      footer={
        <>
          <span className="min-w-0 flex-1 truncate text-ui-xs text-foreground-subtlest">
            {data ? `${data.rows.length} ${t('db.redisEntriesUnit')}` : ''}
          </span>
          <Button
            variant="ghost"
            size="lg"
            className="h-8 px-3 text-ui-base"
            onClick={() => setReloadTick((v) => v + 1)}
          >
            {t('dbHosts.refresh', '刷新')}
          </Button>
          <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={onClose}>
            {t('common.close', '关闭')}
          </Button>
        </>
      }
    >
      <div className="flex h-[380px] flex-col">
        {error ? <p className="break-all font-mono text-ui-xs text-destructive">{error}</p> : null}
        {data ? (
          data.headers.length > 0 ? (
            <DataTable className="flex-1" columns={data.headers} rows={data.rows} />
          ) : (
            <div className="flex flex-1 items-center justify-center text-ui-sm text-foreground-subtlest select-none">
              {error ?? t('db.noResults')}
            </div>
          )
        ) : (
          <div className="flex flex-1 items-center justify-center gap-2 text-ui-sm text-foreground-subtle select-none">
            {!error ? <Loader2 className="size-4 animate-spin" /> : null}
            {t('db.loadingDefinition')}
          </div>
        )}
      </div>
    </Modal>
  );
}

export function RedisBigkeysModal({
  sessionId,
  onClose,
}: {
  sessionId: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [data, setData] = useState<RedisBigkeysData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reloadTick, setReloadTick] = useState(0);

  useEffect(() => {
    let alive = true;
    redisBigkeys(sessionId, 1000, 50)
      .then((d) => alive && setData(d))
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, [sessionId, reloadTick]);

  return (
    <Modal
      title={t('db.redisBigkeysTitle')}
      onClose={onClose}
      className="w-[620px]"
      footer={
        <>
          <span className="min-w-0 flex-1 truncate text-ui-xs text-foreground-subtlest">
            {data
              ? t('db.redisScannedCount', { count: data.scanned }) +
                (data.truncated ? ` · ${t('db.redisSampleTruncated')}` : '')
              : ''}
          </span>
          <Button
            variant="ghost"
            size="lg"
            className="h-8 px-3 text-ui-base"
            onClick={() => setReloadTick((v) => v + 1)}
          >
            {t('dbHosts.refresh', '刷新')}
          </Button>
          <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={onClose}>
            {t('common.close', '关闭')}
          </Button>
        </>
      }
    >
      <div className="flex h-[380px] flex-col">
        {error ? <p className="break-all font-mono text-ui-xs text-destructive">{error}</p> : null}
        {data ? (
          <DataTable
            className="flex-1"
            columns={data.headers}
            rows={data.rows}
            emptyLabel={t('db.noResults')}
          />
        ) : (
          <div className="flex flex-1 items-center justify-center gap-2 text-ui-sm text-foreground-subtle select-none">
            {!error ? <Loader2 className="size-4 animate-spin" /> : null}
            {t('db.loadingDefinition')}
          </div>
        )}
      </div>
    </Modal>
  );
}
