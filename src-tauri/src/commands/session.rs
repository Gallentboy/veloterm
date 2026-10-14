use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::params;
use tauri::State;

use crate::db::{
  BootstrapStatePayload, DbPool, HostStorePayload, SessionModel, delete_kv, get_all_kv, get_kv,
  load_host_store, save_host_store, set_kv,
};

fn now_timestamp_ms() -> i64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    .unwrap_or(0)
}

pub fn list_sessions_with_pool(pool: &DbPool) -> Result<Vec<SessionModel>, String> {
  let conn = pool
    .get()
    .map_err(|e| format!("failed to get database connection: {e}"))?;

  let mut stmt = conn
    .prepare(
      "SELECT id, name, host, port, username, auth_type, group_name
       FROM sessions
       ORDER BY created_at ASC, id ASC",
    )
    .map_err(|e| format!("failed to prepare list_sessions query: {e}"))?;

  let rows = stmt
    .query_map([], |row| {
      let port_i64: i64 = row.get(3)?;
      Ok(SessionModel {
        id: row.get(0)?,
        name: row.get(1)?,
        host: row.get(2)?,
        port: u16::try_from(port_i64.clamp(1, 65535)).unwrap_or(22),
        username: row.get(4)?,
        auth_type: row.get(5)?,
        group_name: row.get(6)?,
      })
    })
    .map_err(|e| format!("failed to execute list_sessions query: {e}"))?;

  let mut sessions = Vec::new();
  for item in rows {
    sessions.push(item.map_err(|e| format!("failed to decode session row: {e}"))?);
  }
  Ok(sessions)
}

pub fn save_session_with_pool(pool: &DbPool, mut session: SessionModel) -> Result<SessionModel, String> {
  let now = now_timestamp_ms();
  let conn = pool
    .get()
    .map_err(|e| format!("failed to get database connection: {e}"))?;

  if session.id == 0 {
    conn
      .execute(
        "INSERT INTO sessions (name, host, port, username, auth_type, group_name, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
          &session.name,
          &session.host,
          i64::from(session.port),
          &session.username,
          &session.auth_type,
          &session.group_name,
          now,
          now,
        ],
      )
      .map_err(|e| format!("failed to insert new session: {e}"))?;
    session.id = usize::try_from(conn.last_insert_rowid()).unwrap_or(1);
  } else {
    conn
      .execute(
        "INSERT INTO sessions (id, name, host, port, username, auth_type, group_name, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
           name = excluded.name,
           host = excluded.host,
           port = excluded.port,
           username = excluded.username,
           auth_type = excluded.auth_type,
           group_name = excluded.group_name,
           updated_at = excluded.updated_at",
        params![
          session.id,
          &session.name,
          &session.host,
          i64::from(session.port),
          &session.username,
          &session.auth_type,
          &session.group_name,
          now,
          now,
        ],
      )
      .map_err(|e| format!("failed to save session {}: {e}", session.id))?;
  }

  Ok(session)
}

pub fn delete_session_with_pool(pool: &DbPool, id: usize) -> Result<(), String> {
  let conn = pool
    .get()
    .map_err(|e| format!("failed to get database connection: {e}"))?;

  conn
    .execute("DELETE FROM sessions WHERE id = ?1", params![id])
    .map_err(|e| format!("failed to delete session {id}: {e}"))?;

  Ok(())
}

/// 获取全部已保存的会话。
#[tauri::command]
pub async fn list_sessions(pool: State<'_, DbPool>) -> Result<Vec<SessionModel>, String> {
  let pool = pool.inner().clone();
  tokio::task::spawn_blocking(move || list_sessions_with_pool(&pool))
    .await
    .map_err(|e| format!("list_sessions task join error: {e}"))?
}

/// 新增或更新会话（若 ID 为 0 则由数据库自增生成 `usize` 主键）。
#[tauri::command]
pub async fn save_session(
  pool: State<'_, DbPool>,
  session: SessionModel,
) -> Result<SessionModel, String> {
  let pool = pool.inner().clone();
  tokio::task::spawn_blocking(move || save_session_with_pool(&pool, session))
    .await
    .map_err(|e| format!("save_session task join error: {e}"))?
}

/// 根据 ID 删除会话。
#[tauri::command]
pub async fn delete_session(pool: State<'_, DbPool>, id: usize) -> Result<(), String> {
  let pool = pool.inner().clone();
  tokio::task::spawn_blocking(move || delete_session_with_pool(&pool, id))
    .await
    .map_err(|e| format!("delete_session task join error: {e}"))?
}

/// 一次性读取 `veloterm.db` 中的全量持久化状态（`app_kv` + `groups` + `hosts` + `host_groups`）。
#[tauri::command]
pub async fn db_bootstrap_state(pool: State<'_, DbPool>) -> Result<BootstrapStatePayload, String> {
  let pool = pool.inner().clone();
  tokio::task::spawn_blocking(move || {
    let kv = get_all_kv(&pool)?;
    let host_store = load_host_store(&pool)?;
    Ok(BootstrapStatePayload { kv, host_store })
  })
  .await
  .map_err(|e| format!("db_bootstrap_state task join error: {e}"))?
}

#[tauri::command]
pub async fn db_get_all_kv(pool: State<'_, DbPool>) -> Result<HashMap<String, String>, String> {
  let pool = pool.inner().clone();
  tokio::task::spawn_blocking(move || get_all_kv(&pool))
    .await
    .map_err(|e| format!("db_get_all_kv task join error: {e}"))?
}

#[tauri::command]
pub async fn db_get_kv(pool: State<'_, DbPool>, key: String) -> Result<Option<String>, String> {
  let pool = pool.inner().clone();
  tokio::task::spawn_blocking(move || get_kv(&pool, &key))
    .await
    .map_err(|e| format!("db_get_kv task join error: {e}"))?
}

#[tauri::command]
pub async fn db_set_kv(pool: State<'_, DbPool>, key: String, value: String) -> Result<(), String> {
  let pool = pool.inner().clone();
  tokio::task::spawn_blocking(move || set_kv(&pool, &key, &value))
    .await
    .map_err(|e| format!("db_set_kv task join error: {e}"))?
}

#[tauri::command]
pub async fn db_delete_kv(pool: State<'_, DbPool>, key: String) -> Result<(), String> {
  let pool = pool.inner().clone();
  tokio::task::spawn_blocking(move || delete_kv(&pool, &key))
    .await
    .map_err(|e| format!("db_delete_kv task join error: {e}"))?
}

#[tauri::command]
pub async fn db_load_host_store(pool: State<'_, DbPool>) -> Result<HostStorePayload, String> {
  let pool = pool.inner().clone();
  tokio::task::spawn_blocking(move || load_host_store(&pool))
    .await
    .map_err(|e| format!("db_load_host_store task join error: {e}"))?
}

#[tauri::command]
pub async fn db_save_host_store(
  pool: State<'_, DbPool>,
  store: HostStorePayload,
) -> Result<(), String> {
  let pool = pool.inner().clone();
  tokio::task::spawn_blocking(move || save_host_store(&pool, &store))
    .await
    .map_err(|e| format!("db_save_host_store task join error: {e}"))?
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::db::init_db_at_path;
  use uuid::Uuid;

  #[test]
  fn session_crud_roundtrip() {
    let temp_dir = std::env::temp_dir().join(format!("veloterm-session-crud-{}", Uuid::new_v4()));
    let db_path = temp_dir.join("veloterm.db");
    let pool = init_db_at_path(&db_path, "crud-test-key").expect("init_db_at_path should succeed");

    // 1. 新增（id = 0 自动分配自增 usize 主键）
    let created = save_session_with_pool(
      &pool,
      SessionModel {
        id: 0,
        name: "bastion-1".to_string(),
        host: "192.168.1.10".to_string(),
        port: 2222,
        username: "admin".to_string(),
        auth_type: "key".to_string(),
        group_name: Some("Gateway".to_string()),
      },
    )
    .expect("save_session_with_pool should succeed");
    assert!(created.id > 0);

    // 2. 查询列表
    let list = list_sessions_with_pool(&pool).expect("list_sessions_with_pool should succeed");
    assert_eq!(list.len(), 1);
    assert_eq!(list.first().unwrap(), &created);

    // 3. 更新同 ID
    let mut updated_input = created.clone();
    updated_input.name = "bastion-primary".to_string();
    updated_input.port = 22;
    let updated =
      save_session_with_pool(&pool, updated_input.clone()).expect("update should succeed");
    assert_eq!(updated.name, "bastion-primary");
    assert_eq!(updated.port, 22);

    let list_after_update = list_sessions_with_pool(&pool).expect("list should succeed");
    assert_eq!(list_after_update, vec![updated_input]);

    // 4. 删除
    delete_session_with_pool(&pool, created.id).expect("delete should succeed");
    let list_after_delete = list_sessions_with_pool(&pool).expect("list should succeed");
    assert!(list_after_delete.is_empty());

    let _ = std::fs::remove_dir_all(&temp_dir);
  }
}
