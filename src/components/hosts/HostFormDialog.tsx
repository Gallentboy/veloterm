import { ChevronDown } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { FormField, PasswordInput, Select, TextArea, TextInput } from '@/components/hosts/formFields';
import { groupPathOptions, jumpHostOptions } from '@/hosts/storage';
import type {
  AuthConfig,
  AuthMethod,
  GroupNode,
  HostNode,
  SshHostKeyVerificationMode,
} from '@/hosts/types';
import { cn } from '@/lib/utils';

export interface HostFormResult {
  name: string;
  host: string;
  port: number;
  username: string;
  auth: AuthConfig;
  jumpHostId: number | null;
  hostKeyVerification: SshHostKeyVerificationMode;
  allowInsecureAlgos: boolean;
  /** 所属分组（可多选） */
  groupIds: number[];
}

interface HostFormDialogProps {
  /** 编辑时传入已有节点；新建为 null */
  node: HostNode | null;
  hosts: HostNode[];
  groups: GroupNode[];
  onSave: (result: HostFormResult) => void;
  onClose: () => void;
}

const AUTH_METHODS: Array<{ method: AuthMethod; labelKey: string }> = [
  { method: 'password', labelKey: 'hosts.form.authPassword' },
  { method: 'key', labelKey: 'hosts.form.authKey' },
  { method: 'agent', labelKey: 'hosts.form.authAgent' },
];

const HOST_KEY_POLICIES: Array<{ mode: SshHostKeyVerificationMode; labelKey: string }> = [
  { mode: 'auto_accept', labelKey: 'hosts.form.hostKeyAutoAccept' },
  { mode: 'auto_reject', labelKey: 'hosts.form.hostKeyAutoReject' },
  { mode: 'ignore', labelKey: 'hosts.form.hostKeyIgnore' },
];

/** 主机新建/编辑对话框：主机名 + 地址/端口/用户名 + 三种认证方式 + 跳板机/公钥策略/旧版算法兼容 + 所属分组（多选）。 */
export function HostFormDialog({ node, hosts, groups, onSave, onClose }: HostFormDialogProps) {
  const { t } = useTranslation();
  const [name, setName] = useState(node?.name ?? '');
  const [host, setHost] = useState(node?.host ?? '');
  const [port, setPort] = useState(String(node?.port ?? 22));
  const [username, setUsername] = useState(node?.username ?? '');
  const [auth, setAuth] = useState<AuthConfig>(() => node?.auth ?? { method: 'password', password: '' });
  const [jumpHostId, setJumpHostId] = useState<number | null>(node?.jumpHostId ?? null);
  const [hostKeyVerification, setHostKeyVerification] = useState<SshHostKeyVerificationMode>(
    node?.hostKeyVerification ?? 'auto_accept',
  );
  const [allowInsecureAlgos, setAllowInsecureAlgos] = useState<boolean>(
    node?.allowInsecureAlgos ?? false,
  );
  const [groupIds, setGroupIds] = useState<number[]>(node?.groupIds ?? []);
  const [error, setError] = useState<string | null>(null);

  const groupOptions = groupPathOptions(groups);
  const availableJumpHosts = jumpHostOptions(hosts, node?.id);

  const toggleGroup = (id: number) => {
    setGroupIds((current) =>
      current.includes(id) ? current.filter((gid) => gid !== id) : [...current, id],
    );
  };

  const handleSave = () => {
    const portNumber = Number(port);
    if (!name.trim()) {
      setError(t('hosts.form.nameRequired'));
      return;
    }
    if (!host.trim()) {
      setError(t('hosts.form.hostRequired'));
      return;
    }
    if (!Number.isInteger(portNumber) || portNumber < 1 || portNumber > 65535) {
      setError(t('hosts.form.portInvalid'));
      return;
    }
    // 过滤掉已被删除的分组与跳板引用
    const validGroupIds = groupIds.filter((id) => groups.some((group) => group.id === id));
    const validJumpHostId =
      jumpHostId != null && availableJumpHosts.some((item) => item.id === jumpHostId)
        ? jumpHostId
        : null;
    onSave({
      name: name.trim(),
      host: host.trim(),
      port: portNumber,
      username: username.trim(),
      auth,
      jumpHostId: validJumpHostId,
      hostKeyVerification,
      allowInsecureAlgos,
      groupIds: validGroupIds,
    });
  };

  return (
    <Modal
      title={node ? t('hosts.form.editTitle') : t('hosts.form.newTitle')}
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
        <FormField label={t('hosts.form.hostName')}>
          <TextInput value={name} onChange={(e) => setName(e.target.value)} placeholder="web-1" />
        </FormField>

        <div className="flex gap-3">
          <div className="min-w-0 flex-1">
            <FormField label={t('hosts.form.host')}>
              <TextInput
                value={host}
                onChange={(e) => setHost(e.target.value)}
                placeholder="192.168.1.10"
              />
            </FormField>
          </div>
          <div className="w-24 shrink-0">
            <FormField label={t('hosts.form.port')}>
              <TextInput
                type="number"
                value={port}
                onChange={(e) => setPort(e.target.value)}
                min={1}
                max={65535}
              />
            </FormField>
          </div>
        </div>

        <FormField label={t('hosts.form.username')}>
          <TextInput value={username} onChange={(e) => setUsername(e.target.value)} placeholder="root" />
        </FormField>

        <FormField label={t('hosts.form.auth')}>
          <div className="flex flex-col gap-1.5">
            {AUTH_METHODS.map(({ method, labelKey }) => (
              <label
                key={method}
                className={cn(
                  'flex cursor-pointer items-center gap-2 rounded-lg border p-2 transition-colors',
                  auth.method === method
                    ? 'border-border-hover bg-surface'
                    : 'border-border hover:bg-surface-hover',
                )}
              >
                <input
                  type="radio"
                  name="auth-method"
                  checked={auth.method === method}
                  onChange={() => {
                    if (method === 'agent') {
                      setAuth({ method: 'agent' });
                    } else if (method === 'password') {
                      setAuth({ method: 'password', password: '' });
                    } else {
                      setAuth({ method: 'key', privateKey: '', passphrase: '' });
                    }
                  }}
                  className="accent-foreground"
                />
                <span className="text-ui-sm text-foreground select-none">{t(labelKey)}</span>
                {method === 'agent' ? (
                  <span className="text-ui-xs text-foreground-subtlest">{t('hosts.form.agentHint')}</span>
                ) : null}
              </label>
            ))}
          </div>
        </FormField>

        {auth.method === 'password' ? (
          <FormField label={t('hosts.form.password')}>
            <PasswordInput
              value={auth.method === 'password' ? auth.password : ''}
              onChange={(e) => setAuth({ method: 'password', password: e.target.value })}
              autoComplete="new-password"
            />
          </FormField>
        ) : null}

        {auth.method === 'key' ? (
          <>
            <FormField label={t('hosts.form.privateKey')}>
              <TextArea
                value={auth.method === 'key' ? auth.privateKey : ''}
                onChange={(e) =>
                  setAuth((current) =>
                    current.method === 'key' ? { ...current, privateKey: e.target.value } : current,
                  )
                }
                placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
                spellCheck={false}
              />
            </FormField>
            <FormField label={`${t('hosts.form.passphrase')}（${t('common.optional')}）`}>
              <PasswordInput
                value={auth.method === 'key' ? (auth.passphrase ?? '') : ''}
                onChange={(e) =>
                  setAuth((current) =>
                    current.method === 'key' ? { ...current, passphrase: e.target.value } : current,
                  )
                }
                autoComplete="new-password"
              />
            </FormField>
          </>
        ) : null}

        <FormField label={t('hosts.form.jumpHost')}>
          <div className="relative">
            <Select
              value={jumpHostId ?? ''}
              onChange={(e) => setJumpHostId(e.target.value ? Number(e.target.value) : null)}
            >
              <option value="">{t('hosts.form.jumpHostNone')}</option>
              {availableJumpHosts.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.label}
                </option>
              ))}
            </Select>
            <ChevronDown className="pointer-events-none absolute right-2 top-1/2 size-3.5 -translate-y-1/2 text-foreground-subtle" />
          </div>
        </FormField>

        <FormField label={t('hosts.form.hostKeyVerification')}>
          <div className="relative">
            <Select
              value={hostKeyVerification}
              onChange={(e) =>
                setHostKeyVerification(e.target.value as SshHostKeyVerificationMode)
              }
            >
              {HOST_KEY_POLICIES.map((policy) => (
                <option key={policy.mode} value={policy.mode}>
                  {t(policy.labelKey)}
                </option>
              ))}
            </Select>
            <ChevronDown className="pointer-events-none absolute right-2 top-1/2 size-3.5 -translate-y-1/2 text-foreground-subtle" />
          </div>
        </FormField>

        <label
          className={cn(
            'flex cursor-pointer items-start gap-2 rounded-lg border p-2.5 transition-colors',
            allowInsecureAlgos
              ? 'border-border-hover bg-surface'
              : 'border-border hover:bg-surface-hover',
          )}
        >
          <input
            type="checkbox"
            checked={allowInsecureAlgos}
            onChange={(e) => setAllowInsecureAlgos(e.target.checked)}
            className="mt-0.5 accent-foreground"
          />
          <div className="flex min-w-0 flex-1 flex-col gap-0.5 select-none">
            <span className="text-ui-sm text-foreground">{t('hosts.form.allowInsecureAlgos')}</span>
            <span className="text-ui-xs text-foreground-subtlest">
              {t('hosts.form.allowInsecureAlgosHint')}
            </span>
          </div>
        </label>

        <FormField label={t('hosts.form.groups')}>
          {groupOptions.length > 0 ? (
            <div className="flex max-h-36 flex-col gap-0.5 overflow-y-auto rounded-lg border border-border p-1.5">
              {groupOptions.map((option) => (
                <label
                  key={option.id}
                  className="flex cursor-pointer items-center gap-2 rounded-md px-1.5 py-1 transition-colors hover:bg-surface-hover"
                >
                  <input
                    type="checkbox"
                    checked={groupIds.includes(option.id)}
                    onChange={() => toggleGroup(option.id)}
                    className="accent-foreground"
                  />
                  <span className="truncate text-ui-sm text-foreground select-none">{option.path}</span>
                </label>
              ))}
            </div>
          ) : (
            <p className="text-ui-sm text-foreground-subtlest">{t('hosts.form.noGroups')}</p>
          )}
        </FormField>

        {error ? <p className="text-ui-sm text-destructive">{error}</p> : null}
      </div>
    </Modal>
  );
}
