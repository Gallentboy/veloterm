import {
  BookOpen,
  Command,
  CornerDownLeft,
  Cylinder,
  Navigation,
  Pin,
  Search,
  Server,
} from 'lucide-react';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { createPortal } from 'react-dom';

import { useCommands } from '@/services/commandStore';
import {
  extractPlaceholders,
  type CommandRecord,
} from '@/services/commandService';
import { sendToTerminal } from '@/services/aiTerminalTools';
import { useTerminalStore } from '@/terminal/TerminalProvider';
import { commandStore } from '@/services/commandStore';
import { useHostsStore } from '@/hosts/HostsProvider';
import type { HostNode } from '@/hosts/types';
import {
  dbConnectionList,
  type DbConnectionRecord,
} from '@/services/dbConnectionsService';
import {
  flattenManualSections,
  getManualChapters,
  type FlatSectionItem,
} from '@/manual';
import { SIDEBAR_NAV_ITEMS, type Page } from '@/components/WorkspaceSidebar';
import { useSftpStore } from '@/services/SftpProvider';
import { cn } from '@/lib/utils';

export type PaletteCategory = 'all' | 'hosts' | 'dbs' | 'commands' | 'manual' | 'navigation';

interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
  onNavigate?: (page: Page) => void;
  onOpenManual?: (sectionId?: string) => void;
}

type PaletteItem =
  | {
      id: string;
      type: 'command';
      category: 'commands';
      title: string;
      subtitle: string;
      keywords: string[];
      isPinned?: boolean;
      dangerLevel: string;
      envLevel?: string;
      raw: CommandRecord;
    }
  | {
      id: string;
      type: 'host';
      category: 'hosts';
      title: string;
      subtitle: string;
      keywords: string[];
      envLevel?: string;
      raw: HostNode;
    }
  | {
      id: string;
      type: 'db';
      category: 'dbs';
      title: string;
      subtitle: string;
      keywords: string[];
      envLevel?: string;
      raw: DbConnectionRecord;
    }
  | {
      id: string;
      type: 'manual';
      category: 'manual';
      title: string;
      subtitle: string;
      keywords: string[];
      envLevel?: string;
      raw: FlatSectionItem;
    }
  | {
      id: string;
      type: 'navigation';
      category: 'navigation';
      title: string;
      subtitle: string;
      keywords: string[];
      envLevel?: string;
      rawPage: Page;
    };

const CATEGORIES: { id: PaletteCategory; labelKey: string; prefix?: string }[] = [
  { id: 'all', labelKey: 'commands.palette.catAll' },
  { id: 'hosts', labelKey: 'commands.palette.catHosts', prefix: '@' },
  { id: 'dbs', labelKey: 'commands.palette.catDbs', prefix: '#' },
  { id: 'commands', labelKey: 'commands.palette.catCommands', prefix: '>' },
  { id: 'manual', labelKey: 'commands.palette.catManual', prefix: '?' },
  { id: 'navigation', labelKey: 'commands.palette.catNav', prefix: '/' },
];

/**
 * 全局全能快捷面板（Raycast 风格双栏 Omnibar）：
 * 支持命令、远程主机、数据库连接、使用手册、功能页面全域搜索与快捷动作。
 */
export function CommandPalette({ open, onClose, onNavigate, onOpenManual }: CommandPaletteProps) {
  const { t, i18n } = useTranslation();
  const commands = useCommands();
  const { hosts } = useHostsStore();
  const terminalStore = useTerminalStore();
  const sftpStore = useSftpStore();
  const activeTerminalKey = terminalStore.activeSessionId;

  const [dbConnections, setDbConnections] = useState<DbConnectionRecord[]>([]);
  const [keyword, setKeyword] = useState('');
  const [activeCategory, setActiveCategory] = useState<PaletteCategory>('all');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [copied, setCopied] = useState(false);

  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // 每次打开时拉取最新数据库连接列表并重置状态
  useEffect(() => {
    if (open) {
      setKeyword('');
      setActiveCategory('all');
      setSelectedIndex(0);
      setCopied(false);
      void dbConnectionList()
        .then((list) => setDbConnections(list))
        .catch(() => setDbConnections([]));
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  // 手册全量小节
  const manualSections = useMemo(
    () => flattenManualSections(getManualChapters(i18n.language)),
    [i18n.language],
  );

  // 解析前缀语法：@ 主机, # 数据库, > 命令, ? 手册, / 导航
  const { effectiveCategory, queryText } = useMemo(() => {
    const trimmed = keyword.trim();
    if (trimmed.startsWith('@')) {
      return { effectiveCategory: 'hosts' as PaletteCategory, queryText: trimmed.slice(1).trim() };
    }
    if (trimmed.startsWith('#')) {
      return { effectiveCategory: 'dbs' as PaletteCategory, queryText: trimmed.slice(1).trim() };
    }
    if (trimmed.startsWith('>')) {
      return { effectiveCategory: 'commands' as PaletteCategory, queryText: trimmed.slice(1).trim() };
    }
    if (trimmed.startsWith('?') || trimmed.startsWith('？')) {
      return { effectiveCategory: 'manual' as PaletteCategory, queryText: trimmed.slice(1).trim() };
    }
    if (trimmed.startsWith('/') || trimmed.startsWith('、')) {
      return { effectiveCategory: 'navigation' as PaletteCategory, queryText: trimmed.slice(1).trim() };
    }
    return { effectiveCategory: activeCategory, queryText: trimmed };
  }, [keyword, activeCategory]);

  // 汇聚全域搜索源数据
  const allItems = useMemo<PaletteItem[]>(() => {
    const list: PaletteItem[] = [];

    // 1. 命令库
    for (const cmd of commands) {
      list.push({
        id: `cmd-${cmd.id}`,
        type: 'command',
        category: 'commands',
        title: cmd.title,
        subtitle: cmd.command,
        keywords: [cmd.title, cmd.command, cmd.description, ...cmd.tags],
        isPinned: cmd.isPinned,
        dangerLevel: cmd.dangerLevel,
        raw: cmd,
      });
    }

    // 2. 远程主机
    for (const h of hosts) {
      list.push({
        id: `host-${h.id}`,
        type: 'host',
        category: 'hosts',
        title: h.name,
        subtitle: `${h.username}@${h.host}:${h.port}`,
        keywords: [h.name, h.host, h.username, String(h.port), h.envLevel ?? ''],
        envLevel: h.envLevel,
        raw: h,
      });
    }

    // 3. 数据库连接
    for (const db of dbConnections) {
      list.push({
        id: `db-${db.id}`,
        type: 'db',
        category: 'dbs',
        title: db.name,
        subtitle: `${db.engine.toUpperCase()} · ${db.host}:${db.port}${db.database ? `/${db.database}` : ''}`,
        keywords: [db.name, db.host, db.engine, db.username ?? '', db.database ?? ''],
        envLevel: db.envLevel,
        raw: db,
      });
    }

    // 4. 使用手册小节
    for (const m of manualSections) {
      list.push({
        id: `manual-${m.section.id}`,
        type: 'manual',
        category: 'manual',
        title: m.section.title,
        subtitle: m.chapter.title,
        keywords: [m.section.title, m.chapter.title, ...(m.section.keywords ?? [])],
        raw: m,
      });
    }

    // 5. 页面导航
    for (const nav of SIDEBAR_NAV_ITEMS) {
      const pageTitle = t(nav.navKey);
      list.push({
        id: `nav-${nav.page}`,
        type: 'navigation',
        category: 'navigation',
        title: pageTitle,
        subtitle: t('commands.palette.navAction', '跳转到页面'),
        keywords: [pageTitle, nav.page, 'navigation', 'page', 'goto'],
        rawPage: nav.page,
      });
    }

    return list;
  }, [commands, hosts, dbConnections, manualSections, t]);

  // 过滤与排序
  const filteredItems = useMemo(() => {
    let pool = allItems;
    if (effectiveCategory !== 'all') {
      pool = pool.filter((item) => item.category === effectiveCategory);
    }
    if (!queryText) {
      // 无输入时优先展示置顶命令、主机及关键功能页面
      return pool.sort((a, b) => {
        if (a.type === 'command' && a.isPinned && !(b.type === 'command' && b.isPinned)) return -1;
        if (!(a.type === 'command' && a.isPinned) && b.type === 'command' && b.isPinned) return 1;
        return 0;
      });
    }
    const q = queryText.toLowerCase();
    return pool.filter((item) => {
      if (item.title.toLowerCase().includes(q)) return true;
      if (item.subtitle.toLowerCase().includes(q)) return true;
      return item.keywords.some((k) => k.toLowerCase().includes(q));
    });
  }, [allItems, effectiveCategory, queryText]);

  // 保持当前选中的 item 游标安全
  const selectedItem = filteredItems[selectedIndex] ?? filteredItems[0] ?? null;

  useEffect(() => {
    setSelectedIndex(0);
  }, [effectiveCategory, queryText]);

  // 滚动到选中项
  useEffect(() => {
    const el = listRef.current?.children[selectedIndex] as HTMLElement | undefined;
    el?.scrollIntoView({ block: 'nearest' });
  }, [selectedIndex]);

  // 复制当前项核心信息（Alt+C 快捷键）
  const handleCopyCurrent = useCallback(async () => {
    if (!selectedItem) return;
    let textToCopy = '';
    if (selectedItem.type === 'command') {
      textToCopy = selectedItem.raw.command;
    } else if (selectedItem.type === 'host') {
      textToCopy = `ssh -p ${selectedItem.raw.port} ${selectedItem.raw.username}@${selectedItem.raw.host}`;
    } else if (selectedItem.type === 'db') {
      textToCopy = `${selectedItem.raw.host}:${selectedItem.raw.port}`;
    } else if (selectedItem.type === 'manual') {
      textToCopy = selectedItem.raw.section.title;
    } else {
      textToCopy = selectedItem.title;
    }
    try {
      await navigator.clipboard.writeText(textToCopy);
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {
      // ignore
    }
  }, [selectedItem]);

  // 执行主动作（Enter）
  const handleExecute = useCallback(
    async (item: PaletteItem) => {
      onClose();
      if (item.type === 'command') {
        const placeholders = extractPlaceholders(item.raw.command);
        if (placeholders.length === 0 && item.raw.dangerLevel === 'safe' && activeTerminalKey != null) {
          const result = await sendToTerminal(activeTerminalKey, `${item.raw.command}\n`);
          if (result.ok) {
            void commandStore.used(item.raw.id);
          }
        } else {
          onNavigate?.('commands');
        }
      } else if (item.type === 'host') {
        terminalStore.createSession({ hostId: item.raw.id, customTitle: item.raw.name });
        onNavigate?.('terminal');
      } else if (item.type === 'db') {
        window.dispatchEvent(new CustomEvent('veloterm:db-connect-host', { detail: item.raw }));
        onNavigate?.('db');
      } else if (item.type === 'manual') {
        onOpenManual?.(item.raw.section.id);
      } else if (item.type === 'navigation') {
        onNavigate?.(item.rawPage);
      }
    },
    [activeTerminalKey, onClose, onNavigate, onOpenManual, terminalStore],
  );

  // 辅助动作（⌘Enter：对主机直接打开 SFTP）
  const handleSecondaryAction = useCallback(
    (item: PaletteItem) => {
      if (item.type === 'host') {
        onClose();
        sftpStore.createSession({ hostId: item.raw.id, hostName: item.raw.name });
        onNavigate?.('files');
      }
    },
    [onClose, onNavigate, sftpStore],
  );

  // 键盘快捷键监听
  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, filteredItems.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Tab') {
      e.preventDefault();
      const currIdx = CATEGORIES.findIndex((c) => c.id === effectiveCategory);
      const nextIdx = e.shiftKey
        ? (currIdx - 1 + CATEGORIES.length) % CATEGORIES.length
        : (currIdx + 1) % CATEGORIES.length;
      setActiveCategory(CATEGORIES[nextIdx]?.id ?? 'all');
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (!selectedItem) return;
      if (e.metaKey || e.ctrlKey) {
        handleSecondaryAction(selectedItem);
      } else {
        void handleExecute(selectedItem);
      }
    } else if (e.altKey && e.code === 'KeyC') {
      e.preventDefault();
      void handleCopyCurrent();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
    }
  };

  const [mainContainer, setMainContainer] = useState<HTMLElement | null>(null);
  useEffect(() => {
    setMainContainer(document.querySelector<HTMLElement>('main'));
  }, []);

  if (!open) return null;

  const overlay = (
    <div
      className={cn(
        'z-[80] flex items-start justify-center bg-black/40 backdrop-blur-xs pt-[10vh] px-4',
        mainContainer ? 'absolute inset-0' : 'fixed inset-0',
      )}
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-label={t('commands.palette.title')}
    >
      <div
        className="flex w-[780px] max-w-full h-[520px] max-h-[85vh] flex-col overflow-hidden rounded-2xl border border-border bg-popover shadow-2xl card-glossy"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 顶部搜索框 */}
        <div className="flex items-center gap-2.5 border-b border-border/50 px-3.5 py-3 bg-surface/20">
          <Search className="size-4.5 shrink-0 text-foreground-subtle" />
          {effectiveCategory !== 'all' ? (
            <span className="flex items-center gap-1 rounded bg-primary/10 px-2 py-0.5 font-mono text-ui-xs font-semibold text-primary border border-primary/20 select-none">
              {t(CATEGORIES.find((c) => c.id === effectiveCategory)?.labelKey ?? '')}
            </span>
          ) : null}
          <input
            ref={inputRef}
            value={keyword}
            onChange={(e) => {
              setKeyword(e.target.value);
              setSelectedIndex(0);
            }}
            onKeyDown={handleKeyDown}
            placeholder={t(
              'commands.palette.placeholderOmni',
              '搜索主机、数据库、命令、手册或输入 @ # > ? / 前缀...',
            )}
            className="min-w-0 flex-1 bg-transparent text-ui-base text-foreground outline-none placeholder:text-foreground-subtlest"
          />
          <kbd className="rounded border border-border bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtle select-none">
            ESC
          </kbd>
        </div>

        {/* 分类切换 Chips 栏 */}
        <div className="flex shrink-0 items-center gap-1 border-b border-border/40 px-3 py-1.5 bg-surface/30 text-ui-xs select-none overflow-x-auto scrollbar-hair">
          {CATEGORIES.map((cat) => (
            <button
              key={cat.id}
              type="button"
              onClick={() => {
                setActiveCategory(cat.id);
                inputRef.current?.focus();
              }}
              className={cn(
                'flex items-center gap-1 rounded-full px-2.5 py-0.5 font-medium transition-colors',
                effectiveCategory === cat.id
                  ? 'bg-selected text-foreground font-semibold shadow-2xs border border-border/60'
                  : 'text-foreground-subtle hover:bg-surface-hover hover:text-foreground border border-transparent',
              )}
            >
              {cat.prefix ? (
                <span className="font-mono text-ui-xs text-primary font-bold">{cat.prefix}</span>
              ) : null}
              <span>{t(cat.labelKey)}</span>
            </button>
          ))}
          <span className="ml-auto text-[11px] text-foreground-subtlest hidden sm:inline">
            Tab 切换分类
          </span>
        </div>

        {/* 双栏主体区 */}
        <div className="flex min-h-0 flex-1">
          {/* 左侧列表栏 */}
          <div
            ref={listRef}
            className="w-[54%] shrink-0 border-r border-border/40 overflow-y-auto p-1.5 flex flex-col gap-0.5"
          >
            {filteredItems.length === 0 ? (
              <div className="flex flex-1 flex-col items-center justify-center py-16 select-none text-foreground-subtlest">
                <Search className="size-8 opacity-30 mb-2" />
                <span className="text-ui-sm">{t('commands.palette.noResults')}</span>
              </div>
            ) : (
              filteredItems.map((item, index) => {
                const isSelected = index === selectedIndex;
                return (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => void handleExecute(item)}
                    onMouseEnter={() => setSelectedIndex(index)}
                    className={cn(
                      'flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-left transition-colors relative',
                      isSelected
                        ? 'bg-selected text-foreground'
                        : 'hover:bg-surface-hover/70 text-foreground-subtle',
                    )}
                  >
                    {/* 左侧选中竖线微指示 */}
                    {isSelected ? (
                      <span className="absolute left-0 top-1.5 bottom-1.5 w-1 rounded-r-full bg-primary" />
                    ) : null}

                    {/* 类型图标 */}
                    <div className="flex size-7 shrink-0 items-center justify-center rounded-md bg-surface border border-border/40 text-foreground">
                      {item.type === 'command' ? (
                        <Command className="size-3.5 text-terminal-yellow" />
                      ) : item.type === 'host' ? (
                        <Server className="size-3.5 text-terminal-green" />
                      ) : item.type === 'db' ? (
                        <Cylinder className="size-3.5 text-terminal-blue" />
                      ) : item.type === 'manual' ? (
                        <BookOpen className="size-3.5 text-terminal-magenta" />
                      ) : (
                        <Navigation className="size-3.5 text-terminal-cyan" />
                      )}
                    </div>

                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-1.5">
                        {item.type === 'command' && item.isPinned ? (
                          <Pin className="size-2.5 shrink-0 text-primary" />
                        ) : null}
                        <span className="truncate text-ui-sm font-medium text-foreground">
                          {item.title}
                        </span>
                        {item.envLevel ? (
                          <span
                            className={cn(
                              'shrink-0 rounded px-1 py-0.2 font-mono text-[9px] font-bold uppercase',
                              item.envLevel === 'prod'
                                ? 'bg-red-500/15 text-red-500 border border-red-500/30'
                                : item.envLevel === 'staging'
                                  ? 'bg-amber-500/15 text-amber-500 border border-amber-500/30'
                                  : 'bg-surface text-foreground-subtlest border border-border/40',
                            )}
                          >
                            {item.envLevel}
                          </span>
                        ) : null}
                      </div>
                      <div className="truncate font-mono text-[11px] text-foreground-subtle/80">
                        {item.subtitle}
                      </div>
                    </div>

                    {isSelected ? (
                      <CornerDownLeft className="size-3 shrink-0 text-foreground-subtlest" />
                    ) : null}
                  </button>
                );
              })
            )}
          </div>

          {/* 右侧详情预览面板（Raycast 质感） */}
          <div className="w-[46%] min-w-0 bg-surface/20 p-4 flex flex-col justify-between overflow-y-auto">
            {selectedItem ? (
              <div className="flex flex-col gap-3 min-h-0">
                {/* 标题区 */}
                <div className="flex items-start gap-2.5">
                  <div className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-surface border border-border/60 shadow-xs">
                    {selectedItem.type === 'command' ? (
                      <Command className="size-5 text-terminal-yellow" />
                    ) : selectedItem.type === 'host' ? (
                      <Server className="size-5 text-terminal-green" />
                    ) : selectedItem.type === 'db' ? (
                      <Cylinder className="size-5 text-terminal-blue" />
                    ) : selectedItem.type === 'manual' ? (
                      <BookOpen className="size-5 text-terminal-magenta" />
                    ) : (
                      <Navigation className="size-5 text-terminal-cyan" />
                    )}
                  </div>
                  <div className="min-w-0 flex-1">
                    <h3 className="truncate text-ui-base font-bold text-foreground">
                      {selectedItem.title}
                    </h3>
                    <p className="truncate text-ui-xs text-foreground-subtle">
                      {selectedItem.subtitle}
                    </p>
                  </div>
                </div>

                {/* 详情根据类型渲染 */}
                {selectedItem.type === 'command' && (
                  <div className="flex flex-col gap-2.5 min-h-0">
                    <div className="flex flex-wrap gap-1.5">
                      <span
                        className={cn(
                          'rounded px-1.5 py-0.5 text-[10px] font-bold uppercase border',
                          selectedItem.raw.dangerLevel === 'confirm'
                            ? 'bg-warning/15 text-warning border-warning/30'
                            : 'bg-terminal-green/15 text-terminal-green border-terminal-green/30',
                        )}
                      >
                        {selectedItem.raw.dangerLevel}
                      </span>
                      {selectedItem.raw.tags.map((tag) => (
                        <span
                          key={tag}
                          className="rounded bg-surface px-1.5 py-0.5 text-[10px] text-foreground-subtle border border-border/40"
                        >
                          #{tag}
                        </span>
                      ))}
                    </div>
                    {selectedItem.raw.description ? (
                      <p className="text-ui-xs leading-relaxed text-foreground-subtle">
                        {selectedItem.raw.description}
                      </p>
                    ) : null}
                    <div className="flex flex-col gap-1">
                      <span className="text-[10px] uppercase font-mono font-semibold tracking-wider text-foreground-subtlest">
                        Command
                      </span>
                      <pre className="max-h-36 overflow-auto rounded-lg border border-border/60 bg-surface/60 p-2.5 font-mono text-ui-xs text-foreground whitespace-pre-wrap break-all select-text font-ligatures">
                        {selectedItem.raw.command}
                      </pre>
                    </div>
                  </div>
                )}

                {selectedItem.type === 'host' && (
                  <div className="flex flex-col gap-2.5 text-ui-xs">
                    <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 rounded-lg border border-border/50 bg-surface/40 p-2.5 font-mono">
                      <span className="text-foreground-subtlest">Host:</span>
                      <span className="text-foreground truncate select-text">{selectedItem.raw.host}</span>
                      <span className="text-foreground-subtlest">Port:</span>
                      <span className="text-foreground">{selectedItem.raw.port}</span>
                      <span className="text-foreground-subtlest">User:</span>
                      <span className="text-foreground select-text">{selectedItem.raw.username}</span>
                      <span className="text-foreground-subtlest">Auth:</span>
                      <span className="text-foreground uppercase">{selectedItem.raw.auth.method}</span>
                      {selectedItem.raw.envLevel ? (
                        <>
                          <span className="text-foreground-subtlest">Env:</span>
                          <span className="flex items-center gap-1.5">
                            {selectedItem.raw.envLevel === 'prod' ? (
                              <span className="size-1.5 rounded-full bg-red-500 status-pulse-rose" />
                            ) : selectedItem.raw.envLevel === 'staging' ? (
                              <span className="size-1.5 rounded-full bg-amber-500 status-pulse-amber" />
                            ) : null}
                            <span className="uppercase font-semibold">{selectedItem.raw.envLevel}</span>
                          </span>
                        </>
                      ) : null}
                    </div>
                    <div className="rounded-lg border border-border/40 bg-surface/20 p-2 text-ui-xs text-foreground-subtle">
                      按 <kbd className="px-1 py-0.5 rounded bg-surface border border-border text-[10px] font-mono">Enter</kbd> 开启终端会话，或按 <kbd className="px-1 py-0.5 rounded bg-surface border border-border text-[10px] font-mono">⌘Enter</kbd> 打开 SFTP 文件管理。
                    </div>
                  </div>
                )}

                {selectedItem.type === 'db' && (
                  <div className="flex flex-col gap-2.5 text-ui-xs">
                    <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 rounded-lg border border-border/50 bg-surface/40 p-2.5 font-mono">
                      <span className="text-foreground-subtlest">Engine:</span>
                      <span className="text-foreground uppercase font-bold text-terminal-blue">{selectedItem.raw.engine}</span>
                      <span className="text-foreground-subtlest">Host:</span>
                      <span className="text-foreground select-text">{selectedItem.raw.host}:{selectedItem.raw.port}</span>
                      {selectedItem.raw.database ? (
                        <>
                          <span className="text-foreground-subtlest">Database:</span>
                          <span className="text-foreground select-text">{selectedItem.raw.database}</span>
                        </>
                      ) : null}
                      {selectedItem.raw.username ? (
                        <>
                          <span className="text-foreground-subtlest">User:</span>
                          <span className="text-foreground select-text">{selectedItem.raw.username}</span>
                        </>
                      ) : null}
                      <span className="text-foreground-subtlest">Mode:</span>
                      <span className="text-foreground">{selectedItem.raw.readonly ? '只读 (Read Only)' : '读写 (Read/Write)'}</span>
                    </div>
                    <div className="rounded-lg border border-border/40 bg-surface/20 p-2 text-ui-xs text-foreground-subtle">
                      按 <kbd className="px-1 py-0.5 rounded bg-surface border border-border text-[10px] font-mono">Enter</kbd> 直达数据库工作台并连接。
                    </div>
                  </div>
                )}

                {selectedItem.type === 'manual' && (
                  <div className="flex flex-col gap-2 min-h-0 text-ui-xs">
                    <span className="font-semibold text-primary">
                      {selectedItem.raw.chapter.title}
                    </span>
                    <p className="line-clamp-6 text-foreground-subtle leading-relaxed whitespace-pre-line select-text">
                      {selectedItem.raw.section.content.slice(0, 360)}...
                    </p>
                    <div className="mt-2 rounded-lg border border-border/40 bg-surface/20 p-2 text-ui-xs text-foreground-subtle">
                      按 <kbd className="px-1 py-0.5 rounded bg-surface border border-border text-[10px] font-mono">Enter</kbd> 打开使用手册并直达本节。
                    </div>
                  </div>
                )}

                {selectedItem.type === 'navigation' && (
                  <div className="flex flex-col gap-2 text-ui-xs text-foreground-subtle">
                    <p className="leading-relaxed">
                      快速切换到 {selectedItem.title} 视图。
                    </p>
                    <div className="rounded-lg border border-border/40 bg-surface/20 p-2">
                      按 <kbd className="px-1 py-0.5 rounded bg-surface border border-border text-[10px] font-mono">Enter</kbd> 立即跳转。
                    </div>
                  </div>
                )}
              </div>
            ) : null}

            {/* 预览区底部的快捷操作提示 */}
            <div className="mt-4 pt-2.5 border-t border-border/40 flex items-center justify-between text-[11px] text-foreground-subtlest">
              <span>{copied ? '✓ 已复制到剪贴板' : '⌥C 复制核心信息'}</span>
              <span>{selectedItem?.type ? selectedItem.type.toUpperCase() : ''}</span>
            </div>
          </div>
        </div>

        {/* 底部操作栏（Raycast Action Bar） */}
        <div className="flex items-center justify-between border-t border-border/50 px-3.5 py-2 text-ui-xs text-foreground-subtle select-none bg-surface/40">
          <div className="flex items-center gap-3">
            <span>↑↓ 导航</span>
            <span>Tab 切换分类</span>
            <span>{filteredItems.length} 个结果</span>
          </div>
          <div className="flex items-center gap-2.5">
            {selectedItem?.type === 'host' ? (
              <span className="flex items-center gap-1">
                <kbd className="px-1.5 py-0.5 rounded bg-surface border border-border text-[10px] font-mono">⌘↵</kbd>
                SFTP
              </span>
            ) : null}
            <span className="flex items-center gap-1">
              <kbd className="px-1.5 py-0.5 rounded bg-surface border border-border text-[10px] font-mono">↵</kbd>
              {selectedItem?.type === 'command'
                ? t('commands.palette.send')
                : t('commands.palette.open', '打开')}
            </span>
          </div>
        </div>
      </div>
    </div>
  );

  return mainContainer ? createPortal(overlay, mainContainer) : overlay;
}
