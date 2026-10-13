import {
  FolderSync,
  Plus,
  Server,
} from 'lucide-react';
import { useEffect, useId } from 'react';
import { useTranslation } from 'react-i18next';
import { SftpSessionView } from '@/components/files/SftpSessionView';
import { SftpTabTrigger } from '@/components/files/SftpTabTrigger';
import { Button } from '@/components/ui/button';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { EmptyState } from '@/components/ui/EmptyState';
import { useHostsStore } from '@/hosts/HostsProvider';
import { cn } from '@/lib/utils';
import { useSftpStore } from '@/services/SftpProvider';

export interface FilesScreenProps {
  isVisible?: boolean;
  onNavigateHosts?: () => void;
  /** 打开可编辑文件到编辑器页（由 AppShell 注入：openFile + 切页） */
  onOpenEditor?: (file: { hostId: number; remotePath: string }) => void;
}

export function FilesScreen({ isVisible = true, onNavigateHosts, onOpenEditor }: FilesScreenProps) {
  const { t } = useTranslation();
  const selectId = useId();
  const { hosts } = useHostsStore();
  const {
    sessions,
    activeSession,
    createSession,
    closeSession,
    activateSession,
    updateSessionPath,
  } = useSftpStore();

  // 快捷键支持：⌘W 关闭当前 Tab、⌘⇧[ / ⌘⇧] 切换前后 Tab
  useEffect(() => {
    if (!isVisible) return;

    const onKeyDown = (event: KeyboardEvent) => {
      const mod = event.metaKey || event.ctrlKey;
      if (!mod) return;
      const key = event.key.toLowerCase();

      if (!event.altKey && !event.shiftKey && key === 'w') {
        if (activeSession) {
          event.preventDefault();
          closeSession(activeSession.id);
        }
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
  }, [activateSession, activeSession, closeSession, isVisible, sessions]);

  // 新建指定主机的 SFTP Tab
  const handleOpenHostSession = (hostId: number) => {
    const host = hosts.find((h) => h.id === hostId);
    if (!host) return;
    createSession({
      hostId: host.id,
      hostName: host.name,
      initialPath: '/',
    });
  };

  return (
    <section
      data-testid="files-screen"
      className="flex h-full min-h-0 flex-col overflow-hidden bg-background"
    >
      {/* 顶部多 Tab 栏（完全对齐 ZCode Terminal 顶栏规范） */}
      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border/50 px-3 py-1.5">
        <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
          {sessions.length > 0 ? (
            <div
              role="tablist"
              aria-label={t('files.title')}
              className="flex h-7 w-max items-center gap-1"
            >
              {sessions.map((session) => (
                <SftpTabTrigger
                  key={session.id}
                  session={session}
                  title={session.hostName}
                  closeLabel={t('files.closeTab', { title: session.hostName })}
                  isActive={session.id === activeSession?.id}
                  onActivate={activateSession}
                  onClose={closeSession}
                />
              ))}
            </div>
          ) : (
            <span className="text-ui-xs text-foreground-subtle select-none pl-1">
              {t('files.title')}
            </span>
          )}

          {/* 新建 SFTP Tab 按钮（带主机选择下拉） */}
          {hosts.length > 0 && (
            <div className="relative inline-flex items-center shrink-0">
              <ControlHintTooltip title={t('files.newTab')} side="bottom">
                <div className="relative flex items-center justify-center">
                  <Button
                    type="button"
                    size="icon-xs"
                    variant="ghost"
                    className="h-7 w-7 rounded-lg text-foreground-subtle hover:text-foreground hover:bg-hover active:bg-hover/80"
                    aria-label={t('files.newTab')}
                  >
                    <Plus className="size-3.5" />
                  </Button>
                  <select
                    id={selectId}
                    aria-label={t('files.newTab')}
                    value=""
                    onChange={(e) => {
                      const hostId = Number(e.target.value);
                      if (hostId) handleOpenHostSession(hostId);
                    }}
                    className="absolute inset-0 w-full h-full opacity-0 cursor-pointer text-xs"
                  >
                    <option value="" disabled>
                      {t('files.newTab')}
                    </option>
                    {hosts.map((host) => (
                      <option key={host.id} value={host.id}>
                        {host.name} ({host.username}@{host.host})
                      </option>
                    ))}
                  </select>
                </div>
              </ControlHintTooltip>
            </div>
          )}
        </div>

        {/* 右侧主机标识与状态指示 */}
        <div className="flex shrink-0 items-center gap-2">
          {activeSession ? (
            <span className="rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtle select-none">
              {activeSession.hostName}
            </span>
          ) : null}
        </div>
      </div>

      {/* 会话容器：无会话时显示引导空态，有会话时非激活 Tab 保持挂载 hidden */}
      {sessions.length === 0 ? (
        <EmptyState
          icon={FolderSync}
          title={t('files.emptyTitle')}
          description={t('files.emptyDesc')}
          primaryAction={
            hosts.length > 0
              ? {
                  label: t('files.openSftp'),
                  icon: Plus,
                  onClick: () => handleOpenHostSession(hosts[0]!.id),
                }
              : undefined
          }
          secondaryAction={
            onNavigateHosts
              ? {
                  label: hosts.length > 0 ? t('terminal.connectHost') : t('files.goToHosts'),
                  icon: Server,
                  variant: hosts.length > 0 ? 'outline' : 'default',
                  onClick: onNavigateHosts,
                }
              : undefined
          }
        />
      ) : (
        <div className="min-h-0 flex-1 overflow-hidden">
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
                <SftpSessionView
                  sessionId={String(session.hostId)}
                  hostId={session.hostId}
                  initialPath={session.currentPath}
                  isVisible={Boolean(isVisible && isTabActive)}
                  onPathChange={(path) => updateSessionPath(session.id, path)}
                  onOpenEditor={onOpenEditor}
                />
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
