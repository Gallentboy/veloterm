//! 一次性种子脚本：把 `seed_package_shell_commands.json` 里的 Linux 包管理 / 权限用户 / 归档压缩 / Shell 与环境 / 定时作业常用命令写入 `commands` 表。
//!
//! 运行方式（在 `src-tauri/` 目录下）：
//!
//! ```sh
//! cargo run --example seed_package_shell_commands
//! ```
//!
//! 设计要点：
//! - 复用 `app_lib::db::init_db_at_path`，因此 SQLCipher 主密钥与连接池定制逻辑和正式程序完全一致；
//! - 幂等：按 `title|command` 去重，重复执行不会产生副本；
//! - id 分配与 `command_library::command_save` 一致（`MAX(id)+1`），因此会接在 batch-1/2/3/4 的 id 之后；
//! - 不修改 `command_library.rs` 中的任何私有函数，仅自行建表 + INSERT。
//!
//! 数据库路径可用 `VELOTERM_DB_PATH` 覆盖，主密钥可用 `VELOTERM_DB_KEY` 覆盖。

use std::collections::HashSet;
use std::path::PathBuf;

use rusqlite::params;

use app_lib::db;

const SEED_JSON: &str = include_str!("seed_package_shell_commands.json");

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeedParam {
    name: String,
    label: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    default: String,
    #[serde(default)]
    example: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeedRecord {
    title: String,
    command: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    purpose: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    params: Vec<SeedParam>,
    #[serde(default)]
    danger_level: String,
    #[serde(default)]
    example_output: String,
}

fn resolve_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("VELOTERM_DB_PATH")
        && !p.trim().is_empty()
    {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("com.gallentboy.veloterm.app")
        .join(db::DEFAULT_DB_FILE_NAME)
}

/// 与 `command_library.rs::ensure_table` 保持一致的建表语句。
fn ensure_commands_table(pool: &db::DbPool) -> Result<(), String> {
    let conn = pool.get().map_err(|e| e.to_string())?;
    conn.execute_batch(
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

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn main() -> Result<(), String> {
    let path = resolve_db_path();
    let key = std::env::var("VELOTERM_DB_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .unwrap_or_else(|| db::DEFAULT_MASTER_KEY.to_string());

    println!("[seed-pkg] db  = {}", path.display());
    println!(
        "[seed-pkg] key = {}",
        if std::env::var("VELOTERM_DB_KEY").is_ok() {
            "来自 VELOTERM_DB_KEY"
        } else {
            "使用内置默认密钥"
        }
    );

    let pool = db::init_db_at_path(&path, &key)?;
    ensure_commands_table(&pool)?;

    let seeds: Vec<SeedRecord> = serde_json::from_str(SEED_JSON)
        .map_err(|e| format!("seed_package_shell_commands.json 解析失败: {e}"))?;
    println!("[seed-pkg] 待写入 {} 条记录", seeds.len());

    let conn = pool.get().map_err(|e| e.to_string())?;

    // 已有记录的 title|command 去重键
    let existing: HashSet<String> = {
        let mut stmt = conn
            .prepare("SELECT title, command FROM commands")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(format!(
                    "{}|{}",
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<HashSet<_>, _>>()
            .map_err(|e| e.to_string())?
    };

    let mut next_id: i64 = conn
        .query_row("SELECT COALESCE(MAX(id), 0) FROM commands", [], |row| {
            row.get(0)
        })
        .map_err(|e| e.to_string())?;

    let mut inserted = 0usize;
    let mut skipped = 0usize;
    let now = now_ms();

    for rec in &seeds {
        let dedup_key = format!("{}|{}", rec.title, rec.command);
        if existing.contains(&dedup_key) {
            println!("[seed-pkg] 跳过(已存在): {}", rec.title);
            skipped += 1;
            continue;
        }
        if rec.title.trim().is_empty() || rec.command.trim().is_empty() {
            println!("[seed-pkg] 跳过(标题或命令为空): {:?}", rec.title);
            skipped += 1;
            continue;
        }

        next_id += 1;
        let danger = match rec.danger_level.trim() {
            "confirm" => "confirm",
            "destructive" => "destructive",
            _ => "safe",
        };

        conn
      .execute(
        "INSERT INTO commands (id, title, command, description, purpose, category, tags, params,
           danger_level, example_output, usage_count, last_used_at, is_pinned, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,0,NULL,0,?11,?11)",
        params![
          next_id,
          &rec.title,
          &rec.command,
          &rec.description,
          &rec.purpose,
          &rec.category,
          serde_json::to_string(&rec.tags).unwrap_or_else(|_| "[]".to_string()),
          serde_json::to_string(
            &rec
              .params
              .iter()
              .map(|p| serde_json::json!({
                "name": p.name,
                "label": p.label,
                "description": p.description,
                "required": p.required,
                "default": p.default,
                "example": p.example,
              }))
              .collect::<Vec<_>>()
          )
          .unwrap_or_else(|_| "[]".to_string()),
          danger,
          &rec.example_output,
          now,
        ],
      )
      .map_err(|e| format!("插入 {} 失败: {e}", rec.title))?;

        println!("[seed-pkg] 写入 id={:<3} {}", next_id, rec.title);
        inserted += 1;
    }

    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM commands", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;

    println!("[seed-pkg] 完成：新增 {inserted} 条，跳过 {skipped} 条，表内共 {total} 条");

    // 回读校验：确认 JSON 列可正常解析、排序键与统计列符合预期
    println!("[seed-pkg] ---- 回读校验 ----");
    let mut stmt = conn
    .prepare(
      "SELECT id, title, category, danger_level, tags, params, usage_count, is_pinned, last_used_at, created_at
       FROM commands
       ORDER BY is_pinned DESC, usage_count DESC, COALESCE(last_used_at, 0) DESC, id ASC",
    )
    .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, i64>(9)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    for row in rows {
        let (
            id,
            title,
            category,
            danger,
            tags_json,
            params_json,
            usage,
            pinned,
            last_used,
            created,
        ) = row.map_err(|e| e.to_string())?;
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
        let params: Vec<serde_json::Value> = serde_json::from_str(&params_json).unwrap_or_default();
        let param_names: Vec<String> = params
            .iter()
            .filter_map(|p| p.get("name").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        println!(
            "[seed-pkg] id={:<3} {:<8} {:<11} pinned={} used={} last_used={:?} created={} tags={:?} params={:?} | {}",
            id, category, danger, pinned, usage, last_used, created, tags, param_names, title
        );
    }

    Ok(())
}
