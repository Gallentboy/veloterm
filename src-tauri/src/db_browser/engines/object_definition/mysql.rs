//! MySQL 的对象定义（函数/过程/触发器/事件/序列）。

use super::super::{oracle_engine, ObjectDefinition};
use oracle_async::Queryable as _;

pub(crate) async fn definition(
    conn: &mut mysql_async::Conn,
    schema: &str,
    name: &str,
    kind: &str,
) -> Result<ObjectDefinition, String> {
                // OB Oracle 租户：表/视图走 SHOW CREATE TABLE 或 GET_DDL，
                // 过程/函数/触发器/包走 all_source（ODC 同源口径）
                if conn.ob_is_oracle_mode() {
                    return oracle_engine::ob_oracle_object_definition(
                        conn, schema, name, kind,
                    )
                    .await;
                }
                use mysql_async::prelude::Queryable as _;
                // 序列：MySQL 本身没有序列对象（AUTO_INCREMENT 列即其替代
                // 形态），取定义直接返回空——不报错，前端 DDL 面板自然空白。
                // OB MySQL 租户虽然有 CREATE SEQUENCE（树上会列出名字），但
                // 服务端没有 SHOW CREATE SEQUENCE、也没有可复用的标准元数据
                // 视图，官方 ODC 4.5 同样只给名字，这里一并返回空。
                if kind == "sequence" {
                    return Ok(ObjectDefinition {
                        definition: String::new(),
                        call_template: String::new(),
                        arg_count: 0,
                    });
                }
                // 触发器：DBeaver MySQLTrigger 同源——SHOW CREATE TRIGGER
                //（列名含 "Create Trigger"；无调用骨架，返回空 template）
                if kind == "trigger" {
                    let s = schema.replace('`', "``");
                    let n = name.replace('`', "``");
                    let mut result = conn
                        .query_iter(format!("SHOW CREATE TRIGGER `{s}`.`{n}`"))
                        .await
                        .map_err(|e| format!("MySQL 拉取触发器定义失败：{e}"))?;
                    let create_idx = result
                        .columns_ref()
                        .iter()
                        .position(|c| {
                            c.name_str().to_lowercase().contains("create")
                                && !c.name_str().to_lowercase().contains("sql_mode")
                        })
                        .ok_or_else(|| "SHOW CREATE TRIGGER 结果中未找到定义列".to_string())?;
                    let rows: Vec<mysql_async::Row> = result
                        .collect()
                        .await
                        .map_err(|e| format!("MySQL 读取触发器定义失败：{e}"))?;
                    let row = rows
                        .into_iter()
                        .next()
                        .ok_or_else(|| "SHOW CREATE TRIGGER 未返回定义".to_string())?;
                    return Ok(ObjectDefinition {
                        definition: row.get::<String, usize>(create_idx).unwrap_or_default(),
                        call_template: String::new(),
                        arg_count: 0,
                    });
                }
                // 事件：SHOW CREATE EVENT（DBeaver MySQLEvent 的 EVENT_DEFINITION
                // 列拼 DDL 同型，直接取原句更保真）
                if kind == "event" {
                    let s = schema.replace('`', "``");
                    let n = name.replace('`', "``");
                    let mut result = conn
                        .query_iter(format!("SHOW CREATE EVENT `{s}`.`{n}`"))
                        .await
                        .map_err(|e| format!("MySQL 拉取事件定义失败：{e}"))?;
                    let create_idx = result
                        .columns_ref()
                        .iter()
                        .position(|c| c.name_str().to_lowercase().starts_with("create"))
                        .ok_or_else(|| "SHOW CREATE EVENT 结果中未找到定义列".to_string())?;
                    let rows: Vec<mysql_async::Row> = result
                        .collect()
                        .await
                        .map_err(|e| format!("MySQL 读取事件定义失败：{e}"))?;
                    let row = rows
                        .into_iter()
                        .next()
                        .ok_or_else(|| "SHOW CREATE EVENT 未返回定义".to_string())?;
                    return Ok(ObjectDefinition {
                        definition: row.get::<String, usize>(create_idx).unwrap_or_default(),
                        call_template: String::new(),
                        arg_count: 0,
                    });
                }
                let verb = if kind == "function" { "FUNCTION" } else { "PROCEDURE" };
                let s = schema.replace('`', "``");
                let n = name.replace('`', "``");
                let mut result = conn
                    .query_iter(format!("SHOW CREATE {verb} `{s}`.`{n}`"))
                    .await
                    .map_err(|e| format!("MySQL 拉取定义失败：{e}"))?;
                // 列名随版本漂移（Procedure/sql_mode/Create Procedure/字符集…），
                // 找名字含 "create" 的列取定义文本
                let create_idx = result
                    .columns_ref()
                    .iter()
                    .position(|c| c.name_str().to_lowercase().contains("create"))
                    .ok_or_else(|| "SHOW CREATE 结果中未找到定义列".to_string())?;
                let rows: Vec<mysql_async::Row> = result
                    .collect()
                    .await
                    .map_err(|e| format!("MySQL 读取定义失败：{e}"))?;
                let row = rows
                    .into_iter()
                    .next()
                    .ok_or_else(|| "SHOW CREATE 未返回定义".to_string())?;
                let definition = row
                    .get::<String, usize>(create_idx)
                    .unwrap_or_default();
                // 入参个数（ordinal_position=0 是函数返回值，排除）
                let arg_count: i64 = conn
                    .exec_first(
                        "SELECT COUNT(*) FROM information_schema.parameters \
                         WHERE specific_schema = ? AND specific_name = ? \
                           AND ordinal_position > 0",
                        (schema, name),
                    )
                    .await
                    .map_err(|e| format!("MySQL 查询参数个数失败：{e}"))?
                    .unwrap_or(0);
                let call_template = if kind == "function" {
                    format!("SELECT `{s}`.`{n}`()")
                } else {
                    format!("CALL `{s}`.`{n}`()")
                };
                Ok(ObjectDefinition {
                    definition,
                    call_template,
                    arg_count: arg_count.max(0) as usize,
                })
}
