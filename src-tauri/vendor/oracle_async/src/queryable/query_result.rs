use std::collections::VecDeque;
use std::sync::Arc;
use log::debug;
use crate::conn::routines::{fetch::FetchRoutine, Routine};
use crate::conn::Conn;
use crate::error::Result;
use super::row::{ColumnMeta, Row};

pub struct QueryResult<'a> {
    pub(crate) conn: &'a mut Conn,
    pub(crate) columns: Arc<Vec<ColumnMeta>>,
    pub(crate) rows_buffer: VecDeque<Row>,
    pub(crate) cursor_id: u16,
    pub(crate) rows_affected: u64,
    pub(crate) has_more_rows: bool,
    pub(crate) prefetch_count: u32,
    /// 待接续的隐式结果集（Msg 27）：主游标取尽后逐个链入（PLSQL 匿名块
    /// 的 dbms_sql.return_result / 多结果集）
    pub(crate) implicit: VecDeque<(Arc<Vec<ColumnMeta>>, u16)>,
}

impl<'a> QueryResult<'a> {
    pub fn new(
        conn: &'a mut Conn,
        columns: Arc<Vec<ColumnMeta>>,
        initial_rows: Vec<Row>,
        cursor_id: u16,
        rows_affected: u64,
        has_more_rows: bool,
    ) -> Self {
        Self {
            conn,
            columns,
            rows_buffer: VecDeque::from(initial_rows),
            cursor_id,
            rows_affected,
            has_more_rows,
            prefetch_count: 100,
            implicit: VecDeque::new(),
        }
    }

    pub fn columns(&self) -> &[ColumnMeta] {
        &self.columns
    }

    pub fn rows_affected(&self) -> u64 {
        self.rows_affected
    }

    pub fn cursor_id(&self) -> u16 {
        self.cursor_id
    }

    /// Abort query execution by sending a TNS Break Marker packet.
    pub async fn break_connection(&mut self) -> Result<()> {
        self.conn.break_connection().await
    }

    /// Fetch next row asynchronously. Automatically executes a fetch routine if the buffer is empty
    /// and the server cursor has more rows; 主游标取尽后自动链入下一个隐式结果集（Msg 27）。
    pub async fn next_row(&mut self) -> Result<Option<Row>> {
        loop {
            if let Some(row) = self.rows_buffer.pop_front() {
                return Ok(Some(row));
            }

            if self.has_more_rows && self.cursor_id != 0 {
                // Fetch next batch from Oracle
                let mut routine =
                    FetchRoutine::new(self.cursor_id, self.columns.clone(), self.prefetch_count);
                let resp = routine.call(self.conn).await?;

                self.has_more_rows = resp.has_more_rows;
                // exec 未内联返回列时，首个 fetch 才带 describe——更新列集供后续分批复用
                if let Some(cols) = resp.columns {
                    self.columns = cols;
                }
                self.rows_buffer = VecDeque::from(resp.rows);

                if let Some(row) = self.rows_buffer.pop_front() {
                    return Ok(Some(row));
                }
            }

            // 主游标结束：链入下一个隐式结果集（列集换新、游标 ID 换新）
            match self.implicit.pop_front() {
                Some((cols, cid)) => {
                    debug!("switching to implicit result set (cursor {cid})");
                    self.columns = cols;
                    self.cursor_id = cid;
                    self.has_more_rows = true;
                    continue;
                }
                None => return Ok(None),
            }
        }
    }

    /// Collect all remaining rows into a vector.
    pub async fn collect(mut self) -> Result<Vec<Row>> {
        let mut rows = Vec::new();
        while let Some(row) = self.next_row().await? {
            rows.push(row);
        }
        Ok(rows)
    }
}
