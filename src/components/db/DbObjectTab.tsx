/**
 * 数据库对象 Tab 页（DbScreen tab 栏的「查看定义 / 表详情」形态，终端 Tab
 * 同款交互）：从对象树右键/单击打开，独立 Tab 不覆盖会话的编辑器与结果
 * 面板；重复打开同一对象复用既有 Tab 并刷新内容。自取数——sessionId 由
 * 所属会话持有，会话 tab 关闭后已取到的内容仍在（快照）、刷新才报错。
 * 「插入调用模板 / 运行」经 onInsertSql/onRunSql 回写所属会话的 SQL
 * 编辑器（组件内嵌在会话结果面板，无需跳转）。
 */

import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  AlertCircle,
  Braces,
  Cable,
  Clock,
  Copy,
  Globe,
  HardDrive,
  Link,
  Loader2,
  Package,
  Play,
  Puzzle,
  RefreshCw,
  ScrollText,
  ShieldCheck,
  Sigma,
  Trash2,
  Type,
  User,
  Zap,
} from 'lucide-react';

import { DataTable } from '@/components/ui/DataTable';
import { ReadOnlySqlView } from '@/components/db/ReadOnlySqlView';
import { nodeIcon } from '@/components/db/DbSessionView';
import {
  dbObjectDefinition,
  dbTableDetail,
  type ObjectDefinition,
  type TableDetail,
} from '@/services/dbConnectionsService';
import { cn } from '@/lib/utils';

/** 会话编辑器回写入口（内嵌在会话结果面板，直接调用） */
export interface DbSessionApi {
  /** 模板写入编辑器（不执行） */
  insertSql: (sql: string) => void;
  /** 写入编辑器并立即执行 */
  runSql: (sql: string) => void;
}

/** 会话视图发起「打开对象 tab」的载荷（sessionKey 由 DbScreen 补齐） */
export interface OpenObjectTabRequest {
  sessionId: string;
  schema: string;
  name: string;
  kind: string;
  variant: 'definition' | 'tableDetail';
}

/** 对象 tab 的载荷（DbScreen 持有；refreshToken 递增触发重新取数） */
export interface DbObjectTabPayload {
  sessionId: string;
  schema: string;
  name: string;
  kind: string;
  variant: 'definition' | 'tableDetail';
  refreshToken: number;
}

type DefinitionKind = Parameters<typeof dbObjectDefinition>[3];

/** 定义类对象的标题图标（原 DbSessionView 定义视图同款映射） */
function definitionIcon(kind: string) {
  switch (kind) {
    case 'function':
      return <Sigma className="size-3.5 shrink-0 text-terminal-magenta" />;
    case 'trigger':
      return <Zap className="size-3.5 shrink-0 text-terminal-red" />;
    case 'synonym':
      return <Link className="size-3.5 shrink-0 text-terminal-cyan" />;
    case 'package':
      return <Package className="size-3.5 shrink-0 text-terminal-yellow" />;
    case 'event':
      return <Clock className="size-3.5 shrink-0 text-terminal-blue" />;
    case 'rule':
      return <ScrollText className="size-3.5 shrink-0 text-terminal-magenta" />;
    case 'ftable':
      return <Globe className="size-3.5 shrink-0 text-terminal-blue" />;
    case 'extension':
      return <Puzzle className="size-3.5 shrink-0 text-terminal-cyan" />;
    case 'dblink':
      return <Cable className="size-3.5 shrink-0 text-terminal-yellow" />;
    case 'recycle':
      return <Trash2 className="size-3.5 shrink-0 text-foreground-subtlest" />;
    case 'datatype':
      return <Type className="size-3.5 shrink-0 text-terminal-magenta" />;
    case 'tablespace':
      return <HardDrive className="size-3.5 shrink-0 text-terminal-cyan" />;
    case 'user':
      return <User className="size-3.5 shrink-0 text-terminal-green" />;
    case 'role':
      return <ShieldCheck className="size-3.5 shrink-0 text-terminal-yellow" />;
    default:
      return <Braces className="size-3.5 shrink-0 text-terminal-magenta" />;
  }
}

export function DbObjectTab({
  payload,
  active = true,
  onInsertSql,
  onRunSql,
  onClose,
}: {
  payload: DbObjectTabPayload;
  /** 是否激活（keep-alive 非激活页 hidden）：透传给 ReadOnlySqlView，
   * 共享 Monaco 实例由激活者独占（搬 DOM + setModel） */
  active?: boolean;
  /** 模板写入所属会话的 SQL 编辑器（不执行） */
  onInsertSql: (sql: string) => void;
  /** 写入所属会话编辑器并立即执行 */
  onRunSql: (sql: string) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [definition, setDefinition] = useState<ObjectDefinition | null>(null);
  const [detail, setDetail] = useState<TableDetail | null>(null);
  const [subTab, setSubTab] = useState<'ddl' | 'columns' | 'indexes' | 'constraints' | 'references'>('ddl');
  const [copied, setCopied] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  // 本地刷新计数（头部刷新按钮）：与外部 refreshToken（重复打开）叠加触发重取
  const [localRefresh, setLocalRefresh] = useState(0);

  // 自取数：首次挂载与 refreshToken 变化（重复打开同一对象）都重新拉取
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    setNotice(null);
    if (payload.variant === 'definition') {
      dbObjectDefinition(
        payload.sessionId,
        payload.schema,
        payload.name,
        payload.kind as DefinitionKind,
      )
        .then((def) => {
          if (!cancelled) setDefinition(def);
        })
        .catch((e) => {
          if (!cancelled) setError(String(e));
        })
        .finally(() => {
          if (!cancelled) setLoading(false);
        });
    } else {
      dbTableDetail(
        payload.sessionId,
        payload.schema,
        payload.name,
        payload.kind === 'view' ? 'view' : 'table',
      )
        .then((d) => {
          if (!cancelled) {
            setDetail(d);
            setSubTab('ddl');
          }
        })
        .catch((e) => {
          if (!cancelled) setError(String(e));
        })
        .finally(() => {
          if (!cancelled) setLoading(false);
        });
    }
    return () => {
      cancelled = true;
    };
  }, [payload.sessionId, payload.schema, payload.name, payload.kind, payload.variant, payload.refreshToken, localRefresh]);

  const copyText = (text: string) => {
    void navigator.clipboard
      .writeText(text)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      })
      .catch(() => undefined);
  };

  if (loading) {
    return (
      <div className="flex flex-1 items-center justify-center gap-2 text-ui-sm text-foreground-subtle select-none">
        <Loader2 className="size-4 animate-spin" />
        {t('db.loadingDefinition')}
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-4">
        <AlertCircle className="size-5 text-destructive" />
        <pre className="max-w-lg whitespace-pre-wrap break-all text-center font-mono text-ui-xs text-destructive">
          {error}
        </pre>
      </div>
    );
  }

  if (payload.variant === 'tableDetail' && detail) {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border/50 bg-surface/30 px-2 select-none">
          {nodeIcon(payload.kind === 'view' ? 'view' : 'table')}
          {/* 标题截断不在此处加 tooltip：tab 页已有同名 tooltip，正文头部再加会双弹 */}
          <span className="max-w-44 truncate font-mono text-ui-xs font-semibold text-foreground">
            {payload.name}
          </span>
          {detail.comment ? (
            <span className="hidden max-w-52 truncate rounded bg-surface px-1.5 py-px text-ui-xs text-foreground-subtle lg:inline-block">
              {detail.comment}
            </span>
          ) : null}
          {detail.size ? (
            <span className="shrink-0 rounded bg-primary/10 px-1.5 py-px font-mono text-[9px] font-semibold text-primary">
              {detail.size}
            </span>
          ) : null}
          {detail.rowCount ? (
            <span className="shrink-0 rounded bg-surface px-1.5 py-px font-mono text-[9px] text-foreground-subtle">
              {detail.rowCount} rows
            </span>
          ) : null}
          <span className="flex items-center gap-0.5">
            {(['ddl', 'columns', 'indexes', 'constraints', 'references'] as const)
              .filter(
                (tab) =>
                  (tab !== 'constraints' || detail.constraintHeaders.length > 0) &&
                  (tab !== 'references' || detail.referenceHeaders.length > 0),
              )
              .map((tab) => (
                <button
                  key={tab}
                  type="button"
                  onClick={() => setSubTab(tab)}
                  className={cn(
                    'rounded-md border px-2 py-0.5 text-ui-xs transition-colors',
                    subTab === tab
                      ? 'border-border bg-surface font-semibold text-foreground'
                      : 'border-transparent text-foreground-subtle hover:bg-hover hover:text-foreground',
                  )}
                >
                  {t(`db.tab_${tab}`)}
                </button>
              ))}
          </span>
          <div className="flex-1" />
          <button
            type="button"
            onClick={() => setLocalRefresh((v) => v + 1)}
            className="flex items-center gap-1 rounded-md px-2 py-0.5 text-ui-xs text-foreground-subtle hover:bg-hover hover:text-foreground"
          >
            <RefreshCw className="size-3" />
            {t('db.refreshObject')}
          </button>
          {subTab === 'ddl' ? (
            <button
              type="button"
              onClick={() => copyText(detail.ddl)}
              className="flex items-center gap-1 rounded-md px-2 py-0.5 text-ui-xs text-foreground-subtle hover:bg-hover hover:text-foreground"
            >
              {copied ? (
                <span className="text-terminal-green">{t('files.copied', '已复制')}</span>
              ) : (
                <>
                  <Copy className="size-3" />
                  <span>{t('db.copyDefinition')}</span>
                </>
              )}
            </button>
          ) : null}
          <button
            type="button"
            onClick={onClose}
            className="rounded p-1 text-foreground-subtlest hover:text-foreground"
            aria-label={t('common.close', '关闭')}
          >
            ✕
          </button>
        </div>
        {subTab === 'ddl' ? (
          // DDL 用 Monaco 只读渲染（SQL 高亮 + 折行）；共享单实例，
          // 刷新走 setValue 不重建
          <ReadOnlySqlView
            active={active}
            content={detail.ddl}
            className="scrollbar-slim min-h-0 flex-1"
          />
        ) : subTab === 'columns' ? (
          <DataTable className="flex-1" columns={detail.columnHeaders} rows={detail.columns} />
        ) : subTab === 'indexes' ? (
          <DataTable
            className="flex-1"
            columns={detail.indexHeaders}
            rows={detail.indexes}
            emptyLabel={t('db.noIndexes')}
          />
        ) : subTab === 'references' ? (
          <DataTable
            className="flex-1"
            columns={detail.referenceHeaders}
            rows={detail.references}
            emptyLabel={t('db.noReferences')}
          />
        ) : (
          <DataTable
            className="flex-1"
            columns={detail.constraintHeaders}
            rows={detail.constraints}
            emptyLabel={t('db.noConstraints')}
          />
        )}
      </div>
    );
  }

  if (!definition) {
    return (
      <div className="flex flex-1 items-center justify-center text-ui-sm text-foreground-subtlest select-none">
        {t('db.noResults')}
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border/50 bg-surface/30 px-2 select-none">
        {definitionIcon(payload.kind)}
        <span className="max-w-44 truncate font-mono text-ui-xs font-semibold text-foreground">
          {payload.name}
        </span>
        <span className="text-ui-xs text-foreground-subtle">{t('db.objectDefinition')}</span>
        {definition.argCount > 0 ? (
          <span className="rounded bg-surface px-1.5 py-px font-mono text-[9px] text-foreground-subtle">
            {t('db.argCount', { count: definition.argCount })}
          </span>
        ) : null}
        <div className="flex-1" />
        <button
          type="button"
          onClick={() => setLocalRefresh((v) => v + 1)}
          className="flex items-center gap-1 rounded-md px-2 py-0.5 text-ui-xs text-foreground-subtle hover:bg-hover hover:text-foreground"
        >
          <RefreshCw className="size-3" />
          {t('db.refreshObject')}
        </button>
        <button
          type="button"
          onClick={() => copyText(definition.definition)}
          className="flex items-center gap-1 rounded-md px-2 py-0.5 text-ui-xs text-foreground-subtle hover:bg-hover hover:text-foreground"
          title={t('files.copyCode', '复制')}
        >
          {copied ? (
            <span className="text-terminal-green">{t('files.copied', '已复制')}</span>
          ) : (
            <>
              <Copy className="size-3" />
              <span>{t('db.copyDefinition')}</span>
            </>
          )}
        </button>
        <button
          type="button"
          onClick={() => {
            onInsertSql(definition.callTemplate);
            setNotice(t('db.templateInserted'));
          }}
          className="rounded-md px-2 py-0.5 text-ui-xs text-foreground-subtle hover:bg-hover hover:text-foreground disabled:opacity-40"
        >
          {t('db.insertCallTemplate')}
        </button>
        <button
          type="button"
          onClick={() => {
            const { callTemplate, argCount } = definition;
            if (argCount > 0) {
              // 有参数：模板进编辑器引导补参，不直接执行（空参会报错）
              onInsertSql(callTemplate);
              setNotice(t('db.needsArgs', { count: argCount }));
            } else {
              onRunSql(callTemplate);
            }
          }}
          className="flex items-center gap-1 rounded-md bg-surface px-2 py-0.5 text-ui-xs font-medium text-foreground hover:bg-hover disabled:opacity-40"
        >
          <Play className="size-3 text-terminal-green fill-terminal-green" />
          {definition.argCount > 0 ? t('db.fillArgsAndRun') : t('db.runCall')}
        </button>
        <button
          type="button"
          onClick={onClose}
          className="rounded p-1 text-foreground-subtlest hover:text-foreground"
          aria-label={t('common.close', '关闭')}
        >
          ✕
        </button>
      </div>
      {notice ? (
        <div className="shrink-0 border-b border-border/40 bg-surface/20 px-3 py-1 text-ui-xs text-foreground-subtle select-none">
          {notice}
        </div>
      ) : null}
      <ReadOnlySqlView
        active={active}
        content={definition.definition}
        className="min-h-0 flex-1"
      />
    </div>
  );
}
