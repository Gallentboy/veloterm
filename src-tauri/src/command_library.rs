//! 常用命令库：个人运维知识库 + 快速启动器。
//!
//! - 单表 `commands`，扁平结构（一条记录 = 一个使用意图，tags 关联同命令多用法）
//! - `params` 列为结构化 JSON（占位符自动提取 + 用户补充说明）
//! - `uuid` 列为导入导出判重键（存量行迁移回填 + 唯一索引；无 uuid 的旧条目按
//!   `title|command` 兜底匹配）
//! - 全字段模糊搜索（title/command/description/purpose/category/tags）
//! - 排序：is_pinned DESC → usage_count DESC → last_used_at DESC
//! - 导入导出：明文 JSON v1 信封（兼容旧版裸数组）与 VTCD 二进制容器（deflate + SHA-256）

use std::collections::HashMap;
use std::io::{Read, Write};

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

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
    /// 稳定标识：导入导出判重键（迁移回填存量行，保存路径兜底生成） */
    #[serde(default)]
    pub uuid: String,
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
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS commands (
        id             INTEGER PRIMARY KEY,
        uuid           TEXT NOT NULL DEFAULT '',
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

    // 列级迁移（在建表之后）：老库升级补 uuid 列。SQLite 的 ADD COLUMN 无 IF NOT EXISTS，
    // 列已存在时报 duplicate column，吞掉该错误即幂等；新库建表已含该列，同样走这条路。
    if let Err(e) = conn.execute_batch("ALTER TABLE commands ADD COLUMN uuid TEXT NOT NULL DEFAULT '';")
        && !e.to_string().contains("duplicate column")
    {
        return Err(format!("failed to apply column migration `uuid`: {e}"));
    }
    // 存量行一次性回填 v4 形态 uuid（8-4-4-4-12，SQL 内生成）——必须在建唯一索引前完成
    conn.execute_batch(
        "UPDATE commands
        SET uuid = lower(hex(randomblob(4)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(2)))
             ||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(6)))
        WHERE uuid = '';",
    )
    .map_err(|e| e.to_string())?;
    conn.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS idx_commands_uuid ON commands(uuid);")
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
        uuid: row.get("uuid")?,
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
        let rows = stmt
            .query_map([], row_to_record)
            .map_err(|e| e.to_string())?;
        return rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
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
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// 获取下一个可用 id（新记录前端传 0，后端分配）
fn next_command_id(pool: &DbPool) -> Result<usize, String> {
    let conn = pool.get().map_err(|e| e.to_string())?;
    let max: i64 = conn
        .query_row("SELECT COALESCE(MAX(id), 0) FROM commands", [], |row| {
            row.get(0)
        })
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
    // uuid 列带唯一索引：空 uuid（新记录或旧负载）兜底生成，避免撞唯一约束；
    // ON CONFLICT 更新分支不写 uuid，本地 uuid 始终保留
    if record.uuid.trim().is_empty() {
        record.uuid = Uuid::new_v4().to_string();
    }
    record.updated_at = chrono_now_ms();

    let conn = pool.get().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO commands (id, uuid, title, command, description, purpose, category, tags, params,
         danger_level, example_output, usage_count, last_used_at, is_pinned, created_at, updated_at)
       VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
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
            record.uuid,
            record.title,
            record.command,
            record.description,
            record.purpose,
            record.category,
            serde_json::to_string(&record.tags).unwrap_or_else(|_| "[]".to_string()),
            serde_json::to_string(&record.params).unwrap_or_else(|_| "[]".to_string()),
            match record.danger_level {
                DangerLevel::Safe => "safe",
                DangerLevel::Confirm => "confirm",
                DangerLevel::Destructive => "destructive",
            },
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
    conn.execute("DELETE FROM commands WHERE id = ?1", [id as i64])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ─────────────────────── 内置预置命令（种子） ───────────────────────

/// 预置命令集：由 `scripts/build-command-seed.mjs` 从 `src-tauri/seed-parts/`
/// 分片合并产出（内容按官方文档求证：GNU 手册 / Microsoft Learn / 各上游仓库，
/// 字段契约与校验规则见脚本头注释）。
const BUILTIN_SEED_JSON: &str = include_str!("../assets/builtin-commands-seed.json");

/// app_kv 版本标记键：种子只在该版本首次启动执行一次。
const SEED_KV_KEY: &str = "builtin-commands-seed:version";
/// 当前种子数据版本（数据变更时递增：升级后首个启动补插新增条目，已有条目
/// 与用户删除操作不受影响）。
const SEED_VERSION: &str = "1";

/// 种子条目：CommandRecord 的子集（不含本机数据 id/usage_count/last_used_at/
/// updated_at，均由播种路径统一初始化）。种子 JSON 里的 `platforms` 字段
///（适配系统多选，恒 ["win"]）为前瞻数据，当前 schema 未落库——serde 默认
/// 忽略未知字段，待适配系统列合入后在此接线。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeedEntry {
    uuid: String,
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
    params: Vec<CommandParam>,
    #[serde(default)]
    danger_level: DangerLevel,
    #[serde(default)]
    example_output: String,
    #[serde(default)]
    is_pinned: bool,
    #[serde(default)]
    created_at: i64,
}

/// 播种内置预置命令（内置工具链全量命令 + Windows cmd 常用命令的详细用法）。
/// 幂等三层：kv 版本标记短路（同版本只跑一次，用户删除的预置条目不会复活）
/// → uuid 唯一索引 + INSERT OR IGNORE（版本升级补插时与已有条目去重）
/// → ensure_table 建表幂等。返回 (本次插入数, 跳过数)。
pub fn seed_builtin_commands(pool: &DbPool) -> Result<(usize, usize), String> {
    ensure_table(pool)?;
    if crate::db::get_kv(pool, SEED_KV_KEY)? == Some(SEED_VERSION.to_string()) {
        return Ok((0, 0));
    }
    let entries: Vec<SeedEntry> = serde_json::from_str(BUILTIN_SEED_JSON)
        .map_err(|e| format!("解析内置预置命令 JSON 失败: {e}"))?;
    let now = crate::db::now_timestamp_ms();
    let mut inserted = 0usize;
    {
        let mut conn = pool.get().map_err(|e| e.to_string())?;
        for e in &entries {
            let changed = conn
                .execute(
                    "INSERT OR IGNORE INTO commands
                       (uuid, title, command, description, purpose, category, tags,
                        params, danger_level, example_output,
                        usage_count, is_pinned, created_at, updated_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,0,?11,?12,?13)",
                    rusqlite::params![
                        e.uuid,
                        e.title,
                        e.command,
                        e.description,
                        e.purpose,
                        e.category,
                        serde_json::to_string(&e.tags).unwrap_or_else(|_| "[]".to_string()),
                        serde_json::to_string(&e.params).unwrap_or_else(|_| "[]".to_string()),
                        match e.danger_level {
                            DangerLevel::Safe => "safe",
                            DangerLevel::Confirm => "confirm",
                            DangerLevel::Destructive => "destructive",
                        },
                        e.example_output,
                        e.is_pinned as i64,
                        e.created_at,
                        now,
                    ],
                )
                .map_err(|err| format!("播种预置命令失败（uuid={}）: {err}", e.uuid))?;
            inserted += changed;
        }
    }
    crate::db::set_kv(pool, SEED_KV_KEY, SEED_VERSION)?;
    Ok((inserted, entries.len() - inserted))
}

/// 记录一次使用：递增 usage_count + 更新 last_used_at
#[tauri::command]
pub fn command_used(id: usize, state: State<'_, DbPool>) -> Result<(), String> {
    ensure_table(&state)?;
    let conn = state.get().map_err(|e| e.to_string())?;
    conn.execute(
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
        let mut stmt = conn
            .prepare("SELECT * FROM commands ORDER BY is_pinned DESC, usage_count DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], row_to_record)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
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

// ─────────────────────── 导入 / 导出（文件载体） ───────────────────────

/// v1 信封的 format 标识。
const EXPORT_FORMAT: &str = "veloterm-commands";
/// v1 信封 / VTCD 容器的当前版本。
const EXPORT_VERSION: u64 = 1;

/// VTCD 二进制容器布局：
/// magic 4B("VTCD") + 版本 u16LE + flags u16LE + payload 长度 u32LE + payload + SHA-256(payload) 32B
const VTCD_MAGIC: &[u8; 4] = b"VTCD";
const VTCD_HEADER_LEN: usize = 12;
const VTCD_DIGEST_LEN: usize = 32;
/// flags bit0：payload 为 deflate 压缩（本版本恒为 1）。
const VTCD_FLAG_DEFLATE: u16 = 1;

/// `command_export_file` 的返回。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFileResult {
    pub path: String,
    pub count: usize,
}

/// `command_import_start` 的返回。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStartResult {
    pub path: String,
    pub preview: ImportPreview,
}

/// 导入预览（dry-run，不落库）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub new_count: usize,
    pub conflicts: Vec<ConflictItem>,
    pub invalid: Vec<InvalidItem>,
}

/// 与库内已有命令命中（uuid 或 title|command 兜底）的导入条目。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictItem {
    pub uuid: Option<String>,
    pub title: String,
    pub command: String,
}

/// 行级校验失败的导入条目（index 为其在 commands 数组中的 0 基下标）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvalidItem {
    pub index: usize,
    pub title: Option<String>,
    pub reason: String,
}

/// `command_import_apply` 的返回。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportApplyResult {
    pub inserted: usize,
    pub updated: usize,
    pub skipped: usize,
    pub invalid: usize,
}

/// 导出条目（camelCase 契约）。不含 id / usageCount / lastUsedAt / updatedAt ——
/// id 与使用统计属本机数据，updatedAt 由导入端写入时重算。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportEntry {
    uuid: String,
    title: String,
    command: String,
    description: String,
    purpose: String,
    category: String,
    tags: Vec<String>,
    /// 原 params JSON 结构原样（数组）。
    params: serde_json::Value,
    danger_level: DangerLevel,
    example_output: String,
    is_pinned: bool,
    created_at: i64,
}

/// 明文 JSON 载体的 v1 信封。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportEnvelope {
    format: &'static str,
    version: u64,
    exported_at: i64,
    count: usize,
    commands: Vec<ExportEntry>,
}

// ── VTCD 二进制容器 ──

fn le_u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?))
}

fn le_u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
}

/// payload = ZlibEncoder 压缩的紧凑 JSON，尾部附 SHA-256(payload)。
fn encode_vtcmd(json: &str) -> Result<Vec<u8>, String> {
    let mut encoder = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(json.as_bytes())
        .map_err(|e| format!("压缩失败: {e}"))?;
    let payload = encoder.finish().map_err(|e| format!("压缩失败: {e}"))?;
    let payload_len =
        u32::try_from(payload.len()).map_err(|_| "导出内容过大".to_string())?;

    let mut out = Vec::with_capacity(VTCD_HEADER_LEN + payload.len() + VTCD_DIGEST_LEN);
    out.extend_from_slice(VTCD_MAGIC);
    out.extend_from_slice(&1u16.to_le_bytes()); // 容器版本
    out.extend_from_slice(&VTCD_FLAG_DEFLATE.to_le_bytes());
    out.extend_from_slice(&payload_len.to_le_bytes());
    out.extend_from_slice(&payload);
    out.extend_from_slice(&Sha256::digest(&payload));
    Ok(out)
}

/// 校验 magic / 版本 / flags / 长度 / SHA-256，全部通过后解压返回 JSON 字符串。
fn decode_vtcmd(bytes: &[u8]) -> Result<String, String> {
    if bytes.get(0..4) != Some(VTCD_MAGIC.as_slice()) {
        return Err("不是有效的 VTCD 命令库文件".to_string());
    }
    let version = le_u16_at(bytes, 4).ok_or("文件校验失败，可能已损坏")?;
    if version != 1 {
        return Err(format!("不支持的容器版本: {version}"));
    }
    let flags = le_u16_at(bytes, 6).ok_or("文件校验失败，可能已损坏")?;
    if flags != VTCD_FLAG_DEFLATE {
        return Err(format!("不支持的容器 flags: {flags}"));
    }
    let payload_len = le_u32_at(bytes, 8).ok_or("文件校验失败，可能已损坏")? as usize;
    let total = VTCD_HEADER_LEN + payload_len + VTCD_DIGEST_LEN;
    if bytes.len() != total {
        return Err("文件校验失败，可能已损坏".to_string());
    }
    let payload = bytes
        .get(VTCD_HEADER_LEN..VTCD_HEADER_LEN + payload_len)
        .ok_or("文件校验失败，可能已损坏")?;
    let digest = bytes
        .get(VTCD_HEADER_LEN + payload_len..total)
        .ok_or("文件校验失败，可能已损坏")?;
    if Sha256::digest(payload).as_slice() != digest {
        return Err("文件校验失败，可能已损坏".to_string());
    }

    let mut decoder = ZlibDecoder::new(payload);
    let mut json_bytes = Vec::new();
    decoder
        .read_to_end(&mut json_bytes)
        .map_err(|e| format!("解压失败: {e}"))?;
    String::from_utf8(json_bytes).map_err(|_| "文件不是有效的 UTF-8 JSON".to_string())
}

// ── 载体识别与解析 ──

/// 解析 JSON 根，三种形态都接受：
/// ① v1 信封对象（校验 format=="veloterm-commands" 或缺省；version>1 整体 Err）
/// ② 旧版裸数组（`command_export` 历史产物，条目无 uuid 走 title|command 兜底）
/// ③ 空数组合法（全 0 预览）
fn parse_commands_payload(json: &str) -> Result<Vec<serde_json::Value>, String> {
    let root: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("JSON 解析失败: {e}"))?;
    match root {
        serde_json::Value::Array(items) => Ok(items),
        serde_json::Value::Object(map) => {
            if let Some(format_field) = map.get("format")
                && format_field.as_str() != Some(EXPORT_FORMAT)
            {
                return Err("不支持的导出格式（format 字段非法）".to_string());
            }
            if let Some(version) = map.get("version") {
                let n = version
                    .as_u64()
                    .ok_or_else(|| "不支持的导出版本（version 字段非法）".to_string())?;
                if n > EXPORT_VERSION {
                    return Err(format!("不支持的导出版本: {n}"));
                }
            }
            match map.get("commands") {
                Some(serde_json::Value::Array(items)) => Ok(items.clone()),
                _ => Err("文件缺少 commands 数组".to_string()),
            }
        }
        _ => Err("无法识别的文件结构".to_string()),
    }
}

/// 读文件 + 识别载体：前 4 字节为 "VTCD" 走二进制容器，否则按 UTF-8 JSON 解析。
/// 仅整文件级问题（magic/版本/校验和/根 JSON 解析失败）在此返回 Err。
fn read_import_file(path: &str) -> Result<Vec<serde_json::Value>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
    let json = if bytes.get(0..4) == Some(VTCD_MAGIC.as_slice()) {
        decode_vtcmd(&bytes)?
    } else {
        String::from_utf8(bytes).map_err(|_| "文件不是有效的 UTF-8 JSON 文本".to_string())?
    };
    parse_commands_payload(&json)
}

// ── 行级校验 ──

/// 行级校验通过后的导入条目。
#[derive(Debug, Clone)]
struct ImportEntry {
    uuid: Option<String>,
    title: String,
    command: String,
    description: String,
    purpose: String,
    category: String,
    tags: Vec<String>,
    params: serde_json::Value,
    danger_level: DangerLevel,
    example_output: String,
    is_pinned: bool,
    created_at: i64,
}

const DANGER_LEVEL_VALUES: &str = "safe/confirm/destructive";

fn json_string_field(
    map: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    map.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

/// 单条目校验：title/command 必填非空，tags/params 必须是数组，dangerLevel 取值合法。
/// 失败返回中文 reason（行级，不阻塞其他条目）。其余字段缺省/类型不符时宽容降级。
fn parse_import_entry(value: &serde_json::Value) -> Result<ImportEntry, String> {
    let Some(obj) = value.as_object() else {
        return Err("条目不是 JSON 对象".to_string());
    };

    let title = json_string_field(obj, "title").unwrap_or_default();
    if title.trim().is_empty() {
        return Err("缺少 title".to_string());
    }
    let command = json_string_field(obj, "command").unwrap_or_default();
    if command.trim().is_empty() {
        return Err("缺少 command".to_string());
    }

    let tags = match obj.get("tags") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(items)) => {
            let mut tags = Vec::with_capacity(items.len());
            for item in items {
                let Some(tag) = item.as_str() else {
                    return Err("tags 结构非法".to_string());
                };
                tags.push(tag.to_string());
            }
            tags
        }
        Some(_) => return Err("tags 结构非法".to_string()),
    };

    let params = match obj.get("params") {
        None | Some(serde_json::Value::Null) => serde_json::Value::Array(Vec::new()),
        Some(value @ serde_json::Value::Array(_)) => value.clone(),
        Some(_) => return Err("params 结构非法".to_string()),
    };

    let danger_level = match obj.get("dangerLevel").and_then(|v| v.as_str()) {
        None | Some("safe") => DangerLevel::Safe,
        Some("confirm") => DangerLevel::Confirm,
        Some("destructive") => DangerLevel::Destructive,
        Some(_) => return Err(format!("dangerLevel 非法（应为 {DANGER_LEVEL_VALUES}）")),
    };

    Ok(ImportEntry {
        uuid: json_string_field(obj, "uuid").filter(|u| !u.is_empty()),
        title,
        command,
        description: json_string_field(obj, "description").unwrap_or_default(),
        purpose: json_string_field(obj, "purpose").unwrap_or_default(),
        category: json_string_field(obj, "category").unwrap_or_default(),
        tags,
        params,
        danger_level,
        example_output: json_string_field(obj, "exampleOutput").unwrap_or_default(),
        is_pinned: obj.get("isPinned").and_then(|v| v.as_bool()).unwrap_or(false),
        created_at: obj
            .get("createdAt")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(chrono_now_ms),
    })
}

/// 逐条校验，拆出有效条目与无效清单（无效行只记录，不阻塞有效行）。
fn prepare_import_entries(items: Vec<serde_json::Value>) -> (Vec<ImportEntry>, Vec<InvalidItem>) {
    let mut valid = Vec::new();
    let mut invalid = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        let title_hint = item
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.is_empty());
        match parse_import_entry(&item) {
            Ok(entry) => valid.push(entry),
            Err(reason) => invalid.push(InvalidItem {
                index,
                title: title_hint,
                reason,
            }),
        }
    }
    (valid, invalid)
}

// ── 与库内比对 ──

/// 库内已有命令的判重索引：uuid 优先，title|command 兜底。
#[derive(Default)]
struct ExistingIndex {
    by_uuid: HashMap<String, i64>,
    by_key: HashMap<String, i64>,
}

fn conflict_key(title: &str, command: &str) -> String {
    format!("{title}|{command}")
}

fn load_existing_index(conn: &rusqlite::Connection) -> Result<ExistingIndex, String> {
    let mut stmt = conn
        .prepare("SELECT id, uuid, title, command FROM commands")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut index = ExistingIndex::default();
    for row in rows {
        let (id, uuid, title, command) = row.map_err(|e| e.to_string())?;
        if !uuid.is_empty() {
            index.by_uuid.insert(uuid, id);
        }
        index.by_key.insert(conflict_key(&title, &command), id);
    }
    Ok(index)
}

/// 判重：条目有非空 uuid → 按 uuid 匹配；否则（或未命中）按 title|command 精确匹配。
fn find_existing(index: &ExistingIndex, entry: &ImportEntry) -> Option<i64> {
    if let Some(uuid) = &entry.uuid
        && let Some(id) = index.by_uuid.get(uuid)
    {
        return Some(*id);
    }
    index
        .by_key
        .get(&conflict_key(&entry.title, &entry.command))
        .copied()
}

/// 插入后同步索引，保证同文件内后续条目能看到刚插入的行（同文件重复 uuid 不撞唯一索引）。
fn remember_inserted(index: &mut ExistingIndex, id: i64, uuid: &str, entry: &ImportEntry) {
    index.by_uuid.insert(uuid.to_string(), id);
    index
        .by_key
        .insert(conflict_key(&entry.title, &entry.command), id);
}

// ── 导出 / 导入实现（与 Tauri 命令壳分离，便于单测） ──

/// 全量命令（排序与 command_list 全量分支一致）。
fn list_all_records(pool: &DbPool) -> Result<Vec<CommandRecord>, String> {
    let conn = pool.get().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT * FROM commands
         ORDER BY is_pinned DESC, usage_count DESC, COALESCE(last_used_at, 0) DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], row_to_record).map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn record_to_export_entry(record: &CommandRecord) -> ExportEntry {
    ExportEntry {
        uuid: record.uuid.clone(),
        title: record.title.clone(),
        command: record.command.clone(),
        description: record.description.clone(),
        purpose: record.purpose.clone(),
        category: record.category.clone(),
        tags: record.tags.clone(),
        params: serde_json::to_value(&record.params)
            .unwrap_or(serde_json::Value::Array(Vec::new())),
        danger_level: record.danger_level.clone(),
        example_output: record.example_output.clone(),
        is_pinned: record.is_pinned,
        created_at: record.created_at,
    }
}

/// 生成 v1 信封的两份 JSON：.json 文件落 pretty，.vtcmd payload 用紧凑版。
fn build_export_payload(records: &[CommandRecord]) -> Result<(String, String), String> {
    let entries: Vec<ExportEntry> = records.iter().map(record_to_export_entry).collect();
    let envelope = ExportEnvelope {
        format: EXPORT_FORMAT,
        version: EXPORT_VERSION,
        exported_at: chrono_now_ms(),
        count: entries.len(),
        commands: entries,
    };
    let pretty = serde_json::to_string_pretty(&envelope).map_err(|e| e.to_string())?;
    let compact = serde_json::to_string(&envelope).map_err(|e| e.to_string())?;
    Ok((pretty, compact))
}

fn export_file_impl(pool: &DbPool, path: &str, format: &str) -> Result<ExportFileResult, String> {
    let records = list_all_records(pool)?;
    let (pretty, compact) = build_export_payload(&records)?;
    let bytes = match format {
        "vtcmd" => encode_vtcmd(&compact)?,
        _ => pretty.into_bytes(),
    };
    std::fs::write(path, bytes).map_err(|e| format!("写入文件失败: {e}"))?;
    Ok(ExportFileResult {
        path: path.to_string(),
        count: records.len(),
    })
}

/// 导入预览（dry-run 不落库）：读文件 + 行级校验 + 与库内比对。
fn import_preview_impl(pool: &DbPool, path: &str) -> Result<ImportPreview, String> {
    ensure_table(pool)?;
    let items = read_import_file(path)?;
    let (valid, invalid) = prepare_import_entries(items);
    let conn = pool.get().map_err(|e| e.to_string())?;
    let index = load_existing_index(&conn)?;

    let mut new_count = 0usize;
    let mut conflicts = Vec::new();
    for entry in &valid {
        if find_existing(&index, entry).is_some() {
            conflicts.push(ConflictItem {
                uuid: entry.uuid.clone(),
                title: entry.title.clone(),
                command: entry.command.clone(),
            });
        } else {
            new_count += 1;
        }
    }
    Ok(ImportPreview {
        new_count,
        conflicts,
        invalid,
    })
}

fn danger_level_str(level: &DangerLevel) -> &'static str {
    match level {
        DangerLevel::Safe => "safe",
        DangerLevel::Confirm => "confirm",
        DangerLevel::Destructive => "destructive",
    }
}

/// 导入插入：id 自增分配、uuid 为空则生成、created_at 用条目值（缺省 now）、
/// updated_at=now、usage_count=0、last_used_at=NULL。
fn insert_imported_entry(
    conn: &rusqlite::Connection,
    entry: &ImportEntry,
    uuid: &str,
    id: i64,
    now_ms: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO commands (id, uuid, title, command, description, purpose, category, tags, params,
         danger_level, example_output, usage_count, last_used_at, is_pinned, created_at, updated_at)
       VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,0,NULL,?12,?13,?14)",
        rusqlite::params![
            id,
            uuid,
            entry.title,
            entry.command,
            entry.description,
            entry.purpose,
            entry.category,
            serde_json::to_string(&entry.tags).unwrap_or_else(|_| "[]".to_string()),
            serde_json::to_string(&entry.params).unwrap_or_else(|_| "[]".to_string()),
            danger_level_str(&entry.danger_level),
            entry.example_output,
            entry.is_pinned as i64,
            entry.created_at,
            now_ms,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// overwrite 策略：只更新导出字段，保留本地 id/uuid/created_at/usage_count/last_used_at；
/// updated_at 重算。
fn overwrite_imported_entry(
    conn: &rusqlite::Connection,
    id: i64,
    entry: &ImportEntry,
    now_ms: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE commands SET title=?2, command=?3, description=?4, purpose=?5, category=?6,
         tags=?7, params=?8, danger_level=?9, example_output=?10, is_pinned=?11, updated_at=?12
       WHERE id=?1",
        rusqlite::params![
            id,
            entry.title,
            entry.command,
            entry.description,
            entry.purpose,
            entry.category,
            serde_json::to_string(&entry.tags).unwrap_or_else(|_| "[]".to_string()),
            serde_json::to_string(&entry.params).unwrap_or_else(|_| "[]".to_string()),
            danger_level_str(&entry.danger_level),
            entry.example_output,
            entry.is_pinned as i64,
            now_ms,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 按策略落库：skip（命中跳过）/ overwrite（覆盖导出字段）/ duplicate（命中也插入，
/// 重新生成 uuid）。重新从 path 读文件解析（与预览两阶段各自读盘）。
fn import_apply_impl(
    pool: &DbPool,
    path: &str,
    strategy: &str,
) -> Result<ImportApplyResult, String> {
    if !matches!(strategy, "skip" | "overwrite" | "duplicate") {
        return Err(format!("不支持的导入策略: {strategy}"));
    }
    ensure_table(pool)?;
    let items = read_import_file(path)?;
    let (valid, invalid) = prepare_import_entries(items);

    let mut conn = pool.get().map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut index = load_existing_index(&tx)?;
    let mut next_id: i64 = tx
        .query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM commands", [], |row| {
            row.get(0)
        })
        .map_err(|e| e.to_string())?;
    let now_ms = chrono_now_ms();

    let mut inserted = 0usize;
    let mut updated = 0usize;
    let mut skipped = 0usize;

    for entry in &valid {
        match find_existing(&index, entry) {
            Some(id) => match strategy {
                "skip" => skipped += 1,
                "overwrite" => {
                    overwrite_imported_entry(&tx, id, entry, now_ms)?;
                    updated += 1;
                }
                _ => {
                    // duplicate：命中也插入，但重新生成 uuid
                    let uuid = Uuid::new_v4().to_string();
                    insert_imported_entry(&tx, entry, &uuid, next_id, now_ms)?;
                    remember_inserted(&mut index, next_id, &uuid, entry);
                    next_id += 1;
                    inserted += 1;
                }
            },
            None => {
                // 未命中插入：条目 uuid 为空（或已被占用——同文件重复 uuid）则生成新 uuid
                let uuid = match entry.uuid.as_deref() {
                    Some(u) if !index.by_uuid.contains_key(u) => u.to_string(),
                    _ => Uuid::new_v4().to_string(),
                };
                insert_imported_entry(&tx, entry, &uuid, next_id, now_ms)?;
                remember_inserted(&mut index, next_id, &uuid, entry);
                next_id += 1;
                inserted += 1;
            }
        }
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(ImportApplyResult {
        inserted,
        updated,
        skipped,
        invalid: invalid.len(),
    })
}

// ── Tauri 命令（文件导入导出） ──

/// 导出为文件：弹保存对话框（rfd，建议文件名 veloterm-commands-YYYYMMDD-HHmm.<ext>，
/// filter 只放对应一种扩展名）。format ∈ "json"（明文 v1 信封，pretty）|"vtcmd"（二进制
/// 容器）。用户取消返回 Ok(None)。
#[tauri::command]
pub async fn command_export_file(
    format: String,
    state: State<'_, DbPool>,
) -> Result<Option<ExportFileResult>, String> {
    let ext = match format.as_str() {
        "json" => "json",
        "vtcmd" => "vtcmd",
        other => return Err(format!("不支持的导出格式: {other}")),
    };
    let filter_name = if ext == "json" {
        "JSON 文件"
    } else {
        "VeloTerm 命令库"
    };
    let suggested = format!(
        "veloterm-commands-{}.{ext}",
        format_utc_compact(chrono_now_ms())
    );
    let picked = rfd::AsyncFileDialog::new()
        .set_file_name(&suggested)
        .add_filter(filter_name, &[ext])
        .save_file()
        .await;
    let Some(handle) = picked else {
        return Ok(None);
    };
    let path = handle.path().to_string_lossy().to_string();
    let pool = state.inner().clone();
    let result = tokio::task::spawn_blocking(move || export_file_impl(&pool, &path, &format))
        .await
        .map_err(|e| format!("导出任务失败: {e}"))??;
    Ok(Some(result))
}

/// 导入第一步：弹打开对话框（*.vtcmd 与 *.json，多选关闭），读文件 + 识别载体 +
/// 解析 + 行级校验 + 与库内比对，dry-run 不落库。用户取消返回 Ok(None)。
#[tauri::command]
pub async fn command_import_start(
    state: State<'_, DbPool>,
) -> Result<Option<ImportStartResult>, String> {
    let picked = rfd::AsyncFileDialog::new()
        .add_filter("VeloTerm 命令库", &["vtcmd", "json"])
        .pick_file()
        .await;
    let Some(handle) = picked else {
        return Ok(None);
    };
    let path = handle.path().to_string_lossy().to_string();
    let pool = state.inner().clone();
    let preview_path = path.clone();
    let preview =
        tokio::task::spawn_blocking(move || import_preview_impl(&pool, &preview_path))
            .await
            .map_err(|e| format!("导入任务失败: {e}"))??;
    Ok(Some(ImportStartResult { path, preview }))
}

/// 导入第二步：按策略落库（重新从 path 读文件解析，两阶段各自读盘）。
/// strategy ∈ "skip" | "overwrite" | "duplicate"，其他值 Err。
#[tauri::command]
pub async fn command_import_apply(
    path: String,
    strategy: String,
    state: State<'_, DbPool>,
) -> Result<ImportApplyResult, String> {
    if !matches!(strategy.as_str(), "skip" | "overwrite" | "duplicate") {
        return Err(format!("不支持的导入策略: {strategy}"));
    }
    let pool = state.inner().clone();
    tokio::task::spawn_blocking(move || import_apply_impl(&pool, &path, &strategy))
        .await
        .map_err(|e| format!("导入任务失败: {e}"))?
}

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// UTC 紧凑时间戳（导出建议文件名用）：YYYYMMDD-HHmm。
fn format_utc_compact(unix_ms: i64) -> String {
    let secs = unix_ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}")
}

/// Howard Hinnant 的 civil_from_days：自 1970-01-01 起的天数 → (年, 月, 日)（UTC）。
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db_at_path;
    use serde_json::json;

    // ── 测试基建 ──

    fn test_pool() -> DbPool {
        let temp_dir = std::env::temp_dir().join(format!("veloterm-cmdlib-{}", Uuid::new_v4()));
        init_db_at_path(&temp_dir.join("veloterm.db"), "cmdlib-test-key")
            .expect("init_db_at_path should succeed")
    }

    fn temp_payload_path(name: &str, bytes: &[u8]) -> String {
        let path =
            std::env::temp_dir().join(format!("veloterm-cmdlib-{name}-{}", Uuid::new_v4()));
        std::fs::write(&path, bytes).expect("write temp payload should succeed");
        path.to_string_lossy().to_string()
    }

    fn blank_record(title: &str, command: &str) -> CommandRecord {
        CommandRecord {
            id: 0,
            uuid: String::new(),
            title: title.to_string(),
            command: command.to_string(),
            description: String::new(),
            purpose: String::new(),
            category: String::new(),
            tags: Vec::new(),
            params: Vec::new(),
            danger_level: DangerLevel::Safe,
            example_output: String::new(),
            usage_count: 0,
            last_used_at: None,
            is_pinned: false,
            created_at: 0,
            updated_at: 0,
        }
    }

    // ── 内置预置命令播种 ──

    #[test]
    fn builtin_seed_payload_is_valid() {
        let entries: Vec<SeedEntry> =
            serde_json::from_str(BUILTIN_SEED_JSON).expect("seed json should parse");
        assert!(
            !entries.is_empty(),
            "内置预置命令集不应为空（seed-parts 未合并？先跑 scripts/build-command-seed.mjs）"
        );
        let mut seen = std::collections::HashSet::new();
        for e in &entries {
            assert!(!e.uuid.trim().is_empty(), "uuid 为空: {}", e.title);
            assert!(seen.insert(e.uuid.as_str()), "uuid 重复: {}", e.uuid);
            assert!(!e.title.trim().is_empty() && !e.command.trim().is_empty());
            // 占位符契约：每个 param 必须出现在 command 模板里
            let placeholders: std::collections::HashSet<String> = e
                .command
                .match_indices("{{")
                .filter_map(|(i, _)| e.command[i + 2..].split("}}").next())
                .map(|s| s.trim().to_string())
                .collect();
            for p in &e.params {
                assert!(
                    placeholders.contains(&p.name),
                    "param `{}` 未出现在 command 中（{}）",
                    p.name, e.title
                );
            }
        }
    }

    #[test]
    fn seed_is_version_gated_and_idempotent() {
        let pool = test_pool();
        let total = serde_json::from_str::<Vec<SeedEntry>>(BUILTIN_SEED_JSON)
            .expect("seed json should parse")
            .len();

        // 空库首次播种：全量插入
        let (first, skipped) =
            seed_builtin_commands(&pool).expect("first seed should succeed");
        assert!(first > 0, "空库首次播种应插入条目");
        assert_eq!(first + skipped, total);

        // 第二次：kv 版本标记短路，零插入
        let (second, _) = seed_builtin_commands(&pool).expect("second seed should succeed");
        assert_eq!(second, 0, "同版本重复播种应被 kv 标记短路");

        // 用户删除一条后再次播种：不复活（版本门控语义）
        {
            let conn = pool.get().expect("conn should succeed");
            conn.execute(
                "DELETE FROM commands WHERE id = (SELECT MIN(id) FROM commands)",
                [],
            )
            .expect("delete should succeed");
        }
        let (third, _) = seed_builtin_commands(&pool).expect("third seed should succeed");
        assert_eq!(third, 0, "被删除的预置条目不应在同版本内复活");

        // 库中剩余数 = 首次插入数 - 1
        let count: i64 = {
            let conn = pool.get().expect("conn should succeed");
            conn.query_row("SELECT COUNT(*) FROM commands", [], |r| r.get(0))
                .expect("count should succeed")
        };
        assert_eq!(count as usize, first - 1);
    }

    fn insert_seed_row(pool: &DbPool, id: i64, uuid: &str, title: &str, command: &str, usage: i64) {
        let conn = pool.get().expect("pool get should succeed");
        conn.execute(
            "INSERT INTO commands (id, uuid, title, command, description, purpose, category, tags,
             params, danger_level, example_output, usage_count, last_used_at, is_pinned,
             created_at, updated_at)
           VALUES (?1,?2,?3,?4,'','','','[]','[]','safe','',?5,NULL,0,1000,1000)",
            rusqlite::params![id, uuid, title, command, usage],
        )
        .expect("seed insert should succeed");
    }

    fn row_count(pool: &DbPool) -> i64 {
        let conn = pool.get().expect("pool get should succeed");
        conn.query_row("SELECT COUNT(*) FROM commands", [], |row| row.get(0))
            .expect("count should succeed")
    }

    fn text_field_of(pool: &DbPool, column: &str, title: &str) -> String {
        let conn = pool.get().expect("pool get should succeed");
        conn.query_row(
            &format!("SELECT {column} FROM commands WHERE title = ?1"),
            [title],
            |row| row.get::<_, String>(0),
        )
        .expect("text field query should succeed")
    }

    fn int_field_of(pool: &DbPool, column: &str, title: &str) -> i64 {
        let conn = pool.get().expect("pool get should succeed");
        conn.query_row(
            &format!("SELECT {column} FROM commands WHERE title = ?1"),
            [title],
            |row| row.get::<_, i64>(0),
        )
        .expect("int field query should succeed")
    }

    // ── 既有模型测试 ──

    #[test]
    fn danger_level_serialization() {
        assert_eq!(
            serde_json::to_string(&DangerLevel::Safe).unwrap(),
            "\"safe\""
        );
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

    // ── VTCD 二进制容器 ──

    #[test]
    fn vtcmd_container_roundtrip() {
        let json = r#"{"format":"veloterm-commands","version":1,"count":0,"commands":[]}"#;
        let encoded = encode_vtcmd(json).expect("encode should succeed");

        // 头部布局：magic + 版本 u16LE + flags u16LE + payload 长度 u32LE
        assert_eq!(encoded.get(0..4), Some(VTCD_MAGIC.as_slice()));
        assert_eq!(le_u16_at(&encoded, 4), Some(1));
        assert_eq!(le_u16_at(&encoded, 6), Some(VTCD_FLAG_DEFLATE));
        let payload_len = le_u32_at(&encoded, 8).expect("payload len should exist") as usize;
        assert_eq!(encoded.len(), VTCD_HEADER_LEN + payload_len + VTCD_DIGEST_LEN);

        let decoded = decode_vtcmd(&encoded).expect("decode should succeed");
        assert_eq!(decoded, json);
    }

    #[test]
    fn vtcmd_tampered_payload_fails_checksum() {
        // 篡改 payload 首字节：SHA-256 校验在解压前判定，必命中「文件校验失败」
        let mut tampered = encode_vtcmd("{\"commands\":[]}").expect("encode should succeed");
        if let Some(byte) = tampered.get_mut(VTCD_HEADER_LEN) {
            *byte ^= 0xFF;
        }
        let err = decode_vtcmd(&tampered).expect_err("tampered payload should fail");
        assert!(err.contains("文件校验失败"), "unexpected error: {err}");

        // 篡改尾部校验和本身同样报校验失败
        let mut bad_digest = encode_vtcmd("{}").expect("encode should succeed");
        let digest_at = bad_digest.len() - VTCD_DIGEST_LEN;
        if let Some(byte) = bad_digest.get_mut(digest_at) {
            *byte ^= 0xFF;
        }
        let err = decode_vtcmd(&bad_digest).expect_err("tampered digest should fail");
        assert!(err.contains("文件校验失败"), "unexpected error: {err}");
    }

    // ── 载体识别与解析 ──

    #[test]
    fn payload_root_forms() {
        // 旧版裸数组（command_export 历史产物）
        let items =
            parse_commands_payload(r#"[{"title":"a","command":"b"}]"#).expect("bare array ok");
        assert_eq!(items.len(), 1);
        // v1 信封（含空数组：合法，全 0 预览）
        let envelope = r#"{"format":"veloterm-commands","version":1,"commands":[]}"#;
        assert!(parse_commands_payload(envelope).expect("envelope ok").is_empty());
        // 信封缺 format 字段合法
        assert!(parse_commands_payload(r#"{"version":1,"commands":[]}"#).is_ok());
        // version > 1 整体拒绝
        assert!(
            parse_commands_payload(r#"{"format":"veloterm-commands","version":2,"commands":[]}"#)
                .is_err()
        );
        // format 不符拒绝
        assert!(parse_commands_payload(r#"{"format":"other","commands":[]}"#).is_err());
        // 缺 commands 数组拒绝
        assert!(parse_commands_payload(r#"{"format":"veloterm-commands"}"#).is_err());
        // 非对象/数组根拒绝；根 JSON 非法拒绝
        assert!(parse_commands_payload("\"hello\"").is_err());
        assert!(parse_commands_payload("not-json").is_err());
    }

    // ── 判重与三策略 ──

    #[test]
    fn uuid_conflict_strategies() {
        let pool = test_pool();
        ensure_table(&pool).expect("ensure_table should succeed");
        let seeded_uuid = "11111111-1111-4111-8111-111111111111";
        insert_seed_row(&pool, 1, seeded_uuid, "旧标题", "systemctl status nginx", 5);

        let payload = json!({
            "format": "veloterm-commands",
            "version": 1,
            "exportedAt": 1,
            "count": 1,
            "commands": [
                {
                    "uuid": seeded_uuid,
                    "title": "新标题",
                    "command": "systemctl status nginx",
                    "dangerLevel": "confirm",
                    "tags": ["systemd"],
                },
            ],
        })
        .to_string();
        let path = temp_payload_path("uuid-conflict.json", payload.as_bytes());

        // skip：命中跳过，库内不动
        let r = import_apply_impl(&pool, &path, "skip").expect("skip apply should succeed");
        assert_eq!(
            r,
            ImportApplyResult { inserted: 0, updated: 0, skipped: 1, invalid: 0 }
        );
        assert_eq!(row_count(&pool), 1);
        assert_eq!(text_field_of(&pool, "title", "旧标题"), "旧标题");

        // overwrite：更新导出字段，保留本地 uuid / usage_count / id
        let r =
            import_apply_impl(&pool, &path, "overwrite").expect("overwrite apply should succeed");
        assert_eq!(
            r,
            ImportApplyResult { inserted: 0, updated: 1, skipped: 0, invalid: 0 }
        );
        assert_eq!(row_count(&pool), 1);
        assert_eq!(text_field_of(&pool, "title", "新标题"), "新标题");
        assert_eq!(text_field_of(&pool, "uuid", "新标题"), seeded_uuid);
        assert_eq!(int_field_of(&pool, "usage_count", "新标题"), 5);

        // duplicate：命中也插入（重新生成 uuid），库内变两行且 uuid 不同
        let r =
            import_apply_impl(&pool, &path, "duplicate").expect("duplicate apply should succeed");
        assert_eq!(
            r,
            ImportApplyResult { inserted: 1, updated: 0, skipped: 0, invalid: 0 }
        );
        assert_eq!(row_count(&pool), 2);
        let dup_uuid = {
            let conn = pool.get().expect("pool get should succeed");
            conn.query_row(
                "SELECT uuid FROM commands WHERE id = 2",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("dup uuid query should succeed")
        };
        assert_ne!(dup_uuid, seeded_uuid);
        assert!(!dup_uuid.is_empty());
    }

    #[test]
    fn legacy_bare_array_import_falls_back_to_title_command() {
        let pool = test_pool();
        ensure_table(&pool).expect("ensure_table should succeed");

        // 库内已有 t1|c1（uuid 由 save 路径生成）
        let mut seeded = blank_record("t1", "c1");
        save_command_impl(&mut seeded, &pool).expect("seed save should succeed");
        assert!(!seeded.uuid.is_empty(), "save path should assign uuid");

        // 旧版 command_export 产物：CommandRecord 裸数组（含本地字段，无 uuid）
        let payload = json!([
            {
                "id": 1,
                "title": "t1",
                "command": "c1",
                "description": "",
                "purpose": "",
                "category": "",
                "tags": [],
                "params": [],
                "dangerLevel": "safe",
                "exampleOutput": "",
                "usageCount": 3,
                "isPinned": false,
                "createdAt": 123,
                "updatedAt": 123,
            },
            { "title": "t2", "command": "c2" },
        ])
        .to_string();
        let path = temp_payload_path("bare-array.json", payload.as_bytes());

        // 预览：t1 走 title|command 兜底命中，t2 为新条目
        let preview = import_preview_impl(&pool, &path).expect("preview should succeed");
        assert_eq!(preview.new_count, 1);
        assert_eq!(preview.conflicts.len(), 1);
        assert!(preview.conflicts.first().unwrap().uuid.is_none());

        let r = import_apply_impl(&pool, &path, "skip").expect("apply should succeed");
        assert_eq!(
            r,
            ImportApplyResult { inserted: 1, updated: 0, skipped: 1, invalid: 0 }
        );

        // 无 uuid 条目插入时生成新 uuid
        assert!(!text_field_of(&pool, "uuid", "t2").is_empty());
        // 旧条目里的 usageCount 不随导入写入（usage_count=0）
        assert_eq!(int_field_of(&pool, "usage_count", "t2"), 0);
    }

    #[test]
    fn invalid_rows_do_not_block_valid_ones() {
        let pool = test_pool();
        ensure_table(&pool).expect("ensure_table should succeed");
        let payload = json!({
            "format": "veloterm-commands",
            "version": 1,
            "commands": [
                { "title": "ok", "command": "echo ok", "dangerLevel": "safe" },
                { "command": "missing-title" },
                { "title": "bad-dl", "command": "x", "dangerLevel": "yolo" },
                { "title": "bad-tags", "command": "x", "tags": "not-array" },
            ],
        })
        .to_string();
        let path = temp_payload_path("mixed-invalid.json", payload.as_bytes());

        let preview = import_preview_impl(&pool, &path).expect("preview should succeed");
        assert_eq!(preview.new_count, 1);
        assert!(preview.conflicts.is_empty());
        assert_eq!(preview.invalid.len(), 3);
        let first = preview.invalid.first().unwrap();
        assert_eq!(first.index, 1);
        assert!(first.reason.contains("title"), "reason: {}", first.reason);

        let r = import_apply_impl(&pool, &path, "skip").expect("apply should succeed");
        assert_eq!(
            r,
            ImportApplyResult { inserted: 1, updated: 0, skipped: 0, invalid: 3 }
        );
        assert_eq!(row_count(&pool), 1);
        assert_eq!(text_field_of(&pool, "title", "ok"), "ok");
    }

    // ── 存量迁移 ──

    #[test]
    fn legacy_table_backfills_unique_uuids() {
        let pool = test_pool();
        {
            let conn = pool.get().expect("pool get should succeed");
            // 旧版表结构（无 uuid 列）
            conn.execute_batch(
                "CREATE TABLE commands (
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
            .expect("legacy create should succeed");
            for (id, title, command) in [(1i64, "a", "aa"), (2, "b", "bb")] {
                conn.execute(
                    "INSERT INTO commands (id, title, command, created_at, updated_at)
                   VALUES (?1,?2,?3,1,1)",
                    rusqlite::params![id, title, command],
                )
                .expect("legacy insert should succeed");
            }
        }

        ensure_table(&pool).expect("migration should succeed");

        let conn = pool.get().expect("pool get should succeed");
        let mut uuids: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT uuid FROM commands ORDER BY id")
                .expect("prepare should succeed");
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .expect("query should succeed");
            rows.collect::<Result<Vec<_>, _>>()
                .expect("rows should succeed")
        };
        assert_eq!(uuids.len(), 2);
        for uuid in &uuids {
            assert!(!uuid.is_empty());
            // v4 形态 8-4-4-4-12
            assert_eq!(uuid.len(), 36);
            for pos in [8, 13, 18, 23] {
                assert_eq!(uuid.get(pos..pos + 1), Some("-"), "uuid shape: {uuid}");
            }
        }
        uuids.sort();
        uuids.dedup();
        assert_eq!(uuids.len(), 2, "backfilled uuids must be unique");

        // 唯一索引已建立：重复 uuid 插入被拒
        let dup_rejected = conn
            .execute(
                "INSERT INTO commands (id, uuid, title, command, created_at, updated_at)
               VALUES (3,?1,'c','cc',1,1)",
                rusqlite::params![uuids.first().cloned()],
            )
            .is_err();
        assert!(dup_rejected, "duplicate uuid insert must be rejected");
    }

    // ── 导出 → 导入闭环 ──

    #[test]
    fn vtcmd_export_import_cycle() {
        let pool = test_pool();
        ensure_table(&pool).expect("ensure_table should succeed");
        let mut record = blank_record("cycle", "echo cycle");
        save_command_impl(&mut record, &pool).expect("save should succeed");

        // 从库导出 → VTCD 容器落盘 → 预览应显示 uuid 命中冲突
        let records = list_all_records(&pool).expect("list should succeed");
        let (_, compact) = build_export_payload(&records).expect("build payload should succeed");
        let bytes = encode_vtcmd(&compact).expect("encode should succeed");
        let path = temp_payload_path("cycle.vtcmd", &bytes);

        let preview = import_preview_impl(&pool, &path).expect("preview should succeed");
        assert_eq!(preview.new_count, 0);
        assert_eq!(preview.conflicts.len(), 1);
        let conflict = preview.conflicts.first().unwrap();
        assert_eq!(conflict.uuid.as_deref(), Some(record.uuid.as_str()));
        assert_eq!(conflict.title, "cycle");
        assert!(preview.invalid.is_empty());

        let r = import_apply_impl(&pool, &path, "skip").expect("apply should succeed");
        assert_eq!(
            r,
            ImportApplyResult { inserted: 0, updated: 0, skipped: 1, invalid: 0 }
        );

        // 空数组文件：全 0 预览，合法
        let empty_path = temp_payload_path("empty.json", b"[]");
        let empty = import_preview_impl(&pool, &empty_path).expect("empty preview should succeed");
        assert_eq!(empty.new_count, 0);
        assert!(empty.conflicts.is_empty());
        assert!(empty.invalid.is_empty());
    }

    #[test]
    fn utc_compact_formatting() {
        assert_eq!(format_utc_compact(0), "19700101-0000");
        // 365 天后（1970 非闰年）
        assert_eq!(format_utc_compact(86_400_000 * 365), "19710101-0000");
        // 2025-10-09 08:53:20 UTC = 1760000000s
        assert_eq!(format_utc_compact(1_760_000_000_000), "20251009-0853");
    }
}
