import {
  AlertCircle,
  Check,
  Copy,
  Lock,
  Play,
  Plus,
  Power,
  RefreshCw,
  Search,
  Settings,
  Trash2,
  Webhook,
} from 'lucide-react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
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

  // 删除规则
  const handleDeleteRule = async (id: string, name: string) => {
    if (!window.confirm(t('webhook.deleteConfirm', { name }))) return;
    try {
      await webhookRuleDelete(id);
      if (selectedRuleId === id) {
        setSelectedRuleId(null);
        setEditingRule(null);
      }
      await refreshData();
    } catch (err) {
      console.error('删除规则失败:', err);
    }
  };

  // 切换规则启用/禁用
  const handleToggleRule = async (id: string, currentEnabled: boolean, e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await webhookRuleToggle(id, !currentEnabled);
      await refreshData();
      if (editingRule && editingRule.id === id) {
        setEditingRule({ ...editingRule, enabled: !currentEnabled });
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
  const handleClearLogs = async () => {
    if (!window.confirm(t('webhook.clearLogsConfirm'))) return;
    try {
      await webhookLogsClear();
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
      <div className="flex h-12 shrink-0 items-center justify-between border-b border-border/70 px-4 bg-surface/30">
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2">
            <Webhook className="size-5 text-terminal-blue" />
            <span className="font-semibold text-ui-base text-foreground">{t('webhook.title')}</span>
          </div>

          <div className="h-4 w-px bg-border/60" />

          {/* 服务运行状态指示灯 */}
          <div className="flex items-center gap-2">
            <span
              className={cn(
                'size-2.5 rounded-full transition-colors',
                status?.isRunning ? 'bg-terminal-green shadow-xs shadow-terminal-green/50 animate-pulse' : 'bg-foreground-subtlest',
              )}
            />
            <span className="text-ui-xs font-mono">
              {status?.isRunning
                ? `${t('webhook.serverRunning')} (${status.bindIp}:${status.bindPort})`
                : t('webhook.serverStopped')}
            </span>
            {status?.error && (
              <span className="flex items-center gap-1 text-[11px] text-destructive max-w-xs truncate" title={status.error}>
                <AlertCircle className="size-3 shrink-0" />
                <span>{status.error}</span>
              </span>
            )}
          </div>
        </div>

        <div className="flex items-center gap-2">
          {/* 服务开关按钮 */}
          <Button
            type="button"
            variant={settings?.enabled ? 'default' : 'outline'}
            size="sm"
            onClick={handleToggleService}
            className="h-7.5 gap-1.5 px-3 text-ui-xs"
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
            className="h-7.5 gap-1.5 px-3 text-ui-xs"
          >
            <Settings className="size-3.5 text-foreground-subtle" />
            <span>{t('webhook.globalSettings')}</span>
          </Button>

          {/* Tab 切换 */}
          <div className="flex items-center rounded-lg border border-border/80 bg-surface/60 p-0.5 text-ui-xs">
            <button
              type="button"
              onClick={() => setActiveTab('rules')}
              className={cn(
                'rounded-md px-2.5 py-1 font-medium transition-colors',
                activeTab === 'rules' ? 'bg-selected text-foreground shadow-xs' : 'text-foreground-subtle hover:text-foreground',
              )}
            >
              {t('webhook.rulesTab')} ({rules.length})
            </button>
            <button
              type="button"
              onClick={() => setActiveTab('logs')}
              className={cn(
                'rounded-md px-2.5 py-1 font-medium transition-colors',
                activeTab === 'logs' ? 'bg-selected text-foreground shadow-xs' : 'text-foreground-subtle hover:text-foreground',
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
          {/* 左侧：规则列表 */}
          <div className="flex w-80 sm:w-88 shrink-0 flex-col border-r border-border/70 bg-surface/20">
            {/* 搜索与新建 */}
            <div className="p-2.5 border-b border-border/50 space-y-2">
              <div className="flex items-center gap-1.5">
                <div className="relative flex-1">
                  <Search className="pointer-events-none absolute left-2.5 top-2 size-3.5 text-foreground-subtlest" />
                  <input
                    type="text"
                    value={ruleSearch}
                    onChange={(e) => setRuleSearch(e.target.value)}
                    placeholder={t('webhook.searchPlaceholder')}
                    className="h-7.5 w-full rounded-md border border-border/60 bg-surface/60 pl-8 pr-2.5 text-ui-xs text-foreground placeholder:text-foreground-subtlest focus:outline-none focus:ring-1 focus:ring-border"
                  />
                </div>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={handleCreateNewRule}
                  className="h-7.5 shrink-0 gap-1 px-2.5 text-ui-xs font-normal"
                >
                  <Plus className="size-3.5" />
                  <span>{t('webhook.newRule')}</span>
                </Button>
              </div>
            </div>

            {/* 规则条目列表 */}
            <div className="flex-1 overflow-y-auto p-2 space-y-1.5">
              {filteredRules.length === 0 ? (
                <div className="flex flex-col items-center justify-center p-8 text-center text-ui-xs text-foreground-subtle">
                  <Webhook className="size-8 text-foreground-subtlest mb-2" />
                  <p className="font-medium text-foreground">{t('webhook.emptyRules')}</p>
                  <p className="mt-1 max-w-xs text-foreground-subtlest">{t('webhook.emptyRulesDesc')}</p>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={handleCreateNewRule}
                    className="mt-3 text-ui-xs"
                  >
                    <Plus className="size-3 mr-1" />
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
                        'group flex flex-col gap-1.5 rounded-lg border p-2.5 cursor-pointer transition-all',
                        isSelected
                          ? 'border-border bg-selected/80 shadow-xs'
                          : 'border-border/50 bg-surface/40 hover:border-border/80 hover:bg-hover',
                      )}
                    >
                      <div className="flex items-center justify-between gap-1.5">
                        <div className="flex items-center gap-1.5 min-w-0">
                          <span
                            className={cn(
                              'px-1.5 py-0.5 rounded text-[10px] font-mono font-bold border uppercase',
                              METHOD_COLORS[r.method] || METHOD_COLORS.ANY,
                            )}
                          >
                            {r.method}
                          </span>
                          <span className="font-medium text-ui-xs text-foreground truncate">{r.name}</span>
                        </div>
                        <input
                          type="checkbox"
                          checked={r.enabled}
                          onClick={(e) => handleToggleRule(r.id, r.enabled, e)}
                          title={r.enabled ? '已启用' : '已停用'}
                          className="size-3.5 rounded border-border accent-terminal-blue cursor-pointer"
                        />
                      </div>

                      <div className="flex items-center justify-between text-[11px] text-foreground-subtle font-mono">
                        <span className="truncate" title={r.pathPattern}>{r.pathPattern}</span>
                        <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                          <button
                            type="button"
                            onClick={(e) => handleCopyCurl(r, e)}
                            className="p-1 rounded hover:bg-surface text-foreground-subtle hover:text-foreground"
                            title={t('webhook.copyCurl')}
                          >
                            {copiedId === r.id ? <Check className="size-3 text-terminal-green" /> : <Copy className="size-3" />}
                          </button>
                          <button
                            type="button"
                            onClick={(e) => {
                              e.stopPropagation();
                              void handleDeleteRule(r.id, r.name);
                            }}
                            className="p-1 rounded hover:bg-destructive/10 text-foreground-subtle hover:text-destructive"
                            title={t('common.delete', { defaultValue: '删除' })}
                          >
                            <Trash2 className="size-3" />
                          </button>
                        </div>
                      </div>
                    </div>
                  );
                })
              )}
            </div>
          </div>

          {/* 右侧：规则编辑与测试面板 */}
          <div className="flex flex-1 min-w-0 flex-col overflow-y-auto p-4 space-y-4 bg-background">
            {editingRule ? (
              <div className="max-w-3xl space-y-4">
                <div className="flex items-center justify-between border-b border-border/60 pb-3">
                  <div>
                    <h2 className="text-ui-base font-semibold text-foreground">{editingRule.name}</h2>
                    <p className="text-ui-xs text-foreground-subtle font-mono">{editingRule.pathPattern}</p>
                  </div>
                  <div className="flex items-center gap-2">
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => handleCopyCurl(editingRule)}
                      className="gap-1.5 text-ui-xs"
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
                      className="gap-1.5 text-ui-xs"
                    >
                      {savingRule && <Spinner className="size-3.5" />}
                      <span>{t('webhook.saveRule')}</span>
                    </Button>
                  </div>
                </div>

                {/* 基本匹配条件 */}
                <div className="grid grid-cols-3 gap-3">
                  <div className="col-span-2 space-y-1.5">
                    <label className="text-ui-xs font-medium text-foreground">{t('webhook.ruleName')}</label>
                    <input
                      type="text"
                      value={editingRule.name}
                      onChange={(e) => setEditingRule({ ...editingRule, name: e.target.value })}
                      placeholder={t('webhook.ruleNamePlaceholder')}
                      className="w-full h-8 rounded-md border border-border/80 bg-surface px-2.5 text-ui-xs text-foreground focus:outline-none focus:ring-1 focus:ring-border"
                    />
                  </div>

                  <div className="space-y-1.5">
                    <label className="text-ui-xs font-medium text-foreground">{t('webhook.method')}</label>
                    <select
                      value={editingRule.method}
                      onChange={(e) => setEditingRule({ ...editingRule, method: e.target.value as WebhookMethod })}
                      className="w-full h-8 rounded-md border border-border/80 bg-surface px-2 text-ui-xs text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-border"
                    >
                      <option value="POST">POST</option>
                      <option value="GET">GET</option>
                      <option value="PUT">PUT</option>
                      <option value="DELETE">DELETE</option>
                      <option value="PATCH">PATCH</option>
                      <option value="ANY">ANY (任意方法)</option>
                    </select>
                  </div>
                </div>

                <div className="space-y-1.5">
                  <label className="text-ui-xs font-medium text-foreground">{t('webhook.pathPattern')}</label>
                  <input
                    type="text"
                    value={editingRule.pathPattern}
                    onChange={(e) => setEditingRule({ ...editingRule, pathPattern: e.target.value })}
                    placeholder={t('webhook.pathPatternPlaceholder')}
                    className="w-full h-8 rounded-md border border-border/80 bg-surface px-2.5 text-ui-xs text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-border"
                  />
                  <p className="text-[11px] text-foreground-subtler">{t('webhook.pathPatternHint')}</p>
                </div>

                {/* 签名认证与过滤 */}
                <div className="grid grid-cols-2 gap-3 p-3 rounded-lg border border-border/70 bg-surface/30">
                  <div className="space-y-1.5">
                    <label className="flex items-center gap-1.5 text-ui-xs font-medium text-foreground">
                      <Lock className="size-3.5 text-foreground-subtle" />
                      <span>{t('webhook.signatureType')}</span>
                    </label>
                    <select
                      value={editingRule.signatureType}
                      onChange={(e) => setEditingRule({ ...editingRule, signatureType: e.target.value as WebhookSignatureType })}
                      className="w-full h-8 rounded-md border border-border/80 bg-surface px-2 text-ui-xs text-foreground focus:outline-none focus:ring-1 focus:ring-border"
                    >
                      <option value="none">{t('webhook.sigNone')}</option>
                      <option value="bearer">{t('webhook.sigBearer')}</option>
                      <option value="github_hmac_sha256">{t('webhook.sigGithub')}</option>
                      <option value="gitee_token">{t('webhook.sigGitee')}</option>
                    </select>
                  </div>

                  {editingRule.signatureType !== 'none' && (
                    <div className="space-y-1.5">
                      <label className="text-ui-xs font-medium text-foreground">{t('webhook.secretToken')}</label>
                      <input
                        type="password"
                        value={editingRule.secretToken || ''}
                        onChange={(e) => setEditingRule({ ...editingRule, secretToken: e.target.value || null })}
                        placeholder={t('webhook.secretTokenPlaceholder')}
                        className="w-full h-8 rounded-md border border-border/80 bg-surface px-2.5 text-ui-xs text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-border"
                      />
                    </div>
                  )}

                  <div className="col-span-2 space-y-1.5">
                    <label className="text-ui-xs font-medium text-foreground">{t('webhook.filterExpr')}</label>
                    <input
                      type="text"
                      value={editingRule.filterExpr || ''}
                      onChange={(e) => setEditingRule({ ...editingRule, filterExpr: e.target.value || null })}
                      placeholder={t('webhook.filterExprPlaceholder')}
                      className="w-full h-8 rounded-md border border-border/80 bg-surface px-2.5 text-ui-xs text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-border"
                    />
                  </div>
                </div>

                {/* 动作类型配置 */}
                <div className="space-y-3 p-3 rounded-lg border border-border/70 bg-surface/30">
                  <div className="grid grid-cols-2 gap-3">
                    <div className="space-y-1.5">
                      <label className="text-ui-xs font-medium text-foreground">{t('webhook.actionType')}</label>
                      <select
                        value={editingRule.actionType}
                        onChange={(e) => setEditingRule({ ...editingRule, actionType: e.target.value as WebhookActionType })}
                        className="w-full h-8 rounded-md border border-border/80 bg-surface px-2 text-ui-xs text-foreground focus:outline-none focus:ring-1 focus:ring-border"
                      >
                        <option value="local_command">{t('webhook.actionLocal')}</option>
                        <option value="ssh_command">{t('webhook.actionSsh')}</option>
                        <option value="js_script">{t('webhook.actionJs')}</option>
                      </select>
                    </div>

                    {editingRule.actionType === 'ssh_command' && (
                      <div className="space-y-1.5">
                        <label className="text-ui-xs font-medium text-foreground">{t('webhook.targetHost')}</label>
                        <select
                          value={editingRule.targetHostId || ''}
                          onChange={(e) => setEditingRule({ ...editingRule, targetHostId: Number(e.target.value) || null })}
                          className="w-full h-8 rounded-md border border-border/80 bg-surface px-2 text-ui-xs text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-border"
                        >
                          <option value="">{t('webhook.selectHost')}</option>
                          {hosts.map((h) => (
                            <option key={h.id} value={h.id}>
                              {h.name} ({h.username}@{h.host}:{h.port})
                            </option>
                          ))}
                        </select>
                      </div>
                    )}

                    {editingRule.actionType === 'local_command' && (
                      <div className="space-y-1.5">
                        <label className="text-ui-xs font-medium text-foreground">{t('webhook.cwd')}</label>
                        <input
                          type="text"
                          value={editingRule.cwd || ''}
                          onChange={(e) => setEditingRule({ ...editingRule, cwd: e.target.value || null })}
                          placeholder={t('webhook.cwdPlaceholder')}
                          className="w-full h-8 rounded-md border border-border/80 bg-surface px-2.5 text-ui-xs text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-border"
                        />
                      </div>
                    )}
                  </div>

                  {/* 执行内容 */}
                  <div className="space-y-1.5">
                    <label className="text-ui-xs font-medium text-foreground">
                      {editingRule.actionType === 'js_script' ? t('webhook.jsScript') : t('webhook.command')}
                    </label>
                    <textarea
                      rows={6}
                      value={editingRule.commandText}
                      onChange={(e) => setEditingRule({ ...editingRule, commandText: e.target.value })}
                      placeholder={editingRule.actionType === 'js_script' ? '// console.log(webhook.req.body);' : t('webhook.commandPlaceholder')}
                      className="w-full rounded-md border border-border/80 bg-surface p-2.5 text-ui-xs text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-border"
                    />
                    {editingRule.actionType === 'js_script' && (
                      <p className="text-[11px] text-foreground-subtler">{t('webhook.jsScriptHint')}</p>
                    )}
                  </div>

                  <div className="grid grid-cols-2 gap-3">
                    <div className="space-y-1.5">
                      <label className="text-ui-xs font-medium text-foreground">{t('webhook.responseMode')}</label>
                      <select
                        value={editingRule.responseMode}
                        onChange={(e) => setEditingRule({ ...editingRule, responseMode: e.target.value as 'async' | 'sync' })}
                        className="w-full h-8 rounded-md border border-border/80 bg-surface px-2 text-ui-xs text-foreground focus:outline-none focus:ring-1 focus:ring-border"
                      >
                        <option value="async">{t('webhook.responseAsync')}</option>
                        <option value="sync">{t('webhook.responseSync')}</option>
                      </select>
                    </div>

                    <div className="space-y-1.5">
                      <label className="text-ui-xs font-medium text-foreground">{t('webhook.timeout')}</label>
                      <input
                        type="number"
                        min={1}
                        max={600}
                        value={editingRule.timeoutSecs}
                        onChange={(e) => setEditingRule({ ...editingRule, timeoutSecs: Number(e.target.value) || 60 })}
                        className="w-full h-8 rounded-md border border-border/80 bg-surface px-2.5 text-ui-xs text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-border"
                      />
                    </div>
                  </div>
                </div>

                {/* ── 模拟测试面板 ── */}
                <div className="rounded-lg border border-border/70 bg-surface/30 p-3 space-y-3">
                  <div className="flex items-center justify-between">
                    <span className="text-ui-xs font-semibold text-foreground flex items-center gap-1.5">
                      <Play className="size-3.5 text-terminal-green" />
                      <span>{t('webhook.testRule')}</span>
                    </span>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={handleTestRule}
                      disabled={testing}
                      className="h-7 gap-1.5 px-2.5 text-ui-xs"
                    >
                      {testing ? <Spinner className="size-3" /> : <Play className="size-3 text-terminal-green" />}
                      <span>{testing ? t('webhook.testing') : t('webhook.testRule')}</span>
                    </Button>
                  </div>

                  <div className="space-y-1">
                    <label className="text-[11px] text-foreground-subtle">模拟测试 JSON Body:</label>
                    <textarea
                      rows={3}
                      value={testPayload}
                      onChange={(e) => setTestPayload(e.target.value)}
                      className="w-full rounded-md border border-border/70 bg-surface p-2 text-ui-xs font-mono text-foreground focus:outline-none focus:ring-1 focus:ring-border"
                    />
                  </div>

                  {testResult && (
                    <div className="rounded border border-border/60 bg-surface/80 p-2.5 space-y-1.5 text-ui-xs font-mono">
                      <div className="flex items-center gap-2">
                        <span
                          className={cn(
                            'px-1.5 py-0.5 rounded text-[10px] font-bold uppercase',
                            testResult.success ? 'bg-terminal-green/20 text-terminal-green' : 'bg-destructive/20 text-destructive',
                          )}
                        >
                          {testResult.success ? t('webhook.execSuccess') : t('webhook.execFailed')}
                        </span>
                        <span className="text-foreground-subtle">Exit Code: {testResult.exitCode ?? 'None'}</span>
                        <span className="text-foreground-subtle">耗时: {testResult.durationMs}ms</span>
                      </div>
                      {testResult.stdout && (
                        <div>
                          <span className="text-[10px] text-foreground-subtler">stdout:</span>
                          <pre className="mt-0.5 overflow-x-auto whitespace-pre-wrap text-foreground">{testResult.stdout}</pre>
                        </div>
                      )}
                      {testResult.stderr && (
                        <div>
                          <span className="text-[10px] text-destructive">stderr:</span>
                          <pre className="mt-0.5 overflow-x-auto whitespace-pre-wrap text-destructive">{testResult.stderr}</pre>
                        </div>
                      )}
                    </div>
                  )}
                </div>
              </div>
            ) : (
              <div className="flex flex-1 flex-col items-center justify-center text-ui-xs text-foreground-subtler">
                <p>请选择或新建一条 WebHook 规则进行配置</p>
              </div>
            )}
          </div>
        </div>
      ) : (
        /* ── 审计日志视图 ── */
        <div className="flex flex-1 min-h-0 w-full flex-col overflow-hidden p-4">
          <div className="flex items-center justify-between pb-3 border-b border-border/60">
            <div className="flex items-center gap-2">
              <span className="text-ui-sm font-semibold text-foreground">{t('webhook.logsTitle')}</span>
              <span className="text-ui-xs text-foreground-subtle">({totalLogs})</span>
            </div>
            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={refreshLogs}
                disabled={logsLoading}
                className="h-7.5 gap-1.5 px-2.5 text-ui-xs"
              >
                <RefreshCw className={cn('size-3.5', logsLoading && 'animate-spin')} />
                <span>{t('common.refresh', { defaultValue: '刷新' })}</span>
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={handleClearLogs}
                disabled={logs.length === 0}
                className="h-7.5 gap-1.5 px-2.5 text-ui-xs text-destructive hover:bg-destructive/10"
              >
                <Trash2 className="size-3.5" />
                <span>{t('webhook.clearLogs')}</span>
              </Button>
            </div>
          </div>

          <div className="flex-1 overflow-y-auto pt-2 space-y-2">
            {logs.length === 0 ? (
              <div className="flex flex-col items-center justify-center p-12 text-center text-ui-xs text-foreground-subtle">
                <p className="font-medium text-foreground">{t('webhook.emptyLogs')}</p>
                <p className="mt-1 text-foreground-subtler">{t('webhook.emptyLogsDesc')}</p>
              </div>
            ) : (
              logs.map((log) => {
                const isExpanded = selectedLogId === log.id;
                return (
                  <div
                    key={log.id}
                    className="flex flex-col rounded-lg border border-border/60 bg-surface/30 overflow-hidden text-ui-xs font-mono"
                  >
                    <div
                      onClick={() => setSelectedLogId(isExpanded ? null : log.id)}
                      className="flex items-center justify-between p-2.5 cursor-pointer hover:bg-hover/60 transition-colors"
                    >
                      <div className="flex items-center gap-2 min-w-0">
                        <span
                          className={cn(
                            'px-1.5 py-0.5 rounded text-[10px] font-bold border uppercase',
                            METHOD_COLORS[log.method] || METHOD_COLORS.ANY,
                          )}
                        >
                          {log.method}
                        </span>
                        <span className="font-medium text-foreground truncate">{log.path}</span>
                        {log.ruleName && (
                          <span className="text-[11px] text-foreground-subtle truncate">
                            · {log.ruleName}
                          </span>
                        )}
                      </div>

                      <div className="flex items-center gap-3 shrink-0">
                        <span
                          className={cn(
                            'px-1.5 py-0.5 rounded text-[10px] font-semibold',
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
                        <span className="text-[11px] text-foreground-subtlest">{log.clientIp}</span>
                        <span className="text-[11px] text-foreground-subtlest">{formatTime(log.createdAt)}</span>
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          onClick={(e) => {
                            e.stopPropagation();
                            void handleReplayLog(log.id);
                          }}
                          disabled={replayingId === log.id || !log.ruleId}
                          className="h-6 gap-1 px-1.5 text-[11px] text-terminal-blue hover:bg-terminal-blue/10"
                        >
                          {replayingId === log.id ? <Spinner className="size-3" /> : <Play className="size-3" />}
                          <span>{t('webhook.replay')}</span>
                        </Button>
                      </div>
                    </div>

                    {isExpanded && (
                      <div className="border-t border-border/50 p-3 bg-surface/60 space-y-2">
                        {log.requestHeaders && (
                          <div>
                            <span className="text-[10px] text-foreground-subtlest font-sans font-medium">Headers:</span>
                            <pre className="mt-0.5 rounded bg-background p-2 text-[11px] overflow-x-auto text-foreground-subtle whitespace-pre-wrap">
                              {log.requestHeaders}
                            </pre>
                          </div>
                        )}
                        {log.requestBody && (
                          <div>
                            <span className="text-[10px] text-foreground-subtlest font-sans font-medium">Body:</span>
                            <pre className="mt-0.5 rounded bg-background p-2 text-[11px] overflow-x-auto text-foreground-subtle whitespace-pre-wrap">
                              {log.requestBody}
                            </pre>
                          </div>
                        )}
                        {(log.stdoutTail || log.stderrTail) && (
                          <div>
                            <span className="text-[10px] text-foreground-subtlest font-sans font-medium">Execution Output:</span>
                            {log.stdoutTail && (
                              <pre className="mt-0.5 rounded bg-background p-2 text-[11px] overflow-x-auto text-terminal-green whitespace-pre-wrap">
                                {log.stdoutTail}
                              </pre>
                            )}
                            {log.stderrTail && (
                              <pre className="mt-0.5 rounded bg-background p-2 text-[11px] overflow-x-auto text-destructive whitespace-pre-wrap">
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
    </div>
  );
}
