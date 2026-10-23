//! 达梦 DM8 引擎专属辅助：schema 清单、标识符校验、版本探测、行/值字符串化。
//!
//! 从 engines.rs 拆出（纯移动，零行为变化）。驱动是 vendor 树内的 dm-pure
//! （纯 Rust 同步实现，经 `with_dameng` 的 spawn_blocking 包裹调用）。
//!
//! 数据字典口径：schema 清单取 `SYSOBJECTS WHERE TYPE$ = 'SCH'`（达梦元数据
//! 基表，权威来源），其余对象走 `ALL_*` 系列 Oracle 风格视图——两者在
//! Oracle 兼容与 MySQL 兼容模式下都存在。

use super::format_float;

/// 达梦 schema（模式）清单：SYSOBJECTS 是达梦的元数据基表，`TYPE$ = 'SCH'`
/// 即模式对象——这是清单的权威来源。
///
/// **不能用 ALL_USERS**：达梦的用户与模式可分离（`CREATE SCHEMA xxx
/// AUTHORIZATION user` 可建与用户名不同的模式，一个用户也可挂多个模式），
/// `ALL_USERS` 只列用户、会漏掉独立命名的业务模式。
const DM_SCHEMAS_SQL: &str = "SELECT NAME FROM SYSOBJECTS WHERE TYPE$ = 'SCH'";

/// 达梦 schema 清单查询（`list_databases` 与 `catalog_root` 共用）。
pub(crate) fn dm_schema_names(client: &mut dm_pure::DmClient) -> Result<Vec<String>, String> {
    dm_query_strings(client, DM_SCHEMAS_SQL, &[])
}

/// 达梦标识符校验（拼进 SET SCHEMA 前）：同 Oracle 的普通标识符字符集
/// [A-Za-z0-9_$#]，非空。防止拼接注入。
pub(crate) fn dm_valid_identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '#'))
}

/// 服务端版本描述。dm-pure 没有 server_version API（只有 server_encoding_name），
/// 故读 V$INSTANCE.BUILD_VERSION；部分部署该视图不可见时降级 V$VERSION
///（多行 banner），都拿不到就回 "DM8"（版本号只是展示，不影响功能）。
pub(crate) fn dm_server_version(client: &mut dm_pure::DmClient) -> String {
    if let Some(v) = dm_query_first_string(client, "SELECT BUILD_VERSION FROM V$INSTANCE", &[])
        .filter(|s| !s.trim().is_empty())
    {
        return v.trim().to_string();
    }
    if let Ok(rows) = dm_query_rows(client, "SELECT * FROM V$VERSION", &[]) {
        // V$VERSION 是「BANNER」单列多行：取首行非空即达梦版本串
        if let Some(line) = rows
            .iter()
            .map(|row| dm_row_string(row, 0))
            .find(|s| !s.trim().is_empty())
        {
            return line.trim().to_string();
        }
    }
    "DM8".to_string()
}

/// 达梦行类型：每单元格一个解码后的 Value。
pub(crate) type DmRow = Vec<dm_pure::Value>;

/// 执行查询并把所有行返回（非 Query 结果返回空）。dm-pure 的 execute 已把
/// 整结果集解在内存，这里只是所有权转移。
pub(crate) fn dm_query_rows(
    client: &mut dm_pure::DmClient,
    sql: &str,
    params: &[dm_pure::Param],
) -> Result<Vec<DmRow>, dm_pure::DmError> {
    match client.execute(sql, params)? {
        dm_pure::ExecuteResult::Query { rows, .. } => Ok(rows),
        _ => Ok(Vec::new()),
    }
}

/// 单值查询的便捷形态：取首行首列并字符串化（无结果返回 None）。
/// 元数据查询（版本、DDL）大多走这条路径。
pub(crate) fn dm_query_first_string(
    client: &mut dm_pure::DmClient,
    sql: &str,
    params: &[dm_pure::Param],
) -> Option<String> {
    let rows = dm_query_rows(client, sql, params).ok()?;
    let first = rows.first()?;
    Some(dm_text_at(first, 0))
}

/// 文本列的取值：CLOB 正常解成 Str，但 DBMS_METADATA 等系统函数在部分
/// 服务端版本/路径上会以 BLOB 字节返回——此时按 UTF-8 容错解码（不能走
/// dm_value_to_string 的 `<BLOB N bytes>` 占位，否则 DDL 面板只剩占位符）。
/// NULL 返回空串（调用方按「无内容」处理：DDL 降级、版本回退）。
pub(crate) fn dm_text_at(row: &[dm_pure::Value], i: usize) -> String {
    match row.get(i) {
        Some(dm_pure::Value::Bytes(b)) | Some(dm_pure::Value::Raw(b)) => {
            String::from_utf8_lossy(b).into_owned()
        }
        Some(dm_pure::Value::Null) | None => String::new(),
        Some(other) => dm_value_to_string(other),
    }
}

/// 取首列字符串清单（schema/对象名列表查询用）。
pub(crate) fn dm_query_strings(
    client: &mut dm_pure::DmClient,
    sql: &str,
    params: &[dm_pure::Param],
) -> Result<Vec<String>, String> {
    let rows = dm_query_rows(client, sql, params).map_err(|e| format!("达梦查询失败：{e}"))?;
    Ok(rows.iter().map(|row| dm_row_string(row, 0)).collect())
}

/// 行按列取字符串（越界列回 NULL，与 oracle_row_string 同语义）。
pub(crate) fn dm_row_string(row: &[dm_pure::Value], i: usize) -> String {
    row.get(i)
        .map(dm_value_to_string)
        .unwrap_or_else(|| "NULL".to_string())
}

/// 行按列取 i64（计数/元数据数值列；取不到回 0）。
pub(crate) fn dm_row_i64(row: &[dm_pure::Value], i: usize) -> i64 {
    row.get(i).map(dm_value_to_i64).unwrap_or(0)
}

/// 数值变体统一取 i64：COUNT(*) 的返回类型随服务器/驱动路径漂移
///（BigInt / Int / Decimal 字符串均见过），统一归一。非数值回 0。
pub(crate) fn dm_value_to_i64(value: &dm_pure::Value) -> i64 {
    use dm_pure::Value as V;
    match value {
        V::TinyInt(v) => *v as i64,
        V::SmallInt(v) => *v as i64,
        V::Int(v) => *v as i64,
        V::BigInt(v) => *v,
        V::Bool(v) => *v as i64,
        // Decimal 保持字符串（精度）——计数场景 parse 即可：整数形态直取，
        // 带小数点的（如 "7.0"）走 f64 再截断，失败回 0
        V::Decimal(s) => s
            .trim()
            .parse::<i64>()
            .unwrap_or_else(|_| s.trim().parse::<f64>().map(|f| f as i64).unwrap_or(0)),
        V::Real(v) => *v as i64,
        V::Double(v) => *v as i64,
        _ => 0,
    }
}

/// 达梦单元格字符串化（与 sqlite_viewer / MySQL 臂展示语义对齐）：
/// NULL 字面量、二进制给长度占位（不把 BLOB 原字节倒进表格）、整数去 .0。
pub(crate) fn dm_value_to_string(value: &dm_pure::Value) -> String {
    use dm_pure::Value as V;
    match value {
        V::Null => "NULL".to_string(),
        V::Bool(b) => b.to_string(),
        V::TinyInt(v) => v.to_string(),
        V::SmallInt(v) => v.to_string(),
        V::Int(v) => v.to_string(),
        V::BigInt(v) => v.to_string(),
        V::Real(v) => format_float(*v as f64),
        V::Double(v) => format_float(*v),
        // DECIMAL 原样输出（驱动已保证是精确的十进制字符串，不经 f64）
        V::Decimal(s) => s.clone(),
        V::Str(s) => s.clone(),
        V::Bytes(b) | V::Raw(b) => format!("<BLOB {} bytes>", b.len()),
        // 日期时间三型都实现了 Display（yyyy-mm-dd / hh:mm:ss.ffffff 形态）；
        // Interval 是驱动已格式化好的字符串
        V::Date(v) => v.to_string(),
        V::Time(v) => v.to_string(),
        V::DateTime(v) => v.to_string(),
        V::Interval(s) => s.clone(),
    }
}
