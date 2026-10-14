import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { FormField, Select, TextInput } from '@/components/hosts/formFields';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { groupPathOptions } from '@/hosts/storage';
import type { GroupNode } from '@/hosts/types';

interface GroupFormDialogProps {
  /** 编辑时传入已有分组；新建为 null */
  node: GroupNode | null;
  /** 新建时的默认父分组 */
  defaultParentId: number | null;
  groups: GroupNode[];
  onSave: (result: { name: string; parentId: number | null }) => void;
  onClose: () => void;
}

/** 分组新建/编辑对话框：名称 + 上级分组（排除自身子树防环，支持多级嵌套）。 */
export function GroupFormDialog({ node, defaultParentId, groups, onSave, onClose }: GroupFormDialogProps) {
  const { t } = useTranslation();
  const [name, setName] = useState(node?.name ?? '');
  const [parentId, setParentId] = useState<number | null>(node?.parentId ?? defaultParentId);
  const [error, setError] = useState<string | null>(null);

  const options = groupPathOptions(groups, node?.id);

  const handleSave = () => {
    if (!name.trim()) {
      setError(t('hosts.form.nameRequired'));
      return;
    }
    onSave({ name: name.trim(), parentId });
  };

  return (
    <Modal
      title={node ? t('groups.form.editTitle') : t('groups.form.newTitle')}
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={onClose}>
            {t('hosts.form.cancel')}
          </Button>
          <Button size="lg" className="h-8 px-3 text-ui-base" onClick={handleSave}>
            {t('hosts.form.save')}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3.5">
        <FormField label={t('groups.form.name')}>
          <TextInput value={name} onChange={(e) => setName(e.target.value)} placeholder="生产环境" />
        </FormField>
        <FormField label={t('groups.form.parent')}>
          <Select value={parentId ?? ''} onChange={(e) => setParentId(e.target.value ? Number(e.target.value) : null)}>
            <option value="">{t('groups.form.root')}</option>
            {options.map((option) => (
              <option key={option.id} value={option.id}>
                {option.path}
              </option>
            ))}
          </Select>
        </FormField>
        {error ? <p className="text-ui-sm text-destructive">{error}</p> : null}
      </div>
    </Modal>
  );
}
