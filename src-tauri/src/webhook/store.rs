//! WebHook SQLCipher 持久化：全局设置、路由规则与审计日志。

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::now_timestamp_ms;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookSettings {
    pub enabled: bool,
    pub bind_ip: String,
    pub bind_port: u16,
    pub global_secret: Option<String>,
    pub max_body_bytes: usize,
    pub updated_at: i64,
}

impl Default for WebhookSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_ip: "127.0.0.1".to_string(),
            bind_port: 9876,
            global_secret: None,
            max_body_bytes: 2 * 1024 * 1024, // 2MB
            updated_at: now_timestamp_ms(),
        }
    }
}

/// 链式执行中的单个动作步骤
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookActionStep {
    pub id: String,
    pub name: Option<String>,
    pub action_type: String,         // local_command, ssh_command, js_script
    pub target_host_id: Option<i64>, // SSH 目标主机
    pub command_text: String,        // 待执行的命令文本或 JS 源码
    pub cwd: Option<String>,         // 本地命令工作目录
    pub timeout_secs: Option<u64>,   // 可选步骤超时
    pub continue_on_error: Option<bool>, // 失败时是否继续流水线
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub method: String,              // GET, POST, PUT, DELETE, PATCH, ANY
    pub path_pattern: String,        // 如 /api/deploy/:service
    pub secret_token: Option<String>,
    pub signature_type: String,      // none, bearer, github_hmac_sha256, gitee_token
    pub filter_expr: Option<String>, // JSON 字段过滤 (如 "body.ref == \"refs/heads/main\"")
    pub action_type: String,         // local_command, ssh_command, js_script
    pub target_host_id: Option<i64>, // SSH 目标主机
    pub command_text: String,        // 待执行的命令文本或 JS 源码
    pub cwd: Option<String>,         // 本地命令工作目录
    pub timeout_secs: u64,           // 超时秒数，默认 60
    pub response_mode: String,       // async (202), sync (200)
    pub sort_order: i32,
    pub actions_json: Option<String>,// 链式多步骤 JSON 序列化定义
    pub created_at: i64,
    pub updated_at: i64,
}

impl WebhookRule {
    /// 获取完整的链式执行步骤列表（若配置了 actions_json 则返回其列表，否则由原有单动作平滑过渡为单步列表）
    pub fn get_action_steps(&self) -> Vec<WebhookActionStep> {
        if let Some(ref json) = self.actions_json {
            if let Ok(steps) = serde_json::from_str::<Vec<WebhookActionStep>>(json) {
                if !steps.is_empty() {
                    return steps;
                }
            }
        }

        vec![WebhookActionStep {
            id: format!("step_1_{}", self.id),
            name: Some("主要动作".to_string()),
            action_type: self.action_type.clone(),
            target_host_id: self.target_host_id,
            command_text: self.command_text.clone(),
            cwd: self.cwd.clone(),
            timeout_secs: Some(self.timeout_secs),
            continue_on_error: Some(false),
        }]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookLog {
    pub id: String,
    pub rule_id: Option<String>,
    pub rule_name: Option<String>,
    pub client_ip: String,
    pub method: String,
    pub path: String,
    pub query_string: Option<String>,
    pub request_headers: Option<String>, // JSON 序列化
    pub request_body: Option<String>,
    pub match_status: String,            // MATCHED, UNMATCHED, AUTH_FAILED
    pub execution_status: Option<String>,// PENDING, RUNNING, SUCCESS, FAILED, TIMEOUT
    pub exit_code: Option<i32>,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    pub duration_ms: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookStatus {
    pub is_running: bool,
    pub bind_ip: String,
    pub bind_port: u16,
    pub active_rules_count: usize,
    pub error: Option<String>,
}

/// 幂等建表
pub fn ensure_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS webhook_settings (
            id             INTEGER PRIMARY KEY CHECK (id = 1),
            enabled        INTEGER NOT NULL DEFAULT 0,
            bind_ip        TEXT NOT NULL DEFAULT '127.0.0.1',
            bind_port      INTEGER NOT NULL DEFAULT 9876,
            global_secret  TEXT,
            max_body_bytes INTEGER NOT NULL DEFAULT 2097152,
            updated_at     INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS webhook_rules (
            id             TEXT PRIMARY KEY,
            name           TEXT NOT NULL,
            enabled        INTEGER NOT NULL DEFAULT 1,
            method         TEXT NOT NULL,
            path_pattern   TEXT NOT NULL,
            secret_token   TEXT,
            signature_type TEXT NOT NULL DEFAULT 'none',
            filter_expr    TEXT,
            action_type    TEXT NOT NULL,
            target_host_id INTEGER,
            command_text   TEXT NOT NULL,
            cwd            TEXT,
            timeout_secs   INTEGER NOT NULL DEFAULT 60,
            response_mode  TEXT NOT NULL DEFAULT 'async',
            sort_order     INTEGER NOT NULL DEFAULT 0,
            created_at     INTEGER NOT NULL,
            updated_at     INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_webhook_rules_sort
            ON webhook_rules(sort_order ASC, created_at ASC);

        CREATE TABLE IF NOT EXISTS webhook_logs (
            id               TEXT PRIMARY KEY,
            rule_id          TEXT,
            rule_name        TEXT,
            client_ip        TEXT NOT NULL,
            method           TEXT NOT NULL,
            path             TEXT NOT NULL,
            query_string     TEXT,
            request_headers  TEXT,
            request_body     TEXT,
            match_status     TEXT NOT NULL,
            execution_status TEXT,
            exit_code        INTEGER,
            stdout_tail      TEXT,
            stderr_tail      TEXT,
            duration_ms      INTEGER,
            created_at       INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_webhook_logs_created_at
            ON webhook_logs(created_at DESC);",
    )
    .map_err(|e| format!("初始化 WebHook 数据表失败: {e}"))?;

    // 幂等增加 actions_json 列（支持多步骤链式流水线执行）
    let _ = conn.execute("ALTER TABLE webhook_rules ADD COLUMN actions_json TEXT", []);

    // 默认插入一条 id=1 的全局设置（如果不存在）
    conn.execute(
        "INSERT OR IGNORE INTO webhook_settings (id, enabled, bind_ip, bind_port, max_body_bytes, updated_at)
         VALUES (1, 0, '127.0.0.1', 9876, 2097152, ?1)",
        params![now_timestamp_ms()],
    )
    .map_err(|e| format!("初始化 WebHook 全局设置记录失败: {e}"))?;

    Ok(())
}

pub fn get_settings(conn: &Connection) -> Result<WebhookSettings, String> {
    let mut stmt = conn
        .prepare(
            "SELECT enabled, bind_ip, bind_port, global_secret, max_body_bytes, updated_at
             FROM webhook_settings WHERE id = 1",
        )
        .map_err(|e| e.to_string())?;

    let row = stmt
        .query_row([], |r| {
            let enabled_int: i32 = r.get(0)?;
            Ok(WebhookSettings {
                enabled: enabled_int != 0,
                bind_ip: r.get(1)?,
                bind_port: r.get(2)?,
                global_secret: r.get(3)?,
                max_body_bytes: r.get(4)?,
                updated_at: r.get(5)?,
            })
        })
        .optional()
        .map_err(|e| e.to_string())?;

    Ok(row.unwrap_or_default())
}

pub fn update_settings(conn: &Connection, settings: &WebhookSettings) -> Result<(), String> {
    let now = now_timestamp_ms();
    conn.execute(
        "INSERT INTO webhook_settings (id, enabled, bind_ip, bind_port, global_secret, max_body_bytes, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
            enabled = excluded.enabled,
            bind_ip = excluded.bind_ip,
            bind_port = excluded.bind_port,
            global_secret = excluded.global_secret,
            max_body_bytes = excluded.max_body_bytes,
            updated_at = excluded.updated_at",
        params![
            if settings.enabled { 1 } else { 0 },
            settings.bind_ip,
            settings.bind_port,
            settings.global_secret,
            settings.max_body_bytes,
            now
        ],
    )
    .map_err(|e| format!("保存 WebHook 全局设置失败: {e}"))?;

    Ok(())
}

pub fn list_rules(conn: &Connection) -> Result<Vec<WebhookRule>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, enabled, method, path_pattern, secret_token, signature_type,
                    filter_expr, action_type, target_host_id, command_text, cwd,
                    timeout_secs, response_mode, sort_order, created_at, updated_at, actions_json
             FROM webhook_rules
             ORDER BY sort_order ASC, created_at ASC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |r| {
            let enabled_int: i32 = r.get(2)?;
            let timeout: i64 = r.get(12)?;
            Ok(WebhookRule {
                id: r.get(0)?,
                name: r.get(1)?,
                enabled: enabled_int != 0,
                method: r.get(3)?,
                path_pattern: r.get(4)?,
                secret_token: r.get(5)?,
                signature_type: r.get(6)?,
                filter_expr: r.get(7)?,
                action_type: r.get(8)?,
                target_host_id: r.get(9)?,
                command_text: r.get(10)?,
                cwd: r.get(11)?,
                timeout_secs: timeout.max(1) as u64,
                response_mode: r.get(13)?,
                sort_order: r.get(14)?,
                created_at: r.get(15)?,
                updated_at: r.get(16)?,
                actions_json: r.get(17)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut list = Vec::new();
    for item in rows {
        list.push(item.map_err(|e| e.to_string())?);
    }
    Ok(list)
}

pub fn get_rule(conn: &Connection, id: &str) -> Result<Option<WebhookRule>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, enabled, method, path_pattern, secret_token, signature_type,
                    filter_expr, action_type, target_host_id, command_text, cwd,
                    timeout_secs, response_mode, sort_order, created_at, updated_at, actions_json
             FROM webhook_rules
             WHERE id = ?1",
        )
        .map_err(|e| e.to_string())?;

    let rule = stmt
        .query_row(params![id], |r| {
            let enabled_int: i32 = r.get(2)?;
            let timeout: i64 = r.get(12)?;
            Ok(WebhookRule {
                id: r.get(0)?,
                name: r.get(1)?,
                enabled: enabled_int != 0,
                method: r.get(3)?,
                path_pattern: r.get(4)?,
                secret_token: r.get(5)?,
                signature_type: r.get(6)?,
                filter_expr: r.get(7)?,
                action_type: r.get(8)?,
                target_host_id: r.get(9)?,
                command_text: r.get(10)?,
                cwd: r.get(11)?,
                timeout_secs: timeout.max(1) as u64,
                response_mode: r.get(13)?,
                sort_order: r.get(14)?,
                created_at: r.get(15)?,
                updated_at: r.get(16)?,
                actions_json: r.get(17)?,
            })
        })
        .optional()
        .map_err(|e| e.to_string())?;

    Ok(rule)
}

pub fn save_rule(conn: &Connection, rule: &WebhookRule) -> Result<(), String> {
    let now = now_timestamp_ms();
    conn.execute(
        "INSERT INTO webhook_rules (
            id, name, enabled, method, path_pattern, secret_token, signature_type,
            filter_expr, action_type, target_host_id, command_text, cwd,
            timeout_secs, response_mode, sort_order, created_at, updated_at, actions_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            enabled = excluded.enabled,
            method = excluded.method,
            path_pattern = excluded.path_pattern,
            secret_token = excluded.secret_token,
            signature_type = excluded.signature_type,
            filter_expr = excluded.filter_expr,
            action_type = excluded.action_type,
            target_host_id = excluded.target_host_id,
            command_text = excluded.command_text,
            cwd = excluded.cwd,
            timeout_secs = excluded.timeout_secs,
            response_mode = excluded.response_mode,
            sort_order = excluded.sort_order,
            updated_at = excluded.updated_at,
            actions_json = excluded.actions_json",
        params![
            rule.id,
            rule.name,
            if rule.enabled { 1 } else { 0 },
            rule.method.to_uppercase(),
            rule.path_pattern,
            rule.secret_token,
            rule.signature_type,
            rule.filter_expr,
            rule.action_type,
            rule.target_host_id,
            rule.command_text,
            rule.cwd,
            rule.timeout_secs as i64,
            rule.response_mode,
            rule.sort_order,
            if rule.created_at == 0 { now } else { rule.created_at },
            now,
            rule.actions_json,
        ],
    )
    .map_err(|e| format!("保存 WebHook 规则失败: {e}"))?;

    Ok(())
}

pub fn delete_rule(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM webhook_rules WHERE id = ?1", params![id])
        .map_err(|e| format!("删除 WebHook 规则失败: {e}"))?;
    Ok(())
}

pub fn toggle_rule(conn: &Connection, id: &str, enabled: bool) -> Result<(), String> {
    let now = now_timestamp_ms();
    conn.execute(
        "UPDATE webhook_rules SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
        params![if enabled { 1 } else { 0 }, now, id],
    )
    .map_err(|e| format!("切换 WebHook 规则状态失败: {e}"))?;
    Ok(())
}

pub fn insert_log(conn: &Connection, log: &WebhookLog) -> Result<(), String> {
    conn.execute(
        "INSERT INTO webhook_logs (
            id, rule_id, rule_name, client_ip, method, path, query_string,
            request_headers, request_body, match_status, execution_status,
            exit_code, stdout_tail, stderr_tail, duration_ms, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            log.id,
            log.rule_id,
            log.rule_name,
            log.client_ip,
            log.method,
            log.path,
            log.query_string,
            log.request_headers,
            log.request_body,
            log.match_status,
            log.execution_status,
            log.exit_code,
            log.stdout_tail,
            log.stderr_tail,
            log.duration_ms,
            log.created_at,
        ],
    )
    .map_err(|e| format!("记录 WebHook 审计日志失败: {e}"))?;

    Ok(())
}

pub fn update_log_execution(
    conn: &Connection,
    id: &str,
    status: &str,
    exit_code: Option<i32>,
    stdout: Option<&str>,
    stderr: Option<&str>,
    duration_ms: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE webhook_logs SET
            execution_status = ?1,
            exit_code = ?2,
            stdout_tail = ?3,
            stderr_tail = ?4,
            duration_ms = ?5
         WHERE id = ?6",
        params![status, exit_code, stdout, stderr, duration_ms, id],
    )
    .map_err(|e| format!("更新 WebHook 审计执行状态失败: {e}"))?;

    Ok(())
}

pub fn list_logs(
    conn: &Connection,
    limit: usize,
    offset: usize,
    rule_id: Option<&str>,
) -> Result<(Vec<WebhookLog>, usize), String> {
    let total: usize = if let Some(rid) = rule_id {
        conn.query_row(
            "SELECT COUNT(1) FROM webhook_logs WHERE rule_id = ?1",
            params![rid],
            |r| r.get(0),
        )
        .unwrap_or(0)
    } else {
        conn.query_row("SELECT COUNT(1) FROM webhook_logs", [], |r| r.get(0))
            .unwrap_or(0)
    };

    let sql = if rule_id.is_some() {
        "SELECT id, rule_id, rule_name, client_ip, method, path, query_string,
                request_headers, request_body, match_status, execution_status,
                exit_code, stdout_tail, stderr_tail, duration_ms, created_at
         FROM webhook_logs
         WHERE rule_id = ?1
         ORDER BY created_at DESC
         LIMIT ?2 OFFSET ?3"
    } else {
        "SELECT id, rule_id, rule_name, client_ip, method, path, query_string,
                request_headers, request_body, match_status, execution_status,
                exit_code, stdout_tail, stderr_tail, duration_ms, created_at
         FROM webhook_logs
         ORDER BY created_at DESC
         LIMIT ?1 OFFSET ?2"
    };

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;

    let map_row = |r: &rusqlite::Row| {
        Ok(WebhookLog {
            id: r.get(0)?,
            rule_id: r.get(1)?,
            rule_name: r.get(2)?,
            client_ip: r.get(3)?,
            method: r.get(4)?,
            path: r.get(5)?,
            query_string: r.get(6)?,
            request_headers: r.get(7)?,
            request_body: r.get(8)?,
            match_status: r.get(9)?,
            execution_status: r.get(10)?,
            exit_code: r.get(11)?,
            stdout_tail: r.get(12)?,
            stderr_tail: r.get(13)?,
            duration_ms: r.get(14)?,
            created_at: r.get(15)?,
        })
    };

    let logs: Vec<WebhookLog> = if let Some(rid) = rule_id {
        let rows = stmt
            .query_map(params![rid, limit as i64, offset as i64], map_row)
            .map_err(|e| e.to_string())?;
        rows.filter_map(|r| r.ok()).collect()
    } else {
        let rows = stmt
            .query_map(params![limit as i64, offset as i64], map_row)
            .map_err(|e| e.to_string())?;
        rows.filter_map(|r| r.ok()).collect()
    };

    Ok((logs, total))
}

pub fn clear_logs(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM webhook_logs", [])
        .map_err(|e| format!("清空 WebHook 审计日志失败: {e}"))?;
    Ok(())
}
