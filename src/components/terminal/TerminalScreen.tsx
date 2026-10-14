import {
  Eraser,
  Grid2X2,
  LayoutGrid,
  Plus,
  Radio,
  Server,
  SquareTerminal,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AskAiErrorChip } from '@/components/ai/AskAiErrorChip';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { BroadcastCommandBar } from '@/components/terminal/BroadcastCommandBar';
import { TerminalGridContainer } from '@/components/terminal/TerminalGridContainer';
import { TerminalTabTrigger } from '@/components/terminal/TerminalTabTrigger';
import { Button } from '@/components/ui/button';
import { EmptyState } from '@/components/ui/EmptyState';
import { useHostsStore } from '@/hosts/HostsProvider';
import { cn } from '@/lib/utils';
import { terminalSessionRegistry } from '@/terminal/terminalSessionRegistry';
import { useTerminalStore } from '@/terminal/TerminalProvider';
import type { TerminalSize } from '@/terminal/types';
import { checkHardwareAcceleration } from '@/terminal/webglSupport';

export interface TerminalScreenProps {
  isVisible?: boolean;
  isPanelResizing?: boolean;
  onNavigateHosts?: () => void;
  /** 打开 AI 对话（可携带错误现场） */
  onOpenAiChat?: (prompt?: string) => void;
}

/**
 * 终端主页面（结构移植自 ZCode `Terminal.tsx`）：
 * - 顶部多 Tab 栏 + 新建/清屏操作 + 当前会话 WebGL2 状态、Shell 与尺寸状态；
 * - 若所有终端关闭，展示清爽的空白状态引导，提供本地终端快捷创建与主机连接入口；
 * - 本地终端完整继承系统终端 Profile（login shell `-l`、`$SHELL`、`$PATH`、UTF-8）；
 * - 所有已打开 Tab 保活挂载（非激活 Tab 仅 `hidden` 隐藏，不卸载 xterm 实例）；
 * - 跨左侧导航页面切换时，由 `terminalSessionRegistry` 将 DOM 暂存至后台并保持 `Channel<Vec<u8>>` 存活。
 */
export function TerminalScreen({
  isVisible = true,
  isPanelResizing = false,
  onNavigateHosts,
  onOpenAiChat,
}: TerminalScreenProps) {
  const { t } = useTranslation();
  const { hosts } = useHostsStore();
  const {
    transport,
    sessions,
    activeSession,
    layoutMode,
    toggleLayoutMode,
    maximizedSessionId,
    setMaximizedSessionId,
    isBroadcastActive,
    toggleBroadcast,
    createSession,
    closeSession,
    activateSession,
    updateShellLabel,
    renameSession,
    clearSession,
  } = useTerminalStore();

  const [sizesBySession, setSizesBySession] = useState<Record<number, TerminalSize>>({});
  const [rendererBySession, setRendererBySession] = useState<Record<number, 'webgl' | 'canvas2d' | 'dom'>>({});

  // ── 新建终端下拉菜单（命令面板风格：↑↓ 导航 + Enter 选中 + Esc 关闭）──
  const [isNewMenuOpen, setIsNewMenuOpen] = useState(false);
  const [newMenuIndex, setNewMenuIndex] = useState(0);
  const newMenuRef = useRef<HTMLDivElement>(null);
  const newMenuTriggerRef = useRef<HTMLDivElement>(null);
  const newMenuItemRefs = useRef<Array<HTMLButtonElement | null>>([]);

  // 菜单选项：本地终端 + 远程主机，按顺序展平供键盘导航
  const newMenuItems = useMemo(() => {
    const items: Array<{ key: string; label: string; sub?: string; icon: 'local' | 'remote'; hostId?: number }> = [
      { key: 'local', label: '新建本地终端', sub: '⌘T', icon: 'local' },
    ];
    for (const host of hosts) {
      items.push({
        key: `host-${host.id}`,
        label: host.name,
        sub: `${host.username}@${host.host}`,
        icon: 'remote',
        hostId: host.id,
      });
    }
    return items;
  }, [hosts]);

  const executeNewMenuItem = useCallback(
    (index: number) => {
      const item = newMenuItems[index];
      if (!item) return;
      setIsNewMenuOpen(false);
      setNewMenuIndex(0);
      if (item.hostId != null) {
        const host = hosts.find((h) => h.id === item.hostId);
        if (host) {
          createSession({ hostId: host.id, customTitle: host.name });
        }
      } else {
        createSession();
      }
    },
    [createSession, hosts, newMenuItems],
  );

  // 菜单打开/索引变化时，把焦点移到当前选中项——键盘事件从菜单项发出而非终端 textarea，
  // Tab/↑↓ 自然走菜单路径，不被 xterm 或 WKWebView 原生键盘处理截获
  useEffect(() => {
    if (!isNewMenuOpen) return;
    newMenuItemRefs.current[newMenuIndex]?.focus();
  }, [isNewMenuOpen, newMenuIndex]);

  // 键盘导航兜底（菜单项自身已持有焦点，这里捕获阶段监听处理 ↑↓/Tab/Enter/Esc）
  useEffect(() => {
    if (!isNewMenuOpen) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'ArrowDown' || (event.key === 'Tab' && !event.shiftKey)) {
        event.preventDefault();
        setNewMenuIndex((i) => (i + 1) % newMenuItems.length);
      } else if (event.key === 'ArrowUp' || (event.key === 'Tab' && event.shiftKey)) {
        event.preventDefault();
        setNewMenuIndex((i) => (i - 1 + newMenuItems.length) % newMenuItems.length);
      } else if (event.key === 'Enter' && !(event as KeyboardEvent & { isComposing?: boolean }).isComposing) {
        event.preventDefault();
        executeNewMenuItem(newMenuIndex);
      } else if (event.key === 'Escape') {
        event.preventDefault();
        setIsNewMenuOpen(false);
        setNewMenuIndex(0);
      }
    };
    window.addEventListener('keydown', onKeyDown, true);
    return () => window.removeEventListener('keydown', onKeyDown, true);
  }, [executeNewMenuItem, isNewMenuOpen, newMenuIndex, newMenuItems.length]);

  // 点击外部关闭
  useEffect(() => {
    if (!isNewMenuOpen) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!newMenuRef.current?.contains(target) && !newMenuTriggerRef.current?.contains(target)) {
        setIsNewMenuOpen(false);
        setNewMenuIndex(0);
      }
    };
    window.addEventListener('pointerdown', onPointerDown);
    return () => window.removeEventListener('pointerdown', onPointerDown);
  }, [isNewMenuOpen]);

  const handleSizeChange = useCallback((sessionId: number, size: TerminalSize) => {
    setSizesBySession((prev) => {
      const existing = prev[sessionId];
      if (existing && existing.cols === size.cols && existing.rows === size.rows) {
        return prev;
      }
      return {
        ...prev,
        [sessionId]: size,
      };
    });
  }, []);

  const handleRendererStatusChange = useCallback((sessionId: number, kind: 'webgl' | 'canvas2d' | 'dom') => {
    setRendererBySession((prev) => {
      if (prev[sessionId] === kind) return prev;
      return {
        ...prev,
        [sessionId]: kind,
      };
    });
  }, []);

  const handleSessionExit = useCallback(
    (sessionId: number, _exitCode: number) => {
      closeSession(sessionId);
    },
    [closeSession],
  );

  // 终端页快捷键：
  // ⌘T 新建 Tab、⌘W 关闭当前 Tab、⌘⇧[ / ⌘⇧] 切换前后 Tab、⌘F 终端搜索
  // ⌘⇧G 切换标签/九宫格、⌘⇧B 切换协同广播、Esc 退出最大化或广播
  useEffect(() => {
    if (!isVisible) return;

    const onKeyDown = (event: KeyboardEvent) => {
      const mod = event.metaKey || event.ctrlKey;

      if (!event.altKey && !event.shiftKey && !mod && event.key === 'Escape') {
        // 搜索栏聚焦（自绘终端搜索栏的输入框）时 Esc 只关搜索栏，由其内部处理，
        // 这里不触发「退出最大化」等其他全局 Esc 语义
        if (document.activeElement?.closest('[data-terminal-search-bar]')) {
          return;
        }
        if (maximizedSessionId !== null) {
          setMaximizedSessionId(null);
          return;
        }
      }

      if (!mod) return;
      const key = event.key.toLowerCase();

      // ⌘F/Ctrl+F：打开激活会话的终端搜索栏（终端已聚焦时由 xterm 键处理分支处理，
      // 这里覆盖焦点在终端之外——如侧栏/顶栏——时的兜底，同时拦截浏览器 find-in-page）
      if (!event.altKey && !event.shiftKey && key === 'f') {
        if (activeSession) {
          event.preventDefault();
          terminalSessionRegistry.showSearch(activeSession.id);
        }
        return;
      }

      if (!event.altKey && !event.shiftKey && key === 't') {
        event.preventDefault();
        createSession();
        return;
      }

      if (!event.altKey && !event.shiftKey && key === 'w') {
        if (activeSession) {
          event.preventDefault();
          closeSession(activeSession.id);
        }
        return;
      }

      if (event.shiftKey && key === 'g') {
        event.preventDefault();
        toggleLayoutMode();
        return;
      }

      if (event.shiftKey && key === 'b') {
        event.preventDefault();
        toggleBroadcast();
        return;
      }

      if (event.shiftKey && (event.key === '[' || event.key === '{')) {
        event.preventDefault();
        if (sessions.length > 1 && activeSession) {
          const idx = sessions.findIndex((s) => s.id === activeSession.id);
          const prevIdx = (idx - 1 + sessions.length) % sessions.length;
          const target = sessions[prevIdx];
          if (target) activateSession(target.id);
        }
        return;
      }

      if (event.shiftKey && (event.key === ']' || event.key === '}')) {
        event.preventDefault();
        if (sessions.length > 1 && activeSession) {
          const idx = sessions.findIndex((s) => s.id === activeSession.id);
          const nextIdx = (idx + 1) % sessions.length;
          const target = sessions[nextIdx];
          if (target) activateSession(target.id);
        }
      }
    };

    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [
    activateSession,
    activeSession,
    closeSession,
    createSession,
    isVisible,
    maximizedSessionId,
    sessions,
    setMaximizedSessionId,
    toggleBroadcast,
    toggleLayoutMode,
  ]);

  const activeSize = activeSession ? sizesBySession[activeSession.id] : undefined;
  const activeRenderer = activeSession ? (rendererBySession[activeSession.id] ?? 'dom') : 'dom';
  /** 探测结果带缓存，挂载时读取一次即可；用于 WebGL 徽标 tooltip 展示实际 GPU */
  const webglRenderer = useMemo(() => checkHardwareAcceleration().renderer, []);

  return (
    <section
      data-testid="terminal-screen"
      className="relative flex h-full min-h-0 flex-col overflow-hidden bg-background"
    >
      {/* 顶部多 Tab 栏（ZCode Terminal 顶栏规范） */}
      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border/50 px-3 py-1.5">
        <div className="scrollbar-slim flex min-w-0 flex-1 items-center gap-1 overflow-x-auto overflow-y-hidden">
          {sessions.length > 0 ? (
            <div role="tablist" aria-label={t('terminal.title')} className="flex h-7 w-max items-center gap-1">
              {sessions.map((session) => {
                // 标题不再拼接序号：customTitle（重命名）> shellLabel > 默认名
                const title = session.customTitle || session.shellLabel || t('terminal.defaultTab');
                return (
                  <TerminalTabTrigger
                    key={session.id}
                    session={session}
                    title={title}
                    closeLabel={t('terminal.closeTab', { title })}
                    isActive={session.id === activeSession?.id}
                    onActivate={activateSession}
                    onClose={closeSession}
                    onRename={(next) => renameSession(session.id, next)}
                  />
                );
              })}
            </div>
          ) : (
            <span className="text-ui-xs text-foreground-subtle select-none pl-1">
              {t('terminal.title')}
            </span>
          )}

          {/* 新建终端 Tab 按钮：命令菜单式下拉（↑↓ 导航 + Enter 选中 + Esc 关闭） */}
          {sessions.length > 0 && (
            <div className="relative inline-flex items-center shrink-0">
              <ControlHintTooltip
                title={hosts.length > 0 ? '新建终端 / 连接主机 (⌘T)' : '新建终端 (⌘T)'}
                side="bottom"
              >
                <div ref={newMenuTriggerRef} className="relative flex items-center justify-center">
                  <Button
                    type="button"
                    size="icon-xs"
                    variant="ghost"
                    className="h-7 w-7 rounded-lg text-foreground-subtle hover:text-foreground hover:bg-hover active:bg-hover/80"
                    onClick={() => {
                      if (hosts.length > 0) {
                        setNewMenuIndex(0);
                        setIsNewMenuOpen((open) => !open);
                      } else {
                        createSession();
                      }
                    }}
                    aria-label="新建终端"
                    aria-expanded={isNewMenuOpen}
                  >
                    <Plus className="size-3.5" />
                  </Button>
                </div>
              </ControlHintTooltip>

              {/* 命令菜单弹层 */}
              {isNewMenuOpen ? (
                <div
                  ref={newMenuRef}
                  role="menu"
                  aria-label="新建终端或连接主机"
                  className="absolute top-full left-0 z-50 mt-1 min-w-56 max-h-64 overflow-y-auto rounded-lg border border-border bg-menu p-1 shadow-lg"
                >
                  {newMenuItems.map((item, index) => {
                    const isActive = index === newMenuIndex;
                    return (
                      <button
                        key={item.key}
                        ref={(el) => { newMenuItemRefs.current[index] = el; }}
                        type="button"
                        role="menuitem"
                        tabIndex={isActive ? 0 : -1}
                        onClick={() => executeNewMenuItem(index)}
                        onMouseEnter={() => setNewMenuIndex(index)}
                        className={cn(
                          'flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left outline-none transition-colors',
                          isActive ? 'bg-hover text-foreground' : 'text-foreground-subtle hover:text-foreground',
                        )}
                      >
                        {item.icon === 'local' ? (
                          <SquareTerminal className="size-3.5 shrink-0 text-primary" />
                        ) : (
                          <Server className="size-3.5 shrink-0 text-foreground-subtle" />
                        )}
                        <span className="min-w-0 flex-1 truncate text-ui-sm font-medium">{item.label}</span>
                        {item.sub ? (
                          <span className="shrink-0 font-mono text-ui-xs text-foreground-subtlest">{item.sub}</span>
                        ) : null}
                      </button>
                    );
                  })}
                </div>
              ) : null}
            </div>
          )}
        </div>

        {/* 右侧状态与辅助动作 */}
        <div className="flex shrink-0 items-center gap-1.5">
          {activeRenderer !== 'dom' ? (
            <span
              className={cn(
                'rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs select-none',
                activeRenderer === 'webgl' ? 'text-success' : 'text-foreground-subtle',
              )}
              title={activeRenderer === 'webgl' ? `WebGL2 硬件加速 · ${webglRenderer}` : 'Canvas 2D 渲染'}
            >
              {activeRenderer === 'webgl' ? 'GPU' : '2D'}
            </span>
          ) : null}
          {activeSession?.shellLabel ? (
            <span className="rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtle select-none">
              {activeSession.shellLabel}
            </span>
          ) : null}
          {activeSize ? (
            <span className="font-mono text-ui-xs text-foreground-subtlest select-none">
              {activeSize.cols}×{activeSize.rows}
            </span>
          ) : null}

          {/* 协同广播开关按钮 */}
          {sessions.length > 0 && (
            <ControlHintTooltip
              title={isBroadcastActive ? '收起协同广播 (⌘⇧B)' : '开启多终端协同广播 (⌘⇧B)'}
              side="bottom"
              align="end"
            >
              <Button
                type="button"
                size="sm"
                variant={isBroadcastActive ? 'secondary' : 'ghost'}
                onClick={toggleBroadcast}
                className={cn(
                  'h-7 px-2 text-xs font-medium gap-1.5 rounded-lg border transition-all',
                  isBroadcastActive
                    ? 'border-primary/50 bg-primary/10 text-primary shadow-xs ring-1 ring-primary/20'
                    : 'border-border/60 text-foreground-subtle hover:text-foreground hover:bg-hover'
                )}
                aria-label="协同广播"
              >
                <Radio className={cn('size-3.5', isBroadcastActive && 'text-primary animate-pulse')} />
                <span>{isBroadcastActive ? '广播中' : '广播'}</span>
              </Button>
            </ControlHintTooltip>
          )}

          {/* 网格平铺视图 / 标签页视图切换按钮 */}
          {sessions.length > 0 && (
            <ControlHintTooltip
              title={layoutMode === 'grid' ? '切换为标签页视图 (⌘⇧G)' : '切换为九宫格平铺视图 (⌘⇧G)'}
              side="bottom"
              align="end"
            >
              <Button
                type="button"
                size="sm"
                variant={layoutMode === 'grid' ? 'secondary' : 'ghost'}
                onClick={toggleLayoutMode}
                className={cn(
                  'h-7 px-2 text-xs font-medium gap-1.5 rounded-lg border transition-all',
                  layoutMode === 'grid'
                    ? 'border-primary/50 bg-primary/10 text-primary shadow-xs ring-1 ring-primary/20'
                    : 'border-border/60 text-foreground-subtle hover:text-foreground hover:bg-hover'
                )}
                aria-label="切换网格视图"
              >
                {layoutMode === 'grid' ? (
                  <>
                    <LayoutGrid className="size-3.5 text-primary" />
                    <span>宫格</span>
                  </>
                ) : (
                  <>
                    <Grid2X2 className="size-3.5" />
                    <span>宫格</span>
                  </>
                )}
              </Button>
            </ControlHintTooltip>
          )}

          {activeSession ? (
            <ControlHintTooltip title={t('terminal.clear')} side="bottom" align="end">
              <Button
                type="button"
                size="icon-md"
                variant="ghost"
                onClick={() => clearSession(activeSession.id)}
                aria-label={t('terminal.clear')}
              >
                <Eraser className="size-4" />
              </Button>
            </ControlHintTooltip>
          ) : null}
        </div>
      </div>

      {/* 终端会话容器：无会话时显示空白引导，有会话时根据布局模式呈现 */}
      {sessions.length === 0 ? (
        <EmptyState
          icon={SquareTerminal}
          title={t('terminal.emptyTitle')}
          description={t('terminal.emptyDesc')}
          primaryAction={{
            label: t('terminal.newLocal'),
            icon: Plus,
            shortcut: '⌘T',
            onClick: () => createSession(),
          }}
          secondaryAction={
            onNavigateHosts
              ? {
                  label: t('terminal.connectHost'),
                  icon: Server,
                  onClick: onNavigateHosts,
                }
              : undefined
          }
        />
      ) : (
        /* 统一布局容器：宫格平铺 / 标签页仅切换 CSS 类，TerminalSession 保挂载不重挂 */
        <TerminalGridContainer
          layoutMode={layoutMode}
          sessions={sessions}
          activeSessionId={activeSession?.id ?? 0}
          maximizedSessionId={maximizedSessionId}
          transport={transport}
          isPanelResizing={isPanelResizing}
          isVisible={isVisible}
          onActivateSession={activateSession}
          onMaximizeSession={setMaximizedSessionId}
          onCloseSession={closeSession}
          onShellLabelChange={updateShellLabel}
          onSizeChange={handleSizeChange}
          onRendererStatusChange={handleRendererStatusChange}
          onExit={handleSessionExit}
        />
      )}

      {/* 协同广播命令输入栏（吸附于底部） */}
      {isBroadcastActive && sessions.length > 0 && (
        <BroadcastCommandBar sessions={sessions} onClose={toggleBroadcast} />
      )}

      <AskAiErrorChip onAsk={(prompt) => onOpenAiChat?.(prompt)} />
    </section>
  );
}
