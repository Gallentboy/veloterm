use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use r2d2::{CustomizeConnection, Pool};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OptionalExtension;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::terminal::ssh::{RelayTransport, SSHTargetAuth, SshHostKeyVerificationMode, SshRelayEndpoint};

pub type DbPool = Pool<SqliteConnectionManager>;

pub const DEFAULT_DB_FILE_NAME: &str = "veloterm.db";
pub const DEFAULT_MASTER_KEY: &str = "veloterm-sqlcipher-master-key-2026";

static GLOBAL_DB_POOL: OnceLock<DbPool> = OnceLock::new();

pub fn global_pool() -> Option<DbPool> {
    GLOBAL_DB_POOL.get().cloned()
}

pub fn now_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// 会话持久化数据模型（对应 `sessions` 表）。
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct SessionModel {
    #[serde(default)]
    pub id: usize,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: String,
    pub group_name: Option<String>,
}

/// 分组树节点持久化模型（对应 `groups` 表，与前端 `GroupNode` 对齐）。
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GroupRecord {
    pub id: usize,
    pub parent_id: Option<usize>,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 主机完整持久化模型（对应 `hosts` + `host_groups` 表，含加密存储的 `auth` 凭据，与前端 `HostNode` 对齐）。
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostRecord {
    pub id: usize,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SSHTargetAuth,
    #[serde(default)]
    pub jump_host_id: Option<usize>,
    #[serde(default)]
    pub host_key_verification: SshHostKeyVerificationMode,
    #[serde(default)]
    pub allow_insecure_algos: bool,
    /// 终端就绪钩子：会话建立后自动在终端执行的命令（引用命令库 commands.id；
    /// NULL = 未启用。仅存主键 id，命令内容以命令库为准，命令被删后钩子静默失效）
    #[serde(default)]
    pub ready_command_id: Option<usize>,
    /// SSH 中继（veloterm_relay）：有值时该主机经中继连接（首跳传输替换）。
    /// secret 存 SQLCipher 加密库；relay_host 为空视为未启用
    #[serde(default)]
    pub relay: Option<SshRelayEndpoint>,
    #[serde(default)]
    pub group_ids: Vec<usize>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 主机与分组完整数据集（与前端 `HostStoreData` v2 对齐）。
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostStorePayload {
    pub version: u8,
    pub groups: Vec<GroupRecord>,
    pub hosts: Vec<HostRecord>,
}

impl Default for HostStorePayload {
    fn default() -> Self {
        Self {
            version: 2,
            groups: Vec::new(),
            hosts: Vec::new(),
        }
    }
}

/// 前端启动时一次性拉取的全量持久化状态（包含 `app_kv` 键值配置表与 `HostStorePayload`）。
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapStatePayload {
    pub kv: HashMap<String, String>,
    pub host_store: HostStorePayload,
}

/// 自定义连接池初始化器：确保每次从连接池获取连接时自动注入 `PRAGMA key` 解密密钥。
#[derive(Debug, Clone)]
struct SqlCipherConnectionCustomizer {
    master_key: String,
}

impl CustomizeConnection<rusqlite::Connection, rusqlite::Error> for SqlCipherConnectionCustomizer {
    fn on_acquire(&self, conn: &mut rusqlite::Connection) -> Result<(), rusqlite::Error> {
        conn.pragma_update(None, "key", &self.master_key)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    }
}

fn ensure_integer_primary_key_table(
    conn: &rusqlite::Connection,
    table_name: &str,
) -> Result<(), rusqlite::Error> {
    let id_type: Option<String> = conn
        .query_row(
            &format!("SELECT type FROM pragma_table_info('{table_name}') WHERE name = 'id'"),
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(t) = id_type
        && !t.eq_ignore_ascii_case("INTEGER")
    {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS {table_name};"))?;
    }
    Ok(())
}

/// 在指定路径初始化 SQLCipher 加密数据库连接池并完成全部表结构迁移。
pub fn init_db_at_path(db_path: &Path, master_key: &str) -> Result<DbPool, String> {
    if master_key.trim().is_empty() {
        return Err("SQLCipher master_key must not be empty".to_string());
    }

    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create app_data_dir {}: {e}", parent.display()))?;
    }

    let key_for_init = master_key.to_owned();
    let manager = SqliteConnectionManager::file(db_path).with_init(move |conn| {
        conn.pragma_update(None, "key", &key_for_init)?;
        Ok(())
    });

    let pool = Pool::builder()
        .max_size(8)
        .connection_customizer(Box::new(SqlCipherConnectionCustomizer {
            master_key: master_key.to_owned(),
        }))
        .build(manager)
        .map_err(|e| format!("failed to build SQLCipher connection pool: {e}"))?;

    let conn = pool
        .get()
        .map_err(|e| format!("failed to acquire connection from SQLCipher pool: {e}"))?;

    ensure_integer_primary_key_table(&conn, "sessions")
        .map_err(|e| format!("failed to check sessions schema: {e}"))?;
    ensure_integer_primary_key_table(&conn, "known_hosts")
        .map_err(|e| format!("failed to check known_hosts schema: {e}"))?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          name TEXT NOT NULL,
          host TEXT NOT NULL,
          port INTEGER NOT NULL DEFAULT 22,
          username TEXT NOT NULL,
          auth_type TEXT NOT NULL,
          group_name TEXT,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
      );

      CREATE TABLE IF NOT EXISTS groups (
          id INTEGER PRIMARY KEY,
          parent_id INTEGER,
          name TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
      );

      CREATE TABLE IF NOT EXISTS hosts (
          id INTEGER PRIMARY KEY,
          name TEXT NOT NULL,
          host TEXT NOT NULL,
          port INTEGER NOT NULL DEFAULT 22,
          username TEXT NOT NULL,
          auth_json TEXT NOT NULL,
          jump_host_id INTEGER,
          host_key_verification TEXT NOT NULL DEFAULT 'auto_accept',
          allow_insecure_algos INTEGER NOT NULL DEFAULT 0,
          ready_command_id INTEGER,
          relay_host TEXT,
          relay_port INTEGER,
          relay_transport TEXT,
          relay_key_id INTEGER,
          relay_secret TEXT,
          relay_target TEXT,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
      );

      CREATE TABLE IF NOT EXISTS host_groups (
          host_id INTEGER NOT NULL,
          group_id INTEGER NOT NULL,
          PRIMARY KEY (host_id, group_id),
          FOREIGN KEY (host_id) REFERENCES hosts(id) ON DELETE CASCADE,
          FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE CASCADE
      );

      CREATE TABLE IF NOT EXISTS known_hosts (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          host TEXT NOT NULL,
          port INTEGER NOT NULL,
          key_type TEXT NOT NULL,
          key_base64 TEXT NOT NULL,
          created_at INTEGER NOT NULL
      );

      CREATE TABLE IF NOT EXISTS app_kv (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL,
          updated_at INTEGER NOT NULL
      );

      CREATE TABLE IF NOT EXISTS editor_drafts (
          file_id INTEGER PRIMARY KEY,
          host_id INTEGER NOT NULL,
          remote_path TEXT NOT NULL,
          base_hash TEXT NOT NULL,
          content TEXT NOT NULL,
          updated_at INTEGER NOT NULL
      );

      CREATE TABLE IF NOT EXISTS port_forwards (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          host_id INTEGER NOT NULL,
          kind TEXT NOT NULL,
          bind_host TEXT NOT NULL,
          bind_port INTEGER NOT NULL,
          target_host TEXT NOT NULL,
          target_port INTEGER NOT NULL,
          enabled INTEGER NOT NULL DEFAULT 0,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
      );",
    )
    .map_err(|e| format!("failed to initialize schema in SQLCipher database: {e}"))?;

    // 列级迁移（在建表之后）：老库升级补列。SQLite 的 ADD COLUMN 无 IF NOT EXISTS，
    // 列已存在时报 duplicate column，吞掉该错误即幂等；新库建表已含该列，同样走这条路。
    // 后续新增列时照抄一段同构语句即可
    if let Err(e) = conn.execute_batch("ALTER TABLE hosts ADD COLUMN ready_command_id INTEGER")
        && !e.to_string().contains("duplicate column")
    {
        return Err(format!("failed to apply column migration `ready_command_id`: {e}"));
    }

    // SSH 中继六列（relay_host 非空 = 启用；其余列仅在 relay_host 非空时有意义）
    for col_def in [
        "relay_host TEXT",
        "relay_port INTEGER",
        "relay_transport TEXT",
        "relay_key_id INTEGER",
        "relay_secret TEXT",
        "relay_target TEXT",
    ] {
        let sql = format!("ALTER TABLE hosts ADD COLUMN {col_def}");
        if let Err(e) = conn.execute_batch(&sql)
            && !e.to_string().contains("duplicate column")
        {
            return Err(format!("failed to apply column migration `{col_def}`: {e}"));
        }
    }

    log::info!(
        "[DB] initialized SQLCipher encrypted database at {}",
        db_path.display()
    );

    Ok(pool)
}

/// 初始化应用默认 SQLCipher 数据库：
/// 路径位于操作系统标准应用数据目录 `app_handle.path().app_data_dir()?.join("veloterm.db")`。
pub fn init_db(app_handle: &AppHandle, master_key: &str) -> Result<DbPool, String> {
    let app_data_dir: PathBuf = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app_data_dir: {e}"))?;

    fs::create_dir_all(&app_data_dir).map_err(|e| {
        format!(
            "failed to recursively create app_data_dir {}: {e}",
            app_data_dir.display()
        )
    })?;

    let db_path = app_data_dir.join(DEFAULT_DB_FILE_NAME);
    let pool = init_db_at_path(&db_path, master_key)?;
    let _ = GLOBAL_DB_POOL.set(pool.clone());
    Ok(pool)
}

// ===== `app_kv` 通用键值持久化操作 =====

pub fn get_all_kv(pool: &DbPool) -> Result<HashMap<String, String>, String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    let mut stmt = conn
        .prepare("SELECT key, value FROM app_kv")
        .map_err(|e| format!("failed to prepare get_all_kv: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("failed to query app_kv: {e}"))?;

    let mut map = HashMap::new();
    for row in rows {
        let (k, v) = row.map_err(|e| format!("failed to read app_kv row: {e}"))?;
        map.insert(k, v);
    }
    Ok(map)
}

pub fn get_kv(pool: &DbPool, key: &str) -> Result<Option<String>, String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    conn.query_row(
        "SELECT value FROM app_kv WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| format!("failed to query app_kv key={key}: {e}"))
}

pub fn set_kv(pool: &DbPool, key: &str, value: &str) -> Result<(), String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    let now = now_timestamp_ms();
    conn.execute(
        "INSERT INTO app_kv (key, value, updated_at) VALUES (?1, ?2, ?3)
       ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, now],
    )
    .map_err(|e| format!("failed to set app_kv key={key}: {e}"))?;
    Ok(())
}

pub fn delete_kv(pool: &DbPool, key: &str) -> Result<(), String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    conn.execute("DELETE FROM app_kv WHERE key = ?1", params![key])
        .map_err(|e| format!("failed to delete app_kv key={key}: {e}"))?;
    Ok(())
}

// ===== `groups` / `hosts` / `host_groups` / `sessions` 持久化操作 =====

/// 远程文件编辑器草稿行（对应 `editor_drafts` 表，与前端 `DraftRow` 对齐）。
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EditorDraftRecord {
    pub file_id: i64,
    pub host_id: i64,
    pub remote_path: String,
    pub base_hash: String,
    pub content: String,
    pub updated_at: i64,
}

/// 加载全部编辑器草稿（按 `file_id` 升序）。
pub fn load_editor_drafts(pool: &DbPool) -> Result<Vec<EditorDraftRecord>, String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    let mut stmt = conn
        .prepare(
            "SELECT file_id, host_id, remote_path, base_hash, content, updated_at
       FROM editor_drafts
       ORDER BY file_id ASC",
        )
        .map_err(|e| format!("failed to prepare editor_drafts query: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(EditorDraftRecord {
                file_id: row.get(0)?,
                host_id: row.get(1)?,
                remote_path: row.get(2)?,
                base_hash: row.get(3)?,
                content: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })
        .map_err(|e| format!("failed to query editor_drafts: {e}"))?;

    let mut drafts = Vec::new();
    for row in rows {
        drafts.push(row.map_err(|e| format!("failed to decode editor_drafts row: {e}"))?);
    }
    Ok(drafts)
}

/// 新增或更新（upsert by `file_id`）一条编辑器草稿。
pub fn save_editor_draft(pool: &DbPool, draft: &EditorDraftRecord) -> Result<(), String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    let now = now_timestamp_ms();
    conn.execute(
        "INSERT INTO editor_drafts (file_id, host_id, remote_path, base_hash, content, updated_at)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6)
       ON CONFLICT(file_id) DO UPDATE SET
         host_id = excluded.host_id,
         remote_path = excluded.remote_path,
         base_hash = excluded.base_hash,
         content = excluded.content,
         updated_at = excluded.updated_at",
        params![
            draft.file_id,
            draft.host_id,
            &draft.remote_path,
            &draft.base_hash,
            &draft.content,
            now
        ],
    )
    .map_err(|e| format!("failed to save editor draft file_id={}: {e}", draft.file_id))?;
    Ok(())
}

/// 按 `file_id` 删除一条编辑器草稿（不存在时静默成功）。
pub fn delete_editor_draft(pool: &DbPool, file_id: i64) -> Result<(), String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    conn.execute(
        "DELETE FROM editor_drafts WHERE file_id = ?1",
        params![file_id],
    )
    .map_err(|e| format!("failed to delete editor draft file_id={file_id}: {e}"))?;
    Ok(())
}

// ===== `groups` / `hosts` / `host_groups` / `sessions` 持久化操作 =====

fn auth_method_str(auth: &SSHTargetAuth) -> &'static str {
    match auth {
        SSHTargetAuth::Password { .. } => "password",
        SSHTargetAuth::Key { .. } => "key",
        SSHTargetAuth::Agent => "agent",
    }
}

fn verification_mode_to_str(mode: SshHostKeyVerificationMode) -> &'static str {
    match mode {
        SshHostKeyVerificationMode::AutoAccept | SshHostKeyVerificationMode::Prompt => {
            "auto_accept"
        }
        SshHostKeyVerificationMode::AutoReject => "auto_reject",
        SshHostKeyVerificationMode::Ignore => "ignore",
    }
}

fn verification_mode_from_str(raw: &str) -> SshHostKeyVerificationMode {
    match raw.trim() {
        "auto_reject" => SshHostKeyVerificationMode::AutoReject,
        "ignore" => SshHostKeyVerificationMode::Ignore,
        _ => SshHostKeyVerificationMode::AutoAccept,
    }
}

fn relay_transport_to_str(t: RelayTransport) -> &'static str {
    match t {
        RelayTransport::Tcp => "tcp",
        RelayTransport::Ws => "ws",
        RelayTransport::Quic => "quic",
    }
}

fn relay_transport_from_str(raw: &str) -> Option<RelayTransport> {
    match raw.trim() {
        "tcp" => Some(RelayTransport::Tcp),
        "ws" => Some(RelayTransport::Ws),
        "quic" => Some(RelayTransport::Quic),
        _ => None,
    }
}

/// 行列 → 中继配置。relay_host 为空/缺失返回 None；部分字段缺失（旧库半态）按
/// 默认值补齐（transport 默认 tcp、port 默认 7300、key_id 默认 1）。
/// relay_target 列已废弃（目标改由客户端自动携带），不再读取。
fn row_to_relay(
    relay_host: Option<&str>,
    relay_port: Option<i64>,
    relay_transport: Option<&str>,
    relay_key_id: Option<i64>,
    relay_secret: Option<&str>,
) -> Option<SshRelayEndpoint> {
    let host = relay_host?.trim().to_string();
    if host.is_empty() {
        return None;
    }
    let secret = relay_secret?.trim().to_string();
    if secret.is_empty() {
        return None;
    }
    Some(SshRelayEndpoint {
        host,
        port: relay_port
            .and_then(|p| u16::try_from(p).ok())
            .unwrap_or(7300),
        transport: relay_transport
            .and_then(relay_transport_from_str)
            .unwrap_or(RelayTransport::Tcp),
        key_id: relay_key_id.map(|k| k.clamp(0, 255) as u8).unwrap_or(1),
        secret,
    })
}

pub fn load_host_store(pool: &DbPool) -> Result<HostStorePayload, String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;

    // 1. 读取全部 groups
    let mut group_stmt = conn
        .prepare(
            "SELECT id, parent_id, name, created_at, updated_at
       FROM groups
       ORDER BY id ASC",
        )
        .map_err(|e| format!("failed to prepare groups query: {e}"))?;
    let group_rows = group_stmt
        .query_map([], |row| {
            Ok(GroupRecord {
                id: row.get(0)?,
                parent_id: row.get(1)?,
                name: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })
        .map_err(|e| format!("failed to query groups: {e}"))?;
    let mut groups = Vec::new();
    for g in group_rows {
        groups.push(g.map_err(|e| format!("failed to decode group row: {e}"))?);
    }

    // 2. 读取全部 host_groups 关联
    let mut hg_stmt = conn
        .prepare("SELECT host_id, group_id FROM host_groups ORDER BY host_id ASC, group_id ASC")
        .map_err(|e| format!("failed to prepare host_groups query: {e}"))?;
    let hg_rows = hg_stmt
        .query_map([], |row| {
            Ok((row.get::<_, usize>(0)?, row.get::<_, usize>(1)?))
        })
        .map_err(|e| format!("failed to query host_groups: {e}"))?;
    let mut groups_by_host: HashMap<usize, Vec<usize>> = HashMap::new();
    for hg in hg_rows {
        let (host_id, group_id) =
            hg.map_err(|e| format!("failed to decode host_groups row: {e}"))?;
        groups_by_host.entry(host_id).or_default().push(group_id);
    }

    // 3. 读取全部 hosts
    let mut host_stmt = conn
        .prepare(
            "SELECT id, name, host, port, username, auth_json, jump_host_id,
              host_key_verification, allow_insecure_algos, ready_command_id,
              relay_host, relay_port, relay_transport, relay_key_id, relay_secret,
              created_at, updated_at
       FROM hosts
       ORDER BY id ASC",
        )
        .map_err(|e| format!("failed to prepare hosts query: {e}"))?;
    let host_rows = host_stmt
        .query_map([], |row| {
            let id: usize = row.get(0)?;
            let port_i64: i64 = row.get(3)?;
            let auth_json: String = row.get(5)?;
            let verification_raw: String = row.get(7)?;
            let allow_insecure_int: i64 = row.get(8)?;
            let auth: SSHTargetAuth = serde_json::from_str(&auth_json).unwrap_or_default();
            let relay = row_to_relay(
                row.get::<_, Option<String>>(10)?.as_deref(),
                row.get::<_, Option<i64>>(11)?,
                row.get::<_, Option<String>>(12)?.as_deref(),
                row.get::<_, Option<i64>>(13)?,
                row.get::<_, Option<String>>(14)?.as_deref(),
            );
            Ok(HostRecord {
                id,
                name: row.get(1)?,
                host: row.get(2)?,
                port: u16::try_from(port_i64.clamp(1, 65535)).unwrap_or(22),
                username: row.get(4)?,
                auth,
                jump_host_id: row.get(6)?,
                host_key_verification: verification_mode_from_str(&verification_raw),
                allow_insecure_algos: allow_insecure_int != 0,
                ready_command_id: row.get(9)?,
                relay,
                group_ids: groups_by_host.remove(&id).unwrap_or_default(),
                created_at: row.get(15)?,
                updated_at: row.get(16)?,
            })
        })
        .map_err(|e| format!("failed to query hosts: {e}"))?;

    let mut hosts = Vec::new();
    for h in host_rows {
        hosts.push(h.map_err(|e| format!("failed to decode host row: {e}"))?);
    }

    // 4. 若 `hosts` 表为空但 `sessions` 表中有通过 `save_session` 写入的记录，自动水合为 HostRecord
    if hosts.is_empty() {
        let mut sess_stmt = conn
      .prepare(
        "SELECT id, name, host, port, username, auth_type, group_name, created_at, updated_at
         FROM sessions
         ORDER BY created_at ASC, id ASC",
      )
      .map_err(|e| format!("failed to prepare sessions fallback query: {e}"))?;
        let sess_rows = sess_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, usize>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            })
            .map_err(|e| format!("failed to query sessions fallback: {e}"))?;

        let mut next_host_id: usize = 1;
        let mut next_group_id: usize = groups.iter().map(|g| g.id).max().unwrap_or(0) + 1;

        for item in sess_rows {
            let (
                sess_id,
                name,
                host,
                port_i64,
                username,
                auth_type,
                group_name,
                created_at,
                updated_at,
            ) = item.map_err(|e| format!("failed to decode session row: {e}"))?;
            let host_id = if sess_id > 0 && !hosts.iter().any(|h: &HostRecord| h.id == sess_id) {
                sess_id
            } else {
                while hosts.iter().any(|h: &HostRecord| h.id == next_host_id) {
                    next_host_id += 1;
                }
                next_host_id
            };
            next_host_id = next_host_id.max(host_id + 1);

            let mut group_ids = Vec::new();
            if let Some(g_name) = group_name.filter(|s| !s.trim().is_empty()) {
                if let Some(existing) = groups.iter().find(|g| g.name == g_name) {
                    group_ids.push(existing.id);
                } else {
                    let gid = next_group_id;
                    next_group_id += 1;
                    groups.push(GroupRecord {
                        id: gid,
                        parent_id: None,
                        name: g_name,
                        created_at,
                        updated_at,
                    });
                    group_ids.push(gid);
                }
            }

            let auth = match auth_type.as_str() {
                "key" => SSHTargetAuth::Key {
                    private_key: String::new(),
                    passphrase: None,
                },
                "agent" => SSHTargetAuth::Agent,
                _ => SSHTargetAuth::Password {
                    password: String::new(),
                },
            };

            hosts.push(HostRecord {
                id: host_id,
                name,
                host,
                port: u16::try_from(port_i64.clamp(1, 65535)).unwrap_or(22),
                username,
                auth,
                jump_host_id: None,
                host_key_verification: SshHostKeyVerificationMode::AutoAccept,
                allow_insecure_algos: false,
                ready_command_id: None,
                relay: None,
                group_ids,
                created_at,
                updated_at,
            });
        }
    }

    Ok(HostStorePayload {
        version: 2,
        groups,
        hosts,
    })
}

/// 在单个 SQLCipher 事务内原子保存 `groups`、`hosts`、`host_groups` 并同步镜像至 `sessions` 表。
pub fn save_host_store(pool: &DbPool, store: &HostStorePayload) -> Result<(), String> {
    let mut conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    let tx = conn
        .transaction()
        .map_err(|e| format!("failed to begin transaction: {e}"))?;

    tx.execute("DELETE FROM host_groups", [])
        .map_err(|e| format!("failed to clear host_groups: {e}"))?;
    tx.execute("DELETE FROM hosts", [])
        .map_err(|e| format!("failed to clear hosts: {e}"))?;
    tx.execute("DELETE FROM groups", [])
        .map_err(|e| format!("failed to clear groups: {e}"))?;
    tx.execute("DELETE FROM sessions", [])
        .map_err(|e| format!("failed to clear sessions: {e}"))?;

    let group_name_by_id: HashMap<usize, String> = store
        .groups
        .iter()
        .map(|g| (g.id, g.name.clone()))
        .collect();

    for group in &store.groups {
        tx.execute(
            "INSERT INTO groups (id, parent_id, name, created_at, updated_at)
       VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                group.id,
                group.parent_id,
                &group.name,
                group.created_at,
                group.updated_at
            ],
        )
        .map_err(|e| format!("failed to insert group {}: {e}", group.id))?;
    }

    for host in &store.hosts {
        let auth_json = serde_json::to_string(&host.auth)
            .map_err(|e| format!("failed to serialize host auth: {e}"))?;
        let verification_str = verification_mode_to_str(host.host_key_verification);
        let allow_insecure_int = if host.allow_insecure_algos {
            1i64
        } else {
            0i64
        };

        tx.execute(
            "INSERT INTO hosts (
          id, name, host, port, username, auth_json, jump_host_id,
          host_key_verification, allow_insecure_algos, ready_command_id,
          relay_host, relay_port, relay_transport, relay_key_id, relay_secret,
          created_at, updated_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                host.id,
                &host.name,
                &host.host,
                i64::from(host.port),
                &host.username,
                &auth_json,
                host.jump_host_id,
                verification_str,
                allow_insecure_int,
                host.ready_command_id,
                host.relay.as_ref().map(|r| r.host.clone()),
                host.relay.as_ref().map(|r| i64::from(r.port)),
                host.relay.as_ref().map(|r| relay_transport_to_str(r.transport)),
                host.relay.as_ref().map(|r| i64::from(r.key_id)),
                host.relay.as_ref().map(|r| r.secret.clone()),
                host.created_at,
                host.updated_at,
            ],
        )
        .map_err(|e| format!("failed to insert host {}: {e}", host.id))?;

        for &gid in &host.group_ids {
            if group_name_by_id.contains_key(&gid) {
                tx.execute(
                    "INSERT OR IGNORE INTO host_groups (host_id, group_id) VALUES (?1, ?2)",
                    params![host.id, gid],
                )
                .map_err(|e| format!("failed to insert host_group ({}, {}): {e}", host.id, gid))?;
            }
        }

        let primary_group_name = host
            .group_ids
            .first()
            .and_then(|gid| group_name_by_id.get(gid))
            .cloned();

        tx.execute(
      "INSERT INTO sessions (id, name, host, port, username, auth_type, group_name, created_at, updated_at)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
      params![
        host.id,
        &host.name,
        &host.host,
        i64::from(host.port),
        &host.username,
        auth_method_str(&host.auth),
        primary_group_name,
        host.created_at,
        host.updated_at,
      ],
    )
    .map_err(|e| format!("failed to mirror host {} into sessions: {e}", host.id))?;
    }

    tx.commit()
        .map_err(|e| format!("failed to commit save_host_store transaction: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn sqlcipher_encrypts_database_on_disk() {
        let temp_dir =
            std::env::temp_dir().join(format!("veloterm-sqlcipher-test-{}", Uuid::new_v4()));
        let db_path = temp_dir.join("veloterm.db");
        let master_key = "test-super-secret-master-key";

        let pool = init_db_at_path(&db_path, master_key).expect("init_db_at_path should succeed");
        {
            let conn = pool.get().expect("pool get should succeed");
            conn
        .execute(
          "INSERT INTO sessions (id, name, host, port, username, auth_type, group_name, created_at, updated_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
          rusqlite::params![
            1usize,
            "prod-web",
            "10.0.0.12",
            22u16,
            "ubuntu",
            "key",
            Some("Production"),
            1_700_000_000i64,
            1_700_000_000i64
          ],
        )
        .expect("insert into encrypted db should succeed");
        }
        drop(pool);

        // 验证普通未提供密钥的 SQLite 连接打开该文件时会报错（非明文 SQLite Header）
        let raw_bytes = fs::read(&db_path).expect("db file should exist");
        assert!(
            !raw_bytes.starts_with(b"SQLite format 3\0"),
            "encrypted database file must not start with plaintext SQLite header"
        );

        let unencrypted_conn =
            rusqlite::Connection::open(&db_path).expect("opening file handle succeeds");
        let unencrypted_read_result: Result<i64, _> =
            unencrypted_conn.query_row("SELECT count(*) FROM sessions", [], |row| row.get(0));
        assert!(
            unencrypted_read_result.is_err(),
            "reading without PRAGMA key must fail on SQLCipher database"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn host_store_and_kv_roundtrip_in_sqlcipher() {
        let temp_dir = std::env::temp_dir().join(format!("veloterm-store-test-{}", Uuid::new_v4()));
        let db_path = temp_dir.join("veloterm.db");
        let pool = init_db_at_path(&db_path, "store-key").expect("init_db_at_path should succeed");

        set_kv(&pool, "veloterm-theme", "dark").expect("set_kv should succeed");
        assert_eq!(
            get_kv(&pool, "veloterm-theme").expect("get_kv should succeed"),
            Some("dark".to_string())
        );

        let payload = HostStorePayload {
            version: 2,
            groups: vec![GroupRecord {
                id: 1,
                parent_id: None,
                name: "Production".to_string(),
                created_at: 100,
                updated_at: 100,
            }],
            hosts: vec![HostRecord {
                id: 1,
                name: "web-1".to_string(),
                host: "10.0.1.5".to_string(),
                port: 22,
                username: "root".to_string(),
                auth: SSHTargetAuth::Password {
                    password: "secret-password".to_string(),
                },
                jump_host_id: None,
                host_key_verification: SshHostKeyVerificationMode::AutoAccept,
                allow_insecure_algos: true,
                ready_command_id: Some(7),
                relay: Some(SshRelayEndpoint {
                    host: "relay.example.com".to_string(),
                    port: 7300,
                    transport: RelayTransport::Quic,
                    key_id: 3,
                    secret: "ab".repeat(32),
                }),
                group_ids: vec![1],
                created_at: 200,
                updated_at: 200,
            },
            HostRecord {
                id: 2,
                name: "web-2".to_string(),
                host: "10.0.1.6".to_string(),
                port: 22,
                username: "root".to_string(),
                auth: SSHTargetAuth::Agent,
                jump_host_id: Some(1),
                host_key_verification: SshHostKeyVerificationMode::AutoAccept,
                allow_insecure_algos: false,
                ready_command_id: None,
                relay: None,
                group_ids: vec![],
                created_at: 201,
                updated_at: 201,
            }],
        };

        save_host_store(&pool, &payload).expect("save_host_store should succeed");
        let loaded = load_host_store(&pool).expect("load_host_store should succeed");
        assert_eq!(loaded, payload);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn editor_draft_roundtrip() {
        let temp_dir = std::env::temp_dir().join(format!("veloterm-draft-test-{}", Uuid::new_v4()));
        let db_path = temp_dir.join("veloterm.db");
        let pool = init_db_at_path(&db_path, "draft-key").expect("init_db_at_path should succeed");

        assert!(
            load_editor_drafts(&pool)
                .expect("load should succeed")
                .is_empty()
        );

        let draft = EditorDraftRecord {
            file_id: 7,
            host_id: 3,
            remote_path: "/etc/nginx/nginx.conf".to_string(),
            base_hash: "sha256:abc".to_string(),
            content: "worker_processes 1;\n".to_string(),
            updated_at: 0,
        };
        save_editor_draft(&pool, &draft).expect("save draft should succeed");

        let mut updated = draft.clone();
        updated.content = "worker_processes 2;\n".to_string();
        save_editor_draft(&pool, &updated).expect("upsert draft should succeed");

        let all = load_editor_drafts(&pool).expect("load should succeed");
        assert_eq!(all.len(), 1);
        let first = all.first().unwrap();
        assert_eq!(first.content, "worker_processes 2;\n");
        assert_eq!(first.file_id, 7);
        assert!(first.updated_at > 0);

        delete_editor_draft(&pool, 7).expect("delete should succeed");
        assert!(
            load_editor_drafts(&pool)
                .expect("load should succeed")
                .is_empty()
        );
        // 删除不存在的草稿不应报错
        delete_editor_draft(&pool, 404).expect("delete missing draft should be a no-op");

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
