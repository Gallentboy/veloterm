// Copyright (c) 2017 Anatoly Ikorsky
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

use futures_util::FutureExt;
use mysql_common::{
    io::ParseBuf,
    named_params::ParsedNamedParams,
    packets::{ComStmtClose, StmtPacket},
};

use std::{borrow::Cow, sync::Arc};

use crate::{
    conn::routines::{ExecRoutine, PrepareRoutine},
    consts::CapabilityFlags,
    error::*,
    Column, Params,
};

use super::AsQuery;

/// Result of a `StatementLike::to_statement` call.
pub enum ToStatementResult<'a> {
    /// Statement is immediately available.
    Immediate(Statement),
    /// We need some time to get a statement and the operation itself may fail.
    Mediate(crate::BoxFuture<'a, Statement>),
}

pub trait StatementLike: Send + Sync {
    /// Returns a statement.
    fn to_statement<'a>(self, conn: &'a mut crate::Conn) -> ToStatementResult<'a>
    where
        Self: 'a;
}

fn to_statement_move<'a, T: AsQuery + 'a>(
    stmt: T,
    conn: &'a mut crate::Conn,
) -> ToStatementResult<'a> {
    let fut = async move {
        let query = stmt.as_query();
        let parsed = ParsedNamedParams::parse(query.as_ref())?;
        let inner_stmt = match conn.get_cached_stmt(parsed.query()) {
            Some(inner_stmt) => inner_stmt,
            None => {
                conn.prepare_statement(Cow::Borrowed(parsed.query()))
                    .await?
            }
        };
        Ok(Statement::new(
            inner_stmt,
            parsed
                .params()
                .iter()
                .map(|x| x.as_ref().to_vec())
                .collect::<Vec<_>>(),
        ))
    }
    .boxed();
    ToStatementResult::Mediate(fut)
}

impl StatementLike for Cow<'_, str> {
    fn to_statement<'a>(self, conn: &'a mut crate::Conn) -> ToStatementResult<'a>
    where
        Self: 'a,
    {
        to_statement_move(self, conn)
    }
}

impl StatementLike for &'_ str {
    fn to_statement<'a>(self, conn: &'a mut crate::Conn) -> ToStatementResult<'a>
    where
        Self: 'a,
    {
        to_statement_move(self, conn)
    }
}

impl StatementLike for String {
    fn to_statement<'a>(self, conn: &'a mut crate::Conn) -> ToStatementResult<'a>
    where
        Self: 'a,
    {
        to_statement_move(self, conn)
    }
}

impl StatementLike for Box<str> {
    fn to_statement<'a>(self, conn: &'a mut crate::Conn) -> ToStatementResult<'a>
    where
        Self: 'a,
    {
        to_statement_move(self, conn)
    }
}

impl StatementLike for Arc<str> {
    fn to_statement<'a>(self, conn: &'a mut crate::Conn) -> ToStatementResult<'a>
    where
        Self: 'a,
    {
        to_statement_move(self, conn)
    }
}

impl StatementLike for Cow<'_, [u8]> {
    fn to_statement<'a>(self, conn: &'a mut crate::Conn) -> ToStatementResult<'a>
    where
        Self: 'a,
    {
        to_statement_move(self, conn)
    }
}

impl StatementLike for &'_ [u8] {
    fn to_statement<'a>(self, conn: &'a mut crate::Conn) -> ToStatementResult<'a>
    where
        Self: 'a,
    {
        to_statement_move(self, conn)
    }
}

impl StatementLike for Vec<u8> {
    fn to_statement<'a>(self, conn: &'a mut crate::Conn) -> ToStatementResult<'a>
    where
        Self: 'a,
    {
        to_statement_move(self, conn)
    }
}

impl StatementLike for Box<[u8]> {
    fn to_statement<'a>(self, conn: &'a mut crate::Conn) -> ToStatementResult<'a>
    where
        Self: 'a,
    {
        to_statement_move(self, conn)
    }
}

impl StatementLike for Arc<[u8]> {
    fn to_statement<'a>(self, conn: &'a mut crate::Conn) -> ToStatementResult<'a>
    where
        Self: 'a,
    {
        to_statement_move(self, conn)
    }
}

impl StatementLike for Statement {
    fn to_statement<'a>(self, _conn: &'a mut crate::Conn) -> ToStatementResult<'static>
    where
        Self: 'a,
    {
        ToStatementResult::Immediate(self.clone())
    }
}

impl<T: StatementLike + Clone> StatementLike for &'_ T {
    fn to_statement<'a>(self, conn: &'a mut crate::Conn) -> ToStatementResult<'a>
    where
        Self: 'a,
    {
        self.clone().to_statement(conn)
    }
}

/// Statement data.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StmtInner {
    pub(crate) raw_query: Arc<[u8]>,
    columns: Option<Box<[Column]>>,
    params: Option<Box<[Column]>>,
    stmt_packet: StmtPacket,
    connection_id: u32,
}

impl StmtInner {
    pub(crate) fn from_payload(
        pld: &[u8],
        connection_id: u32,
        raw_query: Arc<[u8]>,
    ) -> std::io::Result<Self> {
        let stmt_packet = ParseBuf(pld).parse(())?;

        Ok(Self {
            raw_query,
            columns: None,
            params: None,
            stmt_packet,
            connection_id,
        })
    }

    pub(crate) fn with_params(mut self, params: Vec<Column>) -> Self {
        self.params = if params.is_empty() {
            None
        } else {
            Some(params.into_boxed_slice())
        };
        self
    }

    pub(crate) fn with_columns(mut self, columns: Vec<Column>) -> Self {
        self.columns = if columns.is_empty() {
            None
        } else {
            Some(columns.into_boxed_slice())
        };
        self
    }

    pub(crate) fn columns(&self) -> &[Column] {
        self.columns.as_ref().map(AsRef::as_ref).unwrap_or(&[])
    }

    pub(crate) fn params(&self) -> &[Column] {
        self.params.as_ref().map(AsRef::as_ref).unwrap_or(&[])
    }

    pub(crate) fn id(&self) -> u32 {
        self.stmt_packet.statement_id()
    }

    pub(crate) const fn connection_id(&self) -> u32 {
        self.connection_id
    }

    pub(crate) fn num_params(&self) -> u16 {
        self.stmt_packet.num_params()
    }

    pub(crate) fn num_columns(&self) -> u16 {
        self.stmt_packet.num_columns()
    }
}

/// Prepared statement.
///
/// Statement is only valid for connection with id `Statement::connection_id()`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Statement {
    pub(crate) inner: Arc<StmtInner>,
    /// An empty vector in case of no named params.
    pub(crate) named_params: Vec<Vec<u8>>,
}

impl Statement {
    pub(crate) fn new(inner: Arc<StmtInner>, named_params: Vec<Vec<u8>>) -> Self {
        Self {
            inner,
            named_params,
        }
    }

    /// Returned columns.
    pub fn columns(&self) -> &[Column] {
        self.inner.columns()
    }

    /// Required parameters.
    pub fn params(&self) -> &[Column] {
        self.inner.params()
    }

    /// MySql statement identifier.
    pub fn id(&self) -> u32 {
        self.inner.id()
    }

    /// MySql connection identifier.
    pub fn connection_id(&self) -> u32 {
        self.inner.connection_id()
    }

    /// Number of parameters.
    pub fn num_params(&self) -> u16 {
        self.inner.num_params()
    }

    /// Number of columns.
    pub fn num_columns(&self) -> u16 {
        self.inner.num_columns()
    }
}

impl crate::Conn {
    /// Low-level helpers, that reads the given number of column packets terminated by EOF packet.
    ///
    /// Requires `num > 0`.
    pub(crate) async fn read_column_defs<U>(&mut self, num: U) -> Result<Vec<Column>>
    where
        U: Into<usize>,
    {
        let num = num.into();
        debug_assert!(num > 0);
        let packets = self.read_packets(num).await?;
        // N 列标记：重映射时从原始类型码 207/208 记录（真机实测 OB 把
        // N 列字符集编号虚报为 utf8mb4、与 VARCHAR2 同号，字符集不可依据；
        // JDBC 按 typeOracleMap 的类型码分派 nCharacterEncoding=UTF-16BE
        // 解码，本标记同语义，next_row 消费）
        let mut nchar_flags = Vec::with_capacity(num);
        let defs = packets
            .into_iter()
            .map(|x| {
                let parsed = ParseBuf(&x).parse::<Column>(());
                match parsed {
                    Ok(col) => {
                        nchar_flags.push(false);
                        Ok(col)
                    }
                    Err(_) => {
                        // OB Oracle 扩展列类型码降级：mysql_common 的 ColumnType
                        // 不认识 OB 扩展码（160-163/200-211，Java ColumnType.java
                        // 的 typeOracleMap），解析失败时把类型码字节替换为
                        // VAR_STRING(253) 重试——文本协议下这些类型都以字符串
                        // 返回，降级安全；N 类型（207/208）同时记标记供
                        // next_row 做 UTF-16BE 解码。PooledBuf 不可变，
                        // 复制到栈缓冲再改。
                        let mut buf = x.as_ref().to_vec();
                        let is_nchar = remap_ob_column_type(&mut buf);
                        nchar_flags.push(is_nchar);
                        ParseBuf(&buf).parse::<Column>(()).map_err(Error::from)
                    }
                }
            })
            .collect::<std::result::Result<Vec<Column>, _>>()
            .map_err(Error::from)?;
        self.set_ob_nchar_column_flags(nchar_flags);

        if !self
            .capabilities()
            .contains(CapabilityFlags::CLIENT_DEPRECATE_EOF)
        {
            self.read_packet().await?;
        }

        Ok(defs)
    }

    /// Helper, that retrieves `Statement` from `StatementLike`.
    pub(crate) async fn get_statement<U>(&mut self, stmt_like: U) -> Result<Statement>
    where
        U: StatementLike,
    {
        match stmt_like.to_statement(self) {
            ToStatementResult::Immediate(statement) => Ok(statement),
            ToStatementResult::Mediate(statement) => statement.await,
        }
    }

    /// Low-level helper, that prepares the given statement.
    ///
    /// `raw_query` is a query with `?` placeholders (if any).
    async fn prepare_statement(&mut self, raw_query: Cow<'_, [u8]>) -> Result<Arc<StmtInner>> {
        let inner_stmt = self.routine(PrepareRoutine::new(raw_query)).await?;

        if let Some(old_stmt) = self.cache_stmt(&inner_stmt) {
            self.close_statement(old_stmt.id()).await?;
        }

        Ok(inner_stmt)
    }

    /// Helper, that executes the given statement with the given params.
    pub(crate) async fn execute_statement<P>(
        &mut self,
        statement: &Statement,
        params: P,
    ) -> Result<()>
    where
        P: Into<Params>,
    {
        self.routine(ExecRoutine::new(statement, params.into()))
            .await?;
        Ok(())
    }

    /// Helper, that closes statement with the given id.
    pub(crate) async fn close_statement(&mut self, id: u32) -> Result<()> {
        self.stmt_cache_mut().remove(id);
        self.write_command(&ComStmtClose::new(id)).await
    }
}

/// 把列定义包里的 OB Oracle 扩展列类型码替换为 VAR_STRING(253)。
///
/// 列定义包结构：6 个 lenenc 字符串（catalog/schema/table/org_table/name/
/// org_name）后跟 13 字节固定段（2B filler_len + 2B charset + 4B col_len +
/// **1B column_type** + 2B flags + 1B decimals + 2B filler）。本函数走完
/// lenenc 字符串后定位 column_type 字节，若值在 OB_ORACLE_EXT_TYPES 里则
/// 替换为 VAR_STRING(253)；返回是否为 N 类型（207/208，调用方据此记
/// UTF-16BE 解码标记）。非 OB 场景下该字节不会命中扩展码，无副作用。
#[inline]
fn remap_ob_column_type(packet: &mut [u8]) -> bool {
    let mut pos = 0usize;
    for _ in 0..6 {
        let len = match packet.get(pos) {
            Some(&l) if l < 0xFB => {
                pos += 1;
                l as usize
            }
            Some(0xFC) => match (packet.get(pos + 1), packet.get(pos + 2)) {
                (Some(&a), Some(&b)) => {
                    pos += 3;
                    (u16::from(a) | (u16::from(b) << 8)) as usize
                }
                _ => return false,
            },
            Some(0xFD) => match (
                packet.get(pos + 1),
                packet.get(pos + 2),
                packet.get(pos + 3),
            ) {
                (Some(&a), Some(&b), Some(&c)) => {
                    pos += 4;
                    (u32::from(a) | (u32::from(b) << 8) | (u32::from(c) << 16)) as usize
                }
                _ => return false,
            },
            _ => return false,
        };
        pos += len;
        if pos >= packet.len() {
            return false;
        }
    }
    // 固定段 13 字节（mysql_common Column::deserialize 的 parse(13) 同口径）：
    // fixed_length_fields_len(1B lenenc，实际恒 0x0C 单字节) + charset(2) +
    // column_length(4) = 7 后即 column_type。此前误把 fixed_len 当 2 字节
    //（真机实测 Unknown column type 202 的根因：偏 1 字节打到 flags 高位，
    // 类型码原封未动）
    let type_off = pos + 7;
    if let Some(&t) = packet.get(type_off) {
        if crate::ob20::OB_ORACLE_EXT_TYPES.contains(&t) {
            packet[type_off] = 253; // VAR_STRING
            return matches!(t, 207 | 208); // NVARCHAR2 / NCHAR
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::remap_ob_column_type;

    /// 构造一个列定义包（6 个 lenenc 字符串 + 13 字节固定段），类型码可指定。
    /// 结构对齐 mysql_common Column::deserialize（catalog + 5 个 meta 字符串 +
    /// parse(13) 固定段）。
    fn column_def(column_type: u8) -> Vec<u8> {
        let mut p = Vec::new();
        for s in ["def", "S", "T", "T", "N", "N"] {
            p.push(s.len() as u8);
            p.extend_from_slice(s.as_bytes());
        }
        p.push(0x0c); // fixed_length_fields_len（lenenc，实际恒单字节 12）
        p.extend_from_slice(&[0x00, 0x21]); // charset
        p.extend_from_slice(&[0x00, 0x00, 0x00, 0x0a]); // column_length
        p.push(column_type);
        p.extend_from_slice(&[0x00, 0x00]); // flags
        p.push(0x00); // decimals
        p.extend_from_slice(&[0x00, 0x00]); // filler
        p
    }

    /// 真机实锤的失败形态：TIMESTAMP 列的类型码 202（OB_TIMESTAMP_NANO）。
    /// 此前偏移算错（fixed_len 误当 2 字节），替换打到 flags 高位、类型码
    /// 原封不动——修复后必须命中类型字节；N 类型（207/208）额外返回 true
    #[test]
    fn remap_hits_type_byte_for_ob_ext_types() {
        for code in crate::ob20::OB_ORACLE_EXT_TYPES {
            let mut p = column_def(*code);
            let is_nchar = remap_ob_column_type(&mut p);
            assert_eq!(p[14 + 7], 253, "类型码 {code} 应被降级为 VAR_STRING");
            assert_eq!(
                is_nchar,
                matches!(code, 207 | 208),
                "类型码 {code} 的 N 标记不符"
            );
        }
    }

    /// 标准类型码不受影响（非 OB 扩展码不改写、返回 false）
    #[test]
    fn remap_leaves_standard_types_alone() {
        for code in [0u8, 3, 7, 10, 246, 253, 254] {
            let mut p = column_def(code);
            assert!(!remap_ob_column_type(&mut p), "标准类型码 {code} 不应标记 N");
            assert_eq!(p[14 + 7], code, "标准类型码 {code} 不应被改写");
        }
    }
}
