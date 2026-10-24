//! table_detail 的引擎分发。各引擎实现为自由函数，由 `engines/mod.rs` 的
//! `DbEngine::table_detail` 分派调用。
//!
//! 各引擎文件签名统一：
//! `pub(super) async fn detail(conn, schema, table, kind) -> Result<TableDetail, String>`
//!
//! 拆自 engines/mod.rs 的 `table_detail`（1520 行的七臂 match），行为零变更。

pub(super) mod dameng;
pub(super) mod mysql;
pub(super) mod oracle;
pub(super) mod postgres;
pub(super) mod redis;
pub(super) mod sqlite;
