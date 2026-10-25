//! 官方 oracledb 驱动桥接（12c+ 路径）。
//!
//! Oracle 官方纯 Rust 瘦驱动（`oracledb` crate，python-oracledb thin 的
//! clean-room 移植）——同步 API、零 C 依赖。连接 12c+（19c/21c/26ai 过
//! 官方认证），10g/11g 由自研 oracle_async 承接；版本自动路由见
//! engines/connect.rs 的 connect_oracle。
//!
//! 并发模型：`oracledb::Connection` 是 Send + Sync（内部 `Arc<Mutex<Client>>`
//! 串行化协议访问），本桥接持 `Arc<Connection>`，每个操作一次
//! `spawn_blocking`——并发操作在驱动内部的互斥锁上自然排队，不存在两
//! 个线程同时打一条连接的协议错乱。外层超时把 future 丢弃后，阻塞线程
//! 可能仍在跑：后续操作会在互斥锁上等它收尾，`call_timeout`（每个网络
//! 往返 30s 上限，建连时设置）保证卡死操作必然返回、锁必然释放。
//!
//! 已知降级（相对自研驱动，用户已确认接受）：
//! - **无中断 API**（无 cancel/interrupt/break）：「取消」只能丢弃后续
//!   批次，正在执行的语句等 call_timeout 或自然返回；
//! - **无隐式结果集 API**：PL/SQL `dbms_sql.return_result` 只见主结果集
//!   （自研驱动的 Msg 27 自动接续在此路径不存在）；
//! - **无 0x3B banner 等价物**：版本串用 `get_server_version()` 的五段
//!   数字（如 "19.0.0.0.0"）。

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use super::oracle_engine::{ora_server_text, ora_text_sniff};
use oracledb::{
    Connection as OfficialConnection, ErrorKind, FromDbValue, OracleIntervalDS,
    OracleIntervalYM, OracleNumber, OracleTimestamp, Row,
};

/// 每个网络往返的超时上限（call timeout）。同时是「取消」的兜底延迟
/// 上界：正在执行的服务端语句最迟一个往返窗口后返回。
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// 官方驱动的会话句柄（Arc 共享给各 spawn_blocking 闭包）。
pub struct OfficialOracle {
    conn: Arc<OfficialConnection>,
    /// 建连时缓存的主版本（目录 SQL 版本自适应判据，如 all_users 的
    /// oracle_maintained 分支、LISTAGG 聚列分支）
    version_major: u32,
    /// 建连时缓存的版本串（五段数字形态）
    version_text: String,
}

impl OfficialOracle {
    /// 建连（Easy Connect 串 + 凭据 + call timeout + 版本缓存）。
    /// 失败时错误已经过 [`oracle_err_official`] 映射——路由层据此降级
    /// 保留自研探测连接。
    pub(crate) async fn connect(
        host: &str,
        port: u16,
        service: &str,
        username: &str,
        password: &str,
    ) -> Result<Self, String> {
        let connect_string = format!("{host}:{port}/{service}");
        let username = username.to_string();
        let password = password.to_string();
        tokio::task::spawn_blocking(move || {
            let config = oracledb::Config::default()
                .set_credentials(&username, &password)
                .set_connect_string(&connect_string)
                .map_err(oracle_err_official)?;
            let conn = oracledb::connect(config).map_err(oracle_err_official)?;
            // 每个网络往返的上限：取消兜底 + 防卡死（见模块头「并发模型」）
            conn.set_call_timeout(Some(CALL_TIMEOUT))
                .map_err(oracle_err_official)?;
            let version = conn.version().map_err(oracle_err_official)?;
            Ok::<_, String>(Self {
                conn: Arc::new(conn),
                version_major: version.0 as u32,
                version_text: version.to_string(),
            })
        })
        .await
        .map_err(|e| format!("Oracle 官方驱动阻塞任务异常：{e}"))?
    }

    pub(crate) fn version_major(&self) -> u32 {
        self.version_major
    }

    pub(crate) fn version_text(&self) -> &str {
        &self.version_text
    }

    /// 在阻塞线程上跑一个闭包（闭包拿 `&Connection`）。
    async fn run<T, F>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&OfficialConnection) -> Result<T, oracledb::Error> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || f(&conn))
            .await
            .map_err(|e| format!("Oracle 官方驱动阻塞任务异常：{e}"))?
            .map_err(oracle_err_official)
    }

    /// 执行无结果集语句（DML/DDL/ALTER SESSION），返回受影响行数。
    pub(crate) async fn exec_sql(&self, sql: &str) -> Result<u64, String> {
        let sql = sql.to_string();
        self.run(move |conn| {
            let mut stmt = conn.statement(&sql)?.fetch_lobs().build()?;
            stmt.ensure_fully_parsed()?;
            if stmt.is_query() {
                // 目录层不该传查询进来；防御性取尽（等价空执行）
                let _ = stmt.query(&[])?.count();
                Ok(0)
            } else {
                let result = stmt.execute(&[])?;
                Ok(result.rows_affected())
            }
        })
        .await
    }

    /// 查询并全量收集为字符串化行（小结果集；行流见 exec_streaming 分派）。
    pub(crate) async fn collect_rows(&self, sql: &str) -> Result<Vec<Vec<String>>, String> {
        let sql = sql.to_string();
        self.run(move |conn| {
            let mut stmt = conn.statement(&sql)?.fetch_lobs().build()?;
            stmt.ensure_fully_parsed()?;
            if !stmt.is_query() {
                // 非查询误入：等价空行集（列信息为空）
                let _ = stmt.execute(&[])?;
                return Ok(Vec::new());
            }
            let cursor = stmt.query(&[])?;
            let mut rows = Vec::new();
            for row in cursor {
                let row = row?;
                rows.push(row_to_strings(&row));
            }
            Ok(rows)
        })
        .await
    }

    pub(crate) async fn commit(&self) -> Result<(), String> {
        self.run(|conn| conn.commit()).await
    }

    pub(crate) async fn rollback(&self) -> Result<(), String> {
        self.run(|conn| conn.rollback()).await
    }

    /// 查询工作台行流：批量取行（每批 ≤ [`super::ROWS_PER_BATCH`]）+
    /// 批间取消检查。取消语义（用户已确认的降级）：置 cancelled 后丢弃
    /// 后续批次，正在取的那一批在后台自然收尾（游标随闭包结束释放）；
    /// 服务端仍在执行的语句等 call_timeout 或自然返回——官方驱动无
    /// 中断 API。
    pub(crate) async fn exec_streaming(
        &self,
        sql: &str,
        max_rows: usize,
        cancel_notify: &tokio::sync::Notify,
        mut on_columns: impl FnMut(&[String]),
        mut on_rows: impl FnMut(Vec<Vec<String>>),
    ) -> Result<super::ExecSummary, String> {
        use super::ROWS_PER_BATCH;

        // 一次性管道值：Query 臂带游标天然大于 Exec 臂，不装箱
        #[allow(clippy::large_enum_variant)]
        enum Start {
            Query {
                cursor: Option<oracledb::Cursor>,
                columns: Vec<String>,
            },
            Exec {
                affected: u64,
            },
        }

        // 语句启动：DML → affected；查询 → 游标 + 列名（与 Legacy 臂的
        // 首段 select! 取消点对齐：等待启动期间取消直接返回）
        let conn = Arc::clone(&self.conn);
        let sql_owned = sql.to_string();
        let start = tokio::select! {
            res = tokio::task::spawn_blocking(move || -> Result<Start, oracledb::Error> {
                let mut stmt = conn.statement(&sql_owned)?.fetch_lobs().build()?;
                stmt.ensure_fully_parsed()?;
                if stmt.is_query() {
                    let cursor = stmt.query(&[])?;
                    let columns: Vec<String> = cursor
                        .columns()
                        .iter()
                        .map(|m| m.name().to_string())
                        .collect();
                    Ok(Start::Query { cursor: Some(cursor), columns })
                } else {
                    let result = stmt.execute(&[])?;
                    Ok(Start::Exec { affected: result.rows_affected() })
                }
            }) => {
                res.map_err(|e| format!("Oracle 官方驱动阻塞任务异常：{e}"))
                    .and_then(|r| r.map_err(oracle_err_official))?
            }
            _ = cancel_notify.notified() => {
                // 启动期间的取消：后台阻塞任务自然收尾，不await
                return Ok(super::ExecSummary {
                    cancelled: true,
                    ..Default::default()
                });
            }
        };

        let mut summary = super::ExecSummary::default();
        let cursor = match start {
            // DML / DDL / PLSQL 无结果集：affected 走 rows_affected
            Start::Exec { affected } => {
                summary.affected = Some(affected);
                return Ok(summary);
            }
            Start::Query { cursor, columns } => {
                on_columns(&columns);
                summary.columns = columns;
                cursor
            }
        };

        // 批间取消检查的行循环；批内（≤300 行）不可打断。取消时游标已
        // move 进后台闭包，随其收尾自然释放（call_timeout 兜底）
        let mut cursor_opt: Option<oracledb::Cursor> = cursor;
        loop {
            if summary.rows_read >= max_rows {
                // 与 Legacy 臂语义对齐：到达 max_rows 即标截断（不探余行）
                summary.truncated = true;
                break;
            }
            let cap = ROWS_PER_BATCH.min(max_rows - summary.rows_read);
            let cursor_in = cursor_opt.take();
            let batch = tokio::select! {
                res = tokio::task::spawn_blocking(move || fetch_batch(cursor_in, cap)) => {
                    res.map_err(|e| format!("Oracle 官方驱动阻塞任务异常：{e}"))
                        .and_then(|r| r.map_err(|e| format!("Oracle 行读取错误：{}", oracle_err_official(e))))?
                }
                _ = cancel_notify.notified() => {
                    summary.cancelled = true;
                    break;
                }
            };
            let (cursor_back, rows, done) = batch;
            cursor_opt = cursor_back;
            summary.rows_read += rows.len();
            if !rows.is_empty() {
                on_rows(rows);
            }
            if done {
                break;
            }
        }

        Ok(summary)
    }
}

/// 整行 → 字符串单元格（NULL 渲染 "NULL"，对齐自研驱动的
/// OracleValue Display 语义；Binary 渲染 `<N bytes>` 同源）。
fn row_to_strings(row: &Row) -> Vec<String> {
    (0..row.columns().len())
        .map(|i| cell_to_string(row, i))
        .collect()
}

/// 从游标取至多 `cap` 行（阻塞线程上执行）：返回（游标，行集，是否取尽）。
/// 取尽后游标仍随返回值还回（调用方决定是否丢弃），语句资源随游标
/// Drop 释放。
/// 一批的结果：游标（还回续取）+ 已字符串化行集 + 是否取尽。
type FetchedBatch = (Option<oracledb::Cursor>, Vec<Vec<String>>, bool);

fn fetch_batch(
    mut cursor: Option<oracledb::Cursor>,
    cap: usize,
) -> Result<FetchedBatch, oracledb::Error> {
    let mut rows = Vec::new();
    let mut done = false;
    if let Some(cur) = cursor.as_mut() {
        while rows.len() < cap {
            match cur.next() {
                Some(Ok(row)) => rows.push(row_to_strings(&row)),
                Some(Err(e)) => return Err(e),
                None => {
                    done = true;
                    break;
                }
            }
        }
    } else {
        done = true;
    }
    Ok((cursor, rows, done))
}

/// 单元格 → 展示字符串：按列元数据的 db_type 分派到具体类型再
/// `to_string`，类型不匹配（或未知类型）落入通用回退链
/// String → Bytes → Timestamp（rustgrid decode_cell 同型的字符串化简化）。
fn cell_to_string(row: &Row, i: usize) -> String {
    use oracledb::{
        DB_TYPE_BINARY_DOUBLE, DB_TYPE_BINARY_FLOAT, DB_TYPE_BOOLEAN, DB_TYPE_DATE,
        DB_TYPE_INTERVAL_DS, DB_TYPE_INTERVAL_YM, DB_TYPE_NUMBER, DB_TYPE_TIMESTAMP,
        DB_TYPE_TIMESTAMP_LTZ, DB_TYPE_TIMESTAMP_TZ,
    };
    let db_type = row
        .columns()
        .get(i)
        .map(|meta| meta.db_type())
        .unwrap_or(oracledb::DB_TYPE_UNKNOWN);
    if db_type == DB_TYPE_NUMBER {
        if let Some(s) = cell_try::<OracleNumber>(row, i) {
            return s;
        }
    } else if db_type == DB_TYPE_BINARY_FLOAT {
        if let Some(s) = cell_try::<f32>(row, i) {
            return s;
        }
    } else if db_type == DB_TYPE_BINARY_DOUBLE {
        if let Some(s) = cell_try::<f64>(row, i) {
            return s;
        }
    } else if db_type == DB_TYPE_BOOLEAN {
        if let Some(s) = cell_try::<bool>(row, i) {
            return s;
        }
    } else if db_type == DB_TYPE_DATE
        || db_type == DB_TYPE_TIMESTAMP
        || db_type == DB_TYPE_TIMESTAMP_TZ
        || db_type == DB_TYPE_TIMESTAMP_LTZ
    {
        if let Some(s) = cell_try::<OracleTimestamp>(row, i) {
            return s;
        }
    } else if db_type == DB_TYPE_INTERVAL_DS
        && let Some(s) = cell_try::<OracleIntervalDS>(row, i)
    {
        return s;
    } else if db_type == DB_TYPE_INTERVAL_YM
        && let Some(s) = cell_try::<OracleIntervalYM>(row, i)
    {
        return s;
    }
    // 通用回退链：字符串族（VARCHAR/CHAR/NVARCHAR/CLOB 内联/ROWID）→
    // 二进制族（RAW/BLOB 内联）→ 时间戳 → "NULL"
    cell_try::<String>(row, i)
        .or_else(|| {
            row.get::<Option<Vec<u8>>>(i).ok().map(|v| match v {
                Some(bytes) => format!("<{} bytes>", bytes.len()),
                None => "NULL".to_string(),
            })
        })
        .or_else(|| cell_try::<OracleTimestamp>(row, i))
        .unwrap_or_else(|| "NULL".to_string())
}

/// 按类型取一格：Ok(Some) → to_string；Ok(None) → "NULL"；
/// Err（类型不匹配/不支持转换）→ None（落入下一档回退）。
fn cell_try<T>(row: &Row, i: usize) -> Option<String>
where
    T: for<'a> FromDbValue<'a> + fmt::Display,
{
    match row.get::<Option<T>>(i) {
        Ok(Some(v)) => Some(v.to_string()),
        Ok(None) => Some("NULL".to_string()),
        Err(_) => None,
    }
}

/// oracledb::Error → 展示文案。DbError（服务器 ORA 错误）走结构化码 +
/// 原文的 [`ora_server_text`]；其余变体（协议/IO/取消超时）Display 文本
/// 嗅探 [`ora_text_sniff`]——两个函数与自研驱动的 oracle_err 共享同一
/// 中文码表与输出形态，两路径错误文案完全一致。
pub(crate) fn oracle_err_official(e: oracledb::Error) -> String {
    match e.kind() {
        ErrorKind::DbError(db) => {
            ora_server_text(db.code() as i32, db.message())
        }
        _ => ora_text_sniff(&e.to_string()),
    }
}
