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
    /// 'local_command' | 'ssh_command'
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
