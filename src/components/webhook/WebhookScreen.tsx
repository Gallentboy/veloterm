import {
  AlertCircle,
  ArrowDown,
  ArrowUp,
  Check,
  ChevronRight,
  ChevronsUpDown,
  Clock,
  Copy,
  Layers,
  Lock,
  Play,
  Plus,
  Power,
  RefreshCw,
  Search,
  Server,
  Settings,
  Shield,
  Terminal,
  Trash2,
  Webhook,
} from 'lucide-react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/ConfirmDialog';
import { SelectMenu, type SelectMenuOption } from '@/components/ui/SelectMenu';
import { Spinner } from '@/components/ui/spinner';
import { cn } from '@/lib/utils';
import {
  webhookGetSettings,
  webhookGetStatus,
  webhookLogReplay,
  webhookLogsClear,
  webhookLogsList,
  webhookRuleDelete,
  webhookRuleList,
  webhookRuleSave,
  webhookRuleTest,
  webhookRuleToggle,
  webhookUpdateSettings,
  type WebhookActionStep,
  type WebhookActionType,
  type WebhookLog,
  type WebhookMethod,
  type WebhookRule,
  type WebhookSettings,
  type WebhookSignatureType,
  type WebhookStatus,
  type WebhookTestResult,
} from '@/services/webhookService';
import { WebhookSettingsModal } from '@/components/webhook/WebhookSettingsModal';
import { WebhookScriptEditor } from '@/components/webhook/WebhookScriptEditor';
import { WebhookCommandEditor } from '@/components/webhook/WebhookCommandEditor';
import { WebhookReadOnlyEditor } from '@/components/webhook/WebhookReadOnlyEditor';
import { loadHostStore } from '@/hosts/storage';
import type { HostNode } from '@/hosts/types';

const METHOD_COLORS: Record<string, string> = {
  GET: 'text-terminal-blue bg-terminal-blue/10 border-terminal-blue/20',
  POST: 'text-terminal-green bg-terminal-green/10 border-terminal-green/20',
  PUT: 'text-terminal-yellow bg-terminal-yellow/10 border-terminal-yellow/20',
  DELETE: 'text-destructive bg-destructive/10 border-destructive/20',
  PATCH: 'text-purple-400 bg-purple-400/10 border-purple-400/20',
  ANY: 'text-foreground-subtle bg-surface border-border',
};

/** 解析或回退提取规则中的全部流水线步骤 */
function extractRuleSteps(r: WebhookRule, defaultStepPrefix = 'Step'): WebhookActionStep[] {
  if (r.actions && r.actions.length > 0) return r.actions;
  if (r.actionsJson) {
    try {
      const parsed = JSON.parse(r.actionsJson);
      if (Array.isArray(parsed) && parsed.length > 0) {
        return parsed;
      }
    } catch {}
  }
  return [
    {
      id: `step_${Date.now()}_1`,
      name: `${defaultStepPrefix} 1`,
      actionType: r.actionType || 'local_command',
      targetHostId: r.targetHostId,
      commandText: r.commandText || '',
      cwd: r.cwd,
      timeoutSecs: r.timeoutSecs || 60,
      continueOnError: false,
    },
  ];
}

function formatTime(ms: number): string {
  if (!ms) return '-';
  const d = new Date(ms);
  return `${d.getMonth() + 1}-${d.getDate()} ${d.getHours().toString().padStart(2, '0')}:${d.getMinutes().toString().padStart(2, '0')}:${d.getSeconds().toString().padStart(2, '0')}`;
}

/** 语义化 Switch 开关组件（彻底禁用原生 input checkbox，保障跨平台风格绝对一致） */
function Switch({
  checked,
  disabled,
  onChange,
  className,
  'aria-label': ariaLabel,
}: {
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
  className?: string;
  'aria-label'?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation();
        onChange(!checked);
      }}
      className={cn(
        'relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border transition-colors focus:outline-none focus:ring-1 focus:ring-primary/40',
        checked ? 'border-transparent bg-primary' : 'border-border bg-input',
        disabled && 'cursor-not-allowed opacity-50',
        className,
      )}
    >
      <span
        className={cn(
          'inline-block size-4 rounded-full shadow-sm transition-transform duration-150',
          checked ? 'translate-x-4.5 bg-primary-foreground' : 'translate-x-0.5 bg-foreground-subtle',
        )}
      />
    </button>
  );
}

export function WebhookScreen() {
  const { t } = useTranslation();

  const [settings, setSettings] = useState<WebhookSettings | null>(null);
  const [status, setStatus] = useState<WebhookStatus | null>(null);
  const [rules, setRules] = useState<WebhookRule[]>([]);
  const [hosts, setHosts] = useState<HostNode[]>([]);
  const [loading, setLoading] = useState(true);

  // 视图 Tab：'rules' | 'logs'
  const [activeTab, setActiveTab] = useState<'rules' | 'logs'>('rules');
  const [settingsModalOpen, setSettingsModalOpen] = useState(false);

  // 选中的规则
  const [selectedRuleId, setSelectedRuleId] = useState<string | null>(null);
  const [editingRule, setEditingRule] = useState<WebhookRule | null>(null);
  const [ruleSearch, setRuleSearch] = useState('');
  const [savingRule, setSavingRule] = useState(false);
  const [copiedId, setCopiedId] = useState<string | null>(null);

  // 流水线步骤折叠状态管理（stepKey -> isCollapsed）
  const [collapsedStepKeys, setCollapsedStepKeys] = useState<Record<string, boolean>>({});

  // 删除确认对话框（替代 window.confirm）
  const [deletingRule, setDeletingRule] = useState<WebhookRule | null>(null);
  const [clearingLogsConfirm, setClearingLogsConfirm] = useState(false);
  const [clearingLogsBusy, setClearingLogsBusy] = useState(false);

  // 测试运行面板状态
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<WebhookTestResult | null>(null);
  const [testPayload] = useState('{\n  "ref": "refs/heads/main",\n  "event": "push"\n}');

  // 审计日志列表
  const [logs, setLogs] = useState<WebhookLog[]>([]);
  const [totalLogs, setTotalLogs] = useState(0);
  const [logsLoading, setLogsLoading] = useState(false);
  const [replayingId, setReplayingId] = useState<string | null>(null);
  const [selectedLogId, setSelectedLogId] = useState<string | null>(null);

  // 刷新状态与列表
  const refreshData = useCallback(async () => {
    try {
      const [s, st, r] = await Promise.all([
        webhookGetSettings(),
        webhookGetStatus(),
        webhookRuleList(),
      ]);
      const hStore = loadHostStore();
      setSettings(s);
      setStatus(st);
      setRules(r);
      setHosts(hStore.hosts || []);

      if (r.length > 0 && !selectedRuleId) {
        setSelectedRuleId(r[0].id);
        setEditingRule({ ...r[0] });
      }
    } catch (err) {
      console.error('加载 WebHook 数据失败:', err);
    } finally {
      setLoading(false);
    }
  }, [selectedRuleId]);

  const refreshLogs = useCallback(async () => {
    setLogsLoading(true);
    try {
      const resp = await webhookLogsList(50, 0);
      setLogs(resp.items);
      setTotalLogs(resp.total);
    } catch (err) {
      console.error('加载 WebHook 审计日志失败:', err);
    } finally {
      setLogsLoading(false);
    }
  }, []);

  useEffect(() => {
    void refreshData();
  }, [refreshData]);

  useEffect(() => {
    if (activeTab === 'logs') {
      void refreshLogs();
    }
  }, [activeTab, refreshLogs]);

  // 快速启停服务
  const handleToggleService = async () => {
    if (!settings) return;
    const nextEnabled = !settings.enabled;
    try {
      await webhookUpdateSettings({ ...settings, enabled: nextEnabled });
      await refreshData();
    } catch (err) {
      console.error('切换服务开关失败:', err);
    }
  };

  // 新建规则
  const handleCreateNewRule = () => {
    const newId = `rule_${Date.now()}`;
    const stepPrefix = t('webhook.stepIndex', { defaultValue: '步骤' });
    const receiveText = t('webhook.stepDefaultReceive', { defaultValue: '接收请求' });
    const defaultSteps: WebhookActionStep[] = [
      {
        id: `step_${Date.now()}_1`,
        name: `${stepPrefix} 1: ${receiveText}`,
        actionType: 'local_command',
        targetHostId: hosts.length > 0 ? hosts[0].id : null,
        commandText: 'echo "Received WebHook: ${method} ${path}"',
        cwd: null,
        timeoutSecs: 60,
        continueOnError: false,
      },
    ];
    const newRule: WebhookRule = {
      id: newId,
      name: `${t('webhook.newRule')} ${rules.length + 1}`,
      enabled: true,
      method: 'POST',
      pathPattern: `/api/hook/${rules.length + 1}`,
      secretToken: null,
      signatureType: 'none',
      filterExpr: null,
      actionType: 'local_command',
      targetHostId: hosts.length > 0 ? hosts[0].id : null,
      commandText: 'echo "Received WebHook: ${method} ${path}"',
      cwd: null,
      timeoutSecs: 60,
      responseMode: 'async',
      actions: defaultSteps,
      actionsJson: JSON.stringify(defaultSteps),
      sortOrder: rules.length,
      createdAt: Date.now(),
      updatedAt: Date.now(),
    };
    setSelectedRuleId(newId);
    setEditingRule(newRule);
    setCollapsedStepKeys({});
    setTestResult(null);
  };

  // 选中规则
  const handleSelectRule = (r: WebhookRule) => {
    const steps = extractRuleSteps(r);
    setSelectedRuleId(r.id);
    setEditingRule({ ...r, actions: steps });
    setCollapsedStepKeys({});
    setTestResult(null);
  };

  // 保存当前编辑的规则
  const handleSaveRule = async () => {
    if (!editingRule) return;
    setSavingRule(true);
    const steps = editingRule.actions && editingRule.actions.length > 0
      ? editingRule.actions
      : extractRuleSteps(editingRule);
    const firstStep = steps[0];
    const ruleToSave: WebhookRule = {
      ...editingRule,
      actions: steps,
      actionsJson: JSON.stringify(steps),
      actionType: firstStep ? firstStep.actionType : editingRule.actionType,
      targetHostId: firstStep ? firstStep.targetHostId : editingRule.targetHostId,
      commandText: firstStep ? firstStep.commandText : editingRule.commandText,
      cwd: firstStep ? firstStep.cwd : editingRule.cwd,
      timeoutSecs: firstStep?.timeoutSecs || editingRule.timeoutSecs,
    };
    try {
      await webhookRuleSave(ruleToSave);
      await refreshData();
    } catch (err) {
      console.error('保存规则失败:', err);
    } finally {
      setSavingRule(false);
    }
  };

  // 确认删除规则
  const handleConfirmDeleteRule = async () => {
    if (!deletingRule) return;
    try {
      await webhookRuleDelete(deletingRule.id);
      if (selectedRuleId === deletingRule.id) {
        setSelectedRuleId(null);
        setEditingRule(null);
      }
      setDeletingRule(null);
      await refreshData();
    } catch (err) {
      console.error('删除规则失败:', err);
    }
  };

  // 切换规则启用/禁用
  const handleToggleRule = async (id: string, nextEnabled: boolean) => {
    try {
      await webhookRuleToggle(id, nextEnabled);
      await refreshData();
      if (editingRule && editingRule.id === id) {
        setEditingRule({ ...editingRule, enabled: nextEnabled });
      }
    } catch (err) {
      console.error('切换规则启用状态失败:', err);
    }
  };

  // 流水线步骤操作
  const handleUpdateStep = (index: number, patch: Partial<WebhookActionStep>) => {
    if (!editingRule) return;
    const currentSteps = editingRule.actions && editingRule.actions.length > 0
      ? editingRule.actions
      : extractRuleSteps(editingRule);
    const nextSteps = currentSteps.map((s, i) => (i === index ? { ...s, ...patch } : s));
    setEditingRule({ ...editingRule, actions: nextSteps });
  };

  const handleAddStep = () => {
    if (!editingRule) return;
    const currentSteps = editingRule.actions && editingRule.actions.length > 0
      ? editingRule.actions
      : extractRuleSteps(editingRule);
    const newIdx = currentSteps.length + 1;
    const stepPrefix = t('webhook.stepIndex', { defaultValue: '步骤' });
    const newStep: WebhookActionStep = {
      id: `step_${Date.now()}_${newIdx}`,
      name: `${stepPrefix} ${newIdx}`,
      actionType: 'js_script',
      targetHostId: hosts.length > 0 ? hosts[0].id : null,
      commandText: '// webhook.prev?.data\nreturn { success: true };',
      cwd: null,
      timeoutSecs: editingRule.timeoutSecs || 60,
      continueOnError: false,
    };
    setEditingRule({ ...editingRule, actions: [...currentSteps, newStep] });
  };

  const handleDeleteStep = (index: number) => {
    if (!editingRule) return;
    const currentSteps = editingRule.actions && editingRule.actions.length > 0
      ? editingRule.actions
      : extractRuleSteps(editingRule);
    if (currentSteps.length <= 1) return;
    const nextSteps = currentSteps.filter((_, i) => i !== index);
    setEditingRule({ ...editingRule, actions: nextSteps });
  };

  const handleMoveStep = (index: number, direction: 'up' | 'down') => {
    if (!editingRule) return;
    const currentSteps = [...(editingRule.actions && editingRule.actions.length > 0
      ? editingRule.actions
      : extractRuleSteps(editingRule))];
    const targetIndex = direction === 'up' ? index - 1 : index + 1;
    if (targetIndex < 0 || targetIndex >= currentSteps.length) return;
    const temp = currentSteps[index];
    currentSteps[index] = currentSteps[targetIndex];
    currentSteps[targetIndex] = temp;
    setEditingRule({ ...editingRule, actions: currentSteps });
  };

  const handleDuplicateStep = (index: number) => {
    if (!editingRule) return;
    const currentSteps = editingRule.actions && editingRule.actions.length > 0
      ? editingRule.actions
      : extractRuleSteps(editingRule);
    const target = currentSteps[index];
    const duplicated: WebhookActionStep = {
      ...target,
      id: `step_${Date.now()}_${currentSteps.length + 1}`,
      name: `${target.name || '步骤'} (${t('webhook.duplicateStep', { defaultValue: '副本' })})`,
    };
    const nextSteps = [...currentSteps];
    nextSteps.splice(index + 1, 0, duplicated);
    setEditingRule({ ...editingRule, actions: nextSteps });
  };

  const handleToggleStepCollapse = (stepKey: string) => {
    setCollapsedStepKeys((prev) => ({
      ...prev,
      [stepKey]: !prev[stepKey],
    }));
  };

  const handleToggleAllStepsCollapse = (steps: WebhookActionStep[]) => {
    setCollapsedStepKeys((prev) => {
      const allCollapsed = steps.every((s, idx) => prev[s.id || `step_${idx}`]);
      const next: Record<string, boolean> = {};
      for (let i = 0; i < steps.length; i++) {
        const key = steps[i].id || `step_${i}`;
        next[key] = !allCollapsed;
      }
      return next;
    });
  };

  // 复制 cURL
  const handleCopyCurl = (r: WebhookRule, e?: React.MouseEvent) => {
    e?.stopPropagation();
    const port = settings?.bindPort || 9876;
    const ip = settings?.bindIp === '0.0.0.0' ? '127.0.0.1' : settings?.bindIp || '127.0.0.1';
    let url = `http://${ip}:${port}${r.pathPattern.replace(/:([a-zA-Z0-9_]+)/g, '$1_value')}`;

    const headers: string[] = [];
    if (r.method !== 'GET') {
      headers.push('-H "Content-Type: application/json"');
    }
    if (r.signatureType === 'bearer' && r.secretToken) {
      headers.push(`-H "Authorization: Bearer ${r.secretToken}"`);
    } else if (r.signatureType === 'gitee_token' && r.secretToken) {
      headers.push(`-H "X-Gitee-Token: ${r.secretToken}"`);
    }

    if (settings?.globalSecret) {
      headers.push(`-H "X-Velo-Token: ${settings.globalSecret}"`);
    }

    const headerStr = headers.length > 0 ? ` ${headers.join(' ')}` : '';
    const bodyStr = r.method !== 'GET' ? ` -d '{"event":"trigger"}'` : '';
    const curlCmd = `curl -X ${r.method}${headerStr} "${url}"${bodyStr}`;

    void navigator.clipboard.writeText(curlCmd);
    setCopiedId(r.id);
    setTimeout(() => setCopiedId(null), 2000);
  };

  // 模拟测试当前编辑规则
  const handleTestRule = async () => {
    if (!editingRule) return;
    setTesting(true);
    setTestResult(null);

    let parsedJson: unknown = undefined;
    try {
      parsedJson = JSON.parse(testPayload);
    } catch {
      // 容错非 JSON
    }

    const ctx = {
      method: editingRule.method === 'ANY' ? 'POST' : editingRule.method,
      path: editingRule.pathPattern.replace(/:([a-zA-Z0-9_]+)/g, 'test_$1'),
      clientIp: '127.0.0.1',
      pathParams: { service: 'test-svc', env: 'test' },
      queryParams: { token: 'mock-token' },
      headers: {
        'content-type': 'application/json',
        'user-agent': 'VeloTerm-Mock-Tester',
      },
      bodyJson: parsedJson,
      rawBody: testPayload,
    };

    try {
      const res = await webhookRuleTest(editingRule, ctx);
      setTestResult(res);
    } catch (err) {
      setTestResult({
        success: false,
        exitCode: 1,
        stdout: '',
        stderr: err instanceof Error ? err.message : String(err),
        durationMs: 0,
      });
    } finally {
      setTesting(false);
    }
  };

  // 重放历史请求
  const handleReplayLog = async (logId: string) => {
    setReplayingId(logId);
    try {
      await webhookLogReplay(logId);
      await refreshLogs();
    } catch (err) {
      console.error('重放失败:', err);
    } finally {
      setReplayingId(null);
    }
  };

  // 清空日志
  const handleConfirmClearLogs = async () => {
    setClearingLogsBusy(true);
    try {
      await webhookLogsClear();
      setClearingLogsConfirm(false);
      setLogs([]);
      setTotalLogs(0);
      await refreshLogs();
    } catch (err) {
      console.error('清空日志失败:', err);
    } finally {
      setClearingLogsBusy(false);
    }
  };

  // 过滤后的规则列表
  const filteredRules = useMemo(() => {
    if (!ruleSearch.trim()) return rules;
    const q = ruleSearch.toLowerCase();
    return rules.filter(
      (r) => r.name.toLowerCase().includes(q) || r.pathPattern.toLowerCase().includes(q),
    );
  }, [rules, ruleSearch]);

  // 下拉选项配置
  const methodOptions: SelectMenuOption[] = [
    { value: 'POST', label: 'POST' },
    { value: 'GET', label: 'GET' },
    { value: 'PUT', label: 'PUT' },
    { value: 'DELETE', label: 'DELETE' },
    { value: 'PATCH', label: 'PATCH' },
    { value: 'ANY', label: t('webhook.methodAny', { defaultValue: 'ANY (任意 HTTP 方法)' }) },
  ];

  const signatureOptions: SelectMenuOption[] = [
    { value: 'none', label: t('webhook.sigNone') },
    { value: 'bearer', label: t('webhook.sigBearer') },
    { value: 'github_hmac_sha256', label: t('webhook.sigGithub') },
    { value: 'gitee_token', label: t('webhook.sigGitee') },
  ];

  const actionOptions: SelectMenuOption[] = [
    { value: 'local_command', label: t('webhook.actionLocal') },
    { value: 'ssh_command', label: t('webhook.actionSsh') },
    { value: 'js_script', label: t('webhook.actionJs') },
  ];

  const hostOptions: SelectMenuOption[] = useMemo(() => {
    return [
      { value: '', label: t('webhook.selectHost') },
      ...hosts.map((h) => ({
        value: String(h.id),
        label: `${h.name} (${h.username}@${h.host}:${h.port})`,
      })),
    ];
  }, [hosts, t]);

  const responseModeOptions: SelectMenuOption[] = [
    { value: 'async', label: t('webhook.responseAsync') },
    { value: 'sync', label: t('webhook.responseSync') },
  ];

  const matchStatusMap: Record<string, string> = {
    MATCHED: t('webhook.statusMatched', { defaultValue: '匹配成功' }),
    UNMATCHED: t('webhook.statusUnmatched', { defaultValue: '未匹配路由' }),
    AUTH_FAILED: t('webhook.statusAuthFailed', { defaultValue: '鉴权未通过' }),
    FILTER_SKIPPED: t('webhook.statusFilterSkipped', { defaultValue: '被过滤跳过' }),
  };

  if (loading) {
    return (
      <div className="flex h-full w-full items-center justify-center text-foreground-subtle">
        <Spinner className="size-6 text-foreground-subtle" />
      </div>
    );
  }

  return (
    <div className="flex h-full w-full flex-col overflow-hidden bg-background text-foreground select-none">
      {/* ── 顶栏：服务状态与控制 ── */}
      <div className="flex h-14 shrink-0 items-center justify-between border-b border-border/80 px-6 bg-surface/40">
        <div className="flex items-center gap-4">
          <div className="flex items-center gap-2.5">
            <div className="flex size-9 items-center justify-center rounded-xl bg-primary/10 text-primary">
              <Webhook className="size-5" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-ui-base font-semibold text-foreground tracking-tight">{t('webhook.title')}</h1>
                <span className="rounded-full bg-surface px-2 py-0.5 text-ui-xs text-foreground-subtle font-medium border border-border/50">
                  {status?.isRunning
                    ? `${t('webhook.serverRunning')} · ${status.bindIp}:${status.bindPort}`
                    : t('webhook.serverStopped')}
                </span>
              </div>
            </div>
          </div>

          <div className="h-5 w-px bg-border/60" />

          {/* 服务运行状态指示灯 */}
          <div className="flex items-center gap-2.5">
            <span
              className={cn(
                'size-2.5 rounded-full transition-colors',
                status?.isRunning
                  ? 'bg-terminal-green shadow-xs shadow-terminal-green/50 animate-pulse'
                  : 'bg-foreground-subtlest',
              )}
            />
            {status?.error && (
              <span className="flex items-center gap-1.5 text-ui-xs text-destructive max-w-sm truncate" title={status.error}>
                <AlertCircle className="size-3.5 shrink-0" />
                <span>{status.error}</span>
              </span>
            )}
          </div>
        </div>

        <div className="flex items-center gap-3">
          {/* 服务开关按钮 */}
          <Button
            type="button"
            variant={settings?.enabled ? 'default' : 'outline'}
            size="sm"
            onClick={handleToggleService}
            className="h-8.5 gap-2 px-3.5 text-ui-xs"
          >
            <Power className="size-3.5" />
            <span>{settings?.enabled ? t('webhook.stopService', { defaultValue: '停用服务' }) : t('webhook.startService', { defaultValue: '启用服务' })}</span>
          </Button>

          {/* 全局设置按钮 */}
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => setSettingsModalOpen(true)}
            className="h-8.5 gap-2 px-3.5 text-ui-xs"
          >
            <Settings className="size-3.5 text-foreground-subtle" />
            <span>{t('webhook.globalSettings')}</span>
          </Button>

          {/* Tab 切换 */}
          <div className="flex items-center rounded-lg border border-border/70 bg-surface/70 p-1 text-ui-xs">
            <button
              type="button"
              onClick={() => setActiveTab('rules')}
              className={cn(
                'rounded-md px-3 py-1 font-medium transition-colors',
                activeTab === 'rules' ? 'bg-background text-foreground shadow-xs' : 'text-foreground-subtle hover:text-foreground',
              )}
            >
              {t('webhook.rulesTab')} ({rules.length})
            </button>
            <button
              type="button"
              onClick={() => setActiveTab('logs')}
              className={cn(
                'rounded-md px-3 py-1 font-medium transition-colors',
                activeTab === 'logs' ? 'bg-background text-foreground shadow-xs' : 'text-foreground-subtle hover:text-foreground',
              )}
            >
              {t('webhook.logsTab')}
            </button>
          </div>
        </div>
      </div>

      {/* ── 主体区域 ── */}
      {activeTab === 'rules' ? (
        <div className="flex flex-1 min-h-0 w-full overflow-hidden">
          {/* 左侧：规则列表（宽度提升为 360px，间距更宽裕） */}
          <div className="flex w-88 md:w-96 shrink-0 flex-col border-r border-border/80 bg-surface/30">
            {/* 搜索与新建 */}
            <div className="p-3.5 border-b border-border/60">
              <div className="flex items-center gap-2">
                <div className="relative flex-1">
                  <Search className="pointer-events-none absolute left-3 top-2.5 size-4 text-foreground-subtlest" />
                  <input
                    type="text"
                    value={ruleSearch}
                    onChange={(e) => setRuleSearch(e.target.value)}
                    placeholder={t('webhook.searchPlaceholder')}
                    className="h-9 w-full rounded-md border border-border bg-input pl-9 pr-3 text-ui-sm text-foreground placeholder:text-foreground-subtlest focus:outline-none focus:border-border-hover transition-colors"
                  />
                </div>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={handleCreateNewRule}
                  className="h-9 shrink-0 gap-1.5 px-3 text-ui-xs font-medium"
                >
                  <Plus className="size-3.5" />
                  <span>{t('webhook.newRule')}</span>
                </Button>
              </div>
            </div>

            {/* 规则条目列表 */}
            <div className="flex-1 overflow-y-auto p-3 space-y-2.5">
              {filteredRules.length === 0 ? (
                <div className="flex flex-col items-center justify-center p-12 text-center text-ui-xs text-foreground-subtle">
                  <div className="flex size-12 items-center justify-center rounded-2xl bg-surface/80 text-foreground-subtlest mb-3 border border-border/50">
                    <Webhook className="size-6" />
                  </div>
                  <p className="font-semibold text-ui-sm text-foreground">{t('webhook.emptyRules')}</p>
                  <p className="mt-1.5 max-w-xs text-ui-xs text-foreground-subtlest leading-relaxed">
                    {t('webhook.emptyRulesDesc')}
                  </p>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={handleCreateNewRule}
                    className="mt-4 h-8 text-ui-xs"
                  >
                    <Plus className="size-3.5 mr-1.5" />
                    <span>{t('webhook.newRule')}</span>
                  </Button>
                </div>
              ) : (
                filteredRules.map((r) => {
                  const isSelected = selectedRuleId === r.id;
                  return (
                    <div
                      key={r.id}
                      onClick={() => handleSelectRule(r)}
                      className={cn(
                        'group flex flex-col gap-2 rounded-xl border p-3.5 cursor-pointer transition-all',
                        isSelected
                          ? 'border-primary/60 bg-selected/90 shadow-sm'
                          : 'border-border/60 bg-surface/50 hover:border-border hover:bg-hover',
                      )}
                    >
                      <div className="flex items-center justify-between gap-2">
                        <div className="flex items-center gap-2 min-w-0">
                          <span
                            className={cn(
                              'px-2 py-0.5 rounded text-[11px] font-mono font-bold border uppercase tracking-wider',
                              METHOD_COLORS[r.method] || METHOD_COLORS.ANY,
                            )}
                          >
                            {r.method}
                          </span>
                          <span className="font-semibold text-ui-sm text-foreground truncate">{r.name}</span>
                        </div>
                        <Switch
                          checked={r.enabled}
                          onChange={(checked) => void handleToggleRule(r.id, checked)}
                          aria-label={r.enabled ? t('common.enabled', { defaultValue: '已启用' }) : t('common.disabled', { defaultValue: '已停用' })}
                        />
                      </div>

                      <div className="flex items-center justify-between text-ui-xs text-foreground-subtle font-mono pt-0.5">
                        <span className="truncate pr-2" title={r.pathPattern}>{r.pathPattern}</span>
                        <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity shrink-0">
                          <button
                            type="button"
                            onClick={(e) => handleCopyCurl(r, e)}
                            className="p-1.5 rounded-md hover:bg-surface text-foreground-subtle hover:text-foreground transition-colors"
                            title={t('webhook.copyCurl')}
                          >
                            {copiedId === r.id ? <Check className="size-3.5 text-terminal-green" /> : <Copy className="size-3.5" />}
                          </button>
                          <button
                            type="button"
                            onClick={(e) => {
                              e.stopPropagation();
                              setDeletingRule(r);
                            }}
                            className="p-1.5 rounded-md hover:bg-destructive/10 text-foreground-subtle hover:text-destructive transition-colors"
                            title={t('common.delete', { defaultValue: '删除' })}
                          >
                            <Trash2 className="size-3.5" />
                          </button>
                        </div>
                      </div>
                    </div>
                  );
                })
              )}
            </div>
          </div>

          {/* 右侧：规则编辑与测试面板（空间充裕，大方排版） */}
          <div className="flex flex-1 min-w-0 flex-col overflow-y-auto p-6 md:p-8 space-y-6 bg-background">
            {editingRule ? (
              <div className="max-w-4xl space-y-6">
                {/* 规则头部标题栏 */}
                <div className="flex items-center justify-between border-b border-border/70 pb-4">
                  <div className="space-y-1">
                    <h2 className="text-ui-lg font-bold text-foreground tracking-tight">{editingRule.name}</h2>
                    <p className="text-ui-xs text-foreground-subtle font-mono">{editingRule.pathPattern}</p>
                  </div>
                  <div className="flex items-center gap-2.5">
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => handleCopyCurl(editingRule)}
                      className="gap-2 text-ui-xs h-8.5 px-3.5"
                    >
                      {copiedId === editingRule.id ? <Check className="size-3.5 text-terminal-green" /> : <Copy className="size-3.5" />}
                      <span>{t('webhook.copyCurl')}</span>
                    </Button>
                    <Button
                      type="button"
                      variant="default"
                      size="sm"
                      onClick={handleSaveRule}
                      disabled={savingRule}
                      className="gap-2 text-ui-xs h-8.5 px-4"
                    >
                      {savingRule && <Spinner className="size-3.5" />}
                      <span>{t('webhook.saveRule')}</span>
                    </Button>
                  </div>
                </div>

                {/* 1. 基本匹配条件 */}
                <div className="space-y-4 rounded-xl border border-border/70 bg-surface/30 p-5">
                  <span className="text-ui-xs font-semibold text-foreground-subtle uppercase tracking-wider flex items-center gap-1.5">
                    <Layers className="size-4" />
                    <span>{t('webhook.matchConfig', { defaultValue: '请求匹配配置' })}</span>
                  </span>

                  <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                    <div className="md:col-span-2 space-y-1.5">
                      <label className="text-ui-xs font-medium text-foreground-subtle">{t('webhook.ruleName')}</label>
                      <input
                        type="text"
                        value={editingRule.name}
                        onChange={(e) => setEditingRule({ ...editingRule, name: e.target.value })}
                        placeholder={t('webhook.ruleNamePlaceholder')}
                        className="w-full h-9 rounded-md border border-border bg-input px-3 text-ui-sm text-foreground focus:outline-none focus:border-border-hover transition-colors"
                      />
                    </div>

                    <div className="space-y-1.5">
                      <label className="text-ui-xs font-medium text-foreground-subtle">{t('webhook.method')}</label>
                      <SelectMenu
                        value={editingRule.method}
                        options={methodOptions}
                        onChange={(val) => setEditingRule({ ...editingRule, method: val as WebhookMethod })}
                        triggerClassName="h-9 text-ui-sm font-mono"
                      />
                    </div>
                  </div>

                  <div className="space-y-1.5">
                    <label className="text-ui-xs font-medium text-foreground-subtle">{t('webhook.pathPattern')}</label>
                    <input
                      type="text"
                      value={editingRule.pathPattern}
                      onChange={(e) => setEditingRule({ ...editingRule, pathPattern: e.target.value })}
                      placeholder={t('webhook.pathPatternPlaceholder')}
                      className="w-full h-9 rounded-md border border-border bg-input px-3 text-ui-sm text-foreground font-mono focus:outline-none focus:border-border-hover transition-colors"
                    />
                    <p className="text-ui-xs text-foreground-subtler leading-relaxed">{t('webhook.pathPatternHint')}</p>
                  </div>
                </div>

                {/* 2. 签名认证与过滤表达式 */}
                <div className="space-y-4 rounded-xl border border-border/70 bg-surface/30 p-5">
                  <span className="text-ui-xs font-semibold text-foreground-subtle uppercase tracking-wider flex items-center gap-1.5">
                    <Shield className="size-4" />
                    <span>{t('webhook.securityConfig', { defaultValue: '安全鉴权与过滤' })}</span>
                  </span>

                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <div className="space-y-1.5">
                      <label className="flex items-center gap-1.5 text-ui-xs font-medium text-foreground-subtle">
                        <Lock className="size-3.5 text-foreground-subtle" />
                        <span>{t('webhook.signatureType')}</span>
                      </label>
                      <SelectMenu
                        value={editingRule.signatureType}
                        options={signatureOptions}
                        onChange={(val) => setEditingRule({ ...editingRule, signatureType: val as WebhookSignatureType })}
                        triggerClassName="h-9 text-ui-sm"
                      />
                    </div>

                    {editingRule.signatureType !== 'none' ? (
                      <div className="space-y-1.5">
                        <label className="text-ui-xs font-medium text-foreground-subtle">{t('webhook.secretToken')}</label>
                        <input
                          type="password"
                          value={editingRule.secretToken || ''}
                          onChange={(e) => setEditingRule({ ...editingRule, secretToken: e.target.value || null })}
                          placeholder={t('webhook.secretTokenPlaceholder')}
                          className="w-full h-9 rounded-md border border-border bg-input px-3 text-ui-sm text-foreground font-mono focus:outline-none focus:border-border-hover transition-colors"
                        />
                      </div>
                    ) : (
                      <div className="space-y-1.5 opacity-60">
                        <label className="text-ui-xs font-medium text-foreground-subtle">{t('webhook.secretToken')}</label>
                        <div className="w-full h-9 rounded-md border border-border/40 bg-surface/30 px-3 flex items-center text-ui-xs text-foreground-subtlest">
                          {t('webhook.noSecretRequired', { defaultValue: '无需填写密钥' })}
                        </div>
                      </div>
                    )}

                    <div className="md:col-span-2 space-y-1.5">
                      <label className="text-ui-xs font-medium text-foreground-subtle">{t('webhook.filterExpr')}</label>
                      <input
                        type="text"
                        value={editingRule.filterExpr || ''}
                        onChange={(e) => setEditingRule({ ...editingRule, filterExpr: e.target.value || null })}
                        placeholder={t('webhook.filterExprPlaceholder')}
                        className="w-full h-9 rounded-md border border-border bg-input px-3 text-ui-sm text-foreground font-mono focus:outline-none focus:border-border-hover transition-colors"
                      />
                    </div>
                  </div>
                </div>

                {/* 3. 动作执行流水线（支持单步及多步骤链式执行） */}
                {(() => {
                  const currentSteps = editingRule.actions && editingRule.actions.length > 0
                    ? editingRule.actions
                    : extractRuleSteps(editingRule);
                  const isMultiStep = currentSteps.length > 1;

                  return (
                    <div className="space-y-4 rounded-xl border border-border/70 bg-surface/30 p-5">
                      <div className="flex items-center justify-between border-b border-border/50 pb-3">
                        <div className="flex items-center gap-2">
                          <Terminal className="size-4 text-terminal-blue" />
                          <span className="text-ui-sm font-semibold text-foreground">
                            {t('webhook.pipeline', { defaultValue: '动作执行流水线' })}
                          </span>
                          <span className="rounded-full bg-surface px-2 py-0.5 font-mono text-[11px] text-foreground-subtle border border-border/50 select-none">
                            {currentSteps.length} {t('webhook.stepIndex', { defaultValue: '步骤' })}
                          </span>
                        </div>
                        <div className="flex items-center gap-2">
                          {currentSteps.length > 1 && (
                            <Button
                              type="button"
                              variant="ghost"
                              size="sm"
                              onClick={() => handleToggleAllStepsCollapse(currentSteps)}
                              className="h-7.5 gap-1.5 px-2 text-ui-xs text-foreground-subtle hover:text-foreground"
                              title={
                                currentSteps.every((s, i) => collapsedStepKeys[s.id || `step_${i}`])
                                  ? t('webhook.expandAllSteps', { defaultValue: '全部展开' })
                                  : t('webhook.collapseAllSteps', { defaultValue: '全部折叠' })
                              }
                            >
                              <ChevronsUpDown className="size-3.5" />
                              <span>
                                {currentSteps.every((s, i) => collapsedStepKeys[s.id || `step_${i}`])
                                  ? t('webhook.expandAllSteps', { defaultValue: '全部展开' })
                                  : t('webhook.collapseAllSteps', { defaultValue: '全部折叠' })}
                              </span>
                            </Button>
                          )}
                          <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            onClick={handleAddStep}
                            className="h-7.5 gap-1.5 px-2.5 text-ui-xs font-medium"
                          >
                            <Plus className="size-3.5 text-terminal-green" />
                            <span>{t('webhook.addStep', { defaultValue: '添加流水线步骤' })}</span>
                          </Button>
                        </div>
                      </div>

                      <p className="text-ui-xs text-foreground-subtler leading-relaxed">
                        {t(
                          'webhook.pipelineHint',
                          { defaultValue: '支持按序链式执行多步骤（如：第1步JS处理 -> 第2步命令部署 -> 第3步JS发通知）。后一步骤可通过 ${prev.stdout}、${prev.data.xxx} 或 webhook.prev 获取前一步输出。' }
                        )}
                      </p>

                      {/* 步骤卡片列表 */}
                      <div className="space-y-3.5">
                        {currentSteps.map((step, idx) => {
                          const stepKey = step.id || `step_${idx}`;
                          const isCollapsed = Boolean(collapsedStepKeys[stepKey]);
                          const summaryText = (step.commandText || '')
                            .split('\n')
                            .map((line) => line.trim())
                            .find((line) => line.length > 0 && !line.startsWith('//') && !line.startsWith('#')) || (step.commandText || '').trim();

                          return (
                            <div
                              key={stepKey}
                              className="rounded-lg border border-border/60 bg-surface/30 overflow-hidden shadow-xs transition-colors"
                            >
                              {/* 步骤标题栏（可点击整栏折叠/展开） */}
                              <div
                                onClick={() => handleToggleStepCollapse(stepKey)}
                                className={cn(
                                  'flex h-10 items-center justify-between bg-surface/40 px-3 select-none cursor-pointer hover:bg-surface/60 transition-colors',
                                  !isCollapsed && 'border-b border-border/40',
                                )}
                              >
                                <div className="flex flex-1 min-w-0 items-center gap-2">
                                  <ChevronRight
                                    className={cn(
                                      'size-3.5 text-foreground-subtle shrink-0 transition-transform duration-200',
                                      !isCollapsed && 'rotate-90',
                                    )}
                                  />
                                  <span className="flex size-5.5 shrink-0 items-center justify-center rounded-full bg-surface border border-border text-[11px] font-mono font-semibold text-foreground">
                                    {idx + 1}
                                  </span>
                                  <input
                                    type="text"
                                    value={step.name || ''}
                                    onClick={(e) => e.stopPropagation()}
                                    onChange={(e) => handleUpdateStep(idx, { name: e.target.value })}
                                    placeholder={`${t('webhook.stepIndex', { defaultValue: '步骤' })} ${idx + 1}`}
                                    className="h-6.5 max-w-xs rounded border border-transparent bg-transparent px-1.5 font-mono text-ui-xs font-medium text-foreground hover:border-border focus:border-border focus:bg-input focus:outline-none transition-colors"
                                  />
                                  <span
                                    className={cn(
                                      'hidden sm:inline-flex rounded px-1.5 py-0.5 text-[10px] font-mono select-none shrink-0',
                                      step.actionType === 'js_script'
                                        ? 'bg-terminal-yellow/15 text-terminal-yellow'
                                        : step.actionType === 'ssh_command'
                                        ? 'bg-purple-500/15 text-purple-400'
                                        : 'bg-terminal-blue/15 text-terminal-blue',
                                    )}
                                  >
                                    {step.actionType === 'js_script'
                                      ? 'JS / TS'
                                      : step.actionType === 'ssh_command'
                                      ? 'SSH'
                                      : 'Local'}
                                  </span>

                                  {/* 折叠时在标题栏右侧展示代码预览 */}
                                  {isCollapsed && summaryText && (
                                    <span className="text-[11px] text-foreground-subtlest font-mono truncate max-w-xs sm:max-w-md hidden md:inline ml-1">
                                      {summaryText}
                                    </span>
                                  )}
                                </div>

                                <div
                                  className="flex items-center gap-1 shrink-0 ml-2"
                                  onClick={(e) => e.stopPropagation()}
                                >
                                  <Button
                                    type="button"
                                    variant="ghost"
                                    size="sm"
                                    onClick={() => handleMoveStep(idx, 'up')}
                                    disabled={idx === 0}
                                    className="size-7 p-0 text-foreground-subtle hover:text-foreground disabled:opacity-30"
                                    title={t('webhook.moveUp', { defaultValue: '上移' })}
                                  >
                                    <ArrowUp className="size-3.5" />
                                  </Button>
                                  <Button
                                    type="button"
                                    variant="ghost"
                                    size="sm"
                                    onClick={() => handleMoveStep(idx, 'down')}
                                    disabled={idx === currentSteps.length - 1}
                                    className="size-7 p-0 text-foreground-subtle hover:text-foreground disabled:opacity-30"
                                    title={t('webhook.moveDown', { defaultValue: '下移' })}
                                  >
                                    <ArrowDown className="size-3.5" />
                                  </Button>
                                  <Button
                                    type="button"
                                    variant="ghost"
                                    size="sm"
                                    onClick={() => handleDuplicateStep(idx)}
                                    className="size-7 p-0 text-foreground-subtle hover:text-foreground"
                                    title={t('webhook.duplicateStep', { defaultValue: '复制步骤' })}
                                  >
                                    <Copy className="size-3.5" />
                                  </Button>
                                  <Button
                                    type="button"
                                    variant="ghost"
                                    size="sm"
                                    onClick={() => handleDeleteStep(idx)}
                                    disabled={currentSteps.length <= 1}
                                    className="size-7 p-0 text-destructive/80 hover:text-destructive hover:bg-destructive/10 disabled:opacity-30"
                                    title={t('webhook.deleteStep', { defaultValue: '删除步骤' })}
                                  >
                                    <Trash2 className="size-3.5" />
                                  </Button>
                                </div>
                              </div>

                              {/* 步骤主体配置（仅在未折叠时展开） */}
                              {!isCollapsed && (
                                <div className="p-3.5 space-y-3.5 animate-in fade-in duration-150">
                                  <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                                    <div className="space-y-1">
                                      <label className="text-[11px] font-medium text-foreground-subtle">
                                        {t('webhook.actionType')}
                                      </label>
                                      <SelectMenu
                                        value={step.actionType}
                                        options={actionOptions}
                                        onChange={(val) => handleUpdateStep(idx, { actionType: val as WebhookActionType })}
                                        triggerClassName="h-8 text-ui-xs"
                                      />
                                    </div>

                                    {step.actionType === 'ssh_command' && (
                                      <div className="space-y-1">
                                        <label className="flex items-center gap-1 text-[11px] font-medium text-foreground-subtle">
                                          <Server className="size-3 text-foreground-subtle" />
                                          <span>{t('webhook.targetHost')}</span>
                                        </label>
                                        <SelectMenu
                                          value={step.targetHostId ? String(step.targetHostId) : ''}
                                          options={hostOptions}
                                          onChange={(val) => handleUpdateStep(idx, { targetHostId: val ? Number(val) : null })}
                                          triggerClassName="h-8 text-ui-xs"
                                        />
                                      </div>
                                    )}

                                    {step.actionType === 'local_command' && (
                                      <div className="space-y-1">
                                        <label className="text-[11px] font-medium text-foreground-subtle">
                                          {t('webhook.cwd')}
                                        </label>
                                        <input
                                          type="text"
                                          value={step.cwd || ''}
                                          onChange={(e) => handleUpdateStep(idx, { cwd: e.target.value || null })}
                                          placeholder={t('webhook.cwdPlaceholder')}
                                          className="w-full h-8 rounded-md border border-border bg-input px-2.5 text-ui-xs text-foreground font-mono focus:outline-none focus:border-border-hover transition-colors"
                                        />
                                      </div>
                                    )}

                                    <div className="space-y-1">
                                      <label className="flex items-center gap-1 text-[11px] font-medium text-foreground-subtle">
                                        <Clock className="size-3 text-foreground-subtle" />
                                        <span>{t('webhook.stepTimeout', { defaultValue: '步骤独立超时 (秒)' })}</span>
                                      </label>
                                      <input
                                        type="number"
                                        min={1}
                                        max={600}
                                        value={step.timeoutSecs || 60}
                                        onChange={(e) => handleUpdateStep(idx, { timeoutSecs: Number(e.target.value) || 60 })}
                                        className="w-full h-8 rounded-md border border-border bg-input px-2.5 text-ui-xs text-foreground font-mono focus:outline-none focus:border-border-hover transition-colors"
                                      />
                                    </div>
                                  </div>

                                  {/* 流水线错误控制策略（仅多步骤时呈现） */}
                                  {isMultiStep && (
                                    <div className="flex items-center gap-2 pt-0.5">
                                      <input
                                        type="checkbox"
                                        id={`continue_on_err_${step.id || idx}`}
                                        checked={Boolean(step.continueOnError)}
                                        onChange={(e) => handleUpdateStep(idx, { continueOnError: e.target.checked })}
                                        className="size-3.5 rounded border-border accent-foreground-subtle"
                                      />
                                      <label
                                        htmlFor={`continue_on_err_${step.id || idx}`}
                                        className="text-ui-xs text-foreground-subtle select-none cursor-pointer"
                                      >
                                        {t('webhook.continueOnError', { defaultValue: '步骤失败时仍继续后续流水线' })}
                                      </label>
                                    </div>
                                  )}

                                  {/* 动作逻辑：JS 脚本嵌入 Monaco 编辑器；Shell 命令嵌入高亮文本框 */}
                                  <div className="space-y-1.5 pt-1">
                                    <label className="text-[11px] font-medium text-foreground-subtle flex items-center justify-between">
                                      <span>
                                        {step.actionType === 'js_script' ? t('webhook.jsScript') : t('webhook.command')}
                                      </span>
                                      {step.actionType !== 'js_script' && isMultiStep && idx > 0 && (
                                        <span className="text-[10px] text-foreground-subtlest font-mono">
                                          {t('webhook.prevVariablesHint', { defaultValue: '可使用 ${prev.stdout} / ${prev.data.xxx}' })}
                                        </span>
                                      )}
                                    </label>

                                    {step.actionType === 'js_script' ? (
                                      <WebhookScriptEditor
                                        value={step.commandText}
                                        onChange={(val) => handleUpdateStep(idx, { commandText: val })}
                                        height={200}
                                      />
                                    ) : (
                                      <WebhookCommandEditor
                                        value={step.commandText}
                                        onChange={(val) => handleUpdateStep(idx, { commandText: val })}
                                        actionType={step.actionType as 'local_command' | 'ssh_command'}
                                        isMultiStep={isMultiStep}
                                        stepIndex={idx}
                                        height={200}
                                      />
                                    )}
                                  </div>
                                </div>
                              )}
                            </div>
                          );
                        })}
                      </div>

                      {/* 响应模式与默认总超时 */}
                      <div className="grid grid-cols-1 md:grid-cols-2 gap-4 pt-3 border-t border-border/50">
                        <div className="space-y-1.5">
                          <label className="text-ui-xs font-medium text-foreground-subtle">
                            {t('webhook.responseMode')}
                          </label>
                          <SelectMenu
                            value={editingRule.responseMode}
                            options={responseModeOptions}
                            onChange={(val) => setEditingRule({ ...editingRule, responseMode: val as 'async' | 'sync' })}
                            triggerClassName="h-9 text-ui-sm"
                          />
                        </div>

                        <div className="space-y-1.5">
                          <label className="flex items-center gap-1.5 text-ui-xs font-medium text-foreground-subtle">
                            <Clock className="size-3.5 text-foreground-subtle" />
                            <span>{t('webhook.timeout')}</span>
                          </label>
                          <input
                            type="number"
                            min={1}
                            max={600}
                            value={editingRule.timeoutSecs}
                            onChange={(e) => setEditingRule({ ...editingRule, timeoutSecs: Number(e.target.value) || 60 })}
                            className="w-full h-9 rounded-md border border-border bg-input px-3 text-ui-sm text-foreground font-mono focus:outline-none focus:border-border-hover transition-colors"
                          />
                        </div>
                      </div>
                    </div>
                  );
                })()}

                {/* 4. 模拟测试面板 */}
                <div className="rounded-xl border border-border/60 bg-surface/30 p-5 space-y-4">
                  <div className="flex items-center justify-between">
                    <div className="space-y-0.5">
                      <span className="text-ui-sm font-semibold text-foreground flex items-center gap-2">
                        <Play className="size-4 text-terminal-green" />
                        <span>{t('webhook.testRule')}</span>
                      </span>
                      <p className="text-ui-xs text-foreground-subtle">{t('webhook.testRuleDesc', { defaultValue: '模拟外部调用此 WebHook 接口，原地执行并观察实际返回' })}</p>
                    </div>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={handleTestRule}
                      disabled={testing}
                      className="h-8.5 gap-2 px-3.5 text-ui-xs"
                    >
                      {testing ? <Spinner className="size-3.5" /> : <Play className="size-3.5 text-terminal-green" />}
                      <span>{testing ? t('webhook.testing') : t('webhook.testRule')}</span>
                    </Button>
                  </div>

                  <div className="space-y-1.5">
                    <label className="text-ui-xs font-medium text-foreground-subtle">{t('webhook.mockPayload', { defaultValue: '模拟请求体 (JSON Body):' })}</label>
                    <WebhookReadOnlyEditor
                      value={testPayload}
                      language="json"
                      minHeight={64}
                      maxHeight={320}
                    />
                  </div>

                  {testResult && (
                    <div className="rounded-lg border border-border bg-surface/90 p-4 space-y-2 text-ui-xs font-mono shadow-xs">
                      <div className="flex items-center gap-3">
                        <span
                          className={cn(
                            'px-2 py-0.5 rounded text-ui-2xs font-bold uppercase tracking-wider',
                            testResult.success ? 'bg-terminal-green/20 text-terminal-green' : 'bg-destructive/20 text-destructive',
                          )}
                        >
                          {testResult.success ? t('webhook.execSuccess') : t('webhook.execFailed')}
                        </span>
                        <span className="text-foreground-subtle font-medium">{t('webhook.exitCode', { defaultValue: 'Exit Code' })}: {testResult.exitCode ?? 'None'}</span>
                        <span className="text-foreground-subtle font-medium">{t('webhook.duration', { defaultValue: '耗时' })}: {testResult.durationMs}ms</span>
                      </div>
                      {testResult.stdout && (
                        <div className="space-y-1">
                          <span className="text-[11px] text-foreground-subtler block">{t('webhook.stdout', { defaultValue: 'Standard Output' })}:</span>
                          <WebhookReadOnlyEditor
                            value={testResult.stdout}
                            language="shell"
                            headerTitle="stdout"
                            minHeight={48}
                            maxHeight={260}
                          />
                        </div>
                      )}
                      {testResult.stderr && (
                        <div className="space-y-1">
                          <span className="text-[11px] text-destructive block">{t('webhook.stderr', { defaultValue: 'Standard Error' })}:</span>
                          <WebhookReadOnlyEditor
                            value={testResult.stderr}
                            language="shell"
                            headerTitle="stderr"
                            minHeight={48}
                            maxHeight={260}
                            isError
                          />
                        </div>
                      )}
                    </div>
                  )}
                </div>
              </div>
            ) : (
              <div className="flex flex-1 flex-col items-center justify-center text-ui-xs text-foreground-subtle space-y-2">
                <Webhook className="size-10 text-foreground-subtlest" />
                <p className="text-ui-sm font-medium text-foreground">{t('webhook.noRuleSelected', { defaultValue: '未选择任何 WebHook 规则' })}</p>
                <p className="text-foreground-subtler">{t('webhook.selectRuleHint', { defaultValue: '在左侧列表中点击选择一条规则，或点击「+ 新建规则」开始配置' })}</p>
              </div>
            )}
          </div>
        </div>
      ) : (
        /* ── 审计日志视图（开阔表格与详情） ── */
        <div className="flex flex-1 min-h-0 w-full flex-col overflow-hidden p-6 md:p-8 space-y-4">
          <div className="flex items-center justify-between pb-3 border-b border-border/70">
            <div className="flex items-center gap-2.5">
              <span className="text-ui-base font-bold text-foreground">{t('webhook.logsTitle')}</span>
              <span className="rounded-full bg-surface px-2.5 py-0.5 text-ui-xs text-foreground-subtle border border-border/50">
                {t('webhook.totalLogsCount', { count: totalLogs, defaultValue: `${totalLogs} 条记录` })}
              </span>
            </div>
            <div className="flex items-center gap-2.5">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={refreshLogs}
                disabled={logsLoading}
                className="h-8.5 gap-2 px-3 text-ui-xs"
              >
                <RefreshCw className={cn('size-3.5', logsLoading && 'animate-spin')} />
                <span>{t('common.refresh', { defaultValue: '刷新' })}</span>
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => {
                  if (logs.length > 0 || totalLogs > 0) {
                    setClearingLogsConfirm(true);
                  }
                }}
                disabled={logs.length === 0 && totalLogs === 0}
                title={logs.length === 0 && totalLogs === 0 ? t('webhook.emptyLogs') : t('webhook.clearLogs')}
                className="h-8.5 gap-2 px-3 text-ui-xs text-destructive hover:bg-destructive/10"
              >
                <Trash2 className="size-3.5" />
                <span>{t('webhook.clearLogs')}</span>
              </Button>
            </div>
          </div>

          <div className="flex-1 overflow-y-auto space-y-3">
            {logs.length === 0 ? (
              <div className="flex flex-col items-center justify-center p-16 text-center text-ui-xs text-foreground-subtle">
                <div className="flex size-12 items-center justify-center rounded-2xl bg-surface/80 text-foreground-subtlest mb-3 border border-border/50">
                  <Clock className="size-6" />
                </div>
                <p className="font-semibold text-ui-sm text-foreground">{t('webhook.emptyLogs')}</p>
                <p className="mt-1.5 max-w-sm text-foreground-subtler leading-relaxed">{t('webhook.emptyLogsDesc')}</p>
              </div>
            ) : (
              logs.map((log) => {
                const isExpanded = selectedLogId === log.id;
                return (
                  <div
                    key={log.id}
                    className="flex flex-col rounded-xl border border-border/70 bg-surface/40 overflow-hidden text-ui-xs font-mono transition-all hover:border-border"
                  >
                    <div
                      onClick={() => setSelectedLogId(isExpanded ? null : log.id)}
                      className="flex items-center justify-between p-3.5 cursor-pointer hover:bg-hover/60 transition-colors"
                    >
                      <div className="flex items-center gap-3 min-w-0">
                        <span
                          className={cn(
                            'px-2 py-0.5 rounded text-[11px] font-bold border uppercase tracking-wider',
                            METHOD_COLORS[log.method] || METHOD_COLORS.ANY,
                          )}
                        >
                          {log.method}
                        </span>
                        <span className="font-semibold text-ui-sm text-foreground truncate">{log.path}</span>
                        {log.ruleName && (
                          <span className="text-ui-xs text-foreground-subtle truncate">
                            · {log.ruleName}
                          </span>
                        )}
                      </div>

                      <div className="flex items-center gap-4 shrink-0">
                        <span
                          className={cn(
                            'px-2 py-0.5 rounded text-[11px] font-semibold tracking-wide',
                            log.matchStatus === 'MATCHED'
                              ? log.executionStatus === 'SUCCESS'
                                ? 'bg-terminal-green/15 text-terminal-green'
                                : 'bg-destructive/15 text-destructive'
                              : 'bg-foreground-subtlest/20 text-foreground-subtle',
                          )}
                        >
                          {log.matchStatus === 'MATCHED'
                            ? log.executionStatus === 'SUCCESS'
                              ? t('webhook.execSuccess')
                              : t('webhook.execFailed')
                            : matchStatusMap[log.matchStatus] || log.matchStatus}
                        </span>
                        <span className="text-ui-xs text-foreground-subtlest">{log.clientIp}</span>
                        <span className="text-ui-xs text-foreground-subtlest">{formatTime(log.createdAt)}</span>
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          onClick={(e) => {
                            e.stopPropagation();
                            void handleReplayLog(log.id);
                          }}
                          disabled={replayingId === log.id || !log.ruleId}
                          className="h-7 gap-1.5 px-2.5 text-ui-xs text-terminal-blue hover:bg-terminal-blue/10"
                        >
                          {replayingId === log.id ? <Spinner className="size-3" /> : <Play className="size-3" />}
                          <span>{t('webhook.replay')}</span>
                        </Button>
                      </div>
                    </div>

                    {isExpanded && (
                      <div className="border-t border-border/60 p-4 bg-surface/70 space-y-3 font-sans">
                        {log.requestHeaders && (
                          <div className="space-y-1">
                            <span className="text-ui-xs text-foreground-subtler font-medium">{t('webhook.logHeaders', { defaultValue: '请求头 (Headers)' })}:</span>
                            <WebhookReadOnlyEditor
                              value={log.requestHeaders}
                              language="json"
                              headerTitle="Headers"
                              minHeight={44}
                              maxHeight={200}
                            />
                          </div>
                        )}
                        {log.requestBody && (
                          <div className="space-y-1">
                            <span className="text-ui-xs text-foreground-subtler font-medium">{t('webhook.logPayload', { defaultValue: '请求体 (Payload Body)' })}:</span>
                            <WebhookReadOnlyEditor
                              value={log.requestBody}
                              language="json"
                              headerTitle="Payload"
                              minHeight={44}
                              maxHeight={220}
                            />
                          </div>
                        )}
                        {(log.stdoutTail || log.stderrTail) && (
                          <div className="space-y-1">
                            <span className="text-ui-xs text-foreground-subtler font-medium">{t('webhook.logOutput', { defaultValue: '执行输出 (Output)' })}:</span>
                            {log.stdoutTail && (
                              <WebhookReadOnlyEditor
                                value={log.stdoutTail}
                                language="shell"
                                headerTitle="stdout"
                                minHeight={44}
                                maxHeight={220}
                              />
                            )}
                            {log.stderrTail && (
                              <WebhookReadOnlyEditor
                                value={log.stderrTail}
                                language="shell"
                                headerTitle="stderr"
                                minHeight={44}
                                maxHeight={220}
                                isError
                              />
                            )}
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                );
              })
            )}
          </div>
        </div>
      )}

      {/* ── 全局设置弹窗 ── */}
      {settings && (
        <WebhookSettingsModal
          settings={settings}
          isOpen={settingsModalOpen}
          onClose={() => setSettingsModalOpen(false)}
          onSaved={refreshData}
        />
      )}

      {/* ── 删除规则确认弹窗 ── */}
      <ConfirmDialog
        open={Boolean(deletingRule)}
        title={t('webhook.deleteRule')}
        description={t('webhook.deleteConfirm', { name: deletingRule?.name || '' })}
        onConfirm={() => void handleConfirmDeleteRule()}
        onCancel={() => setDeletingRule(null)}
      />

      {/* ── 清空日志确认弹窗 ── */}
      <ConfirmDialog
        open={clearingLogsConfirm}
        busy={clearingLogsBusy}
        title={t('webhook.clearLogs')}
        description={t('webhook.clearLogsConfirm')}
        confirmLabel={t('webhook.clearLogs', { defaultValue: '清空' })}
        onConfirm={() => void handleConfirmClearLogs()}
        onCancel={() => setClearingLogsConfirm(false)}
      />
    </div>
  );
}
