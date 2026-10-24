//! Redis 的「表详情」：键的检视块（无 DDL/索引/约束概念）。

use super::super::{
    redis_db_of, redis_inspect_block, redis_key_entries, redis_key_miss_hint, redis_key_type,
    redis_select, RedisConn, TableDetail,
};

pub(crate) async fn detail(
    conn: &mut RedisConn,
    current_db: u32,
    schema: &str,
    table: &str,
) -> Result<TableDetail, String> {
    // Redis 没有 DDL：ddl 字段承载一段人类可读的键检视块（纯文本，
    // 不是可执行语句——前端 DDL Tab 是只读代码块，语义成立）。
    // schema 是逻辑库名（db0 形态）：树里点开的 key 属于该库，
    // 与会话当前库可能不同，故临时切过去再切回。
    // 前置一次 TYPE：不存在的 key 直接报错，避免返回一份全是
    // 占位值的「详情」误导用户（key 过期/被删是常态）。
    // schema 解析失败时回退当前库（前端对老调用方传裸 key 的兼容），
    // 但错误信息里带上「在哪个库找的」，否则跨库场景会被误报成 key 过期
    let target = redis_db_of(schema, current_db)?;
    redis_select(conn, target).await?;
    let inspected = async {
        let kind = redis_key_type(conn, table)
            .await
            .map_err(|e| redis_key_miss_hint(e, target, schema))?;
        let ddl = redis_inspect_block(conn, table, &kind).await;
        let (_, rows) = redis_key_entries(conn, table, &kind).await;
        Ok::<_, String>((ddl, rows))
    }
    .await;
    redis_select(conn, current_db).await.ok();
    let (ddl, rows) = inspected?;
    Ok(TableDetail {
        ddl,
        comment: String::new(),
        size: String::new(),
        row_count: String::new(),
        column_headers: vec!["field".into(), "value".into()],
        columns: rows,
        // 无索引概念：空 headers + 空 rows，前端空表自然不渲染
        index_headers: Vec::new(),
        indexes: Vec::new(),
        constraint_headers: Vec::new(),
        constraints: Vec::new(),
        reference_headers: Vec::new(),
        references: Vec::new(),
    })
}
