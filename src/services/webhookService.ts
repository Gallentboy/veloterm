/**
 * WebHook 守护服务与动态路由前端接口契约
 */
import { invoke } from '@tauri-apps/api/core';

export interface WebhookSettings {
  enabled: boolean;
  bindIp: string;
  bindPort: number;
  globalSecret?: string | null;
  maxBodyBytes: number;
  updatedAt: number;
}

export type WebhookMethod = 'GET' | 'POST' | 'PUT' | 'DELETE' | 'PATCH' | 'ANY';

export type WebhookSignatureType = 'none' | 'bearer' | 'github_hmac_sha256' | 'gitee_token';

export type WebhookActionType = 'local_command' | 'ssh_command' | 'js_script';

export type WebhookResponseMode = 'async' | 'sync';

export interface WebhookRule {
  id: string;
  name: string;
  enabled: boolean;
  method: WebhookMethod;
  pathPattern: string;
  secretToken?: string | null;
  signatureType: WebhookSignatureType;
  filterExpr?: string | null;
  actionType: WebhookActionType;
  targetHostId?: number | null;
  commandText: string;
  cwd?: string | null;
  timeoutSecs: number;
  responseMode: WebhookResponseMode;
  sortOrder: number;
  createdAt: number;
  updatedAt: number;
}

export interface WebhookLog {
  id: string;
  ruleId?: string | null;
  ruleName?: string | null;
  clientIp: string;
  method: string;
  path: string;
  queryString?: string | null;
  requestHeaders?: string | null;
  requestBody?: string | null;
  matchStatus: 'MATCHED' | 'UNMATCHED' | 'AUTH_FAILED' | 'FILTER_SKIPPED';
  executionStatus?: 'RUNNING' | 'SUCCESS' | 'FAILED' | 'TIMEOUT' | null;
  exitCode?: number | null;
  stdoutTail?: string | null;
  stderrTail?: string | null;
  durationMs?: number | null;
  createdAt: number;
}

export interface WebhookStatus {
  isRunning: boolean;
  bindIp: string;
  bindPort: number;
  activeRulesCount: number;
  error?: string | null;
}

export interface WebhookTestResult {
  success: boolean;
  exitCode?: number | null;
  stdout: string;
  stderr: string;
  durationMs: number;
}

export interface WebhookLogsResponse {
  items: WebhookLog[];
  total: number;
  limit: number;
  offset: number;
}

/** 读取 WebHook 全局设置 */
export async function webhookGetSettings(): Promise<WebhookSettings> {
  return invoke<WebhookSettings>('webhook_get_settings');
}

/** 保存 WebHook 全局设置（并重载 HTTP 守护服务） */
export async function webhookUpdateSettings(settings: WebhookSettings): Promise<void> {
  return invoke<void>('webhook_update_settings', { settings });
}

/** 获取 WebHook 当前运行状态 */
export async function webhookGetStatus(): Promise<WebhookStatus> {
  return invoke<WebhookStatus>('webhook_get_status');
}

/** 获取全部路由规则列表 */
export async function webhookRuleList(): Promise<WebhookRule[]> {
  return invoke<WebhookRule[]>('webhook_rule_list');
}

/** 保存或更新路由规则 */
export async function webhookRuleSave(rule: WebhookRule): Promise<void> {
  return invoke<void>('webhook_rule_save', { rule });
}

/** 删除指定路由规则 */
export async function webhookRuleDelete(id: string): Promise<void> {
  return invoke<void>('webhook_rule_delete', { id });
}

/** 快速启用/禁用指定路由规则 */
export async function webhookRuleToggle(id: string, enabled: boolean): Promise<void> {
  return invoke<void>('webhook_rule_toggle', { id, enabled });
}

/** 模拟测试指定规则的执行 */
export async function webhookRuleTest(
  rule: WebhookRule,
  testContext: {
    method: string;
    path: string;
    clientIp: string;
    pathParams: Record<string, string>;
    queryParams: Record<string, string>;
    headers: Record<string, string>;
    bodyJson?: unknown;
    rawBody: string;
  },
): Promise<WebhookTestResult> {
  return invoke<WebhookTestResult>('webhook_rule_test', { rule, testContext });
}

/** 获取审计日志列表 */
export async function webhookLogsList(
  limit: number,
  offset: number,
  ruleId?: string,
): Promise<WebhookLogsResponse> {
  return invoke<WebhookLogsResponse>('webhook_logs_list', { limit, offset, ruleId });
}

/** 清空全部审计日志 */
export async function webhookLogsClear(): Promise<void> {
  return invoke<void>('webhook_logs_clear');
}

/** 重新触发/重放某条历史审计请求 */
export async function webhookLogReplay(logId: string): Promise<WebhookTestResult & { logId: string }> {
  return invoke<WebhookTestResult & { logId: string }>('webhook_log_replay', { logId });
}
