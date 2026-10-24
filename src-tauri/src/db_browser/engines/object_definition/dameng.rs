//! 达梦的对象定义（函数/过程/序列/触发器/同义词/包/回收站/表空间/用户/角色）。

use super::super::{
    dm_query_first_string, dm_query_rows, dm_row_i64, dm_row_string, dm_text_at, DbEngine,
    ObjectDefinition,
};

pub(crate) async fn definition(
    engine: &mut DbEngine,
    schema: &str,
    name: &str,
    kind: &str,
) -> Result<ObjectDefinition, String> {
                // 达梦数据字典同 Oracle 风格：ALL_SOURCE 按行存源码（每行自带换行），
                // ALL_ARGUMENTS 的 ARGUMENT_NAME 为 NULL 者是函数返回值，需排除
                let schema_o = schema.to_string();
                let name_o = name.to_string();
                let kind_o = kind.to_string();
                engine.with_dameng(move |client| {
                    if kind_o == "sequence" {
                        let ddl = dm_query_first_string(
                            client,
                            "SELECT DBMS_METADATA.GET_DDL('SEQUENCE', ?, ?) FROM DUAL",
                            &[
                                dm_pure::Param::Str(name_o.clone()),
                                dm_pure::Param::Str(schema_o.clone()),
                            ],
                        )
                        .unwrap_or_default();
                        if ddl.trim().is_empty() {
                            return Err("未找到该序列（可能已被删除）".to_string());
                        }
                        return Ok(ObjectDefinition {
                            definition: ddl,
                            // 达梦序列取下一个值同样需 FROM DUAL
                            call_template: format!(
                                "SELECT \"{schema_o}\".\"{name_o}\".NEXTVAL FROM DUAL;"
                            ),
                            arg_count: 0,
                        });
                    }
                    // 表空间/用户/角色：实例级管理对象无 DDL，定义视图给属性
                    // 块（Redis 键检视块同型的只读文本形态）
                    if kind_o == "tablespace" {
                        let ts = dm_query_rows(
                            client,
                            "SELECT NAME FROM V$TABLESPACE WHERE NAME = ?",
                            &[dm_pure::Param::Str(name_o.clone())],
                        )
                        .unwrap_or_default();
                        if ts.is_empty() {
                            return Err("未找到该表空间（可能已被删除）".to_string());
                        }
                        // 数据文件：V$DATAFILE.GROUP_ID = 表空间 id
                        //（DBeaver DamengTablespace.FileCache 同源）
                        let files = dm_query_rows(
                            client,
                            "SELECT PATH FROM V$DATAFILE \
                             WHERE GROUP_ID = (SELECT ID FROM V$TABLESPACE WHERE NAME = ?) \
                             ORDER BY PATH",
                            &[dm_pure::Param::Str(name_o.clone())],
                        )
                        .unwrap_or_default();
                        let mut lines = vec![
                            format!("表空间：{name_o}"),
                            String::new(),
                            format!("数据文件（{} 个）：", files.len()),
                        ];
                        for row in &files {
                            let path = dm_row_string(row, 0);
                            if path != "NULL" {
                                lines.push(format!("  {path}"));
                            }
                        }
                        return Ok(ObjectDefinition {
                            definition: lines.join("\n"),
                            call_template: String::new(),
                            arg_count: 0,
                        });
                    }
                    if kind_o == "user" {
                        // 默认表空间：INFO3 低 16 位是默认表空间 id
                        //（DBeaver DamengUser 同源的 INFO3 & 0xFFFF 位掩码）
                        let rows = dm_query_rows(
                            client,
                            "SELECT USER_OBJ.NAME, TS.NAME \
                             FROM SYSOBJECTS USER_OBJ \
                             LEFT JOIN SYS.V$TABLESPACE TS \
                               ON USER_OBJ.INFO3 & 0x000000000000FFFF = TS.ID \
                             WHERE USER_OBJ.SUBTYPE$ = 'USER' AND USER_OBJ.NAME = ?",
                            &[dm_pure::Param::Str(name_o.clone())],
                        )
                        .unwrap_or_default();
                        let Some(row) = rows.into_iter().next() else {
                            return Err("未找到该用户（可能已被删除）".to_string());
                        };
                        let ts = {
                            let raw = dm_row_string(&row, 1);
                            if raw == "NULL" {
                                "（未设置）".to_string()
                            } else {
                                raw
                            }
                        };
                        return Ok(ObjectDefinition {
                            definition: format!("用户：{name_o}\n默认表空间：{ts}"),
                            call_template: String::new(),
                            arg_count: 0,
                        });
                    }
                    if kind_o == "role" {
                        let role = dm_query_rows(
                            client,
                            "SELECT ID FROM SYSOBJECTS \
                             WHERE TYPE$ = 'UR' AND SUBTYPE$ = 'ROLE' AND NAME = ?",
                            &[dm_pure::Param::Str(name_o.clone())],
                        )
                        .unwrap_or_default();
                        if role.is_empty() {
                            return Err("未找到该角色（可能已被删除）".to_string());
                        }
                        // 成员用户（DBeaver DamengRole.PrivUserCache 同源：
                        // SYSGRANTS.PRIVID = -1 即角色授予，OBJID = 角色 id）
                        let members = dm_query_rows(
                            client,
                            "SELECT OBJ.NAME FROM SYSOBJECTS OBJ, SYSGRANTS GRANTS \
                             WHERE GRANTS.PRIVID = -1 AND OBJ.TYPE$ = 'UR' \
                               AND OBJ.ID = GRANTS.URID \
                               AND GRANTS.OBJID = (SELECT ID FROM SYSOBJECTS \
                                                   WHERE TYPE$ = 'UR' AND SUBTYPE$ = 'ROLE' AND NAME = ?) \
                             ORDER BY OBJ.NAME",
                            &[dm_pure::Param::Str(name_o.clone())],
                        )
                        .unwrap_or_default();
                        let mut lines = vec![
                            format!("角色：{name_o}"),
                            String::new(),
                            format!("成员用户（{} 个）：", members.len()),
                        ];
                        for row in &members {
                            let m = dm_row_string(row, 0);
                            if m != "NULL" {
                                lines.push(format!("  {m}"));
                            }
                        }
                        return Ok(ObjectDefinition {
                            definition: lines.join("\n"),
                            call_template: String::new(),
                            arg_count: 0,
                        });
                    }
                    // 回收站：DBA_RECYCLEBIN 生成 FLASHBACK / PURGE 语句提示
                    //（达梦 FLASHBACK DROP 语法同 Oracle，表名需 schema 限定）
                    if kind_o == "recycle" {
                        let rows = dm_query_rows(
                            client,
                            "SELECT OBJECT_NAME, TYPE, DROPTIME FROM DBA_RECYCLEBIN \
                             WHERE OWNER = ? AND ORIGINAL_NAME = ? \
                             ORDER BY DROPTIME DESC",
                            &[
                                dm_pure::Param::Str(schema_o.clone()),
                                dm_pure::Param::Str(name_o.clone()),
                            ],
                        )
                        .unwrap_or_default();
                        if rows.is_empty() {
                            return Err("未找到该回收站对象（可能已被清除或恢复）".to_string());
                        }
                        let mut lines: Vec<String> = vec![format!(
                            "-- 回收站对象 {} 个（原名 {name_o}），恢复用 FLASHBACK、彻底删除用 PURGE：",
                            rows.len()
                        )];
                        for row in &rows {
                            let obj = dm_row_string(row, 0);
                            if obj == "NULL" {
                                continue;
                            }
                            let otype = dm_row_string(row, 1);
                            let droptime = dm_row_string(row, 2);
                            lines.push(format!("-- {obj}（{otype}，DROP 于 {droptime}）"));
                            lines.push(format!(
                                "FLASHBACK TABLE \"{schema_o}\".\"{obj}\" TO BEFORE DROP;"
                            ));
                            lines.push(format!("PURGE TABLE \"{schema_o}\".\"{obj}\";"));
                        }
                        let call_template = rows
                            .first()
                            .map(|row| {
                                let obj = dm_row_string(row, 0);
                                format!(
                                    "FLASHBACK TABLE \"{schema_o}\".\"{obj}\" TO BEFORE DROP;"
                                )
                            })
                            .unwrap_or_default();
                        return Ok(ObjectDefinition {
                            definition: lines.join("\n"),
                            call_template,
                            arg_count: 0,
                        });
                    }
                    // 触发器：DBeaver DamengMetaModel 同源——DBMS_METADATA.GET_DDL
                    //（达梦触发器源码不在 ALL_SOURCE，走 GET_DDL 才能拿到完整定义）
                    if kind_o == "trigger" {
                        let ddl = dm_query_first_string(
                            client,
                            "SELECT DBMS_METADATA.GET_DDL('TRIGGER', ?, ?) FROM DUAL",
                            &[
                                dm_pure::Param::Str(name_o.clone()),
                                dm_pure::Param::Str(schema_o.clone()),
                            ],
                        )
                        .unwrap_or_default();
                        if ddl.trim().is_empty() {
                            return Err("未找到该触发器（可能已被删除）".to_string());
                        }
                        return Ok(ObjectDefinition {
                            definition: ddl,
                            call_template: String::new(),
                            arg_count: 0,
                        });
                    }
                    // 同义词 / 包：达梦 DBMS_METADATA.GET_DDL（ObjectType 枚举名
                    // 'SYNONYM'/'PACKAGE'，DBeaver DamengConstants 同源）
                    if kind_o == "synonym" || kind_o == "package" {
                        let obj_enum = if kind_o == "synonym" { "SYNONYM" } else { "PACKAGE" };
                        let ddl = dm_query_first_string(
                            client,
                            &format!("SELECT DBMS_METADATA.GET_DDL('{obj_enum}', ?, ?) FROM DUAL"),
                            &[
                                dm_pure::Param::Str(name_o.clone()),
                                dm_pure::Param::Str(schema_o.clone()),
                            ],
                        )
                        .unwrap_or_default();
                        if ddl.trim().is_empty() {
                            return Err("未找到该对象（可能已被删除）".to_string());
                        }
                        return Ok(ObjectDefinition {
                            definition: ddl,
                            call_template: String::new(),
                            arg_count: 0,
                        });
                    }
                    let obj_type = if kind_o == "function" { "FUNCTION" } else { "PROCEDURE" };
                    let rows = dm_query_rows(
                        client,
                        "SELECT TEXT FROM ALL_SOURCE WHERE OWNER = ? AND NAME = ? AND TYPE = ? \
                         ORDER BY LINE",
                        &[
                            dm_pure::Param::Str(schema_o.clone()),
                            dm_pure::Param::Str(name_o.clone()),
                            dm_pure::Param::Str(obj_type.to_string()),
                        ],
                    )
                    .map_err(|e| format!("达梦拉取定义失败：{e}"))?;
                    let definition = rows
                        .iter()
                        .map(|row| dm_text_at(row, 0))
                        .collect::<Vec<_>>()
                        .join("");
                    // ALL_SOURCE 无行时降级 GET_DDL：系统内置过程（SYSDBA 的
                    // SP_ARCH_*/SP_DB_* 等）源码不对外暴露但 GET_DDL 可出规约
                    //（真机实测 ALL_SOURCE 查不到、GET_DDL 待验——两者都空则
                    // 明确报「无源码」而非误报「可能已被删除」）
                    if definition.trim().is_empty() {
                        let ddl = dm_query_first_string(
                            client,
                            &format!(
                                "SELECT DBMS_METADATA.GET_DDL('{obj_type}', ?, ?) FROM DUAL"
                            ),
                            &[
                                dm_pure::Param::Str(name_o.clone()),
                                dm_pure::Param::Str(schema_o.clone()),
                            ],
                        )
                        .unwrap_or_default();
                        if ddl.trim().is_empty() {
                            return Err(format!(
                                "{name_o}：ALL_SOURCE 与 GET_DDL 均无源码（系统内置对象？）"
                            ));
                        }
                        return Ok(ObjectDefinition {
                            definition: ddl,
                            call_template: String::new(),
                            arg_count: 0,
                        });
                    }
                    let arg_count = dm_query_rows(
                        client,
                        "SELECT COUNT(*) FROM ALL_ARGUMENTS WHERE OWNER = ? AND OBJECT_NAME = ? \
                         AND ARGUMENT_NAME IS NOT NULL AND DATA_LEVEL = 0",
                        &[
                            dm_pure::Param::Str(schema_o.clone()),
                            dm_pure::Param::Str(name_o.clone()),
                        ],
                    )
                    .ok()
                    .and_then(|rows| rows.first().map(|row| dm_row_i64(row, 0)))
                    .unwrap_or(0);
                    let call_template = if kind_o == "function" {
                        // 达梦 Oracle 兼容模式下函数调用需要 FROM DUAL
                        format!("SELECT \"{schema_o}\".\"{name_o}\"() FROM DUAL")
                    } else {
                        format!("CALL \"{schema_o}\".\"{name_o}\"()")
                    };
                    Ok(ObjectDefinition {
                        definition,
                        call_template,
                        arg_count: arg_count.max(0) as usize,
                    })
                })
                .await
}
