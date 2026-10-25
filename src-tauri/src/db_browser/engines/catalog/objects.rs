//! catalog_objects 的引擎分发：分组内的对象清单。
//! 拆自 engines/mod.rs 的同名方法，行为零变更（含 Redis 前置 early-return
//! 与 MySQL/PG 共享 SQL 模板元组）。

use super::super::{
    CatalogNode, DbEngine, OB_DICT_PREFIX, ObjectGroup, dm_query_rows, dm_query_strings,
    dm_row_string, ob_internal_object, ob_mysql_mviews, ob_mysql_sequences,
    ob_oracle_mviews, ob_oracle_packages, ob_oracle_tables, ob_quote_literal, odbc_is_view_type,
    odbc_row_string, oracle_dict_prefix, oracle_row_string, oracle_scalar_string, parse_redis_db,
    quote_ident, recycle_node, redis_scan_typed_keys, redis_select,
};

pub(crate) async fn objects(
    engine: &mut DbEngine,
    schema: &str,
    group: ObjectGroup,
) -> Result<Vec<CatalogNode>, String> {
        let kind = group.as_str().to_string();
        // Redis 分组没有 SQL 形态：先进 Redis 臂（key 是「对象」，detail 带
        // 值类型供前端生成读命令），SQL 引擎的 group→SQL 映射留在下面。
        if let DbEngine::Redis { conn, db, .. } = engine {
            let type_name = group
                .redis_type()
                .ok_or_else(|| format!("Redis 不支持的对象分组：{kind}"))?;
            let target =
                parse_redis_db(schema).ok_or_else(|| format!("无效的 Redis 逻辑库：{schema}"))?;
            redis_select(conn, target).await?;
            let scanned = redis_scan_typed_keys(conn).await;
            redis_select(conn, *db).await.ok();
            return Ok(scanned?
                .into_iter()
                .filter(|(_, t)| t == type_name)
                .map(|(key, t)| CatalogNode {
                    // key 里可能含 `.`，故用 `{db}.{key}` 仅作唯一标识（前端
                    // Redis 分支按 node.name 取裸 key，不切 key）
                    key: format!("{schema}.{key}"),
                    name: key,
                    // kind = "table"：复用前端「表节点可展开/可右键」的现成通路
                    kind: "table".to_string(),
                    detail: Some(t),
                })
                .collect());
        }
        let (sql_mysql, sql_pg, name_col): (&str, &str, &str) = match group {
            ObjectGroup::Table => (
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = ? AND table_type = 'BASE TABLE' ORDER BY table_name",
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = $1 AND table_type = 'BASE TABLE' ORDER BY table_name",
                "table_name",
            ),
            ObjectGroup::View => (
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = ? AND table_type = 'VIEW' ORDER BY table_name",
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = $1 AND table_type = 'VIEW' ORDER BY table_name",
                "table_name",
            ),
            ObjectGroup::Function => (
                "SELECT routine_name FROM information_schema.routines \
                 WHERE routine_schema = ? AND routine_type = 'FUNCTION' ORDER BY routine_name",
                "SELECT routine_name FROM information_schema.routines \
                 WHERE routine_schema = $1 AND routine_type = 'FUNCTION' ORDER BY routine_name",
                "routine_name",
            ),
            ObjectGroup::Procedure => (
                "SELECT routine_name FROM information_schema.routines \
                 WHERE routine_schema = ? AND routine_type = 'PROCEDURE' ORDER BY routine_name",
                "SELECT routine_name FROM information_schema.routines \
                 WHERE routine_schema = $1 AND routine_type = 'PROCEDURE' ORDER BY routine_name",
                "routine_name",
            ),
            ObjectGroup::Sequence => (
                "SELECT sequence_name FROM information_schema.sequences \
                 WHERE sequence_schema = ? ORDER BY sequence_name",
                "SELECT sequence_name FROM information_schema.sequences \
                 WHERE sequence_schema = $1 ORDER BY sequence_name",
                "sequence_name",
            ),
            // 触发器：DBeaver MySQLCatalog.TriggerCache 同源
            //（PG 侧不用通用模板，在 Postgres 臂单独拼 pg_trigger 查询）
            ObjectGroup::Trigger => (
                "SELECT trigger_name FROM information_schema.triggers \
                 WHERE trigger_schema = ? GROUP BY trigger_name ORDER BY trigger_name",
                // 占位：PG 走 pg_catalog，不会用这条（Postgres 臂对 Trigger/MView
                // 分组单独拼 SQL 后再执行）
                "SELECT trigger_name FROM information_schema.triggers \
                 WHERE trigger_schema = $1 GROUP BY trigger_name ORDER BY trigger_name",
                "trigger_name",
            ),
            ObjectGroup::MaterializedView => {
                // MySQL/OB MySQL 模式无物化视图（分组树上也不会出现），
                // 走到这里返回空集保持签名
                (
                    "SELECT NULL AS name FROM DUAL WHERE 1 = 0",
                    "SELECT NULL AS name WHERE 1 = 0",
                    "name",
                )
            }
            // 事件（MySQL Event Scheduler；DBeaver MySQLCatalog.EventCache 同源）
            ObjectGroup::Event => (
                "SELECT event_name FROM information_schema.events \
                 WHERE event_schema = ? ORDER BY event_name",
                "SELECT NULL AS name WHERE 1 = 0",
                "event_name",
            ),
            // 同义词 / 包 / 规则：不适用 MySQL/PG 通用模板（Oracle 系/PG 专属），
            // 各引擎臂单独处理；这里给空集占位（外部表/扩展/DB Link/回收站/
            // 类型/达梦实例级分组同理，均由对应引擎臂单独拼 SQL）
            ObjectGroup::Synonym
            | ObjectGroup::Package
            | ObjectGroup::Rule
            | ObjectGroup::ForeignTable
            | ObjectGroup::Extension
            | ObjectGroup::DbLink
            | ObjectGroup::RecycleBin
            | ObjectGroup::DataType
            | ObjectGroup::Tablespace
            | ObjectGroup::DmUser
            | ObjectGroup::DmRole => {
                (
                    "SELECT NULL AS name FROM DUAL WHERE 1 = 0",
                    "SELECT NULL AS name WHERE 1 = 0",
                    "name",
                )
            }
            // Redis 六型在上面的 early return 里已消化；走到这里说明是
            // 非 Redis 引擎收到了 Redis 分组（前端/调用方拼错），直接拒绝
            // 而不是编造一条查不出东西的 SQL。
            ObjectGroup::RedisString
            | ObjectGroup::RedisHash
            | ObjectGroup::RedisList
            | ObjectGroup::RedisSet
            | ObjectGroup::RedisZset
            | ObjectGroup::RedisStream => {
                return Err(format!("当前引擎不支持对象分组：{kind}"));
            }
        };
        let _ = name_col;
        match engine {
            DbEngine::MySql(conn) => {
                // OB Oracle：all_objects 按类型列（与原生 Oracle 臂同源；BIN$
                // 回收站滤除）。OB 无物化视图/事件/外部表等 MySQL 专有分组，
                // 这些分组树上不出现，走到这里返回空集
                if conn.ob_is_oracle_mode() {
                    use mysql_async::prelude::Queryable as _;
                    // 表与包走各自的权威清单（表=ALL_TABLES、包=PACKAGE ∪
                    // PACKAGE BODY 去重），与 catalog_groups 的计数同源
                    let special: Option<Vec<String>> = match group {
                        ObjectGroup::Table => Some(ob_oracle_tables(conn, schema).await?),
                        ObjectGroup::Package => Some(ob_oracle_packages(conn, schema).await?),
                        ObjectGroup::MaterializedView => {
                            Some(ob_oracle_mviews(conn, schema).await)
                        }
                        _ => None,
                    };
                    let names = match special {
                        Some(names) => names,
                        None => {
                            let obj_type = match group {
                                ObjectGroup::View => Some("VIEW"),
                                ObjectGroup::Function => Some("FUNCTION"),
                                ObjectGroup::Procedure => Some("PROCEDURE"),
                                ObjectGroup::Sequence => Some("SEQUENCE"),
                                ObjectGroup::Trigger => Some("TRIGGER"),
                                ObjectGroup::Synonym => Some("SYNONYM"),
                                ObjectGroup::DataType => Some("TYPE"),
                                _ => None,
                            };
                            let Some(obj_type) = obj_type else {
                                return Ok(Vec::new());
                            };
                            let sql = format!(
                                "SELECT object_name FROM {OB_DICT_PREFIX}_objects \
                                 WHERE owner = {} AND object_type = {} \
                                   AND object_name NOT LIKE 'BIN$%' ORDER BY object_name",
                                ob_quote_literal(schema),
                                ob_quote_literal(obj_type)
                            );
                            conn.exec(sql, ())
                                .await
                                .map_err(|e| format!("OceanBase 查询对象清单失败：{e}"))?
                        }
                    };
                    return Ok(names
                        .into_iter()
                        .map(|name| CatalogNode {
                            key: format!("{schema}.{name}"),
                            name,
                            kind: kind.clone(),
                            detail: None,
                        })
                        .collect());
                }
                use mysql_async::prelude::Queryable as _;
                // OB MySQL 租户的两处口径差异：
                // - 序列：OB 专有语法 SHOW SEQUENCES（information_schema 无此表）
                // - 表：滤掉物化视图的容器表/日志表（内部实现，不是用户对象）
                let ob_mysql = conn.ob_compat_mode() == Some(mysql_async::ObCompatMode::MySql);
                // 原生 MySQL 全系没有序列对象（information_schema.sequences 是
                // MariaDB 专有表）：非 MariaDB 直接返回空集，不去发注定失败的查询
                if group == ObjectGroup::Sequence && !ob_mysql && !conn.is_mariadb() {
                    return Ok(Vec::new());
                }
                let mut names: Vec<String> = if ob_mysql && group == ObjectGroup::Sequence {
                    match ob_mysql_sequences(conn, schema).await {
                        Some(list) => list,
                        None => conn
                            .exec(sql_mysql, (schema,))
                            .await
                            .map_err(|e| format!("MySQL 查询对象清单失败：{e}"))?,
                    }
                } else if ob_mysql && group == ObjectGroup::MaterializedView {
                    // OB MySQL 物化视图（OCEANBASE.DBA_MVIEWS）；低版本视图不存在
                    // → 空集（分组本身也不会出现，此处防御）
                    ob_mysql_mviews(conn, schema).await.unwrap_or_default()
                } else {
                    conn.exec(sql_mysql, (schema,))
                        .await
                        .map_err(|e| format!("MySQL 查询对象清单失败：{e}"))?
                };
                if ob_mysql && group == ObjectGroup::Table {
                    names.retain(|n| !ob_internal_object(n));
                }
                Ok(names
                    .into_iter()
                    .map(|name| CatalogNode {
                        key: format!("{schema}.{name}"),
                        name,
                        kind: kind.clone(),
                        detail: None,
                    })
                    .collect())
            }
            DbEngine::Postgres(client) => {
                // 触发器与物化视图不走通用 information_schema 模板（前者在
                // information_schema 无表、后者不在 tables 里）——DBeaver 同源
                // 的 pg_catalog 查询在此单独拼装
                let sql_pg_dyn = match group {
                    ObjectGroup::Trigger => Some(
                        "SELECT t.tgname FROM pg_catalog.pg_trigger t \
                         JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
                         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = $1 AND NOT t.tgisinternal \
                         ORDER BY t.tgname",
                    ),
                    ObjectGroup::MaterializedView => Some(
                        "SELECT c.relname FROM pg_catalog.pg_class c \
                         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = $1 AND c.relkind = 'm' \
                         ORDER BY c.relname",
                    ),
                    ObjectGroup::ForeignTable => Some(
                        "SELECT c.relname FROM pg_catalog.pg_class c \
                         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = $1 AND c.relkind = 'f' \
                         ORDER BY c.relname",
                    ),
                    ObjectGroup::Rule => Some(
                        "SELECT r.rulename FROM pg_catalog.pg_rewrite r \
                         JOIN pg_catalog.pg_class c ON c.oid = r.ev_class \
                         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = $1 AND r.rulename <> '_RETURN' \
                         ORDER BY r.rulename",
                    ),
                    ObjectGroup::Extension => Some(
                        "SELECT e.extname FROM pg_catalog.pg_extension e \
                         JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace \
                         WHERE n.nspname = $1 \
                         ORDER BY e.extname",
                    ),
                    _ => None,
                };
                let final_sql = sql_pg_dyn.unwrap_or(sql_pg);
                let rows = client
                    .query(final_sql, &[&schema])
                    .await
                    .map_err(|e| format!("PostgreSQL 查询对象清单失败：{e}"))?;
                Ok(rows
                    .iter()
                    .filter_map(|row| row.try_get::<_, String>(0).ok())
                    .map(|name| CatalogNode {
                        key: format!("{schema}.{name}"),
                        name,
                        kind: kind.clone(),
                        detail: None,
                    })
                    .collect())
            }
            DbEngine::Oracle(conn) => {
                // all_objects 按对象类型列（Oracle 无 information_schema）；
                // 对象类型大写、BIN$ 回收站滤除；INVALID 对象以徽标 detail 标出
                //（DBeaver 对失效对象标红同义，VALID 不标降噪）。
                // 字典视图前缀自适应（DBA 权限 → dba_*）
                let dict_p = oracle_dict_prefix(conn).await;
                // DB Link / 回收站不在 all_objects，单独走 all_db_links /
                // dba_recyclebin（dba 视图无权限时降级 user_recyclebin）
                if group == ObjectGroup::DbLink {
                    let sql = format!(
                        "SELECT db_link, username, host FROM {dict_p}_db_links \
                         WHERE owner = {} ORDER BY db_link",
                        ob_quote_literal(schema)
                    );
                    let mut rs = conn
                        .query_rows(&sql)
                        .await
                        .map_err(|e| format!("Oracle 查询 DB Link 清单失败：{e}"))?;
                    let mut nodes = Vec::new();
                    while let Some(row) = rs.next_row() {
                        let name = oracle_row_string(row, 0);
                        let user = oracle_row_string(row, 1);
                        let host = oracle_row_string(row, 2);
                        let detail = match (user != "NULL", host != "NULL") {
                            (true, true) => Some(format!("{user}@{host}")),
                            (true, false) => Some(user),
                            (false, true) => Some(host),
                            (false, false) => None,
                        };
                        nodes.push(CatalogNode {
                            key: format!("{schema}.{name}"),
                            name,
                            kind: kind.clone(),
                            detail,
                        });
                    }
                    return Ok(nodes);
                }
                if group == ObjectGroup::RecycleBin {
                    let dba_sql = format!(
                        "SELECT OBJECT_NAME, ORIGINAL_NAME, TYPE FROM dba_recyclebin \
                         WHERE OWNER = {} ORDER BY DROPTIME DESC",
                        ob_quote_literal(schema)
                    );
                    let mut nodes: Vec<CatalogNode> = Vec::new();
                    match conn.query_rows(&dba_sql).await {
                        Ok(mut rs) => {
                            while let Some(row) = rs.next_row() {
                                if let Some(node) = recycle_node(row, schema, &kind) {
                                    nodes.push(node);
                                }
                            }
                        }
                        // 无 dba 视图权限：user_recyclebin 只含登录用户对象，
                        // 仅当展开的 schema 即登录用户时降级可用
                        Err(_) => {
                            let login =
                                oracle_scalar_string(conn, "SELECT USER FROM dual").await;
                            if !login.is_empty()
                                && login.eq_ignore_ascii_case(schema)
                                && let Ok(mut rs) = conn
                                    .query_rows(
                                        "SELECT OBJECT_NAME, ORIGINAL_NAME, TYPE \
                                         FROM user_recyclebin ORDER BY DROPTIME DESC",
                                    )
                                    .await
                            {
                                while let Some(row) = rs.next_row() {
                                    if let Some(node) = recycle_node(row, schema, &kind) {
                                        nodes.push(node);
                                    }
                                }
                            }
                        }
                    }
                    return Ok(nodes);
                }
                let obj_type = match group {
                    ObjectGroup::Table => "TABLE",
                    ObjectGroup::View => "VIEW",
                    ObjectGroup::MaterializedView => "MATERIALIZED VIEW",
                    ObjectGroup::Function => "FUNCTION",
                    ObjectGroup::Procedure => "PROCEDURE",
                    ObjectGroup::Sequence => "SEQUENCE",
                    ObjectGroup::Trigger => "TRIGGER",
                    ObjectGroup::Synonym => "SYNONYM",
                    ObjectGroup::Package => "PACKAGE",
                    ObjectGroup::DataType => "TYPE",
                    // Redis/事件/规则/扩展/达梦实例级分组不适用 Oracle（树上不会
                    // 出现）；给个不存在的类型名让服务端返回空集
                    _ => "VELOTERM_UNREACHABLE",
                };
                let sql = format!(
                    "SELECT object_name, status FROM {dict_p}_objects \
                     WHERE owner = {} AND object_type = {} \
                       AND object_name NOT LIKE 'BIN$%' ORDER BY object_name",
                    ob_quote_literal(schema),
                    ob_quote_literal(obj_type)
                );
                let mut rs = conn
                    .query_rows(&sql)
                    .await
                    .map_err(|e| format!("Oracle 查询对象清单失败：{e}"))?;
                let mut nodes = Vec::new();
                while let Some(row) = rs.next_row() {
                    let name = oracle_row_string(row, 0);
                    // INVALID 才标（触发器/过程/视图失效常见，全标 VALID 太吵）
                    let status = oracle_row_string(row, 1);
                    nodes.push(CatalogNode {
                        key: format!("{schema}.{name}"),
                        name,
                        kind: kind.clone(),
                        detail: if status == "INVALID" {
                            Some(status)
                        } else {
                            None
                        },
                    });
                }
                Ok(nodes)
            }
            // 函数入口的 early return 已消费全部 Redis 分组；这里只是让
            // match 穷尽（借用检查器不会因 early return 收窄 self 类型）
            DbEngine::Redis { .. } => Err(format!("当前引擎不支持对象分组：{kind}")),
            DbEngine::Sqlite(_) => {
                // sqlite_master 按 type 列（table/view）；函数/过程/序列在 SQLite
                // 不存在，直接返回空清单（树上也不会出现这些分组）
                let obj_type = match group {
                    ObjectGroup::Table => "table",
                    ObjectGroup::View => "view",
                    ObjectGroup::Trigger
                    | ObjectGroup::MaterializedView
                    | ObjectGroup::ForeignTable
                    | ObjectGroup::Extension
                    | ObjectGroup::DbLink
                    | ObjectGroup::RecycleBin
                    | ObjectGroup::DataType
                    | ObjectGroup::Tablespace
                    | ObjectGroup::DmUser
                    | ObjectGroup::DmRole
                    | ObjectGroup::Synonym
                    | ObjectGroup::Package
                    | ObjectGroup::Event
                    | ObjectGroup::Rule
                    | ObjectGroup::Function
                    | ObjectGroup::Procedure
                    | ObjectGroup::Sequence => {
                        return Ok(Vec::new());
                    }
                    // Redis 六型已在函数入口的 early return 拦掉，此处不可达
                    ObjectGroup::RedisString
                    | ObjectGroup::RedisHash
                    | ObjectGroup::RedisList
                    | ObjectGroup::RedisSet
                    | ObjectGroup::RedisZset
                    | ObjectGroup::RedisStream => {
                        return Ok(Vec::new());
                    }
                }
                .to_string();
                let schema_o = schema.to_string();
                let kind_o = kind.clone();
                engine.with_sqlite(move |conn| {
                    let schema_q = quote_ident(&schema_o)?;
                    let sql = format!(
                        "SELECT name FROM {schema_q}.sqlite_master \
                         WHERE type = ?1 AND name NOT LIKE 'sqlite_%' ORDER BY name"
                    );
                    let names: Vec<String> = {
                        let mut stmt = conn
                            .prepare(&sql)
                            .map_err(|e| format!("SQLite 查询对象清单失败：{e}"))?;
                        let mut rows = stmt
                            .query_map([&obj_type], |row| row.get::<_, String>(0))
                            .map_err(|e| format!("SQLite 查询对象清单失败：{e}"))?;
                        let mut out: Vec<String> = Vec::new();
                        for row in rows.by_ref() {
                            out.push(row.map_err(|e| format!("SQLite 读取对象清单失败：{e}"))?);
                        }
                        out
                    };
                    Ok(names
                        .into_iter()
                        .map(|name| CatalogNode {
                            key: format!("{schema_o}.{name}"),
                            name,
                            kind: kind_o.clone(),
                            detail: None,
                        })
                        .collect())
                })
                .await
            }
            DbEngine::Dameng(_) => {
                // 各分组对应各自的达梦字典表（序列的 owner 列叫 SEQUENCE_OWNER）；
                // 表/视图不用 ALL_OBJECTS 而是专用字典，避开回收站对象与同名的
                // 分区子表（ALL_OBJECTS 里会出现）
                // 回收站（DM8 RECYCLEBIN 参数开启后可用）：DBA_RECYCLEBIN 视图
                // 同 Oracle 形态，key 用 BIN$ 实名防同名多版本冲突、显示名用
                // 原名；未开启/无视图时容错为空
                if let ObjectGroup::RecycleBin = group {
                    let schema_o = schema.to_string();
                    let kind_o = kind.clone();
                    return engine
                        .with_dameng(move |client| {
                            let rows = dm_query_rows(
                                client,
                                "SELECT OBJECT_NAME, ORIGINAL_NAME, TYPE FROM DBA_RECYCLEBIN \
                                 WHERE OWNER = ? ORDER BY DROPTIME DESC",
                                &[dm_pure::Param::Str(schema_o.clone())],
                            )
                            .unwrap_or_default();
                            Ok(rows
                                .iter()
                                .filter_map(|row| {
                                    let obj = dm_row_string(row, 0);
                                    if obj == "NULL" {
                                        return None;
                                    }
                                    let original = dm_row_string(row, 1);
                                    let otype = dm_row_string(row, 2);
                                    Some(CatalogNode {
                                        key: format!("{schema_o}.{obj}"),
                                        name: if original == "NULL" {
                                            obj
                                        } else {
                                            original
                                        },
                                        kind: kind_o.clone(),
                                        detail: if otype == "NULL" {
                                            None
                                        } else {
                                            Some(otype)
                                        },
                                    })
                                })
                                .collect())
                        })
                        .await;
                }
                let sql = match group {
                    ObjectGroup::Table => {
                        "SELECT t.NAME FROM SYSOBJECTS t, SYSOBJECTS s \
                         WHERE t.SCHID = s.ID AND s.NAME = ? \
                           AND t.TYPE$ = 'SCHOBJ' AND t.SUBTYPE$ IN ('UTAB','STAB','ITAB') \
                         ORDER BY t.NAME"
                    }
                    ObjectGroup::View => {
                        "SELECT t.NAME FROM SYSOBJECTS t, SYSOBJECTS s \
                         WHERE t.SCHID = s.ID AND s.NAME = ? \
                           AND t.TYPE$ = 'SCHOBJ' AND t.SUBTYPE$ = 'VIEW' ORDER BY t.NAME"
                    }
                    ObjectGroup::Function => {
                        "SELECT t.NAME FROM SYSOBJECTS t, SYSOBJECTS s \
                         WHERE t.SCHID = s.ID AND s.NAME = ? \
                           AND t.TYPE$ = 'SCHOBJ' AND t.SUBTYPE$ = 'PROC' AND (t.INFO1 & 1) = 0 \
                         ORDER BY t.NAME"
                    }
                    ObjectGroup::Procedure => {
                        "SELECT t.NAME FROM SYSOBJECTS t, SYSOBJECTS s \
                         WHERE t.SCHID = s.ID AND s.NAME = ? \
                           AND t.TYPE$ = 'SCHOBJ' AND t.SUBTYPE$ = 'PROC' AND (t.INFO1 & 1) = 1 \
                         ORDER BY t.NAME"
                    }
                    ObjectGroup::Sequence => {
                        "SELECT t.NAME FROM SYSOBJECTS t, SYSOBJECTS s \
                         WHERE t.SCHID = s.ID AND s.NAME = ? \
                           AND t.TYPE$ = 'SCHOBJ' AND t.SUBTYPE$ = 'SEQ' ORDER BY t.NAME"
                    }
                    ObjectGroup::Trigger => {
                        "SELECT t.NAME FROM SYSOBJECTS t, SYSOBJECTS s \
                         WHERE t.SCHID = s.ID AND s.NAME = ? \
                           AND t.TYPE$ = 'SCHOBJ' AND t.SUBTYPE$ = 'TRIG' ORDER BY t.NAME"
                    }
                    // 同义词：达梦 SYSOBJECTS 的 SUBTYPE$='SYNONYM'
                    ObjectGroup::Synonym => {
                        "SELECT t.NAME FROM SYSOBJECTS t, SYSOBJECTS s \
                         WHERE t.SCHID = s.ID AND s.NAME = ? \
                           AND t.TYPE$ = 'SCHOBJ' AND t.SUBTYPE$ = 'SYN' ORDER BY t.NAME"
                    }
                    // 包：达梦 SYSOBJECTS 的 SUBTYPE$='PKGS'（包规范）
                    ObjectGroup::Package => {
                        "SELECT t.NAME FROM SYSOBJECTS t, SYSOBJECTS s \
                         WHERE t.SCHID = s.ID AND s.NAME = ? \
                           AND t.TYPE$ = 'SCHOBJ' AND t.SUBTYPE$ = 'PKGS' ORDER BY t.NAME"
                    }
                    // 表空间/用户/角色（实例级，DBeaver DamengDataSource 三缓存
                    // 同源实证 SQL；不按 schema 过滤，无绑定参数）
                    ObjectGroup::Tablespace => {
                        "SELECT NAME FROM V$TABLESPACE ORDER BY NAME"
                    }
                    ObjectGroup::DmUser => {
                        "SELECT NAME FROM SYSOBJECTS WHERE SUBTYPE$ = 'USER' ORDER BY NAME"
                    }
                    ObjectGroup::DmRole => {
                        "SELECT NAME FROM SYSOBJECTS \
                         WHERE TYPE$ = 'UR' AND SUBTYPE$ = 'ROLE' \
                           AND (INFO2 IS NULL OR INFO2 != 1) AND INFO1 = 0 \
                         ORDER BY NAME"
                    }
                    ObjectGroup::MaterializedView
                    | ObjectGroup::ForeignTable
                    | ObjectGroup::Extension
                    | ObjectGroup::DbLink
                    // 回收站已在函数入口单独处理，此处不可达；类型组不适用达梦
                    //（DBeaver 达梦的"数据类型"是 JDBC getTypeInfo 协议层的内建
                    // 类型表，raw SQL 侧无对应字典）
                    | ObjectGroup::DataType
                    | ObjectGroup::RecycleBin
                    | ObjectGroup::Event
                    | ObjectGroup::Rule
                    // Redis 六型已在函数入口的 early return 拦掉，不可达；
                    // 空查询保持签名（返回空集）
                    | ObjectGroup::RedisString
                    | ObjectGroup::RedisHash
                    | ObjectGroup::RedisList
                    | ObjectGroup::RedisSet
                    | ObjectGroup::RedisZset
                    | ObjectGroup::RedisStream => {
                        "SELECT NULL AS name FROM DUAL WHERE 1 = 0"
                    }
                };
                // 实例级分组（表空间/用户/角色）的清单 SQL 无绑定参数：
                // dm 的协议层按占位符计数绑参，多余参数会报错
                let instance_level = matches!(
                    group,
                    ObjectGroup::Tablespace | ObjectGroup::DmUser | ObjectGroup::DmRole
                );
                let schema_o = schema.to_string();
                let kind_o = kind.clone();
                engine.with_dameng(move |client| {
                    let params: Vec<dm_pure::Param> = if instance_level {
                        Vec::new()
                    } else {
                        vec![dm_pure::Param::Str(schema_o.clone())]
                    };
                    let names = dm_query_strings(client, sql, &params)?;
                    Ok(names
                        .into_iter()
                        .map(|name| CatalogNode {
                            key: format!("{schema_o}.{name}"),
                            name,
                            kind: kind_o.clone(),
                            detail: None,
                        })
                        .collect())
                })
                .await
            }
            DbEngine::Odbc(_) => {
                // 标准 SQLTables 全量取回后在客户端按 TABLE_TYPE 过滤（不把
                // 类型传给 SQLTables 的 table_type 参数——部分驱动对
                // "VIEW" 过滤不匹配 "SYSTEM VIEW"，客户端过滤口径与分组
                // 计数严格一致）。显示名带 schema 前缀（SCHEM.NAME），供
                // 列/主键/索引查询拆开限定
                let want_view = matches!(group, ObjectGroup::View);
                let want_table = matches!(group, ObjectGroup::Table);
                if !want_view && !want_table {
                    // 其它分组（函数/过程等）ODBC 目录体系没有，树上也不会
                    // 出现；防御性空集
                    return Ok(Vec::new());
                }
                let catalog = schema.to_string();
                let kind_o = kind.clone();
                engine
                    .with_odbc(move |eng| {
                        let api = crate::db_browser::engines::odbc_engine::odbc_api()?;
                        let (_, rows) = api
                            .tables(eng.dbc.0, &catalog, "", "", "")
                            .map_err(|e| format!("ODBC 查询对象清单失败：{e}"))?;
                        let mut nodes = Vec::new();
                        for row in &rows {
                            let t = odbc_row_string(row, 3);
                            let matched = if want_view {
                                odbc_is_view_type(&t)
                            } else {
                                t.eq_ignore_ascii_case("TABLE")
                                    || t.eq_ignore_ascii_case("SYSTEM TABLE")
                            };
                            if !matched {
                                continue;
                            }
                            let name = odbc_row_string(row, 2);
                            if name.is_empty() || name == "NULL" {
                                continue;
                            }
                            let schem = odbc_row_string(row, 1);
                            let display = if schem.is_empty() || schem == "NULL" {
                                name
                            } else {
                                format!("{schem}.{name}")
                            };
                            nodes.push(CatalogNode {
                                key: format!("{catalog}.{display}"),
                                name: display,
                                kind: kind_o.clone(),
                                detail: None,
                            });
                        }
                        Ok(nodes)
                    })
                    .await
            }
            DbEngine::Closed => Err("连接已关闭".to_string()),
        }
    }

