import { Check, ChevronDown, Copy, Eye, EyeOff, History, Loader2, Plus, Trash2 } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { SelectMenu } from '@/components/ui/SelectMenu';
import { FormField, PasswordInput, TextArea, TextInput } from '@/components/hosts/formFields';
import { groupPathOptions, jumpHostOptions } from '@/hosts/storage';
import { writeClipboardText } from '@/lib/clipboard';
import {
  hostPasswordHistoryClear,
  hostPasswordHistoryList,
  type HostPasswordHistoryEntry,
} from '@/services/hostPasswordHistoryService';
import { relayList, type RelayConfig } from '@/services/relayConfigService';
import type {
  AuthConfig,
  AuthMethod,
  GroupNode,
  HostEnvironmentLevel,
  HostNode,
  PasswdPrompts,
  RelayTransport,
  SshHostKeyVerificationMode,
  SshRelayEndpoint,
} from '@/hosts/types';
import { commandStore, useCommands } from '@/services/commandStore';
import { cn } from '@/lib/utils';

export interface HostFormResult {
  name: string;
  host: string;
  port: number;
  username: string;
  auth: AuthConfig;
  jumpHostId: number | null;
  /** 中继库引用；选中下拉中的已保存中继时非空（嵌入式 relay 同时镜像其字段） */
  relayId: number | null;
  /** SSH 中继（veloterm_relay）；null = 不经中继 */
  relay: SshRelayEndpoint | null;
  hostKeyVerification: SshHostKeyVerificationMode;
  allowInsecureAlgos: boolean;
  /** 终端就绪钩子：会话建立后自动执行的命令库命令 id；null = 未启用 */
  readyCommandId: number | null;
  /** 环境级别：dev 开发 / test 测试 / staging 预发 / prod 生产 */
  envLevel: HostEnvironmentLevel;
  /** 是否开启危险命令拦截护盾 */
  safetyGuardEnabled: boolean;
  /** passwd 改密提示词配置（严格匹配用）；undefined = 未配置（用内置默认模式）。
 *  显式携带该键（而非省略）：编辑保存时 updateHost 的展开合并才能清掉旧值 */
  passwdPrompts: PasswdPrompts | undefined;
  /** 所属分组（可多选） */
  groupIds: number[];
}

interface HostFormDialogProps {
  /** 编辑时传入已有节点；新建为 null */
  node: HostNode | null;
  /** 复制模式：node 仅作为预填初值（全部字段照搬），保存由父层落库为新主机，
   *  标题与编辑区分，避免误以为是改原主机 */
  isDuplicate?: boolean;
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

const RELAY_TRANSPORTS: Array<{ transport: RelayTransport; labelKey: string }> = [
  { transport: 'tcp', labelKey: 'hosts.form.relayTransportTcp' },
  { transport: 'ws', labelKey: 'hosts.form.relayTransportWs' },
  { transport: 'quic', labelKey: 'hosts.form.relayTransportQuic' },
];

/** 密码历史默认打码占位（与密码输入框的视觉口径一致） */
const PASSWORD_MASK = '••••••••';

/** 本地时区格式化到秒（与命令历史 / SFTP 列表的时间口径一致） */
function formatReplacedAt(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  });
}

interface PasswordHistoryDialogProps {
  hostId: number;
  onClose: () => void;
}

/**
 * 密码历史弹层：**打开时才请求**（后端每主机最多 20 条）。
 * 每行可单独显隐/复制；清空走弹层内二次确认（不叠 ConfirmDialog，避免嵌套弹层
 * 的 Escape 语义纠缠）。
 */
function PasswordHistoryDialog({ hostId, onClose }: PasswordHistoryDialogProps) {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<HostPasswordHistoryEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [visibleIds, setVisibleIds] = useState<ReadonlySet<number>>(() => new Set());
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [clearing, setClearing] = useState(false);
  const copyTimerRef = useRef(0);

  const load = useCallback(() => {
    setEntries(null);
    setError(null);
    hostPasswordHistoryList(hostId)
      .then(setEntries)
      .catch((err) => setError(String(err)));
  }, [hostId]);

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => () => window.clearTimeout(copyTimerRef.current), []);

  // 本弹层嵌套在主机表单 Modal 内：Escape 必须只关自己。window 捕获阶段拦截并
  // stopPropagation，外层 Modal 的冒泡监听收不到（否则会连主机表单一起关掉、
  // 丢掉未保存的编辑）；自身关闭也在这里处理，不依赖被拦截的冒泡监听。
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.stopPropagation();
      if (confirmClear) {
        setConfirmClear(false);
      } else {
        onClose();
      }
    };
    window.addEventListener('keydown', onKeyDown, true);
    return () => window.removeEventListener('keydown', onKeyDown, true);
  }, [confirmClear, onClose]);

  const toggleVisible = (id: number) => {
    setVisibleIds((current) => {
      const next = new Set(current);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  const handleCopy = async (entry: HostPasswordHistoryEntry) => {
    try {
      await writeClipboardText(entry.password);
      setCopiedId(entry.id);
      window.clearTimeout(copyTimerRef.current);
      copyTimerRef.current = window.setTimeout(() => setCopiedId(null), 1500);
    } catch {
      // 剪贴板不可用时静默（与终端 / 命令历史的复制路径一致）
    }
  };

  const handleClear = async () => {
    setClearing(true);
    try {
      await hostPasswordHistoryClear(hostId);
      setEntries([]);
      setError(null);
      setConfirmClear(false);
    } catch (err) {
      setError(`${t('hostPasswordHistory.clearFailed')}: ${String(err)}`);
      setConfirmClear(false);
    } finally {
      setClearing(false);
    }
  };

  return (
    <Modal
      title={t('hostPasswordHistory.title')}
      onClose={onClose}
      className="w-[560px]"
      footer={
        <>
          <Button
            variant="ghost"
            size="lg"
            className="mr-auto h-8 gap-1.5 px-3 text-ui-base text-destructive hover:bg-destructive/10"
            onClick={() => setConfirmClear(true)}
            disabled={confirmClear || clearing || entries == null || entries.length === 0}
          >
            <Trash2 className="size-4" />
            <span>{t('hostPasswordHistory.clear')}</span>
          </Button>
          <Button variant="outline" size="lg" className="h-8 px-3 text-ui-base" onClick={onClose}>
            {t('common.close')}
          </Button>
        </>
      }
    >
      {confirmClear ? (
        <div className="flex flex-col gap-3">
          <p className="text-ui-sm text-foreground">{t('hostPasswordHistory.clearConfirm')}</p>
          <div className="flex justify-end gap-2">
            <Button
              variant="outline"
              size="lg"
              className="h-8 px-3 text-ui-base"
              onClick={() => setConfirmClear(false)}
              disabled={clearing}
            >
              {t('common.cancel')}
            </Button>
            <Button
              variant="destructive"
              size="lg"
              className="h-8 gap-1.5 px-3 text-ui-base"
              onClick={() => void handleClear()}
              disabled={clearing}
            >
              {clearing ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <Trash2 className="size-4" />
              )}
              <span>{t('hostPasswordHistory.clear')}</span>
            </Button>
          </div>
        </div>
      ) : error != null && entries == null ? (
        <div className="flex flex-col items-center gap-2 py-8 text-center">
          <p className="text-ui-sm text-destructive">{t('hostPasswordHistory.loadFailed')}</p>
          <p className="max-w-full break-all font-mono text-ui-xs text-foreground-subtlest">
            {error}
          </p>
          <Button
            variant="outline"
            size="lg"
            className="mt-1 h-8 px-3 text-ui-base"
            onClick={load}
          >
            {t('hostPasswordHistory.retry')}
          </Button>
        </div>
      ) : entries == null ? (
        <div className="flex items-center justify-center gap-2 py-10 text-ui-sm text-foreground-subtle select-none">
          <Loader2 className="size-4 animate-spin" />
          <span>{t('hostPasswordHistory.loading')}</span>
        </div>
      ) : entries.length === 0 ? (
        <p className="py-10 text-center text-ui-sm text-foreground-subtle">
          {t('hostPasswordHistory.empty')}
        </p>
      ) : (
        <div className="flex flex-col gap-1.5">
          {error ? (
            <p className="break-all rounded-md border border-destructive/20 bg-destructive/10 px-2 py-1 text-ui-xs text-destructive">
              {error}
            </p>
          ) : null}
          <p className="pb-0.5 text-ui-xs text-foreground-subtlest select-none">
            {t('hostPasswordHistory.hint')}
          </p>
          {entries.map((entry) => {
            const visible = visibleIds.has(entry.id);
            const copied = copiedId === entry.id;
            const sourceLabel =
              entry.source === 'kbd-int-change'
                ? t('hostPasswordHistory.sourceAuto')
                : entry.source === 'user-input'
                  ? t('hostPasswordHistory.sourceUserInput')
                  : entry.source;
            return (
              <div
                key={entry.id}
                className="flex items-center gap-2 rounded-lg border border-border/60 bg-surface/40 px-2.5 py-1.5"
              >
                <span className="w-40 shrink-0 font-mono text-ui-xs text-foreground-subtle">
                  {formatReplacedAt(entry.replacedAt)}
                </span>
                <span
                  className="min-w-0 flex-1 truncate font-mono text-ui-sm text-foreground"
                  title={visible ? entry.password : undefined}
                >
                  {visible ? entry.password : PASSWORD_MASK}
                </span>
                <span className="shrink-0 rounded border border-border/60 bg-surface px-1.5 py-0.5 text-ui-xs text-foreground-subtle select-none">
                  {sourceLabel}
                </span>
                <ControlHintTooltip
                  title={visible ? t('common.hidePassword') : t('common.showPassword')}
                >
                  <button
                    type="button"
                    aria-label={visible ? t('common.hidePassword') : t('common.showPassword')}
                    onClick={() => toggleVisible(entry.id)}
                    className="inline-flex size-6 items-center justify-center rounded-md text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
                  >
                    {visible ? <EyeOff className="size-3.5" /> : <Eye className="size-3.5" />}
                  </button>
                </ControlHintTooltip>
                <ControlHintTooltip
                  title={copied ? t('hostPasswordHistory.copied') : t('hostPasswordHistory.copy')}
                >
                  <button
                    type="button"
                    aria-label={t('hostPasswordHistory.copy')}
                    onClick={() => void handleCopy(entry)}
                    className="inline-flex size-6 items-center justify-center rounded-md text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
                  >
                    {copied ? <Check className="size-3.5 text-primary" /> : <Copy className="size-3.5" />}
                  </button>
                </ControlHintTooltip>
              </div>
            );
          })}
        </div>
      )}
    </Modal>
  );
}

/** 主机新建/编辑对话框：主机名 + 地址/端口/用户名 + 三种认证方式 + 跳板机/公钥策略/旧版算法兼容 + 所属分组（多选）。 */
export function HostFormDialog({ node, isDuplicate = false, hosts, groups, onSave, onClose }: HostFormDialogProps) {
  const { t } = useTranslation();
  const [name, setName] = useState(node?.name ?? '');
  const [host, setHost] = useState(node?.host ?? '');
  const [port, setPort] = useState(String(node?.port ?? 22));
  const [username, setUsername] = useState(node?.username ?? '');
  const [auth, setAuth] = useState<AuthConfig>(() => node?.auth ?? { method: 'password', password: '' });
  const [jumpHostId, setJumpHostId] = useState<number | null>(node?.jumpHostId ?? null);
  // SSH 中继：启用后首跳经 veloterm_relay 建立（与跳板可组合：中继送达链首端点）
  const [relayEnabled, setRelayEnabled] = useState<boolean>(node?.relay != null || node?.relayId != null);
  const [relayId, setRelayId] = useState<number | null>(node?.relayId ?? null);
  const [relayConfigs, setRelayConfigs] = useState<RelayConfig[]>([]);
  const [relay, setRelay] = useState<SshRelayEndpoint>(
    node?.relay ?? {
      host: '',
      port: 7300,
      transport: 'tcp',
      keyId: 1,
      secret: '',
      certFingerprint: null,
    },
  );

  // 中继库（侧栏「中继」页）加载一次，供下拉选择
  useEffect(() => {
    void relayList().then(setRelayConfigs).catch(() => {
      // 中继库不可用时退回手动输入
    });
  }, []);

  const applyRelayConfig = (config: RelayConfig) => {
    setRelayId(config.id);
    setRelay({
      host: config.host,
      port: config.port,
      transport: config.transport,
      keyId: config.keyId,
      secret: config.secret,
      certFingerprint: config.certFingerprint ?? null,
    });
  };

  const clearRelayId = () => setRelayId(null);
  const [hostKeyVerification, setHostKeyVerification] = useState<SshHostKeyVerificationMode>(
    node?.hostKeyVerification ?? 'auto_accept',
  );
  const [allowInsecureAlgos, setAllowInsecureAlgos] = useState<boolean>(
    node?.allowInsecureAlgos ?? false,
  );
  const [groupIds, setGroupIds] = useState<number[]>(node?.groupIds ?? []);
  const [envLevel, setEnvLevel] = useState<HostEnvironmentLevel>(node?.envLevel ?? 'dev');
  const [safetyGuardEnabled, setSafetyGuardEnabled] = useState<boolean>(
    node?.safetyGuardEnabled ?? true,
  );
  const [error, setError] = useState<string | null>(null);
  // 密码历史弹层：仅编辑已存在主机时可用（新主机还没有可查的 host id）
  const [historyOpen, setHistoryOpen] = useState(false);

  // passwd 改密提示词：三条独立编辑（后端判完整性，任一为空即回退内置默认模式）。
  // 默认收起：属低频高级配置，避免表单过长；仅密码认证方式下显示
  const [passwdPromptsOpen, setPasswdPromptsOpen] = useState(false);
  const [passwdPromptCurrent, setPasswdPromptCurrent] = useState(
    node?.passwdPrompts?.current ?? '',
  );
  const [passwdPromptNew, setPasswdPromptNew] = useState(node?.passwdPrompts?.new ?? '');
  const [passwdPromptRetype, setPasswdPromptRetype] = useState(
    node?.passwdPrompts?.retype ?? '',
  );

  // 终端就绪钩子：引用命令库主键 id（可下拉选已有，也可内联新建保存后选用）
  const [readyCommandId, setReadyCommandId] = useState<number | null>(
    node?.readyCommandId ?? null,
  );
  const commands = useCommands();
  const [creatingCommand, setCreatingCommand] = useState(false);
  const [newCommandTitle, setNewCommandTitle] = useState('');
  const [newCommandText, setNewCommandText] = useState('');
  const [commandError, setCommandError] = useState<string | null>(null);

  // 命令库可能尚未加载（首次打开主机表单前未进过命令页）：拉一次全量
  useEffect(() => {
    if (!commandStore.isLoaded()) {
      void commandStore.refresh();
    }
  }, []);

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
    // 过滤掉已被删除的分组与跳板引用；就绪钩子同理只保留仍存在的命令 id
    const validGroupIds = groupIds.filter((id) => groups.some((group) => group.id === id));
    const validJumpHostId =
      jumpHostId != null && availableJumpHosts.some((item) => item.id === jumpHostId)
        ? jumpHostId
        : null;
    const validReadyCommandId =
      readyCommandId != null && commands.some((cmd) => cmd.id === readyCommandId)
        ? readyCommandId
        : null;
    // 中继字段校验：启用时必填项齐全，secret 必须 64 位十六进制，keyId 0-255
    let relayResult: SshRelayEndpoint | null = null;
    if (relayEnabled) {
      const relayPortNumber = Number(relay.port);
      const relayKeyIdNumber = Number(relay.keyId);
      if (!relay.host.trim()) {
        setError(t('hosts.form.relayHostRequired'));
        return;
      }
      if (!Number.isInteger(relayPortNumber) || relayPortNumber < 1 || relayPortNumber > 65535) {
        setError(t('hosts.form.relayPortInvalid'));
        return;
      }
      if (!Number.isInteger(relayKeyIdNumber) || relayKeyIdNumber < 0 || relayKeyIdNumber > 255) {
        setError(t('hosts.form.relayKeyIdInvalid'));
        return;
      }
      if (!/^[0-9a-fA-F]{64}$/.test(relay.secret.trim())) {
        setError(t('hosts.form.relaySecretInvalid'));
        return;
      }
      const fingerprintRaw = relay.certFingerprint?.trim() ?? '';
      if (fingerprintRaw !== '') {
        const normalized = fingerprintRaw.replace(/:/g, '');
        if (!/^[0-9a-fA-F]{64}$/.test(normalized)) {
          setError(t('hosts.form.relayFingerprintInvalid'));
          return;
        }
      }
      relayResult = {
        host: relay.host.trim(),
        port: relayPortNumber,
        transport: relay.transport,
        keyId: relayKeyIdNumber,
        secret: relay.secret.trim(),
        certFingerprint: fingerprintRaw || null,
      };
    }
    // 三条都为空 → 未配置（undefined）；任一非空 → 传完整对象，由后端判断完整性
    const promptValues = [
      passwdPromptCurrent.trim(),
      passwdPromptNew.trim(),
      passwdPromptRetype.trim(),
    ] as const;
    const passwdPrompts: PasswdPrompts | undefined = promptValues.some((value) => value !== '')
      ? {
          current: promptValues[0],
          new: promptValues[1],
          retype: promptValues[2],
        }
      : undefined;
      onSave({
        name: name.trim(),
        host: host.trim(),
        port: portNumber,
        username: username.trim(),
        auth,
        jumpHostId: validJumpHostId,
        relayId: relayResult != null && relayId != null ? relayId : null,
        relay: relayResult,
        hostKeyVerification,
        allowInsecureAlgos,
        readyCommandId: validReadyCommandId,
        envLevel,
        safetyGuardEnabled,
        passwdPrompts,
        groupIds: validGroupIds,
      });
  };

  /** 内联新建命令：存入命令库（id=0 由后端分配）后立即选为就绪钩子 */
  const handleCreateCommand = async () => {
    if (!newCommandTitle.trim() || !newCommandText.trim()) {
      setCommandError(t('hosts.form.readyHookRequired'));
      return;
    }
    try {
      const saved = await commandStore.save({
        id: 0,
        title: newCommandTitle.trim(),
        command: newCommandText.trim(),
        description: '',
        purpose: '',
        category: '',
        tags: [],
        platforms: [],
        params: [],
        dangerLevel: 'safe',
        exampleOutput: '',
        usageCount: 0,
        lastUsedAt: null,
        isPinned: false,
        createdAt: 0,
        updatedAt: 0,
      });
      setReadyCommandId(saved.id);
      setCreatingCommand(false);
      setNewCommandTitle('');
      setNewCommandText('');
      setCommandError(null);
    } catch (err) {
      setCommandError(String(err));
    }
  };

  return (
    <Modal
      title={isDuplicate ? t('hosts.form.duplicateTitle') : node ? t('hosts.form.editTitle') : t('hosts.form.newTitle')}
      onClose={onClose}
      // 双栏并排：默认 420px 单栏会让字段纵向堆得过长，加宽到 760px 分左右两列
      className="w-[760px]"
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
        <div className="flex items-start gap-5">
          {/* 左列：连接信息（名称 / 地址端口 / 用户名 / 认证凭据） */}
          <div className="flex min-w-0 flex-1 flex-col gap-3.5">
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
              <TextInput
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                placeholder="root"
              />
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
                      <span className="text-ui-xs text-foreground-subtlest">
                        {t('hosts.form.agentHint')}
                      </span>
                    ) : null}
                  </label>
                ))}
              </div>
            </FormField>

            {auth.method === 'password' ? (
              <>
                <FormField label={t('hosts.form.password')}>
                  <PasswordInput
                    value={auth.method === 'password' ? auth.password : ''}
                    onChange={(e) => setAuth({ method: 'password', password: e.target.value })}
                    autoComplete="new-password"
                  />
                  {node != null ? (
                    <button
                      type="button"
                      onClick={() => setHistoryOpen(true)}
                      className="inline-flex items-center gap-1 self-start rounded text-ui-xs text-foreground-subtle transition-colors hover:text-foreground"
                    >
                      <History className="size-3" />
                      <span>{t('hostPasswordHistory.entry')}</span>
                    </button>
                  ) : null}
                </FormField>

                {/* 改密提示词配置：可折叠（默认收起），仅密码认证相关 */}
                <div className="rounded-lg border border-border bg-surface/30">
                  <button
                    type="button"
                    onClick={() => setPasswdPromptsOpen((value) => !value)}
                    aria-expanded={passwdPromptsOpen}
                    className="flex w-full cursor-pointer items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-left transition-colors hover:bg-surface-hover"
                  >
                    <ChevronDown
                      className={cn(
                        'size-3.5 shrink-0 text-foreground-subtle transition-transform',
                        !passwdPromptsOpen && '-rotate-90',
                      )}
                    />
                    <span className="text-ui-sm text-foreground-subtle select-none">
                      {t('passwdPrompts.sectionTitle')}
                    </span>
                  </button>
                  {passwdPromptsOpen ? (
                    <div className="flex flex-col gap-3 border-t border-border/50 px-2.5 py-2.5">
                      <p className="text-ui-xs text-foreground-subtlest select-none">
                        {t('passwdPrompts.hint')}
                      </p>
                      <FormField label={t('passwdPrompts.current')} hint={t('passwdPrompts.perLine')}>
                        <TextArea
                          value={passwdPromptCurrent}
                          onChange={(e) => setPasswdPromptCurrent(e.target.value)}
                          spellCheck={false}
                          className="font-mono"
                          rows={2}
                        />
                      </FormField>
                      <FormField label={t('passwdPrompts.new')} hint={t('passwdPrompts.perLine')}>
                        <TextArea
                          value={passwdPromptNew}
                          onChange={(e) => setPasswdPromptNew(e.target.value)}
                          spellCheck={false}
                          className="font-mono"
                          rows={2}
                        />
                      </FormField>
                      <FormField label={t('passwdPrompts.retype')} hint={t('passwdPrompts.perLine')}>
                        <TextArea
                          value={passwdPromptRetype}
                          onChange={(e) => setPasswdPromptRetype(e.target.value)}
                          spellCheck={false}
                          className="font-mono"
                          rows={2}
                        />
                      </FormField>
                    </div>
                  ) : null}
                </div>
              </>
            ) : null}

            {auth.method === 'key' ? (
              <>
                <FormField label={t('hosts.form.privateKey')}>
                  <TextArea
                    value={auth.method === 'key' ? auth.privateKey : ''}
                    onChange={(e) =>
                      setAuth((current) =>
                        current.method === 'key'
                          ? { ...current, privateKey: e.target.value }
                          : current,
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
                        current.method === 'key'
                          ? { ...current, passphrase: e.target.value }
                          : current,
                      )
                    }
                    autoComplete="new-password"
                  />
                </FormField>
              </>
            ) : null}
          </div>

          {/* 右列：高级选项（跳板机 / 公钥策略 / 旧版算法 / 就绪钩子 / 所属分组） */}
          <div className="flex min-w-0 flex-1 flex-col gap-3.5">
            <FormField label={t('hosts.form.jumpHost')}>
              <SelectMenu
                aria-label={t('hosts.form.jumpHost')}
                value={jumpHostId != null ? String(jumpHostId) : ''}
                onChange={(next) => setJumpHostId(next === '' ? null : Number(next))}
                options={[
                  { value: '', label: t('hosts.form.jumpHostNone') },
                  ...availableJumpHosts.map((item) => ({
                    value: String(item.id),
                    label: item.label,
                  })),
                ]}
              />
            </FormField>

            <FormField label={t('hosts.form.hostKeyVerification')}>
              <SelectMenu
                aria-label={t('hosts.form.hostKeyVerification')}
                value={hostKeyVerification}
                onChange={(next) => setHostKeyVerification(next as SshHostKeyVerificationMode)}
                options={HOST_KEY_POLICIES.map((policy) => ({
                  value: policy.mode,
                  label: t(policy.labelKey),
                }))}
              />
            </FormField>

            {/* SSH 中继（veloterm_relay）：启用后首跳经中继建立 VLR2 流再跑 SSH */}
            <label
              className={cn(
                'flex cursor-pointer items-start gap-2 rounded-lg border p-2.5 transition-colors',
                relayEnabled
                  ? 'border-border-hover bg-surface'
                  : 'border-border hover:bg-surface-hover',
              )}
            >
              <input
                type="checkbox"
                checked={relayEnabled}
                onChange={(e) => setRelayEnabled(e.target.checked)}
                className="mt-0.5 accent-foreground"
              />
              <div className="flex min-w-0 flex-1 flex-col gap-0.5 select-none">
                <span className="text-ui-sm text-foreground">
                  {t('hosts.form.relayEnable')}
                </span>
                <span className="text-ui-xs text-foreground-subtlest">
                  {t('hosts.form.relayHint')}
                </span>
              </div>
            </label>

            {relayEnabled ? (
              <div className="flex flex-col gap-3.5 rounded-lg border border-border p-2.5">
                {/* 中继库下拉：选中即回填字段并记 relayId（连接时嵌入式优先，库配置更新后需重新选择同步） */}
                {relayConfigs.length > 0 ? (
                  <FormField label={t('hosts.form.relayPicker')}>
                    <SelectMenu
                      aria-label={t('hosts.form.relayPicker')}
                      value={relayId != null ? String(relayId) : ''}
                      onChange={(next) => {
                        if (next === '') {
                          clearRelayId();
                          return;
                        }
                        const config = relayConfigs.find((r) => String(r.id) === next);
                        if (config) applyRelayConfig(config);
                      }}
                      options={[
                        { value: '', label: t('hosts.form.relayManual') },
                        ...relayConfigs.map((r) => ({
                          value: String(r.id),
                          label: r.name,
                          description: `${r.host}:${r.port} · ${r.transport.toUpperCase()}`,
                        })),
                      ]}
                    />
                  </FormField>
                ) : null}
                <div className="flex gap-3">
                  <div className="min-w-0 flex-1">
                    <FormField label={t('hosts.form.relayHost')}>
                      <TextInput
                        value={relay.host}
                        onChange={(e) => setRelay((cur) => ({ ...cur, host: e.target.value }))}
                        placeholder="relay.example.com"
                      />
                    </FormField>
                  </div>
                  <div className="w-24 shrink-0">
                    <FormField label={t('hosts.form.relayPort')}>
                      <TextInput
                        type="number"
                        value={String(relay.port)}
                        onChange={(e) =>
                          setRelay((cur) => ({ ...cur, port: Number(e.target.value) }))
                        }
                        min={1}
                        max={65535}
                      />
                    </FormField>
                  </div>
                </div>

                <div className="flex gap-3">
                  <div className="w-32 shrink-0">
                    <FormField label={t('hosts.form.relayTransport')}>
                      <SelectMenu
                        aria-label={t('hosts.form.relayTransport')}
                        value={relay.transport}
                        onChange={(next) =>
                          setRelay((cur) => ({ ...cur, transport: next as RelayTransport }))
                        }
                        options={RELAY_TRANSPORTS.map((item) => ({
                          value: item.transport,
                          label: t(item.labelKey),
                        }))}
                      />
                    </FormField>
                  </div>
                  <div className="w-24 shrink-0">
                    <FormField label={t('hosts.form.relayKeyId')}>
                      <TextInput
                        type="number"
                        value={String(relay.keyId)}
                        onChange={(e) =>
                          setRelay((cur) => ({ ...cur, keyId: Number(e.target.value) }))
                        }
                        min={0}
                        max={255}
                      />
                    </FormField>
                  </div>
                  <div className="min-w-0 flex-1">
                    <FormField label={t('hosts.form.relaySecret')}>
                      <PasswordInput
                        value={relay.secret}
                        onChange={(e) => setRelay((cur) => ({ ...cur, secret: e.target.value }))}
                        autoComplete="off"
                      />
                    </FormField>
                  </div>
                </div>

                <FormField label={`${t('hosts.form.relayFingerprint')}（${t('common.optional')}）`}>
                  <TextInput
                    value={relay.certFingerprint ?? ''}
                    onChange={(e) =>
                      setRelay((cur) => ({ ...cur, certFingerprint: e.target.value }))
                    }
                    placeholder="AB:CD:EF:…"
                    spellCheck={false}
                    className="font-mono"
                  />
                </FormField>
                <p className="text-ui-xs text-foreground-subtlest select-none">
                  {t('hosts.form.relayFingerprintHint')}
                </p>

              </div>
            ) : null}

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
                <span className="text-ui-sm text-foreground">
                  {t('hosts.form.allowInsecureAlgos')}
                </span>
                <span className="text-ui-xs text-foreground-subtlest">
                  {t('hosts.form.allowInsecureAlgosHint')}
                </span>
              </div>
            </label>

            <FormField label={t('hosts.form.readyHook')}>
              <div className="flex flex-col gap-1.5">
                {/* 新建命令期间隐藏下拉与预览：避免「选已有」与「建新的」两个入口并存造成歧义 */}
                {!creatingCommand ? (
                  <>
                    <SelectMenu
                      aria-label={t('hosts.form.readyHook')}
                      searchable
                      searchPlaceholder={t('hosts.form.readyHookSearch')}
                      emptyLabel={t('hosts.form.readyHookNoMatch')}
                      value={readyCommandId != null ? String(readyCommandId) : ''}
                      onChange={(next) => setReadyCommandId(next === '' ? null : Number(next))}
                      options={[
                        { value: '', label: t('hosts.form.readyHookNone') },
                        ...commands.map((cmd) => ({
                          value: String(cmd.id),
                          label: cmd.title,
                          description: cmd.command,
                        })),
                      ]}
                    />
                    {readyCommandId != null ? (
                      <p className="truncate font-mono text-ui-sm text-foreground-subtlest select-none">
                        {commands.find((cmd) => cmd.id === readyCommandId)?.command ??
                          t('hosts.form.readyHookMissing')}
                      </p>
                    ) : (
                      <p className="text-ui-sm text-foreground-subtlest select-none">
                        {t('hosts.form.readyHookHint')}
                      </p>
                    )}
                  </>
                ) : null}
                {creatingCommand ? (
                  <div className="flex flex-col gap-2 rounded-lg border border-border p-2.5">
                    <TextInput
                      value={newCommandTitle}
                      onChange={(e) => setNewCommandTitle(e.target.value)}
                      placeholder={t('hosts.form.readyHookNewTitle')}
                    />
                    <TextInput
                      value={newCommandText}
                      onChange={(e) => setNewCommandText(e.target.value)}
                      placeholder={t('hosts.form.readyHookNewCommand')}
                      spellCheck={false}
                      className="font-mono"
                    />
                    <div className="flex items-center justify-between gap-2">
                      {commandError ? (
                        <p className="min-w-0 truncate text-ui-sm text-destructive">{commandError}</p>
                      ) : (
                        <span />
                      )}
                      <div className="flex shrink-0 gap-1.5">
                        <Button
                          variant="ghost"
                          size="lg"
                          className="h-8 px-3 text-ui-base"
                          onClick={() => {
                            setCreatingCommand(false);
                            setCommandError(null);
                          }}
                        >
                          {t('hosts.form.cancel')}
                        </Button>
                        <Button
                          size="lg"
                          className="h-8 px-3 text-ui-base"
                          onClick={() => void handleCreateCommand()}
                        >
                          {t('hosts.form.readyHookSaveAndUse')}
                        </Button>
                      </div>
                    </div>
                  </div>
                ) : (
                  <Button
                    variant="ghost"
                    size="lg"
                    className="h-8 self-start px-2.5 text-ui-base"
                    onClick={() => setCreatingCommand(true)}
                  >
                    <Plus className="size-4" />
                    <span>{t('hosts.form.readyHookCreateNew')}</span>
                  </Button>
                )}
              </div>
            </FormField>

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
                      <span className="truncate text-ui-sm text-foreground select-none">
                        {option.path}
                      </span>
                    </label>
                  ))}
                </div>
              ) : (
                <p className="text-ui-sm text-foreground-subtlest">{t('hosts.form.noGroups')}</p>
              )}
            </FormField>
            <FormField label={t('hosts.form.envLevel', '环境级别')}>
              <SelectMenu
                aria-label={t('hosts.form.envLevel', '环境级别')}
                value={envLevel}
                onChange={(next) => setEnvLevel(next as HostEnvironmentLevel)}
                options={[
                  {
                    value: 'dev',
                    label: t('hosts.form.envDev', '开发环境 (Dev)'),
                    description: t('hosts.form.envDevDesc', '普通开发调试机器，安全策略最宽松'),
                  },
                  {
                    value: 'test',
                    label: t('hosts.form.envTest', '测试环境 (Test)'),
                    description: t('hosts.form.envTestDesc', '集成测试机，不阻断常规命令'),
                  },
                  {
                    value: 'staging',
                    label: t('hosts.form.envStaging', '预发环境 (Staging)'),
                    description: t('hosts.form.envStagingDesc', '准生产环境，启用橙色警示与高危阻断'),
                  },
                  {
                    value: 'prod',
                    label: t('hosts.form.envProd', '生产环境 (Prod) 🔥'),
                    description: t('hosts.form.envProdDesc', '线上真实服务器，红边框强提醒与高危命令强阻断'),
                  },
                ]}
              />
            </FormField>

            <label className="flex cursor-pointer items-start gap-2.5 rounded-lg border border-border/80 bg-surface/30 p-2.5 transition-colors hover:bg-surface/60">
              <input
                type="checkbox"
                checked={safetyGuardEnabled}
                onChange={(e) => setSafetyGuardEnabled(e.target.checked)}
                className="mt-0.5 accent-red-500"
              />
              <div className="flex min-w-0 flex-1 flex-col gap-0.5 select-none">
                <span className="text-ui-sm font-medium text-foreground">
                  {t('hosts.form.safetyGuardEnable', '启用危险命令拦截护盾')}
                </span>
                <span className="text-ui-xs text-foreground-subtlest">
                  {t(
                    'hosts.form.safetyGuardHint',
                    '在预发/生产环境下敲入或粘贴 rm -rf /、关机重启、删库等高危操作时强阻断并弹窗二次确认',
                  )}
                </span>
              </div>
            </label>
          </div>
        </div>

        {error ? <p className="text-ui-sm text-destructive">{error}</p> : null}

        {/* 密码历史弹层：portal 到 body，避开表单面板的 overflow 裁剪 */}
        {historyOpen && node != null
          ? createPortal(
              <PasswordHistoryDialog hostId={node.id} onClose={() => setHistoryOpen(false)} />,
              document.body,
            )
          : null}
      </div>
    </Modal>
  );
}
