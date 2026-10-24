//! SQLite 的表详情。

use super::super::{DbEngine, TableDetail};
use super::super::{quote_ident, quote_string_literal, SqliteIncomingRow};

pub(crate) async fn detail(
    engine: &mut DbEngine,
    schema: &str,
    table: &str,
) -> Result<TableDetail, String> {
    // DDL 直接取 sqlite_master.sql（建表原句，含列定义与约束）；
    // 内部表（sqlite_ 前缀）sql 为 NULL 时降级用列元数据拼装。
    // 列结构 PRAGMA table_info；索引 PRAGMA index_list + index_info
    //（后者按 seqno 排序拼列名——index_list 只给索引名不给列）
    let schema_o = schema.to_string();
    let table_o = table.to_string();
    engine
        .with_sqlite(move |conn| {
            let schema_q = quote_ident(&schema_o)?;
            let table_lit = quote_string_literal(&table_o);
            let sql = format!(
                "SELECT sql FROM {schema_q}.sqlite_master \
                 WHERE name = ?1 AND type IN ('table','view')"
            );
            let ddl = conn
                .query_row(&sql, [&table_o], |row| row.get::<_, Option<String>>(0))
                .ok()
                .flatten()
                .unwrap_or_default();

            let col_sql = format!("PRAGMA {schema_q}.table_info({table_lit})");
            // table_info 列序：cid(0) name(1) type(2) notnull(3)
            // dflt_value(4) pk(5)。语句/行迭代器放进块作用域，出块即释放
            //（Statement 借用 conn，不能与后续 prepare 同时活跃）
            let columns: Vec<Vec<String>> = {
                let mut stmt = conn
                    .prepare(&col_sql)
                    .map_err(|e| format!("SQLite 查询列结构失败：{e}"))?;
                let mut rows = stmt
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                            row.get::<_, i64>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, i64>(5)?,
                        ))
                    })
                    .map_err(|e| format!("SQLite 查询列结构失败：{e}"))?;
                let mut out: Vec<Vec<String>> = Vec::new();
                for row in rows.by_ref() {
                    let (name, data_type, notnull, dflt, pk) =
                        row.map_err(|e| format!("SQLite 读取列结构失败：{e}"))?;
                    out.push(vec![
                        name,
                        data_type,
                        if notnull == 0 {
                            "YES".into()
                        } else {
                            "NO".into()
                        },
                        dflt.unwrap_or_else(|| "NULL".into()),
                        pk.to_string(),
                    ]);
                }
                out
            };

            let idx_sql = format!("PRAGMA {schema_q}.index_list({table_lit})");
            // index_list 列序：seq(0) name(1) unique(2) origin(3) partial(4)
            let raw_indexes: Vec<(String, i64)> = {
                let mut stmt = conn
                    .prepare(&idx_sql)
                    .map_err(|e| format!("SQLite 查询索引失败：{e}"))?;
                let mut rows = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
                    })
                    .map_err(|e| format!("SQLite 查询索引失败：{e}"))?;
                let mut out: Vec<(String, i64)> = Vec::new();
                for row in rows.by_ref() {
                    out.push(row.map_err(|e| format!("SQLite 读取索引失败：{e}"))?);
                }
                out
            };

            let mut indexes: Vec<Vec<String>> = Vec::with_capacity(raw_indexes.len());
            for (index_name, unique) in raw_indexes {
                let info_sql = format!(
                    "PRAGMA {schema_q}.index_info({})",
                    quote_string_literal(&index_name)
                );
                // index_info 列序：seqno(0) cid(1) name(2)。表达式索引的
                // 列名是 NULL（源码 sqlite3VdbeMultiLoad 传 0 → OP_Null），
                // 跳过——否则空项会混进列清单
                let mut cols: Vec<(i64, String)> = {
                    let mut stmt = conn
                        .prepare(&info_sql)
                        .map_err(|e| format!("SQLite 查询索引列失败：{e}"))?;
                    let mut rows = stmt
                        .query_map([], |row| {
                            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(2)?))
                        })
                        .map_err(|e| format!("SQLite 查询索引列失败：{e}"))?;
                    let mut out: Vec<(i64, String)> = Vec::new();
                    for row in rows.by_ref() {
                        let (seqno, name) =
                            row.map_err(|e| format!("SQLite 读取索引列失败：{e}"))?;
                        if let Some(name) = name.filter(|n| !n.is_empty()) {
                            out.push((seqno, name));
                        }
                    }
                    out
                };
                cols.sort_by_key(|(seqno, _)| *seqno);
                let joined = cols
                    .into_iter()
                    .map(|(_, name)| name)
                    .collect::<Vec<_>>()
                    .join(", ");
                indexes.push(vec![
                    index_name,
                    if unique != 0 {
                        "UNIQUE".into()
                    } else {
                        String::new()
                    },
                    joined,
                ]);
            }

            // DDL 降级：内部表/异常形态无 sql 文本，用列元数据拼装
            let ddl = if ddl.trim().is_empty() {
                let q = |v: &str| format!("\"{}\"", v.replace('"', "\"\""));
                let lines: Vec<String> = columns
                    .iter()
                    .map(|c| {
                        let mut line = format!(
                            "  {} {}",
                            q(c.first().unwrap_or(&String::new())),
                            c.get(1).cloned().unwrap_or_default()
                        );
                        if c.get(2).map(|v| v == "NO").unwrap_or(false) {
                            line.push_str(" NOT NULL");
                        }
                        line
                    })
                    .collect();
                format!(
                    "CREATE TABLE {}.{} (\n{}\n);",
                    q(&schema_o),
                    q(&table_o),
                    lines.join(",\n")
                )
            } else {
                ddl
            };

            // 反向外键（谁引用了本表）：pragma_foreign_key_list 表值
            // 函数与 sqlite_master 一条 JOIN 查全库，按（表, 外键 id）
            // 分组聚列（PRAGMA 行按 id,seq 顺序产出，GROUP_CONCAT 扫描
            // 序即列序）；on_update/on_delete 非 NO ACTION 时拼后缀。
            // 查询失败（旧版 SQLite 无表值函数）降级为空 Tab
            let inc_sql = format!(
                "SELECT m.name, CAST(fk.id AS TEXT), \
                   GROUP_CONCAT(fk.\"from\"), GROUP_CONCAT(fk.\"to\"), \
                   MAX(fk.on_update), MAX(fk.on_delete) \
                 FROM {schema_q}.sqlite_master m \
                 JOIN pragma_foreign_key_list(m.name) fk \
                 WHERE m.type = 'table' AND fk.\"table\" = ?1 \
                 GROUP BY m.name, fk.id ORDER BY m.name, fk.id"
            );
            let incoming: Vec<Vec<String>> = {
                let rows: Vec<SqliteIncomingRow> = {
                    let mut stmt = conn
                        .prepare(&inc_sql)
                        .map_err(|e| format!("SQLite 查询外键引用失败：{e}"))?;
                    let mut rows = stmt
                        .query_map([&table_o], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, Option<String>>(2)?,
                                row.get::<_, Option<String>>(3)?,
                                row.get::<_, Option<String>>(4)?,
                                row.get::<_, Option<String>>(5)?,
                            ))
                        })
                        .map_err(|e| format!("SQLite 查询外键引用失败：{e}"))?;
                    let mut out = Vec::new();
                    for row in rows.by_ref() {
                        out.push(
                            row.map_err(|e| format!("SQLite 读取外键引用失败：{e}"))?,
                        );
                    }
                    out
                };
                rows.into_iter()
                    .map(|(tbl, id, cols, ref_cols, on_upd, on_del)| {
                        let mut refs = ref_cols.unwrap_or_default();
                        for (kw, rule) in [("ON UPDATE", on_upd), ("ON DELETE", on_del)] {
                            if let Some(r) = rule.filter(|r| {
                                !r.is_empty() && r != "NO ACTION" && r != "NONE"
                            }) {
                                refs.push_str(&format!(" {kw} {r}"));
                            }
                        }
                        vec![id, tbl, cols.unwrap_or_default(), refs]
                    })
                    .collect()
            };

            Ok(TableDetail {
                ddl,
                comment: String::new(),
                size: String::new(),
                row_count: String::new(),
                column_headers: vec![
                    "Column".into(),
                    "Type".into(),
                    "Nullable".into(),
                    "Default".into(),
                    "PK".into(),
                ],
                columns,
                index_headers: vec!["Index".into(), "Unique".into(), "Columns".into()],
                indexes,
                // SQLite 约束内嵌在建表 DDL 里（sqlite_master.sql 已含
                // PRIMARY KEY/UNIQUE/CHECK/REFERENCES 原文），不单独列；
                // 但反向外键可低成本查全库（pragma 表值函数 JOIN）
                constraint_headers: Vec::new(),
                constraints: Vec::new(),
                reference_headers: vec![
                    "Constraint".into(),
                    "Table".into(),
                    "Columns".into(),
                    "References".into(),
                ],
                references: incoming,
            })
        })
        .await
}
