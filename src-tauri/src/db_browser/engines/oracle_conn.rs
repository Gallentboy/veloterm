//! Oracle 会话的驱动统一抽象（双驱动形态）。
//!
//! - [`OracleConn::Legacy`]：自研 vendor/oracle_async（纯 Rust 原生异步，
//!   TNS/TTC 线路协议）。10g/11g 的最终会话，同时充当 12c+ 路由前的
//!   版本探测器，以及官方驱动连接失败时的降级兜底。
//! - [`OracleConn::Official`]（后续阶段接线）：Oracle 官方 oracledb crate
//!   （12c+，同步 API，spawn_blocking 桥接，见 oracle_official.rs）。
//!
//! 目录/详情/定义层的小结果集统一经 [`OracleConn::query_rows`] 全量收集：
//! 行值在驱动层渲染成展示字符串（NULL → "NULL"，对齐 Legacy 的
//! OracleValue Display 语义），消费侧不再感知驱动类型。查询工作台的
//! 行流（分批回调 + 取消）不走这里——exec_streaming 按变体分派
//! （Legacy 流式游标 + TNS Break Marker；Official 批量桥接）。
//!
//! 错误文案：各方法的 Err 已经过 oracle_err（或官方桥接的同义映射）
//! 转换，调用方只补上下文前缀。

use super::oracle_err;
use super::oracle_official::OfficialOracle;

/// Oracle 会话：自研驱动与官方驱动的统一分发。Legacy 装箱：自研 Conn
/// 体积 ~456B，与官方臂（Arc + 版本缓存）悬殊；会话枚举只持一份，
/// 多一次间接寻址无成本。
pub enum OracleConn {
    /// 自研驱动（vendor/oracle_async）
    Legacy(Box<oracle_async::Conn>),
    /// Oracle 官方 oracledb（12c+，spawn_blocking 桥接，见 oracle_official.rs）
    Official(OfficialOracle),
}

/// 已字符串化的行集，消费侧 [`Self::next_row`] 游标推进。
/// 目录层只做拼接展示，不做类型还原；数量级为数据字典查询结果（小结果集）。
pub(crate) struct OracleRowSet {
    rows: Vec<Vec<String>>,
    cursor: usize,
}

impl OracleRowSet {
    /// 由已字符串化的行集构造（官方驱动桥接的收集路径用）。
    pub(crate) fn from_rows(rows: Vec<Vec<String>>) -> Self {
        Self { rows, cursor: 0 }
    }

    /// 下一行（消费完返回 None）。
    pub(crate) fn next_row(&mut self) -> Option<&[String]> {
        let row = self.rows.get(self.cursor)?;
        self.cursor += 1;
        Some(row.as_slice())
    }

    /// 首行首列（标量查询取值的常见形态），无行返回 None。
    pub(crate) fn scalar(&mut self) -> Option<&str> {
        self.next_row().and_then(|row| row.first().map(String::as_str))
    }
}

impl OracleConn {
    /// 查询并全量收集（列名 + 字符串化行）。目录/详情/定义层小结果集专用；
    /// 行流场景（结果面板）见 engines 模块的 exec_streaming。
    pub(crate) async fn query_rows(&mut self, sql: &str) -> Result<OracleRowSet, String> {
        match self {
            Self::Legacy(conn) => {
                use oracle_async::Queryable as _;
                let mut rs = conn.query(sql).await.map_err(oracle_err)?;
                let mut rows = Vec::new();
                while let Some(row) = rs.next_row().await.map_err(oracle_err)? {
                    rows.push(row.values().iter().map(|v| v.to_string()).collect());
                }
                // 关服务端游标（防 ORA-01000 泄漏；幂等，含隐式游标）
                let _ = rs.close().await;
                Ok(OracleRowSet { rows, cursor: 0 })
            }
            Self::Official(off) => Ok(OracleRowSet::from_rows(
                off.collect_rows(sql).await?,
            )),
        }
    }

    /// 执行无结果集语句（DML/DDL/ALTER SESSION），返回受影响行数。
    pub(crate) async fn exec(&mut self, sql: &str) -> Result<u64, String> {
        match self {
            Self::Legacy(conn) => {
                use oracle_async::Queryable as _;
                conn.exec(sql).await.map_err(oracle_err)
            }
            Self::Official(off) => off.exec_sql(sql).await,
        }
    }

    /// 提交（Legacy：协议级 simpleObject 0x0E，不占游标资源）。
    pub(crate) async fn commit(&mut self) -> Result<(), String> {
        match self {
            Self::Legacy(conn) => conn.commit().await.map_err(oracle_err),
            Self::Official(off) => off.commit().await,
        }
    }

    /// 回滚（Legacy：协议级 simpleObject 0x0F）。
    pub(crate) async fn rollback(&mut self) -> Result<(), String> {
        match self {
            Self::Legacy(conn) => conn.rollback().await.map_err(oracle_err),
            Self::Official(off) => off.rollback().await,
        }
    }

    /// 服务端主版本（目录 SQL 版本自适应的判据，如 all_users 的
    /// oracle_maintained 分支、LISTAGG 聚列分支）；未知为 0 → 调用方走保守分支。
    pub(crate) fn version_major(&self) -> u32 {
        match self {
            Self::Legacy(conn) => conn.db_version_major(),
            Self::Official(off) => off.version_major(),
        }
    }

    /// 版本展示串（Legacy：0x3B 完整 banner 优先——与原生 OCI / go-ora
    /// 严格一致，兜底数字版本号，再兜底 "Oracle"）。
    pub(crate) fn version_display(&self) -> String {
        match self {
            Self::Legacy(conn) => {
                let banner = conn.db_version_banner().trim();
                if !banner.is_empty() {
                    banner.to_string()
                } else {
                    let text = conn.db_version().trim();
                    if text.is_empty() {
                        "Oracle".to_string()
                    } else {
                        text.to_string()
                    }
                }
            }
            Self::Official(off) => off.version_text().to_string(),
        }
    }
}
