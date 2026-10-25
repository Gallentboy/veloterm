//! ODBC 的表详情：标准 Catalog API 的最小结构视图。
//!
//! 通用 ODBC 没有 DDL 生成路（各库方言不可知），ddl 字段给提示文本；
//! 结构三块：SQLColumns（列/类型/可空）、SQLPrimaryKeys（主键）、
//! SQLStatistics（索引，列按 ORDINAL_POSITION 聚合）。

use std::collections::BTreeMap;

use super::super::{
    odbc_engine, odbc_is_null, odbc_row_string, odbc_split_qualified, DbEngine, TableDetail,
};

pub(crate) async fn detail(
    engine: &mut DbEngine,
    schema: &str,
    table: &str,
    _kind: &str,
) -> Result<TableDetail, String> {
    let catalog = schema.to_string();
    let qualified = table.to_string();
    engine
        .with_odbc(move |eng| {
            let api = odbc_engine::odbc_api()?;
            let (schem, tbl) = odbc_split_qualified(&qualified);
            let schem = schem.unwrap_or_default();

            // 列结构：SQLColumns 标准行序（0 基）：COLUMN_NAME=3 / TYPE_NAME=5 /
            // NULLABLE=10（0=NOT NULL，1=可空，2=未知）/ REMARKS=11 /
            // IS_NULLABLE=17（"YES"/"NO" 文本，部分驱动不填）
            let (_, rows) = api
                .columns_catalog(eng.dbc.0, &catalog, &schem, &tbl)
                .map_err(|e| format!("ODBC 查询列结构失败：{e}"))?;
            let mut columns: Vec<Vec<String>> = Vec::new();
            for row in &rows {
                let name = odbc_row_string(row, 3);
                if name.is_empty() || name == "NULL" {
                    continue;
                }
                let ty = odbc_row_string(row, 5);
                let nullable = if !odbc_is_null(row, 17) {
                    if odbc_row_string(row, 17).eq_ignore_ascii_case("YES") {
                        "YES".to_string()
                    } else {
                        "NO".to_string()
                    }
                } else if !odbc_is_null(row, 10) {
                    // NULLABLE 数值：0=NOT NULL，1/2=可空/未知
                    if odbc_row_string(row, 10) == "0" {
                        "NO".to_string()
                    } else {
                        "YES".to_string()
                    }
                } else {
                    String::new()
                };
                let comment = if odbc_is_null(row, 11) {
                    String::new()
                } else {
                    odbc_row_string(row, 11)
                };
                columns.push(vec![name, ty, nullable, comment]);
            }

            // 主键：SQLPrimaryKeys（COLUMN_NAME=3 / KEY_SEQ=4 / PK_NAME=5），
            // 列按 KEY_SEQ 排序聚成一行 PK 约束
            let pk_rows = api
                .primary_keys(eng.dbc.0, &catalog, &schem, &tbl)
                .map_err(|e| format!("ODBC 查询主键失败：{e}"))?;
            let mut pk_cols: Vec<(i64, String)> = pk_rows
                .iter()
                .map(|r| {
                    (
                        odbc_row_string(r, 4).parse::<i64>().unwrap_or(0),
                        odbc_row_string(r, 3),
                    )
                })
                .collect();
            pk_cols.sort_by_key(|(seq, _)| *seq);
            let constraints = if pk_cols.is_empty() {
                Vec::new()
            } else {
                let cols = pk_cols
                    .iter()
                    .map(|(_, c)| c.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                let pk_name = pk_rows
                    .first()
                    .map(|r| odbc_row_string(r, 5))
                    .filter(|n| n != "NULL" && !n.is_empty())
                    .unwrap_or_else(|| "PRIMARY".to_string());
                vec![vec![pk_name, "PK".to_string(), cols, String::new()]]
            };

            // 索引：SQLStatistics（NON_UNIQUE=3：0=唯一 / INDEX_NAME=5 /
            // ORDINAL_POSITION=7 / COLUMN_NAME=8）。表基数统计行没有索引名，
            // 跳过；列按位置聚进 BTreeMap 保序
            let stat_rows = api
                .statistics(eng.dbc.0, &catalog, &schem, &tbl)
                .map_err(|e| format!("ODBC 查询索引失败：{e}"))?;
            let mut indexes: BTreeMap<String, (bool, BTreeMap<i64, String>)> = BTreeMap::new();
            for r in &stat_rows {
                let idx_name = odbc_row_string(r, 5);
                if idx_name.is_empty() || idx_name == "NULL" {
                    continue;
                }
                let unique = odbc_row_string(r, 3) == "0";
                let pos = odbc_row_string(r, 7).parse::<i64>().unwrap_or(0);
                let col = odbc_row_string(r, 8);
                if col.is_empty() || col == "NULL" {
                    continue;
                }
                indexes
                    .entry(idx_name)
                    .or_insert((unique, BTreeMap::new()))
                    .1
                    .insert(pos, col);
            }
            let index_rows: Vec<Vec<String>> = indexes
                .into_iter()
                .map(|(name, (unique, cols))| {
                    vec![
                        name,
                        if unique {
                            "UNIQUE".to_string()
                        } else {
                            String::new()
                        },
                        cols.into_values().collect::<Vec<_>>().join(", "),
                    ]
                })
                .collect();

            Ok(TableDetail {
                // 通用 ODBC 不做 DDL 生成（方言不可知）；给统计性提示文本
                ddl: format!(
                    "-- 通用 ODBC 引擎不支持 DDL 生成\n-- {qualified}：{} 列 / {} 索引",
                    columns.len(),
                    index_rows.len()
                ),
                comment: String::new(),
                size: String::new(),
                row_count: String::new(),
                column_headers: vec![
                    "Column".into(),
                    "Type".into(),
                    "Nullable".into(),
                    "Comment".into(),
                ],
                columns,
                index_headers: vec!["Index".into(), "Unique".into(), "Columns".into()],
                indexes: index_rows,
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
                references: Vec::new(),
            })
        })
        .await
}
