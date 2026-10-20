//! 命令历史的查询 / 清理 / 采集开关命令（侧栏「历史」页调用）。
//! 采集本身在 terminal::command_history（输出流旁路），本模块只做读取面。

use serde::Serialize;

use crate::db::{self, global_pool};
use crate::terminal::command_history as capture;

fn pool() -> Result<db::DbPool, String> {
    global_pool().ok_or_else(|| "数据库未初始化".to_string())
}

/// 前端行结构（camelCase 序列化）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandHistoryRow {
    pub id: i64,
    pub host_id: Option<i64>,
    pub session_id: i64,
    pub command: String,
    pub exit_code: i32,
    /// unix 毫秒
    pub created_at: i64,
}

fn to_row(entry: capture::CommandHistoryEntry) -> CommandHistoryRow {
    CommandHistoryRow {
        id: entry.id,
        host_id: entry.record.host_id,
        session_id: entry.record.session_id,
        command: entry.record.command,
        exit_code: entry.record.exit_code,
        created_at: entry.record.created_at,
    }
}

/// 按主机过滤 / 只看失败 / 仅本地，时间倒序分页（limit 默认 200，offset 默认 0）
#[tauri::command]
pub fn command_history_list(
    host_id: Option<i64>,
    only_failed: Option<bool>,
    local_only: Option<bool>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<CommandHistoryRow>, String> {
    let limit = limit.unwrap_or(200).clamp(1, 1000);
    let offset = offset.unwrap_or(0).max(0);
    capture::list(
        &pool()?,
        host_id,
        only_failed.unwrap_or(false),
        local_only.unwrap_or(false),
        limit,
        offset,
    )
    .map(|entries| entries.into_iter().map(to_row).collect())
}

/// 清空历史（指定主机只清该主机；仅本地只清本地记录；均不传清全部）。返回删除条数
#[tauri::command]
pub fn command_history_clear(
    host_id: Option<i64>,
    local_only: Option<bool>,
) -> Result<usize, String> {
    capture::clear(&pool()?, host_id, local_only.unwrap_or(false))
}

/// 读采集开关（剥离永远执行，开关只控制入库）
#[tauri::command]
pub fn command_history_get_enabled() -> bool {
    capture::enabled()
}

/// 写采集开关：内存即时生效 + kv 持久化（下次启动回灌）
#[tauri::command]
pub fn command_history_set_enabled(enabled: bool) -> Result<(), String> {
    capture::set_enabled(enabled);
    capture::persist_enabled(&pool()?, enabled);
    Ok(())
}
