//! 主机密码历史：自动改密/手输更新前留档被替换的旧密码，供「改错了找回」。
//! 记录由 `db::update_host_password_with_source` 在更新前写入（每主机最多 20 条），
//! 本模块只做读取与清空。

use crate::db::{self, global_pool};

/// 历史条目（前端 camelCase）
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostPasswordHistoryEntry {
    pub id: i64,
    pub host_id: i64,
    /// 被替换掉的旧密码明文（库本身是 SQLCipher 加密的）
    pub password: String,
    /// 被替换时刻（unix 毫秒）
    pub replaced_at: i64,
    /// kbd-int-change（认证内改密）/ user-input（手输通过认证）
    pub source: String,
}

fn pool() -> Result<db::DbPool, String> {
    global_pool().ok_or_else(|| "数据库未初始化".to_string())
}

/// 查某主机的密码历史（按替换时间倒序，最多 20 条）
#[tauri::command]
pub fn host_password_history_list(host_id: i64) -> Result<Vec<HostPasswordHistoryEntry>, String> {
    let conn = pool()?.get().map_err(|e| format!("pool get: {e}"))?;
    let mut stmt = conn
        .prepare(
            "SELECT id, host_id, password, replaced_at, source
             FROM host_password_history WHERE host_id = ?1
             ORDER BY replaced_at DESC, id DESC LIMIT 20",
        )
        .map_err(|e| format!("prepare: {e}"))?;
    let rows = stmt
        .query_map(rusqlite::params![host_id], |row| {
            Ok(HostPasswordHistoryEntry {
                id: row.get(0)?,
                host_id: row.get(1)?,
                password: row.get(2)?,
                replaced_at: row.get(3)?,
                source: row.get(4)?,
            })
        })
        .map_err(|e| format!("query: {e}"))?;
    let mut entries = Vec::new();
    for row in rows {
        entries.push(row.map_err(|e| format!("row: {e}"))?);
    }
    Ok(entries)
}

/// 清空某主机的密码历史（或全部：host_id = None）
#[tauri::command]
pub fn host_password_history_clear(host_id: Option<i64>) -> Result<usize, String> {
    let conn = pool()?.get().map_err(|e| format!("pool get: {e}"))?;
    conn.execute(
        "DELETE FROM host_password_history WHERE (?1 IS NULL OR host_id = ?1)",
        rusqlite::params![host_id],
    )
    .map_err(|e| format!("delete: {e}"))
}
