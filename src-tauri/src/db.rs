use std::collections::{HashMap, HashSet};
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
    /// secret 存 SQLCipher 加密库；relay_host 为空视为未启用。
    /// 连接时嵌入式配置优先；为空时按 relay_id 从中继库解析
    #[serde(default)]
    pub relay: Option<SshRelayEndpoint>,
    /// 中继库引用（侧栏「中继」页配置的 id）；嵌入式 relay 为空且此值有效时生效
    #[serde(default)]
    pub relay_id: Option<usize>,
    #[serde(default)]
    pub group_ids: Vec<usize>,
    /// shell 内 passwd 改密提示词配置（JSON：{current,new,retype}）。
    /// 三条都非空时 passwd_input 用配置严格匹配（准确度高），
    /// 否则 fallback 内置默认模式。null = 未配置。
    #[serde(default)]
    pub passwd_prompts: Option<crate::terminal::passwd_input::PasswdPrompts>,
    /// 环境级别：dev 开发 / test 测试 / staging 预发 / prod 生产；缺省为 "dev"
    #[serde(default = "default_env_level")]
    pub env_level: String,
    /// 是否开启生产环境安全防护罩（高危命令拦截）；缺省为 true
    #[serde(default = "default_true")]
    pub safety_guard_enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

fn default_env_level() -> String {
    "dev".to_string()
}

fn default_true() -> bool {
    true
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
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
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
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA temp_store = MEMORY;
             PRAGMA cache_size = -2000;",
        )?;
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
          passwd_prompts TEXT,
          env_level TEXT NOT NULL DEFAULT 'dev',
          safety_guard_enabled INTEGER NOT NULL DEFAULT 1,
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
      );

      CREATE TABLE IF NOT EXISTS relays (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          name TEXT NOT NULL,
          host TEXT NOT NULL,
          port INTEGER NOT NULL DEFAULT 7300,
          transport TEXT NOT NULL DEFAULT 'tcp',
          key_id INTEGER NOT NULL DEFAULT 1,
          secret TEXT NOT NULL,
          cert_fingerprint TEXT,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
      );

      CREATE TABLE IF NOT EXISTS zk_hosts (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          name TEXT NOT NULL,
          endpoints TEXT NOT NULL,
          relay_id INTEGER,
          default_credential_id INTEGER,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
      );

      CREATE TABLE IF NOT EXISTS zk_credentials (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          host_id INTEGER NOT NULL,
          username TEXT NOT NULL,
          password TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL,
          UNIQUE(host_id, username)
      );

      -- 主机密码历史：每次自动/手动更新保存密码前，把**被替换掉的旧密码**
      -- 连同修改时间记一条，供「改错了找回上一个密码」。与 hosts 表同级加密
      -- （同一 SQLCipher 库），保留条数由写入侧限制（每主机最多 20 条）。
      CREATE TABLE IF NOT EXISTS host_password_history (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          host_id INTEGER NOT NULL,
          password TEXT NOT NULL,
          -- 该密码被替换的时刻（unix 毫秒）
          replaced_at INTEGER NOT NULL,
          -- 来源：kbd-int-change（认证内改密）/ user-input（手输通过认证）
          source TEXT NOT NULL
      );
      CREATE INDEX IF NOT EXISTS idx_host_password_history_host
          ON host_password_history(host_id, replaced_at DESC);

      CREATE TABLE IF NOT EXISTS grpc_tls_profiles (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          server_name TEXT NOT NULL UNIQUE,
          ca_pem TEXT,
          client_cert_pem TEXT,
          client_key_pem TEXT,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
      );

      CREATE TABLE IF NOT EXISTS command_history (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          host_id INTEGER,
          session_id INTEGER NOT NULL,
          command TEXT NOT NULL,
          exit_code INTEGER NOT NULL,
          created_at INTEGER NOT NULL
      );

      CREATE INDEX IF NOT EXISTS idx_command_history_host_time
          ON command_history(host_id, created_at DESC);
      CREATE INDEX IF NOT EXISTS idx_command_history_failed
          ON command_history(created_at DESC) WHERE exit_code != 0;",
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

    // SSH 中继六列（relay_host 非空 = 启用；其余列仅在 relay_host 非空时有意义）；
    // cert_fingerprint 为后续追加（QUIC 证书 pin）；relay_id 为中继库引用
    // （侧栏「中继」页保存的配置，嵌入式字段优先，引用解析在连接组链处）
    for col_def in [
        "relay_host TEXT",
        "relay_port INTEGER",
        "relay_transport TEXT",
        "relay_key_id INTEGER",
        "relay_secret TEXT",
        "relay_target TEXT",
        "cert_fingerprint TEXT",
        "relay_id INTEGER",
        "passwd_prompts TEXT",
        "env_level TEXT DEFAULT 'dev'",
        "safety_guard_enabled INTEGER DEFAULT 1",
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
    // 内置预置命令播种（幂等差量，见 command_library::seed_builtin_commands）；
    // 失败仅告警不阻断启动——命令库自身功能不受影响
    if let Err(e) = crate::command_library::seed_builtin_commands(&pool) {
        log::warn!("[DB] 内置预置命令播种失败（不影响启动）: {e}");
    }
    let _ = GLOBAL_DB_POOL.set(pool.clone());
    // 命令历史采集开关回灌（kv 持久化；缺省开）
    crate::terminal::command_history::init_enabled_from_db(&pool);
    Ok(pool)
}

// ===== `relays` 中继配置持久化操作 =====

/// 中继配置记录（与 relays.rs 的 RelayRecord 同构；此处独立定义避免模块循环）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RelayRecord {
    pub id: usize,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub transport: RelayTransport,
    pub key_id: u8,
    pub secret: String,
    #[serde(default)]
    pub cert_fingerprint: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub fn list_relays(pool: &DbPool) -> Result<Vec<RelayRecord>, String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, host, port, transport, key_id, secret, cert_fingerprint, created_at, updated_at
       FROM relays ORDER BY id ASC",
        )
        .map_err(|e| format!("failed to prepare relays query: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            let port_i64: i64 = row.get(3)?;
            let transport_raw: String = row.get(4)?;
            let key_id_i64: i64 = row.get(5)?;
            let transport = match transport_raw.trim() {
                "ws" => RelayTransport::Ws,
                "quic" => RelayTransport::Quic,
                _ => RelayTransport::Tcp,
            };
            Ok(RelayRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                host: row.get(2)?,
                port: u16::try_from(port_i64.clamp(1, 65535)).unwrap_or(7300),
                transport,
                key_id: key_id_i64.clamp(0, 255) as u8,
                secret: row.get(6)?,
                cert_fingerprint: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        })
        .map_err(|e| format!("failed to query relays: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("failed to decode relay row: {e}"))?);
    }
    Ok(out)
}

pub fn save_relay(pool: &DbPool, record: RelayRecord) -> Result<RelayRecord, String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let transport = match record.transport {
        RelayTransport::Tcp => "tcp",
        RelayTransport::Ws => "ws",
        RelayTransport::Quic => "quic",
    };
    let cert = record
        .cert_fingerprint
        .as_deref()
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty());
    if record.id == 0 {
        conn.execute(
            "INSERT INTO relays (name, host, port, transport, key_id, secret, cert_fingerprint, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            rusqlite::params![
                record.name,
                record.host,
                i64::from(record.port),
                transport,
                i64::from(record.key_id),
                record.secret,
                cert,
                now,
            ],
        )
        .map_err(|e| format!("failed to insert relay: {e}"))?;
        let id: usize = conn.last_insert_rowid() as usize;
        Ok(RelayRecord { id, created_at: now, updated_at: now, ..record })
    } else {
        conn.execute(
            "UPDATE relays SET name=?2, host=?3, port=?4, transport=?5, key_id=?6, secret=?7, cert_fingerprint=?8, updated_at=?9
             WHERE id=?1",
            rusqlite::params![
                record.id,
                record.name,
                record.host,
                i64::from(record.port),
                transport,
                i64::from(record.key_id),
                record.secret,
                cert,
                now,
            ],
        )
        .map_err(|e| format!("failed to update relay: {e}"))?;
        Ok(RelayRecord { updated_at: now, ..record })
    }
}

pub fn delete_relay(pool: &DbPool, id: usize) -> Result<(), String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    conn.execute("DELETE FROM relays WHERE id=?1", rusqlite::params![id])
        .map_err(|e| format!("failed to delete relay: {e}"))?;
    Ok(())
}

// ===== `zk_hosts` / `zk_credentials` ZK 主机持久化操作 =====

/// ZK 主机凭证（对应 `zk_credentials` 表；密码明文存 SQLCipher 加密库，与 SSH 密码同级别）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZkCredentialRecord {
    pub id: usize,
    pub username: String,
    pub password: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// ZK 主机完整持久化模型（对应 `zk_hosts` 表 + 按主机挂载的凭证列表，按 id ASC）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZkHostRecord {
    pub id: usize,
    pub name: String,
    pub endpoints: String,
    pub relay_id: Option<usize>,
    pub default_credential_id: Option<usize>,
    pub credentials: Vec<ZkCredentialRecord>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 列出全部 ZK 主机（id ASC），每主机挂 credentials（id ASC，密码明文返回）。
pub fn list_zk_hosts(pool: &DbPool) -> Result<Vec<ZkHostRecord>, String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;

    // 凭证按 host_id 分组（id ASC）
    let mut credentials_by_host: HashMap<usize, Vec<ZkCredentialRecord>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT id, host_id, username, password, created_at, updated_at
           FROM zk_credentials
           ORDER BY id ASC",
            )
            .map_err(|e| format!("failed to prepare zk_credentials query: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, usize>(1)?,
                    ZkCredentialRecord {
                        id: row.get(0)?,
                        username: row.get(2)?,
                        password: row.get(3)?,
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                    },
                ))
            })
            .map_err(|e| format!("failed to query zk_credentials: {e}"))?;
        for r in rows {
            let (host_id, cred) =
                r.map_err(|e| format!("failed to decode zk credential row: {e}"))?;
            credentials_by_host.entry(host_id).or_default().push(cred);
        }
    }

    let mut stmt = conn
        .prepare(
            "SELECT id, name, endpoints, relay_id, default_credential_id, created_at, updated_at
       FROM zk_hosts
       ORDER BY id ASC",
        )
        .map_err(|e| format!("failed to prepare zk_hosts query: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            let id: usize = row.get(0)?;
            Ok(ZkHostRecord {
                id,
                name: row.get(1)?,
                endpoints: row.get(2)?,
                relay_id: row.get::<_, Option<i64>>(3)?.map(|v| v.max(0) as usize),
                default_credential_id: row.get::<_, Option<i64>>(4)?.map(|v| v.max(0) as usize),
                credentials: credentials_by_host.remove(&id).unwrap_or_default(),
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .map_err(|e| format!("failed to query zk_hosts: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("failed to decode zk host row: {e}"))?);
    }
    Ok(out)
}

/// 是否为 zk_credentials 的 UNIQUE(host_id, username) 冲突（SQLite 报错文案
/// 形如 `UNIQUE constraint failed: zk_credentials.host_id, zk_credentials.username`）。
fn is_username_unique_violation(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ffi::ErrorCode::ConstraintViolation,
                ..
            },
            Some(message),
        ) if message.contains("zk_credentials.username")
    )
}

fn zk_credential_write_error(cred: &ZkCredentialRecord, err: rusqlite::Error) -> String {
    if is_username_unique_violation(&err) {
        format!("凭证用户名重复：{}", cred.username)
    } else {
        format!("failed to write zk credential: {err}")
    }
}

/// 解析 default_credential_id：
/// 1) 指向同步后仍属于本主机的凭证（UPDATE 命中）→ 原样保留；
/// 2) 指向请求内凭证（按请求侧 id 匹配——新建主机 / 新增凭证时前端只能给 id=0
///    占位，按请求顺序映射到自增真实 id）→ 映射后的真实 id；
/// 3) 其余（不存在 / 不属于本主机 / 已随同步删除）→ None。
fn resolve_default_credential(
    requested: Option<usize>,
    kept_ids: &[usize],
    request_credentials: &[ZkCredentialRecord],
    inserted: &[(usize, usize)],
) -> Option<usize> {
    let want = requested?;
    if kept_ids.contains(&want) {
        return Some(want);
    }
    let pos = request_credentials.iter().position(|c| c.id == want)?;
    inserted
        .iter()
        .find(|(p, _)| *p == pos)
        .map(|(_, real)| *real)
}

/// 单条插入凭证（忽略请求侧 id，走自增），返回真实 id。
fn insert_zk_credential(
    tx: &rusqlite::Transaction<'_>,
    host_id: usize,
    cred: &ZkCredentialRecord,
    now: i64,
) -> Result<usize, String> {
    tx.execute(
        "INSERT INTO zk_credentials (host_id, username, password, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![host_id, cred.username, cred.password, now],
    )
    .map_err(|e| zk_credential_write_error(cred, e))?;
    Ok(tx.last_insert_rowid() as usize)
}

/// 本主机现存凭证 id 列表（id ASC）。
fn zk_credential_ids(conn: &rusqlite::Connection, host_id: usize) -> Result<Vec<usize>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM zk_credentials WHERE host_id = ?1 ORDER BY id ASC")
        .map_err(|e| format!("failed to prepare zk_credentials query: {e}"))?;
    let rows = stmt
        .query_map(params![host_id], |row| row.get::<_, usize>(0))
        .map_err(|e| format!("failed to query zk_credentials: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("failed to decode zk credential id: {e}"))?);
    }
    Ok(out)
}

/// 读取单个 ZK 主机（含凭证，id ASC）；不存在返回 None。
fn read_zk_host(conn: &rusqlite::Connection, id: usize) -> Result<Option<ZkHostRecord>, String> {
    let mut host = conn
        .query_row(
            "SELECT id, name, endpoints, relay_id, default_credential_id, created_at, updated_at
       FROM zk_hosts WHERE id = ?1",
            params![id],
            |row| {
                Ok(ZkHostRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    endpoints: row.get(2)?,
                    relay_id: row.get::<_, Option<i64>>(3)?.map(|v| v.max(0) as usize),
                    default_credential_id: row.get::<_, Option<i64>>(4)?.map(|v| v.max(0) as usize),
                    credentials: Vec::new(),
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|e| format!("failed to query zk host {id}: {e}"))?;
    if let Some(host) = host.as_mut() {
        let mut stmt = conn
            .prepare(
                "SELECT id, username, password, created_at, updated_at
           FROM zk_credentials WHERE host_id = ?1 ORDER BY id ASC",
            )
            .map_err(|e| format!("failed to prepare zk_credentials query: {e}"))?;
        let rows = stmt
            .query_map(params![id], |row| {
                Ok(ZkCredentialRecord {
                    id: row.get(0)?,
                    username: row.get(1)?,
                    password: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .map_err(|e| format!("failed to query zk_credentials: {e}"))?;
        for r in rows {
            host.credentials
                .push(r.map_err(|e| format!("failed to decode zk credential row: {e}"))?);
        }
    }
    Ok(host)
}

/// 新建（id=0）或更新（id>0）ZK 主机，单事务内原子完成：
/// - id=0：插入 zk_hosts + 逐条插入凭证（id 忽略走自增），default_credential_id
///   按请求侧 id 解析到自增真实 id 后回填；
/// - id>0：更新 meta（name/endpoints/relay_id/default_credential_id/updated_at），
///   凭证做集合同步——请求 id>0 的按 id UPDATE（未命中视为新增）、id=0 的 INSERT、
///   库里存在但请求缺失的 DELETE（先删后写，允许「删旧用户 + 新增同名用户」）；
///   同步后 default_credential_id 不指向本主机现存凭证 → 置 None（落库与返回值都置）。
/// - username 在同一主机内冲突（撞 UNIQUE(host_id, username)）→ `凭证用户名重复：{username}`。
///
/// 返回落库后的完整记录（从库里回读，含真实 id）。
pub fn save_zk_host(pool: &DbPool, record: ZkHostRecord) -> Result<ZkHostRecord, String> {
    let name = record.name.trim().to_string();
    let endpoints = record.endpoints.trim().to_string();
    if name.is_empty() {
        return Err("ZK 主机名称不能为空".to_string());
    }
    if endpoints.is_empty() {
        return Err("ZK 地址不能为空".to_string());
    }

    let mut conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    let tx = conn
        .transaction()
        .map_err(|e| format!("failed to begin transaction: {e}"))?;
    let now = now_timestamp_ms();
    let relay_id: Option<i64> = record.relay_id.map(|v| v as i64);

    if record.id == 0 {
        // 新建：default 先置空，凭证自增 id 分配后再解析回填
        tx.execute(
            "INSERT INTO zk_hosts (name, endpoints, relay_id, default_credential_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, NULL, ?4, ?4)",
            params![name, endpoints, relay_id, now],
        )
        .map_err(|e| format!("failed to insert zk host: {e}"))?;
        let host_id = tx.last_insert_rowid() as usize;

        let mut inserted: Vec<(usize, usize)> = Vec::new();
        for (pos, cred) in record.credentials.iter().enumerate() {
            let real_id = insert_zk_credential(&tx, host_id, cred, now)?;
            inserted.push((pos, real_id));
        }
        let resolved =
            resolve_default_credential(record.default_credential_id, &[], &record.credentials, &inserted);
        tx.execute(
            "UPDATE zk_hosts SET default_credential_id = ?2 WHERE id = ?1",
            params![host_id, resolved],
        )
        .map_err(|e| format!("failed to update zk host default credential: {e}"))?;
        tx.commit()
            .map_err(|e| format!("failed to commit zk host insert: {e}"))?;
        read_zk_host(&conn, host_id)?.ok_or_else(|| format!("failed to read back zk host {host_id}"))
    } else {
        // 更新 meta（default 先写请求值，凭证同步后统一校正）
        let changed = tx
            .execute(
                "UPDATE zk_hosts SET name=?2, endpoints=?3, relay_id=?4, default_credential_id=?5, updated_at=?6
                 WHERE id=?1",
                params![
                    record.id,
                    name,
                    endpoints,
                    relay_id,
                    record.default_credential_id.map(|v| v as i64),
                    now
                ],
            )
            .map_err(|e| format!("failed to update zk host: {e}"))?;
        if changed == 0 {
            return Err(format!("ZK 主机不存在：id={}", record.id));
        }

        // 集合同步（先删缺失，再写请求集合）
        let existing_ids = zk_credential_ids(&tx, record.id)?;
        let requested_ids: Vec<usize> = record
            .credentials
            .iter()
            .map(|c| c.id)
            .filter(|id| *id > 0)
            .collect();
        for existing in existing_ids {
            if !requested_ids.contains(&existing) {
                tx.execute(
                    "DELETE FROM zk_credentials WHERE host_id = ?1 AND id = ?2",
                    params![record.id, existing],
                )
                .map_err(|e| format!("failed to delete zk credential {existing}: {e}"))?;
            }
        }

        let mut kept_ids: Vec<usize> = Vec::new();
        let mut inserted: Vec<(usize, usize)> = Vec::new();
        for (pos, cred) in record.credentials.iter().enumerate() {
            if cred.id > 0 {
                let hit = tx
                    .execute(
                        "UPDATE zk_credentials SET username=?2, password=?3, updated_at=?4
                         WHERE id=?1 AND host_id=?5",
                        params![cred.id, cred.username, cred.password, now, record.id],
                    )
                    .map_err(|e| zk_credential_write_error(cred, e))?;
                if hit > 0 {
                    kept_ids.push(cred.id);
                } else {
                    // id 不属于本主机（或已不存在）：按新增处理，id 忽略走自增
                    let real_id = insert_zk_credential(&tx, record.id, cred, now)?;
                    inserted.push((pos, real_id));
                }
            } else {
                let real_id = insert_zk_credential(&tx, record.id, cred, now)?;
                inserted.push((pos, real_id));
            }
        }

        // default 校正：不指向本主机同步后现存凭证 → 置 None
        let resolved = resolve_default_credential(
            record.default_credential_id,
            &kept_ids,
            &record.credentials,
            &inserted,
        );
        tx.execute(
            "UPDATE zk_hosts SET default_credential_id = ?2 WHERE id = ?1",
            params![record.id, resolved],
        )
        .map_err(|e| format!("failed to update zk host default credential: {e}"))?;
        tx.commit()
            .map_err(|e| format!("failed to commit zk host update: {e}"))?;
        read_zk_host(&conn, record.id)?
            .ok_or_else(|| format!("failed to read back zk host {}", record.id))
    }
}

/// 删除 ZK 主机（zk_credentials 无外键约束，先删凭证再删主机）；不存在时静默成功（幂等）。
pub fn delete_zk_host(pool: &DbPool, id: usize) -> Result<(), String> {
    let mut conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    let tx = conn
        .transaction()
        .map_err(|e| format!("failed to begin transaction: {e}"))?;
    tx.execute("DELETE FROM zk_credentials WHERE host_id = ?1", params![id])
        .map_err(|e| format!("failed to delete zk credentials: {e}"))?;
    tx.execute("DELETE FROM zk_hosts WHERE id = ?1", params![id])
        .map_err(|e| format!("failed to delete zk host: {e}"))?;
    tx.commit()
        .map_err(|e| format!("failed to commit zk host delete: {e}"))?;
    Ok(())
}

// ===== `grpc_tls_profiles`：gRPC TLS 配置库（校验域名为唯一 key）=====

/// gRPC TLS 配置记录：CA/客户端证书/私钥明文存 SQLCipher 加密库（与 SSH 密码同级）；
/// server_name 是用户可读的唯一标识（下拉展示、冲突即报错）。PEM 全部可空——
/// 空 CA = 系统根证书，空客户端证书对 = 不做 mTLS。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GrpcTlsProfileRecord {
    pub id: usize,
    pub server_name: String,
    #[serde(default)]
    pub ca_pem: Option<String>,
    #[serde(default)]
    pub client_cert_pem: Option<String>,
    #[serde(default)]
    pub client_key_pem: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub fn list_grpc_tls_profiles(pool: &DbPool) -> Result<Vec<GrpcTlsProfileRecord>, String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    let mut stmt = conn
        .prepare(
            "SELECT id, server_name, ca_pem, client_cert_pem, client_key_pem, created_at, updated_at
       FROM grpc_tls_profiles ORDER BY server_name ASC",
        )
        .map_err(|e| format!("failed to prepare grpc tls profiles query: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(GrpcTlsProfileRecord {
                id: row.get(0)?,
                server_name: row.get(1)?,
                ca_pem: row.get(2)?,
                client_cert_pem: row.get(3)?,
                client_key_pem: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .map_err(|e| format!("failed to query grpc tls profiles: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("failed to decode grpc tls profile row: {e}"))?);
    }
    Ok(out)
}

pub fn save_grpc_tls_profile(
    pool: &DbPool,
    record: GrpcTlsProfileRecord,
) -> Result<GrpcTlsProfileRecord, String> {
    if record.client_cert_pem.is_some() != record.client_key_pem.is_some() {
        return Err("客户端证书与私钥必须成对提供".into());
    }
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    // PEM 原文存储（trim 会破坏尾换行等格式）；trim 仅用于「是否为空」判断，空串归一为 None
    let clean_opt = |v: &Option<String>| v.as_ref().filter(|s| !s.trim().is_empty()).cloned();
    if record.id == 0 {
        conn.execute(
            "INSERT INTO grpc_tls_profiles (server_name, ca_pem, client_cert_pem, client_key_pem, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![
                record.server_name,
                clean_opt(&record.ca_pem),
                clean_opt(&record.client_cert_pem),
                clean_opt(&record.client_key_pem),
                now,
            ],
        )
        .map_err(|e| grpc_tls_unique_error(&record.server_name, &e))?;
        let id: usize = conn.last_insert_rowid() as usize;
        Ok(GrpcTlsProfileRecord {
            id,
            created_at: now,
            updated_at: now,
            ..record
        })
    } else {
        conn.execute(
            "UPDATE grpc_tls_profiles SET server_name=?2, ca_pem=?3, client_cert_pem=?4, client_key_pem=?5, updated_at=?6
             WHERE id=?1",
            params![
                record.id,
                record.server_name,
                clean_opt(&record.ca_pem),
                clean_opt(&record.client_cert_pem),
                clean_opt(&record.client_key_pem),
                now,
            ],
        )
        .map_err(|e| grpc_tls_unique_error(&record.server_name, &e))?;
        Ok(GrpcTlsProfileRecord {
            updated_at: now,
            ..record
        })
    }
}

/// UNIQUE(server_name) 冲突转成业务文案，其余原样上抛。
fn grpc_tls_unique_error(server_name: &str, err: &rusqlite::Error) -> String {
    if matches!(
        err,
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ffi::ErrorCode::ConstraintViolation,
                ..
            },
            ..
        )
    ) {
        format!("校验域名已存在：{server_name}")
    } else {
        format!("failed to write grpc tls profile: {err}")
    }
}

/// 删除 gRPC TLS 配置；不存在时静默成功（幂等）。
pub fn delete_grpc_tls_profile(pool: &DbPool, id: usize) -> Result<(), String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    conn.execute(
        "DELETE FROM grpc_tls_profiles WHERE id=?1",
        params![id],
    )
    .map_err(|e| format!("failed to delete grpc tls profile: {e}"))?;
    Ok(())
}

/// 按 id 取单条配置（grpc_call 命令层解析用）。
pub fn grpc_tls_profile_by_id(pool: &DbPool, id: usize) -> Result<GrpcTlsProfileRecord, String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    conn.query_row(
        "SELECT id, server_name, ca_pem, client_cert_pem, client_key_pem, created_at, updated_at
         FROM grpc_tls_profiles WHERE id=?1",
        params![id],
        |row| {
            Ok(GrpcTlsProfileRecord {
                id: row.get(0)?,
                server_name: row.get(1)?,
                ca_pem: row.get(2)?,
                client_cert_pem: row.get(3)?,
                client_key_pem: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        },
    )
    .map_err(|_| format!("TLS 配置不存在（id={id}），可能已被删除"))
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

/// 更新主机保存的登录密码（kbd-int 认证内改密成功后自动回写）。
/// 只动 auth_json 单列：读现行 JSON → 仅当为 Password 变体时替换
/// password 字段 → UPDATE（Key/Agent 变体不动——改密流程只在密码
/// 认证场景触发）。返回是否实际更新。
pub fn update_host_password(
    pool: &DbPool,
    host_id: usize,
    new_password: &str,
) -> Result<bool, String> {
    update_host_password_with_source(pool, host_id, new_password, "kbd-int-change")
}

/// 密码变更的**唯一入口**：写历史 + 更新密码 + 发事件，三步一条龙。
///
/// 所有触发密码变更的路径（kbd-int 改密 / kbd-int 手输 / shell-passwd /
/// 将来的手动更新）都应调这个方法，不要各自重复实现三步逻辑。
///
/// - `source`：`kbd-int-change` / `user-input` / `shell-passwd`（写入历史表）
/// - `app_handle`：有值时成功后 emit `host-password-updated`（前端刷新内存态）
///
/// 返回 `Ok(true)` = 密码实际变更；`Ok(false)` = 主机不存在或非密码认证（静默跳过）
pub fn change_host_password(
    host_id: usize,
    new_password: &str,
    source: &str,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<bool, String> {
    let pool = global_pool().ok_or("db pool unavailable")?;
    let updated = update_host_password_with_source(&pool, host_id, new_password, source)?;
    if updated {
        log::info!(
            "[DB] host {} password updated (source: {})",
            host_id,
            source
        );
        if let Some(app) = app_handle {
            use tauri::Emitter;
            let _ = app.emit(
                "host-password-updated",
                serde_json::json!({ "hostId": host_id }),
            );
        }
    }
    Ok(updated)
}

/// 更新保存密码并在历史表留档被替换的旧密码。
///
/// `source`：`kbd-int-change`（认证内改密）或 `user-input`（手输密码通过认证）。
/// 历史留档发生在 UPDATE 之前（旧值只在更新前可得），失败不阻断主流程——
/// 找回密码是辅助能力，不能因为历史写入失败而影响登录密码更新。
pub fn update_host_password_with_source(
    pool: &DbPool,
    host_id: usize,
    new_password: &str,
    source: &str,
) -> Result<bool, String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    let auth_json: Option<String> = conn
        .query_row(
            "SELECT auth_json FROM hosts WHERE id = ?1",
            params![host_id as i64],
            |row| row.get(0),
        )
        .map_err(|e| format!("failed to read host auth: {e}"))?;
    let Some(auth_json) = auth_json else {
        return Ok(false); // 主机不存在（临时连接等）——静默跳过
    };
    let mut auth: SSHTargetAuth = serde_json::from_str(&auth_json)
        .map_err(|e| format!("failed to parse auth_json: {e}"))?;
    let old_password = match &mut auth {
        SSHTargetAuth::Password { password } => {
            let old = password.clone();
            *password = new_password.to_string();
            old
        }
        _ => return Ok(false), // Key/Agent：密码改密流程不适用，不动
    };
    let new_json = serde_json::to_string(&auth)
        .map_err(|e| format!("failed to serialize auth: {e}"))?;
    let now = now_timestamp_ms();
    // 历史留档：把即将被替换的旧密码记一条（旧值 == 新值时不记，避免噪声）。
    // 失败只告警不阻断——找回密码是辅助能力，不能影响主流程的密码更新。
    if !old_password.is_empty() && old_password != new_password {
        if let Err(e) = conn.execute(
            "INSERT INTO host_password_history (host_id, password, replaced_at, source)
             VALUES (?1, ?2, ?3, ?4)",
            params![host_id as i64, old_password, now, source],
        ) {
            log::warn!("[DB] 密码历史写入失败（不影响本次更新）: {e}");
        } else {
            // 每主机最多保留 20 条，超出按时间淘汰最旧的
            let _ = conn.execute(
                "DELETE FROM host_password_history WHERE host_id = ?1 AND id NOT IN (
                     SELECT id FROM host_password_history WHERE host_id = ?1
                     ORDER BY replaced_at DESC, id DESC LIMIT 20
                 )",
                params![host_id as i64],
            );
        }
    }
    let changed = conn
        .execute(
            "UPDATE hosts SET auth_json = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_json, now, host_id as i64],
        )
        .map_err(|e| format!("failed to update host password: {e}"))?;
    if changed == 0 {
        return Ok(false);
    }
    // 回读校验：确认落库值与传入一致（调用方据此打印比对）
    let saved_json: String = conn
        .query_row(
            "SELECT auth_json FROM hosts WHERE id = ?1",
            params![host_id as i64],
            |row| row.get(0),
        )
        .map_err(|e| format!("failed to re-read host auth: {e}"))?;
    let saved: SSHTargetAuth = serde_json::from_str(&saved_json)
        .map_err(|e| format!("failed to parse saved auth: {e}"))?;
    match saved {
        SSHTargetAuth::Password { password } => {
            if password == new_password {
                Ok(true)
            } else {
                Err(format!(
                    "回读不一致：写入 {} 字节、读回 {} 字节",
                    new_password.len(),
                    password.len()
                ))
            }
        }
        _ => Err("回读 auth 不是 Password 变体".to_string()),
    }
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
    cert_fingerprint: Option<&str>,
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
        cert_fingerprint: cert_fingerprint
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty()),
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
              relay_host, relay_port, relay_transport, relay_key_id, relay_secret, cert_fingerprint, relay_id,
              passwd_prompts, env_level, safety_guard_enabled,
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
                row.get::<_, Option<String>>(15)?.as_deref(),
            );
            let relay_id: Option<usize> = row.get::<_, Option<i64>>(16)?.map(|v| v.max(0) as usize);
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
                relay_id,
                passwd_prompts: row
                    .get::<_, Option<String>>(17)?
                    .and_then(|j| serde_json::from_str(&j).ok()),
                env_level: row.get::<_, Option<String>>(18)?.unwrap_or_else(default_env_level),
                safety_guard_enabled: row.get::<_, Option<i64>>(19)?.map(|v| v != 0).unwrap_or(true),
                group_ids: groups_by_host.remove(&id).unwrap_or_default(),
                created_at: row.get(20)?,
                updated_at: row.get(21)?,
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
                relay_id: None,
                passwd_prompts: None,
                env_level: default_env_level(),
                safety_guard_enabled: true,
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
/// 性能优化：采用增量 Diff + UPSERT 机制替代旧版的清空重插反模式，
/// 仅删除已移除项、按 ID 冲突增量更新现有项，保护 SQLite B-Tree 页面连续性并避免全表抖动与数据库碎片膨胀。
pub fn save_host_store(pool: &DbPool, store: &HostStorePayload) -> Result<(), String> {
    let mut conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    let tx = conn
        .transaction()
        .map_err(|e| format!("failed to begin transaction: {e}"))?;

    // ── 1. 分组 (groups) 增量同步 ──
    let incoming_group_ids: HashSet<usize> = store.groups.iter().map(|g| g.id).collect();
    {
        let mut stmt_existing_groups = tx
            .prepare("SELECT id FROM groups")
            .map_err(|e| format!("failed to prepare existing groups query: {e}"))?;
        let existing_group_ids: HashSet<usize> = stmt_existing_groups
            .query_map([], |row| row.get(0))
            .map_err(|e| format!("failed to query existing groups: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        // 仅删除已从前端移除的分组
        let mut stmt_del_group = tx
            .prepare("DELETE FROM groups WHERE id = ?1")
            .map_err(|e| format!("failed to prepare delete group statement: {e}"))?;
        for gid in existing_group_ids.difference(&incoming_group_ids) {
            stmt_del_group
                .execute(params![gid])
                .map_err(|e| format!("failed to delete removed group {gid}: {e}"))?;
        }

        // UPSERT 现有或新增分组
        let mut stmt_group = tx
            .prepare(
                "INSERT INTO groups (id, parent_id, name, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                   parent_id = excluded.parent_id,
                   name = excluded.name,
                   updated_at = excluded.updated_at",
            )
            .map_err(|e| format!("failed to prepare group upsert: {e}"))?;

        for group in &store.groups {
            stmt_group
                .execute(params![
                    group.id,
                    group.parent_id,
                    &group.name,
                    group.created_at,
                    group.updated_at
                ])
                .map_err(|e| format!("failed to upsert group {}: {e}", group.id))?;
        }
    }

    let group_name_by_id: HashMap<usize, String> = store
        .groups
        .iter()
        .map(|g| (g.id, g.name.clone()))
        .collect();

    // ── 2. 主机 (hosts) 及其关联表增量同步 ──
    let incoming_host_ids: HashSet<usize> = store.hosts.iter().map(|h| h.id).collect();
    {
        let mut stmt_existing_hosts = tx
            .prepare("SELECT id FROM hosts")
            .map_err(|e| format!("failed to prepare existing hosts query: {e}"))?;
        let existing_host_ids: HashSet<usize> = stmt_existing_hosts
            .query_map([], |row| row.get(0))
            .map_err(|e| format!("failed to query existing hosts: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        // 仅删除已从前端移除的主机及其在关联表中的孤儿行
        let mut stmt_del_host_groups = tx
            .prepare("DELETE FROM host_groups WHERE host_id = ?1")
            .map_err(|e| format!("failed to prepare delete host_groups: {e}"))?;
        let mut stmt_del_session = tx
            .prepare("DELETE FROM sessions WHERE id = ?1")
            .map_err(|e| format!("failed to prepare delete session: {e}"))?;
        let mut stmt_del_host = tx
            .prepare("DELETE FROM hosts WHERE id = ?1")
            .map_err(|e| format!("failed to prepare delete host: {e}"))?;

        for hid in existing_host_ids.difference(&incoming_host_ids) {
            stmt_del_host_groups
                .execute(params![hid])
                .map_err(|e| format!("failed to delete removed host_groups for host {hid}: {e}"))?;
            stmt_del_session
                .execute(params![hid])
                .map_err(|e| format!("failed to delete removed session for host {hid}: {e}"))?;
            stmt_del_host
                .execute(params![hid])
                .map_err(|e| format!("failed to delete removed host {hid}: {e}"))?;
        }

        // UPSERT 现有或新增主机
        let mut stmt_host = tx
            .prepare(
                "INSERT INTO hosts (
                  id, name, host, port, username, auth_json, jump_host_id,
                  host_key_verification, allow_insecure_algos, ready_command_id,
                  relay_host, relay_port, relay_transport, relay_key_id, relay_secret, cert_fingerprint, relay_id,
                  passwd_prompts, env_level, safety_guard_enabled,
                  created_at, updated_at
               ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)
               ON CONFLICT(id) DO UPDATE SET
                  name = excluded.name,
                  host = excluded.host,
                  port = excluded.port,
                  username = excluded.username,
                  auth_json = excluded.auth_json,
                  jump_host_id = excluded.jump_host_id,
                  host_key_verification = excluded.host_key_verification,
                  allow_insecure_algos = excluded.allow_insecure_algos,
                  ready_command_id = excluded.ready_command_id,
                  relay_host = excluded.relay_host,
                  relay_port = excluded.relay_port,
                  relay_transport = excluded.relay_transport,
                  relay_key_id = excluded.relay_key_id,
                  relay_secret = excluded.relay_secret,
                  cert_fingerprint = excluded.cert_fingerprint,
                  relay_id = excluded.relay_id,
                  passwd_prompts = excluded.passwd_prompts,
                  env_level = excluded.env_level,
                  safety_guard_enabled = excluded.safety_guard_enabled,
                  updated_at = excluded.updated_at",
            )
            .map_err(|e| format!("failed to prepare host upsert: {e}"))?;

        let mut stmt_host_group = tx
            .prepare("INSERT OR IGNORE INTO host_groups (host_id, group_id) VALUES (?1, ?2)")
            .map_err(|e| format!("failed to prepare host_group insert: {e}"))?;

        let mut stmt_session = tx
            .prepare(
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
            )
            .map_err(|e| format!("failed to prepare session upsert: {e}"))?;

        for host in &store.hosts {
            let auth_json = serde_json::to_string(&host.auth)
                .map_err(|e| format!("failed to serialize host auth: {e}"))?;
            let verification_str = verification_mode_to_str(host.host_key_verification);
            let allow_insecure_int = if host.allow_insecure_algos {
                1i64
            } else {
                0i64
            };

            stmt_host
                .execute(params![
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
                    host.relay.as_ref().and_then(|r| r.cert_fingerprint.clone()),
                    host.relay_id,
                    host.passwd_prompts
                        .as_ref()
                        .map(|p| serde_json::to_string(p).unwrap_or_default()),
                    &host.env_level,
                    if host.safety_guard_enabled { 1i64 } else { 0i64 },
                    host.created_at,
                    host.updated_at,
                ])
                .map_err(|e| format!("failed to upsert host {}: {e}", host.id))?;

            // 局部刷新单主机的 group 关系
            stmt_del_host_groups
                .execute(params![host.id])
                .map_err(|e| format!("failed to clear host_groups for host {}: {e}", host.id))?;

            for &gid in &host.group_ids {
                if group_name_by_id.contains_key(&gid) {
                    stmt_host_group
                        .execute(params![host.id, gid])
                        .map_err(|e| format!("failed to insert host_group ({}, {}): {e}", host.id, gid))?;
                }
            }

            let primary_group_name = host
                .group_ids
                .first()
                .and_then(|gid| group_name_by_id.get(gid))
                .cloned();

            stmt_session
                .execute(params![
                    host.id,
                    &host.name,
                    &host.host,
                    i64::from(host.port),
                    &host.username,
                    auth_method_str(&host.auth),
                    primary_group_name,
                    host.created_at,
                    host.updated_at,
                ])
                .map_err(|e| format!("failed to mirror host {} into sessions: {e}", host.id))?;
        }
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
                    cert_fingerprint: Some(
                        "F3:6A:D3:91:5D:6E:69:F8:5C:55:63:F6:C5:A0:7F:8E".to_string(),
                    ),
                }),
                relay_id: Some(1),
                passwd_prompts: None,
                env_level: "prod".to_string(),
                safety_guard_enabled: true,
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
                relay_id: None,
                passwd_prompts: None,
                env_level: "dev".to_string(),
                safety_guard_enabled: true,
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
    fn relay_store_roundtrip() {
        let temp_dir =
            std::env::temp_dir().join(format!("veloterm-relay-test-{}", Uuid::new_v4()));
        let db_path = temp_dir.join("veloterm.db");
        let pool = init_db_at_path(&db_path, "relay-key").expect("init_db_at_path should succeed");

        let created = save_relay(
            &pool,
            RelayRecord {
                id: 0,
                name: "edge-relay".to_string(),
                host: "relay.example.com".to_string(),
                port: 7301,
                transport: RelayTransport::Ws,
                key_id: 2,
                secret: "cd".repeat(32),
                cert_fingerprint: Some("AB:CD:EF".to_string()),
                created_at: 0,
                updated_at: 0,
            },
        )
        .expect("save_relay insert should succeed");
        assert!(created.id > 0);
        assert!(created.created_at > 0);

        let updated = save_relay(
            &pool,
            RelayRecord {
                port: 9443,
                transport: RelayTransport::Quic,
                cert_fingerprint: None,
                ..created.clone()
            },
        )
        .expect("save_relay update should succeed");
        assert_eq!(updated.id, created.id);

        let listed = list_relays(&pool).expect("list_relays should succeed");
        assert_eq!(listed, vec![updated.clone()]);

        delete_relay(&pool, updated.id).expect("delete_relay should succeed");
        assert!(
            list_relays(&pool)
                .expect("list after delete should succeed")
                .is_empty()
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn zk_host_store_roundtrip() {
        let temp_dir =
            std::env::temp_dir().join(format!("veloterm-zkhost-test-{}", Uuid::new_v4()));
        let db_path = temp_dir.join("veloterm.db");
        let pool =
            init_db_at_path(&db_path, "zk-host-key").expect("init_db_at_path should succeed");

        // 1. 新增：id=0、2 条凭证（请求侧 id=0 占位）、default 指向第一条
        let created = save_zk_host(
            &pool,
            ZkHostRecord {
                id: 0,
                name: "prod-zk".to_string(),
                endpoints: "10.0.0.1:2181,10.0.0.2:2181".to_string(),
                relay_id: Some(3),
                default_credential_id: Some(0),
                credentials: vec![
                    ZkCredentialRecord {
                        id: 0,
                        username: "alice".to_string(),
                        password: "alice-pass".to_string(),
                        created_at: 0,
                        updated_at: 0,
                    },
                    ZkCredentialRecord {
                        id: 0,
                        username: "bob".to_string(),
                        password: "bob-pass".to_string(),
                        created_at: 0,
                        updated_at: 0,
                    },
                ],
                created_at: 0,
                updated_at: 0,
            },
        )
        .expect("save_zk_host insert should succeed");
        assert!(created.id > 0);
        assert!(created.created_at > 0);
        assert_eq!(created.credentials.len(), 2);
        let alice_id = created
            .credentials
            .first()
            .expect("first credential should exist")
            .id;
        let bob_id = created
            .credentials
            .get(1)
            .expect("second credential should exist")
            .id;
        assert!(alice_id > 0);
        assert_eq!(bob_id, alice_id + 1);
        // default 已解析到第一条凭证的自增真实 id
        assert_eq!(created.default_credential_id, Some(alice_id));
        let listed = list_zk_hosts(&pool).expect("list_zk_hosts should succeed");
        assert_eq!(listed, vec![created.clone()]);

        // 2. 更新：改名/endpoints、凭证删一条（bob 缺失）、加一条（id=0）、
        //    改一条 username/password（alice）、default 换人（指向新增凭证）
        let updated = save_zk_host(
            &pool,
            ZkHostRecord {
                id: created.id,
                name: "prod-zk-2".to_string(),
                endpoints: "10.1.0.1:2181".to_string(),
                relay_id: None,
                default_credential_id: Some(0),
                credentials: vec![
                    ZkCredentialRecord {
                        id: alice_id,
                        username: "alice2".to_string(),
                        password: "new-pass".to_string(),
                        created_at: 0,
                        updated_at: 0,
                    },
                    ZkCredentialRecord {
                        id: 0,
                        username: "carol".to_string(),
                        password: "carol-pass".to_string(),
                        created_at: 0,
                        updated_at: 0,
                    },
                ],
                created_at: created.created_at,
                updated_at: created.updated_at,
            },
        )
        .expect("save_zk_host update should succeed");
        assert_eq!(updated.name, "prod-zk-2");
        assert_eq!(updated.endpoints, "10.1.0.1:2181");
        assert_eq!(updated.relay_id, None);
        assert_eq!(updated.created_at, created.created_at);
        let usernames: Vec<&str> = updated
            .credentials
            .iter()
            .map(|c| c.username.as_str())
            .collect();
        assert_eq!(usernames, vec!["alice2", "carol"]);
        // alice 更新保留原 id；carol 分配自增 id；default 换到 carol
        let carol_id = updated
            .credentials
            .get(1)
            .expect("second credential should exist")
            .id;
        assert_eq!(
            updated
                .credentials
                .first()
                .expect("first credential should exist")
                .id,
            alice_id
        );
        assert!(carol_id > bob_id);
        assert_eq!(updated.default_credential_id, Some(carol_id));
        let listed = list_zk_hosts(&pool).expect("list_zk_hosts should succeed");
        assert_eq!(listed, vec![updated.clone()]);
        // 被删凭证（bob）不在
        let stored = listed.first().expect("host should remain listed");
        assert!(stored
            .credentials
            .iter()
            .all(|c| c.id != bob_id && c.username != "bob"));

        // 3. default 指向不属于本主机的凭证 → 返回值与库里都置 None
        let dangling = save_zk_host(
            &pool,
            ZkHostRecord {
                default_credential_id: Some(999_999),
                ..updated.clone()
            },
        )
        .expect("save_zk_host with dangling default should succeed");
        assert_eq!(dangling.default_credential_id, None);
        assert_eq!(
            list_zk_hosts(&pool)
                .expect("list after dangling default should succeed")
                .first()
                .map(|h| h.default_credential_id),
            Some(None)
        );

        // 4. 同主机 username 重复 → save 返回 Err 且文案含「凭证用户名重复」
        let dup_err = save_zk_host(
            &pool,
            ZkHostRecord {
                credentials: vec![
                    ZkCredentialRecord {
                        id: 0,
                        username: "dup-user".to_string(),
                        password: "p1".to_string(),
                        created_at: 0,
                        updated_at: 0,
                    },
                    ZkCredentialRecord {
                        id: 0,
                        username: "dup-user".to_string(),
                        password: "p2".to_string(),
                        created_at: 0,
                        updated_at: 0,
                    },
                ],
                ..dangling.clone()
            },
        )
        .expect_err("duplicate credential username should fail");
        assert!(
            dup_err.contains("凭证用户名重复"),
            "unexpected error: {dup_err}"
        );

        // name/endpoints trim 后为空 → 错误
        let empty_name_err = save_zk_host(
            &pool,
            ZkHostRecord {
                name: "   ".to_string(),
                ..dangling.clone()
            },
        )
        .expect_err("empty zk host name should fail");
        assert!(empty_name_err.contains("ZK 主机名称不能为空"));
        let empty_endpoints_err = save_zk_host(
            &pool,
            ZkHostRecord {
                endpoints: "  ".to_string(),
                ..dangling.clone()
            },
        )
        .expect_err("empty zk endpoints should fail");
        assert!(empty_endpoints_err.contains("ZK 地址不能为空"));

        // 5. 删除 → list 为空；重复删除幂等不报错
        delete_zk_host(&pool, dangling.id).expect("delete_zk_host should succeed");
        delete_zk_host(&pool, dangling.id).expect("delete_zk_host should be idempotent");
        assert!(
            list_zk_hosts(&pool)
                .expect("list after delete should succeed")
                .is_empty()
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn grpc_tls_profile_roundtrip() {
        let temp_dir =
            std::env::temp_dir().join(format!("veloterm-grpctls-test-{}", Uuid::new_v4()));
        let db_path = temp_dir.join("veloterm.db");
        let pool =
            init_db_at_path(&db_path, "grpc-tls-key").expect("init_db_at_path should succeed");

        // 1. 新增 + list 比对 + 按 id 取
        let created = save_grpc_tls_profile(
            &pool,
            GrpcTlsProfileRecord {
                id: 0,
                server_name: "livebos-server".to_string(),
                ca_pem: Some("-----BEGIN CERTIFICATE-----\nca\n".to_string()),
                client_cert_pem: Some("-----BEGIN CERTIFICATE-----\ncert\n".to_string()),
                client_key_pem: Some("-----BEGIN PRIVATE KEY-----\nkey\n".to_string()),
                created_at: 0,
                updated_at: 0,
            },
        )
        .expect("save_grpc_tls_profile insert should succeed");
        assert!(created.id > 0);
        let listed = list_grpc_tls_profiles(&pool).expect("list should succeed");
        assert_eq!(listed, vec![created.clone()]);
        let fetched =
            grpc_tls_profile_by_id(&pool, created.id).expect("fetch by id should succeed");
        assert_eq!(fetched, created);

        // 2. 域名唯一冲突（新增同名 / 更新撞名）与错误文案
        let dup_err = save_grpc_tls_profile(
            &pool,
            GrpcTlsProfileRecord {
                id: 0,
                server_name: "livebos-server".to_string(),
                ca_pem: None,
                client_cert_pem: None,
                client_key_pem: None,
                created_at: 0,
                updated_at: 0,
            },
        )
        .expect_err("duplicate server_name should fail");
        assert!(dup_err.contains("校验域名已存在"), "got: {dup_err}");

        // 3. 更新：改 CA、去客户端证书对；list 比对
        let updated = save_grpc_tls_profile(
            &pool,
            GrpcTlsProfileRecord {
                ca_pem: None,
                client_cert_pem: None,
                client_key_pem: None,
                ..created.clone()
            },
        )
        .expect("save_grpc_tls_profile update should succeed");
        let listed = list_grpc_tls_profiles(&pool).expect("list after update should succeed");
        assert_eq!(listed, vec![updated.clone()]);

        // 4. 证书/私钥只给其一 → 拒绝
        let unpaired_err = save_grpc_tls_profile(
            &pool,
            GrpcTlsProfileRecord {
                id: 0,
                server_name: "another.example.com".to_string(),
                client_cert_pem: Some("cert-only".to_string()),
                ..Default::default()
            },
        )
        .expect_err("unpaired client cert/key should fail");
        assert!(unpaired_err.contains("成对"), "got: {unpaired_err}");

        // 5. 删除幂等；按 id 取已删配置报不存在
        delete_grpc_tls_profile(&pool, updated.id).expect("delete should succeed");
        delete_grpc_tls_profile(&pool, updated.id).expect("delete should be idempotent");
        let gone_err =
            grpc_tls_profile_by_id(&pool, updated.id).expect_err("deleted profile should 404");
        assert!(gone_err.contains("TLS 配置不存在"), "got: {gone_err}");

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
