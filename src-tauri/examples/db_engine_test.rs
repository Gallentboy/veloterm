//! 数据库工作台全链路真机测试 CLI（Oracle / 达梦）。
//!
//! 与 ob_oracle_test 同型（PASS/WARN/FAIL 逐项 + 汇总退出码），但驱动的是
//! **app 层真实代码路径**（db_browser::DbEngine 的目录/定义/表详情/执行
//! 计划全套），不是驱动层桩——通过即工作台可用。某步失败不中断后续
//! （除非连接本身失败），给完整诊断全景。
//!
//! 用法：
//! ```bash
//! # Oracle
//! cargo run --example db_engine_test -- oracle \
//!     --host 127.0.0.1 --port 1521 --user scott --pass tiger \
//!     --service XEPDB1 [--schema SCOTT] [--table T1]
//! # 达梦
//! cargo run --example db_engine_test -- dameng \
//!     --host 127.0.0.1 --port 5236 --user SYSDBA --pass SYSDBA001 \
//!     [--schema SYSDBA] [--table T1] [--dm-compat oracle|mysql]
//! ```
//!
//! `--schema` 缺省取登录用户名原样；`--table` 缺省取对象树第一张表。
//! 达梦的表空间/用户/角色是实例级组（挂 SYS schema），CLI 自动切 SYS 测。

use std::sync::{Arc, Mutex};
use std::time::Instant;

use app_lib::db::DbConnectionRecord;
use app_lib::db_browser::engines::{DbEngine, ObjectGroup};
use app_lib::db_browser::{explain, explain_core, sqlsplit};

// ── 参数与报告器 ─────────────────────────────────────────────────────────

struct Args {
    engine: String,
    host: String,
    port: u16,
    user: String,
    pass: String,
    service: Option<String>,
    schema: Option<String>,
    table: Option<String>,
    dm_compat: String,
}

fn parse_args() -> Result<Args, String> {
    let mut argv = std::env::args().skip(1);
    let engine = argv
        .next()
        .ok_or("缺引擎参数：oracle | dameng")?;
    if engine != "oracle" && engine != "dameng" {
        return Err(format!("未知引擎 {engine}（支持 oracle | dameng）"));
    }
    let mut a = Args {
        engine,
        host: String::new(),
        port: 0,
        user: String::new(),
        pass: String::new(),
        service: None,
        schema: None,
        table: None,
        dm_compat: "oracle".into(),
    };
    let mut it = argv.peekable();
    while let Some(flag) = it.next() {
        let val = it
            .next()
            .ok_or_else(|| format!("参数 {flag} 缺值"))?;
        match flag.as_str() {
            "--host" => a.host = val,
            "--port" => a.port = val.parse().map_err(|_| "端口非数字")?,
            "--user" => a.user = val,
            "--pass" => a.pass = val,
            "--service" => a.service = Some(val),
            "--schema" => a.schema = Some(val),
            "--table" => a.table = Some(val),
            "--dm-compat" => a.dm_compat = val,
            other => return Err(format!("未知参数 {other}")),
        }
    }
    if a.host.is_empty() || a.port == 0 || a.user.is_empty() {
        return Err("必填：--host --port --user [--pass]".into());
    }
    if a.engine == "oracle" && a.service.is_none() {
        return Err("Oracle 必填 --service（服务名）".into());
    }
    Ok(a)
}

enum Outcome {
    Pass(String),
    Warn(String),
    Fail(String),
}

struct Reporter {
    pass: usize,
    warn: usize,
    fail: usize,
    id: usize,
    started: Instant,
}

impl Reporter {
    fn new() -> Self {
        Self {
            pass: 0,
            warn: 0,
            fail: 0,
            id: 0,
            started: Instant::now(),
        }
    }

    fn report(&mut self, name: &str, outcome: Outcome, elapsed_ms: u128) {
        self.id += 1;
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
        println!("[{tag}] {:02} {name:<24} ({elapsed_ms}ms) {msg}", self.id);
    }

    fn summary(&self) -> bool {
        println!(
            "\n══ 汇总：{} 通过 · {} 警告 · {} 失败（总耗时 {}ms）══",
            self.pass,
            self.warn,
            self.fail,
            self.started.elapsed().as_millis()
        );
        self.fail == 0
    }
}

/// 一步测试：跑闭包 → 报告（不中断）
async fn step<F, Fut>(r: &mut Reporter, name: &str, f: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Outcome>,
{
    let t = Instant::now();
    let outcome = f().await;
    r.report(name, outcome, t.elapsed().as_millis());
}

// ── 辅助 ─────────────────────────────────────────────────────────────────

/// exec_streaming 收集（列头 + 行数；与 db_explain 的 exec_collect 同型）
async fn query(
    engine: &mut DbEngine,
    cancel: &Arc<tokio::sync::Notify>,
    sql: &str,
) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    let columns: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let rows: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
    let cols_sink = Arc::clone(&columns);
    let rows_sink = Arc::clone(&rows);
    let summary = engine
        .exec_streaming(
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
        )
        .await?;
    if summary.cancelled {
        return Err("意外取消".into());
    }
    let columns = columns.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let rows = rows.lock().unwrap_or_else(|p| p.into_inner()).clone();
    Ok((columns, rows))
}

/// 结构化计划树连通性校验 + 首层预览
fn check_plan(plan: &explain::ExplainPlan) -> Result<String, String> {
    if plan.nodes.is_empty() {
        return Err("计划节点为空".into());
    }
    let ids: std::collections::HashSet<i64> = plan.nodes.iter().map(|n| n.id).collect();
    let orphans = plan
        .nodes
        .iter()
        .filter(|n| n.parent_id.is_some_and(|p| !ids.contains(&p)))
        .count();
    let roots = plan.nodes.iter().filter(|n| n.parent_id.is_none()).count();
    let titles: Vec<&str> = plan
        .nodes
        .iter()
        .take(4)
        .map(|n| n.title.as_str())
        .collect();
    if roots == 0 {
        return Err("计划无根节点".into());
    }
    Ok(format!(
        "{} 节点（根 {} · 孤儿 {}）：{}",
        plan.nodes.len(),
        roots,
        orphans,
        titles.join(" → ")
    ))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("参数错误：{e}\n\n用法：cargo run --example db_engine_test -- oracle|dameng --host H --port P --user U --pass W [--service S] [--schema S] [--table T] [--dm-compat oracle|mysql]");
            std::process::exit(2);
        }
    };
    let dialect_key = if args.engine == "dameng" {
        format!("dameng:{}", args.dm_compat)
    } else {
        "oracle".to_string()
    };
    println!(
        "══ db_engine_test：{} @ {}:{}（dialect={dialect_key}）══",
        args.engine, args.host, args.port
    );

    let mut r = Reporter::new();

    // 01 连接
    let record = DbConnectionRecord {
        id: 0,
        name: "cli-test".into(),
        engine: args.engine.clone(),
        host: args.host.clone(),
        port: args.port,
        database: None,
        service_name: args.service.clone(),
        mode: None,
        db_path: None,
        encryption_mode: "none".into(),
        username: Some(args.user.clone()),
        password: Some(args.pass.clone()),
        ssl_mode: "disable".into(),
        ssh_host_id: None,
        env_level: "dev".into(),
        readonly: false,
        color: None,
        compatible_mode: if args.engine == "dameng" {
            Some(args.dm_compat.clone())
        } else {
            None
        },
        extra_options: None,
        created_at: 0,
        updated_at: 0,
    };
    let connect_start = Instant::now();
    let connected = DbEngine::connect(&record).await;
    let (mut engine, version) = match connected {
        Ok(v) => v,
        Err(e) => {
            r.report(
                "connect",
                Outcome::Fail(e.clone()),
                connect_start.elapsed().as_millis(),
            );
            r.summary();
            eprintln!("\n连接失败，中止：{e}");
            std::process::exit(1);
        }
    };
    r.report("connect", Outcome::Pass(version), connect_start.elapsed().as_millis());

    let cancel: Arc<tokio::sync::Notify> = Arc::new(tokio::sync::Notify::new());

    // 02 库/schema 清单
    // Oracle 的 owner 名在字典里恒为大写（未加引号创建的用户）；应用前端
    // 从 all_users 拿的是正确大小写，CLI 缺省对 Oracle 大写化免得查空
    let mut schema = args.schema.clone().unwrap_or_else(|| {
        if args.engine == "oracle" {
            args.user.to_uppercase()
        } else {
            args.user.clone()
        }
    });
    step(&mut r, "list_databases", || async {
        match engine.list_databases().await {
            Ok(list) if !list.is_empty() => {
                let shown: Vec<String> = list.iter().take(5).cloned().collect();
                Outcome::Pass(format!("{} 个（{}…）", list.len(), shown.join(",")))
            }
            Ok(_) => Outcome::Warn("清单为空".into()),
            Err(e) => Outcome::Fail(e),
        }
    })
    .await;

    // 03 切 schema（Oracle/达梦 = 会话级 SET SCHEMA）
    step(&mut r, "select_database", || async {
        match engine.select_database(&schema).await {
            Ok(()) => Outcome::Pass(format!("SET SCHEMA {schema}")),
            Err(e) => Outcome::Fail(e),
        }
    })
    .await;

    // 04 对象树分组计数
    let mut table_count = 0usize;
    step(&mut r, "catalog_groups", || async {
        match engine.catalog_groups(&schema).await {
            Ok(groups) => {
                let names: Vec<String> = groups
                    .iter()
                    .map(|g| format!("{}×{}", g.name, g.detail.as_deref().unwrap_or("?")))
                    .collect();
                if let Some(t) = groups
                    .iter()
                    .find(|g| g.name == "TABLE")
                    .and_then(|g| g.detail.clone())
                    .and_then(|d| d.parse().ok())
                {
                    table_count = t;
                }
                if table_count > 0 {
                    Outcome::Pass(names.join(" "))
                } else {
                    Outcome::Warn(format!("TABLE 组为空（{}）", names.join(" ")))
                }
            }
            Err(e) => Outcome::Fail(e),
        }
    })
    .await;

    // 05 表清单 + 选定测试表
    let mut test_table = args.table.clone().unwrap_or_default();
    step(&mut r, "catalog_objects(table)", || async {
        match engine.catalog_objects(&schema, ObjectGroup::Table).await {
            Ok(objs) if !objs.is_empty() => {
                if test_table.is_empty() {
                    test_table = objs
                        .first()
                        .map(|o| o.name.clone())
                        .unwrap_or_default();
                }
                Outcome::Pass(format!("{} 张，测试表 {test_table}", objs.len()))
            }
            Ok(_) => Outcome::Fail("表清单为空（后续表相关步骤将失败）".into()),
            Err(e) => Outcome::Fail(e),
        }
    })
    .await;

    // 06 列清单（树第三层）
    let table_for_cols = test_table.clone();
    let schema_for_cols = schema.clone();
    step(&mut r, "catalog_columns", || async {
        if table_for_cols.is_empty() {
            return Outcome::Warn("无测试表，跳过".into());
        }
        match engine.catalog_columns(&schema_for_cols, &table_for_cols).await {
            Ok(cols) if !cols.is_empty() => Outcome::Pass(format!("{table_for_cols}：{} 列", cols.len())),
            Ok(_) => Outcome::Fail("列清单为空".into()),
            Err(e) => Outcome::Fail(e),
        }
    })
    .await;

    // 07 对象定义扫描：各适用组取首个对象拉定义
    let def_groups: Vec<ObjectGroup> = if args.engine == "oracle" {
        vec![
            ObjectGroup::View,
            ObjectGroup::Function,
            ObjectGroup::Procedure,
            ObjectGroup::Sequence,
            ObjectGroup::Trigger,
            ObjectGroup::Synonym,
            ObjectGroup::Package,
            ObjectGroup::DbLink,
            ObjectGroup::RecycleBin,
            ObjectGroup::DataType,
        ]
    } else {
        vec![
            ObjectGroup::View,
            ObjectGroup::Function,
            ObjectGroup::Procedure,
            ObjectGroup::Sequence,
            ObjectGroup::Trigger,
            ObjectGroup::Synonym,
            ObjectGroup::Package,
            ObjectGroup::RecycleBin,
            // 实例级组：达梦挂 SYS schema 下
            ObjectGroup::Tablespace,
            ObjectGroup::DmUser,
            ObjectGroup::DmRole,
        ]
    };
    for group in def_groups {
        let g_schema = if args.engine == "dameng"
            && matches!(group, ObjectGroup::Tablespace | ObjectGroup::DmUser | ObjectGroup::DmRole)
        {
            "SYS".to_string()
        } else {
            schema.clone()
        };
        let name = format!("definition:{}", group.as_str());
        step(&mut r, &name, || async {
            match engine.catalog_objects(&g_schema, group).await {
                Ok(objs) if !objs.is_empty() => {
                    let Some(first) = objs.first() else {
                        return Outcome::Fail("清单非空但取首对象失败".into());
                    };
                    let sample: Vec<&str> = objs
                        .iter()
                        .take(3)
                        .map(|o| o.name.as_str())
                        .collect();
                    match engine
                        .object_definition(&g_schema, &first.name, group.as_str())
                        .await
                    {
                        Ok(def) if !def.definition.trim().is_empty() => Outcome::Pass(format!(
                            "{}（{} 字符）",
                            first.name,
                            def.definition.len()
                        )),
                        Ok(_) => Outcome::Fail(format!("{}：定义为空", first.name)),
                        Err(e) => {
                            // 系统内置对象无源码是常态（SYSDBA 的 SP_* 备份/
                            // 维护过程不对外暴露源码）：降为 WARN 不记失败
                            if e.contains("均无源码") {
                                Outcome::Warn(format!("{}：{e}", first.name))
                            } else {
                                Outcome::Fail(format!(
                                    "{}：{e}（同组样本：{}）",
                                    first.name,
                                    sample.join(", ")
                                ))
                            }
                        }
                    }
                }
                // 组为空是常态（如回收站/DB Link 未使用）：警告不失败
                Ok(_) => Outcome::Warn("组为空，跳过".into()),
                Err(e) => Outcome::Fail(e),
            }
        })
        .await;
    }

    // 08 表详情（DDL/列/索引/约束/反向引用/注释/大小/行数）
    let schema_for_detail = schema.clone();
    let table_for_detail = test_table.clone();
    step(&mut r, "table_detail", || async {
        if table_for_detail.is_empty() {
            return Outcome::Warn("无测试表，跳过".into());
        }
        match engine
            .table_detail(&schema_for_detail, &table_for_detail, "table")
            .await
        {
            Ok(d) => {
                let mut parts = vec![
                    format!("DDL {}B", d.ddl.len()),
                    format!("列 {}", d.columns.len()),
                    format!("索引 {}", d.indexes.len()),
                ];
                if !d.constraint_headers.is_empty() {
                    parts.push(format!("约束 {}", d.constraints.len()));
                }
                if !d.reference_headers.is_empty() {
                    parts.push(format!(
                        "反向外键 {}（{}）",
                        d.references.len(),
                        d.reference_headers.join("/")
                    ));
                }
                if !d.comment.is_empty() {
                    parts.push(format!("注释「{}」", d.comment));
                }
                if !d.size.is_empty() || !d.row_count.is_empty() {
                    parts.push(format!("{} / {} 行", d.size, d.row_count));
                }
                if d.columns.is_empty() {
                    Outcome::Fail(format!("{}：列结构为空（{}）", table_for_detail, parts.join(" · ")))
                } else {
                    Outcome::Pass(parts.join(" · "))
                }
            }
            Err(e) => Outcome::Fail(e),
        }
    })
    .await;

    // 09 执行计划（结构化节点树；Oracle 走 PLAN_TABLE 探测 + 两段式，
    // 同时回归 case-6 位图 / case-19 流控 / 伪步骤剔除等驱动修复点）
    let explain_schema = schema.clone();
    let table_for_explain = test_table.clone();
    let engine_kind = args.engine.clone();
    let dk = dialect_key.clone();
    step(&mut r, "explain", || async {
        if table_for_explain.is_empty() {
            return Outcome::Warn("无测试表，跳过".into());
        }
        let sql = format!(
            // COUNT(*) 规避 ROWNUM 依赖（达梦 mysql 兼容模式可能不认
            // ROWNUM）；EXPLAIN 只做优化不执行，无全表风险
            "SELECT COUNT(*) FROM \"{explain_schema}\".\"{table_for_explain}\""
        );
        let stmts = sqlsplit::split_statements(&dk, &sql);
        if stmts.is_empty() {
            return Outcome::Fail("语句切分为空".into());
        }
        match explain_core(
            &mut engine,
            &cancel,
            &engine_kind,
            None,
            &stmts,
            false,
        )
        .await
        {
            Ok(plans) => match plans.first() {
                Some(p) => match check_plan(p) {
                    Ok(msg) => Outcome::Pass(msg),
                    Err(e) => Outcome::Fail(e),
                },
                None => Outcome::Fail("无计划结果".into()),
            },
            Err(e) => Outcome::Fail(e),
        }
    })
    .await;

    // 10 多语句流式（与 db_query 同款：先 sqlsplit 切分再逐条 exec——
    // Oracle 不吃带分号的多语句原文，ORA-00911）
    let dk_multi = dialect_key.clone();
    step(&mut r, "query_multi", || async {
        let stmts = sqlsplit::split_statements(
            &dk_multi,
            "SELECT 1 AS V FROM DUAL; SELECT 2 AS V FROM DUAL",
        );
        if stmts.len() != 2 {
            return Outcome::Fail(format!("切分出 {} 条（应为 2）", stmts.len()));
        }
        let mut vals: Vec<String> = Vec::new();
        for s in &stmts {
            match query(&mut engine, &cancel, &s.sql).await {
                Ok((_cols, rows)) => {
                    vals.push(
                        rows.first()
                            .and_then(|r| r.first().cloned())
                            .unwrap_or_default(),
                    );
                }
                Err(e) => return Outcome::Fail(e),
            }
        }
        Outcome::Pass(format!("两条各 1 行：[{}]", vals.join(",")))
    })
    .await;

    // 11 长中文 SQL（case-19 流控 + 位图回归：响应流含中文列名与多行数据）
    step(&mut r, "chinese_long_sql", || async {
        let long = "中文列值回归测试执行计划位图与流控路径".repeat(30);
        let sql = format!(
            "SELECT '{long}' AS 中文列名别名 FROM DUAL UNION ALL \
             SELECT '{long}' AS 中文列名别名 FROM DUAL"
        );
        match query(&mut engine, &cancel, &sql).await {
            Ok((cols, rows))
                if cols.first().is_some_and(|c| c.contains("中文列名别名"))
                    && rows.len() == 2 =>
            {
                let name = cols.first().cloned().unwrap_or_default();
                Outcome::Pass(format!("列「{name}」· {} 行", rows.len()))
            }
            Ok((cols, rows)) => {
                Outcome::Fail(format!("列 {:?} 行数 {}", cols, rows.len()))
            }
            Err(e) => Outcome::Fail(e),
        }
    })
    .await;

    // 12 只读白名单纯断言（sqlsplit：EXPLAIN/SELECT 放行、DML 拒绝）
    step(&mut r, "readonly_guard", || async {
        let ok = |sql: &str, want: bool| {
            let stmts = sqlsplit::split_statements(&dk, sql);
            stmts.iter().all(|s| sqlsplit::statement_is_readonly(&dk, s)) == want
        };
        let (a, b, c) = (
            ok("SELECT 1 FROM DUAL", true),
            ok("EXPLAIN SELECT 1 FROM DUAL", true),
            ok("DELETE FROM NO_SUCH_TABLE", false),
        );
        if a && b && c {
            Outcome::Pass("SELECT/EXPLAIN 放行 · DELETE 拒绝".into())
        } else {
            Outcome::Fail(format!("判定异常 select={a} explain={b} delete={c}"))
        }
    })
    .await;

    // 13 关闭
    step(&mut r, "close", || async {
        engine.close().await;
        Outcome::Pass(String::new())
    })
    .await;

    let ok = r.summary();
    std::process::exit(if ok { 0 } else { 1 });
}
