//! 达梦的表详情。

use super::super::{DbEngine, format_bytes};
use super::super::{
    dm_query_first_string, dm_query_rows, dm_row_string, TableDetail,
};

pub(crate) async fn detail(
    engine: &mut DbEngine,
    schema: &str,
    table: &str,
    kind: &str,
) -> Result<TableDetail, String> {
    // 与 Oracle 同型：DBMS_METADATA.GET_DDL 首选（返回 CLOB，驱动可能
    // 给 Str 或 Bytes），失败降级：视图读 ALL_VIEWS.TEXT、表拼列元数据。
    // 达梦的 ALL_IND_COLUMNS 可用，但 LISTAGG 兼容性不稳，索引列在
    // Rust 侧聚合（ALL_INDEXES × ALL_IND_COLUMNS 按 INDEX_NAME 分组）
    let schema_o = schema.to_string();
    let table_o = table.to_string();
    let kind_o = kind.to_string();
    engine
        .with_dameng(move |client| {
            let object_type = if kind_o == "view" { "VIEW" } else { "TABLE" };
            let ddl = dm_query_first_string(
                client,
                "SELECT DBMS_METADATA.GET_DDL(?, ?, ?) FROM DUAL",
                &[
                    dm_pure::Param::Str(object_type.to_string()),
                    dm_pure::Param::Str(table_o.clone()),
                    dm_pure::Param::Str(schema_o.clone()),
                ],
            )
            .unwrap_or_default();

            // 列结构：DATA_DEFAULT 在 Oracle 系字典里是 LONG，部分驱动/版本
            // 读不了——首查失败就退到不带 Default 的三列（宁可少一列，
            // 不能让整张表的列结构都拉不出来）；左联 ALL_COL_COMMENTS
            // 取列注释（注释列失败退化为空，不影响主列结构）
            let col_rows = match dm_query_rows(
                client,
                "SELECT c.COLUMN_NAME, c.DATA_TYPE, c.NULLABLE, c.DATA_DEFAULT, cc.COMMENTS \
                 FROM ALL_TAB_COLUMNS c \
                 LEFT JOIN ALL_COL_COMMENTS cc \
                   ON cc.OWNER = c.OWNER AND cc.TABLE_NAME = c.TABLE_NAME \
                  AND cc.COLUMN_NAME = c.COLUMN_NAME \
                 WHERE c.OWNER = ? AND c.TABLE_NAME = ? \
                 ORDER BY c.COLUMN_ID",
                &[
                    dm_pure::Param::Str(schema_o.clone()),
                    dm_pure::Param::Str(table_o.clone()),
                ],
            ) {
                Ok(rows) => rows,
                Err(_) => dm_query_rows(
                    client,
                    "SELECT COLUMN_NAME, DATA_TYPE, NULLABLE \
                     FROM ALL_TAB_COLUMNS WHERE OWNER = ? AND TABLE_NAME = ? \
                     ORDER BY COLUMN_ID",
                    &[
                        dm_pure::Param::Str(schema_o.clone()),
                        dm_pure::Param::Str(table_o.clone()),
                    ],
                )
                .map_err(|e| format!("达梦查询列结构失败：{e}"))?,
            };
            let columns: Vec<Vec<String>> = col_rows
                .iter()
                .map(|row| {
                    vec![
                        dm_row_string(row, 0),
                        dm_row_string(row, 1),
                        // 字典的 Y/N 统一为另两引擎的 YES/NO 展示
                        match dm_row_string(row, 2).as_str() {
                            "Y" => "YES".to_string(),
                            "N" => "NO".to_string(),
                            other => other.to_string(),
                        },
                        // 降级路没有第四列：越界由 dm_row_string 回 NULL
                        dm_row_string(row, 3),
                        // 列注释（降级路越界同样回空）
                        dm_row_string(row, 4),
                    ]
                })
                .collect();

            // 索引：每行一个（索引名, 唯一性, 列名），按 INDEX_NAME 聚合成
            // 「一索引一行」（与 MySQL/Oracle 臂的展示口径一致）
            let idx_rows = dm_query_rows(
                client,
                "SELECT i.INDEX_NAME, i.UNIQUENESS, c.COLUMN_NAME \
                 FROM ALL_INDEXES i \
                 JOIN ALL_IND_COLUMNS c \
                   ON i.OWNER = c.INDEX_OWNER AND i.INDEX_NAME = c.INDEX_NAME \
                 WHERE i.TABLE_OWNER = ? AND i.TABLE_NAME = ? \
                 ORDER BY i.INDEX_NAME, c.COLUMN_POSITION",
                &[
                    dm_pure::Param::Str(schema_o.clone()),
                    dm_pure::Param::Str(table_o.clone()),
                ],
            )
            .unwrap_or_default();
            let mut indexes: Vec<Vec<String>> = Vec::new();
            // 行已按 INDEX_NAME 排序：与上一个索引同名则追加列，否则开新行。
            // 用 last_name 记账而非 match indexes.last_mut()——避免在
            // 新增分支里同时持有集合的可变借用
            let mut last_name: Option<String> = None;
            for row in &idx_rows {
                let name = dm_row_string(row, 0);
                let unique = if dm_row_string(row, 1).eq_ignore_ascii_case("UNIQUE") {
                    "UNIQUE".to_string()
                } else {
                    String::new()
                };
                let col = dm_row_string(row, 2);
                if last_name.as_deref() == Some(name.as_str()) {
                    if let Some(cells) = indexes.last_mut().and_then(|last| last.get_mut(2))
                    {
                        if !cells.is_empty() {
                            cells.push_str(", ");
                        }
                        cells.push_str(&col);
                    }
                } else {
                    indexes.push(vec![name.clone(), unique, col]);
                    last_name = Some(name);
                }
            }

            let ddl = if ddl.trim().is_empty() {
                if kind_o == "view" {
                    // 视图降级：ALL_VIEWS.TEXT 即查询体
                    let text = dm_query_first_string(
                        client,
                        "SELECT TEXT FROM ALL_VIEWS WHERE OWNER = ? AND VIEW_NAME = ?",
                        &[
                            dm_pure::Param::Str(schema_o.clone()),
                            dm_pure::Param::Str(table_o.clone()),
                        ],
                    )
                    .unwrap_or_default();
                    if text.trim().is_empty() {
                        "-- 无法获取 DDL（DBMS_METADATA 与 ALL_VIEWS 均无结果）".to_string()
                    } else {
                        text
                    }
                } else {
                    // 表降级：列元数据拼装（含 NOT NULL，与 Oracle 臂同型）
                    let q = |v: &str| format!("\"{}\"", v.replace('"', "\"\""));
                    let lines: Vec<String> = columns
                        .iter()
                        .map(|c| {
                            let mut line = format!(
                                "  {} {}",
                                q(c.first().map(String::as_str).unwrap_or("")),
                                c.get(1).cloned().unwrap_or_default()
                            );
                            if c.get(2).map(|v| v == "NO").unwrap_or(false) {
                                line.push_str(" NOT NULL");
                            }
                            line
                        })
                        .collect();
                    if lines.is_empty() {
                        "-- 无法获取 DDL".to_string()
                    } else {
                        format!(
                            "CREATE TABLE {}.{} (\n{}\n);",
                            q(&schema_o),
                            q(&table_o),
                            lines.join(",\n")
                        )
                    }
                }
            } else {
                ddl
            };

            // 约束清单与列（DBeaver DamengMetaModel.
            // prepareUniqueConstraintsLoadStatement 同源实证 SQL）：约束
            // 在 SYSCONS 基表，列不是逐约束存放，而是按 SYSINDEXES 的
            // KEYNUM/KEYINFO 位图解码（达梦内建函数 SF_COL_IS_IDX_KEY）；
            // CHECK 约束无索引（LISTAGG 的 CASE 分支为 NULL 不聚），
            // 文本在 SYSCONS.CHECKINFO。TYPE$ 映射 P/U/F/C。
            // 外键引用目标不在此 SQL（DBeaver 走 JDBC getImportedKeys
            // 协议），我们用达梦 Oracle 兼容视图 ALL_CONSTRAINTS/
            // ALL_CONS_COLUMNS 反查（社区实证口径，r_constraint_name
            // 关联同 Oracle）
            let cons_rows = dm_query_rows(
                client,
                "SELECT CONS_OBJ.NAME, SYSCONS.TYPE$, SYSCONS.CHECKINFO, \
                   LISTAGG(CASE WHEN SYSINDEXES.ID IS NULL THEN NULL \
                                ELSE SYSCOLUMNS.NAME END, ',') \
                     WITHIN GROUP (ORDER BY SYSCOLUMNS.COLID) AS COLS \
                 FROM SYSOBJECTS CONS_OBJ, SYSCOLUMNS SYSCOLUMNS, \
                      SYSOBJECTS TAB_OBJ, SYSOBJECTS SCH_OBJ, SYSCONS SYSCONS \
                 LEFT JOIN SYSINDEXES SYSINDEXES ON SYSINDEXES.ID = SYSCONS.INDEXID \
                 WHERE CONS_OBJ.ID = SYSCONS.ID \
                   AND SYSCOLUMNS.ID = SYSCONS.TABLEID \
                   AND CONS_OBJ.PID = TAB_OBJ.ID \
                   AND CONS_OBJ.SUBTYPE$ = 'CONS' \
                   AND (SF_COL_IS_IDX_KEY(SYSINDEXES.KEYNUM, SYSINDEXES.KEYINFO, SYSCOLUMNS.COLID) = 1 \
                        OR SYSINDEXES.ID IS NULL) \
                   AND TAB_OBJ.SCHID = SCH_OBJ.ID \
                   AND SCH_OBJ.NAME = ?1 AND TAB_OBJ.NAME = ?2 \
                 GROUP BY SYSCONS.ID, SYSCONS.VALID, CONS_OBJ.NAME, \
                          SYSCONS.TYPE$, SYSCONS.CHECKINFO, CONS_OBJ.CRTDATE, \
                          SYSINDEXES.XTYPE \
                 ORDER BY CONS_OBJ.NAME",
                &[
                    dm_pure::Param::Str(schema_o.clone()),
                    dm_pure::Param::Str(table_o.clone()),
                ],
            )
            .unwrap_or_default();
            let mut constraints: Vec<Vec<String>> = Vec::new();
            // F 约束名清单，引用目标随后统一反查
            let mut fk_names: Vec<String> = Vec::new();
            for row in &cons_rows {
                let name = dm_row_string(row, 0);
                let type_raw = dm_row_string(row, 1);
                let ctype = match type_raw.as_str() {
                    "P" => "PK",
                    "U" => "U",
                    "F" => "F",
                    _ => "C",
                }
                .to_string();
                if type_raw == "F" {
                    fk_names.push(name.clone());
                }
                // CHECK 约束列清单为 NULL，展示 CHECKINFO 文本
                let cols_raw = dm_row_string(row, 3);
                let cols = if cols_raw.is_empty() || cols_raw == "NULL" {
                    let check = dm_row_string(row, 2);
                    if check == "NULL" {
                        String::new()
                    } else {
                        check
                    }
                } else {
                    cols_raw
                };
                constraints.push(vec![name, ctype, cols, String::new()]);
            }
            // 外键目标 + 删除规则反查：先取外键自身的
            // r_owner/r_constraint_name/delete_rule（DELETE_RULE 列在
            // 老版本兼容视图可能缺失，首查失败降级不带规则重试），
            // 再取被引用约束的表名与目标列，拼 "table(cols) ON DELETE ..."
            for fk_name in &fk_names {
                let fk_self = dm_query_rows(
                    client,
                    "SELECT R_OWNER, R_CONSTRAINT_NAME, DELETE_RULE \
                     FROM ALL_CONSTRAINTS \
                     WHERE OWNER = ?1 AND CONSTRAINT_NAME = ?2 AND CONSTRAINT_TYPE = 'R'",
                    &[
                        dm_pure::Param::Str(schema_o.clone()),
                        dm_pure::Param::Str(fk_name.clone()),
                    ],
                )
                .or_else(|_| {
                    dm_query_rows(
                        client,
                        "SELECT R_OWNER, R_CONSTRAINT_NAME, NULL AS DELETE_RULE \
                         FROM ALL_CONSTRAINTS \
                         WHERE OWNER = ?1 AND CONSTRAINT_NAME = ?2 AND CONSTRAINT_TYPE = 'R'",
                        &[
                            dm_pure::Param::Str(schema_o.clone()),
                            dm_pure::Param::Str(fk_name.clone()),
                        ],
                    )
                })
                .unwrap_or_default();
                let Some(self_row) = fk_self.into_iter().next() else {
                    continue;
                };
                let r_owner = dm_row_string(&self_row, 0);
                let r_cons = dm_row_string(&self_row, 1);
                let del_rule = dm_row_string(&self_row, 2);
                if r_owner.is_empty()
                    || r_owner == "NULL"
                    || r_cons.is_empty()
                    || r_cons == "NULL"
                {
                    continue;
                }
                let ref_rows = dm_query_rows(
                    client,
                    "SELECT rc.TABLE_NAME || '(' || NVL((SELECT LISTAGG(cc.COLUMN_NAME, ',') \
                        WITHIN GROUP (ORDER BY cc.POSITION) \
                        FROM ALL_CONS_COLUMNS cc \
                        WHERE cc.OWNER = rc.OWNER AND cc.CONSTRAINT_NAME = rc.CONSTRAINT_NAME), '') || ')' \
                     FROM ALL_CONSTRAINTS rc \
                     WHERE rc.OWNER = ?1 AND rc.CONSTRAINT_NAME = ?2",
                    &[
                        dm_pure::Param::Str(r_owner),
                        dm_pure::Param::Str(r_cons),
                    ],
                )
                .unwrap_or_default();
                if let Some(row) = ref_rows.into_iter().next()
                    && let Some(cell) = constraints
                        .iter_mut()
                        .find(|c| c.first().map(String::as_str) == Some(fk_name.as_str()))
                    && let Some(r) = cell.get_mut(3)
                {
                    *r = dm_row_string(&row, 0);
                    let rule = del_rule.trim();
                    if !rule.is_empty() && rule != "NO ACTION" && rule != "NULL" {
                        r.push_str(" ON DELETE ");
                        r.push_str(rule);
                    }
                }
            }

            // 反向外键（谁引用了本表）：达梦 Oracle 兼容视图同型口径；
            // 第 3/4 列分别是引用方外键列与本表被引用列
            let inc_rows = dm_query_rows(
                client,
                "SELECT c.CONSTRAINT_NAME, c.OWNER || '.' || c.TABLE_NAME, \
                   (SELECT LISTAGG(cc.COLUMN_NAME, ',') WITHIN GROUP (ORDER BY cc.POSITION) \
                    FROM ALL_CONS_COLUMNS cc \
                    WHERE cc.OWNER = c.OWNER AND cc.CONSTRAINT_NAME = c.CONSTRAINT_NAME), \
                   (SELECT LISTAGG(cc2.COLUMN_NAME, ',') WITHIN GROUP (ORDER BY cc2.POSITION) \
                    FROM ALL_CONS_COLUMNS cc2 \
                    WHERE cc2.OWNER = c.R_OWNER AND cc2.CONSTRAINT_NAME = c.R_CONSTRAINT_NAME) \
                 FROM ALL_CONSTRAINTS c \
                 WHERE c.CONSTRAINT_TYPE = 'R' AND c.R_OWNER = ?1 \
                   AND c.R_CONSTRAINT_NAME IN (SELECT rc.CONSTRAINT_NAME FROM ALL_CONSTRAINTS rc \
                     WHERE rc.OWNER = ?1 AND rc.TABLE_NAME = ?2 \
                       AND rc.CONSTRAINT_TYPE IN ('P','U')) \
                 ORDER BY c.OWNER, c.TABLE_NAME, c.CONSTRAINT_NAME",
                &[
                    dm_pure::Param::Str(schema_o.clone()),
                    dm_pure::Param::Str(table_o.clone()),
                ],
            )
            .unwrap_or_default();
            let dm_incoming: Vec<Vec<String>> = inc_rows
                .iter()
                .map(|row| {
                    vec![
                        dm_row_string(row, 0),
                        dm_row_string(row, 1),
                        dm_row_string(row, 2),
                        dm_row_string(row, 3),
                    ]
                })
                .collect();

            // 表注释 + 表大小 + 行数（DBeaver DamengSchema 同源）：
            // ALL_TAB_COMMENTS 取注释；TABLE_USED_PAGES(OWNER,TABLE_NAME)*PAGE
            // 取磁盘占用（达梦专有函数）；ALL_TABLES.NUM_ROWS 统计近似
            let stat = dm_query_rows(
                client,
                "SELECT (SELECT COMMENTS FROM ALL_TAB_COMMENTS \
                   WHERE OWNER = ?1 AND TABLE_NAME = ?2 AND TABLE_TYPE = 'TABLE'), \
                   (SELECT TABLE_USED_PAGES(?1, ?2) * PAGE FROM DUAL), \
                   (SELECT NUM_ROWS FROM ALL_TABLES WHERE OWNER = ?1 AND TABLE_NAME = ?2) \
                 FROM DUAL",
                &[
                    dm_pure::Param::Str(schema_o.clone()),
                    dm_pure::Param::Str(table_o.clone()),
                ],
            );
            let (dm_comment, dm_size, dm_rows) = match
                stat.ok().and_then(|rows| rows.into_iter().next())
            {
                Some(row) => {
                    (
                        {
                            let raw = dm_row_string(&row, 0);
                            if raw == "NULL" { String::new() } else { raw }
                        },
                        {
                            let raw = dm_row_string(&row, 1);
                            if raw.is_empty() || raw == "NULL" {
                                String::new()
                            } else {
                                format_bytes(raw.parse::<i64>().unwrap_or(0))
                            }
                        },
                        {
                            let raw = dm_row_string(&row, 2);
                            if raw.is_empty() || raw == "NULL" {
                                String::new()
                            } else {
                                raw
                            }
                        },
                    )
                }
                None => (String::new(), String::new(), String::new()),
            };

            Ok(TableDetail {
                ddl,
                comment: dm_comment,
                size: dm_size,
                row_count: dm_rows,
                column_headers: vec![
                    "Column".into(),
                    "Type".into(),
                    "Nullable".into(),
                    "Default".into(),
                    "Comment".into(),
                ],
                columns,
                index_headers: vec!["Index".into(), "Unique".into(), "Columns".into()],
                indexes,
                constraint_headers: vec![
                    "Constraint".into(),
                    "Type".into(),
                    "Columns".into(),
                    "References".into(),
                ],
                constraints,
                reference_headers: vec![
                    "Constraint".into(),
                    "Table".into(),
                    "Columns".into(),
                    "References".into(),
                ],
                references: dm_incoming,
            })
        })
        .await
}
