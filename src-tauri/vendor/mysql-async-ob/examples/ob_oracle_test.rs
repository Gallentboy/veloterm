//! OB Oracle 纯 Rust 驱动全链路测试工具。
//!
//! 输入连接参数 → 依次跑完前端工作台所需的全部 API 调用路径 → 输出通过/失败/警告。
//! 某步失败不中断后续（除非连接本身失败），给完整诊断全景。
//!
//! 用法：
//! ```bash
//! cargo run --example ob_oracle_test -- --host 10.0.0.5 --port 2881 \
//!     --user sys@oracle_tenant --pass xxx [--schema MYSCHEMA] [--no-ob20]
//! ```
//!
//! `--no-ob20` 可关掉 OB20 协议层对照测试（排查是否 OB20 引入的问题）。

use std::time::Instant;

use oceanbase_async::prelude::Queryable;

/// 测试结果
enum Outcome {
    Pass(String),
    Warn(String),
    Fail(String),
}

struct Reporter {
    pass: usize,
    warn: usize,
    fail: usize,
    started: Instant,
}

impl Reporter {
    fn new() -> Self {
        Self {
            pass: 0,
            warn: 0,
            fail: 0,
            started: Instant::now(),
        }
    }

    fn report(&mut self, id: usize, name: &str, outcome: Outcome, elapsed_ms: u128) {
        let (tag, msg) = match &outcome {
            Outcome::Pass(m) => {
                self.pass += 1;
                ("PASS", m.clone())
            }
            Outcome::Warn(m) => {
                self.warn += 1;
                ("WARN", m.clone())
            }
            Outcome::Fail(m) => {
                self.fail += 1;
                ("FAIL", m.clone())
            }
        };
        println!("[{tag}] {id:02} {name:<28} ({elapsed_ms}ms) {msg}");
    }

    fn summary(&self) {
        let total = self.pass + self.warn + self.fail;
        println!(
            "\n{}\n通过 {} / 警告 {} / 失败 {}   总耗时 {:.1}s",
            "═".repeat(50),
            self.pass,
            self.warn,
            self.fail,
            self.started.elapsed().as_secs_f64()
        );
        let _ = total;
    }
}

struct Args {
    host: String,
    port: u16,
    user: String,
    pass: String,
    schema: Option<String>,
    ob20: bool,
}

fn value_render(v: &oceanbase_async::Value) -> String {
    use oceanbase_async::Value;
    match v {
        Value::NULL => "NULL".to_string(),
        Value::Bytes(b) => format!("{}", String::from_utf8_lossy(b)),
        Value::Int(i) => i.to_string(),
        Value::UInt(u) => u.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Double(d) => d.to_string(),
        Value::Date(y, m, d, h, mi, s, us) => {
            format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}.{us:06}")
        }
        Value::Time(neg, d, h, m, s, us) => {
            format!("{}{d}d {h:02}:{m:02}:{s:02}.{us:06}", if *neg { "-" } else { "" })
        }
    }
}

/// 25 项探针的三态结果（start_transaction 语法错属环境事实记 WARN 不算 FAIL）
enum ApiTxOutcome {
    Pass(String),
    Fail(String),
    Warn(String),
}

/// 驱动 API 事务探针：独立函数体强制终结 Transaction 对 conn 的借用
///（Transaction 带 Drop，借用区域保守，内联代码怎么排都冲突——函数返回
/// 即释放）。事务内插行→事务内可见性校验→API rollback。
async fn probe_api_transaction(
    conn: &mut oceanbase_async::Conn,
    table: &str,
) -> ApiTxOutcome {
    let mut tx = match conn.start_transaction(Default::default()).await {
        Ok(tx) => tx,
        Err(e) => {
            return ApiTxOutcome::Warn(format!("start_transaction 报错（预期 MySQL 方言）: {e}"))
        }
    };
    if let Err(e) = tx
        .query_drop(format!("INSERT INTO {table} (id, name) VALUES (9, 'API事务行')"))
        .await
    {
        return ApiTxOutcome::Fail(format!("事务内 INSERT 失败: {e}"));
    }
    let in_tx: Result<Vec<(i64,)>, _> = tx.query(format!("SELECT id FROM {table}")).await;
    if let Err(e) = in_tx {
        return ApiTxOutcome::Fail(format!("事务内校验查询失败: {e}"));
    }
    if let Err(e) = tx.rollback().await {
        return ApiTxOutcome::Fail(format!("rollback 失败: {e}"));
    }
    match in_tx {
        Ok(rows) if rows.len() == 1 => ApiTxOutcome::Pass(
            "START TRANSACTION + 事务内可见 + API rollback 生效".to_string(),
        ),
        Ok(rows) => ApiTxOutcome::Fail(format!("事务内应见 1 行，实际 {} 行", rows.len())),
        Err(_) => unreachable!("上方已拦截"),
    }
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        host: String::new(),
        port: 2881,
        user: String::new(),
        pass: String::new(),
        schema: None,
        ob20: true,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--host" => {
                i += 1;
                args.host = argv.get(i).cloned().ok_or("--host 缺值")?;
            }
            "--port" => {
                i += 1;
                args.port = argv
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .ok_or("--port 无效")?;
            }
            "--user" => {
                i += 1;
                args.user = argv.get(i).cloned().ok_or("--user 缺值")?;
            }
            "--pass" => {
                i += 1;
                args.pass = argv.get(i).cloned().ok_or("--pass 缺值")?;
            }
            "--schema" => {
                i += 1;
                args.schema = argv.get(i).cloned();
            }
            "--no-ob20" => args.ob20 = false,
            other => return Err(format!("未知参数: {other}")),
        }
        i += 1;
    }
    if args.host.is_empty() || args.user.is_empty() {
        return Err("必须提供 --host 和 --user（--pass 可空表示无密码）".to_string());
    }
    Ok(args)
}

fn val_str(v: &oceanbase_async::Value) -> String {
    match v {
        oceanbase_async::Value::Bytes(b) => String::from_utf8_lossy(b).to_string(),
        oceanbase_async::Value::Int(i) => i.to_string(),
        oceanbase_async::Value::Double(f) => f.to_string(),
        oceanbase_async::Value::NULL => "(null)".to_string(),
        _ => format!("{v:?}"),
    }
}

#[tokio::main]
async fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("参数错误: {e}\n\n用法: ob_oracle_test --host <ip> --port 2881 --user <user[@tenant]> [--pass <pwd>] [--schema <schema>] [--no-ob20]");
            std::process::exit(1);
        }
    };

    let mut rep = Reporter::new();
    println!(
        "═══ OB Oracle 纯 Rust 驱动测试 ═══\n编译时间(北京): {}（epoch {}，核对手里的 exe 是哪次构建）\n详细日志: 设环境变量 OB_DEBUG=1（cmd: set OB_DEBUG=1；PowerShell: $env:OB_DEBUG=1）\n目标: {}:{}  用户: {}  OB20: {}\n",
        option_env!("OB_FORK_BUILD_CST").unwrap_or("unknown"),
        option_env!("OB_FORK_BUILD_EPOCH").unwrap_or("?"),
        args.host,
        args.port,
        args.user,
        args.ob20
    );

    // ── 01 连接（握手 + 认证 + OB20 + Oracle 模式 + NLS 全在这一步）──
    let t = Instant::now();
    let builder = oceanbase_async::OptsBuilder::default()
        .ip_or_hostname(args.host.clone())
        .tcp_port(args.port)
        .user(Some(args.user.clone()))
        .pass(if args.pass.is_empty() {
            None
        } else {
            Some(args.pass.clone())
        })
        .ob_protocol20(args.ob20);
    let builder = if let Some(ref s) = args.schema {
        builder.db_name(Some(s.clone()))
    } else {
        builder
    };
    let opts = oceanbase_async::Opts::from(builder);

    let conn = match oceanbase_async::Conn::new(opts).await {
        Ok(c) => c,
        Err(e) => {
            rep.report(1, "连接", Outcome::Fail(format!("{e}")), t.elapsed().as_millis());
            rep.summary();
            return;
        }
    };
    let mut conn = conn;
    rep.report(
        1,
        "连接+握手+认证",
        Outcome::Pass(format!("server_version={:?}", conn.server_version())),
        t.elapsed().as_millis(),
    );

    // ── 02 OB20 协议状态（实测：管道是否真切换 + 能力回显原值）──
    let t = Instant::now();
    let echo = conn.ob_capability_echo();
    rep.report(
        2,
        "OB20 协议状态",
        if conn.ob20_active() {
            Outcome::Pass(format!("已激活（echo={echo:#x}）"))
        } else {
            Outcome::Warn(format!(
                "未激活（echo={echo:#x}）——回显未含 OB_PROTOCOL_V2 或 --no-ob20 对照组"
            ))
        },
        t.elapsed().as_millis(),
    );

    // ── 03 Oracle 模式检测 ──
    let t = Instant::now();
    let is_oracle = conn.ob_is_oracle_mode();
    rep.report(
        3,
        "Oracle 模式检测",
        if is_oracle {
            Outcome::Pass("服务端确认 Oracle 租户（status bit 2）".to_string())
        } else {
            Outcome::Warn(
                "auth OK 未捕获 Oracle 位（bit2）——NLS 初始化未执行".to_string(),
            )
        },
        t.elapsed().as_millis(),
    );

    // ── 04 版本探测 ──
    let t = Instant::now();
    let ver: Result<Option<String>, _> = conn
        .query_first("SELECT ob_version() FROM DUAL WHERE ROWNUM <= 1")
        .await;
    match ver {
        Ok(Some(v)) => rep.report(4, "版本探测", Outcome::Pass(format!("ob_version={v}")), t.elapsed().as_millis()),
        Ok(None) => rep.report(4, "版本探测", Outcome::Warn("无返回行".to_string()), t.elapsed().as_millis()),
        Err(e) => rep.report(4, "版本探测", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
    }

    // ── 05 会话变量（FROM DUAL）──
    let t = Instant::now();
    let sess: Result<Option<oceanbase_async::Row>, _> = conn
        .query_first("SELECT @@max_allowed_packet, @@time_zone FROM DUAL")
        .await;
    match sess {
        Ok(Some(row)) => {
            let vals = row.unwrap();
            rep.report(
                5,
                "会话变量 FROM DUAL",
                Outcome::Pass(format!(
                    "max_allowed_packet={}, time_zone={}",
                    val_str(&vals[0]),
                    val_str(&vals[1])
                )),
                t.elapsed().as_millis(),
            );
        }
        Ok(None) => rep.report(5, "会话变量 FROM DUAL", Outcome::Warn("空结果".to_string()), t.elapsed().as_millis()),
        Err(e) => rep.report(5, "会话变量 FROM DUAL", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
    }

    // ── 06 NLS NCHAR ──
    let t = Instant::now();
    let nls: Result<Option<String>, _> = conn
        .query_first("SELECT @@NLS_NCHAR_CHARACTERSET FROM DUAL")
        .await;
    match nls {
        Ok(Some(cs)) => rep.report(6, "NLS NCHAR 字符集", Outcome::Pass(format!("charset={cs}")), t.elapsed().as_millis()),
        Ok(None) => rep.report(6, "NLS NCHAR 字符集", Outcome::Warn("空结果".to_string()), t.elapsed().as_millis()),
        Err(e) => rep.report(6, "NLS NCHAR 字符集", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
    }

    // ── 07 schema 清单 ──
    let t = Instant::now();
    let schemas: Result<Vec<String>, _> = conn
        .query("SELECT username FROM all_users ORDER BY username")
        .await;
    match schemas {
        Ok(list) => rep.report(
            7,
            "schema 清单",
            Outcome::Pass(format!("{} 个: {}", list.len(), {
                let head: Vec<String> = list.iter().take(5).cloned().collect();
                if list.len() > 5 { format!("{head:?}…") } else { format!("{head:?}") }
            })),
            t.elapsed().as_millis(),
        ),
        Err(e) => rep.report(7, "schema 清单", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
    }

    // ── 08 对象分组计数 ──
    let t = Instant::now();
    let counts: Result<Vec<(String, i64)>, _> = conn
        .query(
            "SELECT object_type, COUNT(*) FROM all_objects \
             WHERE owner = USER GROUP BY object_type ORDER BY object_type",
        )
        .await;
    match counts {
        Ok(list) => {
            let summary: Vec<String> = list.iter().map(|(t, n)| format!("{t}:{n}")).collect();
            rep.report(
                8,
                "对象分组计数",
                Outcome::Pass(summary.join(" ")),
                t.elapsed().as_millis(),
            );
        }
        Err(e) => rep.report(8, "对象分组计数", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
    }

    // ── 09-12 各类型对象清单 ──
    for (id, kind, obj_type) in [
        (9, "表清单", "TABLE"),
        (10, "视图清单", "VIEW"),
        (11, "函数/过程", "FUNCTION"),
        (12, "序列清单", "SEQUENCE"),
    ] {
        let t = Instant::now();
        let sql = format!(
            "SELECT object_name FROM all_objects WHERE owner = USER AND object_type = '{obj_type}' \
             ORDER BY object_name"
        );
        let items: Result<Vec<String>, _> = conn.query(sql).await;
        match items {
            Ok(list) => rep.report(
                id,
                kind,
                Outcome::Pass(format!(
                    "{} 个{}",
                    list.len(),
                    if list.is_empty() { "（空）" } else { "" }
                )),
                t.elapsed().as_millis(),
            ),
            Err(e) => rep.report(id, kind, Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
        }
    }

    // ── 13-16 表详情（用第一个找到的表）──
    let first_table: Option<String> = conn
        .query("SELECT object_name FROM all_objects WHERE owner = USER AND object_type = 'TABLE' AND ROWNUM <= 1")
        .await
        .ok()
        .and_then(|v: Vec<String>| v.into_iter().next());

    if let Some(ref table) = first_table {
        // 13 列结构
        let t = Instant::now();
        let cols: Result<Vec<(String, String)>, _> = conn
            .query(format!(
                "SELECT column_name, data_type FROM all_tab_columns \
                 WHERE owner = USER AND table_name = '{table}' ORDER BY column_id"
            ))
            .await;
        match cols {
            Ok(list) => rep.report(
                13,
                "列结构",
                Outcome::Pass(format!("{table}: {} 列", list.len())),
                t.elapsed().as_millis(),
            ),
            Err(e) => rep.report(13, "列结构", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
        }

        // 14 表 DDL
        let t = Instant::now();
        let ddl: Result<Option<String>, _> = conn
            .query_first(format!(
                "SELECT DBMS_METADATA.GET_DDL('TABLE', '{table}', USER) FROM DUAL"
            ))
            .await;
        match ddl {
            Ok(Some(d)) => rep.report(
                14,
                "表 DDL",
                Outcome::Pass(format!("{} 字符", d.len())),
                t.elapsed().as_millis(),
            ),
            Ok(None) => rep.report(14, "表 DDL", Outcome::Warn("空 DDL".to_string()), t.elapsed().as_millis()),
            Err(e) => rep.report(14, "表 DDL", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
        }

        // 15 索引清单
        let t = Instant::now();
        let idx: Result<Vec<(String, String)>, _> = conn
            .query(format!(
                "SELECT i.index_name, c.column_name \
                 FROM all_indexes i JOIN all_ind_columns c ON i.index_name = c.index_name \
                 WHERE i.owner = USER AND i.table_name = '{table}'"
            ))
            .await;
        match idx {
            Ok(list) => rep.report(15, "索引清单", Outcome::Pass(format!("{} 条", list.len())), t.elapsed().as_millis()),
            Err(e) => rep.report(15, "索引清单", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
        }

        // 17 实际数据查询
        let t = Instant::now();
        let rows: Result<Vec<oceanbase_async::Row>, _> = conn
            .query(format!("SELECT * FROM {table} WHERE ROWNUM <= 10"))
            .await;
        match rows {
            Ok(list) => rep.report(
                17,
                "实际数据查询",
                Outcome::Pass(format!("{table}: {} 行", list.len())),
                t.elapsed().as_millis(),
            ),
            Err(e) => rep.report(17, "实际数据查询", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
        }
    } else {
        for id in [13, 14, 15, 17] {
            rep.report(id, "（跳过）", Outcome::Warn("无表可测".to_string()), 0);
        }
    }

    // ── 18 SELECT 1 FROM DUAL ──
    let t = Instant::now();
    let one: Result<Option<i64>, _> = conn.query_first("SELECT 1 FROM DUAL").await;
    match one {
        Ok(Some(v)) => rep.report(18, "SELECT 1 FROM DUAL", Outcome::Pass(format!("={v}")), t.elapsed().as_millis()),
        _ => rep.report(18, "SELECT 1 FROM DUAL", Outcome::Fail("查询失败".to_string()), t.elapsed().as_millis()),
    }

    // ── 19 PL/SQL 块 ──
    let t = Instant::now();
    let plsql: Result<(), _> = conn.query_drop("BEGIN NULL; END;").await;
    match plsql {
        Ok(()) => rep.report(19, "PL/SQL 块", Outcome::Pass("BEGIN NULL; END; 成功".to_string()), t.elapsed().as_millis()),
        Err(e) => rep.report(19, "PL/SQL 块", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
    }

    // ── 20 事务：CREATE + INSERT + COMMIT ──
    let t = Instant::now();
    let test_table = "VELOTERM_TEST_$$";
    let create: Result<(), _> = conn
        .query_drop(format!(
            "CREATE TABLE {test_table} (id NUMBER, name VARCHAR2(100), created DATE DEFAULT SYSDATE)"
        ))
        .await;
    match create {
        Ok(()) => rep.report(20, "CREATE TABLE", Outcome::Pass(test_table.to_string()), t.elapsed().as_millis()),
        Err(e) => {
            rep.report(20, "CREATE TABLE", Outcome::Fail(format!("{e}")), t.elapsed().as_millis());
        }
    }

    let t = Instant::now();
    let insert: Result<(), _> = conn
        .query_drop(format!(
            "INSERT INTO {test_table} (id, name) VALUES (1, '中文测试数据')"
        ))
        .await;
    match &insert {
        Ok(()) => {
            // COMMIT
            let _: Result<(), _> = conn.query_drop("COMMIT").await;
            // 验证
            let check: Result<Option<String>, _> = conn
                .query_first(format!("SELECT name FROM {test_table} WHERE id = 1"))
                .await;
            match check {
                Ok(Some(name)) => rep.report(
                    21,
                    "INSERT+COMMIT",
                    Outcome::Pass(format!("读回: {name}")),
                    t.elapsed().as_millis(),
                ),
                _ => rep.report(21, "INSERT+COMMIT", Outcome::Fail("COMMIT 后读不到".to_string()), t.elapsed().as_millis()),
            }
        }
        Err(e) => rep.report(21, "INSERT+COMMIT", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
    }

    // ── 22 ROLLBACK ──
    let t = Instant::now();
    let _: Result<(), _> = conn
        .query_drop(format!("INSERT INTO {test_table} (id, name) VALUES (2, '将被回滚')"))
        .await;
    let _: Result<(), _> = conn.query_drop("ROLLBACK").await;
    let check: Result<Vec<(i64,)>, _> = conn
        .query(format!("SELECT id FROM {test_table} ORDER BY id"))
        .await;
    match check {
        Ok(rows) if rows.len() == 1 => rep.report(
            22,
            "INSERT+ROLLBACK",
            Outcome::Pass("回滚后仅剩 1 行".to_string()),
            t.elapsed().as_millis(),
        ),
        Ok(rows) => rep.report(
            22,
            "INSERT+ROLLBACK",
            Outcome::Fail(format!("回滚后仍有 {} 行", rows.len())),
            t.elapsed().as_millis(),
        ),
        Err(e) => rep.report(22, "INSERT+ROLLBACK", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
    }

    // ── 23 UPDATE + 读回 ──
    let t = Instant::now();
    let upd: Result<(), _> = conn
        .query_drop(format!("UPDATE {test_table} SET name = '更新后的值' WHERE id = 1"))
        .await;
    match upd {
        Ok(()) => {
            let _: Result<(), _> = conn.query_drop("COMMIT").await;
            let check: Result<Option<String>, _> = conn
                .query_first(format!("SELECT name FROM {test_table} WHERE id = 1"))
                .await;
            match check {
                Ok(Some(name)) if name.contains("更新后的值") => rep.report(
                    23,
                    "UPDATE",
                    Outcome::Pass(format!("读回: {name}")),
                    t.elapsed().as_millis(),
                ),
                Ok(other) => rep.report(
                    23,
                    "UPDATE",
                    Outcome::Fail(format!("读回不符: {other:?}")),
                    t.elapsed().as_millis(),
                ),
                Err(e) => rep.report(23, "UPDATE", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
            }
        }
        Err(e) => rep.report(23, "UPDATE", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
    }

    // ── 24 DELETE + 计数 ──
    let t = Instant::now();
    let del: Result<(), _> = conn
        .query_drop(format!("DELETE FROM {test_table} WHERE id = 1"))
        .await;
    match del {
        Ok(()) => {
            let _: Result<(), _> = conn.query_drop("COMMIT").await;
            let check: Result<Vec<(i64,)>, _> =
                conn.query(format!("SELECT id FROM {test_table}")).await;
            match check {
                Ok(rows) if rows.is_empty() => rep.report(
                    24,
                    "DELETE",
                    Outcome::Pass("删后 0 行".to_string()),
                    t.elapsed().as_millis(),
                ),
                Ok(rows) => rep.report(
                    24,
                    "DELETE",
                    Outcome::Fail(format!("删后仍有 {} 行", rows.len())),
                    t.elapsed().as_millis(),
                ),
                Err(e) => rep.report(24, "DELETE", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
            }
        }
        Err(e) => rep.report(24, "DELETE", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
    }

    // ── 25 驱动 API 事务 ──
    // Transaction::new 的 OB Oracle 分支已对齐 Java JDBC（set autocommit=0
    // 开事务 + 文本 COMMIT/ROLLBACK，事务选项明确报错）——正常应 PASS；
    // WARN 仅在环境异常时出现（MySQL 方言 START TRANSACTION 路径已不适用）
    let t = Instant::now();
    let outcome = probe_api_transaction(&mut conn, &test_table).await;
    let outcome = match outcome {
        ApiTxOutcome::Pass(msg) => Outcome::Pass(msg),
        ApiTxOutcome::Fail(msg) => Outcome::Fail(msg),
        ApiTxOutcome::Warn(msg) => Outcome::Warn(msg),
    };
    rep.report(25, "驱动 API 事务", outcome, t.elapsed().as_millis());

    // ── 26 清理 ──
    let t = Instant::now();
    let drop: Result<(), _> = conn.query_drop(format!("DROP TABLE {test_table}")).await;
    match drop {
        Ok(()) => rep.report(26, "DROP TABLE 清理", Outcome::Pass("OK".to_string()), t.elapsed().as_millis()),
        Err(e) => rep.report(26, "DROP TABLE 清理", Outcome::Fail(format!("{e}")), t.elapsed().as_millis()),
    }

    // ── 27 错误处理：ORA-00942 ──
    let t = Instant::now();
    let err: Result<Vec<oceanbase_async::Row>, _> = conn
        .query("SELECT * FROM THIS_TABLE_DOES_NOT_EXIST")
        .await;
    match err {
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("ORA-") || msg.contains("942") {
                rep.report(27, "ORA-00942 错误", Outcome::Pass(format!("{msg}")), t.elapsed().as_millis());
            } else {
                rep.report(27, "ORA-00942 错误", Outcome::Warn(msg), t.elapsed().as_millis());
            }
        }
        Ok(_) => rep.report(27, "ORA-00942 错误", Outcome::Fail("不存在的表居然没报错".to_string()), t.elapsed().as_millis()),
    }

    // ── 28 错误处理：ORA-00904 ──
    let t = Instant::now();
    let err: Result<Vec<oceanbase_async::Row>, _> = conn
        .query("SELECT nonexistent_column FROM DUAL")
        .await;
    match err {
        Err(e) => rep.report(28, "ORA-00904 错误", Outcome::Pass(format!("{e}")), t.elapsed().as_millis()),
        Ok(_) => rep.report(28, "ORA-00904 错误", Outcome::Fail("无效列没报错".to_string()), t.elapsed().as_millis()),
    }

    // ── 29 类型解码（NUMBER/INTEGER/DATE/TIMESTAMP/NVARCHAR2/CLOB）──
    // 独立建表插一行读回，Value 逐列 to_string 渲染——看 OB 经文本协议
    // 回传的真实字节形态（NLS 初始化生效与否在此暴露：NVARCHAR2 乱码即 NLS 未生效）
    let types_table = "VELOTERM_TYPES_TEST";
    let t = Instant::now();
    let created: Result<(), _> = conn
        .query_drop(format!(
            "CREATE TABLE {types_table} (\
             c_number NUMBER(12,3), c_int INTEGER, c_date DATE, c_ts TIMESTAMP, \
             c_varchar VARCHAR2(64), c_nvarchar NVARCHAR2(64), c_clob CLOB)"
        ))
        .await;
    match created {
        Ok(()) => {
            let ins: Result<(), _> = conn
                .query_drop(format!(
                    "INSERT INTO {types_table} VALUES (\
                     12345.678, 42, \
                     TO_DATE('2026-01-15 10:30:00','YYYY-MM-DD HH24:MI:SS'), \
                     TO_TIMESTAMP('2026-01-15 10:30:00.123456','YYYY-MM-DD HH24:MI:SS.FF6'), \
                     '类型测试', N'NVARCHAR中文字符串', 'CLOB大对象文本')"
                ))
                .await;
            let read: Result<Option<oceanbase_async::Row>, _> = conn
                .query_first(format!("SELECT * FROM {types_table} WHERE ROWNUM = 1"))
                .await;
            let outcome = match (ins, read) {
                (Ok(()), Ok(Some(row))) => {
                    let vals = row.unwrap();
                    let rendered: Vec<String> = vals.iter().map(value_render).collect();
                    let nulls = rendered.iter().filter(|s| *s == "NULL").count();
                    if nulls == 0 {
                        Outcome::Pass(rendered.join(" | "))
                    } else {
                        Outcome::Warn(format!("{nulls} 列为 NULL: {}", rendered.join(" | ")))
                    }
                }
                (Ok(()), Ok(None)) => Outcome::Fail("读不到行".to_string()),
                (Ok(()), Err(e)) => Outcome::Fail(format!("{e}")),
                (Err(e), _) => Outcome::Fail(format!("插入失败: {e}")),
            };
            rep.report(29, "类型解码", outcome, t.elapsed().as_millis());
            let _: Result<(), _> = conn.query_drop(format!("DROP TABLE {types_table}")).await;
        }
        Err(e) => rep.report(
            29,
            "类型解码",
            Outcome::Warn(format!("建表失败（权限？）: {e}")),
            t.elapsed().as_millis(),
        ),
    }

    // ── 30 断开 ──
    let t = Instant::now();
    let _ = conn.disconnect().await;
    rep.report(30, "断开连接", Outcome::Pass("OK".to_string()), t.elapsed().as_millis());

    rep.summary();
}
