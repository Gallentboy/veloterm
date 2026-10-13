import { KeyRound, Loader2, Plus, RefreshCw, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { FormField, Select, TextInput } from '@/components/hosts/formFields';
import {
  listAiModels,
  saveAiProvider,
  testAiProvider,
  type AiProvider,
} from '@/services/aiService';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { Modal } from '@/components/ui/modal';

interface AiProviderDialogProps {
  /** 编辑时传入；新建为 null */
  node: AiProvider | null;
  onSave: () => void;
  onClose: () => void;
}

const PROVIDER_TYPES = [
  { value: 'openai', label: 'OpenAI 兼容' },
  { value: 'anthropic', label: 'Anthropic (Claude)' },
  { value: 'gemini', label: 'Google Gemini' },
] as const;

const TYPE_PLACEHOLDERS: Record<string, string> = {
  openai: 'gpt-4o-mini',
  anthropic: 'claude-sonnet-4-5',
  gemini: 'gemini-2.5-flash',
};

/** 各供应商官方默认接口地址（留空即用默认）：placeholder 展示 + 切换类型时清空提示 */
const TYPE_DEFAULT_BASE_URLS: Record<string, string> = {
  openai: 'https://api.openai.com/v1',
  anthropic: 'https://api.anthropic.com',
  gemini: 'https://generativelanguage.googleapis.com',
};

/** AI 供应商新建/编辑：名称/类型/端点/模型 + 多 Key 轮询池编辑 + 连通性测试 */
export function AiProviderDialog({ node, onSave, onClose }: AiProviderDialogProps) {
  const { t } = useTranslation();
  const [name, setName] = useState(node?.name ?? '');
  const [providerType, setProviderType] = useState<string>(node?.providerType ?? 'openai');
  const [baseUrl, setBaseUrl] = useState(node?.baseUrl ?? '');
  const [model, setModel] = useState(node?.model ?? '');
  const [apiKeys, setApiKeys] = useState<string[]>(node?.apiKeys ?? ['']);
  const [isDefault, setIsDefault] = useState(node?.isDefault ?? false);
  const [error, setError] = useState<string | null>(null);
  const [testState, setTestState] = useState<'idle' | 'testing' | 'ok' | 'fail'>('idle');
  const [models, setModels] = useState<string[]>([]);
  const [isLoadingModels, setIsLoadingModels] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [testMessage, setTestMessage] = useState('');

  const updateKey = (index: number, value: string) => {
    setApiKeys((prev) => prev.map((k, i) => (i === index ? value : k)));
  };

  const addKey = () => setApiKeys((prev) => [...prev, '']);
  const removeKey = (index: number) => {
    setApiKeys((prev) => (prev.length <= 1 ? prev : prev.filter((_, i) => i !== index)));
  };

  async function handleLoadModels() {
    setIsLoadingModels(true);
    setModelsError(null);
    try {
      const list = await listAiModels(buildRecord());
      setModels(list);
      if (list.length === 0) {
        setModelsError(t('ai.provider.noModels'));
      }
    } catch (err) {
      setModels([]);
      setModelsError(String(err));
    } finally {
      setIsLoadingModels(false);
    }
  }

  async function handleTest() {
    setTestState('testing');
    setTestMessage('');
    const record = buildRecord();
    let received = '';
    try {
      await testAiProvider(record, (chunk) => {
        if (chunk.delta) received += chunk.delta;
        if (chunk.error) {
          setTestMessage(chunk.error);
        }
      });
      setTestState(received ? 'ok' : 'fail');
      if (!received && !testMessage) setTestMessage(t('ai.provider.testEmpty'));
    } catch (err) {
      setTestState('fail');
      setTestMessage(String(err));
    }
  }

  function buildRecord(): AiProvider {
    return {
      id: node?.id ?? Date.now() % 100000,
      name: name.trim(),
      providerType,
      baseUrl: baseUrl.trim() || null,
      model: model.trim(),
      apiKeys: apiKeys.map((k) => k.trim()).filter((k) => k.length > 0),
      isDefault,
      createdAt: node?.createdAt ?? Date.now(),
      updatedAt: Date.now(),
    };
  }

  async function handleSave() {
    if (!name.trim() || !model.trim()) {
      setError(t('ai.provider.required'));
      return;
    }
    if (apiKeys.every((k) => !k.trim())) {
      setError(t('ai.provider.keyRequired'));
      return;
    }
    try {
      await saveAiProvider(buildRecord());
      onSave();
    } catch (err) {
      setError(String(err));
    }
  }

  return (
    <Modal
      title={node ? t('ai.provider.editTitle') : t('ai.provider.newTitle')}
      onClose={onClose}
      footer={
        <>
          <Button variant="outline" onClick={() => void handleTest()} disabled={testState === 'testing'}>
            {testState === 'testing' ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : null}
            {t('ai.provider.test')}
          </Button>
          <div className="flex-1" />
          {testState === 'ok' && (
            <span className="mr-2 max-w-48 truncate text-ui-xs text-success">{t('ai.provider.testOk')}</span>
          )}
          {testState === 'fail' && (
            <span className="mr-2 max-w-56 truncate text-ui-xs text-destructive" title={testMessage}>
              {testMessage || t('ai.provider.testFail')}
            </span>
          )}
          <Button variant="ghost" onClick={onClose}>
            {t('common.cancel')}
          </Button>
          <Button onClick={() => void handleSave()}>{t('common.save')}</Button>
        </>
      }
    >
      <div className="flex flex-col gap-3.5">
        <FormField label={t('ai.provider.name')}>
          <TextInput value={name} onChange={(e) => setName(e.target.value)} placeholder="DeepSeek" />
        </FormField>

        <div className="flex gap-3">
          <div className="w-44 shrink-0">
            <FormField label={t('ai.provider.type')}>
              <Select
                value={providerType}
                onChange={(e) => {
                  const next = e.target.value;
                  if (next !== providerType) {
                    setProviderType(next);
                    setBaseUrl('');
                  }
                }}
              >
                {PROVIDER_TYPES.map((type) => (
                  <option key={type.value} value={type.value}>
                    {type.label}
                  </option>
                ))}
              </Select>
            </FormField>
          </div>
          <div className="min-w-0 flex-1">
            <FormField label={t('ai.provider.model')}>
              <div className="flex items-center gap-1.5">
                <TextInput
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder={TYPE_PLACEHOLDERS[providerType]}
                />
                <Button
                  variant="outline"
                  size="icon-md"
                  onClick={() => void handleLoadModels()}
                  disabled={isLoadingModels}
                  aria-label={t('ai.provider.loadModels')}
                  title={t('ai.provider.loadModels')}
                  className="shrink-0"
                >
                  <RefreshCw className={cn('size-3.5', isLoadingModels && 'animate-spin')} />
                </Button>
              </div>
              {modelsError ? (
                <span className="text-ui-xs text-destructive">{modelsError}</span>
              ) : null}
              {models.length > 0 ? (
                <div className="mt-1 flex max-h-24 flex-wrap gap-1 overflow-y-auto rounded-md border border-border bg-surface/50 p-1.5">
                  {models.map((m) => (
                    <button
                      key={m}
                      type="button"
                      onClick={() => setModel(m)}
                      className={cn(
                        'rounded-full px-2 py-0.5 font-mono text-ui-xs transition-colors',
                        m === model
                          ? 'bg-primary text-primary-foreground'
                          : 'bg-surface text-foreground-subtle hover:bg-surface-hover hover:text-foreground',
                      )}
                    >
                      {m}
                    </button>
                  ))}
                </div>
              ) : null}
            </FormField>
          </div>
        </div>

        <FormField label={`${t('ai.provider.baseUrl')}（${t('common.optional')}）`}>
          <TextInput
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder={TYPE_DEFAULT_BASE_URLS[providerType]}
          />
          <span className="text-ui-xs text-foreground-subtlest">
            {t('ai.provider.baseUrlHint', { url: TYPE_DEFAULT_BASE_URLS[providerType] })}
          </span>
        </FormField>

        <FormField label={t('ai.provider.keys')}>
          <div className="flex flex-col gap-1.5">
            {apiKeys.map((key, index) => (
              <div key={index} className="flex items-center gap-1.5">
                <KeyRound className="size-3.5 shrink-0 text-foreground-subtlest" />
                <TextInput
                  type="password"
                  value={key}
                  onChange={(e) => updateKey(index, e.target.value)}
                  placeholder={`sk-... (${t('ai.provider.keyN', { index: index + 1 })})`}
                  autoComplete="off"
                />
                <Button
                  variant="ghost"
                  size="icon-sm"
                  onClick={addKey}
                  aria-label={t('ai.provider.addKey')}
                  title={t('ai.provider.addKey')}
                >
                  <Plus className="size-3.5" />
                </Button>
                {apiKeys.length > 1 && (
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => removeKey(index)}
                    aria-label={t('common.delete')}
                  >
                    <Trash2 className="size-3.5 text-destructive" />
                  </Button>
                )}
              </div>
            ))}
            <span className="text-ui-xs text-foreground-subtlest">
              {t('ai.provider.rotationHint')}
            </span>
          </div>
        </FormField>

        <label className="flex cursor-pointer items-center gap-2 select-none">
          <input
            type="checkbox"
            checked={isDefault}
            onChange={(e) => setIsDefault(e.target.checked)}
            className="accent-foreground"
          />
          <span className="text-ui-sm text-foreground">{t('ai.provider.isDefault')}</span>
        </label>

        {error && <p className="text-ui-sm text-destructive">{error}</p>}
      </div>
    </Modal>
  );
}

