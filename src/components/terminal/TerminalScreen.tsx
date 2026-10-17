import {
  Command,
  Eraser,
  Gauge,
  Grid2X2,
  LayoutGrid,
  Plus,
  Radio,
  Server,
  SquareTerminal,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AskAiErrorChip } from '@/components/ai/AskAiErrorChip';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { NewSessionMenu } from '@/components/NewSessionMenu';
import { BroadcastCommandBar } from '@/components/terminal/BroadcastCommandBar';
import { TerminalGridContainer } from '@/components/terminal/TerminalGridContainer';
import { TerminalTabTrigger } from '@/components/terminal/TerminalTabTrigger';
import { Button } from '@/components/ui/button';
import { EmptyState } from '@/components/ui/EmptyState';
import { useHostsStore } from '@/hosts/HostsProvider';
import { hasPrimaryModifier, shortcutLabel } from '@/lib/keyboard';
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
  /** 打开 ⌘K 命令面板 */
  onOpenCommandPalette?: () => void;
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
  onOpenAiChat, onOpenCommandPalette,
}: TerminalScreenProps) {
  const { t } = useTranslation();
  const { hosts } = useHostsStore();
  const {
    transport,
    sessions,
    activeSession,
    layoutMode,
    toggleLayoutMode,
    resourceMonitorOn,
    toggleResourceMonitor,
    maximizedSessionId,
    setMaximizedSessionId,
    isBroadcastActive,
    toggleBroadcast,
    broadcastTargets,
    createSession,
    closeSession,
    activateSession,
    updateShellLabel,
    renameSession,
    clearSession,
    markDisconnected,
    reconnectSession,
  } = useTerminalStore();

  const [sizesBySession, setSizesBySession] = useState<Record<number, TerminalSize>>({});
  const [rendererBySession, setRendererBySession] = useState<Record<number, 'webgl' | 'canvas2d' | 'dom'>>({});

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

  // SSH 会话断开：仅标记断开态（Tab 保留、终端现场保活），不关闭 Tab；
  // 本地会话仍走 handleSessionExit 自动关 Tab，两条路径不再重叠
  const handleSessionDisconnected = useCallback(
    (sessionId: number, reason: string) => {
      markDisconnected(sessionId, reason);
    },
    [markDisconnected],
  );

  // 终端页快捷键（主修饰键 = ⌘/Ctrl）：
  // +T 新建 Tab、+W 关闭当前 Tab、+⇧[ / +⇧] 切换前后 Tab、+F 终端搜索
  // +⇧G 切换标签/九宫格、+⇧B 切换终端联动、+R 重连断开 Tab、Esc 退出最大化或关闭联动
  useEffect(() => {
    if (!isVisible) return;

    const onKeyDown = (event: KeyboardEvent) => {
      const mod = hasPrimaryModifier(event);

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
        if (isBroadcastActive) {
          toggleBroadcast();
          return;
        }
      }

      if (!mod) return;
      const key = event.key.toLowerCase();

      // +F：打开激活会话的终端搜索栏（终端已聚焦时由 xterm 键处理分支处理，
      // 这里覆盖焦点在终端之外——如侧栏/顶栏——时的兜底，同时拦截浏览器 find-in-page）
      if (!event.altKey && !event.shiftKey && key === 'f') {
        if (activeSession) {
          event.preventDefault();
          terminalSessionRegistry.showSearch(activeSession.id);
        }
        return;
      }

      // +R：重连当前激活的已断开 Tab（焦点在终端内时由 xterm 键处理分支处理，
      // 这里覆盖焦点在终端之外——如侧栏/顶栏——时的兜底；活动会话不拦截，保留 ^R 反向搜索）
      if (!event.altKey && !event.shiftKey && key === 'r') {
        if (activeSession?.status === 'disconnected') {
          event.preventDefault();
          reconnectSession(activeSession.id);
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
    isBroadcastActive,
    isVisible,
    maximizedSessionId,
    reconnectSession,
    sessions,
    setMaximizedSessionId,
    toggleBroadcast,
    toggleLayoutMode,
  ]);

  const activeSize = activeSession ? sizesBySession[activeSession.id] : undefined;
  const activeRenderer = activeSession ? (rendererBySession[activeSession.id] ?? 'dom') : 'dom';
  /** 探测结果带缓存，挂载时读取一次即可；用于 WebGL 徽标 tooltip 展示实际 GPU */
  const webglRenderer = useMemo(() => checkHardwareAcceleration().renderer, []);

  /** 渲染加速状态徽标：GPU 加速（金黄） / Canvas 2D（浅绿） / 无加速（正常字色）。
   *  颜色用语义 token（warning/success/foreground），亮暗两套值随主题自动切换 */
  const rendererBadge =
    activeRenderer === 'webgl'
      ? { label: 'GPU', color: 'text-warning', title: `WebGL2 硬件加速 · ${webglRenderer}` }
      : activeRenderer === 'canvas2d'
        ? { label: '2D', color: 'text-success', title: 'Canvas 2D 渲染' }
        : { label: '无加速', color: 'text-foreground', title: '无硬件加速 · DOM 渲染' };

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

          {/* 新建终端 Tab 按钮（支持本地终端或快捷连接远程主机） */}
          {sessions.length > 0 && (
            <NewSessionMenu
              title={`${t('terminal.new')} (${shortcutLabel('T')})`}
              localLabel={t('terminal.newLocal')}
              onNewLocal={() => createSession()}
              onDirectAction={() => createSession()}
              hostSectionLabel={t('terminal.remoteHosts')}
              hosts={hosts}
              onSelectHost={(hostId) => {
                const host = hosts.find((h) => h.id === hostId);
                if (host) {
                  createSession({ hostId: host.id, customTitle: host.name });
                }
              }}
            />
          )}
        </div>

        {/* 右侧状态与辅助动作 */}
        <div className="flex shrink-0 items-center gap-1.5">
            {onOpenCommandPalette ? (
              <ControlHintTooltip title={t('commands.palette.title')} shortcut={shortcutLabel('K')} side="bottom">
                <Button
                  variant="ghost"
                  size="icon-sm"
                  onClick={onOpenCommandPalette}
                  aria-label={t('commands.palette.title')}
                >
                  <Command className="size-3.5" />
                </Button>
              </ControlHintTooltip>
            ) : null}
            {/* 服务器资源监控开关（默认关）：开 = 当前 SSH 会话底部展示监控条并后台采样，
                关 = 隐藏且停止轮询；状态落库跨启动保留 */}
            {sessions.length > 0 ? (
              <ControlHintTooltip
                title={resourceMonitorOn ? t('terminal.resourceMonitor.disable') : t('terminal.resourceMonitor.enable')}
                side="bottom"
              >
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  onClick={toggleResourceMonitor}
                  aria-pressed={resourceMonitorOn}
                  aria-label={t('terminal.resourceMonitor.toggleAria')}
                  className={cn(
                    // 与左侧命令面板按钮完全同规格（icon-sm 26px + rounded-lg +
                    // ghost 默认 text-foreground），关闭态零叠加；
                    // 此前手写 size-7（28px）比 ⌘ 按钮大一圈，并排尺寸不齐
                    resourceMonitorOn && 'bg-selected text-foreground hover:bg-selected'
                  )}
                >
                  <Gauge
                    className={cn('size-3.5', resourceMonitorOn && 'text-primary')}
                    strokeWidth={2.5}
                  />
                </Button>
              </ControlHintTooltip>
            ) : null}
          {activeSession ? (
            <ControlHintTooltip title={rendererBadge.title} side="bottom">
              <span
                className={cn(
                  'rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs select-none',
                  rendererBadge.color,
                )}
              >
                {rendererBadge.label}
              </span>
            </ControlHintTooltip>
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

          {/* 终端联动开关按钮 */}
          {sessions.length > 0 && (
            <ControlHintTooltip
              title={isBroadcastActive ? `收起终端联动 (${shortcutLabel('B', { shift: true })})` : `开启多终端实时联动 (${shortcutLabel('B', { shift: true })})`}
              side="bottom"
              align="end"
            >
              <Button
                type="button"
                size="sm"
                variant="ghost"
                onClick={toggleBroadcast}
                className={cn(
                  'h-7 px-2 text-xs gap-1.5 rounded-lg border transition-colors',
                  isBroadcastActive
                    ? 'border-border bg-selected text-foreground hover:bg-selected'
                    : 'border-border/60 text-foreground-subtle hover:text-foreground hover:bg-hover'
                )}
                aria-label="终端联动"
              >
                <Radio className={cn('size-3.5', isBroadcastActive && 'text-primary animate-pulse')} />
                <span>{isBroadcastActive ? '联动中' : '联动'}</span>
              </Button>
            </ControlHintTooltip>
          )}

          {/* 网格平铺视图 / 标签页视图切换按钮 */}
          {sessions.length > 0 && (
            <ControlHintTooltip
              title={layoutMode === 'grid' ? `切换为标签页视图 (${shortcutLabel('G', { shift: true })})` : `切换为九宫格平铺视图 (${shortcutLabel('G', { shift: true })})`}
              side="bottom"
              align="end"
            >
              <Button
                type="button"
                size="sm"
                variant="ghost"
                onClick={toggleLayoutMode}
                className={cn(
                  'h-7 px-2 text-xs gap-1.5 rounded-lg border transition-colors',
                  layoutMode === 'grid'
                    ? 'border-border bg-selected text-foreground hover:bg-selected'
                    : 'border-border/60 text-foreground-subtle hover:text-foreground hover:bg-hover'
                )}
                aria-label="切换网格视图"
              >
                {layoutMode === 'grid' ? (
                  <>
                    <LayoutGrid className="size-3.5" />
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
            shortcut: shortcutLabel('T'),
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
          broadcastTargets={broadcastTargets}
          isPanelResizing={isPanelResizing}
          isVisible={isVisible}
          resourceMonitorOn={resourceMonitorOn}
          onActivateSession={activateSession}
          onMaximizeSession={setMaximizedSessionId}
          onCloseSession={closeSession}
          onShellLabelChange={updateShellLabel}
          onSizeChange={handleSizeChange}
          onRendererStatusChange={handleRendererStatusChange}
          onExit={handleSessionExit}
          onSessionDisconnected={handleSessionDisconnected}
          onReconnectSession={reconnectSession}
        />
      )}

      {/* 终端联动控制条（吸附于底部）：控制条可见且勾选≥两个目标时，任一目标内
          的用户输入实时镜像到其余目标（纯输入镜像，无命令输入栏） */}
      {isBroadcastActive && sessions.length > 0 && (
        <BroadcastCommandBar sessions={sessions} onClose={toggleBroadcast} />
      )}

      <AskAiErrorChip onAsk={(prompt) => onOpenAiChat?.(prompt)} />
    </section>
  );
}
