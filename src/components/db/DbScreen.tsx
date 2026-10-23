/**
 * 数据库工作台页：多 tab 外壳（ZkScreen 同型，常驻挂载切页/切 tab 状态保留）。
 * tab 双击重命名；关 tab = 断开会话；加号下拉选已保存连接开新会话。
 * 数据库主机页「连接」经 window 事件 veloterm:db-connect-host 递入。
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ChevronDown, Database, Plus, X } from 'lucide-react';

import { DbSessionView } from '@/components/db/DbSessionView';
import {
  dbConnectionList,
  type DbConnectionRecord,
} from '@/services/dbConnectionsService';
import { useDismissOnOutsidePointer } from '@/hooks/useDismissOnOutsidePointer';
import { cn } from '@/lib/utils';

interface DbTab {
  key: number;
  customTitle: string | null;
  baseTitle: string | null;
  pendingConnection: DbConnectionRecord | null;
}

export function DbScreen({ isVisible = true }: { isVisible?: boolean }) {
  const { t } = useTranslation();
  const [tabs, setTabs] = useState<DbTab[]>([]);
  const [activeKey, setActiveKey] = useState(0);
  const nextKeyRef = useRef(1);
  const [renamingKey, setRenamingKey] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [connections, setConnections] = useState<DbConnectionRecord[]>([]);
  // tabs 的 ref 镜像：挂载期注册一次的 window 监听器里要读最新 tab 列表
  // （避免闭包陈旧导致「已有会话仍重复新建」）
  const tabsRef = useRef<DbTab[]>([]);
  useEffect(() => {
    tabsRef.current = tabs;
  }, [tabs]);

  // + 号下拉
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement | null>(null);
  useDismissOnOutsidePointer(menuOpen, [menuRef], () => setMenuOpen(false));

  const reloadConnections = useCallback(() => {
    void dbConnectionList()
      .then(setConnections)
      .catch(() => {
        // 库不可用时 + 退化为空（只能去主机页建连接）
      });
  }, []);

  useEffect(() => {
    reloadConnections();
  }, [reloadConnections]);

  const addTab = useCallback(
    (connection: DbConnectionRecord | null) => {
      const key = nextKeyRef.current;
      nextKeyRef.current += 1;
      setTabs((current) => [
        ...current,
        { key, customTitle: null, baseTitle: connection?.name ?? null, pendingConnection: connection },
      ]);
      setActiveKey(key);
    },
    [],
  );

  /** 打开连接：已有该连接的会话 tab 则激活复用，否则新建 tab */
  const openConnection = useCallback(
    (connectionId: number, create: () => void) => {
      const existing = tabsRef.current.find((tab) => tab.pendingConnection?.id === connectionId);
      if (existing) {
        setActiveKey(existing.key);
        return;
      }
      create();
    },
    [],
  );

  // 数据库主机页「连接」入口（window 自定义事件；本组件常驻挂载监听器始终在）
  useEffect(() => {
    const onConnect = (event: Event) => {
      const record = (event as CustomEvent<DbConnectionRecord>).detail;
      if (record && typeof record.id === 'number') {
        openConnection(record.id, () => addTab(record));
      }
    };
    window.addEventListener('veloterm:db-connect-host', onConnect);
    return () => window.removeEventListener('veloterm:db-connect-host', onConnect);
    // addTab/openConnection 恒稳定（ref + setState），挂载期注册一次
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const closeTab = (key: number) => {
    setTabs((current) => {
      const idx = current.findIndex((tab) => tab.key === key);
      const next = current.filter((tab) => tab.key !== key);
      if (key === activeKey) {
        const neighbor = next[Math.min(idx, next.length - 1)];
        setActiveKey(neighbor ? neighbor.key : 0);
      }
      return next;
    });
  };

  const commitRename = () => {
    if (renamingKey == null) return;
    const key = renamingKey;
    const value = renameValue.trim();
    setTabs((current) =>
      current.map((tab) =>
        tab.key === key ? { ...tab, customTitle: value.length > 0 ? value : null } : tab,
      ),
    );
    setRenamingKey(null);
  };

  const setBaseTitle = (key: number, title: string | null) => {
    setTabs((current) =>
      current.map((tab) => (tab.key === key ? { ...tab, baseTitle: title } : tab)),
    );
  };

  if (!isVisible && tabs.length === 0) {
    return null;
  }

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      {/* tab 栏 */}
      <div className="flex h-9 shrink-0 items-center gap-1 border-b border-border/50 px-2">
        <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
          {tabs.map((tab) => (
            <div
              key={tab.key}
              className={cn(
                'group/tab flex h-7 shrink-0 items-center gap-1 rounded-lg border px-2 transition-colors',
                tab.key === activeKey
                  ? 'border-border-hover bg-surface'
                  : 'border-transparent text-foreground-subtle hover:bg-surface-hover',
              )}
              onClick={() => setActiveKey(tab.key)}
              onDoubleClick={() => {
                setRenamingKey(tab.key);
                setRenameValue(tab.customTitle ?? tab.baseTitle ?? '');
              }}
            >
              <Database className="size-3.5 shrink-0" />
              {renamingKey === tab.key ? (
                <input
                  autoFocus
                  value={renameValue}
                  onChange={(e) => setRenameValue(e.target.value)}
                  onBlur={commitRename}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') commitRename();
                    if (e.key === 'Escape') setRenamingKey(null);
                  }}
                  onClick={(e) => e.stopPropagation()}
                  className="w-28 rounded border border-border bg-background px-1 text-ui-xs text-foreground outline-none"
                />
              ) : (
                <span className="max-w-40 truncate text-ui-sm select-none">
                  {tab.customTitle ?? tab.baseTitle ?? t('db.newTab')}
                </span>
              )}
              <button
                type="button"
                className="shrink-0 rounded p-0.5 text-foreground-subtlest opacity-0 transition-opacity group-hover/tab:opacity-100 hover:text-foreground"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(tab.key);
                }}
              >
                <X className="size-3" />
              </button>
            </div>
          ))}

          {/* + 号：下拉选已保存连接（工作台会话必须绑定已保存连接） */}
          <div ref={menuRef} className="relative shrink-0">
            <button
              type="button"
              aria-label={t('db.newTab')}
              onClick={() => {
                reloadConnections();
                setMenuOpen((v) => !v);
              }}
              className="flex size-7 items-center justify-center rounded-lg text-foreground-subtle transition-colors hover:bg-surface-hover hover:text-foreground"
            >
              <Plus className="size-4" />
            </button>
            {menuOpen ? (
              <div className="absolute left-0 top-full z-50 mt-1 min-w-56 overflow-hidden rounded-lg border border-border bg-menu p-1 shadow-lg">
                <div className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-foreground-subtlest select-none">
                  {t('db.savedConnections')}
                </div>
                {connections.length === 0 ? (
                  <div className="px-2 py-2 text-ui-xs text-foreground-subtlest">
                    {t('db.noSavedConnections')}
                  </div>
                ) : (
                  connections.map((conn) => (
                    <button
                      key={conn.id}
                      type="button"
                      onClick={() => {
                        setMenuOpen(false);
                        openConnection(conn.id, () => addTab(conn));
                      }}
                      className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-ui-sm text-foreground transition-colors hover:bg-menu-hover"
                    >
                      <Database className="size-3.5 shrink-0 text-terminal-cyan" />
                      <span className="min-w-0 flex-1 truncate">{conn.name}</span>
                      <span className="shrink-0 font-mono text-[10px] text-foreground-subtlest">
                        {conn.engine === 'dameng'
                          ? `dameng:${conn.compatibleMode === 'mysql' ? 'mysql' : 'oracle'}`
                          : conn.engine}
                      </span>
                    </button>
                  ))
                )}
                <div className="mt-1 flex items-center gap-1 border-t border-border/50 px-2 pt-1.5 text-ui-xs text-foreground-subtlest select-none">
                  <ChevronDown className="size-3" />
                  {t('db.goHostsHint')}
                </div>
              </div>
            ) : null}
          </div>
        </div>
      </div>

      {/* 会话视图（常驻隐藏，状态保留；卸载即断开会话） */}
      {tabs.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-1 select-none">
          <Database className="size-6 text-foreground-subtlest" />
          <span className="text-ui-sm text-foreground-subtlest">{t('db.noTabs')}</span>
          <span className="text-ui-xs text-foreground-subtlest/70">{t('db.noTabsHint')}</span>
        </div>
      ) : (
        tabs.map((tab) => (
          <div
            key={tab.key}
            className={cn('flex min-h-0 flex-1 flex-col', tab.key !== activeKey && 'hidden')}
          >
            <DbSessionView
              key={tab.key}
              pendingConnection={tab.pendingConnection}
              onBaseTitle={(title) => setBaseTitle(tab.key, title)}
            />
          </div>
        ))
      )}
    </div>
  );
}
