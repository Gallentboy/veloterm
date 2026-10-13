//! AI 能力层：基于 rig 的统一多供应商 LLM 问答。
//!
//! - 三类供应商统一接口：`openai`（含兼容端点，可自定义 base_url）、`anthropic`、`gemini`
//! - 多 API Key 轮询：每个供应商配置任意数量的 key，请求间轮换分摊限流配额
//! - 流式交互：`ai_chat` 通过 Tauri Channel 推送增量文本，与 SFTP 进度同一通信模式
//! - 配置持久化：SQLCipher `ai_providers` 表

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::State;

use crate::db::DbPool;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderRecord {
  pub id: usize,
  pub name: String,
  /// "openai" | "anthropic" | "gemini"
  pub provider_type: String,
  /// 自定义端点（openai 兼容：DeepSeek/GLM 等）；空 = 官方默认
  pub base_url: Option<String>,
  pub model: String,
  /// 多 Key 轮询池
  pub api_keys: Vec<String>,
  #[serde(default)]
  pub is_default: bool,
  pub created_at: i64,
  pub updated_at: i64,
}

/// Channel 推送的流式事件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChunk {
  /// 增量文本
  pub delta: Option<String>,
  /// 本轮使用的 key 序号（从 0 起），便于前端展示轮询状态
  pub key_index: Option<usize>,
  pub error: Option<String>,
  pub done: bool,
}

/// 每供应商的 key 轮询计数（请求间轮换）
static KEY_ROTATION: Mutex<Option<HashMap<usize, usize>>> = Mutex::new(None);

fn next_key_index(provider_id: usize, key_count: usize) -> usize {
  if key_count <= 1 {
    return 0;
  }
  let mut guard = KEY_ROTATION.lock().unwrap();
  let map = guard.get_or_insert_with(HashMap::new);
  let counter = map.entry(provider_id).or_insert(0);
  let current = *counter;
  *counter = (*counter + 1) % key_count;
  current
}

// ─────────────────────────── 数据库 ───────────────────────────

fn ensure_table(pool: &DbPool) -> Result<(), String> {
  let conn = pool.get().map_err(|e| e.to_string())?;
  conn
    .execute_batch(
      "CREATE TABLE IF NOT EXISTS ai_providers (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        provider_type TEXT NOT NULL,
        base_url TEXT,
        model TEXT NOT NULL,
        api_keys TEXT NOT NULL,
        is_default INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
      );",
    )
    .map_err(|e| e.to_string())?;
  Ok(())
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<AiProviderRecord> {
  let api_keys_json: String = row.get("api_keys")?;
  let is_default: i64 = row.get("is_default")?;
  Ok(AiProviderRecord {
    id: row.get::<_, i64>("id")? as usize,
    name: row.get("name")?,
    provider_type: row.get("provider_type")?,
    base_url: row.get("base_url")?,
    model: row.get("model")?,
    api_keys: serde_json::from_str(&api_keys_json).unwrap_or_default(),
    is_default: is_default != 0,
    created_at: row.get("created_at")?,
    updated_at: row.get("updated_at")?,
  })
}

pub fn list_providers(pool: &DbPool) -> Result<Vec<AiProviderRecord>, String> {
  ensure_table(pool)?;
  let conn = pool.get().map_err(|e| e.to_string())?;
  let mut stmt = conn
    .prepare("SELECT * FROM ai_providers ORDER BY id ASC")
    .map_err(|e| e.to_string())?;
  let rows = stmt
    .query_map([], row_to_record)
    .map_err(|e| e.to_string())?;
  rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn get_provider(pool: &DbPool, id: usize) -> Result<AiProviderRecord, String> {
  ensure_table(pool)?;
  let conn = pool.get().map_err(|e| e.to_string())?;
  conn
    .query_row("SELECT * FROM ai_providers WHERE id = ?1", [id as i64], row_to_record)
    .map_err(|e| e.to_string())
}

pub fn save_provider(pool: &DbPool, record: &AiProviderRecord) -> Result<(), String> {
  ensure_table(pool)?;
  let api_keys = serde_json::to_string(&record.api_keys).map_err(|e| e.to_string())?;
  let conn = pool.get().map_err(|e| e.to_string())?;
  conn
    .execute(
      "INSERT INTO ai_providers (id, name, provider_type, base_url, model, api_keys, is_default, created_at, updated_at)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
       ON CONFLICT(id) DO UPDATE SET
         name = excluded.name,
         provider_type = excluded.provider_type,
         base_url = excluded.base_url,
         model = excluded.model,
         api_keys = excluded.api_keys,
         is_default = excluded.is_default,
         updated_at = excluded.updated_at",
      rusqlite::params![
        record.id as i64,
        record.name,
        record.provider_type,
        record.base_url,
        record.model,
        api_keys,
        record.is_default as i64,
        record.created_at,
        record.updated_at,
      ],
    )
    .map_err(|e| e.to_string())?;
  Ok(())
}

pub fn delete_provider(pool: &DbPool, id: usize) -> Result<(), String> {
  ensure_table(pool)?;
  let conn = pool.get().map_err(|e| e.to_string())?;
  conn
    .execute("DELETE FROM ai_providers WHERE id = ?1", [id as i64])
    .map_err(|e| e.to_string())?;
  Ok(())
}

pub fn default_provider(pool: &DbPool) -> Result<Option<AiProviderRecord>, String> {
  let providers = list_providers(pool)?;
  Ok(
    providers
      .iter()
      .find(|p| p.is_default)
      .or_else(|| providers.first())
      .cloned(),
  )
}

// ─────────────────────────── rig 统一流式 ───────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiChatMessage {
  pub role: String, // "user" | "assistant"
  pub content: String,
}

/// 泛型流式执行：三供应商模型收敛到同一循环，Channel 推增量
async fn stream_model<M>(
  model: M,
  history: Vec<AiChatMessage>,
  preamble: Option<String>,
  key_index: usize,
  on_chunk: &Channel<AiChunk>,
) -> Result<(), String>
where
  M: rig::completion::CompletionModel + Clone,
{
  use futures::StreamExt;

  let mut messages = Vec::with_capacity(history.len());
  for item in &history {
    match item.role.as_str() {
      "assistant" => messages.push(rig::message::Message::assistant(item.content.clone())),
      _ => messages.push(rig::message::Message::user(item.content.clone())),
    }
  }

  let mut builder = model.completion_request("").messages(messages);
  if let Some(p) = preamble {
    builder = builder.preamble(p);
  }
  let request = builder.build();

  let mut stream = model.stream(request).await.map_err(|e| e.to_string())?;
  while let Some(part) = stream.next().await {
    match part {
      Ok(rig::streaming::StreamedAssistantContent::Text(text)) => {
        let _ = on_chunk.send(AiChunk {
          delta: Some(text.text.clone()),
          key_index: Some(key_index),
          error: None,
          done: false,
        });
      }
      Ok(_) => {}
      Err(e) => {
        let _ = on_chunk.send(AiChunk {
          delta: None,
          key_index: None,
          error: Some(e.to_string()),
          done: true,
        });
        return Err(e.to_string());
      }
    }
  }
  let _ = on_chunk.send(AiChunk {
    delta: None,
    key_index: None,
    error: None,
    done: true,
  });
  Ok(())
}

fn pick_key(record: &AiProviderRecord) -> Result<(usize, String), String> {
  if record.api_keys.is_empty() {
    return Err("该供应商未配置任何 API Key".to_string());
  }
  let index = next_key_index(record.id, record.api_keys.len());
  let key = record.api_keys.get(index).cloned().ok_or("API Key 池为空")?;
  Ok((index, key))
}

/// 三供应商分别构建模型并进入统一流循环（模型类型不同，无法装箱为一个返回值）
async fn run_provider(
  record: &AiProviderRecord,
  api_key: &str,
  history: Vec<AiChatMessage>,
  preamble: Option<String>,
  key_index: usize,
  on_chunk: &Channel<AiChunk>,
) -> Result<(), String> {
  use rig::client::CompletionClient;
  match record.provider_type.as_str() {
    "openai" => {
      // CompletionsClient = Chat Completions API（OpenAI 兼容端点的最大公约数），
      // 默认 Client 走新 Responses API，部分兼容服务不支持。
      use rig::providers::openai::CompletionsClient;
      let mut builder = CompletionsClient::builder().api_key(api_key);
      if let Some(base_url) = record.base_url.as_deref().filter(|u| !u.trim().is_empty()) {
        builder = builder.base_url(base_url);
      }
      let client = builder.build().map_err(|e| e.to_string())?;
      let model = client.completion_model(record.model.clone());
      stream_model(model, history, preamble, key_index, on_chunk).await
    }
    "anthropic" => {
      use rig::providers::anthropic;
      let mut builder = anthropic::Client::builder().api_key(api_key);
      if let Some(base_url) = record.base_url.as_deref().filter(|u| !u.trim().is_empty()) {
        builder = builder.base_url(base_url);
      }
      let client = builder.build().map_err(|e| e.to_string())?;
      let model = client.completion_model(record.model.clone());
      stream_model(model, history, preamble, key_index, on_chunk).await
    }
    "gemini" => {
      use rig::providers::gemini;
      let mut builder = gemini::Client::builder().api_key(api_key);
      if let Some(base_url) = record.base_url.as_deref().filter(|u| !u.trim().is_empty()) {
        builder = builder.base_url(base_url);
      }
      let client = builder.build().map_err(|e| e.to_string())?;
      let model = client.completion_model(record.model.clone());
      stream_model(model, history, preamble, key_index, on_chunk).await
    }
    other => Err(format!("不支持的供应商类型: {other}")),
  }
}

// ─────────────────────────── Tauri 命令 ───────────────────────────

#[tauri::command]
pub fn ai_provider_list(state: State<'_, DbPool>) -> Result<Vec<AiProviderRecord>, String> {
  list_providers(&state)
}

#[tauri::command]
pub fn ai_provider_save(
  record: AiProviderRecord,
  state: State<'_, DbPool>,
) -> Result<(), String> {
  if record.name.trim().is_empty() {
    return Err("名称不能为空".to_string());
  }
  if record.model.trim().is_empty() {
    return Err("模型不能为空".to_string());
  }
  if record.api_keys.iter().any(|k| k.trim().is_empty()) {
    return Err("API Key 不能为空串".to_string());
  }
  save_provider(&state, &record)
}

#[tauri::command]
pub fn ai_provider_delete(id: usize, state: State<'_, DbPool>) -> Result<(), String> {
  delete_provider(&state, id)
}

/// 统一流式问答：provider_id 为 None 时用默认供应商；多 Key 轮询
#[tauri::command]
pub async fn ai_chat(
  provider_id: Option<usize>,
  history: Vec<AiChatMessage>,
  preamble: Option<String>,
  on_chunk: Channel<AiChunk>,
  state: State<'_, DbPool>,
) -> Result<(), String> {
  let record = match provider_id {
    Some(id) => get_provider(&state, id)?,
    None => default_provider(&state)?.ok_or("尚未配置任何 AI 供应商")?,
  };

  let (key_index, api_key) = pick_key(&record)?;

  // Channel 通知 key 轮询序号（首个事件）
  let _ = on_chunk.send(AiChunk {
    delta: None,
    key_index: Some(key_index),
    error: None,
    done: false,
  });

  run_provider(&record, &api_key, history, preamble, key_index, &on_chunk).await
}

/// 连通性测试：发送一个极短 prompt，返回首个增量或错误
#[tauri::command]
pub async fn ai_provider_test(
  record: AiProviderRecord,
  on_chunk: Channel<AiChunk>,
) -> Result<(), String> {
  let (key_index, api_key) = pick_key(&record)?;
  let history = vec![AiChatMessage {
    role: "user".to_string(),
    content: "回复两个字：连通".to_string(),
  }];
  run_provider(&record, &api_key, history, Some("你是连通性测试助手".to_string()), key_index, &on_chunk).await
}
