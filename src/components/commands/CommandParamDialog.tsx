import { Play, TriangleAlert } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { FormField, TextInput } from '@/components/hosts/formFields';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { cn } from '@/lib/utils';
import {
  extractPlaceholders,
  resolveCommand,
  type CommandRecord,
} from '@/services/commandService';
import { commandStore } from '@/services/commandStore';
import { sendToTerminal } from '@/services/aiTerminalTools';
import { useTerminalStore } from '@/terminal/TerminalProvider';

interface CommandParamDialogProps {
  record: CommandRecord;
  onClose: () => void;
}

/** 执行前参数填写 + 实时预览 + 危险级别确认 */
export function CommandParamDialog({ record, onClose }: CommandParamDialogProps) {
  const { t } = useTranslation();
  const terminalStore = useTerminalStore();
  const activeTerminalKey = terminalStore.activeSessionId;

  const placeholders = useMemo(() => extractPlaceholders(record.command), [record.command]);

  const [values, setValues] = useState<Record<string, string>>(() => {
    const init: Record<string, string> = {};
    for (const param of record.params) {
      init[param.name] = param.default || '';
    }
    // 没有结构化说明的占位符也给空值
    for (const name of placeholders) {
      if (!(name in init)) init[name] = '';
    }
    return init;
  });

  const resolvedCommand = useMemo(
    () => resolveCommand(record.command, values),
    [record.command, values],
  );

  const needsConfirm = record.dangerLevel !== 'safe';
  const isDestructive = record.dangerLevel === 'destructive';
  const hasEmptyRequired = record.params.some(
    (param) => param.required && !values[param.name]?.trim(),
  );

  const handleSend = async () => {
    if (activeTerminalKey == null) return;
    const result = await sendToTerminal(activeTerminalKey, `${resolvedCommand}\n`);
    if (result.ok) {
      void commandStore.used(record.id);
    }
    onClose();
  };

  return (
    <Modal
      title={record.title}
      onClose={onClose}
      className="w-[480px]"
      footer={
        <>
          <span className="mr-auto text-ui-xs text-foreground-subtlest">
            {t('commands.runInTerminal')}
          </span>
          <Button variant="ghost" onClick={onClose}>
            {t('common.cancel')}
          </Button>
          <Button
            variant={isDestructive ? 'destructive' : 'default'}
            disabled={hasEmptyRequired || activeTerminalKey == null}
            onClick={() => void handleSend()}
          >
            <Play className="size-3.5" />
            {t('commands.run')}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3.5">
        {/* 参数表单 */}
        {placeholders.length > 0 ? (
          placeholders.map((name) => {
            const param = record.params.find((p) => p.name === name);
            return (
              <FormField
                key={name}
                label={
                  <span className="flex items-center gap-1.5">
                    <code className="rounded bg-accent/50 px-1 font-mono text-ui-xs text-terminal-blue">
                      {name}
                    </code>
                    {param?.label || name}
                    {param?.required ? <span className="text-destructive">*</span> : null}
                  </span>
                }
              >
                <TextInput
                  value={values[name] ?? ''}
                  onChange={(e) => setValues((prev) => ({ ...prev, [name]: e.target.value }))}
                  placeholder={param?.example || param?.description || '输入参数值'}
                />
                {param?.description ? (
                  <span className="text-ui-xs text-foreground-subtlest">{param.description}</span>
                ) : null}
              </FormField>
            );
          })
        ) : null}

        {/* 命令预览 */}
        <FormField label={t('commands.preview')}>
          <code className="block overflow-x-auto rounded-md border border-border bg-surface px-2.5 py-2 font-mono text-ui-sm text-foreground whitespace-pre-wrap break-all select-all">
            {resolvedCommand}
          </code>
        </FormField>

        {/* 危险级别警告 */}
        {needsConfirm ? (
          <div
            className={cn(
              'flex items-start gap-2 rounded-lg border px-3 py-2',
              isDestructive
                ? 'border-destructive/40 bg-destructive/10'
                : 'border-warning/40 bg-warning/10',
            )}
          >
            <TriangleAlert
              className={cn(
                'mt-0.5 size-4 shrink-0',
                isDestructive ? 'text-destructive' : 'text-warning',
              )}
            />
            <div className="text-ui-sm">
              <span className={cn('font-medium', isDestructive ? 'text-destructive' : 'text-warning')}>
                {isDestructive
                  ? t('commands.confirmDestructive')
                  : t('commands.confirmWarning')}
              </span>
              {record.description ? (
                <p className="mt-0.5 text-foreground-subtle">{record.description}</p>
              ) : null}
            </div>
          </div>
        ) : null}

        {activeTerminalKey == null ? (
          <p className="text-ui-sm text-destructive">{t('commands.noTerminal')}</p>
        ) : null}
      </div>
    </Modal>
  );
}
