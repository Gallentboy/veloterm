//! 定时任务引擎（tokio-cron-scheduler）：命令/脚本的可调度执行。
//!
//! 架构分四层：
//! - **store**：SQLCipher 持久化（scheduled_tasks + scheduled_task_logs 两表，
//!   幂等建表），任务 UUID 与引擎 Job UUID 一致；
//! - **engine**：tokio-cron-scheduler 包装——应用启动时从 DB 加载 enabled
//!   任务逐条 `Job::new_async_tz`，CRUD 时动态 add/remove；
//! - **executor**：执行分派——`local_command` 走 tokio::process（设 cwd、
//!   超时保护、stdout/stderr 尾部截取），`ssh_command` 每次独立建连
//!   （`connect_ssh_chain_headless`，不复用交互终端——定时任务不污染终端
//!   会话，代价是重连开销）；
//! - **命令层**（本文件）：Tauri invoke 的 CRUD + 启停 + 立即执行 + 日志。
//!
//! 引擎实例：setup 闭包非 async，调度器经 `tauri::async_runtime::spawn` 创建
//! 后存入全局 `SCHEDULER_ENGINE`（OnceLock）——命令层从这里取，不挂 manage。
//!
//! cron 表达式：标准 5 字段，由 tokio-cron-scheduler 内部的 `cron` crate 校验。
//! 事件推送：`scheduler-task-event`（started/success/failed），前端 listen()。

pub mod engine;
pub mod executor;
pub mod store;

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::DbPool;

/// 全局引擎实例（async setup 后注入）
pub static SCHEDULER_ENGINE: OnceLock<engine::SchedulerEngine> = OnceLock::new();

/// 引擎就绪信号（测试用）
pub static SCHEDULER_READY: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// 定时任务记录（前端可见的完整形态）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTask {
    pub id: String,
    pub name: String,
    pub cron_expr: String,
    /// 'local_command' | 'ssh_command' | 'js_script'
    pub task_type: String,
    /// 命令或脚本路径
    pub command: String,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub host_id: Option<usize>,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_run_at: Option<i64>,
    pub last_status: Option<String>,
    // ── D1 失败通知 ──
    /// 'failure' | 'always' | null（不发通知）
    #[serde(default)]
    pub notify_on: Option<String>,
    /// webhook URL（POST {task, exitCode, stderrTail}）
    #[serde(default)]
    pub webhook_url: Option<String>,
    // ── D2 超时与重试 ──
    /// 执行超时秒数（默认 600）
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// 最大重试次数（默认 0）
    #[serde(default)]
    pub max_retries: Option<u32>,
    /// 重试间隔秒数（默认 30）
    #[serde(default)]
    pub retry_backoff_secs: Option<u64>,
    // ── D4 一次性任务 ──
    /// 一次性触发时间戳（秒）；有值时 cron_expr 忽略，触发后自动 disable
    #[serde(default)]
    pub run_at: Option<i64>,
    // ── D5 历史保留 ──
    /// 日志保留天数（默认 30；0 = 永久保留）
    #[serde(default)]
    pub retention_days: Option<u32>,
}

/// 执行日志条目
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskLog {
    pub id: i64,
    pub task_id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
}

/// 执行历史查询参数
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskHistoryQuery {
    pub task_id: Option<String>,
    pub status: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// 执行历史条目（含任务关联信息及格式化结果）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskHistoryItem {
    pub id: i64,
    pub task_id: String,
    pub task_name: String,
    pub task_type: Option<String>,
    pub command: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub exit_code: Option<i32>,
    pub status: String,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
}

/// 执行历史分页响应
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskHistoryResponse {
    pub items: Vec<TaskHistoryItem>,
    pub total: usize,
}

/// 前端事件载荷（`scheduler-task-event`）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvent {
    pub task_id: String,
    pub task_name: String,
    /// 'started' | 'success' | 'failed'
    pub kind: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub message: Option<String>,
}

/// 取引擎实例（启动失败/未就绪时返回错误）
fn engine() -> Result<&'static engine::SchedulerEngine, String> {
    SCHEDULER_ENGINE
        .get()
        .ok_or_else(|| "定时任务引擎尚未就绪".to_string())
}

// ── CRUD ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn scheduler_task_create(
    pool: State<'_, DbPool>,
    task: ScheduledTask,
) -> Result<ScheduledTask, String> {
    let created = store::create_task(&pool, &task)?;
    if created.enabled {
        engine()?.add_job(&pool, &created).await?;
    }
    Ok(created)
}

#[tauri::command]
pub async fn scheduler_task_update(
    pool: State<'_, DbPool>,
    task: ScheduledTask,
) -> Result<ScheduledTask, String> {
    let updated = store::update_task(&pool, &task)?;
    // 引擎侧重挂：先删旧 Job（幂等），enabled 时再加新
    engine()?.remove_job(&updated.id).await;
    if updated.enabled {
        engine()?.add_job(&pool, &updated).await?;
    }
    Ok(updated)
}

#[tauri::command]
pub async fn scheduler_task_delete(pool: State<'_, DbPool>, id: String) -> Result<(), String> {
    store::delete_task(&pool, &id)?;
    if let Some(e) = SCHEDULER_ENGINE.get() {
        e.remove_job(&id).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn scheduler_task_toggle(
    pool: State<'_, DbPool>,
    id: String,
    enabled: bool,
) -> Result<ScheduledTask, String> {
    let task = store::set_enabled(&pool, &id, enabled)?;
    let e = engine()?;
    if enabled {
        e.add_job(&pool, &task).await?;
    } else {
        e.remove_job(&id).await;
    }
    Ok(task)
}

#[tauri::command]
pub fn scheduler_task_list(pool: State<'_, DbPool>) -> Result<Vec<ScheduledTask>, String> {
    store::list_tasks(&pool)
}

#[tauri::command]
pub fn scheduler_task_logs(
    pool: State<'_, DbPool>,
    task_id: String,
    limit: Option<usize>,
) -> Result<Vec<TaskLog>, String> {
    store::list_logs(&pool, &task_id, limit.unwrap_or(20).min(100))
}

#[tauri::command]
pub fn scheduler_task_history(
    pool: State<'_, DbPool>,
    query: Option<TaskHistoryQuery>,
) -> Result<TaskHistoryResponse, String> {
    let q = query.unwrap_or_default();
    store::query_history(&pool, &q)
}

#[tauri::command]
pub fn scheduler_history_clear(
    pool: State<'_, DbPool>,
    task_id: Option<String>,
) -> Result<(), String> {
    store::clear_history(&pool, task_id.as_deref())
}

/// 立即执行一次（不等 cron 触发；不影响 cron 调度）
#[tauri::command]
pub async fn scheduler_run_now(id: String) -> Result<(), String> {
    engine()?.run_now(&id).await
}

/// Cron 表达式的未来触发预览（D3）：返回接下来 N 次运行时间的 Unix 秒时间戳。
/// 用 croner（tokio-cron-scheduler 内部依赖）解析，与引擎行为一致
#[tauri::command]
pub fn scheduler_preview_next(
    cron_expr: String,
    count: Option<usize>,
) -> Result<Vec<i64>, String> {
    use croner::parser::CronParser;

    let expr = if cron_expr.trim().split_whitespace().count() == 5 {
        format!("0 {}", cron_expr.trim())
    } else {
        cron_expr.trim().to_string()
    };

    let schedule = CronParser::builder()
        .seconds(croner::parser::Seconds::Required)
        .dom_and_dow(true)
        .build()
        .parse(&expr)
        .map_err(|e| format!("cron 表达式无效：{e}"))?;

    let n = count.unwrap_or(5).min(20);
    let mut times = Vec::with_capacity(n);
    let mut cursor = chrono::Local::now();
    for _ in 0..n {
        match schedule.find_next_occurrence(&cursor, false) {
            Ok(next) => {
                times.push(next.timestamp());
                cursor = next;
            }
            Err(_) => break,
        }
    }
    Ok(times)
}

/// Cron 表达式的人话翻译（D3）：`0 0 9 * * 1-5` → `每工作日 09:00`
#[tauri::command]
pub fn scheduler_describe_cron(cron_expr: String) -> Result<String, String> {
    let parts: Vec<&str> = cron_expr.trim().split_whitespace().collect();
    if parts.len() != 5 && parts.len() != 6 {
        return Err("cron 表达式需要 5 或 6 个字段".to_string());
    }
    // 5 字段时字段序为 分 时 日 月 周；6 字段时为 秒 分 时 日 月 周
    let (min, hour, dom, mon, dow) = if parts.len() == 5 {
        (
            parts.first().unwrap_or(&"*"),
            parts.get(1).unwrap_or(&"*"),
            parts.get(2).unwrap_or(&"*"),
            parts.get(3).unwrap_or(&"*"),
            parts.get(4).unwrap_or(&"*"),
        )
    } else {
        (
            parts.get(1).unwrap_or(&"*"),
            parts.get(2).unwrap_or(&"*"),
            parts.get(3).unwrap_or(&"*"),
            parts.get(4).unwrap_or(&"*"),
            parts.get(5).unwrap_or(&"*"),
        )
    };

    // 快速人话模板（覆盖高频模式）
    let pad2 = |s: &str| -> String {
        if s.len() < 2 { format!("0{s}") } else { s.to_string() }
    };
    let hm = format!("{}:{}", pad2(hour), pad2(min));

    let desc = if min == &"*" && hour == &"*" && dom == &"*" && mon == &"*" && dow == &"*" {
        "每分钟".to_string()
    } else if min.starts_with("*/") && hour == &"*" && dom == &"*" && mon == &"*" && dow == &"*" {
        format!("每 {} 分钟", &min[2..])
    } else if hour.starts_with("*/") && min == &"0" && dom == &"*" && mon == &"*" && dow == &"*" {
        format!("每 {} 小时", &hour[2..])
    } else if min != &"*" && hour != &"*" && dom == &"*" && mon == &"*" && dow == &"*" {
        format!("每天 {hm}")
    } else if min != &"*" && hour != &"*" && dom == &"*" && mon == &"*" && dow == &"1-5" {
        format!("工作日 {hm}")
    } else if min != &"*" && hour != &"*" && dom == &"*" && mon == &"*" && (dow == &"0" || dow == &"7" || dow == &"6,0" || dow == &"0,6") {
        format!("周末 {hm}")
    } else if min != &"*" && hour != &"*" && dom == &"1" && mon == &"*" && dow == &"*" {
        format!("每月 1 日 {hm}")
    } else {
        format!("cron: {cron_expr}")
    };
    Ok(desc)
}
