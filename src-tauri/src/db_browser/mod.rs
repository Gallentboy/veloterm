//! 数据库工作台（侧栏「数据库」菜单）：MySQL / PostgreSQL / Oracle /
//! OceanBase / SQLite 连接管理、对象树浏览与流式 SQL 执行。会话模型与
//! ZkState 同型（tauri manage 注入 State + 会话表 + 前端按 sessionId
//! 交互）；引擎层见 engines.rs（Oracle 走 vendor rust-oracle——官方
//! ODPI-C 绑定、同步 API 经 with_oracle 的 spawn_blocking 包裹，运行期
//! 需 Oracle Instant Client，支持 11.2–23ai；PL/SQL 匿名块经 sqlsplit
//! 整段化后 execute + implicit_result，database 字段在 Oracle 语义 =
//! 服务名，「切库」= CURRENT_SCHEMA；OceanBase 双模式——MySQL 模式租户
//! 复用 mysql_async 臂、Oracle 模式租户走 vendor obclnt-sys/libobclient
//!（obclient.rs 安全包装 + with_obclient 阻塞包裹，database 字段同
//! Oracle 语义 = 初始 schema）；SQLite 是本地文件库——`password` 字段
//! 语义 = 加密口令、`db_path` = 文件路径、`encryption_mode` =
//! none/sqlcipher4/sqlcipher3/sqlcipher2/sqlcipher-raw，取消经 progress
//! handler（引擎侧绑定会话的 cancelled 标志））。
//!
//! 现范围：直连（TLS 四档；Oracle 侧 TlsMode 两档、见 engines.rs 头注）、
//! 对象树三层懒加载、多语句分割执行（Channel 流式回批）、只读拦截、取消
//! 与行数上限。SSH 隧道 / 中继穿透（P1）未接入：ssh_host_id 落库但连接时
//! 报错提示。

pub mod engines;
pub mod obclient;
pub mod sqlsplit;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

use crate::db::{self, DbConnectionRecord, DbPool};
use engines::{CatalogNode, CatalogParent, DbEngine};

// ── 会话状态（ZkState 同型）────────────────────────────────────────────────

/// 连接测试结果（表单「测试连接」按钮：未保存的草稿记录也可测）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbConnectionTestResult {
    pub ok: bool,
    pub elapsed_ms: u64,
    pub server_version: Option<String>,
    pub message: Option<String>,
}

/// 会话元信息（db_connect 返回、前端 tab 展示用）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbSessionMeta {
    pub session_id: String,
    pub connection_id: usize,
    pub name: String,
    pub engine: String,
    /// OceanBase 专有：租户模式（Some("mysql") / Some("oracle")，其他引擎
    /// 恒 None；前端据此展示模式徽标，sqlsplit 方言键映射也消费它）
    pub mode: Option<String>,
    /// 达梦的客户端侧兼容模式（'oracle' | 'mysql'）；其他引擎恒 null。
    /// 前端据此选方言语法（ROWNUM vs LIMIT、FROM DUAL 等），db_query 的
    /// 方言键映射也消费它（与 mode 的分工：mode 属 OceanBase，此处属达梦）
    pub compatible_mode: Option<String>,
    pub database: Option<String>,
    pub server_version: String,
    pub readonly: bool,
    pub env_level: String,
}

/// 一个 DB 会话 = 引擎连接（执行互斥）+ 取消信号组合 + 元信息（切库时更新）。
pub struct DbSession {
    /// RwLock：读多写少，切换数据库（db_use_database）时写更新
    pub meta: std::sync::RwLock<DbSessionMeta>,
    /// tokio Mutex：同一连接同时只跑一条语句（单连接工作台模型）
    pub engine: tokio::sync::Mutex<DbEngine>,
    pub cancel: Arc<tokio::sync::Notify>,
    pub cancelled: Arc<AtomicBool>,
}

/// DB 会话表（tauri manage 注入）。
pub struct DbState {
    next_id: AtomicUsize,
    sessions: Mutex<HashMap<String, Arc<DbSession>>>,
}

impl Default for DbState {
    fn default() -> Self {
        Self::new()
    }
}

impl DbState {
    pub fn new() -> Self {
        Self {
            next_id: AtomicUsize::new(1),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn alloc_session_id(&self) -> String {
        let n = self.next_id.fetch_add(1, Ordering::Relaxed);
        format!("db-{}-{n}", std::process::id())
    }

    fn get(&self, session_id: &str) -> Result<Arc<DbSession>, String> {
        self.sessions
            .lock()
            .map_err(|_| "DB 会话表锁中毒".to_string())?
            .get(session_id)
            .cloned()
            .ok_or_else(|| format!("DB 会话 {session_id} 不存在（已断开？）"))
    }

    fn remove(&self, session_id: &str) -> Option<Arc<DbSession>> {
        self.sessions.lock().ok()?.remove(session_id)
    }
}

fn pool() -> Result<DbPool, String> {
    db::global_pool().ok_or_else(|| "数据库未初始化".to_string())
}

// ── 连接库 CRUD（zk_hosts.rs 薄封装同型）──────────────────────────────────

#[tauri::command]
pub fn db_connection_list() -> Result<Vec<DbConnectionRecord>, String> {
    db::list_db_connections(&pool()?)
}

#[tauri::command]
pub fn db_connection_save(record: DbConnectionRecord) -> Result<DbConnectionRecord, String> {
    let record = DbConnectionRecord {
        name: record.name.trim().to_string(),
        host: record.host.trim().to_string(),
        ..record
    };
    db::save_db_connection(&pool()?, record)
}

#[tauri::command]
pub fn db_connection_delete(id: usize) -> Result<(), String> {
    db::delete_db_connection(&pool()?, id)
}

/// 编辑态表单密码留空（=保留旧密码语义）时从库里补上已存密码。
///
/// SQLite 例外：加密方式为 none 时不补——明文库不该带口令打开（SQLCipher
/// 会对明文文件报 NOTADB「口令错误或不是加密数据库」）。用户把加密连接
/// 改成不加密时，前端传空串正是要清掉残留口令，这里必须尊重该意图。
fn fill_password_from_store(
    record: &mut DbConnectionRecord,
    connection_id: Option<usize>,
) -> Result<(), String> {
    if record.engine == "sqlite" && record.encryption_mode == "none" {
        return Ok(());
    }
    if record.password.as_deref().map(str::is_empty).unwrap_or(true)
        && let Some(id) = connection_id.filter(|v| *v > 0)
        && let Ok(Some(Some(saved))) =
            db::get_db_connection(&pool()?, id).map(|r| r.map(|r| r.password))
    {
        record.password = Some(saved);
    }
    Ok(())
}

/// 连接测试：建连 → 取版本 → 立即断开。失败也返回 200（ok=false 携带原因）。
/// 编辑态密码留空时按 connection_id 从库补（测试结果永远反映真实保存状态，
/// 不会再出现「库里明明有密码、测试却报 using password: NO」的误导）。
#[tauri::command]
pub async fn db_connection_test(
    mut record: DbConnectionRecord,
    connection_id: Option<usize>,
) -> Result<DbConnectionTestResult, String> {
    fill_password_from_store(&mut record, connection_id)?;
    let start = Instant::now();
    match DbEngine::connect(&record).await {
        Ok((mut engine, version)) => {
            engine.close().await;
            Ok(DbConnectionTestResult {
                ok: true,
                elapsed_ms: start.elapsed().as_millis() as u64,
                server_version: Some(version),
                message: None,
            })
        }
        Err(message) => Ok(DbConnectionTestResult {
            ok: false,
            elapsed_ms: start.elapsed().as_millis() as u64,
            server_version: None,
            message: Some(message),
        }),
    }
}

// ── 会话生命周期 ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn db_connect(
    state: State<'_, DbState>,
    connection_id: usize,
) -> Result<DbSessionMeta, String> {
    let record = db::get_db_connection(&pool()?, connection_id)?
        .ok_or_else(|| format!("数据库连接不存在：id={connection_id}"))?;
    if let Some(ssh_host_id) = record.ssh_host_id {
        return Err(format!(
            "SSH 隧道将在后续版本接入（当前配置指向主机 id={ssh_host_id}），请暂时改为直连"
        ));
    }
    let (mut engine, server_version) = DbEngine::connect(&record).await?;
    // SQLite 的取消走 progress handler：引擎侧需要会话的取消标志句柄，
    // 故先建标志、绑进引擎，再随会话一起持有（其它引擎为无操作）
    let cancelled = Arc::new(AtomicBool::new(false));
    engine.bind_cancel_flag(cancelled.clone());
    let session_id = state.alloc_session_id();
    // SQLite 没有连接级库概念：元信息固定展示 main（对象树顶层 = 已附加库）
    let database = if record.engine == "sqlite" {
        Some("main".to_string())
    } else {
        record.database.clone()
    };
    // 达梦的兼容模式在结构体构造前算好（name/env_level 等 move 字段后
    // record 不可再借用）；db_query 的方言键映射消费它
    let dameng_dialect = record.dameng_dialect();
    let meta = DbSessionMeta {
        session_id: session_id.clone(),
        connection_id: record.id,
        name: record.name,
        engine: record.engine.clone(),
        mode: record.mode.clone(),
        compatible_mode: dameng_dialect.map(str::to_string),
        database,
        server_version,
        readonly: record.readonly,
        env_level: record.env_level,
    };
    let session = Arc::new(DbSession {
        meta: std::sync::RwLock::new(meta.clone()),
        engine: tokio::sync::Mutex::new(engine),
        cancel: Arc::new(tokio::sync::Notify::new()),
        cancelled,
    });
    state
        .sessions
        .lock()
        .map_err(|_| "DB 会话表锁中毒".to_string())?
        .insert(session_id, session);
    Ok(meta)
}

#[tauri::command]
pub async fn db_disconnect(state: State<'_, DbState>, session_id: String) -> Result<(), String> {
    if let Some(session) = state.remove(&session_id) {
        // 先打取消信号（可能有正在执行的查询在等待流），再拿执行锁优雅关闭
        session.cancel.notify_waiters();
        let mut engine = session.engine.lock().await;
        engine.close().await;
    }
    Ok(())
}

// ── 对象树 ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn db_catalog(
    state: State<'_, DbState>,
    session_id: String,
    parent: Option<CatalogParentArg>,
) -> Result<Vec<CatalogNode>, String> {
    let session = state.get(&session_id)?;
    let parent = match parent {
        None => CatalogParent::Root,
        Some(CatalogParentArg::Schema { schema }) => CatalogParent::Schema { schema },
        Some(CatalogParentArg::Group { schema, group }) => {
            let group = engines::ObjectGroup::parse(&group)
                .ok_or_else(|| format!("无效的对象分组：{group}"))?;
            CatalogParent::Group { schema, group }
        }
        Some(CatalogParentArg::Table { schema, table }) => CatalogParent::Table { schema, table },
    };
    let mut engine = session.engine.lock().await;
    engine.catalog(&parent).await
}

/// 目录父层（前端参数形态；None = 根层）。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CatalogParentArg {
    Schema { schema: String },
    /// group: table | view | function | procedure | sequence
    Group { schema: String, group: String },
    Table { schema: String, table: String },
}

// ── 查询执行 ───────────────────────────────────────────────────────────────

/// 流式查询事件（Tauri Channel 逐批发给前端）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum QueryEvent {
    Started { statement_count: usize },
    StatementStart { index: usize, sql: String },
    Columns { index: usize, columns: Vec<String> },
    Rows { index: usize, rows: Vec<Vec<String>>, total: usize },
    StatementDone {
        index: usize,
        affected: Option<u64>,
        rows_read: usize,
        truncated: bool,
        cancelled: bool,
        elapsed_ms: u64,
    },
    StatementError { index: usize, message: String },
    Done { elapsed_ms: u64, cancelled: bool },
}

/// 执行 SQL（可多语句；sqlparser 分割，失败降级分号分割）。
/// 只读会话在执行前做白名单拦截；流式按批回传，支持取消（db_query_cancel）
/// 与行数上限（默认 5000，上限 50000）。整体超时 = 每条语句的剩余预算。
#[tauri::command]
pub async fn db_query(
    state: State<'_, DbState>,
    session_id: String,
    sql: String,
    max_rows: Option<usize>,
    timeout_ms: Option<u64>,
    on_event: Channel<QueryEvent>,
) -> Result<(), String> {
    let session = state.get(&session_id)?;
    let (readonly, dialect_key) = {
        let meta = session
            .meta
            .read()
            .map_err(|_| "DB 会话元信息锁中毒".to_string())?;
        // 双模式引擎的方言/只读白名单映射（合成键与真实引擎同名或 dameng:xxx）：
        // - OceanBase：Oracle 模式继承 oracle 方言（PL/SQL 块整段化、只读
        //   白名单同源），MySQL 模式继承 mysql 方言；
        // - 达梦：compatible_mode 展开为 dameng:oracle / dameng:mysql（sqlsplit
        //   侧的两个方言分支）；空值归 oracle（达梦默认部署多为 Oracle 兼容）
        let key = match (meta.engine.as_str(), meta.mode.as_deref()) {
            ("oceanbase", Some("oracle")) => "oracle".to_string(),
            ("oceanbase", _) => "mysql".to_string(),
            ("dameng", _) => format!(
                "dameng:{}",
                meta.compatible_mode.as_deref().unwrap_or("oracle")
            ),
            (other, _) => other.to_string(),
        };
        (meta.readonly, key)
    };
    let statements = sqlsplit::split_statements(&dialect_key, &sql);
    if statements.is_empty() {
        return Err("SQL 语句为空".to_string());
    }
    if readonly {
        for stmt in &statements {
            if !sqlsplit::statement_is_readonly(&dialect_key, stmt) {
                let head: String = stmt.sql.chars().take(60).collect();
                return Err(format!(
                    "会话为只读模式，禁止执行非查询语句：{head}{}",
                    if stmt.sql.chars().count() > 60 { "…" } else { "" }
                ));
            }
        }
    }

    let row_cap = max_rows.unwrap_or(5_000).clamp(1, 50_000);
    let total_budget = Duration::from_millis(timeout_ms.unwrap_or(60_000).clamp(1_000, 3_600_000));
    let started = Instant::now();

    let _ = on_event.send(QueryEvent::Started {
        statement_count: statements.len(),
    });

    let mut engine = session.engine.lock().await;
    let mut any_cancelled = false;

    for (index, stmt) in statements.iter().enumerate() {
        if session.cancelled.load(Ordering::Relaxed) {
            any_cancelled = true;
            break;
        }
        // 复位取消信号：上一条语句的正常结束不应吃掉本条的等待
        session.cancelled.store(false, Ordering::Relaxed);
        let _ = on_event.send(QueryEvent::StatementStart {
            index,
            sql: stmt.sql.clone(),
        });

        let remaining = total_budget.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            let _ = on_event.send(QueryEvent::StatementError {
                index,
                message: "执行超时（整体时间预算用尽）".to_string(),
            });
            any_cancelled = true;
            break;
        }

        let stmt_start = Instant::now();

        // 回调闭包须 Send + 'static（Oracle 同步引擎把回调移入 spawn_blocking）：
        // 各自持有 Channel 克隆与原子行计数，不借用外层局部
        let cols_event = on_event.clone();
        let rows_event = on_event.clone();
        let rows_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let exec = engine.exec_streaming(
            &stmt.sql,
            row_cap,
            &session.cancel,
            move |cols| {
                let _ = cols_event.send(QueryEvent::Columns {
                    index,
                    columns: cols.to_vec(),
                });
            },
            move |rows| {
                let n = rows_counter.fetch_add(rows.len(), Ordering::Relaxed) + rows.len();
                let _ = rows_event.send(QueryEvent::Rows {
                    index,
                    rows,
                    total: n,
                });
            },
        );

        match tokio::time::timeout(remaining, exec).await {
            Ok(Ok(summary)) => {
                if summary.cancelled {
                    any_cancelled = true;
                }
                let _ = on_event.send(QueryEvent::StatementDone {
                    index,
                    affected: summary.affected,
                    rows_read: summary.rows_read,
                    truncated: summary.truncated,
                    cancelled: summary.cancelled,
                    elapsed_ms: stmt_start.elapsed().as_millis() as u64,
                });
                if summary.cancelled {
                    break;
                }
            }
            Ok(Err(message)) => {
                let _ = on_event.send(QueryEvent::StatementError { index, message });
                // 单条语句失败继续执行后续语句（客户端常见行为）；
                // 连接级故障会在下一条语句再次暴露
            }
            Err(_) => {
                let _ = on_event.send(QueryEvent::StatementError {
                    index,
                    message: format!(
                        "执行超时（>{}ms），已中止后续语句",
                        total_budget.as_millis()
                    ),
                });
                any_cancelled = true;
                break;
            }
        }
    }

    let _ = on_event.send(QueryEvent::Done {
        elapsed_ms: started.elapsed().as_millis() as u64,
        cancelled: any_cancelled,
    });
    Ok(())
}

/// 取消当前正在执行的查询（流式循环 select 到信号即止）。
#[tauri::command]
pub async fn db_query_cancel(state: State<'_, DbState>, session_id: String) -> Result<(), String> {
    let session = state.get(&session_id)?;
    session.cancelled.store(true, Ordering::Relaxed);
    session.cancel.notify_waiters();
    Ok(())
}

/// 拉取函数/过程的完整定义与调用骨架（对象树双击消费）。
#[tauri::command]
pub async fn db_object_definition(
    state: State<'_, DbState>,
    session_id: String,
    schema: String,
    name: String,
    kind: String,
) -> Result<engines::ObjectDefinition, String> {
    let session = state.get(&session_id)?;
    let mut engine = session.engine.lock().await;
    engine.object_definition(&schema, &name, &kind).await
}

/// 表/视图详情（DDL + 列结构 + 索引，树右键「表详情」；kind: table|view）。
#[tauri::command]
pub async fn db_table_detail(
    state: State<'_, DbState>,
    session_id: String,
    schema: String,
    table: String,
    kind: String,
) -> Result<engines::TableDetail, String> {
    let session = state.get(&session_id)?;
    let mut engine = session.engine.lock().await;
    engine.table_detail(&schema, &table, &kind).await
}

// ── 当前数据库切换（工具栏下拉）────────────────────────────────────────

/// 可切换的数据库清单（MySQL=全部 schema；PG=服务器上可连的库）。
#[tauri::command]
pub async fn db_list_databases(
    state: State<'_, DbState>,
    session_id: String,
) -> Result<Vec<String>, String> {
    let session = state.get(&session_id)?;
    let mut engine = session.engine.lock().await;
    engine.list_databases().await
}

/// 用表单草稿参数（编辑态密码留空自动从库补）临时建连拉取可选数据库清单，
/// 拉完即断——供连接表单的「数据库」下拉选择（也可手输）。SQLite 返回
/// `PRAGMA database_list` 的附加库名（前端对 SQLite 隐藏该选择器，保留
/// 命令兼容）。
#[tauri::command]
pub async fn db_list_databases_for_record(
    mut record: DbConnectionRecord,
    connection_id: Option<usize>,
) -> Result<Vec<String>, String> {
    fill_password_from_store(&mut record, connection_id)?;
    let (mut engine, _version) = DbEngine::connect(&record).await?;
    let databases = engine.list_databases().await;
    engine.close().await;
    databases
}

/// 切换当前数据库：MySQL 会话内 USE（不重建连接）；PG 的 database 是连接级
/// 属性——按原连接记录重连（同主机/凭证/SSL，替换库名）后换入会话引擎。
/// 返回更新后的会话元信息；前端据此刻刷新对象树。
#[tauri::command]
pub async fn db_use_database(
    state: State<'_, DbState>,
    session_id: String,
    database: String,
) -> Result<DbSessionMeta, String> {
    let session = state.get(&session_id)?;
    let db_name = database.trim().to_string();
    if db_name.is_empty() {
        return Err("数据库名不能为空".to_string());
    }
    let mut engine = session.engine.lock().await;
    match &*engine {
        DbEngine::MySql(_) => {
            engine.select_database(&db_name).await?;
        }
        DbEngine::Postgres(_) => {
            let mut record = db::get_db_connection(&pool()?, session
                .meta
                .read()
                .map_err(|_| "DB 会话元信息锁中毒".to_string())?
                .connection_id)?
                .ok_or_else(|| "连接记录不存在（已被删除？）".to_string())?;
            record.database = Some(db_name.clone());
            let (new_engine, _version) = DbEngine::connect(&record).await?;
            let mut old = std::mem::replace(&mut *engine, new_engine);
            old.close().await;
        }
        DbEngine::Oracle(_) | DbEngine::Dameng(_) => {
            // Oracle / 达梦切 schema = 会话级 SET SCHEMA（只改名字解析，不重连、
            // 不动连接记录）；不走下方 meta 重写分支的 early-return，统一落到
            // 尾部的 database 更新（原 Oracle 臂在此 early-return，导致切换后
            // 元信息永远停在初始 schema——顺带修正）
            engine.select_database(&db_name).await?;
        }
        DbEngine::Sqlite(_) => {
            // SQLite 无连接级切库：对象树顶层已是 database_list 的附加库，
            // 这里只更新元信息供工具栏展示
        }
        DbEngine::ObOracle(_) => {
            // OB Oracle 同 Oracle 臂：切换 = ALTER SESSION SET CURRENT_SCHEMA
            //（不重连、不动连接记录）；元信息沿用现状不重写（meta 更新同
            // Oracle 臂惯例）
            engine.select_database(&db_name).await?;
            let meta = session
                .meta
                .read()
                .map_err(|_| "DB 会话元信息锁中毒".to_string())?;
            return Ok(meta.clone());
        }
        DbEngine::Closed => return Err("连接已关闭".to_string()),
    }
    // 更新并返回元信息（PG 重连路径里 server_version 可能变化，MySQL 保持原值）
    let mut meta = session
        .meta
        .write()
        .map_err(|_| "DB 会话元信息锁中毒".to_string())?;
    meta.database = Some(db_name);
    Ok(meta.clone())
}

// ── Oracle 客户端库目录（运行期 dlopen 的动态绑定形态）─────────────────────
//
// ODPI-C 经 dlopen/LoadLibrary 运行期加载 Oracle 客户端：进程启动零依赖、
// 缺库只在连接时报错（DPI-1047），绝不拖死应用。用户可在数据库页设置
// Instant Client 的目录（rust-oracle 的 InitParams::oracle_client_lib_dir，
// 进程级一次初始化——已初始化后改目录需重启应用生效）。

const ORACLE_CLIENT_DIR_KV: &str = "oracle_client_lib_dir";

/// 读 Oracle 客户端库目录设置（None = 未设置，走系统搜索路径）
#[tauri::command]
pub fn oracle_client_dir_get() -> Result<Option<String>, String> {
    db::get_kv(&pool()?, ORACLE_CLIENT_DIR_KV)
        .map(|dir| dir.filter(|s| !s.is_empty()))
}

/// 写/清除 Oracle 客户端库目录（None = 清除，回到系统搜索路径）。
/// 走 db::set_kv（app_kv.updated_at NOT NULL，裸 INSERT 漏该列会约束失败）
#[tauri::command]
pub fn oracle_client_dir_set(dir: Option<String>) -> Result<(), String> {
    let pool = pool()?;
    match dir.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(d) => db::set_kv(&pool, ORACLE_CLIENT_DIR_KV, d.trim()),
        None => db::delete_kv(&pool, ORACLE_CLIENT_DIR_KV),
    }
}

/// 目录选择对话框（rfd）：返回所选目录；取消返回 None
#[tauri::command]
pub async fn oracle_client_dir_pick() -> Result<Option<String>, String> {
    let picked = rfd::AsyncFileDialog::new()
        .set_title("选择 Oracle Instant Client 目录")
        .pick_folder()
        .await;
    Ok(picked.map(|h| h.path().to_string_lossy().into_owned()))
}

// ── SQLite 数据库文件选择（rfd）───────────────────────────────────────────

/// 选择本地 SQLite 数据库文件；取消返回 None
#[tauri::command]
pub async fn sqlite_file_pick() -> Result<Option<String>, String> {
    let picked = rfd::AsyncFileDialog::new()
        .set_title("选择 SQLite 数据库文件")
        .add_filter("SQLite 数据库", &["db", "sqlite", "sqlite3", "db3", "s3db"])
        .add_filter("所有文件", &["*"])
        .pick_file()
        .await;
    Ok(picked.map(|h| h.path().to_string_lossy().into_owned()))
}
