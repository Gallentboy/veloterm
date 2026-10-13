import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { GroupFormDialog } from '@/components/hosts/GroupFormDialog';
import { HostFormDialog, type HostFormResult } from '@/components/hosts/HostFormDialog';
import { GroupTreeRow, HostTreeRow } from '@/components/hosts/HostTree';
import { Button } from '@/components/ui/button';
import { useHostsStore } from '@/hosts/HostsProvider';
import type { GroupNode, HostNode } from '@/hosts/types';

type HostDialogState = { mode: 'create' } | { mode: 'edit'; node: HostNode } | null;
type GroupDialogState =
  | { mode: 'create'; parentId: number | null }
  | { mode: 'edit'; node: GroupNode }
  | null;

/**
 * 主机管理页：按分组树渲染成员主机（一台主机可出现在多个分组下），
 * 未分组主机单独一节；分组本身的增删改去「分组管理」页。
 */
export function HostsScreen({
  onConnectHost,
  onOpenSftp,
}: {
  onConnectHost?: (host: HostNode) => void;
  onOpenSftp?: (host: HostNode) => void;
}) {
  const { t } = useTranslation();
  const {
    groups,
    hosts,
    childGroups,
    hostsInGroup,
    ungroupedHosts,
    createHost,
    updateHost,
    removeHost,
    createGroup,
    updateGroup,
    removeGroup,
    isExpanded,
    toggleExpanded,
  } = useHostsStore();

  const [hostDialog, setHostDialog] = useState<HostDialogState>(null);
  const [groupDialog, setGroupDialog] = useState<GroupDialogState>(null);
  const [pendingDeleteId, setPendingDeleteId] = useState<number | null>(null);

  const handleDeleteHost = (id: number) => {
    // 两段式确认：第一次点标记，第二次点真正删除
    setPendingDeleteId((current) => {
      if (current === id) {
        removeHost(id);
        return null;
      }
      return id;
    });
  };

  const handleDeleteGroup = (id: number) => {
    // 主机页内也允许直接删分组（只解除关联，主机保留）
    setPendingDeleteId((current) => {
      if (current === id) {
        removeGroup(id);
        return null;
      }
      return id;
    });
  };

  const handleSaveHost = (result: HostFormResult) => {
    if (hostDialog?.mode === 'edit') {
      updateHost(hostDialog.node.id, result);
    } else {
      createHost(result);
    }
    setHostDialog(null);
  };

  const handleSaveGroup = (result: { name: string; parentId: number | null }) => {
    if (groupDialog?.mode === 'edit') {
      updateGroup(groupDialog.node.id, result);
    } else if (groupDialog?.mode === 'create') {
      createGroup(result);
    }
    setGroupDialog(null);
  };

  const renderGroup = (parentId: number | null, depth: number) =>
    childGroups(parentId).map((group) => (
      <div key={group.id} className="flex flex-col">
        <GroupTreeRow
          group={group}
          depth={depth}
          expanded={isExpanded(group.id)}
          pendingDeleteId={pendingDeleteId}
          onToggle={toggleExpanded}
          onEdit={(target) => {
            setPendingDeleteId(null);
            setGroupDialog({ mode: 'edit', node: target });
          }}
          onDelete={handleDeleteGroup}
        />
        {isExpanded(group.id) ? (
          <>
            {hostsInGroup(group.id).map((host) => {
              const jumpHost =
                host.jumpHostId != null ? hosts.find((item) => item.id === host.jumpHostId) : undefined;
              return (
                <HostTreeRow
                  key={`${group.id}-${host.id}`}
                  host={host}
                  jumpHostName={jumpHost?.name}
                  depth={depth + 1}
                  pendingDeleteId={pendingDeleteId}
                  onConnect={onConnectHost}
                  onOpenSftp={onOpenSftp}
                  onEdit={(target) => {
                    setPendingDeleteId(null);
                    setHostDialog({ mode: 'edit', node: target });
                  }}
                  onDelete={handleDeleteHost}
                />
              );
            })}
            {renderGroup(group.id, depth + 1)}
          </>
        ) : null}
      </div>
    ));

  const hasAnything = groups.length > 0 || hosts.length > 0;

  return (
    <div className="flex h-full flex-col overflow-hidden" onClick={() => setPendingDeleteId(null)}>
      {/* 工具栏 */}
      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border/50 px-4 py-2">
        <span className="text-ui-sm text-foreground-subtle select-none">
          {t('hosts.summary', { total: hosts.length })}
        </span>
        <div className="flex items-center gap-1.5">
          <Button
            variant="outline"
            onClick={() => {
              setPendingDeleteId(null);
              setGroupDialog({ mode: 'create', parentId: null });
            }}
          >
            {t('groups.newGroup')}
          </Button>
          <Button
            onClick={() => {
              setPendingDeleteId(null);
              setHostDialog({ mode: 'create' });
            }}
          >
            {t('hosts.newHost')}
          </Button>
        </div>
      </div>

      {/* 分组树 + 成员主机；未分组主机直接平铺在根级，不显示单独的「未分组」标题 */}
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-2" onClick={(e) => e.stopPropagation()}>
        {hasAnything ? (
          <>
            {renderGroup(null, 0)}
            {ungroupedHosts.map((host) => {
              const jumpHost =
                host.jumpHostId != null ? hosts.find((item) => item.id === host.jumpHostId) : undefined;
              return (
                <HostTreeRow
                  key={host.id}
                  host={host}
                  jumpHostName={jumpHost?.name}
                  depth={0}
                  pendingDeleteId={pendingDeleteId}
                  onConnect={onConnectHost}
                  onOpenSftp={onOpenSftp}
                  onEdit={(target) => {
                    setPendingDeleteId(null);
                    setHostDialog({ mode: 'edit', node: target });
                  }}
                  onDelete={handleDeleteHost}
                />
              );
            })}
          </>
        ) : (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 select-none">
            <span className="text-ui-sm text-foreground-subtlest">{t('hosts.empty')}</span>
            <Button variant="outline" onClick={() => setHostDialog({ mode: 'create' })}>
              {t('hosts.newHost')}
            </Button>
          </div>
        )}
      </div>

      {hostDialog ? (
        <HostFormDialog
          node={hostDialog.mode === 'edit' ? hostDialog.node : null}
          hosts={hosts}
          groups={groups}
          onSave={handleSaveHost}
          onClose={() => setHostDialog(null)}
        />
      ) : null}

      {groupDialog ? (
        <GroupFormDialog
          node={groupDialog.mode === 'edit' ? groupDialog.node : null}
          defaultParentId={groupDialog.mode === 'create' ? groupDialog.parentId : null}
          groups={groups}
          onSave={handleSaveGroup}
          onClose={() => setGroupDialog(null)}
        />
      ) : null}
    </div>
  );
}
