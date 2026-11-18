import {
  AlertCircle,
  Check,
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

  // 删除确认对话框（替代 window.confirm）
  const [deletingRule, setDeletingRule] = useState<WebhookRule | null>(null);
  const [clearingLogsConfirm, setClearingLogsConfirm] = useState(false);

  // 测试运行面板状态
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<WebhookTestResult | null>(null);
  const [testPayload, setTestPayload] = useState('{\n  "ref": "refs/heads/main",\n  "event": "push"\n}');

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
      sortOrder: rules.length,
      createdAt: Date.now(),
      updatedAt: Date.now(),
    };
    setSelectedRuleId(newId);
    setEditingRule(newRule);
    setTestResult(null);
  };

  // 选中规则
  const handleSelectRule = (r: WebhookRule) => {
    setSelectedRuleId(r.id);
    setEditingRule({ ...r });
    setTestResult(null);
  };

  // 保存当前编辑的规则
  const handleSaveRule = async () => {
    if (!editingRule) return;
    setSavingRule(true);
    try {
      await webhookRuleSave(editingRule);
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
    try {
      await webhookLogsClear();
      setClearingLogsConfirm(false);
      await refreshLogs();
    } catch (err) {
      console.error('清空日志失败:', err);
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
    { value: 'ANY', label: 'ANY (任意 HTTP 方法)' },
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
            <span>{settings?.enabled ? t('common.disable', { defaultValue: '停用服务' }) : t('common.enable', { defaultValue: '启用服务' })}</span>
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
                    className="h-9 w-full rounded-md border border-border bg-surface/80 pl-9 pr-3 text-ui-sm text-foreground placeholder:text-foreground-subtlest focus:outline-none focus:border-border-hover transition-colors"
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
                          aria-label={r.enabled ? '已启用' : '已停用'}
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
                    <span>请求匹配配置</span>
                  </span>

                  <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                    <div className="md:col-span-2 space-y-1.5">
                      <label className="text-ui-xs font-medium text-foreground-subtle">{t('webhook.ruleName')}</label>
                      <input
                        type="text"
                        value={editingRule.name}
                        onChange={(e) => setEditingRule({ ...editingRule, name: e.target.value })}
                        placeholder={t('webhook.ruleNamePlaceholder')}
                        className="w-full h-9 rounded-md border border-border bg-surface/80 px-3 text-ui-sm text-foreground focus:outline-none focus:border-border-hover transition-colors"
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
                      className="w-full h-9 rounded-md border border-border bg-surface/80 px-3 text-ui-sm text-foreground font-mono focus:outline-none focus:border-border-hover transition-colors"
                    />
                    <p className="text-ui-xs text-foreground-subtler leading-relaxed">{t('webhook.pathPatternHint')}</p>
                  </div>
                </div>

                {/* 2. 签名认证与过滤表达式 */}
                <div className="space-y-4 rounded-xl border border-border/70 bg-surface/30 p-5">
                  <span className="text-ui-xs font-semibold text-foreground-subtle uppercase tracking-wider flex items-center gap-1.5">
                    <Shield className="size-4" />
                    <span>安全鉴权与过滤</span>
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
                          className="w-full h-9 rounded-md border border-border bg-surface/80 px-3 text-ui-sm text-foreground font-mono focus:outline-none focus:border-border-hover transition-colors"
                        />
                      </div>
                    ) : (
                      <div className="space-y-1.5 opacity-60">
                        <label className="text-ui-xs font-medium text-foreground-subtle">{t('webhook.secretToken')}</label>
                        <div className="w-full h-9 rounded-md border border-border/40 bg-surface/30 px-3 flex items-center text-ui-xs text-foreground-subtlest">
                          无需填写密钥
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
                        className="w-full h-9 rounded-md border border-border bg-surface/80 px-3 text-ui-sm text-foreground font-mono focus:outline-none focus:border-border-hover transition-colors"
                      />
                    </div>
                  </div>
                </div>

                {/* 3. 动作类型与执行逻辑 */}
                <div className="space-y-4 rounded-xl border border-border/70 bg-surface/30 p-5">
                  <span className="text-ui-xs font-semibold text-foreground-subtle uppercase tracking-wider flex items-center gap-1.5">
                    <Terminal className="size-4" />
                    <span>动作执行配置</span>
                  </span>

                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <div className="space-y-1.5">
                      <label className="text-ui-xs font-medium text-foreground-subtle">{t('webhook.actionType')}</label>
                      <SelectMenu
                        value={editingRule.actionType}
                        options={actionOptions}
                        onChange={(val) => setEditingRule({ ...editingRule, actionType: val as WebhookActionType })}
                        triggerClassName="h-9 text-ui-sm"
                      />
                    </div>

                    {editingRule.actionType === 'ssh_command' && (
                      <div className="space-y-1.5">
                        <label className="flex items-center gap-1.5 text-ui-xs font-medium text-foreground-subtle">
                          <Server className="size-3.5 text-foreground-subtle" />
                          <span>{t('webhook.targetHost')}</span>
                        </label>
                        <SelectMenu
                          value={editingRule.targetHostId ? String(editingRule.targetHostId) : ''}
                          options={hostOptions}
                          onChange={(val) => setEditingRule({ ...editingRule, targetHostId: val ? Number(val) : null })}
                          triggerClassName="h-9 text-ui-sm"
                        />
                      </div>
                    )}

                    {editingRule.actionType === 'local_command' && (
                      <div className="space-y-1.5">
                        <label className="text-ui-xs font-medium text-foreground-subtle">{t('webhook.cwd')}</label>
                        <input
                          type="text"
                          value={editingRule.cwd || ''}
                          onChange={(e) => setEditingRule({ ...editingRule, cwd: e.target.value || null })}
                          placeholder={t('webhook.cwdPlaceholder')}
                          className="w-full h-9 rounded-md border border-border bg-surface/80 px-3 text-ui-sm text-foreground font-mono focus:outline-none focus:border-border-hover transition-colors"
                        />
                      </div>
                    )}
                  </div>

                  {/* 执行脚本或命令 */}
                  <div className="space-y-2">
                    <label className="text-ui-xs font-medium text-foreground-subtle">
                      {editingRule.actionType === 'js_script' ? t('webhook.jsScript') : t('webhook.command')}
                    </label>
                    <textarea
                      rows={6}
                      value={editingRule.commandText}
                      onChange={(e) => setEditingRule({ ...editingRule, commandText: e.target.value })}
                      placeholder={editingRule.actionType === 'js_script' ? '// console.log(webhook.req.body);' : t('webhook.commandPlaceholder')}
                      className="w-full rounded-lg border border-border bg-surface/90 p-3 text-ui-sm text-foreground font-mono leading-relaxed focus:outline-none focus:border-border-hover transition-colors"
                    />
                    {editingRule.actionType === 'js_script' && (
                      <p className="text-ui-xs text-foreground-subtler leading-relaxed">{t('webhook.jsScriptHint')}</p>
                    )}
                  </div>

                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4 pt-1">
                    <div className="space-y-1.5">
                      <label className="text-ui-xs font-medium text-foreground-subtle">{t('webhook.responseMode')}</label>
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
                        className="w-full h-9 rounded-md border border-border bg-surface/80 px-3 text-ui-sm text-foreground font-mono focus:outline-none focus:border-border-hover transition-colors"
                      />
                    </div>
                  </div>
                </div>

                {/* 4. 模拟测试面板 */}
                <div className="rounded-xl border border-border/70 bg-surface/40 p-5 space-y-4">
                  <div className="flex items-center justify-between">
                    <div className="space-y-0.5">
                      <span className="text-ui-sm font-semibold text-foreground flex items-center gap-2">
                        <Play className="size-4 text-terminal-green" />
                        <span>{t('webhook.testRule')}</span>
                      </span>
                      <p className="text-ui-xs text-foreground-subtle">模拟外部调用此 WebHook 接口，原地执行并观察实际返回</p>
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
                    <label className="text-ui-xs font-medium text-foreground-subtle">模拟请求体 (JSON Body):</label>
                    <textarea
                      rows={3}
                      value={testPayload}
                      onChange={(e) => setTestPayload(e.target.value)}
                      className="w-full rounded-lg border border-border bg-surface/90 p-3 text-ui-xs font-mono text-foreground focus:outline-none focus:border-border-hover transition-colors"
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
                        <span className="text-foreground-subtle font-medium">Exit Code: {testResult.exitCode ?? 'None'}</span>
                        <span className="text-foreground-subtle font-medium">耗时: {testResult.durationMs}ms</span>
                      </div>
                      {testResult.stdout && (
                        <div>
                          <span className="text-[11px] text-foreground-subtler block mb-1">Standard Output:</span>
                          <pre className="rounded bg-background/80 p-3 overflow-x-auto whitespace-pre-wrap text-foreground font-mono leading-relaxed border border-border/50">
                            {testResult.stdout}
                          </pre>
                        </div>
                      )}
                      {testResult.stderr && (
                        <div>
                          <span className="text-[11px] text-destructive block mb-1">Standard Error:</span>
                          <pre className="rounded bg-background/80 p-3 overflow-x-auto whitespace-pre-wrap text-destructive font-mono leading-relaxed border border-destructive/20">
                            {testResult.stderr}
                          </pre>
                        </div>
                      )}
                    </div>
                  )}
                </div>
              </div>
            ) : (
              <div className="flex flex-1 flex-col items-center justify-center text-ui-xs text-foreground-subtle space-y-2">
                <Webhook className="size-10 text-foreground-subtlest" />
                <p className="text-ui-sm font-medium text-foreground">未选择任何 WebHook 规则</p>
                <p className="text-foreground-subtler">在左侧列表中点击选择一条规则，或点击「+ 新建规则」开始配置</p>
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
                {totalLogs} 条记录
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
                onClick={() => setClearingLogsConfirm(true)}
                disabled={logs.length === 0}
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
                            : log.matchStatus}
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
                            <span className="text-ui-xs text-foreground-subtler font-medium">Headers:</span>
                            <pre className="rounded-lg bg-background/80 p-3 text-ui-xs font-mono overflow-x-auto text-foreground-subtle whitespace-pre-wrap border border-border/40">
                              {log.requestHeaders}
                            </pre>
                          </div>
                        )}
                        {log.requestBody && (
                          <div className="space-y-1">
                            <span className="text-ui-xs text-foreground-subtler font-medium">Payload Body:</span>
                            <pre className="rounded-lg bg-background/80 p-3 text-ui-xs font-mono overflow-x-auto text-foreground-subtle whitespace-pre-wrap border border-border/40">
                              {log.requestBody}
                            </pre>
                          </div>
                        )}
                        {(log.stdoutTail || log.stderrTail) && (
                          <div className="space-y-1">
                            <span className="text-ui-xs text-foreground-subtler font-medium">Execution Output:</span>
                            {log.stdoutTail && (
                              <pre className="rounded-lg bg-background/80 p-3 text-ui-xs font-mono overflow-x-auto text-terminal-green whitespace-pre-wrap border border-border/40">
                                {log.stdoutTail}
                              </pre>
                            )}
                            {log.stderrTail && (
                              <pre className="rounded-lg bg-background/80 p-3 text-ui-xs font-mono overflow-x-auto text-destructive whitespace-pre-wrap border border-destructive/20">
                                {log.stderrTail}
                              </pre>
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
        title={t('webhook.clearLogs')}
        description={t('webhook.clearLogsConfirm')}
        onConfirm={() => void handleConfirmClearLogs()}
        onCancel={() => setClearingLogsConfirm(false)}
      />
    </div>
  );
}
