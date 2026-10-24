//! 数据库工作台（侧栏「数据库」菜单）：MySQL / PostgreSQL / Oracle /
//! OceanBase / SQLite 连接管理、对象树浏览与流式 SQL 执行。会话模型与
//! ZkState 同型（tauri manage 注入 State + 会话表 + 前端按 sessionId
//! 交互）；引擎层见 engines.rs（Oracle 走 vendor oracle_async——纯 Rust
//! 原生异步驱动，零 C 依赖无需 Oracle Instant Client，Tokio 全链路异步；
//! PL/SQL 经 sqlsplit 整段化，隐式结果集（Msg 27）由 QueryResult 自动接续，
//! database 字段在 Oracle 语义 =
//! 服务名，「切库」= CURRENT_SCHEMA；OceanBase 双模式——两模式都走纯 Rust
//! 分叉驱动 vendor/mysql-async-ob（MySQL 模式复用 mysql_async 臂，Oracle
//! 模式经其 ob_protocol20 路径，database 字段同 Oracle 语义 = 初始
//! schema）；SQLite 是本地文件库——`password` 字段
//! 语义 = 加密口令、`db_path` = 文件路径、`encryption_mode` =
//! none/sqlcipher4/sqlcipher3/sqlcipher2/sqlcipher-raw，取消经 progress
//! handler（引擎侧绑定会话的 cancelled 标志））。
//!
//! 现范围：直连（TLS 四档；Oracle 侧 TlsMode 两档、见 engines.rs 头注）、
//! 对象树三层懒加载、多语句分割执行（Channel 流式回批）、只读拦截、取消
//! 与行数上限。SSH 隧道 / 中继穿透（P1）未接入：ssh_host_id 落库但连接时
//! 报错提示。

pub mod engines;
pub mod explain;
/// Oracle 19c 错误消息全量中文译文（构建期嵌入、零堆分配稀疏查找）。
/// 数据是机器辅助翻译、非官方译文；查不到时调用方回落 engines 里的
/// `oracle_ora_hint` 精简摘要表。
pub mod oracle_messages;
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
    // Redis 拓扑复用 mode 字段携带（cluster/sentinel；单机 None）：前端据此
    // 禁用集群的切库下拉；OB 的租户模式语义不受影响（engine 已区分两者）
    let mode = if record.engine == "redis" {
        engines::redis_topology_of(record.extra_options.as_deref()).map(str::to_string)
    } else {
        record.mode.clone()
    };
    let meta = DbSessionMeta {
        session_id: session_id.clone(),
        connection_id: record.id,
        name: record.name,
        engine: record.engine.clone(),
        mode,
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
    let total_budget =
        Duration::from_millis(timeout_ms.unwrap_or(60_000).clamp(1_000, 3_600_000));
    let statements: Vec<(String, bool)> = statements
        .into_iter()
        .map(|s| (s.sql, true))
        .collect();
    run_statements_streaming(&session, &on_event, &statements, row_cap, total_budget).await
}

/// db_query / db_explain 共用的流式执行循环：逐条执行 + Channel 事件 +
/// 取消 / 超时 / 行数上限。statements 是（SQL, 是否占结果位次）二元组：
/// emit=false 的辅助语句（Oracle 执行计划的 EXPLAIN PLAN 落表动词）不向前端
/// 发 StatementStart/Done/Columns/Rows，失败则整体失败（如 PLAN_TABLE 缺失
/// 要直接报诊断错误而不是混进结果面板）；emit=true 的单条失败仍继续后续
/// 语句（客户端常见行为）。结果位次 index 只在 emit 语句间递增。
async fn run_statements_streaming(
    session: &Arc<DbSession>,
    on_event: &Channel<QueryEvent>,
    statements: &[(String, bool)],
    row_cap: usize,
    total_budget: Duration,
) -> Result<(), String> {
    let emit_count = statements.iter().filter(|(_, emit)| *emit).count();
    let started = Instant::now();

    let _ = on_event.send(QueryEvent::Started {
        statement_count: emit_count,
    });

    let mut engine = session.engine.lock().await;
    let mut any_cancelled = false;
    let mut index = 0usize;

    for (sql, emit) in statements {
        if session.cancelled.load(Ordering::Relaxed) {
            any_cancelled = true;
            break;
        }
        // 复位取消信号：上一条语句的正常结束不应吃掉本条的等待
        session.cancelled.store(false, Ordering::Relaxed);
        if *emit {
            let _ = on_event.send(QueryEvent::StatementStart {
                index,
                sql: sql.clone(),
            });
        }

        let remaining = total_budget.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            if *emit {
                let _ = on_event.send(QueryEvent::StatementError {
                    index,
                    message: "执行超时（整体时间预算用尽）".to_string(),
                });
            }
            any_cancelled = true;
            break;
        }

        let stmt_start = Instant::now();

        // 回调闭包须 Send + 'static（Oracle 同步引擎把回调移入 spawn_blocking）：
        // 各自持有 Channel 克隆与原子行计数，不借用外层局部；emit=false 的
        // 辅助语句在回调内静默丢弃（不占结果位次）
        let emit_flag = *emit;
        let cols_event = on_event.clone();
        let rows_event = on_event.clone();
        let rows_counter = Arc::new(AtomicUsize::new(0));

        let exec = engine.exec_streaming(
            sql,
            row_cap,
            &session.cancel,
            move |cols| {
                if emit_flag {
                    let _ = cols_event.send(QueryEvent::Columns {
                        index,
                        columns: cols.to_vec(),
                    });
                }
            },
            move |rows| {
                if emit_flag {
                    let n = rows_counter.fetch_add(rows.len(), Ordering::Relaxed) + rows.len();
                    let _ = rows_event.send(QueryEvent::Rows {
                        index,
                        rows,
                        total: n,
                    });
                }
            },
        );

        match tokio::time::timeout(remaining, exec).await {
            Ok(Ok(summary)) => {
                if summary.cancelled {
                    any_cancelled = true;
                }
                if *emit {
                    let _ = on_event.send(QueryEvent::StatementDone {
                        index,
                        affected: summary.affected,
                        rows_read: summary.rows_read,
                        truncated: summary.truncated,
                        cancelled: summary.cancelled,
                        elapsed_ms: stmt_start.elapsed().as_millis() as u64,
                    });
                }
                if summary.cancelled {
                    break;
                }
            }
            Ok(Err(message)) => {
                if *emit {
                    let _ = on_event.send(QueryEvent::StatementError { index, message });
                    // 单条语句失败继续执行后续语句（客户端常见行为）；
                    // 连接级故障会在下一条语句再次暴露
                } else {
                    // 辅助语句失败整体失败：诊断信息直接抛给调用方
                    return Err(message);
                }
            }
            Err(_) => {
                if *emit {
                    let _ = on_event.send(QueryEvent::StatementError {
                        index,
                        message: format!(
                            "执行超时（>{}ms），已中止后续语句",
                            total_budget.as_millis()
                        ),
                    });
                } else {
                    return Err(format!(
                        "执行计划辅助语句超时（>{}ms）",
                        total_budget.as_millis()
                    ));
                }
                any_cancelled = true;
                break;
            }
        }

        if *emit {
            index += 1;
        }
    }

    let _ = on_event.send(QueryEvent::Done {
        elapsed_ms: started.elapsed().as_millis() as u64,
        cancelled: any_cancelled,
    });
    Ok(())
}

/// db_explain 的方言装配器（纯函数，可单测；Oracle 的两段式装配需要连接
/// 探测 plan 表，见 db_explain 内联装配）。查询口径全部对齐 DBeaver 开源实现：
/// - MySQL（含 OB MySQL 模式）：EXPLAIN <stmt>——DBeaver MySQLPlanClassic 用
///   EXPLAIN EXTENDED，但 MySQL 8.0 已移除 EXTENDED/PARTITIONS 关键字（官方
///   what's new：其效果 8.0 恒开），裸 EXPLAIN 全版本可用，此为对 DBeaver 的
///   有据偏离；analyze（MySQL 8.0.18+，真实执行语句）为 EXPLAIN ANALYZE
/// - PostgreSQL：EXPLAIN <stmt>；analyze 为 EXPLAIN (ANALYZE, BUFFERS)
///   （DBeaver PostgreExecutionPlan 参数化形态的 TEXT 版）
/// - SQLite：EXPLAIN QUERY PLAN（只读白名单 AST/降级两路均含 EXPLAIN）
/// - 达梦：EXPLAIN <stmt>（直接返回计划结果集；analyze 概念不适用）
fn explain_statements(
    engine: &str,
    mode: Option<&str>,
    stmts: &[sqlsplit::SplitStatement],
    analyze: bool,
) -> Result<Vec<(String, bool)>, String> {
    let prefix: &str = match engine {
        "mysql" | "oceanbase" => {
            if mode == Some("oracle") {
                // OB Oracle 走 PLAN_TABLE 两段式（explain_core 的 oracle 分支），
                // 不进前缀式装配；走到这里说明调用方绕过了那条分流
                return Err(
                    "OceanBase Oracle 模式使用 PLAN_TABLE 两段式，不走前缀式 EXPLAIN"
                        .to_string(),
                );
            }
            if analyze { "EXPLAIN ANALYZE " } else { "EXPLAIN " }
        }
        "postgres" => {
            if analyze {
                "EXPLAIN (ANALYZE, BUFFERS) "
            } else {
                "EXPLAIN "
            }
        }
        "sqlite" => "EXPLAIN QUERY PLAN ",
        "dameng" => "EXPLAIN ",
        "redis" => return Err("Redis 没有执行计划概念".to_string()),
        // oracle 由 db_explain 单独装配（plan 表探测需要连接）
        _ => return Err(format!("引擎 {engine} 不支持执行计划")),
    };
    Ok(stmts
        .iter()
        .map(|s| (format!("{prefix}{}", s.sql), true))
        .collect())
}

/// 查看执行计划（SQL 编辑器右键菜单）：按引擎方言装配 EXPLAIN 语句执行后，
/// 在后端解析为统一的 PlanNode 节点表（id/parent_id 即边表）直接返回——
/// 前端按节点/边渲染图，不走流式结果通道（计划行是小数据量）。SQL 口径
/// 全部对齐 DBeaver 开源实现，调研与真机验证清单见
/// docs/sql-explain-plan-support.md。取消复用会话取消信号，单语句 60s 超时。
#[tauri::command]
pub async fn db_explain(
    state: State<'_, DbState>,
    session_id: String,
    sql: String,
    analyze: Option<bool>,
) -> Result<Vec<explain::ExplainPlan>, String> {
    let session = state.get(&session_id)?;
    let analyze = analyze.unwrap_or(false);
    let (engine_kind, mode, dialect_key) = {
        let meta = session
            .meta
            .read()
            .map_err(|_| "DB 会话元信息锁中毒".to_string())?;
        // 方言键映射与 db_query 同源（OB/达梦的双模式展开）
        let key = match (meta.engine.as_str(), meta.mode.as_deref()) {
            ("oceanbase", Some("oracle")) => "oracle".to_string(),
            ("oceanbase", _) => "mysql".to_string(),
            ("dameng", _) => format!(
                "dameng:{}",
                meta.compatible_mode.as_deref().unwrap_or("oracle")
            ),
            (other, _) => other.to_string(),
        };
        (meta.engine.clone(), meta.mode.clone(), key)
    };
    let user_stmts = sqlsplit::split_statements(&dialect_key, &sql);
    if user_stmts.is_empty() {
        return Err("SQL 语句为空".to_string());
    }
    // analyze 会真实执行语句（MySQL 8 EXPLAIN ANALYZE / PG ANALYZE）：
    // 只放行白名单判定为只读的语句（DBeaver 靠事后 rollback 防护，我们无
    // 事务管理，改为事前拒绝 DML）。OB Oracle 走 PLAN_TABLE 两段式、只解析
    // 不执行，故与原生 Oracle 一样不受此限制（否则会误拦 DML 的执行计划）
    let plan_only = engine_kind == "oceanbase" && mode.as_deref() == Some("oracle");
    if analyze
        && !plan_only
        && matches!(engine_kind.as_str(), "mysql" | "oceanbase" | "postgres")
    {
        for stmt in &user_stmts {
            if !sqlsplit::statement_is_readonly(&dialect_key, stmt) {
                let head: String = stmt.sql.chars().take(60).collect();
                return Err(format!(
                    "ANALYZE 模式会真实执行语句，仅允许只读语句：{head}{}",
                    if stmt.sql.chars().count() > 60 { "…" } else { "" }
                ));
            }
        }
    }

    let mut engine = session.engine.lock().await;
    explain_core(
        &mut engine,
        &session.cancel,
        &engine_kind,
        mode.as_deref(),
        &dialect_key,
        &user_stmts,
        analyze,
    )
    .await
}

/// 执行一条语句并收集（列头, 行）；取消信号透传，60s 超时。回调须
/// Send + 'static（Oracle 同步引擎把回调移入 spawn_blocking），收集器
/// 走 Arc<Mutex>（与 db_query 的行计数同型），执行完锁取
async fn exec_collect(
    engine: &mut DbEngine,
    cancel: &Arc<tokio::sync::Notify>,
    sql: &str,
) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
        let columns: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let rows: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let cols_sink = Arc::clone(&columns);
        let rows_sink = Arc::clone(&rows);
        let exec = engine.exec_streaming(
            sql,
            10_000,
            cancel,
            move |cols| {
                let mut sink = cols_sink.lock().unwrap_or_else(|p| p.into_inner());
                if sink.is_empty() {
                    *sink = cols.to_vec();
                }
            },
            move |batch| {
                rows_sink
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .extend(batch);
            },
        );
        let summary = tokio::time::timeout(Duration::from_secs(60), exec)
            .await
            .map_err(|_| "执行计划语句超时（>60s）".to_string())??;
        if summary.cancelled {
            return Err("已取消".to_string());
        }
        let columns = columns.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let rows = rows.lock().unwrap_or_else(|p| p.into_inner()).clone();
        Ok((columns, rows))
    }

/// db_explain 命令与真机测试 CLI（examples/db_engine_test）共用的执行核心：
/// 方言装配 + 逐条执行 + 结构化解析。引擎锁与取消信号由调用方持有
pub async fn explain_core(
    engine: &mut DbEngine,
    cancel: &Arc<tokio::sync::Notify>,
    engine_kind: &str,
    mode: Option<&str>,
    dialect_key: &str,
    user_stmts: &[sqlsplit::SplitStatement],
    analyze: bool,
) -> Result<Vec<explain::ExplainPlan>, String> {
    let mut plans: Vec<explain::ExplainPlan> = Vec::with_capacity(user_stmts.len());
    // Oracle 语义（原生 Oracle 与 OB Oracle 租户）走 PLAN_TABLE 两段式：先
    // `EXPLAIN PLAN SET STATEMENT_ID … INTO`，再读 PLAN_TABLE 原始行。OB
    // Oracle 租户虽然经 MySQL 线缆连接（mysql-async-ob），但 SQL 方言与数据
    // 字典都是 Oracle 那套，编排完全一致；差别只有 plan 表可能尚未创建时的
    // 提示文案（OB 没有 sqlplus 的 @?/rdbms/admin/utlxplan.sql）
    let oracle_semantics =
        engine_kind == "oracle" || (engine_kind == "oceanbase" && mode == Some("oracle"));
    if oracle_semantics {
        // plan 表探测（PLAN_TABLE/TOAD_PLAN_TABLE 候选 + ORA-00942 甄别，
        // 同 DBeaver getPlanTableName）；DBeaver 的防御性 DELETE 略去——
        // statement_id 每次调用唯一无残留可撞
        let mut plan_table: Option<&'static str> = None;
        let mut first_err = String::new();
        for name in ["PLAN_TABLE", "TOAD_PLAN_TABLE"] {
            match exec_collect(
                engine,
                cancel,
                &format!("SELECT 1 FROM {name} WHERE ROWNUM = 1"),
            )
            .await
            {
                Ok(_) => {
                    plan_table = Some(name);
                    break;
                }
                Err(e) => {
                    // ORA-00942（表或视图不存在）→ 换下一候选；其他错误
                    //（协议/网络/权限）原样上抛——别把驱动问题误报成缺表
                    if !e.contains("ORA-00942") && first_err.is_empty() {
                        first_err = e;
                    }
                }
            }
        }
        let Some(plan_table) = plan_table else {
            return Err(if first_err.is_empty() {
                if engine_kind == "oracle" {
                    "未找到执行计划表（PLAN_TABLE / TOAD_PLAN_TABLE 均不可访问）：请以当前用户执行 \
                     @?/rdbms/admin/utlxplan.sql 建表，或由 DBA 建全局 PLAN_TABLE\
                     （DBeaver getPlanTableName 同型探测）"
                        .to_string()
                } else {
                    // OB Oracle 租户没有 sqlplus 脚本可执行，只能提示按 Oracle
                    // PLAN_TABLE 结构自建（列集与原生一致：ID/PARENT_ID/OPERATION/
                    // OPTIONS/OBJECT_NAME/COST/CARDINALITY/BYTES…）
                    "OceanBase Oracle 租户未找到执行计划表（PLAN_TABLE / TOAD_PLAN_TABLE 均不可访问）：\
                     请先按 Oracle utlxplan.sql 的 PLAN_TABLE 结构在目标 schema 建表后重试"
                        .to_string()
                }
            } else {
                first_err
            });
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        for (i, stmt) in user_stmts.iter().enumerate() {
            let sid = format!("VELOTERM_{stamp:X}{i}");
            // 落表动词：失败整体上抛（PLAN_TABLE 类诊断直达用户）
            exec_collect(
                engine,
                cancel,
                &format!(
                    "EXPLAIN PLAN SET STATEMENT_ID = '{sid}' INTO {plan_table} FOR {}",
                    stmt.sql
                ),
            )
            .await?;
            let (cols, rows) = exec_collect(
                engine,
                cancel,
                &format!(
                    "SELECT * FROM {plan_table} WHERE STATEMENT_ID = '{sid}' ORDER BY ID"
                ),
            )
            .await?;
            plans.push(explain::parse_plan(&cols, &rows, &stmt.sql));
        }
    } else {
        // 前缀式引擎（MySQL/PG/SQLite/达梦/OB MySQL）：装配与用户语句一一对应
        let assembled = explain_statements(engine_kind, mode, user_stmts, analyze)?;
        for (stmt, (sql, _)) in user_stmts.iter().zip(assembled) {
            let (cols, rows) = exec_collect(engine, cancel, &sql).await?;
            plans.push(explain::parse_plan(&cols, &rows, &stmt.sql));
        }
    }
    Ok(plans)
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

// ── Redis 键管理与诊断 ─────────────────────────────────────────────────

/// Redis 键结构化读（键编辑对话框）：类型 + TTL + 成员明细；成员超上限置
/// truncated（前端禁整体覆写防丢数据）。db 是键所属逻辑库（树节点携带的
/// dbN），缺省用会话当前库。只读会话可用。
#[tauri::command]
pub async fn redis_read_key(
    state: State<'_, DbState>,
    session_id: String,
    key: String,
    db: Option<String>,
) -> Result<engines::RedisKeyData, String> {
    let session = state.get(&session_id)?;
    let mut engine = session.engine.lock().await;
    let DbEngine::Redis { conn, db: current, .. } = &mut *engine else {
        return Err("该命令仅适用于 Redis 会话".to_string());
    };
    let target = db
        .as_deref()
        .and_then(engines::parse_redis_db)
        .unwrap_or(*current);
    engines::redis_read_key_full(conn, *current, target, &key).await
}

/// Redis 键整体覆写（键编辑对话框保存）：DEL + 按类型写入的 MULTI/EXEC
/// 原子管道，keep_ttl 时恢复原 TTL；清空全部成员 = 删除该 key。
/// 只读会话拒绝。
#[tauri::command]
pub async fn redis_write_key(
    state: State<'_, DbState>,
    session_id: String,
    key: String,
    payload: engines::RedisKeyWrite,
    keep_ttl: bool,
    db: Option<String>,
) -> Result<i64, String> {
    let session = state.get(&session_id)?;
    if session
        .meta
        .read()
        .map_err(|_| "DB 会话元信息锁中毒".to_string())?
        .readonly
    {
        return Err("只读会话不允许写键（连接记录开启「只读」）".to_string());
    }
    let mut engine = session.engine.lock().await;
    let DbEngine::Redis { conn, db: current, .. } = &mut *engine else {
        return Err("该命令仅适用于 Redis 会话".to_string());
    };
    let target = db
        .as_deref()
        .and_then(engines::parse_redis_db)
        .unwrap_or(*current);
    engines::redis_write_key_full(conn, *current, target, &key, payload, keep_ttl).await
}

/// Redis 慢日志视图（SLOWLOG GET 的结构化解析）。只读会话可用。
#[tauri::command]
pub async fn redis_slowlog(
    state: State<'_, DbState>,
    session_id: String,
    count: Option<i64>,
) -> Result<engines::RedisTableData, String> {
    let session = state.get(&session_id)?;
    let count = count.unwrap_or(100).clamp(1, 1000) as usize;
    let mut engine = session.engine.lock().await;
    let DbEngine::Redis { conn, .. } = &mut *engine else {
        return Err("该命令仅适用于 Redis 会话".to_string());
    };
    engines::redis_slowlog_rows(conn, count).await
}

/// Redis 大 key 分析（SCAN 采样 + 批量 MEMORY USAGE，按内存倒序取前 N）。
/// 只读会话可用。
#[tauri::command]
pub async fn redis_bigkeys(
    state: State<'_, DbState>,
    session_id: String,
    sample: Option<i64>,
    top_n: Option<i64>,
) -> Result<engines::RedisBigkeysData, String> {
    let session = state.get(&session_id)?;
    let sample = sample.unwrap_or(1000).clamp(100, 50_000) as usize;
    let top_n = top_n.unwrap_or(50).clamp(10, 500) as usize;
    let mut engine = session.engine.lock().await;
    let DbEngine::Redis { conn, .. } = &mut *engine else {
        return Err("该命令仅适用于 Redis 会话".to_string());
    };
    engines::redis_bigkeys_rows(conn, sample, top_n).await
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
        DbEngine::Redis { .. } => {
            // Redis 切换 = SELECT n（连接级状态，不重连）；元信息里 database
            // 存的也是逻辑库名（"db3"），与对象树顶层节点名一致
            engine.select_database(&db_name).await?;
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

#[cfg(test)]
mod tests {
    use super::explain_statements;
    use crate::db_browser::sqlsplit::split_statements;

    /// 各引擎的 EXPLAIN 方言装配向量（口径对齐 DBeaver，MySQL 的
    /// EXTENDED 偏离与 OB Oracle 跳过见 explain_statements 注释）
    #[test]
    fn explain_assembly_per_engine() {
        let stmts = split_statements("mysql", "SELECT 1");
        let assembled = explain_statements("mysql", None, &stmts, false).unwrap();
        assert_eq!(assembled, vec![("EXPLAIN SELECT 1".to_string(), true)]);

        // analyze（真实执行）：MySQL 8.0.18+ / PG 的 ANALYZE 形态
        let assembled = explain_statements("mysql", None, &stmts, true).unwrap();
        assert_eq!(
            assembled,
            vec![("EXPLAIN ANALYZE SELECT 1".to_string(), true)]
        );
        let pg_stmts = split_statements("postgres", "SELECT 1");
        let assembled = explain_statements("postgres", None, &pg_stmts, true).unwrap();
        assert_eq!(
            assembled,
            vec![("EXPLAIN (ANALYZE, BUFFERS) SELECT 1".to_string(), true)]
        );

        // SQLite / 达梦
        let assembled = explain_statements("sqlite", None, &stmts, false).unwrap();
        assert_eq!(
            assembled,
            vec![("EXPLAIN QUERY PLAN SELECT 1".to_string(), true)]
        );
        let assembled = explain_statements("dameng", None, &stmts, false).unwrap();
        assert_eq!(assembled, vec![("EXPLAIN SELECT 1".to_string(), true)]);

        // OceanBase：MySQL 模式走 MySQL 口径，Oracle 模式明确拒绝（暂未接入）
        let assembled = explain_statements("oceanbase", Some("mysql"), &stmts, false).unwrap();
        assert_eq!(assembled, vec![("EXPLAIN SELECT 1".to_string(), true)]);
        assert!(explain_statements("oceanbase", Some("oracle"), &stmts, false).is_err());
    }

    /// 多语句选中：逐条装配为多个 EXPLAIN 结果（SQLite 复合语句语义）
    #[test]
    fn explain_assembly_multi_statements() {
        let stmts = split_statements("mysql", "SELECT 1; SELECT 2");
        let assembled = explain_statements("mysql", None, &stmts, false).unwrap();
        assert_eq!(assembled.len(), 2);
        assert_eq!(
            assembled.get(1).map(|x| x.0.as_str()),
            Some("EXPLAIN SELECT 2")
        );
    }

    /// Redis 无执行计划概念，直接拒绝
    #[test]
    fn explain_assembly_redis_rejected() {
        let stmts = split_statements("mysql", "SELECT 1");
        assert!(explain_statements("redis", None, &stmts, false).is_err());
    }
}
