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

/**
 * WebHook 运行时注入的单次请求对象 (webhook.req)
 *
 * 在 JS / TS 脚本动作中，全局变量 `webhook` 将作为只读对象注入运行时：
 * ```ts
 * const { method, path, bodyJson, queryParams } = webhook.req;
 * console.log(`Received ${method} request to ${path}`);
 * ```
 */
export interface WebhookRequest<TBody = Record<string, unknown>> {
  /** HTTP 请求方法，全大写（如 'GET', 'POST', 'PUT', 'DELETE'） */
  readonly method: string;
  /** 请求的 URI 路径（如 '/deploy/prod'） */
  readonly path: string;
  /** 客户端远端 IP 地址（如 '127.0.0.1'） */
  readonly clientIp: string;
  /** 路径中提取的动态命名参数（如路由定义为 `/deploy/:service/:env` 时，`pathParams.service`, `pathParams.env`） */
  readonly pathParams: Readonly<Record<string, string>>;
  /** URL Query 查询参数键值对映射（如 `?token=abc&debug=1`） */
  readonly queryParams: Readonly<Record<string, string>>;
  /** 全部 HTTP 请求头映射（小写键名，如 `headers['content-type']`） */
  readonly headers: Readonly<Record<string, string>>;
  /** 自动解析后的 JSON 请求体对象；若请求体非 JSON 或为空则为 null */
  readonly bodyJson: Readonly<TBody> | null;
  /** 原始请求体字符串（Text / Binary 字符流） */
  readonly rawBody: string;
}

/** 全局注入的 webhook 根容器类型 */
export interface WebhookGlobalContext<TBody = Record<string, unknown>> {
  readonly req: WebhookRequest<TBody>;
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
