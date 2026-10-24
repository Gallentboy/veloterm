//! SQL schema 补全的真机回归（#[ignore]，显式
//! `cargo test -p app --lib sql_completion_live -- --ignored --nocapture` 触发）：
//! 读本机应用的 veloterm.db（SQLCipher）里第一条 oracle 连接记录，走
//! DbEngine::connect 完整建连，建一张混合大小写测试表后**小写化模拟用户输入**，
//! 验证补全全链路三态 + 规范名解析——Oracle 目录存大写，用户敲小写若不对
//! 规范名，列查询零行返回，即「表名能补、字段名没反应」的根因场景。

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::{Arc, RwLock};

use super::engines::DbEngine;
use super::{DbSession, DbSessionMeta};
use crate::db::DbConnectionRecord;

/// 幂等建一张混合大小写表（先忽略错误 DROP 旧表），返回规范表名
async fn setup_table(engine: &mut DbEngine) -> &'static str {
    let cancel = Arc::new(tokio::sync::Notify::new());
    let _ = engine
        .exec_streaming("DROP TABLE VeloCompLv", 1, &cancel, |_| {}, |_| {})
        .await;
    engine
        .exec_streaming(
            "CREATE TABLE VeloCompLv (VeloId NUMBER, VeloName VARCHAR2(64))",
            1,
            &cancel,
            |_| {},
            |_| {},
        )
        .await
        .expect("建测试表失败");
    "VELOCOMPLV"
}

fn session_for(engine: DbEngine, schema: &str) -> DbSession {
    DbSession {
        meta: RwLock::new(DbSessionMeta {
            session_id: "live-test".into(),
            connection_id: 0,
            name: "live-test".into(),
            engine: "oracle".into(),
            mode: None,
            compatible_mode: None,
            database: Some(schema.to_string()),
            server_version: String::new(),
            readonly: false,
            env_level: String::new(),
        }),
        engine: tokio::sync::Mutex::new(engine),
        cancel: Arc::new(tokio::sync::Notify::new()),
        cancelled: Arc::new(AtomicBool::new(false)),
        auto_commit: Arc::new(AtomicBool::new(false)),
        in_transaction: Arc::new(AtomicBool::new(false)),
        pending_tx_statements: Arc::new(AtomicUsize::new(0)),
        pending_tx_rows: Arc::new(AtomicU64::new(0)),
        completion_cache: std::sync::Mutex::new(Default::default()),
    }
}

#[tokio::test]
#[ignore = "需要本机 veloterm.db 里有 oracle 连接记录（真机门禁，不入默认套件）"]
async fn sql_completion_live_oracle() {
    let db_path = std::env::var("HOME")
        .map(|h| format!("{h}/Library/Application Support/com.gallentboy.veloterm.app/veloterm.db"))
        .unwrap();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.pragma_update(None, "key", crate::db::DEFAULT_MASTER_KEY).unwrap();
    let (id, host, port, user, pass, service, database): (
        i64,
        String,
        i64,
        String,
        String,
        Option<String>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT id, host, port, username, password, service_name, database \
             FROM db_connections WHERE engine='oracle' ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .expect("veloterm.db 里没有 oracle 连接记录");
    drop(conn);

    let record = DbConnectionRecord {
        id: id as usize,
        name: "live".into(),
        engine: "oracle".into(),
        host,
        port: port as u16,
        database,
        service_name: service,
        mode: None,
        db_path: None,
        encryption_mode: "none".into(),
        username: Some(user.clone()),
        password: Some(pass),
        ssl_mode: "disable".into(),
        ssh_host_id: None,
        env_level: String::new(),
        readonly: false,
        color: None,
        compatible_mode: None,
        extra_options: None,
        created_at: 0,
        updated_at: 0,
    };
    let schema = record
        .database
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| user.to_uppercase());

    let (mut engine, version) = DbEngine::connect(&record).await.expect("Oracle 连接失败");
    println!("connected: {version}, schema: {schema}");
    let table = setup_table(&mut engine).await.to_string();
    // 模拟用户输入：全小写（Oracle 目录存大写——规范名解析的靶场景）
    let typed = table.to_ascii_lowercase();
    let session = session_for(engine, &schema);

    // ① 表名位：敲到一半（小写前缀）能对出规范大写表名
    let r = super::sql_completion::sql_completions(&session, "SELECT * FROM velo", 19)
        .await
        .expect("表名位补全失败");
    let labels: Vec<&str> = r.suggestions.iter().map(|s| s.label.as_str()).collect();
    assert!(
        labels.contains(&table.as_str()),
        "表名位应含规范名 {table}，got {labels:?}"
    );
    let hit = r
        .suggestions
        .iter()
        .find(|s| s.label == table)
        .expect("上断言已保证存在");
    assert_eq!(hit.insert_text, table, "插入的应是目录规范名");

    // ② 别名 t.：小写表名 + 小写别名 → 列建议非空且为规范列名
    let sql = format!("SELECT * FROM {typed} v WHERE v.");
    let r = super::sql_completion::sql_completions(&session, &sql, sql.len())
        .await
        .expect("别名点补全失败");
    assert!(
        !r.suggestions.is_empty(),
        "别名 v. 应出列（规范名解析生效）"
    );
    assert!(r.suggestions.iter().any(|s| s.label == "VELOID"), "应含 VELOID");
    assert!(r.suggestions.iter().any(|s| s.label == "VELONAME"), "应含 VELONAME");
    println!("alias dot → {} 列", r.suggestions.len());

    // ③ 裸列名位：语句引用小写表名 → 列并集
    let sql = format!("SELECT * FROM {typed} WHERE velo");
    let r = super::sql_completion::sql_completions(&session, &sql, sql.len())
        .await
        .expect("列名位补全失败");
    assert!(!r.suggestions.is_empty(), "列名位应出列");

    // ④ 大写表名直接点（原样命中路径不回归）
    let sql = format!("SELECT * FROM {table} WHERE {table}.");
    let r = super::sql_completion::sql_completions(&session, &sql, sql.len())
        .await
        .expect("表名点补全失败");
    assert!(!r.suggestions.is_empty(), "大写表名点应出列");

    // 清理测试表
    let mut engine = session.engine.lock().await;
    engine
        .exec_streaming("DROP TABLE VeloCompLv", 1, &session.cancel, |_| {}, |_| {})
        .await
        .expect("清理测试表失败");
}
