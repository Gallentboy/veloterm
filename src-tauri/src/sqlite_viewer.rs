//! SQLite 数据库直读解析器：只读模式打开外部 .db/.sqlite/.sqlite3 文件，
//! 提取表清单、列结构、执行只读 SQL 查询并以表格格式返回数据。

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SqliteTableSummary {
    pub name: String,
    pub row_count: i64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SqliteMetadata {
    pub tables: Vec<SqliteTableSummary>,
    pub default_table: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SqliteQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub total_rows: usize,
    pub query: String,
}

/// 读取 SQLite 数据库元数据与表清单
#[tauri::command]
pub fn read_sqlite_metadata(file_path: String) -> Result<SqliteMetadata, String> {
    let conn = Connection::open_with_flags(
        &file_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("打开 SQLite 数据库失败: {e}"))?;

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .map_err(|e| format!("查询表清单失败: {e}"))?;

    let table_names: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| format!("获取表名失败: {e}"))?
        .filter_map(Result::ok)
        .collect();

    let mut tables = Vec::with_capacity(table_names.len());
    for name in &table_names {
        // 查询每个表的行数
        let count_query = format!("SELECT count(*) FROM \"{}\"", name.replace('"', "\"\""));
        let count: i64 = conn
            .query_row(&count_query, [], |r| r.get(0))
            .unwrap_or(0);
        tables.push(SqliteTableSummary {
            name: name.clone(),
            row_count: count,
        });
    }

    let default_table = tables
        .first()
        .map(|t| t.name.clone())
        .unwrap_or_default();

    Ok(SqliteMetadata {
        tables,
        default_table,
    })
}

/// 执行只读 SQL 查询并返回表格结果（安全限制：仅允许 SELECT/PRAGMA/EXPLAIN）
#[tauri::command]
pub fn query_sqlite_data(
    file_path: String,
    query: String,
    max_rows: Option<usize>,
) -> Result<SqliteQueryResult, String> {
    let clean_query = query.trim();
    if clean_query.is_empty() {
        return Err("SQL 查询语句不能为空".to_string());
    }

    // 只读安全校验
    let upper = clean_query.to_uppercase();
    if !upper.starts_with("SELECT")
        && !upper.starts_with("PRAGMA")
        && !upper.starts_with("EXPLAIN")
        && !upper.starts_with("WITH")
    {
        return Err("为保障数据安全，仅允许执行只读查询（SELECT / PRAGMA / EXPLAIN）".to_string());
    }

    let conn = Connection::open_with_flags(
        &file_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("打开 SQLite 数据库失败: {e}"))?;

    let mut stmt = conn
        .prepare(clean_query)
        .map_err(|e| format!("SQL 语法错误: {e}"))?;

    let column_names: Vec<String> = stmt
        .column_names()
        .into_iter()
        .map(String::from)
        .collect();
    let col_count = column_names.len();

    let cap_limit = max_rows.unwrap_or(5000).min(20000);
    let mut rows_data = Vec::new();

    let mut query_rows = stmt
        .query([])
        .map_err(|e| format!("SQL 执行错误: {e}"))?;

    while let Ok(Some(row)) = query_rows.next() {
        if rows_data.len() >= cap_limit {
            break;
        }

        let mut row_strings = Vec::with_capacity(col_count);
        for i in 0..col_count {
            let val_ref = row.get_ref(i).unwrap_or(ValueRef::Null);
            let s = match val_ref {
                ValueRef::Null => "NULL".to_string(),
                ValueRef::Integer(i) => i.to_string(),
                ValueRef::Real(r) => {
                    let s = format!("{r}");
                    if s.ends_with(".0") {
                        s.trim_end_matches(".0").to_string()
                    } else {
                        s
                    }
                }
                ValueRef::Text(bytes) => String::from_utf8_lossy(bytes).into_owned(),
                ValueRef::Blob(b) => format!("<BLOB {} bytes>", b.len()),
            };
            row_strings.push(s);
        }
        rows_data.push(row_strings);
    }

    let total_rows = rows_data.len();

    Ok(SqliteQueryResult {
        columns: column_names,
        rows: rows_data,
        total_rows,
        query: clean_query.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_existent_db() {
        let res = read_sqlite_metadata("/path/to/non_existent.db".to_string());
        assert!(res.is_err());
    }

    #[test]
    fn test_write_query_rejected() {
        let res = query_sqlite_data(
            "/path/to/any.db".to_string(),
            "DROP TABLE users".to_string(),
            None,
        );
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("仅允许执行只读查询"));
    }
}
