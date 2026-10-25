//! Oracle 的对象定义（函数/过程/序列/触发器/同义词/包/视图/DB Link/回收站/类型）。

use super::super::oracle_conn::OracleConn;
use super::super::{
    ob_quote_literal, oracle_dict_prefix, oracle_row_string,
    oracle_scalar_string, ObjectDefinition,
};

pub(crate) async fn definition(
    conn: &mut OracleConn,
    schema: &str,
    name: &str,
    kind: &str,
) -> Result<ObjectDefinition, String> {
                // 定义：all_source 按行拼回（CREATE OR REPLACE 前缀在首行内）；
                // 入参个数：all_arguments 的具名参数（argument_name 为 NULL 的
                // 是函数返回值，排除）
                // 字典视图前缀自适应（DBA 权限 → dba_*）
                let dict_p = oracle_dict_prefix(conn).await;
                // 视图：无 all_source 源码，走 GET_DDL（UI 主路径是表详情，
                // 此分支供定义视图/CLI 全链路测试直取）
                if kind == "view" {
                    let ddl_sql = format!(
                        "SELECT DBMS_METADATA.GET_DDL('VIEW', {}, {}) FROM dual",
                        ob_quote_literal(name),
                        ob_quote_literal(schema)
                    );
                    let mut ddl = String::new();
                    if let Ok(mut rs) = conn.query_rows(&ddl_sql).await
                        && let Some(row) = rs.next_row()
                    {
                        ddl = oracle_row_string(row, 0);
                    }
                    if ddl.trim().is_empty() {
                        // 降级 all_views 文本（旧库 GET_DDL 权限缺失时）
                        let vs = format!(
                            "SELECT text FROM all_views WHERE owner = {} AND view_name = {}",
                            ob_quote_literal(schema),
                            ob_quote_literal(name)
                        );
                        if let Ok(mut rs) = conn.query_rows(&vs).await
                            && let Some(row) = rs.next_row()
                        {
                            ddl = oracle_row_string(row, 0);
                        }
                    }
                    if ddl.trim().is_empty() {
                        return Err("未找到该视图（可能已被删除）".to_string());
                    }
                    return Ok(ObjectDefinition {
                        definition: ddl,
                        call_template: String::new(),
                        arg_count: 0,
                    });
                }
                // DB Link：all_db_links 无源码，重建 CREATE DATABASE LINK 语句
                //（字典里本就无口令列，不回显）
                // 用户自定义类型：GET_DDL 出完整 CREATE OR REPLACE TYPE 语句
                //（含 spec 与 body 合并形态）；失败降级 ALL_TYPE_ATTRS 属性清单
                if kind == "datatype" {
                    let ddl_sql = format!(
                        "SELECT DBMS_METADATA.GET_DDL('TYPE', {}, {}) FROM dual",
                        ob_quote_literal(name),
                        ob_quote_literal(schema)
                    );
                    let mut ddl = String::new();
                    if let Ok(mut rs) = conn.query_rows(&ddl_sql).await
                        && let Some(row) = rs.next_row()
                    {
                        ddl = oracle_row_string(row, 0);
                    }
                    if !ddl.trim().is_empty() {
                        return Ok(ObjectDefinition {
                            definition: ddl,
                            call_template: String::new(),
                            arg_count: 0,
                        });
                    }
                    let attr_sql = format!(
                        "SELECT ATTR_NAME, ATTR_TYPE_NAME FROM {dict_p}_type_attrs \
                         WHERE OWNER = {} AND TYPE_NAME = {} ORDER BY ATTR_NO",
                        ob_quote_literal(schema),
                        ob_quote_literal(name)
                    );
                    let mut attrs: Vec<(String, String)> = Vec::new();
                    if let Ok(mut rs) = conn.query_rows(&attr_sql).await {
                        while let Some(row) = rs.next_row() {
                            attrs.push((
                                oracle_row_string(row, 0),
                                oracle_row_string(row, 1),
                            ));
                        }
                    }
                    if attrs.is_empty() {
                        return Err("未找到该类型（可能已被删除）".to_string());
                    }
                    let lines: Vec<String> = attrs
                        .iter()
                        .map(|(attr, ty)| format!("  {attr} {ty}"))
                        .collect();
                    let definition = format!(
                        "CREATE OR REPLACE TYPE \"{schema}\".\"{name}\" AS OBJECT (\n{}\n);",
                        lines.join(",\n")
                    );
                    return Ok(ObjectDefinition {
                        definition,
                        call_template: String::new(),
                        arg_count: 0,
                    });
                }
                if kind == "dblink" {
                    let sql = format!(
                        "SELECT username, host, DECODE(owner, 'PUBLIC', 'PUBLIC', 'USER') \
                         FROM {dict_p}_db_links \
                         WHERE owner = {} AND db_link = {}",
                        ob_quote_literal(schema),
                        ob_quote_literal(name)
                    );
                    let mut rs = conn
                        .query_rows(&sql)
                        .await
                        .map_err(|e| format!("Oracle 拉取 DB Link 定义失败：{e}"))?;
                    let row = rs
                        .next_row()
                        .ok_or_else(|| "未找到该 DB Link（可能已被删除）".to_string())?;
                    let user = oracle_row_string(row, 0);
                    let host = oracle_row_string(row, 1);
                    let scope = oracle_row_string(row, 2);
                    let mut definition = String::from("CREATE ");
                    if scope == "PUBLIC" {
                        definition.push_str("PUBLIC ");
                    }
                    definition.push_str(&format!("DATABASE LINK \"{name}\""));
                    if user != "NULL" {
                        definition.push_str(&format!(
                            " CONNECT TO {user} IDENTIFIED BY \"***\""
                        ));
                    }
                    if host != "NULL" {
                        definition.push_str(&format!(" USING '{host}'"));
                    }
                    definition.push(';');
                    return Ok(ObjectDefinition {
                        definition,
                        call_template: format!("SELECT 1 FROM DUAL@\"{name}\";"),
                        arg_count: 0,
                    });
                }
                // 回收站对象：生成 FLASHBACK 恢复 / PURGE 清除语句提示（同名
                // 多次 DROP 会产生多个 BIN$ 版本，按 DROPTIME 倒序逐个列出；
                // dba 视图无权限时降级 user_recyclebin）
                if kind == "recycle" {
                    let dba_sql = format!(
                        "SELECT OBJECT_NAME, TYPE, DROPTIME FROM dba_recyclebin \
                         WHERE OWNER = {} AND ORIGINAL_NAME = {} \
                         ORDER BY DROPTIME DESC",
                        ob_quote_literal(schema),
                        ob_quote_literal(name)
                    );
                    let mut bins: Vec<(String, String, String)> = Vec::new();
                    match conn.query_rows(&dba_sql).await {
                        Ok(mut rs) => {
                            while let Some(row) = rs.next_row() {
                                let obj = oracle_row_string(row, 0);
                                if obj != "NULL" {
                                    bins.push((
                                        obj,
                                        oracle_row_string(row, 1),
                                        oracle_row_string(row, 2),
                                    ));
                                }
                            }
                        }
                        Err(_) => {
                            let login =
                                oracle_scalar_string(conn, "SELECT USER FROM dual").await;
                            if !login.is_empty() && login.eq_ignore_ascii_case(schema) {
                                let user_sql = format!(
                                    "SELECT OBJECT_NAME, TYPE, DROPTIME FROM user_recyclebin \
                                     WHERE ORIGINAL_NAME = {} ORDER BY DROPTIME DESC",
                                    ob_quote_literal(name)
                                );
                                if let Ok(mut rs) = conn.query_rows(&user_sql).await {
                                    while let Some(row) = rs.next_row() {
                                        let obj = oracle_row_string(row, 0);
                                        if obj != "NULL" {
                                            bins.push((
                                                obj,
                                                oracle_row_string(row, 1),
                                                oracle_row_string(row, 2),
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if bins.is_empty() {
                        return Err("未找到该回收站对象（可能已被清除或恢复）".to_string());
                    }
                    let mut lines: Vec<String> = vec![format!(
                        "-- 回收站对象 {} 个（原名 {name}），恢复用 FLASHBACK、彻底删除用 PURGE：",
                        bins.len()
                    )];
                    for (obj, otype, droptime) in &bins {
                        lines.push(format!("-- {obj}（{otype}，DROP 于 {droptime}）"));
                        lines.push(format!("FLASHBACK TABLE \"{obj}\" TO BEFORE DROP;"));
                        lines.push(format!("PURGE TABLE \"{obj}\";"));
                    }
                    let call_template = bins
                        .first()
                        .map(|(obj, _, _)| {
                            format!("FLASHBACK TABLE \"{obj}\" TO BEFORE DROP;")
                        })
                        .unwrap_or_default();
                    return Ok(ObjectDefinition {
                        definition: lines.join("\n"),
                        call_template,
                        arg_count: 0,
                    });
                }
                if kind == "sequence" {
                    let ddl_sql = format!(
                        "SELECT DBMS_METADATA.GET_DDL('SEQUENCE', {}, {}) FROM dual",
                        ob_quote_literal(name),
                        ob_quote_literal(schema)
                    );
                    let mut rs = conn
                        .query_rows(&ddl_sql)
                        .await
                        .map_err(|e| format!("Oracle 拉取序列定义失败：{e}"))?;
                    let ddl = match rs.next_row() {
                        Some(row) => oracle_row_string(row, 0),
                        None => String::new(),
                    };
                    if ddl.trim().is_empty() {
                        return Err("未找到该序列（可能已被删除）".to_string());
                    }
                    return Ok(ObjectDefinition {
                        definition: ddl,
                        call_template: format!("SELECT {schema}.{name}.NEXTVAL FROM dual;"),
                        arg_count: 0,
                    });
                }
                // 触发器：DBeaver OracleTrigger 同源——all_source TYPE='TRIGGER'
                // 拼回完整触发器体（含 CREATE TRIGGER 头）
                if kind == "trigger" {
                    let sql = format!(
                        "SELECT text FROM {dict_p}_source WHERE owner = {} AND name = {} \
                         AND type = 'TRIGGER' ORDER BY line",
                        ob_quote_literal(schema),
                        ob_quote_literal(name)
                    );
                    let mut rs = conn
                        .query_rows(&sql)
                        .await
                        .map_err(|e| format!("Oracle 拉取触发器定义失败：{e}"))?;
                    let mut lines = Vec::new();
                    while let Some(row) = rs.next_row() {
                        lines.push(oracle_row_string(row, 0));
                    }
                    let definition = lines.join("");
                    if definition.trim().is_empty() {
                        return Err("未找到该触发器（可能已被删除）".to_string());
                    }
                    return Ok(ObjectDefinition {
                        definition,
                        call_template: String::new(),
                        arg_count: 0,
                    });
                }
                // 同义词 / 包：all_source 存源码（PACKAGE / PACKAGE BODY 都在），
                // 同义词无源码走 DBMS_METADATA.GET_DDL（DBeaver 同型）
                if kind == "synonym" || kind == "package" {
                    let ddl_sql = format!(
                        "SELECT DBMS_METADATA.GET_DDL('{}', {}, {}) FROM dual",
                        if kind == "synonym" { "SYNONYM" } else { "PACKAGE" },
                        ob_quote_literal(name),
                        ob_quote_literal(schema)
                    );
                    let mut rs = conn
                        .query_rows(&ddl_sql)
                        .await
                        .map_err(|e| format!("Oracle 拉取定义失败：{e}"))?;
                    let mut ddl = String::new();
                    if let Some(row) = rs.next_row() {
                        ddl = oracle_row_string(row, 0);
                    }
                    // 包有源码时优先 all_source（GET_DDL 对包可能不含包体）
                    if kind == "package" && ddl.trim().is_empty() {
                        let src_sql = format!(
                            "SELECT text FROM {dict_p}_source WHERE owner = {} AND name = {} \
                             AND type = 'PACKAGE' ORDER BY line",
                            ob_quote_literal(schema),
                            ob_quote_literal(name)
                        );
                        let mut src_rs = conn
                            .query_rows(&src_sql)
                            .await
                            .map_err(|e| format!("Oracle 拉取包源码失败：{e}"))?;
                        while let Some(row) = src_rs.next_row() {
                            ddl.push_str(&oracle_row_string(row, 0));
                        }
                    }
                    if ddl.trim().is_empty() {
                        return Err("未找到该对象（可能已被删除）".to_string());
                    }
                    return Ok(ObjectDefinition {
                        definition: ddl,
                        call_template: String::new(),
                        arg_count: 0,
                    });
                }
                let obj_type = if kind == "function" { "FUNCTION" } else { "PROCEDURE" };
                let sql = format!(
                    "SELECT text FROM {dict_p}_source WHERE owner = {} AND name = {} AND type = {} ORDER BY line",
                    ob_quote_literal(schema),
                    ob_quote_literal(name),
                    ob_quote_literal(obj_type)
                );
                let mut rs = conn
                    .query_rows(&sql)
                    .await
                    .map_err(|e| format!("Oracle 拉取定义失败：{e}"))?;
                let mut lines = Vec::new();
                while let Some(row) = rs.next_row() {
                    lines.push(oracle_row_string(row, 0));
                }
                let definition = lines.join("");
                if definition.trim().is_empty() {
                    return Err("未找到该函数/过程（可能已被删除）".to_string());
                }

                let cnt_sql = format!(
                    "SELECT COUNT(*) FROM {dict_p}_arguments WHERE owner = {} AND object_name = {} AND argument_name IS NOT NULL",
                    ob_quote_literal(schema),
                    ob_quote_literal(name)
                );
                let mut cnt_rs = conn
                    .query_rows(&cnt_sql)
                    .await
                    .map_err(|e| format!("Oracle 查询参数个数失败：{e}"))?;
                let arg_count: usize = match cnt_rs.next_row() {
                    Some(row) => oracle_row_string(row, 0)
                        .parse::<i64>()
                        .unwrap_or(0)
                        .max(0) as usize,
                    None => 0,
                };

                let call_template = if kind == "function" {
                    format!("SELECT \"{schema}\".\"{name}\"() FROM dual")
                } else {
                    format!("BEGIN \"{schema}\".\"{name}\"(); END;")
                };
                Ok(ObjectDefinition {
                    definition,
                    call_template,
                    arg_count,
                })
}
