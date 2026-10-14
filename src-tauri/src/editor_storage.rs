//! 远程文件编辑器的持久化命令：面板布局（`app_kv`）与未保存草稿（`editor_drafts` 表）。
//!
//! 前端契约（字段均为 camelCase）：
//! - `editor_load_state() -> { panelStateJson, drafts }`
//! - `editor_save_panel_state(stateJson)`
//! - `editor_save_draft(fileId, hostId, remotePath, baseHash, content)`
//! - `editor_delete_draft(fileId)`

use serde::Serialize;
use tauri::State;

use crate::db::{
  DbPool, EditorDraftRecord, delete_editor_draft, get_kv, load_editor_drafts, save_editor_draft,
  set_kv,
};

/// `app_kv` 中存储编辑器面板布局 JSON 的键。
const PANEL_STATE_KV_KEY: &str = "veloterm:editor:panel-state";

/// 编辑器启动时一次性拉取的全量状态（面板布局 + 全部草稿）。
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EditorStatePayload {
  pub panel_state_json: Option<String>,
  pub drafts: Vec<EditorDraftRecord>,
}

/// 读取编辑器全量持久化状态（面板布局 JSON + 全部未保存草稿）。
#[tauri::command]
pub async fn editor_load_state(pool: State<'_, DbPool>) -> Result<EditorStatePayload, String> {
  let pool = pool.inner().clone();
  tokio::task::spawn_blocking(move || {
    let panel_state_json = get_kv(&pool, PANEL_STATE_KV_KEY)?;
    let drafts = load_editor_drafts(&pool)?;
    Ok(EditorStatePayload {
      panel_state_json,
      drafts,
    })
  })
  .await
  .map_err(|e| format!("editor_load_state task join error: {e}"))?
}

/// 保存编辑器面板布局 JSON（整体覆盖）。
#[tauri::command]
pub async fn editor_save_panel_state(
  pool: State<'_, DbPool>,
  state_json: String,
) -> Result<(), String> {
  let pool = pool.inner().clone();
  tokio::task::spawn_blocking(move || set_kv(&pool, PANEL_STATE_KV_KEY, &state_json))
    .await
    .map_err(|e| format!("editor_save_panel_state task join error: {e}"))?
}

/// 新增或更新一条编辑器草稿（按 `fileId` upsert，`updatedAt` 由后端写入）。
#[tauri::command]
pub async fn editor_save_draft(
  pool: State<'_, DbPool>,
  file_id: i64,
  host_id: i64,
  remote_path: String,
  base_hash: String,
  content: String,
) -> Result<(), String> {
  let pool = pool.inner().clone();
  tokio::task::spawn_blocking(move || {
    let draft = EditorDraftRecord {
      file_id,
      host_id,
      remote_path,
      base_hash,
      content,
      updated_at: 0,
    };
    save_editor_draft(&pool, &draft)
  })
  .await
  .map_err(|e| format!("editor_save_draft task join error: {e}"))?
}

/// 按 `fileId` 删除一条编辑器草稿（保存成功或用户放弃编辑后调用）。
#[tauri::command]
pub async fn editor_delete_draft(
  pool: State<'_, DbPool>,
  file_id: i64,
) -> Result<(), String> {
  let pool = pool.inner().clone();
  tokio::task::spawn_blocking(move || delete_editor_draft(&pool, file_id))
    .await
    .map_err(|e| format!("editor_delete_draft task join error: {e}"))?
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::db::init_db_at_path;
  use uuid::Uuid;

  #[test]
  fn editor_state_payload_serializes_camel_case() {
    let payload = EditorStatePayload {
      panel_state_json: Some("{\"layout\":\"split\"}".to_string()),
      drafts: vec![EditorDraftRecord {
        file_id: 7,
        host_id: 3,
        remote_path: "/etc/hosts".to_string(),
        base_hash: "h".to_string(),
        content: "127.0.0.1 localhost".to_string(),
        updated_at: 123,
      }],
    };

    let json = serde_json::to_string(&payload).unwrap();
    assert!(json.contains("\"panelStateJson\""));
    assert!(json.contains("\"fileId\":7"));
    assert!(json.contains("\"hostId\":3"));
    assert!(json.contains("\"remotePath\""));
    assert!(json.contains("\"baseHash\""));
    assert!(json.contains("\"updatedAt\":123"));
  }

  #[test]
  fn editor_drafts_roundtrip_via_db() {
    let temp_dir = std::env::temp_dir().join(format!("veloterm-editor-cmd-{}", Uuid::new_v4()));
    let pool = init_db_at_path(&temp_dir.join("veloterm.db"), "editor-cmd-key")
      .expect("init_db_at_path should succeed");

    set_kv(&pool, PANEL_STATE_KV_KEY, "{\"w\":1}").expect("set panel state should succeed");
    save_editor_draft(
      &pool,
      &EditorDraftRecord {
        file_id: 11,
        host_id: 2,
        remote_path: "/tmp/a.txt".to_string(),
        base_hash: "bh".to_string(),
        content: "hello".to_string(),
        updated_at: 0,
      },
    )
    .expect("save draft should succeed");

    let state_json = get_kv(&pool, PANEL_STATE_KV_KEY).expect("get panel state should succeed");
    let drafts = load_editor_drafts(&pool).expect("load drafts should succeed");
    assert_eq!(state_json.as_deref(), Some("{\"w\":1}"));
    assert_eq!(drafts.len(), 1);
    let first = drafts.first().unwrap();
    assert_eq!(first.file_id, 11);
    assert!(first.updated_at > 0);

    delete_editor_draft(&pool, 11).expect("delete draft should succeed");
    assert!(load_editor_drafts(&pool).expect("load drafts should succeed").is_empty());

    let _ = std::fs::remove_dir_all(&temp_dir);
  }
}
