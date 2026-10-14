//! 常用命令库：个人运维知识库 + 快速启动器。
//!
//! - 单表 `commands`，扁平结构（一条记录 = 一个使用意图，tags 关联同命令多用法）
//! - `params` 列为结构化 JSON（占位符自动提取 + 用户补充说明）
//! - 全字段模糊搜索（title/command/description/purpose/category/tags）
//! - 排序：is_pinned DESC → usage_count DESC → last_used_at DESC

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::DbPool;

// ─────────────────────── 数据模型 ───────────────────────

/// 命令参数（占位符自动提取后用户补充的结构化说明）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommandParam {
  /// 占位符名（与 command 中的 {{name}} 对应） */
  pub name: String,
  /// 显示标签 */
  pub label: String,
  /// 参数说明 */
  #[serde(default)]
  pub description: String,
  /// 是否必填 */
  #[serde(default)]
  pub required: bool,
  /// 默认值（执行时预填） */
  #[serde(default)]
  pub default: String,
  /// 示例值 */
  #[serde(default)]
  pub example: String,
}

/// 危险级别 */
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DangerLevel {
  #[default]
  Safe,
  Confirm,
  Destructive,
}

/// 常用命令记录 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRecord {
  pub id: usize,
  /// 显示名（如「sed 全局替换文件内容」） */
  pub title: String,
  /// 命令模板（支持 {{占位符}}） */
  pub command: String,
  /// 详细解释：命令做什么、flags 含义 */
  pub description: String,
  /// 使用场景/用途 */
  pub purpose: String,
  /// 单级分类（text / network / disk / docker / gpu / ssh / other） */
  pub category: String,
  /// 自由标签 JSON 数组（如 ["sed","替换"]） */
  pub tags: Vec<String>,
  /// 结构化参数说明 */
  pub params: Vec<CommandParam>,
  pub danger_level: DangerLevel,
  /// 示例输出（可选，帮助回忆格式） */
  pub example_output: String,
  pub usage_count: usize,
  pub last_used_at: Option<i64>,
  pub is_pinned: bool,
  pub created_at: i64,
  pub updated_at: i64,
}

// ─────────────────────── 数据库 ───────────────────────

fn ensure_table(pool: &DbPool) -> Result<(), String> {
  let conn = pool.get().map_err(|e| e.to_string())?;
  conn
    .execute_batch(
      "CREATE TABLE IF NOT EXISTS commands (
        id             INTEGER PRIMARY KEY,
        title          TEXT NOT NULL,
        command        TEXT NOT NULL,
        description    TEXT DEFAULT '',
        purpose        TEXT DEFAULT '',
        category       TEXT DEFAULT '',
        tags           TEXT DEFAULT '[]',
        params         TEXT DEFAULT '[]',
        danger_level   TEXT DEFAULT 'safe',
        example_output TEXT DEFAULT '',
        usage_count    INTEGER DEFAULT 0,
        last_used_at   INTEGER,
        is_pinned      INTEGER DEFAULT 0,
        created_at     INTEGER NOT NULL,
        updated_at     INTEGER NOT NULL
      );",
    )
    .map_err(|e| e.to_string())?;
  Ok(())
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<CommandRecord> {
  let tags_json: String = row.get("tags")?;
  let params_json: String = row.get("params")?;
  let danger_str: String = row.get("danger_level")?;
  let last_used: Option<i64> = row.get("last_used_at")?;
  let is_pinned: i64 = row.get("is_pinned")?;

  let danger_level = match danger_str.as_str() {
    "confirm" => DangerLevel::Confirm,
    "destructive" => DangerLevel::Destructive,
    _ => DangerLevel::Safe,
  };

  Ok(CommandRecord {
    id: row.get::<_, i64>("id")? as usize,
    title: row.get("title")?,
    command: row.get("command")?,
    description: row.get("description")?,
    purpose: row.get("purpose")?,
    category: row.get("category")?,
    tags: serde_json::from_str(&tags_json).unwrap_or_default(),
    params: serde_json::from_str(&params_json).unwrap_or_default(),
    danger_level,
    example_output: row.get("example_output")?,
    usage_count: row.get::<_, i64>("usage_count")? as usize,
    last_used_at: last_used,
    is_pinned: is_pinned != 0,
    created_at: row.get("created_at")?,
    updated_at: row.get("updated_at")?,
  })
}


// ─────────────────────── Tauri 命令 ───────────────────────

/// 全字段模糊搜索；keyword 为 None/空时返回全量。
/// 排序：is_pinned DESC → usage_count DESC → last_used_at DESC
#[tauri::command]
pub fn command_list(
  keyword: Option<String>,
  state: State<'_, DbPool>,
) -> Result<Vec<CommandRecord>, String> {
  ensure_table(&state)?;
  let conn = state.get().map_err(|e| e.to_string())?;

  let kw = keyword.unwrap_or_default().trim().to_string();
  if kw.is_empty() {
    let mut stmt = conn
      .prepare(
        "SELECT * FROM commands
         ORDER BY is_pinned DESC, usage_count DESC, COALESCE(last_used_at, 0) DESC",
      )
      .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], row_to_record).map_err(|e| e.to_string())?;
    return rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string());
  }

  let pattern = format!("%{kw}%");
  let mut stmt = conn
    .prepare(
      "SELECT * FROM commands
       WHERE title LIKE ?1 OR command LIKE ?1 OR description LIKE ?1
          OR purpose LIKE ?1 OR category LIKE ?1 OR tags LIKE ?1
       ORDER BY is_pinned DESC, usage_count DESC, COALESCE(last_used_at, 0) DESC",
    )
    .map_err(|e| e.to_string())?;
  let rows = stmt
    .query_map([&pattern], row_to_record)
    .map_err(|e| e.to_string())?;
  rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// 获取下一个可用 id（新记录前端传 0，后端分配）
fn next_command_id(pool: &DbPool) -> Result<usize, String> {
  let conn = pool.get().map_err(|e| e.to_string())?;
  let max: i64 = conn
    .query_row("SELECT COALESCE(MAX(id), 0) FROM commands", [], |row| row.get(0))
    .map_err(|e| e.to_string())?;
  Ok((max + 1) as usize)
}

#[tauri::command]
pub fn command_save(
  mut record: CommandRecord,
  state: State<'_, DbPool>,
) -> Result<CommandRecord, String> {
  ensure_table(&state)?;
  save_command_impl(&mut record, state.inner())?;
  Ok(record)
}

fn save_command_impl(record: &mut CommandRecord, pool: &DbPool) -> Result<(), String> {
  if record.title.trim().is_empty() {
    return Err("标题不能为空".to_string());
  }
  if record.command.trim().is_empty() {
    return Err("命令不能为空".to_string());
  }

  if record.id == 0 {
    record.id = next_command_id(pool)?;
    record.created_at = chrono_now_ms();
  }
  record.updated_at = chrono_now_ms();

  let conn = pool.get().map_err(|e| e.to_string())?;
  conn
    .execute(
      "INSERT INTO commands (id, title, command, description, purpose, category, tags, params,
         danger_level, example_output, usage_count, last_used_at, is_pinned, created_at, updated_at)
       VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
       ON CONFLICT(id) DO UPDATE SET
         title = excluded.title,
         command = excluded.command,
         description = excluded.description,
         purpose = excluded.purpose,
         category = excluded.category,
         tags = excluded.tags,
         params = excluded.params,
         danger_level = excluded.danger_level,
         example_output = excluded.example_output,
         is_pinned = excluded.is_pinned,
         updated_at = excluded.updated_at",
      rusqlite::params![
        record.id as i64,
        record.title,
        record.command,
        record.description,
        record.purpose,
        record.category,
        serde_json::to_string(&record.tags).unwrap_or_else(|_| "[]".to_string()),
        serde_json::to_string(&record.params).unwrap_or_else(|_| "[]".to_string()),
        match record.danger_level { DangerLevel::Safe => "safe", DangerLevel::Confirm => "confirm", DangerLevel::Destructive => "destructive" },
        record.example_output,
        record.usage_count as i64,
        record.last_used_at,
        record.is_pinned as i64,
        record.created_at,
        record.updated_at,
      ],
    )
    .map_err(|e| e.to_string())?;
  Ok(())
}

#[tauri::command]
pub fn command_delete(id: usize, state: State<'_, DbPool>) -> Result<(), String> {
  ensure_table(&state)?;
  let conn = state.get().map_err(|e| e.to_string())?;
  conn
    .execute("DELETE FROM commands WHERE id = ?1", [id as i64])
    .map_err(|e| e.to_string())?;
  Ok(())
}

/// 记录一次使用：递增 usage_count + 更新 last_used_at
#[tauri::command]
pub fn command_used(id: usize, state: State<'_, DbPool>) -> Result<(), String> {
  ensure_table(&state)?;
  let conn = state.get().map_err(|e| e.to_string())?;
  conn
    .execute(
      "UPDATE commands SET usage_count = usage_count + 1, last_used_at = ?2 WHERE id = ?1",
      rusqlite::params![id as i64, chrono_now_ms()],
    )
    .map_err(|e| e.to_string())?;
  Ok(())
}

/// JSON 全量导出
#[tauri::command]
pub fn command_export(state: State<'_, DbPool>) -> Result<String, String> {
  let all = command_list(None, state)?;
  serde_json::to_string_pretty(&all).map_err(|e| e.to_string())
}

/// 合并导入（按 title+command 去重）
#[tauri::command]
pub fn command_import(json: String, state: State<'_, DbPool>) -> Result<usize, String> {
  let incoming: Vec<CommandRecord> =
    serde_json::from_str(&json).map_err(|e| format!("JSON 解析失败: {e}"))?;
  let existing: Vec<CommandRecord> = {
    let conn = state.get().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare("SELECT * FROM commands ORDER BY is_pinned DESC, usage_count DESC").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], row_to_record).map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?
  };
  let existing_keys: std::collections::HashSet<String> = existing
    .iter()
    .map(|r| format!("{}|{}", r.title, r.command))
    .collect();

  let mut imported = 0;
  for mut record in incoming {
    let key = format!("{}|{}", record.title, record.command);
    if existing_keys.contains(&key) {
      continue;
    }
    record.id = 0; // 强制分配新 id
    save_command_impl(&mut record, state.inner())?;
    imported += 1;
  }
  Ok(imported)
}

fn chrono_now_ms() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_millis() as i64)
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn danger_level_serialization() {
    assert_eq!(serde_json::to_string(&DangerLevel::Safe).unwrap(), "\"safe\"");
    assert_eq!(
      serde_json::to_string(&DangerLevel::Destructive).unwrap(),
      "\"destructive\""
    );
    let parsed: DangerLevel = serde_json::from_str("\"confirm\"").unwrap();
    assert_eq!(parsed, DangerLevel::Confirm);
  }

  #[test]
  fn command_param_roundtrip() {
    let param = CommandParam {
      name: "file".to_string(),
      label: "目标文件".to_string(),
      description: "要修改的文件路径".to_string(),
      required: true,
      default: String::new(),
      example: "/etc/config".to_string(),
    };
    let json = serde_json::to_string(&param).unwrap();
    let parsed: CommandParam = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, param);
  }
}
