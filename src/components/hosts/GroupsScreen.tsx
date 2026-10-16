import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { GroupFormDialog } from '@/components/hosts/GroupFormDialog';
import { GroupTreeRow } from '@/components/hosts/HostTree';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/ConfirmDialog';
import { useHostsStore } from '@/hosts/HostsProvider';
import type { GroupNode } from '@/hosts/types';

type GroupDialogState =
  | { mode: 'create'; parentId: number | null }
  | { mode: 'edit'; node: GroupNode }
  | null;

/** 分组管理页：纯分组树维护（多级嵌套、编辑、删除），主机在主机页管理。 */
export function GroupsScreen() {
  const { t } = useTranslation();
  const { groups, childGroups, createGroup, updateGroup, removeGroup, isExpanded, toggleExpanded } =
    useHostsStore();

  const [groupDialog, setGroupDialog] = useState<GroupDialogState>(null);
  // 删除确认弹窗目标（工程约定：删除必须弹窗确认）
  const [deleteGroup, setDeleteGroup] = useState<GroupNode | null>(null);

  const handleDelete = (id: number) => {
    // 删分组连带子分组，但主机只解除关联不删除；后果说明在弹窗里展示
    const target = groups.find((item) => item.id === id);
    if (target) {
      setDeleteGroup(target);
    }
  };

  const handleSave = (result: { name: string; parentId: number | null }) => {
    if (groupDialog?.mode === 'edit') {
      updateGroup(groupDialog.node.id, result);
    } else if (groupDialog?.mode === 'create') {
      createGroup(result);
    }
    setGroupDialog(null);
  };

  const renderGroups = (parentId: number | null, depth: number) =>
    childGroups(parentId).map((group) => (
      <div key={group.id} className="flex flex-col">
        <GroupTreeRow
          group={group}
          depth={depth}
          expanded={isExpanded(group.id)}
          showAddChild
          onToggle={toggleExpanded}
          onEdit={(target) => {
            setDeleteGroup(null);
            setGroupDialog({ mode: 'edit', node: target });
          }}
          onDelete={handleDelete}
          onAddChild={(pid) => {
            setDeleteGroup(null);
            setGroupDialog({ mode: 'create', parentId: pid });
          }}
        />
        {isExpanded(group.id) ? renderGroups(group.id, depth + 1) : null}
      </div>
    ));

  return (
    <div className="flex h-full flex-col overflow-hidden" onClick={() => setDeleteGroup(null)}>
      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border/50 px-4 py-2">
        <span className="text-ui-sm text-foreground-subtle select-none">
          {t('groups.summary', { total: groups.length })}
        </span>
        <Button
          variant="outline"
          size="lg"
          onClick={() => {
            setDeleteGroup(null);
            setGroupDialog({ mode: 'create', parentId: null });
          }}
        >
          {t('groups.newGroup')}
        </Button>
      </div>

      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-2" onClick={(e) => e.stopPropagation()}>
        {groups.length > 0 ? (
          renderGroups(null, 0)
        ) : (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 select-none">
            <span className="text-ui-sm text-foreground-subtlest">{t('groups.empty')}</span>
            <Button variant="outline" size="lg" onClick={() => setGroupDialog({ mode: 'create', parentId: null })}>
              {t('groups.newGroup')}
            </Button>
          </div>
        )}
      </div>

      {groupDialog ? (
        <GroupFormDialog
          node={groupDialog.mode === 'edit' ? groupDialog.node : null}
          defaultParentId={groupDialog.mode === 'create' ? groupDialog.parentId : null}
          groups={groups}
          onSave={handleSave}
          onClose={() => setGroupDialog(null)}
        />
      ) : null}

      <ConfirmDialog
        open={deleteGroup !== null}
        title={t('common.confirmDeleteTitle')}
        description={
          <span>
            {t('common.confirmDelete', { name: deleteGroup?.name ?? '' })}
            <br />
            {t('groups.deleteDesc')}
          </span>
        }
        onConfirm={() => {
          if (deleteGroup) {
            removeGroup(deleteGroup.id);
          }
          setDeleteGroup(null);
        }}
        onCancel={() => setDeleteGroup(null)}
      />
    </div>
  );
}
