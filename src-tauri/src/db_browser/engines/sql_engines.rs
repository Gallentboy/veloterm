//! SQL 系三引擎（SQLite / MySQL / PostgreSQL）的单元格取值与标识符转义辅助。
//!
//! 从 engines.rs 拆出（纯移动，零行为变化）。三者放在同一模块而非各拆一个：
//! 每块只有几十行，且取值语义同源（NULL 字面量、BLOB 长度占位、整数去 .0），
//! 放一起便于对照维护。

use super::format_float;

// ── SQLite ───────────────────────────────────────────────────────────────

/// SQLite 单元格取值：类型驱动（NULL/BLOB 占位、整数去 .0，与 sqlite_viewer
/// 的展示语义对齐）。取值失败按 NULL 兜底（不中断整行）。
#[inline]
pub(crate) fn sqlite_cell(row: &rusqlite::Row<'_>, i: usize) -> String {
    match row.get_ref(i) {
        Ok(rusqlite::types::ValueRef::Null) => "NULL".to_string(),
        Ok(rusqlite::types::ValueRef::Integer(v)) => v.to_string(),
        Ok(rusqlite::types::ValueRef::Real(v)) => format_float(v),
        Ok(rusqlite::types::ValueRef::Text(bytes)) => String::from_utf8_lossy(bytes).into_owned(),
        Ok(rusqlite::types::ValueRef::Blob(b)) => format!("<BLOB {} bytes>", b.len()),
        Err(_) => "NULL".to_string(),
    }
}

/// rusqlite 错误 → 展示文案（统一前缀，与另三引擎的「引擎名：」口径一致）
#[inline(always)]
pub(crate) fn sqlite_err(e: rusqlite::Error) -> String {
    format!("SQLite：{e}")
}

/// 是否被 progress handler 中断（SQLITE_INTERRUPT）。取消映射靠它而不是
/// 错误文案匹配——口令错等其它失败仍按错误上报。
#[inline(always)]
pub(crate) fn sqlite_is_interrupt(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(rusqlite::ffi::ErrorCode::OperationInterrupted)
    )
}

/// 标识符转义（schema 名拼进 SQL）：仅「首字符字母/下划线 + 其余 [A-Za-z0-9_]」
/// 可裸用，其余一律双引号包裹并把内部 `"` 双写。禁止裸拼用户输入。
#[inline]
pub(crate) fn quote_ident(name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("SQLite 无效的库名（空）".to_string());
    }
    if name.contains('\0') {
        return Err("SQLite 无效的库名（含空字符）".to_string());
    }
    let mut chars = name.chars();
    let head_ok = chars
        .next()
        .map(|c| c.is_ascii_alphabetic() || c == '_')
        .unwrap_or(false);
    if head_ok && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(name.to_string())
    } else {
        Ok(format!("\"{}\"", name.replace('"', "\"\"")))
    }
}

/// 字符串字面量转义（PRAGMA 的表/索引名参数是字面量不是标识符）：单引号
/// 包裹，内部 `'` 双写。
#[inline(always)]
pub(crate) fn quote_string_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// 已附加的库名（PRAGMA database_list 的 name 列：main/temp 及 ATTACH 的库）。
/// 列序：seq(0) name(1) file(2)。
pub(crate) fn sqlite_database_names(conn: &rusqlite::Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("PRAGMA database_list")
        .map_err(|e| format!("SQLite 查询库清单失败：{e}"))?;
    let mut rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| format!("SQLite 查询库清单失败：{e}"))?;
    let mut names: Vec<String> = Vec::new();
    for row in rows.by_ref() {
        names.push(row.map_err(|e| format!("SQLite 读取库清单失败：{e}"))?);
    }
    Ok(names)
}

// ── MySQL ────────────────────────────────────────────────────────────────

#[inline]
pub(crate) fn mysql_value_to_string(value: &mysql_async::Value) -> String {
    use mysql_async::Value as V;
    match value {
        V::NULL => "NULL".to_string(),
        V::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
        V::Int(i) => i.to_string(),
        V::UInt(u) => u.to_string(),
        V::Float(f) => format_float(*f as f64),
        V::Double(d) => format_float(*d),
        V::Date(y, mo, d, h, mi, s, us) => {
            let base = format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}");
            if *us == 0 {
                base
            } else {
                format!("{base}.{us:06}")
            }
        }
        V::Time(neg, d, h, m, s, us) => {
            let sign = if *neg { "-" } else { "" };
            let hours = *d as i64 * 24 + *h as i64;
            if *us == 0 {
                format!("{sign}{hours:02}:{m:02}:{s:02}")
            } else {
                format!("{sign}{hours:02}:{m:02}:{s:02}.{us:06}")
            }
        }
    }
}

// ── PostgreSQL ───────────────────────────────────────────────────────────

/// PG 单元格取值：类型驱动的多级降级链（Option 统一处理 NULL）。
#[inline]
pub(crate) fn pg_cell(row: &tokio_postgres::Row, i: usize) -> String {
    if let Ok(v) = row.try_get::<_, Option<String>>(i) {
        return v.unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(v) = row.try_get::<_, Option<i64>>(i) {
        return v.map(|x| x.to_string()).unwrap_or_else(|| "NULL".into());
    }
    if let Ok(v) = row.try_get::<_, Option<f64>>(i) {
        return v.map(format_float).unwrap_or_else(|| "NULL".into());
    }
    if let Ok(v) = row.try_get::<_, Option<bool>>(i) {
        return v.map(|x| x.to_string()).unwrap_or_else(|| "NULL".into());
    }
    if let Ok(v) = row.try_get::<_, Option<rust_decimal::Decimal>>(i) {
        return v.map(|x| x.to_string()).unwrap_or_else(|| "NULL".into());
    }
    if let Ok(v) = row.try_get::<_, Option<pg_uuid::Uuid>>(i) {
        return v.map(|x| x.to_string()).unwrap_or_else(|| "NULL".into());
    }
    if let Ok(v) = row.try_get::<_, Option<serde_json::Value>>(i) {
        return v.map(|x| x.to_string()).unwrap_or_else(|| "NULL".into());
    }
    if let Ok(v) = row.try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(i) {
        return v
            .map(|x| x.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_else(|| "NULL".into());
    }
    if let Ok(v) = row.try_get::<_, Option<std::time::SystemTime>>(i) {
        return v
            .map(|x| {
                x.duration_since(std::time::UNIX_EPOCH)
                    .map(|d| format!("<{}s>", d.as_secs()))
                    .unwrap_or_else(|_| "NULL".into())
            })
            .unwrap_or_else(|| "NULL".into());
    }
    format!(
        "<{}>",
        row.columns()
            .get(i)
            .map(|c| c.type_().to_string())
            .unwrap_or_else(|| "?".to_string())
    )
}
