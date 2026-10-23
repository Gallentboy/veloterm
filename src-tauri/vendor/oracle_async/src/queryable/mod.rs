pub mod values;
pub mod row;
pub mod query_result;

pub use values::{FromValue, OracleValue};
pub use row::{ColumnMeta, Row};
pub use query_result::QueryResult;

use async_trait::async_trait;
use crate::conn::routines::{exec::ExecRoutine, Routine};
use crate::conn::Conn;
use crate::error::Result;

#[async_trait]
pub trait Queryable {
    async fn query<'a>(&'a mut self, query: &str) -> Result<QueryResult<'a>>;

    async fn exec(&mut self, query: &str) -> Result<u64>;

    async fn query_first<'a>(&'a mut self, query: &str) -> Result<Option<Row>> {
        let mut res = self.query(query).await?;
        res.next_row().await
    }
}

#[async_trait]
impl Queryable for Conn {
    async fn query<'a>(&'a mut self, query: &str) -> Result<QueryResult<'a>> {
        let mut routine = ExecRoutine::new(query, true, 100);
        let resp = routine.call(self).await?;
        let mut qr = QueryResult::new(
            self,
            resp.columns,
            resp.rows,
            resp.cursor_id,
            resp.rows_affected,
            resp.has_more_rows,
        );
        // 隐式结果集（Msg 27）排队待链：主游标取尽后逐个接续
        qr.implicit = resp
            .implicit
            .into_iter()
            .map(|ic| (ic.columns, ic.cursor_id))
            .collect();
        Ok(qr)
    }

    async fn exec(&mut self, query: &str) -> Result<u64> {
        let mut routine = ExecRoutine::new(query, false, 0);
        let resp = routine.call(self).await?;
        // 非 query 路径的隐式游标无人消费：立即关闭（防下条语句 ORA-01002）
        for ic in &resp.implicit {
            let _ = self.close_cursor(ic.cursor_id).await;
        }
        Ok(resp.rows_affected)
    }
}
