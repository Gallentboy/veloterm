import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { GroupFormDialog } from '@/components/hosts/GroupFormDialog';
import { HostFormDialog, type HostFormResult } from '@/components/hosts/HostFormDialog';
import { PortForwardDialog } from '@/components/hosts/PortForwardDialog';
import { HostTopologyDialog } from '@/components/hosts/HostTopologyDialog';
import { GlobalTopologyDialog } from '@/components/hosts/GlobalTopologyDialog';
import { GroupTreeRow, HostTreeRow } from '@/components/hosts/HostTree';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/ConfirmDialog';
import { useHostsStore } from '@/hosts/HostsProvider';
import type { GroupNode, HostNode } from '@/hosts/types';

type HostDialogState =
  | { mode: 'create' }
  | { mode: 'edit'; node: HostNode }
  | { mode: 'duplicate'; node: HostNode }
  | null;
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
  // 删除确认弹窗目标（工程约定：删除必须弹窗确认，严禁两次点击式或无确认直接删）
  const [deleteTarget, setDeleteTarget] = useState<
    | { kind: 'host'; id: number; name: string; subtitle: string }
    | { kind: 'group'; id: number; name: string }
    | null
  >(null);
  // 端口转发管理弹层目标主机
  const [forwardsHost, setForwardsHost] = useState<HostNode | null>(null);
  const [topologyHost, setTopologyHost] = useState<HostNode | null>(null);
  const [globalTopology, setGlobalTopology] = useState(false);

  const handleDeleteHost = (id: number) => {
    const host = hosts.find((item) => item.id === id);
    if (!host) return;
    setDeleteTarget({
      kind: 'host',
      id,
      name: host.name,
      subtitle: `${host.username || '—'}@${host.host}:${host.port}`,
    });
  };

  const handleDeleteGroup = (id: number) => {
    // 主机页内也允许直接删分组（只解除关联，主机保留）；后果说明在弹窗里展示
    const group = groups.find((item) => item.id === id);
    if (!group) return;
    setDeleteTarget({ kind: 'group', id, name: group.name });
  };

  const handleDeleteConfirm = () => {
    if (!deleteTarget) return;
    if (deleteTarget.kind === 'host') {
      removeHost(deleteTarget.id);
    } else {
      removeGroup(deleteTarget.id);
    }
    setDeleteTarget(null);
  };

  const handleSaveHost = (result: HostFormResult) => {
    if (hostDialog?.mode === 'edit') {
      updateHost(hostDialog.node.id, result);
    } else {
      // create 与 duplicate 都落库为新主机（复制=预填态的新建，不影响原主机）
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
          onToggle={toggleExpanded}
          onEdit={(target) => {
            setDeleteTarget(null);
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
                          onConnect={onConnectHost}
                  onOpenSftp={onOpenSftp}
                  onForwards={setForwardsHost}
                  onTopology={setTopologyHost}
                  onDuplicate={(target) => {
                    setDeleteTarget(null);
                    setHostDialog({ mode: 'duplicate', node: target });
                  }}
                  onEdit={(target) => {
                    setDeleteTarget(null);
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
    <div className="flex h-full flex-col overflow-hidden" onClick={() => setDeleteTarget(null)}>
      {/* 工具栏 */}
      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border/50 px-4 py-2">
        <span className="text-ui-sm text-foreground-subtle select-none">
          {t('hosts.summary', { total: hosts.length })}
        </span>
        <div className="flex items-center gap-1.5">
          <Button
            variant="outline"
            size="lg"
            onClick={() => {
              setDeleteTarget(null);
              setGlobalTopology(true);
            }}
          >
            {t('topology.globalEntry')}
          </Button>
          <Button
            variant="outline"
            size="lg"
            onClick={() => {
              setDeleteTarget(null);
              setGroupDialog({ mode: 'create', parentId: null });
            }}
          >
            {t('groups.newGroup')}
          </Button>
          <Button
            size="lg"
            onClick={() => {
              setDeleteTarget(null);
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
                          onConnect={onConnectHost}
                  onOpenSftp={onOpenSftp}
                  onForwards={setForwardsHost}
                  onTopology={setTopologyHost}
                  onDuplicate={(target) => {
                    setDeleteTarget(null);
                    setHostDialog({ mode: 'duplicate', node: target });
                  }}
                  onEdit={(target) => {
                    setDeleteTarget(null);
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
            <Button variant="outline" size="lg" onClick={() => setHostDialog({ mode: 'create' })}>
              {t('hosts.newHost')}
            </Button>
          </div>
        )}
      </div>

      {forwardsHost ? (
        <PortForwardDialog host={forwardsHost} onClose={() => setForwardsHost(null)} />
      ) : null}

      {topologyHost ? (
        <HostTopologyDialog
          host={topologyHost}
          hosts={hosts}
          onClose={() => setTopologyHost(null)}
        />
      ) : null}

      {globalTopology ? (
        <GlobalTopologyDialog
          hosts={hosts}
          onClose={() => setGlobalTopology(false)}
          onInspect={(host) => {
            setGlobalTopology(false);
            setTopologyHost(host);
          }}
        />
      ) : null}

      {hostDialog ? (
        <HostFormDialog
          node={hostDialog.mode === 'edit' || hostDialog.mode === 'duplicate' ? hostDialog.node : null}
          isDuplicate={hostDialog.mode === 'duplicate'}
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

      <ConfirmDialog
        open={deleteTarget !== null}
        title={t('common.confirmDeleteTitle')}
        description={
          deleteTarget?.kind === 'group' ? (
            <span>
              {t('common.confirmDelete', { name: deleteTarget.name })}
              <br />
              {t('groups.deleteDesc')}
            </span>
          ) : (
            t('common.confirmDelete', { name: deleteTarget?.name ?? '' })
          )
        }
        detail={deleteTarget?.kind === 'host' ? deleteTarget.subtitle : undefined}
        onConfirm={handleDeleteConfirm}
        onCancel={() => setDeleteTarget(null)}
      />
    </div>
  );
}
