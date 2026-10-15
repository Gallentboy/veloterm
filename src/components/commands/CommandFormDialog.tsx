import { Pin } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { MarkdownContent } from '@/components/ai/MarkdownContent';
import { FormField, TextArea, TextInput } from '@/components/hosts/formFields';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { SelectMenu } from '@/components/ui/SelectMenu';
import { cn } from '@/lib/utils';
import {
  extractPlaceholders,
  type CommandParam,
  type CommandRecord,
  type DangerLevel,
} from '@/services/commandService';
import { commandStore } from '@/services/commandStore';

const CATEGORIES = [
  { value: '', label: '—' },
  { value: 'file', label: '文件与目录' },
  { value: 'text', label: '文本处理' },
  { value: 'archive', label: '归档与压缩' },
  { value: 'package', label: '包管理' },
  { value: 'permission', label: '权限与用户' },
  { value: 'shell', label: 'Shell 与环境' },
  { value: 'schedule', label: '定时与作业' },
  { value: 'network', label: '网络' },
  { value: 'disk', label: '磁盘' },
  { value: 'process', label: '进程' },
  { value: 'system', label: '系统与服务' },
  { value: 'docker', label: 'Docker' },
  { value: 'gpu', label: 'GPU' },
  { value: 'ssh', label: 'SSH' },
  { value: 'other', label: '其他' },
];

const DANGER_OPTIONS: Array<{ value: DangerLevel; label: string }> = [
  { value: 'safe', label: '安全' },
  { value: 'confirm', label: '需确认' },
  { value: 'destructive', label: '危险' },
];

interface CommandFormDialogProps {
  node: CommandRecord | null;
  onClose: () => void;
  onSave: () => void;
}

/**
 * 新建/编辑命令：标题+命令模板（占位符高亮）+ 自动提取参数表单。
 *
 * 布局按**窗口宽度**响应式：≥ 860px 双列（左列命令核心：标题/模板/参数/示例，
 * 右列元信息：详细解释/用途/分类标签/危险级别），窄窗口回落单列堆叠。
 * 对话框宽 760px（与主机编辑对话框同规格），max-w-full 在窄窗口自动收缩，
 * 此时单列避免每列被挤得不可用——860px ≈ 760 + 遮罩内边距的临界点。
 *
 * 「详细解释」支持 Markdown：编辑 / 预览切换，预览复用 AI 气泡的
 * MarkdownContent（react-markdown + GFM + 代码高亮，主题 token 化）。
 */
export function CommandFormDialog({ node, onClose, onSave }: CommandFormDialogProps) {
  const { t } = useTranslation();
  const [title, setTitle] = useState(node?.title ?? '');
  const [command, setCommand] = useState(node?.command ?? '');
  const [description, setDescription] = useState(node?.description ?? '');
  const [purpose, setPurpose] = useState(node?.purpose ?? '');
  const [category, setCategory] = useState(node?.category ?? '');
  const [tagsInput, setTagsInput] = useState((node?.tags ?? []).join(', '));
  const [params, setParams] = useState<CommandParam[]>(node?.params ?? []);
  const [dangerLevel, setDangerLevel] = useState<DangerLevel>(node?.dangerLevel ?? 'safe');
  const [exampleOutput, setExampleOutput] = useState(node?.exampleOutput ?? '');
  const [isPinned, setIsPinned] = useState(node?.isPinned ?? false);
  const [error, setError] = useState<string | null>(null);
  // 详细解释的 Markdown 编辑/预览模式（仅编辑态受控输入；预览态渲染纯文本）
  const [mdPreviewing, setMdPreviewing] = useState(false);

  // 从命令模板自动提取占位符，同步 params 数组
  const placeholders = useMemo(() => extractPlaceholders(command), [command]);

  useEffect(() => {
    setParams((current) => {
      const next = placeholders.map((name) => {
        const existing = current.find((p) => p.name === name);
        if (existing) return existing;
        return {
          name,
          label: name,
          description: '',
          required: true,
          default: '',
          example: '',
        };
      });
      return next;
    });
  }, [placeholders]);

  const updateParam = (name: string, field: keyof CommandParam, value: string | boolean) => {
    setParams((prev) =>
      prev.map((p) => (p.name === name ? { ...p, [field]: value } : p)),
    );
  };

  const buildRecord = (): CommandRecord => ({
    id: node?.id ?? 0,
    title: title.trim(),
    command: command.trim(),
    description: description.trim(),
    purpose: purpose.trim(),
    category,
    tags: tagsInput.split(/[，,]/).map((s) => s.trim()).filter(Boolean),
    params,
    dangerLevel,
    exampleOutput: exampleOutput.trim(),
    usageCount: node?.usageCount ?? 0,
    lastUsedAt: node?.lastUsedAt ?? null,
    isPinned,
    createdAt: node?.createdAt ?? 0,
    updatedAt: 0,
  });

  const handleSave = async () => {
    if (!title.trim() || !command.trim()) {
      setError(t('commands.form.required'));
      return;
    }
    try {
      await commandStore.save(buildRecord());
      onSave();
    } catch (err) {
      setError(String(err));
    }
  };

  /** 命令模板预览：高亮 {{占位符}} */
  const commandPreview = useMemo(() => {
    const parts = command.split(/(\{\{[^}]+\}\})/g);
    return parts.map((part, i) => {
      if (part.startsWith('{{') && part.endsWith('}}')) {
        return (
          <span key={i} className="rounded bg-accent/50 px-0.5 text-terminal-blue">
            {part}
          </span>
        );
      }
      return <span key={i}>{part}</span>;
    });
  }, [command]);

  return (
    <Modal
      title={node ? t('commands.form.editTitle') : t('commands.form.newTitle')}
      onClose={onClose}
      className="w-[760px]"
      footer={
        <>
          <label className="mr-auto flex cursor-pointer items-center gap-1.5 select-none">
            <input
              type="checkbox"
              checked={isPinned}
              onChange={(e) => setIsPinned(e.target.checked)}
              className="accent-foreground"
            />
            <Pin className={cn('size-3.5', isPinned ? 'text-primary' : 'text-foreground-subtlest')} />
            <span className="text-ui-sm text-foreground-subtle">{t('commands.form.pin')}</span>
          </label>
          <Button variant="ghost" onClick={onClose}>
            {t('common.cancel')}
          </Button>
          <Button onClick={() => void handleSave()}>{t('common.save')}</Button>
        </>
      }
    >
      {/* 窗口 ≥ 860px 双列（对话框 760px 不被压缩的临界），窄窗口回落单列堆叠 */}
      <div className="grid grid-cols-1 items-start gap-x-5 gap-y-3.5 min-[860px]:grid-cols-2">
        {/* 左列：命令核心 */}
        <div className="flex min-w-0 flex-col gap-3.5">
          <FormField label={t('commands.form.title')}>
            <TextInput value={title} onChange={(e) => setTitle(e.target.value)} placeholder="sed 全局替换文件内容" />
          </FormField>

          <FormField label={t('commands.form.command')}>
            <div className="flex flex-col gap-1.5">
              <TextArea
                value={command}
                onChange={(e) => setCommand(e.target.value)}
                placeholder="sed -i 's/{{old}}/{{new}}/g' {{file}}"
                spellCheck={false}
              />
              <div className="rounded-md border border-border/60 bg-surface px-2.5 py-1.5 font-mono text-ui-sm whitespace-pre-wrap">
                {commandPreview}
              </div>
            </div>
          </FormField>

          {/* 占位符参数（从模板自动提取） */}
          {params.length > 0 ? (
            <FormField label={t('commands.form.params')}>
              <div className="flex flex-col gap-2 rounded-lg border border-border bg-surface/30 p-2">
                {params.map((param) => (
                  <div key={param.name} className="flex flex-col gap-1">
                    <div className="flex items-center gap-2">
                      <code className="shrink-0 rounded bg-accent/50 px-1.5 font-mono text-ui-xs text-terminal-blue">
                        {param.name}
                      </code>
                      <TextInput
                        value={param.label}
                        onChange={(e) => updateParam(param.name, 'label', e.target.value)}
                        placeholder="显示名"
                        className="h-6 min-w-0 flex-1 text-ui-xs"
                      />
                      <TextInput
                        value={param.default}
                        onChange={(e) => updateParam(param.name, 'default', e.target.value)}
                        placeholder="默认值"
                        className="h-6 w-24 shrink-0 text-ui-xs"
                      />
                    </div>
                    <TextInput
                      value={param.description}
                      onChange={(e) => updateParam(param.name, 'description', e.target.value)}
                      placeholder="参数说明"
                      className="h-6 text-ui-xs"
                    />
                  </div>
                ))}
              </div>
            </FormField>
          ) : null}

          <FormField label={`${t('commands.form.exampleOutput')}（${t('common.optional')}）`}>
            <TextArea
              value={exampleOutput}
              onChange={(e) => setExampleOutput(e.target.value)}
              placeholder="PORT=3307"
              className="h-16"
            />
          </FormField>
        </div>

        {/* 右列：元信息 */}
        <div className="flex min-w-0 flex-col gap-3.5">
          <FormField label={t('commands.form.description')}>
            <div className="flex flex-col gap-1.5">
              {/* Markdown 编辑 / 预览切换 */}
              <div className="flex items-center gap-1 self-end">
                {([false, true] as const).map((mode) => (
                  <button
                    key={String(mode)}
                    type="button"
                    onClick={() => setMdPreviewing(mode)}
                    className={cn(
                      'rounded-md px-2 py-0.5 text-ui-xs transition-colors select-none',
                      mdPreviewing === mode
                        ? 'bg-selected text-foreground'
                        : 'text-foreground-subtle hover:bg-hover hover:text-foreground',
                    )}
                  >
                    {mode ? t('commands.form.mdPreview') : t('commands.form.mdEdit')}
                  </button>
                ))}
              </div>
              {mdPreviewing ? (
                <div className="scrollbar-slim h-24 overflow-y-auto rounded-md border border-border bg-input px-2.5 py-1.5 text-ui-sm">
                  {description.trim() ? (
                    <MarkdownContent content={description} />
                  ) : (
                    <p className="text-ui-sm text-foreground-subtlest select-none">
                      {t('commands.form.mdEmptyPreview')}
                    </p>
                  )}
                </div>
              ) : (
                <TextArea
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder={'支持 Markdown：`-i` 就地修改、**s/old/new/g** 全局替换\n- 列表项\n```bash\n示例代码块\n```'}
                />
              )}
            </div>
          </FormField>

          <FormField label={t('commands.form.purpose')}>
            <TextInput
              value={purpose}
              onChange={(e) => setPurpose(e.target.value)}
              placeholder="批量修改配置文件中的端口/密码"
            />
          </FormField>

          <div className="flex gap-3">
            <div className="w-32 shrink-0">
              <FormField label={t('commands.form.category')}>
                <SelectMenu
                  aria-label={t('commands.form.category')}
                  value={category}
                  onChange={setCategory}
                  options={CATEGORIES}
                />
              </FormField>
            </div>
            <div className="min-w-0 flex-1">
              <FormField label={t('commands.form.tags')}>
                <TextInput
                  value={tagsInput}
                  onChange={(e) => setTagsInput(e.target.value)}
                  placeholder="sed, 替换, 文本"
                />
              </FormField>
            </div>
          </div>

          <FormField label={t('commands.form.danger')}>
            <div className="flex gap-1.5">
              {DANGER_OPTIONS.map((opt) => (
                <button
                  key={opt.value}
                  type="button"
                  onClick={() => setDangerLevel(opt.value)}
                  className={cn(
                    'flex-1 rounded-lg border px-2 py-1.5 text-ui-sm transition-colors',
                    dangerLevel === opt.value
                      ? opt.value === 'destructive'
                        ? 'border-destructive/50 bg-destructive/10 text-destructive'
                        : opt.value === 'confirm'
                          ? 'border-warning/50 bg-warning/10 text-warning'
                          : 'border-border-hover bg-surface text-foreground'
                      : 'border-border text-foreground-subtle hover:bg-surface-hover',
                  )}
                >
                  {opt.label}
                </button>
              ))}
            </div>
          </FormField>
        </div>

        {error ? (
          <p className="col-span-full text-ui-sm text-destructive">{error}</p>
        ) : null}
      </div>
    </Modal>
  );
}
