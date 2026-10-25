//! Oracle 引擎真机回归（#[ignore]，显式
//! `cargo test -p app --lib oracle_live -- --ignored --nocapture` 触发）：
//! 直接读本机应用的 veloterm.db（SQLCipher）里第一条 oracle 连接记录，驱动
//! vendor/oracle_async 完整建连 + PING + SELECT。排查协议问题时配合
//! `RUST_LOG=oracle_async=trace` 使用。

use oracle_async::Queryable as _;

struct PrintLogger;
impl log::Log for PrintLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Trace && m.target().starts_with("oracle_async")
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            println!("[{}] {}", r.level(), r.args());
        }
    }
    fn flush(&self) {}
}

#[tokio::test]
#[ignore = "需要本机 veloterm.db 里有 oracle 连接记录（真机门禁，不入默认套件）"]
async fn oracle_live_connect_and_select() {
    let _ = log::set_boxed_logger(Box::new(PrintLogger));
    log::set_max_level(log::LevelFilter::Trace);

    let db_path = std::env::var("HOME")
        .map(|h| format!("{h}/Library/Application Support/com.gallentboy.veloterm.app/veloterm.db"))
        .unwrap();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.pragma_update(None, "key", crate::db::DEFAULT_MASTER_KEY).unwrap();

    let (host, port, user, pass, service, schema): (String, i64, String, String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT host, port, username, password, service_name, database \
             FROM db_connections WHERE engine='oracle' ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .expect("veloterm.db 里没有 oracle 连接记录");

    println!("live target: {user}@{host}:{port}/{service:?} (schema={schema:?})");

    let mut opts = oracle_async::Opts::builder()
        .host(host)
        .port(port as u16)
        .username(user)
        .password(pass);
    if let Some(svc) = service.filter(|s| !s.is_empty()) {
        opts = opts.service_name(svc);
    }

    let mut oc = oracle_async::Conn::connect(opts.build())
        .await
        .expect("oracle_async 连接失败");

    println!("connected: banner={}", {
        let b = oc.db_version_banner().trim();
        if b.is_empty() { oc.server_version() } else { b }
    });
    // 真实数据库版本（0x3B 查询）：版本分支（如 all_users）依赖它
    println!(
        "db_version={} (major={})",
        oc.db_version(),
        oc.db_version_major()
    );
    assert!(oc.db_version_major() >= 10, "真实版本号应已取得");
    oc.ping().await.expect("ping 失败");

    let mut rs = oc.query("SELECT 'hello' AS GREETING, 42 AS NUM FROM dual")
        .await
        .expect("SELECT 失败");
    println!("columns: {:?}", rs.columns().iter().map(|c| &c.name).collect::<Vec<_>>());
    while let Some(row) = rs.next_row().await.expect("行读取失败") {
        println!("row: {:?}", row.values());
    }

    // DDL/会话语句（真机曾报 ORA-01002 的路径）
    let affected = oc
        .exec("ALTER SESSION SET NLS_DATE_FORMAT='YYYY-MM-DD'")
        .await
        .expect("ALTER SESSION 失败");
    println!("alter session affected: {affected}");

    // 多行翻页：150 行 > 预取 100，触发 [3,5,0] 专用 Fetch 包
    let mut rs = oc
        .query("SELECT level AS N FROM dual CONNECT BY level <= 150")
        .await
        .expect("多行 SELECT 失败");
    let mut count = 0usize;
    let mut first: Option<oracle_async::OracleValue> = None;
    let mut last = None;
    while let Some(row) = rs.next_row().await.expect("多行读取失败") {
        if count == 0 {
            first = row.values().first().cloned();
        }
        last = row.values().first().cloned();
        count += 1;
    }
    println!("multi-row: {count} rows, first={first:?}, last={last:?}");
    assert_eq!(count, 150, "应取回全部 150 行");

    // ── CLOB 内联单元格（TO_CLOB 恒可跑；DBMS_METADATA.GET_DDL 同形态）──
    let mut rs = oc.query("SELECT TO_CLOB('hello-clob-inline') AS C FROM dual")
        .await
        .expect("CLOB 查询失败");
    let row = rs.next_row().await.expect("CLOB 行读取失败").expect("应有 1 行");
    let clob_val = format!("{}", row.values().first().unwrap_or(&oracle_async::OracleValue::Null));
    println!("clob cell: {clob_val}");
    assert_eq!(clob_val, "hello-clob-inline");

    // ── 绑定参数（P0-2）：数字/字符串/日期三种占位符，真机执行 ──
    use oracle_async::{BindParam, BindValue};
    let mut rs = oc
        .query_with_params(
            "SELECT :1 + 1 AS SUM_VAL, :2 || '-suffix' AS STR_VAL, \
             TO_CHAR(:3, 'YYYY-MM-DD') AS DATE_VAL FROM dual",
            vec![
                BindParam::input(BindValue::Int(41)),
                BindParam::input(BindValue::Text("bind".into())),
                BindParam::input(BindValue::Date(
                    chrono::NaiveDate::from_ymd_opt(2026, 10, 24).unwrap(),
                )),
            ],
        )
        .await
        .expect("绑定参数查询失败");
    let row = rs.next_row().await.expect("绑参行读取失败").expect("应有 1 行");
    let sum_val = format!("{}", row.values().first().unwrap_or(&oracle_async::OracleValue::Null));
    let str_val = format!("{}", row.values().get(1).unwrap_or(&oracle_async::OracleValue::Null));
    let date_val = format!("{}", row.values().get(2).unwrap_or(&oracle_async::OracleValue::Null));
    println!("bind params -> sum={sum_val}, str={str_val}, date={date_val}");
    assert_eq!(sum_val, "42", "数字绑定应得 42");
    assert_eq!(str_val, "bind-suffix", "字符串绑定应拼接成功");
    assert_eq!(date_val, "2026-10-24", "日期绑定应正确");

    // 中文绑定（852=GBK 服务器字符集编码路径）
    let mut rs = oc
        .query_with_params(
            "SELECT :1 AS CN FROM dual",
            vec![BindParam::input(BindValue::Text("中文绑定".into()))],
        )
        .await
        .expect("中文绑定查询失败");
    let cn = format!(
        "{}",
        rs.next_row().await.expect("中文绑定行读取失败").expect("应有 1 行")
            .values()
            .first()
            .unwrap_or(&oracle_async::OracleValue::Null)
    );
    println!("bind chinese -> {cn}");
    assert_eq!(cn, "中文绑定", "中文绑定经 GBK 编码往返应无损");

    // ── NLS 会话参数（P2-4，连接时自动加载；走 OUT 参数回读路径）──
    match oc.nls() {
        Some(nls) => println!(
            "nls: date_format={:?}, timestamp={:?}, numeric={:?}, sort={:?}",
            nls.date_format, nls.timestamp, nls.numeric_chars, nls.sort
        ),
        None => println!("nls: 未加载（非致命）"),
    }
    assert!(oc.nls().is_some(), "NLS 应已加载（OUT 参数回读）");
    assert!(
        !oc.nls().unwrap().date_format.is_empty(),
        "NLS_DATE_FORMAT 不应为空"
    );

    // ── 协议级 COMMIT / ROLLBACK（P2-6：simpleObject 0x0E / 0x0F）──
    oc.commit().await.expect("协议级 COMMIT 失败");
    oc.rollback().await.expect("协议级 ROLLBACK 失败");
    println!("protocol commit/rollback: ok");

    // ── 扩展类型解码（P2-1）：INTERVAL / BINARY_DOUBLE（11g 无 BOOLEAN 类型）──
    let mut rs = oc
        .query(
            "SELECT INTERVAL '1-2' YEAR TO MONTH AS IYM, \
                    INTERVAL '3 04:05:06.000007' DAY TO SECOND AS IDS, \
                    CAST(1.5 AS BINARY_DOUBLE) AS BD FROM dual",
        )
        .await
        .expect("扩展类型查询失败");
    if let Some(row) = rs.next_row().await.expect("扩展类型行读取失败") {
        let vals: Vec<String> = row.values().iter().map(|v| v.to_string()).collect();
        println!("ext types -> {vals:?}");
        assert!(vals.first().map(|s| s.contains("01-02")).unwrap_or(false), "INTERVAL YM 解码");
        assert!(vals.get(1).map(|s| s.starts_with("+03")).unwrap_or(false), "INTERVAL DS 解码");
        assert_eq!(vals.get(2).map(|s| s.as_str()), Some("1.5"), "BINARY_DOUBLE 解码");
    }

    // ── NUMBER 编码（P2-2）：Decimal/负数/小数绑定回读 ──
    use rust_decimal::Decimal;
    use std::str::FromStr;
    let mut rs = oc
        .query_with_params(
            "SELECT :1 + 0, :2 + 0 FROM dual",
            vec![
                BindParam::input(BindValue::Number(Decimal::from_str("123.456").unwrap())),
                BindParam::input(BindValue::Float(-0.5)),
            ],
        )
        .await
        .expect("NUMBER 编码绑定失败");
    let row = rs.next_row().await.expect("NUMBER 行读取失败").expect("应有 1 行");
    let d1 = format!("{}", row.values().first().unwrap_or(&oracle_async::OracleValue::Null));
    let d2 = format!("{}", row.values().get(1).unwrap_or(&oracle_async::OracleValue::Null));
    println!("number encode -> {d1}, {d2}");
    assert_eq!(d1, "123.456", "Decimal 绑定应保精度");
    assert_eq!(d2, "-0.5", "f64 负数绑定");

    // ── LOB（P2-3）：临时 CLOB 全流程 create → putString → getData → free ──
    // 严格对齐 go-ora value_getter.go 的调用序列（不 open、不改 locator）；
    // 任一步失败都会立刻体现在此处，不留给后续查询当脏流。
    {
        let mut lob = oracle_async::conn::routines::lob::create_temporary(&mut oc, true)
            .await
            .expect("创建临时 CLOB 失败");
        println!("lob: created, locator {} bytes", lob.source_locator.len());
        oracle_async::conn::routines::lob::put_string(&mut oc, &mut lob, "中文 LOB 写入测试 abc")
            .await
            .expect("putString 失败");
        let back = oracle_async::conn::routines::lob::get_data(&mut oc, &mut lob)
            .await
            .expect("getData 失败");
        // getData 返回服务器字符集原始字节（852=GBK），解码须走驱动同一套表
        let text = oracle_async::proto::charset::decode(&back, lob.charset_id);
        println!(
            "lob: read back {} bytes (charset {}) -> {text:?}",
            back.len(),
            lob.charset_id
        );
        assert_eq!(text, "中文 LOB 写入测试 abc", "临时 CLOB 往返应一致");
        oracle_async::conn::routines::lob::free_temporary(&mut oc, &mut lob)
            .await
            .expect("freeTemporary 失败");
        println!("lob: freed");
        // 释放后会话必须仍然干净：立刻跑一条查询验证流未错位
        let mut rs = oc.query("SELECT 1 FROM dual").await.expect("LOB 后查询失败");
        assert!(rs.next_row().await.expect("LOB 后行读取失败").is_some());
        println!("lob: session still clean after free");
    }

    // ── 目录/定义链路（对象树详情的真实口径，自动发现真实对象）──
    if let Some(schema) = schema.filter(|s| !s.is_empty()) {
        oc.exec(&format!("ALTER SESSION SET CURRENT_SCHEMA = {schema}"))
            .await
            .expect("切 schema 失败");

        // 找一张真实表（ROWNUM 全版本安全；11.2 无 FETCH FIRST 会 ORA-00933）
        let mut rs = oc
            .query(&format!(
                "SELECT table_name FROM all_tables WHERE owner = '{schema}' \
                 AND secondary = 'N' AND ROWNUM <= 1 ORDER BY table_name"
            ))
            .await
            .expect("查表清单失败");
        let table = match rs.next_row().await.expect("表行读取失败") {
            Some(r) => format!("{}", r.values().first().unwrap_or(&oracle_async::OracleValue::Null)),
            None => {
                println!("schema {schema} 无表，目录链路跳过");
                return;
            }
        };
        println!("catalog probe table: {schema}.{table}");

        // 表 DDL（DBMS_METADATA.GET_DDL，CLOB 结果）
        let mut rs = oc
            .query(&format!(
                "SELECT DBMS_METADATA.GET_DDL('TABLE', '{table}', '{schema}') FROM dual"
            ))
            .await
            .expect("GET_DDL 失败");
        if let Some(r) = rs.next_row().await.expect("DDL 行读取失败") {
            let ddl = format!("{}", r.values().first().unwrap_or(&oracle_async::OracleValue::Null));
            println!("table DDL ({} bytes): {}…", ddl.len(), &ddl[..ddl.len().min(80)]);
            assert!(ddl.to_uppercase().contains("CREATE"), "DDL 应含 CREATE");
        }

        // 列结构
        let mut rs = oc
            .query(&format!(
                "SELECT column_name, data_type, nullable FROM all_tab_columns \
                 WHERE owner = '{schema}' AND table_name = '{table}' ORDER BY column_id"
            ))
            .await
            .expect("列结构查询失败");
        let mut cols = 0usize;
        while rs.next_row().await.expect("列行读取失败").is_some() {
            cols += 1;
        }
        println!("table columns: {cols}");
        assert!(cols > 0, "表至少应有一列");

        // 索引（LISTAGG 聚合）
        let mut rs = oc
            .query(&format!(
                "SELECT i.index_name, i.uniqueness, LISTAGG(c.column_name, ', ') WITHIN GROUP \
                 (ORDER BY c.column_position) FROM all_indexes i JOIN all_ind_columns c \
                 ON c.index_name = i.index_name AND c.index_owner = i.owner \
                 WHERE i.table_owner = '{schema}' AND i.table_name = '{table}' \
                 GROUP BY i.index_name, i.uniqueness ORDER BY i.index_name"
            ))
            .await
            .expect("索引查询失败");
        let mut idx = 0usize;
        while rs.next_row().await.expect("索引行读取失败").is_some() {
            idx += 1;
        }
        println!("table indexes: {idx}");

        // 函数/过程源码（all_source 按行拼回）——先跑一条 0 行查询模拟应用
        // 对象树的 column_list（过程节点展开），紧接定义查询：验证
        // 「round-1 1403 → 关游标 → 下一语句」序列无残留错位
        let _ = oc
            .query(&format!(
                "SELECT column_name, data_type FROM all_tab_columns \
                 WHERE owner = '{schema}' AND table_name = 'COMPILE_INVALID_OBJECT_ALL' \
                 ORDER BY column_id"
            ))
            .await
            .expect("过程列查询（应为 0 行）失败");
        let mut rs = oc
            .query(&format!(
                "SELECT object_name, object_type FROM all_objects \
                 WHERE owner = '{schema}' AND object_type IN ('FUNCTION','PROCEDURE') \
                 AND object_name NOT LIKE 'BIN$%' AND ROWNUM <= 1"
            ))
            .await
            .expect("例程查询失败");
        if let Some(r) = rs.next_row().await.expect("例程行读取失败") {
            let (name, kind) = (format!("{}", r.values().first().unwrap_or(&oracle_async::OracleValue::Null)), format!("{}", r.values().get(1).unwrap_or(&oracle_async::OracleValue::Null)));
            println!("routine probe: {kind} {name}");
            let mut rs = oc
                .query(&format!(
                    "SELECT text FROM all_source WHERE owner = '{schema}' \
                     AND name = '{name}' AND type = '{kind}' ORDER BY line"
                ))
                .await
                .expect("源码查询失败");
            let mut src = String::new();
            while let Some(r) = rs.next_row().await.expect("源码行读取失败") {
                src.push_str(&format!("{}", r.values().first().unwrap_or(&oracle_async::OracleValue::Null)));
            }
            println!("routine source ({} bytes): {}…", src.len(), &src[..src.len().min(60)]);
            assert!(src.to_uppercase().contains(&name.to_uppercase()), "源码应含自身名");
            // 中文字符集解码验证：852=ZHS16GBK 库的中文注释不应有 U+FFFD 乱码
            if src.contains('\u{FFFD}') {
                println!("WARN: 源码含乱码替换符（字符集解码未生效？）");
            }
            // 定点验证用户报障对象 FUNC_NEXTID（中文注释 GBK 库）
            let mut rs = oc
                .query(&format!(
                    "SELECT text FROM all_source WHERE owner = '{schema}' \
                     AND name = 'FUNC_NEXTID' AND type = 'FUNCTION' ORDER BY line"
                ))
                .await
                .expect("FUNC_NEXTID 查询失败");
            let mut src2 = String::new();
            while let Some(r) = rs.next_row().await.expect("FUNC_NEXTID 行读取失败") {
                src2.push_str(&format!("{}", r.values().first().unwrap_or(&oracle_async::OracleValue::Null)));
            }
            if !src2.is_empty() {
                println!("FUNC_NEXTID ({} bytes), 含乱码替换符: {}", src2.len(), src2.contains('\u{FFFD}'));
                // 打印含中文的行样本
                for line in src2.lines() {
                    if !line.is_ascii() {
                        println!("中文行样本: {}", &line[..line.len().min(70)]);
                        break;
                    }
                }
                assert!(!src2.contains('\u{FFFD}'), "GBK 解码后不应有乱码替换符");
            }
        }

        // 序列 DDL
        let mut rs = oc
            .query(&format!(
                "SELECT DBMS_METADATA.GET_DDL('SEQUENCE', s.sequence_name, '{schema}') \
                 FROM all_sequences s WHERE s.sequence_owner = '{schema}' AND ROWNUM <= 1"
            ))
            .await
            .expect("序列查询失败");
        if let Some(r) = rs.next_row().await.expect("序列行读取失败") {
            let ddl = format!("{}", r.values().first().unwrap_or(&oracle_async::OracleValue::Null));
            println!("sequence DDL ({} bytes): {}…", ddl.len(), &ddl[..ddl.len().min(60)]);
        }
    }
}

/// 双驱动路由真机回归：经完整 DbEngine::connect 管线连接（区别于上一
/// 个测试的直接驱动调用），断言路由形态与服务端版本一致——11.2 真机
/// 必须落在 Legacy（自研驱动）；12c+ 机器会尝试官方驱动，失败时合法
/// 降级 Legacy（打印实际路径供人工核对）。
#[tokio::test]
#[ignore = "需要本机 veloterm.db 里有 oracle 连接记录（真机门禁，不入默认套件）"]
async fn oracle_live_driver_routing() {
    use super::DbEngine;
    use super::oracle_conn::OracleConn;
    use crate::db::DbConnectionRecord;

    let db_path = std::env::var("HOME")
        .map(|h| format!("{h}/Library/Application Support/com.gallentboy.veloterm.app/veloterm.db"))
        .unwrap();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.pragma_update(None, "key", crate::db::DEFAULT_MASTER_KEY).unwrap();

    let (id, host, port, user, pass, service, schema): (i64, String, i64, String, String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT id, host, port, username, password, service_name, database \
             FROM db_connections WHERE engine='oracle' ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .expect("veloterm.db 里没有 oracle 连接记录");

    let record = DbConnectionRecord {
        id: id as usize,
        name: "live-routing".to_string(),
        engine: "oracle".to_string(),
        host,
        port: port as u16,
        database: schema,
        service_name: service,
        mode: None,
        db_path: None,
        encryption_mode: "none".to_string(),
        username: Some(user),
        password: Some(pass),
        ssl_mode: "disable".to_string(),
        ssh_host_id: None,
        env_level: String::new(),
        readonly: false,
        color: None,
        compatible_mode: None,
        extra_options: None,
        created_at: 0,
        updated_at: 0,
    };

    let (engine, version) = DbEngine::connect(&record)
        .await
        .expect("完整管线连接失败");
    println!("routed version: {version}");

    let DbEngine::Oracle(oc) = &engine else {
        unreachable!("connect_oracle 只产 Oracle 变体");
    };
    let major = oc.version_major();
    match oc {
        OracleConn::Legacy(_) => {
            println!("路由：Legacy（自研驱动），server major = {major}");
            // 11.2 真机锚定：< 12 的服务端不允许落官方路径
            assert!(major < 12, "Legacy 路径只允许 <12 的服务端，实际 {major}");
        }
        OracleConn::Official(_) => {
            println!("路由：Official（官方驱动），server major = {major}");
            assert!(major >= 12, "Official 路径要求 12c+，实际 {major}");
        }
    }

    // 切库（CURRENT_SCHEMA）与提交回滚在路由后的会话上冒烟
    let mut engine = engine;
    engine.select_database("SYS").await.expect("切 schema 失败");
    engine.commit().await.expect("commit 失败");
    engine.rollback().await.expect("rollback 失败");
    println!("select_database/commit/rollback 冒烟通过");
}
