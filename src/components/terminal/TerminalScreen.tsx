import {
  Eraser,
  Grid2X2,
  LayoutGrid,
  Plus,
  Radio,
  Server,
  SquareTerminal,
} from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { BroadcastCommandBar } from '@/components/terminal/BroadcastCommandBar';
import { TerminalGridContainer } from '@/components/terminal/TerminalGridContainer';
import { TerminalSession } from '@/components/terminal/TerminalSession';
import { TerminalTabTrigger } from '@/components/terminal/TerminalTabTrigger';
import { Button } from '@/components/ui/button';
import { EmptyState } from '@/components/ui/EmptyState';
import { cn } from '@/lib/utils';
import { useTerminalStore } from '@/terminal/TerminalProvider';
import { formatTerminalTabTitle } from '@/terminal/terminalPanelState';
import type { TerminalSize } from '@/terminal/types';

export interface TerminalScreenProps {
  isVisible?: boolean;
  isPanelResizing?: boolean;
  onNavigateHosts?: () => void;
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
}: TerminalScreenProps) {
  const { t } = useTranslation();
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
    clearSession,
  } = useTerminalStore();

  const [sizesBySession, setSizesBySession] = useState<Record<number, TerminalSize>>({});
  const [webglBySession, setWebglBySession] = useState<Record<number, boolean>>({});

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

  const handleWebglStatusChange = useCallback((sessionId: number, active: boolean) => {
    setWebglBySession((prev) => {
      if (prev[sessionId] === active) return prev;
      return {
        ...prev,
        [sessionId]: active,
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
  // ⌘T 新建 Tab、⌘W 关闭当前 Tab、⌘⇧[ / ⌘⇧] 切换前后 Tab
  // ⌘⇧G 切换标签/九宫格、⌘⇧B 切换协同广播、Esc 退出最大化或广播
  useEffect(() => {
    if (!isVisible) return;

    const onKeyDown = (event: KeyboardEvent) => {
      const mod = event.metaKey || event.ctrlKey;

      if (!event.altKey && !event.shiftKey && !mod && event.key === 'Escape') {
        if (maximizedSessionId !== null) {
          setMaximizedSessionId(null);
          return;
        }
      }

      if (!mod) return;
      const key = event.key.toLowerCase();

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
  const isActiveWebgl = activeSession ? Boolean(webglBySession[activeSession.id]) : false;

  return (
    <section
      data-testid="terminal-screen"
      className="flex h-full min-h-0 flex-col overflow-hidden bg-background"
    >
      {/* 顶部多 Tab 栏（ZCode Terminal 顶栏规范） */}
      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border/50 px-3 py-1.5">
        <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
          {sessions.length > 0 ? (
            <div role="tablist" aria-label={t('terminal.title')} className="flex h-7 w-max items-center gap-1">
              {sessions.map((session) => {
                const baseName = session.customTitle || session.shellLabel || t('terminal.defaultTab');
                const title = formatTerminalTabTitle(baseName, session.index);
                return (
                  <TerminalTabTrigger
                    key={session.id}
                    session={session}
                    title={title}
                    closeLabel={t('terminal.closeTab', { title })}
                    isActive={session.id === activeSession?.id}
                    onActivate={activateSession}
                    onClose={closeSession}
                  />
                );
              })}
            </div>
          ) : (
            <span className="text-ui-xs text-foreground-subtle select-none pl-1">
              {t('terminal.title')}
            </span>
          )}
        </div>

        {/* 右侧状态与辅助动作 */}
        <div className="flex shrink-0 items-center gap-1.5">
          {isActiveWebgl ? (
            <span className="rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-success select-none">
              WebGL2
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
          {sessions.length > 1 && (
            <ControlHintTooltip
              title={isBroadcastActive ? '关闭协同广播 (⌘⇧B)' : '开启多终端协同广播 (⌘⇧B)'}
              side="bottom"
              align="end"
            >
              <Button
                type="button"
                size="icon-md"
                variant={isBroadcastActive ? 'secondary' : 'ghost'}
                onClick={toggleBroadcast}
                className={cn('size-7 rounded-lg relative', isBroadcastActive && 'bg-primary/10 text-primary')}
                aria-label="协同广播"
              >
                <Radio className={cn('size-3.5', isBroadcastActive && 'text-primary animate-pulse')} />
              </Button>
            </ControlHintTooltip>
          )}

          {/* 网格视图 / 标签视图切换按钮 */}
          {sessions.length > 1 && (
            <ControlHintTooltip
              title={layoutMode === 'grid' ? '切换为标签页视图 (⌘⇧G)' : '切换为九宫格平铺视图 (⌘⇧G)'}
              side="bottom"
              align="end"
            >
              <Button
                type="button"
                size="icon-md"
                variant={layoutMode === 'grid' ? 'secondary' : 'ghost'}
                onClick={toggleLayoutMode}
                className={cn('size-7 rounded-lg', layoutMode === 'grid' && 'bg-primary/10 text-primary')}
                aria-label="切换网格视图"
              >
                {layoutMode === 'grid' ? (
                  <LayoutGrid className="size-3.5" />
                ) : (
                  <Grid2X2 className="size-3.5" />
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
      ) : layoutMode === 'grid' ? (
        /* 九宫格 / 平铺网格矩阵模式 */
        <div className="min-h-0 flex-1 overflow-hidden">
          <TerminalGridContainer
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
            onWebglStatusChange={handleWebglStatusChange}
            onExit={handleSessionExit}
          />
        </div>
      ) : (
        /* 标准单 Tab 标签模式（非激活 Tab hidden 保活） */
        <div className="min-h-0 flex-1 overflow-hidden p-3 pt-2">
          {sessions.map((session) => {
            const isTabActive = session.id === activeSession?.id;
            return (
              <div
                key={session.id}
                role="tabpanel"
                aria-hidden={!isTabActive}
                data-state={isTabActive ? 'active' : 'inactive'}
                className={cn('h-full min-h-0 w-full', !isTabActive && 'hidden')}
              >
                <TerminalSession
                  sessionId={session.id}
                  transport={transport}
                  cwd={session.cwd}
                  hostId={session.hostId}
                  isVisible={isVisible && isTabActive}
                  isPanelResizing={isPanelResizing}
                  onShellLabelChange={updateShellLabel}
                  onSizeChange={handleSizeChange}
                  onWebglStatusChange={handleWebglStatusChange}
                  onExit={handleSessionExit}
                />
              </div>
            );
          })}
        </div>
      )}

      {/* 协同广播命令输入栏（吸附于底部） */}
      {isBroadcastActive && sessions.length > 0 && (
        <BroadcastCommandBar sessions={sessions} onClose={toggleBroadcast} />
      )}
    </section>
  );
}
