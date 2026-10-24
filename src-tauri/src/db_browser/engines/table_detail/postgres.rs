//! PostgreSQL 的表详情。

use super::super::TableDetail;
use super::super::format_bytes;

pub(crate) async fn detail(
    client: &mut tokio_postgres::Client,
    schema: &str,
    table: &str,
    kind: &str,
) -> Result<TableDetail, String> {
    let q = |v: &str| format!("\"{}\"", v.replace('"', "\"\""));
    // 列结构 + 注释（DBeaver PostgreSchema 同源：col_description
    // 取列级注释）
    let col_rows = client
        .query(
            "SELECT column_name, data_type, is_nullable, column_default, \
                    col_description(format('%I.%I', table_schema, table_name)::regclass::oid, ordinal_position) \
             FROM information_schema.columns \
             WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
            &[&schema, &table],
        )
        .await
        .map_err(|e| format!("PostgreSQL 查询列结构失败：{e}"))?;
    let columns: Vec<Vec<String>> = col_rows
        .iter()
        .filter_map(|row| {
            Some(vec![
                row.try_get::<_, String>(0).ok()?,
                row.try_get::<_, String>(1).ok()?,
                row.try_get::<_, String>(2).ok()?,
                row.try_get::<_, Option<String>>(3)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "NULL".into()),
                row.try_get::<_, Option<String>>(4)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
            ])
        })
        .collect();

    let index_rows = client
        .query(
            "SELECT indexname, indexdef FROM pg_indexes \
             WHERE schemaname = $1 AND tablename = $2 ORDER BY indexname",
            &[&schema, &table],
        )
        .await
        .map_err(|e| format!("PostgreSQL 查询索引失败：{e}"))?;
    let indexes: Vec<Vec<String>> = index_rows
        .iter()
        .filter_map(|row| {
            Some(vec![
                row.try_get::<_, String>(0).ok()?,
                row.try_get::<_, String>(1).ok()?,
            ])
        })
        .collect();

    // DDL：视图用 pg_get_viewdef；表由列元数据 + 主键拼接，并附真实索引 DDL
    let ddl = if kind == "view" {
        let def = client
            .query_opt(
                "SELECT pg_get_viewdef(format('%I.%I', $1, $2)::regclass, true)",
                &[&schema, &table],
            )
            .await
            .map_err(|e| format!("PostgreSQL 拉取视图定义失败：{e}"))?
            .and_then(|row| row.try_get::<_, String>(0).ok())
            .unwrap_or_default();
        format!("CREATE OR REPLACE VIEW {}.{} AS\n{def}", q(schema), q(table))
    } else {
        let pk_cols: Vec<String> = client
            .query(
                "SELECT kcu.column_name \
                 FROM information_schema.table_constraints tc \
                 JOIN information_schema.key_column_usage kcu \
                   ON tc.constraint_name = kcu.constraint_name \
                  AND tc.constraint_schema = kcu.constraint_schema \
                 WHERE tc.constraint_type = 'PRIMARY KEY' \
                   AND tc.table_schema = $1 AND tc.table_name = $2 \
                 ORDER BY kcu.ordinal_position",
                &[&schema, &table],
            )
            .await
            .map_err(|e| format!("PostgreSQL 查询主键失败：{e}"))?
            .iter()
            .filter_map(|row| row.try_get::<_, String>(0).ok())
            .collect();
        let mut lines: Vec<String> = columns.iter().map(|c| {
            let mut line = format!("  {} {}", q(c.first().unwrap_or(&String::new())), c.get(1).cloned().unwrap_or_default());
            if c.get(2).map(|v| v == "NO").unwrap_or(false) {
                line.push_str(" NOT NULL");
            }
            line
        }).collect();
        if !pk_cols.is_empty() {
            let joined = pk_cols.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ");
            lines.push(format!("  PRIMARY KEY ({joined})"));
        }
        let body = lines.join(",\n");
        let mut ddl = format!("CREATE TABLE {}.{} (\n{body}\n);", q(schema), q(table));
        if !indexes.is_empty() {
            ddl.push_str("\n\n-- Indexes:\n");
            for row in &indexes {
                if let Some(def) = row.get(1) {
                    ddl.push_str(def);
                    ddl.push(';');
                    ddl.push('\n');
                }
            }
        }
        ddl
    };

    // 约束（DBeaver PostgreSchema.ConstraintCache 同源）：
    // pg_get_constraintdef 直接给出 "PRIMARY KEY (a,b)" /
    // "FOREIGN KEY (x) REFERENCES t(y)" 完整定义文本
    let cons_rows = client
        .query(
            "SELECT conname, contype::char::text, pg_get_constraintdef(oid) \
             FROM pg_catalog.pg_constraint \
             WHERE conrelid = format('%I.%I', $1, $2)::regclass \
             ORDER BY conname",
            &[&schema, &table],
        )
        .await
        .map_err(|e| format!("PostgreSQL 查询约束失败：{e}"))?;
    let constraints: Vec<Vec<String>> = cons_rows
        .iter()
        .filter_map(|row| {
            Some(vec![
                row.try_get::<_, String>(0).ok()?,
                // contype 单字符：p/u/f/c → PK/U/F/C
                match row.try_get::<_, String>(1).ok()?.as_str() {
                    "p" => "PK".to_string(),
                    "u" => "U".to_string(),
                    "f" => "F".to_string(),
                    _ => "C".to_string(),
                },
                row.try_get::<_, String>(2).ok()?,
                String::new(),
            ])
        })
        .collect();

    // 表注释 + 大小 + 行数（DBeaver PostgreTableReal 同源）：
    // obj_description 取表注释；pg_total_relation_size 取总占用（含
    // 索引与 TOAST）；行数取 pg_class.reltuples（统计近似，VACUUM
    // 后更新——避免对大表做 COUNT）
    let stat_row = client
        .query_opt(
            "SELECT obj_description(c.oid, 'pg_class'), \
                    pg_catalog.pg_total_relation_size(c.oid), c.reltuples::bigint \
             FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('r','p')",
            &[&schema, &table],
        )
        .await;
    let (comment, size, row_count) = match stat_row {
        Ok(Some(row)) => (
            row.try_get::<_, Option<String>>(0)
                .ok()
                .flatten()
                .unwrap_or_default(),
            format_bytes(
                row.try_get::<_, i64>(1).unwrap_or(0),
            ),
            {
                let rows = row.try_get::<_, i64>(2).unwrap_or(-1);
                if rows >= 0 { rows.to_string() } else { String::new() }
            },
        ),
        _ => (String::new(), String::new(), String::new()),
    };

    // 反向外键（谁引用了本表）：pg_get_constraintdef 给完整定义
    // 文本（含 REFERENCES 目标与 ON DELETE/UPDATE 规则）
    let ref_rows = client
        .query(
            "SELECT con.conname, ns.nspname || '.' || rel.relname, \
                    pg_get_constraintdef(con.oid) \
             FROM pg_catalog.pg_constraint con \
             JOIN pg_catalog.pg_class rel ON rel.oid = con.conrelid \
             JOIN pg_catalog.pg_namespace ns ON ns.oid = rel.relnamespace \
             WHERE con.confrelid = format('%I.%I', $1, $2)::regclass \
               AND con.contype = 'f' \
             ORDER BY 2, 1",
            &[&schema, &table],
        )
        .await
        .unwrap_or_default();
    let incoming: Vec<Vec<String>> = ref_rows
        .iter()
        .filter_map(|row| {
            Some(vec![
                row.try_get::<_, String>(0).ok()?,
                row.try_get::<_, String>(1).ok()?,
                row.try_get::<_, String>(2).ok()?,
            ])
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
        index_headers: vec!["Index".into(), "Definition".into()],
        indexes,
        constraint_headers: vec![
            "Constraint".into(),
            "Type".into(),
            "Definition".into(),
            "References".into(),
        ],
        constraints,
        reference_headers: vec![
            "Constraint".into(),
            "Table".into(),
            "Definition".into(),
        ],
        references: incoming,
    })
}
