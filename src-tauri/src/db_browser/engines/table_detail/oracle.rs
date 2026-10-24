//! Oracle 的表详情。

use oracle_async::Queryable as _;

use super::super::{
    apply_index_expr_overlay, oracle_dict_prefix,
};
use super::super::{
    ob_quote_literal, oracle_err, oracle_row_string, TableDetail,
};

pub(crate) async fn detail(
    conn: &mut oracle_async::Conn,
    schema: &str,
    table: &str,
    kind: &str,
) -> Result<TableDetail, String> {
    // DDL 首选 DBMS_METADATA.GET_DDL（需权限，失败降级为列元数据
    // 拼装——PG 同型语义）；列结构 all_tab_columns（data_default 是
    // LONG 类型、驱动不支持，故不带 Default 列）；索引
    // all_indexes + all_ind_columns 经 LISTAGG 聚列。
    // 字典视图前缀自适应（DBA 权限 → dba_*）
    let dict_p = oracle_dict_prefix(conn).await;
    let object_type = if kind == "view" { "VIEW" } else { "TABLE" };
    let ddl_sql = format!(
        "SELECT DBMS_METADATA.GET_DDL({}, {}, {}) FROM dual",
        ob_quote_literal(object_type),
        ob_quote_literal(table),
        ob_quote_literal(schema)
    );
    let ddl = match conn.query(&ddl_sql).await {
        Ok(mut rs) => match rs.next_row().await {
            Ok(Some(row)) => oracle_row_string(&row, 0),
            _ => String::new(),
        },
        Err(_) => String::new(),
    };

    let col_sql = format!(
        "SELECT c.column_name, c.data_type, c.nullable, cc.COMMENTS \
         FROM {dict_p}_tab_columns c \
         LEFT JOIN {dict_p}_col_comments cc \
           ON cc.owner = c.owner AND cc.table_name = c.table_name \
          AND cc.column_name = c.column_name \
         WHERE c.owner = {} AND c.table_name = {} ORDER BY c.column_id",
        ob_quote_literal(schema),
        ob_quote_literal(table)
    );
    let mut col_rs = conn
        .query(&col_sql)
        .await
        .map_err(|e| format!("Oracle 查询列结构失败：{}", oracle_err(e)))?;
    let mut columns: Vec<Vec<String>> = Vec::new();
    while let Some(row) = col_rs
        .next_row()
        .await
        .map_err(|e| format!("Oracle 读取列结构失败：{}", oracle_err(e)))?
    {
        let nullable = match row.values().get(2).map(|v| v.to_string()) {
            Some(s) if s == "Y" => "YES".to_string(),
            Some(s) if s == "N" => "NO".to_string(),
            _ => String::new(),
        };
        columns.push(vec![
            oracle_row_string(&row, 0),
            oracle_row_string(&row, 1),
            nullable,
            // 列注释（all_col_comments 无行为 NULL）
            oracle_row_string(&row, 3),
        ]);
    }

    // 函数索引表达式（DBeaver OracleSchema 同源走
    // ALL_IND_EXPRESSIONS）：函数索引的 all_ind_columns 列名是
    // SYS_NC 占位名，真实表达式在 COLUMN_EXPRESSION（LONG 类型
    // 进不了 LISTAGG，按位置单独取回回填）。查询失败（权限/
    // LONG 边界）按无表达式处理，聚列退回占位名
    let expr_sql = format!(
        "SELECT INDEX_NAME, COLUMN_POSITION, COLUMN_EXPRESSION \
          FROM {dict_p}_ind_expressions \
         WHERE INDEX_OWNER = {} AND TABLE_NAME = {} \
         ORDER BY INDEX_NAME, COLUMN_POSITION",
        ob_quote_literal(schema),
        ob_quote_literal(table)
    );
    let mut idx_exprs: std::collections::HashMap<(String, i64), String> =
        std::collections::HashMap::new();
    if let Ok(mut expr_rs) = conn.query(&expr_sql).await {
        while let Ok(Some(row)) = expr_rs.next_row().await {
            let pos = oracle_row_string(&row, 1).parse::<i64>().unwrap_or(0);
            let expr = oracle_row_string(&row, 2);
            if pos > 0 && expr != "NULL" {
                idx_exprs.insert((oracle_row_string(&row, 0), pos), expr);
            }
        }
    }
    let idx_sql = format!(
        "SELECT i.index_name, i.uniqueness, \
                LISTAGG(c.column_name, ', ') WITHIN GROUP \
                  (ORDER BY c.column_position) \
         FROM {dict_p}_indexes i \
         JOIN {dict_p}_ind_columns c \
           ON c.index_name = i.index_name AND c.index_owner = i.owner \
         WHERE i.table_owner = {} AND i.table_name = {} \
         GROUP BY i.index_name, i.uniqueness ORDER BY i.index_name",
        ob_quote_literal(schema),
        ob_quote_literal(table)
    );
    let mut idx_rs = conn
        .query(&idx_sql)
        .await
        .map_err(|e| format!("Oracle 查询索引失败：{}", oracle_err(e)))?;
    let mut indexes: Vec<Vec<String>> = Vec::new();
    while let Some(row) = idx_rs
        .next_row()
        .await
        .map_err(|e| format!("Oracle 读取索引失败：{}", oracle_err(e)))?
    {
        let name = oracle_row_string(&row, 0);
        let is_unique = if oracle_row_string(&row, 1) == "UNIQUE" {
            "UNIQUE".to_string()
        } else {
            String::new()
        };
        let cols = apply_index_expr_overlay(
            &oracle_row_string(&row, 2),
            &name,
            &idx_exprs,
        );
        indexes.push(vec![name, is_unique, cols]);
    }

    // DDL 降级：列元数据拼装（含 NOT NULL）
    let ddl = if ddl.is_empty() {
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
            q(schema),
            q(table),
            lines.join(",\n")
        )
    } else {
        ddl
    };

    // 约束（DBeaver OracleSchema.ConstraintCache 同源）：
    // all_constraints + all_cons_columns；外键经 r_constraint_name
    // 反查目标表，DELETE_RULE 非 NO ACTION 拼 " ON DELETE ..." 后缀。
    // 聚列方式按服务器版本智能分流（DBeaver 同型）：
    // - 11g+（LISTAGG 可用，Oracle 11.1 引入）
    // - 10g 及以下：SYS_CONNECT_BY_PATH 层次查询聚列
    //   （DBeaver OracleSchema:956-977 的 10g 分支同源实现）。
    // 聚列表达式按（owner, constraint）相关名参数化，出向约束、
    // 外键目标与反向外键三处复用
    let agg_11g = |owner: &str, cons: &str| {
        format!(
            "(SELECT LISTAGG(cc.COLUMN_NAME, ',') WITHIN GROUP (ORDER BY cc.POSITION) \
              FROM {dict_p}_cons_columns cc \
              WHERE cc.OWNER = {owner} AND cc.CONSTRAINT_NAME = {cons})"
        )
    };
    let agg_10g = |owner: &str, cons: &str| {
        format!(
            "(SELECT MAX(SUBSTR(SYS_CONNECT_BY_PATH(cc.COLUMN_NAME, ','), 2)) \
              FROM (SELECT cc.COLUMN_NAME, cc.POSITION, \
                           ROW_NUMBER() OVER (ORDER BY cc.POSITION) AS RN \
                    FROM {dict_p}_cons_columns cc \
                    WHERE cc.OWNER = {owner} AND cc.CONSTRAINT_NAME = {cons}) cc \
              START WITH cc.RN = 1 \
              CONNECT BY PRIOR cc.RN = cc.RN - 1)"
        )
    };
    let listagg_ok = conn.db_version_major() >= 11;
    let cols_agg = if listagg_ok {
        agg_11g("c.OWNER", "c.CONSTRAINT_NAME")
    } else {
        agg_10g("c.OWNER", "c.CONSTRAINT_NAME")
    };
    let cons_sql = format!(
        "SELECT c.CONSTRAINT_NAME, c.CONSTRAINT_TYPE, c.R_OWNER, c.R_CONSTRAINT_NAME, \
           c.DELETE_RULE, {cols_agg} AS COLS \
         FROM {dict_p}_constraints c \
         WHERE c.OWNER = {} AND c.TABLE_NAME = {} \
         ORDER BY c.CONSTRAINT_NAME",
        ob_quote_literal(schema),
        ob_quote_literal(table)
    );
    let mut cons_rs = conn
        .query(&cons_sql)
        .await
        .map_err(|e| format!("Oracle 查询约束失败：{}", oracle_err(e)))?;
    let mut constraints: Vec<Vec<String>> = Vec::new();
    // F 约束的（约束名, r_owner.r_constraint_name, delete_rule），
    // 引用目标随后统一反查
    let mut fk_refs: Vec<(String, String, String)> = Vec::new();
    while let Some(row) = cons_rs
        .next_row()
        .await
        .map_err(|e| format!("Oracle 读取约束失败：{}", oracle_err(e)))?
    {
        let name = oracle_row_string(&row, 0);
        let ctype = match oracle_row_string(&row, 1).as_str() {
            "P" => "PK".to_string(),
            "U" => "U".to_string(),
            "R" => "F".to_string(),
            _ => "C".to_string(),
        };
        if ctype == "F" {
            fk_refs.push((
                name.clone(),
                format!(
                    "{}.{}",
                    oracle_row_string(&row, 2),
                    oracle_row_string(&row, 3)
                ),
                oracle_row_string(&row, 4),
            ));
        }
        constraints.push(vec![name, ctype, oracle_row_string(&row, 5), String::new()]);
    }
    // 外键目标表名 + 目标列反查（r_constraint_name → 目标约束），
    // DELETE_RULE 非 NO ACTION 拼 "table(cols) ON DELETE CASCADE"
    for (fk_name, r_owner_name, del_rule) in &fk_refs {
        let parts: Vec<&str> = r_owner_name.splitn(2, '.').collect();
        let [owner_part, cons_part] = parts[..] else {
            continue;
        };
        let owner_lit = ob_quote_literal(owner_part);
        let cons_lit = ob_quote_literal(cons_part);
        let ref_agg = if listagg_ok {
            agg_11g(&owner_lit, &cons_lit)
        } else {
            agg_10g(&owner_lit, &cons_lit)
        };
        let ref_sql = format!(
            "SELECT NVL(TABLE_NAME || '(' || {ref_agg} || ')', TABLE_NAME) \
             FROM {dict_p}_constraints \
             WHERE OWNER = {owner_lit} AND CONSTRAINT_NAME = {cons_lit}"
        );
        let mut ref_rs = conn
            .query(&ref_sql)
            .await
            .map_err(|e| format!("Oracle 查询外键目标失败：{}", oracle_err(e)))?;
        if let Some(row) = ref_rs
            .next_row()
            .await
            .map_err(|e| format!("Oracle 读取外键目标失败：{}", oracle_err(e)))?
            && let Some(cell) = constraints
                .iter_mut()
                .find(|c| c.first().map(String::as_str) == Some(fk_name.as_str()))
            && let Some(r) = cell.get_mut(3)
        {
            *r = oracle_row_string(&row, 0);
            let rule = del_rule.trim();
            if !rule.is_empty() && rule != "NO ACTION" && rule != "NULL" {
                r.push_str(" ON DELETE ");
                r.push_str(rule);
            }
        }
    }

    // 反向外键（谁引用了本表）：本表的 PK/UK 被 r_constraint_name 指向；
    // 第 3/4 列分别是引用方外键列与本表被引用列
    let ref_cols_agg = if listagg_ok {
        agg_11g("c.R_OWNER", "c.R_CONSTRAINT_NAME")
    } else {
        agg_10g("c.R_OWNER", "c.R_CONSTRAINT_NAME")
    };
    let inc_sql = format!(
        "SELECT c.CONSTRAINT_NAME, c.OWNER || '.' || c.TABLE_NAME, \
           {cols_agg}, {ref_cols_agg} \
         FROM {dict_p}_constraints c \
         WHERE c.CONSTRAINT_TYPE = 'R' AND c.R_OWNER = {} \
           AND c.R_CONSTRAINT_NAME IN (SELECT rc.CONSTRAINT_NAME FROM {dict_p}_constraints rc \
             WHERE rc.OWNER = {} AND rc.TABLE_NAME = {} \
               AND rc.CONSTRAINT_TYPE IN ('P','U')) \
         ORDER BY c.OWNER, c.TABLE_NAME, c.CONSTRAINT_NAME",
        ob_quote_literal(schema),
        ob_quote_literal(schema),
        ob_quote_literal(table)
    );
    let mut inc_rs = conn
        .query(&inc_sql)
        .await
        .map_err(|e| format!("Oracle 查询外键引用失败：{}", oracle_err(e)))?;
    let mut incoming: Vec<Vec<String>> = Vec::new();
    while let Some(row) = inc_rs
        .next_row()
        .await
        .map_err(|e| format!("Oracle 读取外键引用失败：{}", oracle_err(e)))?
    {
        incoming.push(vec![
            oracle_row_string(&row, 0),
            oracle_row_string(&row, 1),
            oracle_row_string(&row, 2),
            oracle_row_string(&row, 3),
        ]);
    }

    // 表注释 + 行数（DBeaver OracleTableBase 同源）：
    // all_tab_comments 取注释；all_tables.NUM_ROWS 是统计近似
    // （ANALYZE 后更新）。Oracle 表大小走 dba_segments 但需要权限，
    // 取 all_tables.BLOCKS * 块大小(8KB) 近似——不做，取 NUM_ROWS 即可
    let stat_sql = format!(
         "SELECT (SELECT COMMENTS FROM {dict_p}_tab_comments \
           WHERE owner = {} AND table_name = {} AND table_type = 'TABLE'), \
           (SELECT NUM_ROWS FROM {dict_p}_tables \
           WHERE owner = {} AND table_name = {}) \
         FROM dual",
        ob_quote_literal(schema),
        ob_quote_literal(table),
        ob_quote_literal(schema),
        ob_quote_literal(table)
    );
    let mut stat_rs = conn
        .query(&stat_sql)
        .await
        .map_err(|e| format!("Oracle 查询表统计失败：{}", oracle_err(e)))?;
    let (comment, row_count) = if let Some(row) = stat_rs
        .next_row()
        .await
        .map_err(|e| format!("Oracle 读取表统计失败：{}", oracle_err(e)))?
    {
        (
            {
                // 空值渲染为字面 "NULL" 字符串——归一为空串，
                // 免得表详情注释徽标显示「NULL」
                let raw = oracle_row_string(&row, 0);
                if raw == "NULL" { String::new() } else { raw }
            },
            {
                let raw = oracle_row_string(&row, 1);
                if raw.is_empty() || raw == "NULL" {
                    String::new()
                } else {
                    raw
                }
            },
        )
    } else {
        (String::new(), String::new())
    };

    Ok(TableDetail {
        ddl,
        comment,
        size: String::new(),
        row_count,
        column_headers: vec![
            "Column".into(),
            "Type".into(),
            "Nullable".into(),
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
