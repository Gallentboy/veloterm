//! catalog_groups 的引擎分发：分组计数（表/视图/函数/过程/序列 + 各引擎特有组）。
//! 拆自 engines/mod.rs 的同名方法，行为零变更。

use oracle_async::Queryable as _;

use super::super::{
    CatalogNode, DbEngine, GroupCounts, OB_DICT_PREFIX, ObjectGroup, dm_row_i64, REDIS_VALUE_TYPES, dm_query_rows, dm_row_string,
    group_nodes, ob_mysql_mviews, ob_mysql_sequences, ob_oracle_mviews, ob_oracle_packages,
    ob_oracle_tables, ob_quote_literal, oracle_count_opt, oracle_dict_prefix, oracle_err,
    oracle_row_string, oracle_scalar_string, parse_redis_db, quote_ident, redis_scan_typed_keys,
    redis_select,
};

pub(crate) async fn groups(
    engine: &mut DbEngine,
    schema: &str,
) -> Result<Vec<CatalogNode>, String> {
    match engine {
            DbEngine::MySql(conn) => {
                // OB Oracle：all_objects 聚合（与原生 Oracle 臂同源；BIN$
                // 回收站对象不计数），表与包单独取更权威的字典：
                // - 表走 all_tables（all_objects 会把外部表混进表分组）
                // - 包合并 PACKAGE 与 PACKAGE BODY 并按名去重（只有包体、
                //   没有包声明的包在只看 PACKAGE 时会漏）
                // OB 无 DBA_ 体系故恒 all_*
                if conn.ob_is_oracle_mode() {
                    use mysql_async::prelude::Queryable as _;
                    let sql = format!(
                        "SELECT object_type, COUNT(*) FROM {OB_DICT_PREFIX}_objects \
                         WHERE owner = {} AND object_name NOT LIKE 'BIN$%' \
                           AND object_type IN ('VIEW','FUNCTION','PROCEDURE',\
                             'SEQUENCE','TRIGGER','SYNONYM','TYPE') \
                         GROUP BY object_type",
                        ob_quote_literal(schema)
                    );
                    let counts: Vec<(String, i64)> = conn
                        .exec(sql, ())
                        .await
                        .map_err(|e| format!("OceanBase 查询对象计数失败：{e}"))?;
                    let count_of = |kind: &str| {
                        counts
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(kind))
                            .map(|(_, c)| *c)
                            .unwrap_or(0)
                    };
                    let table_count = ob_oracle_tables(conn, schema).await?.len() as i64;
                    let package_count = ob_oracle_packages(conn, schema).await?.len() as i64;
                    let mview_count = ob_oracle_mviews(conn, schema).await.len() as i64;
                    return Ok(group_nodes(
                        schema,
                        GroupCounts {
                            triggers: Some(count_of("TRIGGER")),
                            synonyms: Some(count_of("SYNONYM")),
                            packages: Some(package_count),
                            mviews: Some(mview_count),
                            datatypes: Some(count_of("TYPE")),
                            ..GroupCounts::base(
                                table_count,
                                count_of("VIEW"),
                                count_of("FUNCTION"),
                                count_of("PROCEDURE"),
                                count_of("SEQUENCE"),
                            )
                        },
                    ));
                }
                use mysql_async::prelude::Queryable as _;
                // OB MySQL 租户：物化视图的容器表 `__MV_CONTAINER_*` 与日志表
                // `MLOG$_*` 是存储实现而非用户对象，计数时排除（LEFT 前缀比较
                // 避免 LIKE 里 `_` 被当通配符）；原生 MySQL 计数口径不变
                let ob_mysql = conn.ob_compat_mode() == Some(mysql_async::ObCompatMode::MySql);
                let tables_sql = if ob_mysql {
                    "SELECT table_type, COUNT(*) FROM information_schema.tables \
                     WHERE table_schema = ? AND LEFT(table_name, 15) <> '__MV_CONTAINER_' \
                       AND LEFT(table_name, 6) <> 'MLOG$_' GROUP BY table_type"
                } else {
                    "SELECT table_type, COUNT(*) FROM information_schema.tables \
                     WHERE table_schema = ? GROUP BY table_type"
                };
                let tables: Vec<(String, i64)> = conn
                    .exec(tables_sql, (schema,))
                    .await
                    .map_err(|e| format!("MySQL 查询表清单失败：{e}"))?;
                let routines: Vec<(String, i64)> = conn
                    .exec(
                        "SELECT routine_type, COUNT(*) FROM information_schema.routines \
                         WHERE routine_schema = ? GROUP BY routine_type",
                        (schema,),
                    )
                    .await
                    .map_err(|e| format!("MySQL 查询例程清单失败：{e}"))?;
                // 序列：OB MySQL 租户走专有语法 SHOW SEQUENCES（OB 的
                // information_schema 没有 sequences 表，旧口径恒 0）；其余只在
                // MariaDB 上查 information_schema.sequences（MySQL 全系没有该表，
                // 先按分支判定，避免每次展开都发一条注定失败的查询）
                let sequences: i64 = if let Some(names) = ob_mysql_sequences(conn, schema).await {
                    names.len() as i64
                } else if conn.is_mariadb() {
                    conn.exec_first::<i64, _, _>(
                        "SELECT COUNT(*) FROM information_schema.sequences \
                         WHERE sequence_schema = ?",
                        (schema,),
                    )
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or(0)
                } else {
                    0
                };
                // 触发器：DBeaver TriggerCache 同源（TRIGGER_SCHEMA 过滤）
                let triggers: i64 = conn
                    .exec_first::<i64, _, _>(
                        "SELECT COUNT(*) FROM information_schema.triggers \
                         WHERE trigger_schema = ?",
                        (schema,),
                    )
                    .await
                    .unwrap_or(Some(0))
                    .unwrap_or(0);
                // 事件（MySQL Event Scheduler；DBeaver EventCache 同源）。
                // 老版本 MySQL 无 events 表 → 静默 0
                let events: i64 = conn
                    .exec_first::<i64, _, _>(
                        "SELECT COUNT(*) FROM information_schema.events \
                         WHERE event_schema = ?",
                        (schema,),
                    )
                    .await
                    .unwrap_or(Some(0))
                    .unwrap_or(0);
                let count_of = |kind: &str, rows: &[(String, i64)]| {
                    rows.iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(kind))
                        .map(|(_, c)| *c)
                        .unwrap_or(0)
                };
                // 物化视图：仅 OB MySQL 租户（OCEANBASE.DBA_MVIEWS，≥4.3.5.2；
                // 低版本该视图不存在 → None，分组不出现）。原生 MySQL 无 MView
                let mviews = if ob_mysql {
                    ob_mysql_mviews(conn, schema).await.map(|v| v.len() as i64)
                } else {
                    None
                };
                Ok(group_nodes(
                    schema,
                    GroupCounts {
                        triggers: Some(triggers),
                        events: Some(events),
                        mviews,
                        ..GroupCounts::base(
                            count_of("BASE TABLE", &tables),
                            count_of("VIEW", &tables),
                            count_of("FUNCTION", &routines),
                            count_of("PROCEDURE", &routines),
                            sequences,
                        )
                    },
                ))
            }
            DbEngine::Postgres(client) => {
                // DBeaver PostgreSchema 同源：pg_class 按 relkind 分组
                // （r=表/v=视图/m=物化视图/S=序列/f=外部表），触发器走 pg_trigger
                //（not tgisinternal 过滤自动创建的内部触发器）。不用
                // information_schema——它看不到物化视图，且 DBeaver 也不用。
                let relkind: Vec<(String, i64)> = client
                    .query(
                        "SELECT c.relkind::char::text, COUNT(*) \
                         FROM pg_catalog.pg_class c \
                         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = $1 AND c.relkind IN ('r','v','m','S','f') \
                         GROUP BY c.relkind",
                        &[&schema],
                    )
                    .await
                    .map_err(|e| format!("PostgreSQL 查询关系清单失败：{e}"))?
                    .iter()
                    .filter_map(|row| {
                        Some((
                            row.try_get::<_, String>(0).ok()?,
                            row.try_get::<_, i64>(1).ok()?,
                        ))
                    })
                    .collect();
                let routines: Vec<(String, i64)> = client
                    .query(
                        "SELECT p.prokind::char::text, COUNT(*) \
                         FROM pg_catalog.pg_proc p \
                         JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
                         WHERE n.nspname = $1 AND p.prokind IN ('f','p') \
                         GROUP BY p.prokind",
                        &[&schema],
                    )
                    .await
                    .map_err(|e| format!("PostgreSQL 查询例程清单失败：{e}"))?
                    .iter()
                    .filter_map(|row| {
                        Some((
                            row.try_get::<_, String>(0).ok()?,
                            row.try_get::<_, i64>(1).ok()?,
                        ))
                    })
                    .collect();
                let triggers: i64 = client
                    .query_opt(
                        "SELECT COUNT(*) FROM pg_catalog.pg_trigger t \
                         JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
                         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = $1 AND NOT t.tgisinternal",
                        &[&schema],
                    )
                    .await
                    .ok()
                    .flatten()
                    .and_then(|row| row.try_get::<_, i64>(0).ok())
                    .unwrap_or(0);
                // 规则（DBeaver PostgreTableReal.RuleCache 同源）：
                // pg_rewrite 排除视图内部规则 _RETURN
                let rules: i64 = client
                    .query_opt(
                        "SELECT COUNT(*) FROM pg_catalog.pg_rewrite r \
                         JOIN pg_catalog.pg_class c ON c.oid = r.ev_class \
                         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = $1 AND r.rulename <> '_RETURN'",
                        &[&schema],
                    )
                    .await
                    .ok()
                    .flatten()
                    .and_then(|row| row.try_get::<_, i64>(0).ok())
                    .unwrap_or(0);
                // 扩展（DBeaver PostgreSchema.ExtensionCache 同源）：
                // pg_extension 按 extnamespace 归属 schema；查询失败容错为 0
                //（极老版本无该表）
                let extensions: i64 = client
                    .query_opt(
                        "SELECT COUNT(*) FROM pg_catalog.pg_extension e \
                         JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace \
                         WHERE n.nspname = $1",
                        &[&schema],
                    )
                    .await
                    .ok()
                    .flatten()
                    .and_then(|row| row.try_get::<_, i64>(0).ok())
                    .unwrap_or(0);
                let count_of = |kind: &str, rows: &[(String, i64)]| {
                    rows.iter()
                        .find(|(k, _)| *k == kind)
                        .map(|(_, c)| *c)
                        .unwrap_or(0)
                };
                    Ok(group_nodes(
                        schema,
                        GroupCounts {
                            triggers: Some(triggers),
                            mviews: Some(count_of("m", &relkind)),
                            foreign_tables: Some(count_of("f", &relkind)),
                            extensions: Some(extensions),
                            rules: Some(rules),
                        ..GroupCounts::base(
                            count_of("r", &relkind),
                            count_of("v", &relkind),
                            count_of("f", &routines),
                            count_of("p", &routines),
                            count_of("S", &relkind),
                        )
                    },
                ))
            }
            DbEngine::Oracle(conn) => {
                // 对象在 all_objects 一表内聚合（SEQUENCE/TRIGGER 也在；DBeaver
                // OracleSchema 同源，加 TRIGGER 类型）；BIN$ 前缀 = 回收站不计数。
                // 字典视图前缀自适应（DBA 权限 → dba_*）
                let dict_p = oracle_dict_prefix(conn).await;
                let sql = format!(
                    "SELECT object_type, COUNT(*) FROM {dict_p}_objects \
                     WHERE owner = {} AND object_name NOT LIKE 'BIN$%' \
                       AND object_type IN \
                         ('TABLE','VIEW','FUNCTION','PROCEDURE','SEQUENCE','TRIGGER','SYNONYM','PACKAGE','TYPE') \
                     GROUP BY object_type",
                    ob_quote_literal(schema)
                );
                let mut rs = conn
                    .query(&sql)
                    .await
                    .map_err(|e| format!("Oracle 查询对象计数失败：{}", oracle_err(e)))?;
                let mut counts: Vec<(String, i64)> = Vec::new();
                while let Some(row) = rs
                    .next_row()
                    .await
                    .map_err(|e| format!("Oracle 读取对象计数失败：{}", oracle_err(e)))?
                {
                    let k = oracle_row_string(&row, 0);
                    let c = row.get::<i64>(1).unwrap_or(0);
                    counts.push((k, c));
                }
                let count_of = |kind: &str| {
                    counts
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(kind))
                        .map(|(_, c)| *c)
                        .unwrap_or(0)
                };
                // DB Link（all_db_links 按 owner；PUBLIC 链接不归入单 schema）
                let db_link_sql = format!(
                    "SELECT COUNT(*) FROM {dict_p}_db_links WHERE owner = {}",
                    ob_quote_literal(schema)
                );
                let db_links = oracle_count_opt(conn, &db_link_sql).await.unwrap_or(0);
                // 回收站（DBeaver Recycle Bin 同义）：dba_recyclebin 按 owner
                // 计数；普通用户无 dba 视图权限时降级 user_recyclebin（只含登录
                // 用户对象，故仅当展开的 schema 即登录用户时计入）
                let rb_dba_sql = format!(
                    "SELECT COUNT(*) FROM dba_recyclebin WHERE owner = {}",
                    ob_quote_literal(schema)
                );
                let recyclebin = match oracle_count_opt(conn, &rb_dba_sql).await {
                    Some(c) => c,
                    None => {
                        let login = oracle_scalar_string(conn, "SELECT USER FROM dual").await;
                        if !login.is_empty() && login.eq_ignore_ascii_case(schema) {
                            oracle_count_opt(conn, "SELECT COUNT(*) FROM user_recyclebin")
                                .await
                                .unwrap_or(0)
                        } else {
                            0
                        }
                    }
                };
                Ok(group_nodes(
                    schema,
                    GroupCounts {
                        triggers: Some(count_of("TRIGGER")),
                        synonyms: Some(count_of("SYNONYM")),
                        packages: Some(count_of("PACKAGE")),
                        db_links: Some(db_links),
                        recyclebin: Some(recyclebin),
                        datatypes: Some(count_of("TYPE")),
                        ..GroupCounts::base(
                            count_of("TABLE"),
                            count_of("VIEW"),
                            count_of("FUNCTION"),
                            count_of("PROCEDURE"),
                            count_of("SEQUENCE"),
                        )
                    },
                ))
            }
            DbEngine::Sqlite(_) => {
                // SQLite 只有表/视图两类（无函数/过程/序列）：计数取
                // sqlite_master 的 type 分组，sqlite_% 前缀是内部表不计数
                let schema_o = schema.to_string();
                engine.with_sqlite(move |conn| {
                    let schema_q = quote_ident(&schema_o)?;
                    let sql = format!(
                        "SELECT type, COUNT(*) FROM {schema_q}.sqlite_master \
                         WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' \
                         GROUP BY type"
                    );
                    let counts: Vec<(String, i64)> = {
                        let mut stmt = conn
                            .prepare(&sql)
                            .map_err(|e| format!("SQLite 查询对象计数失败：{e}"))?;
                        let mut rows = stmt
                            .query_map([], |row| {
                                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                            })
                            .map_err(|e| format!("SQLite 查询对象计数失败：{e}"))?;
                        let mut out: Vec<(String, i64)> = Vec::new();
                        for row in rows.by_ref() {
                            out.push(row.map_err(|e| format!("SQLite 读取对象计数失败：{e}"))?);
                        }
                        out
                    };
                    let count_of = |kind: &str| {
                        counts
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(kind))
                            .map(|(_, c)| *c)
                            .unwrap_or(0)
                    };
                    Ok(group_nodes(
                        &schema_o,
                        GroupCounts::base(
                            count_of("table"),
                            count_of("view"),
                            0,
                            0,
                            0,
                        ),
                    ))
                })
                .await
            }
            DbEngine::Redis { conn, db, .. } => {
                // 逻辑库 → 值类型分组：SCAN 抓 key 再批量 TYPE 归类。key 上限
                // 5000（工作台是浏览用途，全库枚举在百万 key 实例上会把 UI
                // 拖死）；分组计数是「已扫描范围内」的口径，不是 DBSIZE。
                let target = parse_redis_db(schema)
                    .ok_or_else(|| format!("无效的 Redis 逻辑库：{schema}"))?;
                // 浏览别的库不该改掉会话的执行上下文：切过去、扫完切回来
                // （调用方持有 engine 的 tokio Mutex，这段期间无并发命令）
                redis_select(conn, target).await?;
                let scanned = redis_scan_typed_keys(conn).await;
                redis_select(conn, *db).await.ok();
                let typed = scanned?;
                let mut counts = [0usize; REDIS_VALUE_TYPES.len()];
                for (_, type_name) in &typed {
                    if let Some(i) = REDIS_VALUE_TYPES.iter().position(|t| t == type_name)
                        && let Some(c) = counts.get_mut(i)
                    {
                        *c += 1;
                    }
                }
                Ok(REDIS_VALUE_TYPES
                    .iter()
                    .zip(counts.iter())
                    .filter(|(_, count)| **count > 0)
                    .filter_map(|(type_name, count)| {
                        let group = ObjectGroup::from_redis_type(type_name)?;
                        Some(CatalogNode {
                            key: format!("{schema}#{}", group.as_str()),
                            name: (*type_name).to_string(),
                            kind: "group".to_string(),
                            detail: Some(count.to_string()),
                        })
                    })
                    .collect())
            }
            DbEngine::Dameng(_) => {
                // 分类计数：DBeaver DamengMetaModel 同源——直查 SYSOBJECTS 基表
                //（TYPE$+SUBTYPE$ 分型），不用 ALL_* 兼容层。触发器 SUBTYPE$='TRIG'
                //（DBeaver 的 TRIG 三表 JOIN 简化为单表 SCHID 过滤，PID 关联的
                // 目标表信息在对象详情时才取）。任一查询失败不阻断其余分组。
                let schema_o = schema.to_string();
                engine.with_dameng(move |client| {
                    let bind = [dm_pure::Param::Str(schema_o.clone())];
                    let tables = dm_query_rows(
                        client,
                        "SELECT COUNT(*) FROM SYSOBJECTS t, SYSOBJECTS s \
                         WHERE t.SCHID = s.ID AND s.NAME = ? \
                           AND t.TYPE$ = 'SCHOBJ' AND t.SUBTYPE$ IN ('UTAB','STAB','ITAB')",
                        &bind,
                    )
                    .ok()
                    .and_then(|rows| rows.first().map(|row| dm_row_i64(row, 0)))
                    .unwrap_or(0);
                    let views = dm_query_rows(
                        client,
                        "SELECT COUNT(*) FROM SYSOBJECTS t, SYSOBJECTS s \
                         WHERE t.SCHID = s.ID AND s.NAME = ? \
                           AND t.TYPE$ = 'SCHOBJ' AND t.SUBTYPE$ = 'VIEW'",
                        &bind,
                    )
                    .ok()
                    .and_then(|rows| rows.first().map(|row| dm_row_i64(row, 0)))
                    .unwrap_or(0);
                    let sequences = dm_query_rows(
                        client,
                        "SELECT COUNT(*) FROM SYSOBJECTS t, SYSOBJECTS s \
                         WHERE t.SCHID = s.ID AND s.NAME = ? \
                           AND t.TYPE$ = 'SCHOBJ' AND t.SUBTYPE$ = 'SEQ'",
                        &bind,
                    )
                    .ok()
                    .and_then(|rows| rows.first().map(|row| dm_row_i64(row, 0)))
                    .unwrap_or(0);
                    let triggers = dm_query_rows(
                        client,
                        "SELECT COUNT(*) FROM SYSOBJECTS t, SYSOBJECTS s \
                         WHERE t.SCHID = s.ID AND s.NAME = ? \
                           AND t.TYPE$ = 'SCHOBJ' AND t.SUBTYPE$ = 'TRIG'",
                        &bind,
                    )
                    .ok()
                    .and_then(|rows| rows.first().map(|row| dm_row_i64(row, 0)))
                    .unwrap_or(0);
                    // 函数/过程同属 PROC（DBeaver INFO1 区分：1=过程 0=函数）
                    let routine_rows = dm_query_rows(
                        client,
                        "SELECT CASE WHEN t.INFO1 & 1 = 1 THEN 'PROCEDURE' ELSE 'FUNCTION' END, \
                         COUNT(*) FROM SYSOBJECTS t, SYSOBJECTS s \
                         WHERE t.SCHID = s.ID AND s.NAME = ? \
                           AND t.TYPE$ = 'SCHOBJ' AND t.SUBTYPE$ = 'PROC' \
                         GROUP BY CASE WHEN t.INFO1 & 1 = 1 THEN 'PROCEDURE' ELSE 'FUNCTION' END",
                        &bind,
                    )
                    .unwrap_or_default();
                    let count_of = |kind: &str| {
                        routine_rows
                            .iter()
                            .find(|row| {
                                dm_row_string(row.as_slice(), 0).eq_ignore_ascii_case(kind)
                            })
                            .map(|row| dm_row_i64(row.as_slice(), 1))
                            .unwrap_or(0)
                    };
                    // 回收站（DM8 RECYCLEBIN 参数开启后可用，DBA_RECYCLEBIN
                    // 视图同 Oracle 形态）；未开启/无视图时容错为 0
                    let dm_recyclebin = dm_query_rows(
                        client,
                        "SELECT COUNT(*) FROM DBA_RECYCLEBIN WHERE OWNER = ?",
                        &bind,
                    )
                    .ok()
                    .and_then(|rows| rows.first().map(|row| dm_row_i64(row, 0)))
                    .unwrap_or(0);
                    // 实例级管理对象（表空间/用户/角色，DBeaver 挂数据源节点
                    //——我们的树只有 schema 层，约定挂 SYS schema 下）；
                    // V$TABLESPACE 无权限/视图缺失时容错为 0（组自动隐藏）。
                    // 用户/角色清单同源 DBeaver DamengDataSource 的 UserCache/
                    // RoleCache 实证 SQL
                    let (dm_tablespaces, dm_users, dm_roles) = if schema_o == "SYS" {
                        (
                            dm_query_rows(client, "SELECT COUNT(*) FROM V$TABLESPACE", &[])
                                .ok()
                                .and_then(|rows| rows.first().map(|row| dm_row_i64(row, 0)))
                                .unwrap_or(0),
                            dm_query_rows(
                                client,
                                "SELECT COUNT(*) FROM SYSOBJECTS WHERE SUBTYPE$ = 'USER'",
                                &[],
                            )
                            .ok()
                            .and_then(|rows| rows.first().map(|row| dm_row_i64(row, 0)))
                            .unwrap_or(0),
                            dm_query_rows(
                                client,
                                "SELECT COUNT(*) FROM SYSOBJECTS \
                                 WHERE TYPE$ = 'UR' AND SUBTYPE$ = 'ROLE' \
                                   AND (INFO2 IS NULL OR INFO2 != 1) AND INFO1 = 0",
                                &[],
                            )
                            .ok()
                            .and_then(|rows| rows.first().map(|row| dm_row_i64(row, 0)))
                            .unwrap_or(0),
                        )
                    } else {
                        (0, 0, 0)
                    };
                    Ok(group_nodes(
                        &schema_o,
                        GroupCounts {
                            triggers: Some(triggers),
                            recyclebin: Some(dm_recyclebin),
                            tablespaces: Some(dm_tablespaces),
                            dm_users: Some(dm_users),
                            dm_roles: Some(dm_roles),
                            // 同义词（SUBTYPE$='SYN'）与包（SUBTYPE$='PKGS'）
                            synonyms: Some(
                                dm_query_rows(
                                    client,
                                    "SELECT COUNT(*) FROM SYSOBJECTS t, SYSOBJECTS s \
                                     WHERE t.SCHID = s.ID AND s.NAME = ? \
                                       AND t.TYPE$ = 'SCHOBJ' AND t.SUBTYPE$ = 'SYN'",
                                    &bind,
                                )
                                .ok()
                                .and_then(|rows| rows.first().map(|row| dm_row_i64(row, 0)))
                                .unwrap_or(0),
                            ),
                            packages: Some(
                                dm_query_rows(
                                    client,
                                    "SELECT COUNT(*) FROM SYSOBJECTS t, SYSOBJECTS s \
                                     WHERE t.SCHID = s.ID AND s.NAME = ? \
                                       AND t.TYPE$ = 'SCHOBJ' AND t.SUBTYPE$ = 'PKGS'",
                                    &bind,
                                )
                                .ok()
                                .and_then(|rows| rows.first().map(|row| dm_row_i64(row, 0)))
                                .unwrap_or(0),
                            ),
                            ..GroupCounts::base(
                                tables,
                                views,
                                count_of("FUNCTION"),
                                count_of("PROCEDURE"),
                                sequences,
                            )
                        },
                    ))
                })
                .await
            }
            DbEngine::Closed => Err("连接已关闭".to_string()),
        }
    }
