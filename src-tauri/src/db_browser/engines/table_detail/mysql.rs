//! MySQL / OceanBase MySQL 模式的表详情。
//!
//! OB Oracle 租户没有 information_schema：转 `oracle_engine::ob_oracle_table_detail`
//! （ODC `OBOracleSchemaAccessor` 同口径）。

use super::super::{oracle_engine, MySqlColumnRow, TableDetail};
use super::super::{apply_index_expr_overlay, format_bytes};

pub(crate) async fn detail(
    conn: &mut mysql_async::Conn,
    schema: &str,
    table: &str,
    kind: &str,
) -> Result<TableDetail, String> {
    // OB Oracle 租户没有 information_schema：表详情走 ALL_* 数据
    // 字典（ODC `OBOracleSchemaAccessor` 同口径，DDL 优先 OB 扩展
    // 的 SHOW CREATE TABLE）
    if conn.ob_is_oracle_mode() {
        return oracle_engine::ob_oracle_table_detail(conn, schema, table, kind).await;
    }
    use mysql_async::prelude::Queryable as _;
    let s = schema.replace('`', "``");
    let n = table.replace('`', "``");
    let verb = if kind == "view" { "VIEW" } else { "TABLE" };
    let mut result = conn
        .query_iter(format!("SHOW CREATE {verb} `{s}`.`{n}`"))
        .await
        .map_err(|e| format!("MySQL 拉取 DDL 失败：{e}"))?;
    let create_idx = result
        .columns_ref()
        .iter()
        .position(|c| c.name_str().to_lowercase().contains("create"))
        .ok_or_else(|| "SHOW CREATE 结果中未找到定义列".to_string())?;
    let rows: Vec<mysql_async::Row> = result
        .collect()
        .await
        .map_err(|e| format!("MySQL 读取 DDL 失败：{e}"))?;
    let ddl = rows
        .into_iter()
        .next()
        .and_then(|row| row.get::<String, usize>(create_idx))
        .unwrap_or_default();

    let col_rows: Vec<MySqlColumnRow> = conn
        .exec(
            "SELECT column_name, column_type, is_nullable, column_default, column_comment \
             FROM information_schema.columns \
             WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
            (schema, table),
        )
        .await
        .map_err(|e| format!("MySQL 查询列结构失败：{e}"))?;
    let columns: Vec<Vec<String>> = col_rows
        .iter()
        .map(|(name, ty, nullable, default, comment)| {
            vec![
                name.clone(),
                ty.clone(),
                nullable.clone(),
                default.clone().unwrap_or_else(|| "NULL".into()),
                comment.clone().unwrap_or_default(),
            ]
        })
        .collect();

    // 每索引一行：名字 + 唯一性 + 列清单（GROUP_CONCAT 聚合）
    let idx_rows: Vec<(String, i64, String)> = conn
        .exec(
            "SELECT index_name, MIN(non_unique), \
             GROUP_CONCAT(column_name ORDER BY seq_in_index SEPARATOR ', ') \
             FROM information_schema.statistics \
             WHERE table_schema = ? AND table_name = ? GROUP BY index_name ORDER BY index_name",
            (schema, table),
        )
        .await
        .map_err(|e| format!("MySQL 查询索引失败：{e}"))?;
    // 表达式索引列（MySQL 8.0.13+ 函数索引）：EXPRESSION 部件的
    // COLUMN_NAME 是系统占位名，真实定义在 EXPRESSION 列——聚列
    // 仍走 GROUP_CONCAT，按（索引名, 序号）overlay 回填。
    // EXPRESSION 是 MySQL 8.0.13+ 专有列：按服务端版本/分支先判定，
    // 老版本与 MariaDB 直接跳过——否则每次表详情都发一条注定失败的
    // 查询（除了白耗一次往返，错误日志里也会留痕）
    let expr_rows: Vec<(String, i64, String)> =
        if !conn.is_mariadb() && conn.server_version() >= (8, 0, 13) {
            conn.exec(
                "SELECT INDEX_NAME, SEQ_IN_INDEX, EXPRESSION \
                 FROM information_schema.statistics \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND EXPRESSION IS NOT NULL",
                (schema, table),
            )
            .await
            .unwrap_or_default()
        } else {
            Vec::new()
        };
    let idx_exprs: std::collections::HashMap<(String, i64), String> = expr_rows
        .into_iter()
        .map(|(n, s, e)| ((n, s), e))
        .collect();
    let indexes: Vec<Vec<String>> = idx_rows
        .iter()
        .map(|(name, non_unique, cols)| {
            vec![
                name.clone(),
                if *non_unique == 0 { "UNIQUE".into() } else { "".into() },
                apply_index_expr_overlay(cols, name, &idx_exprs),
            ]
        })
        .collect();

    // 约束（DBeaver MySQLCatalog 同源）：kcu 按约束名聚合列；
    // REFERENTIAL_CONSTRAINTS 补外键引用目标与 UPDATE/DELETE 规则
    //（DBeaver MySQLCatalog 同源）。GROUP_CONCAT 聚列
    // 行形态：name / type / 聚合列 / 引用表 / 引用列 / update_rule / delete_rule
    //
    // GROUP BY 必须含 CONSTRAINT_SCHEMA：第 5 列是关联子查询，引用了
    // kcu.CONSTRAINT_SCHEMA；MySQL 8 默认 sql_mode 的 ONLY_FULL_GROUP_BY
    // 会以「Expression #5 ... not in GROUP BY clause」直接拒绝整条查询
    //（5.7+ 默认即如此，MariaDB 默认不开）。
    type MySqlConstraintRow = (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let cons_rows: Vec<MySqlConstraintRow> =
        conn.exec(
            "SELECT kcu.CONSTRAINT_NAME, \
               CASE tc.CONSTRAINT_TYPE \
                 WHEN 'PRIMARY KEY' THEN 'PK' \
                 WHEN 'UNIQUE' THEN 'U' \
                 WHEN 'FOREIGN KEY' THEN 'F' \
                 WHEN 'CHECK' THEN 'C' END, \
               GROUP_CONCAT(kcu.COLUMN_NAME ORDER BY kcu.ORDINAL_POSITION), \
               rc.REFERENCED_TABLE_NAME, \
               (SELECT GROUP_CONCAT(kcu2.COLUMN_NAME ORDER BY kcu2.ORDINAL_POSITION) \
                FROM information_schema.KEY_COLUMN_USAGE kcu2 \
                WHERE kcu2.CONSTRAINT_SCHEMA = kcu.CONSTRAINT_SCHEMA \
                  AND kcu2.CONSTRAINT_NAME = kcu.CONSTRAINT_NAME \
                  AND kcu2.REFERENCED_COLUMN_NAME IS NOT NULL), \
               CASE WHEN rc.UPDATE_RULE IS NULL OR rc.UPDATE_RULE = 'NO ACTION' \
                 THEN '' ELSE CONCAT(' ON UPDATE ', rc.UPDATE_RULE) END, \
               CASE WHEN rc.DELETE_RULE IS NULL OR rc.DELETE_RULE = 'NO ACTION' \
                 THEN '' ELSE CONCAT(' ON DELETE ', rc.DELETE_RULE) END \
             FROM information_schema.KEY_COLUMN_USAGE kcu \
             JOIN information_schema.TABLE_CONSTRAINTS tc \
               ON tc.CONSTRAINT_SCHEMA = kcu.CONSTRAINT_SCHEMA \
              AND tc.CONSTRAINT_NAME = kcu.CONSTRAINT_NAME \
              AND tc.TABLE_NAME = kcu.TABLE_NAME \
             LEFT JOIN information_schema.REFERENTIAL_CONSTRAINTS rc \
               ON rc.CONSTRAINT_SCHEMA = kcu.CONSTRAINT_SCHEMA \
              AND rc.CONSTRAINT_NAME = kcu.CONSTRAINT_NAME \
             WHERE kcu.TABLE_SCHEMA = ? AND kcu.TABLE_NAME = ? \
               AND kcu.REFERENCED_TABLE_NAME IS NULL \
             GROUP BY kcu.CONSTRAINT_SCHEMA, kcu.CONSTRAINT_NAME, tc.CONSTRAINT_TYPE, \
               rc.REFERENCED_TABLE_NAME, rc.UPDATE_RULE, rc.DELETE_RULE",
            (schema, table),
        )
        .await
        .map_err(|e| format!("MySQL 查询约束失败：{e}"))?;
    let constraints: Vec<Vec<String>> = cons_rows
        .iter()
        .map(|(name, ctype, cols, ref_table, ref_cols, upd_rule, del_rule)| {
            let mut references = match (ref_table.as_deref(), ref_cols.as_deref()) {
                (Some(t), Some(c)) if !t.is_empty() => format!("{t}({c})"),
                _ => String::new(),
            };
            // ON UPDATE/DELETE 规则后缀（非 NO ACTION 才展示）：
            // "schema.table(col) ON DELETE CASCADE"
            if !references.is_empty() {
                if let Some(u) = upd_rule.as_deref().filter(|s| !s.is_empty()) {
                    references.push_str(u);
                }
                if let Some(d) = del_rule.as_deref().filter(|s| !s.is_empty()) {
                    references.push_str(d);
                }
            }
            vec![name.clone(), ctype.clone(), cols.clone(), references]
        })
        .collect();

    // 表注释 + 表大小 + 行数（DBeaver MySQLCatalog 同源）：
    // information_schema.TABLES 一条查询带全（TABLE_COMMENT /
    // DATA_LENGTH+INDEX_LENGTH / TABLE_ROWS——行数是统计近似值）
    let stat_row: Option<(Option<String>, i64, i64)> = conn
        .exec_first(
            "SELECT TABLE_COMMENT, (DATA_LENGTH + INDEX_LENGTH), TABLE_ROWS \
             FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND TABLE_TYPE = 'BASE TABLE'",
            (schema, table),
        )
        .await
        .unwrap_or(None);
    let (comment, size, row_count) = match stat_row {
        Some((c, bytes, rows)) => (
            c.unwrap_or_default(),
            format_bytes(bytes),
            if rows >= 0 { rows.to_string() } else { String::new() },
        ),
        None => (String::new(), String::new(), String::new()),
    };

    // 反向外键（谁引用了本表；DBeaver References Tab 同义）：
    // 按被引用侧定位，聚合引用方表/列与本表被引用列
    let ref_rows: Vec<(String, String, String, Option<String>)> = conn
        .exec(
            "SELECT kcu.CONSTRAINT_NAME, \
               CONCAT(kcu.TABLE_SCHEMA, '.', kcu.TABLE_NAME), \
               GROUP_CONCAT(kcu.COLUMN_NAME ORDER BY kcu.ORDINAL_POSITION), \
               GROUP_CONCAT(kcu.REFERENCED_COLUMN_NAME ORDER BY kcu.ORDINAL_POSITION) \
             FROM information_schema.KEY_COLUMN_USAGE kcu \
             WHERE kcu.REFERENCED_TABLE_SCHEMA = ? AND kcu.REFERENCED_TABLE_NAME = ? \
             GROUP BY kcu.TABLE_SCHEMA, kcu.TABLE_NAME, kcu.CONSTRAINT_NAME \
             ORDER BY kcu.TABLE_SCHEMA, kcu.TABLE_NAME, kcu.CONSTRAINT_NAME",
            (schema, table),
        )
        .await
        .unwrap_or_default();
    let incoming: Vec<Vec<String>> = ref_rows
        .into_iter()
        .map(|(name, tbl, cols, ref_cols)| {
            vec![name, tbl, cols, ref_cols.unwrap_or_default()]
        })
        .collect();

    Ok(TableDetail {
        ddl,
        comment,
        size,
        row_count,
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
        references: incoming,
    })
}
