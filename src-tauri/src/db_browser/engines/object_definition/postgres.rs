//! PostgreSQL 的对象定义（函数/过程/序列/触发器/规则）。

use super::super::ObjectDefinition;

pub(crate) async fn definition(
    client: &mut tokio_postgres::Client,
    schema: &str,
    name: &str,
    kind: &str,
) -> Result<ObjectDefinition, String> {
                // 序列：pg_get_sequencedef（PG 10+）直接返回 CREATE SEQUENCE 语句
                if kind == "sequence" {
                    let row = client
                        .query_one(
                            "SELECT pg_get_sequencedef(format('%I.%I', $1, $2)::regclass)",
                            &[&schema, &name],
                        )
                        .await
                        .map_err(|e| format!("PostgreSQL 拉取序列定义失败：{e}"))?;
                    let definition: String = row
                        .try_get(0)
                        .map_err(|e| format!("PostgreSQL 读取序列定义失败：{e}"))?;
                    return Ok(ObjectDefinition {
                        definition,
                        call_template: format!("SELECT nextval('{schema}.{name}');"),
                        arg_count: 0,
                    });
                }
                // 触发器：pg_get_triggerdef（DBeaver PostgreTrigger 同源）
                if kind == "trigger" {
                    let row = client
                        .query_opt(
                            "SELECT pg_catalog.pg_get_triggerdef(t.oid) \
                             FROM pg_catalog.pg_trigger t \
                             JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
                             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                             WHERE n.nspname = $1 AND t.tgname = $2 LIMIT 1",
                            &[&schema, &name],
                        )
                        .await
                        .map_err(|e| format!("PostgreSQL 拉取触发器定义失败：{e}"))?
                        .ok_or_else(|| "未找到该触发器（可能已被删除）".to_string())?;
                    let definition: String = row
                        .try_get(0)
                        .map_err(|e| format!("PostgreSQL 读取触发器定义失败：{e}"))?;
                    return Ok(ObjectDefinition {
                        definition,
                        call_template: String::new(),
                        arg_count: 0,
                    });
                }
                // 规则：pg_get_ruledef（DBeaver PostgreTableReal.RuleCache 同源）
                if kind == "rule" {
                    let row = client
                        .query_opt(
                            "SELECT pg_catalog.pg_get_ruledef(r.oid) \
                             FROM pg_catalog.pg_rewrite r \
                             JOIN pg_catalog.pg_class c ON c.oid = r.ev_class \
                             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                             WHERE n.nspname = $1 AND r.rulename = $2 LIMIT 1",
                            &[&schema, &name],
                        )
                        .await
                        .map_err(|e| format!("PostgreSQL 拉取规则定义失败：{e}"))?
                        .ok_or_else(|| "未找到该规则（可能已被删除）".to_string())?;
                    let definition: String = row
                        .try_get(0)
                        .map_err(|e| format!("PostgreSQL 读取规则定义失败：{e}"))?;
                    return Ok(ObjectDefinition {
                        definition,
                        call_template: String::new(),
                        arg_count: 0,
                    });
                }
                // 扩展（pg_extension）：无源码概念，重建安装语句并标注已装
                // 版本（DBeaver PostgreExtension 的 DDL 同义）；extrelocatable
                // 留在注释里供迁移参考
                if kind == "extension" {
                    let row = client
                        .query_opt(
                            "SELECT e.extversion, e.extrelocatable \
                             FROM pg_catalog.pg_extension e \
                             JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace \
                             WHERE n.nspname = $1 AND e.extname = $2",
                            &[&schema, &name],
                        )
                        .await
                        .map_err(|e| format!("PostgreSQL 拉取扩展信息失败：{e}"))?
                        .ok_or_else(|| "未找到该扩展（可能已被卸载）".to_string())?;
                    let version: String = row
                        .try_get(0)
                        .map_err(|e| format!("PostgreSQL 读取扩展信息失败：{e}"))?;
                    let relocatable: bool = row.try_get(1).unwrap_or(false);
                    let q = |v: &str| format!("\"{}\"", v.replace('"', "\"\""));
                    let definition = format!(
                        "-- 已安装版本 {version}（{}）\nCREATE EXTENSION {} WITH SCHEMA {};",
                        if relocatable { "可重定位" } else { "不可重定位" },
                        q(name),
                        q(schema)
                    );
                    return Ok(ObjectDefinition {
                        definition,
                        call_template: String::new(),
                        arg_count: 0,
                    });
                }
                // 外部表（relkind='f'，FDW）：PG 无现成 pg_get_xxxdef
                //（pg_get_viewdef 不适用），DDL 手工拼装——server 与表级选项在
                // information_schema.foreign_tables / foreign_table_options，
                // 列走 pg_attribute + format_type（DBeaver PostgreTableForeign
                // 的 generateSource 同义），列级选项在 foreign_column_options
                if kind == "ftable" {
                    let q = |v: &str| format!("\"{}\"", v.replace('"', "\"\""));
                    let srv_row = client
                        .query_opt(
                            "SELECT ft.foreign_server_name \
                             FROM information_schema.foreign_tables ft \
                             WHERE ft.foreign_table_schema = $1 \
                               AND ft.foreign_table_name = $2",
                            &[&schema, &name],
                        )
                        .await
                        .map_err(|e| format!("PostgreSQL 拉取外部表信息失败：{e}"))?
                        .ok_or_else(|| "未找到该外部表（可能已被删除）".to_string())?;
                    let server: String = srv_row
                        .try_get(0)
                        .map_err(|e| format!("PostgreSQL 读取外部表信息失败：{e}"))?;
                    let col_rows = client
                        .query(
                            "SELECT a.attname, format_type(a.atttypid, a.atttypmod), \
                                    a.attnotnull, pg_get_expr(ad.adbin, ad.adrelid) \
                             FROM pg_catalog.pg_attribute a \
                             LEFT JOIN pg_catalog.pg_attrdef ad \
                               ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum \
                             WHERE a.attrelid = format('%I.%I', $1, $2)::regclass \
                               AND a.attnum > 0 AND NOT a.attisdropped \
                             ORDER BY a.attnum",
                            &[&schema, &name],
                        )
                        .await
                        .map_err(|e| format!("PostgreSQL 拉取外部表列结构失败：{e}"))?;
                    // 列级选项（FDW 的列名映射等）：attname → OPTIONS 串
                    let col_opt_rows = client
                        .query(
                            "SELECT attname, option_name, option_value \
                             FROM information_schema.foreign_column_options \
                             WHERE foreign_table_schema = $1 \
                               AND foreign_table_name = $2 ORDER BY attname",
                            &[&schema, &name],
                        )
                        .await
                        .unwrap_or_default();
                    let mut col_opts: std::collections::HashMap<String, Vec<String>> =
                        std::collections::HashMap::new();
                    for row in &col_opt_rows {
                        if let (Ok(att), Ok(k), Ok(v)) = (
                            row.try_get::<_, String>(0),
                            row.try_get::<_, String>(1),
                            row.try_get::<_, String>(2),
                        ) {
                            col_opts.entry(att).or_default().push(format!("{k} '{v}'"));
                        }
                    }
                    let col_lines: Vec<String> = col_rows
                        .iter()
                        .filter_map(|row| {
                            let att: String = row.try_get(0).ok()?;
                            let ty: String = row.try_get(1).ok()?;
                            let notnull: bool = row.try_get(2).ok()?;
                            let dflt: Option<String> = row.try_get(3).ok()?;
                            let mut line = format!("  {} {}", q(&att), ty);
                            if let Some(d) = dflt {
                                line.push_str(&format!(" DEFAULT {d}"));
                            }
                            if notnull {
                                line.push_str(" NOT NULL");
                            }
                            if let Some(opts) = col_opts.get(&att) {
                                line.push_str(&format!(" OPTIONS ({})", opts.join(", ")));
                            }
                            Some(line)
                        })
                        .collect();
                    let opt_rows = client
                        .query(
                            "SELECT option_name, option_value \
                             FROM information_schema.foreign_table_options \
                             WHERE foreign_table_schema = $1 \
                               AND foreign_table_name = $2 ORDER BY option_name",
                            &[&schema, &name],
                        )
                        .await
                        .unwrap_or_default();
                    let table_opts = opt_rows
                        .iter()
                        .filter_map(|row| {
                            Some(format!(
                                "{} '{}'",
                                row.try_get::<_, String>(0).ok()?,
                                row.try_get::<_, String>(1).ok()?,
                            ))
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let mut definition = format!(
                        "CREATE FOREIGN TABLE {}.{} (\n{}\n) SERVER {}",
                        q(schema),
                        q(name),
                        col_lines.join(",\n"),
                        q(&server)
                    );
                    if !table_opts.is_empty() {
                        definition.push_str(&format!(" OPTIONS ({table_opts})"));
                    }
                    definition.push(';');
                    return Ok(ObjectDefinition {
                        definition,
                        call_template: format!(
                            "SELECT * FROM {}.{} LIMIT 100;",
                            q(schema),
                            q(name)
                        ),
                        arg_count: 0,
                    });
                }
                let row = client
                    .query_opt(
                        "SELECT pg_get_functiondef(p.oid), p.pronargs \
                         FROM pg_proc p JOIN pg_namespace n ON p.pronamespace = n.oid \
                         WHERE n.nspname = $1 AND p.proname = $2 LIMIT 1",
                        &[&schema, &name],
                    )
                    .await
                    .map_err(|e| format!("PostgreSQL 拉取定义失败：{e}"))?
                    .ok_or_else(|| "未找到该函数/过程（可能已被删除）".to_string())?;
                let definition: String = row
                    .try_get(0)
                    .map_err(|e| format!("PostgreSQL 读取定义失败：{e}"))?;
                let arg_count: i16 = row.try_get(1).unwrap_or(0);
                let q = |v: &str| format!("\"{}\"", v.replace('"', "\"\""));
                let call_template = if kind == "function" {
                    // SELECT * FROM f() 对标量/集合/表函数均合法
                    format!("SELECT * FROM {}.{}()", q(schema), q(name))
                } else {
                    format!("CALL {}.{}()", q(schema), q(name))
                };
                Ok(ObjectDefinition {
                    definition,
                    call_template,
                    arg_count: arg_count.max(0) as usize,
                })
}
