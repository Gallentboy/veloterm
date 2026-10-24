//! 引擎层：MySQL（mysql_async）/ PostgreSQL（tokio-postgres）/ Oracle
//!（vendor oracle_async，纯 Rust 原生异步驱动，TNS/TTC 线路协议）/
//! Redis（redis-rs 多路复用异步连接）/
//! OceanBase（双模式：MySQL 模式租户说标准 MySQL 线缆协议、复用
//! mysql_async 臂仅微调；Oracle 模式租户同分叉（vendor/mysql-async-ob）
//! 的 ob_protocol20 路径——纯 Rust，SM3 认证、OB 方言探测与 N 列
//! 解码内建）/
//! SQLite（rusqlite，bundled-sqlcipher——支持 PRAGMA key 加密）六引擎的
//! enum 分发。不用 dyn trait —— async fn in trait 非 dyn-compatible，
//! 多变体 enum 是最轻的分发形态。
//!
//! TLS 四档映射（客户端实现视角）：
//! - disable：明文（PG 走 NoTls；MySQL 不带 ssl_opts；Oracle 用 TCP 描述符；
//!   Redis 用 ConnectionAddr::Tcp）
//! - prefer / require：rustls + 跳过证书校验（不做明文回退，TLS 失败即报错；
//!   Redis 走 ConnectionAddr::TcpTls{insecure:true}）
//! - verify-full：rustls + 系统根证书（rustls-native-certs）校验域名
//!
//! Redis 侧说明：Redis 没有 schema/表这类协议级元信息，对象树是「逻辑库 →
//! 值类型分组 → key」三层，靠 SCAN + TYPE 探测；逻辑库清单只能由 CONFIG GET
//! databases 推出（协议本身不提供枚举）。执行层是单行命令（sqlsplit 按行
//! 分割），没有 DDL/DML 语义，也没有 affected-rows 概念。
//!
//! Oracle 侧说明：oracle_async 是纯 Rust 原生异步驱动（TNS/TTC 线路协议），
//! 零 C 依赖——无需 Oracle Instant Client / OCI / ODPI-C，各臂直接 await。
//! 取消经 TNS Break Marker 真中断（tokio::select! 协作）；TLS/TCPS 不实现
//! （前端 Oracle 表单隐藏 SSL 选项，后端对非 disable 直接报错，不做静默
//! 明文回退）。
//!
//! OceanBase 侧说明：Oracle 模式走纯 Rust 分叉驱动（同 MySQL 臂四档 TLS
//! 能力）；取消粒度：语句间生效（会话层 AtomicBool），行流中不可打断，
//! 超时后会话层置废连接。
//!
//! SQLite 侧说明：本地文件库（db_path 空 = :memory:），打开不带 CREATE 标志
//!（文件不存在即报错，绝不静默建空库）；加密走 SQLCipher 的 PRAGMA key
//!（sqlcipher2/3 在 key 之后设 cipher_compatibility——SQLCipher 的该 pragma
//! 要求 codec 已由 key 附加，反过来会被静默忽略；sqlcipher4 即默认档位，
//! sqlcipher-raw 直接给 x'<64 位十六进制>' 原始密钥字面量）。rusqlite 的
//! Connection 是 Send 非 Sync，所有阻塞调用经 with_sqlite 的 spawn_blocking
//! 包裹；取消经 progress handler——每 1000 条 VDBE
//! 指令轮询会话标志，命中即 SQLITE_INTERRUPT（exec_streaming 映射为
//! cancelled=true，不是错误）。
//!
//! rustls provider 全局唯一（aws-lc-rs，见 Cargo.toml 约束），MySQL 侧用
//! mysql_async 的 default-rustls feature 同源。

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use futures_util::StreamExt as _;
use serde::Serialize;

use crate::db::DbConnectionRecord;
use oracle_async::Queryable as _;

/// Redis 引擎专属实现（拓扑/命令/键检视/分词），从本文件拆出。
///
/// 子模块取名 `redis_engine` 而非 `redis`：后者会与本文件内大量使用的
/// crate `redis::*` 路径冲突（Rust 中同名模块优先于 extern crate）。
pub mod redis_engine;
/// Oracle 引擎专属辅助（含 OB Oracle 共用的 all_users 黑名单口径），从本文件拆出。
pub mod oracle_engine;
/// 真机回归（#[ignore]，读本机 veloterm.db 的 oracle 连接记录直连验证）
#[cfg(test)]
mod oracle_live_test;
/// Oracle 辅助在 mod.rs 的 match 臂与 db_browser 命令层使用（`pub(crate)` 级，
/// 不能再经 `pub use` 外露）
pub(crate) use self::oracle_engine::{
    ob_quote_literal, oracle_err, oracle_row_string,
    oracle_usernames,
};
/// 仅测试模块使用（`use super::*` 引入）
#[cfg(test)]
pub(crate) use self::oracle_engine::oracle_ora_hint;
/// 达梦 DM8 引擎专属辅助（schema 清单/标识符校验/版本探测/值字符串化），
/// 从本文件拆出。
pub mod dameng_engine;
pub(crate) use self::dameng_engine::{
    dm_query_first_string, dm_query_rows, dm_query_strings, dm_row_i64, dm_row_string,
    dm_schema_names, dm_server_version, dm_text_at, dm_valid_identifier, dm_value_to_string,
};
/// 仅测试模块使用（`use super::*` 引入）
#[cfg(test)]
pub(crate) use self::dameng_engine::dm_value_to_i64;
/// SQL 系三引擎（SQLite / MySQL / PG）的单元格取值与标识符转义，从本文件拆出。
pub mod sql_engines;
pub(crate) use self::sql_engines::{
    mysql_value_to_string, pg_cell, quote_ident, quote_string_literal, sqlite_cell,
    sqlite_database_names, sqlite_err, sqlite_is_interrupt,
};
/// 各引擎建连逻辑 + TLS 辅助（连接管理与引擎操作职责分离）。
mod connect;
/// 表/视图详情（DDL/列/索引/约束/反向外键/注释/统计），按引擎拆子模块。
mod table_detail;
/// 对象定义（函数/过程/序列/触发器/同义词/包/…），按引擎拆子模块。
mod object_definition;
/// 对象树目录层（分组计数等），按函数拆子模块。
mod catalog;
pub use self::redis_engine::{
    RedisBigkeysData, RedisConn, RedisKeyData, RedisKeyWrite, RedisTableData,
};
/// Redis 子模块与 mod.rs 的 match 臂共用的辅助函数（连接期 + 运行期探测）。
/// `redis_run_pipe` / `redis_read_key_full` / `redis_write_key_full` /
/// `redis_slowlog_rows` / `redis_bigkeys_rows` 由 db_browser 命令层直接调用，
/// 经下面的 `pub use` 暴露。
use self::redis_engine::{
    RedisExtraOptions, redis_addr, redis_connect_error, redis_connection_info, redis_database_count,
    redis_db_index, redis_extra_options, redis_inspect_block, redis_key_entries, redis_key_type,
    redis_keyspace_counts, redis_node_list, redis_read_command, redis_run_cmd,
    redis_scan_typed_keys, redis_select, redis_split_args, redis_tls_certs, redis_value_string,
    redis_value_to_table, redis_version,
};
/// 测试模块（`use super::*`）也要用到的表格化辅助
#[cfg(test)]
use self::redis_engine::{redis_info_table, redis_scan_table};
pub use self::redis_engine::{
    redis_bigkeys_rows, redis_read_key_full, redis_run_pipe, redis_slowlog_rows,
    redis_topology_of, redis_write_key_full,
};

/// 目录树节点（前端懒加载消费）。
/// group 节点：库/schema 下的固定分组（表/视图/函数/过程/序列），key 形如
/// `{schema}#{group}`，name 为组名大写（TABLE/VIEW/…，前端 i18n 映射显示），
/// detail 为对象计数；空分组不返回。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogNode {
    pub key: String,
    pub name: String,
    /// database（MySQL 顶层）| schema（PG 顶层）| group | table | view
    /// | function | procedure | sequence | column
    pub kind: String,
    /// 附加说明：group 显示计数、view 显示 VIEW、column 显示数据类型
    pub detail: Option<String>,
}

/// 目录父层（懒加载游标）。
#[derive(Debug, Clone)]
pub enum CatalogParent {
    Root,
    Schema { schema: String },
    Group { schema: String, group: ObjectGroup },
    Table { schema: String, table: String },
}

/// 对象分组类型（表/视图/函数/存储过程/序列 + Redis 六种值类型）。
///
/// Redis 的六个别名带 `r` 前缀（rstring/rhash/…）：`as_str()` 同时是目录树的
/// key 段与前端 CatalogParentArg 的 `group` 参数，若用裸类型名（`string`）
/// 会与 SQL 的 table/view 命名空间混在一起——尤其 `set` 既像集合类型又像
/// SQL 语句。裸类型名只在 parse() 入口兼容接受（前端/外部调用方友好）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectGroup {
    Table,
    View,
    Function,
    Procedure,
    Sequence,
    /// 触发器（MySQL/PG/Oracle/达梦均有；DBeaver 同名列对象类型）
    Trigger,
    /// 物化视图（PG 的 pg_class relkind='m'；MySQL/OB MySQL 模式无此类型）
    MaterializedView,
    /// 外部表（PG relkind='f'，FDW 外部数据封装；DBeaver Foreign Tables 同义）
    ForeignTable,
    /// 扩展（PG pg_extension，按 extnamespace 归属 schema）
    Extension,
    /// DB Link（Oracle all_db_links）
    DbLink,
    /// 回收站（Oracle/达梦 dba_recyclebin/user_recyclebin，BIN$ 闪回对象）
    RecycleBin,
    /// 用户自定义类型（Oracle all_objects OBJECT_TYPE='TYPE'）
    DataType,
    /// 表空间（达梦 V$TABLESPACE；实例级对象，挂在 SYS schema 下）
    Tablespace,
    /// 用户（达梦 SYSOBJECTS SUBTYPE$='USER'；实例级，挂 SYS schema）
    DmUser,
    /// 角色（达梦 SYSOBJECTS TYPE$='UR' AND SUBTYPE$='ROLE'；实例级，挂 SYS）
    DmRole,
    /// 同义词（Oracle 系特有：Oracle ALL_SYNONYMS / 达梦 SYSOBJECTS）
    Synonym,
    /// 包（Oracle 系特有：PACKAGE / PACKAGE BODY）
    Package,
    /// 事件（MySQL Event Scheduler）
    Event,
    /// 规则（PG pg_rewrite）
    Rule,
    RedisString,
    RedisHash,
    RedisList,
    RedisSet,
    RedisZset,
    RedisStream,
}

impl ObjectGroup {
    pub fn as_str(&self) -> &'static str {
        match self {
            ObjectGroup::Table => "table",
            ObjectGroup::View => "view",
            ObjectGroup::Function => "function",
            ObjectGroup::Procedure => "procedure",
            ObjectGroup::Sequence => "sequence",
            ObjectGroup::Trigger => "trigger",
            ObjectGroup::MaterializedView => "mview",
            ObjectGroup::ForeignTable => "ftable",
            ObjectGroup::Extension => "extension",
            ObjectGroup::DbLink => "dblink",
            ObjectGroup::RecycleBin => "recycle",
            ObjectGroup::DataType => "datatype",
            ObjectGroup::Tablespace => "tablespace",
            ObjectGroup::DmUser => "user",
            ObjectGroup::DmRole => "role",
            ObjectGroup::Synonym => "synonym",
            ObjectGroup::Package => "package",
            ObjectGroup::Event => "event",
            ObjectGroup::Rule => "rule",
            ObjectGroup::RedisString => "rstring",
            ObjectGroup::RedisHash => "rhash",
            ObjectGroup::RedisList => "rlist",
            ObjectGroup::RedisSet => "rset",
            ObjectGroup::RedisZset => "rzset",
            ObjectGroup::RedisStream => "rstream",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "table" => Some(ObjectGroup::Table),
            "view" => Some(ObjectGroup::View),
            "function" => Some(ObjectGroup::Function),
            "procedure" => Some(ObjectGroup::Procedure),
            "sequence" => Some(ObjectGroup::Sequence),
            "trigger" => Some(ObjectGroup::Trigger),
            "mview" | "materialized view" => Some(ObjectGroup::MaterializedView),
            "ftable" | "foreign table" => Some(ObjectGroup::ForeignTable),
            "extension" => Some(ObjectGroup::Extension),
            "dblink" | "db link" => Some(ObjectGroup::DbLink),
            "recycle" | "recyclebin" => Some(ObjectGroup::RecycleBin),
            "datatype" | "type" => Some(ObjectGroup::DataType),
            "tablespace" => Some(ObjectGroup::Tablespace),
            "user" => Some(ObjectGroup::DmUser),
            "role" => Some(ObjectGroup::DmRole),
            "synonym" => Some(ObjectGroup::Synonym),
            "package" => Some(ObjectGroup::Package),
            "event" => Some(ObjectGroup::Event),
            "rule" => Some(ObjectGroup::Rule),
            "rstring" | "string" => Some(ObjectGroup::RedisString),
            "rhash" | "hash" => Some(ObjectGroup::RedisHash),
            "rlist" | "list" => Some(ObjectGroup::RedisList),
            "rset" | "set" => Some(ObjectGroup::RedisSet),
            "rzset" | "zset" => Some(ObjectGroup::RedisZset),
            "rstream" | "stream" => Some(ObjectGroup::RedisStream),
            _ => None,
        }
    }

    /// 分组 → Redis TYPE 命令返回的类型名（仅 Redis 变体有值）
    pub fn redis_type(&self) -> Option<&'static str> {
        match self {
            ObjectGroup::RedisString => Some("string"),
            ObjectGroup::RedisHash => Some("hash"),
            ObjectGroup::RedisList => Some("list"),
            ObjectGroup::RedisSet => Some("set"),
            ObjectGroup::RedisZset => Some("zset"),
            ObjectGroup::RedisStream => Some("stream"),
            _ => None,
        }
    }

    /// Redis TYPE 命令返回的类型名 → 分组（未知/新类型返回 None，不进树）
    fn from_redis_type(value: &str) -> Option<Self> {
        match value {
            "string" => Some(ObjectGroup::RedisString),
            "hash" => Some(ObjectGroup::RedisHash),
            "list" => Some(ObjectGroup::RedisList),
            "set" => Some(ObjectGroup::RedisSet),
            "zset" => Some(ObjectGroup::RedisZset),
            "stream" => Some(ObjectGroup::RedisStream),
            _ => None,
        }
    }
}

/// 一条语句的执行摘要。
#[derive(Debug, Clone, Default)]
pub struct ExecSummary {
    pub columns: Vec<String>,
    pub rows_read: usize,
    pub truncated: bool,
    pub affected: Option<u64>,
    pub cancelled: bool,
}

/// 批量回传粒度（行/批）：Channel 事件按批发，前端渐进渲染。
const ROWS_PER_BATCH: usize = 300;

/// 函数/过程的定义与调用骨架。arg_count > 0 时前端引导用户补参后再执行。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectDefinition {
    pub definition: String,
    /// 可直接执行的调用语句（无参形态；有参时前端引导补参）
    pub call_template: String,
    pub arg_count: usize,
}

/// 表/视图详情：DDL + 列结构 + 索引清单 + 约束清单（树右键「表详情」）。
/// 表头用英文通用词（后端不做 i18n，前端 DataTable 直接展示）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDetail {
    pub ddl: String,
    /// 表注释（DBeaver ALL_TAB_COMMENTS / TABLE_COMMENT / obj_description 同源；
    /// 无注释为空串）
    #[serde(default)]
    pub comment: String,
    /// 表大小（人类可读，如 "12.5 MB"；PG 走 pg_total_relation_size、MySQL 走
    /// DATA_LENGTH+INDEX_LENGTH、Oracle 走段统计、达梦走 TABLE_USED_PAGES*PAGE；
    /// 拿不到为空串——各引擎的统计口径略有差异，仅做展示参考）
    #[serde(default)]
    pub size: String,
    /// 行数（近似值：Oracle/达梦取统计信息 NUM_ROWS，MySQL 取 TABLE_ROWS，
    /// PG 用精确 COUNT 仅当表小时——统一取统计信息，大表不做 COUNT）
    #[serde(default)]
    pub row_count: String,
    pub column_headers: Vec<String>,
    pub columns: Vec<Vec<String>>,
    pub index_headers: Vec<String>,
    pub indexes: Vec<Vec<String>>,
    /// 约束（PK/UK/FK/CHECK）：name / type / columns / references。
    /// type 用短码：PK / U / F / C；references 仅外键有值（"schema.table(col,...)"，
    /// 非 NO ACTION 的 ON UPDATE/DELETE 规则以文本后缀追加）
    #[serde(default)]
    pub constraint_headers: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<Vec<String>>,
    /// 反向外键（哪些表引用了本表；DBeaver References Tab 同义）：
    /// name / 引用表 / 引用列 / 本表被引用列。headers 空则前端隐藏 Tab
    #[serde(default)]
    pub reference_headers: Vec<String>,
    #[serde(default)]
    pub references: Vec<Vec<String>>,
}

/// SQLite 反向外键行：引用表 / 外键序号 / 引用方列 / 被引用列 /
/// ON UPDATE 规则 / ON DELETE 规则
pub(crate) type SqliteIncomingRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// MySQL 列结构行：name / column_type / is_nullable / default / comment
pub(crate) type MySqlColumnRow = (String, String, String, Option<String>, Option<String>);

/// SQLite 引擎（本地文件库）。rusqlite::Connection 是 Send 非 Sync，
/// 所有阻塞调用经 spawn_blocking 包裹。
pub struct SqliteEngine {
    conn: rusqlite::Connection,
    /// 会话取消标志：progress handler 每 1000 条 VDBE 指令轮询一次
    cancelled: Arc<AtomicBool>,
}

impl SqliteEngine {
    /// 绑定会话层取消标志并安装 progress handler（命中即中断当前语句）。
    /// 标志用 Arc 共享：C 回调只拿到裸指针，Arc 的所有权留在 self.cancelled
    /// 里保证指针在连接存活期间一直有效（rusqlite 的安全包装 progress_handler
    /// 在 hooks feature 下才导出，本工程未启用，故直接经 ffi 注册）。
    fn bind_cancel(&mut self, flag: Arc<AtomicBool>) {
        // 幂等：同一标志已绑定过（handler 已装好）则无需重复注册
        if Arc::ptr_eq(&self.cancelled, &flag) {
            return;
        }
        // 先注册（指向局部 flag 的堆数据，此刻仍归本函数所有），再交给
        // self.cancelled 持有：两步之间没有 SQLite 执行，回调不会被触发，
        // 不会出现悬垂指针窗口。
        let flag_ptr = Arc::as_ptr(&flag) as *mut std::os::raw::c_void;
        // SAFETY: 回调只对该地址做原子读；地址有效性由 self.cancelled 的 Arc 保证。
        unsafe {
            rusqlite::ffi::sqlite3_progress_handler(
                self.conn.handle(),
                1000,
                Some(sqlite_progress_trampoline),
                flag_ptr,
            );
        }
        self.cancelled = flag;
    }
}

/// progress handler 的 C 回调：返回非 0 即中断当前语句（SQLITE_INTERRUPT）。
unsafe extern "C" fn sqlite_progress_trampoline(
    ctx: *mut std::os::raw::c_void,
) -> std::os::raw::c_int {
    if ctx.is_null() {
        return 0;
    }
    // SAFETY: ctx 由 bind_cancel 传入，指向存活的 AtomicBool（见该处说明）。
    let flag = unsafe { &*(ctx as *const AtomicBool) };
    if flag.load(std::sync::atomic::Ordering::Relaxed) {
        1
    } else {
        0
    }
}

/// 引擎实例：一会话一连接（单查询工作台模型，不做池）。Closed 为优雅关闭后的占位。
/// Oracle 走 oracle_async（纯 Rust 异步，各臂直接 await）；SQLite /
/// 达梦为同步驱动（rusqlite / dm-pure 无 async），持有连接分别经
/// with_sqlite / with_dameng 包裹调用。Redis 用 RedisConn（单机
/// MultiplexedConnection / 集群 ClusterConnection，内部均单任务串行化请求/
/// 响应，clone 廉价）——与「单连接工作台」模型天然契合，不需要额外池。
pub enum DbEngine {
    MySql(mysql_async::Conn),
    Postgres(tokio_postgres::Client),
    Oracle(oracle_async::Conn),
    Redis {
        conn: RedisConn,
        /// 当前逻辑库索引（SELECT n 的目标；元信息展示为 "db{n}"）
        db: u32,
        /// Cluster 拓扑：仅 db0、禁切库（SELECT 在集群协议下不可用）；
        /// sentinel 解析出 master 后按单机连接，该标记恒 false
        cluster: bool,
    },
    Sqlite(SqliteEngine),
    Dameng(dm_pure::DmClient),
    Closed,
}

impl DbEngine {
/// 建连并取服务端版本描述（实现见 connect.rs）。OceanBase 按 mode 分双模式：
/// MySQL 模式复用 mysql_async 臂（engine 变体仍是 MySql），Oracle 模式固定走
/// 纯 Rust 分叉驱动（connect_ob_rust，结果同样落在 MySql 变体）。
pub async fn connect(record: &DbConnectionRecord) -> Result<(DbEngine, String), String> {
    connect::connect(record).await
}


    /// 优雅关闭（MySQL 发 COM_QUIT；PG 直接断；Redis 的 multiplexed 连接随
    /// Drop 关闭后台任务无需发命令）——消费自身，置 Closed 占位。
    pub async fn close(&mut self) {
        let taken = std::mem::replace(self, DbEngine::Closed);
        if let DbEngine::MySql(conn) = taken {
            let _ = conn.disconnect().await;
        } else if let DbEngine::Dameng(mut client) = taken {
            // dm-pure 的 close 只是丢弃 socket（无网络往返），同步调用即可
            client.close();
        }
        // SQLite：rusqlite::Connection 的 Drop 即 sqlite3_close（文件句柄与加密
        // codec 一并释放），无需显式动作
    }

    /// 绑定会话取消标志（仅 SQLite 生效：progress handler 轮询；其它引擎空操作）。
    pub fn bind_cancel_flag(&mut self, flag: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        if let DbEngine::Sqlite(engine) = self {
            engine.bind_cancel(flag);
        }
    }











    /// 同步 SQLite 连接的阻塞包裹：连接取出 → spawn_blocking 内执行闭包 →
    /// 连接归还（错误也归还；panic/取消置 Closed 由会话层重建）。
    /// rusqlite 的 Connection 是 Send 非 Sync，不能跨 await 直接持有。
    async fn with_sqlite<T, F>(&mut self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Connection) -> Result<T, String> + Send + 'static,
    {
        let taken = std::mem::replace(self, DbEngine::Closed);
        let DbEngine::Sqlite(engine) = taken else {
            *self = taken;
            return Err("连接已关闭".to_string());
        };
        let result = tokio::task::spawn_blocking(move || {
            let r = f(&engine.conn);
            (r, engine)
        })
        .await
        .map_err(|e| format!("SQLite 任务失败：{e}"));
        match result {
            Ok((r, engine)) => {
                *self = DbEngine::Sqlite(engine);
                r
            }
            Err(e) => {
                // JoinError（panic/取消）：连接已无法取回，置 Closed 由会话层重建
                *self = DbEngine::Closed;
                Err(e)
            }
        }
    }


    /// 同步达梦连接的阻塞包裹：连接取出 → spawn_blocking
    /// 内执行闭包 → 连接归还（错误/取消也归还，避免连接丢失）。dm-pure 是
    /// 纯同步驱动（std::net），闭包内可直接调用其 API。
    async fn with_dameng<T, F>(&mut self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&mut dm_pure::DmClient) -> Result<T, String> + Send + 'static,
    {
        let taken = std::mem::replace(self, DbEngine::Closed);
        let DbEngine::Dameng(mut client) = taken else {
            *self = taken;
            return Err("连接已关闭".to_string());
        };
        let result = tokio::task::spawn_blocking(move || {
            let r = f(&mut client);
            (r, client)
        })
        .await
        .map_err(|e| format!("达梦任务失败：{e}"));
        match result {
            Ok((r, client)) => {
                *self = DbEngine::Dameng(client);
                r
            }
            Err(e) => {
                // JoinError（panic/取消）：连接已无法取回，置 Closed 由会话层重建
                *self = DbEngine::Closed;
                Err(e)
            }
        }
    }


    /// 可切换的数据库清单（MySQL=全部 schema；PG=服务器上可连的 database，
    /// 与 catalog_root 的 schema 层不同——PG 切库是连接级操作；Redis=逻辑库
    /// db0..db(N-1)，N 来自 CONFIG GET databases）。
    pub async fn list_databases(&mut self) -> Result<Vec<String>, String> {
        match self {
            DbEngine::MySql(conn) => {
                // OB Oracle：清单 = 可见 schema（ALL_USERS），切换走
                // CURRENT_SCHEMA（与原生 Oracle 臂同源语义）
                if conn.ob_is_oracle_mode() {
                    return ob_oracle_usernames(conn).await;
                }
                use mysql_async::prelude::Queryable as _;
                let names: Vec<String> = conn
                    .exec(
                        "SELECT schema_name FROM information_schema.schemata ORDER BY schema_name",
                        (),
                    )
                    .await
                    .map_err(|e| format!("MySQL 查询库清单失败：{e}"))?;
                // OB MySQL 租户：滤掉内建 Oracle 兼容 schema（原生 MySQL 恒等）
                Ok(ob_filter_schemas(conn, names))
            }
            DbEngine::Postgres(client) => {
                let rows = client
                    .query(
                        "SELECT datname FROM pg_database \
                         WHERE datallowconn AND NOT datistemplate ORDER BY datname",
                        &[],
                    )
                    .await
                    .map_err(|e| format!("PostgreSQL 查询库清单失败：{e}"))?;
                Ok(rows
                    .iter()
                    .filter_map(|row| row.try_get::<_, String>(0).ok())
                    .collect())
            }
            DbEngine::Oracle(conn) => {
                // Oracle 无连接级切库：清单 = 可见 schema（版本适配滤系统账号，
                // 见 oracle_usernames），切换走 select_database 的 CURRENT_SCHEMA
                oracle_usernames(conn).await
            }
            DbEngine::Redis { conn, cluster, .. } => {
                // 集群只有 db0（SELECT 在集群协议下不可用），清单收窄避免误导。
                // Redis 协议没有「枚举逻辑库」的命令：库数由 redis.conf 的
                // databases 决定（默认 16），只能经 CONFIG GET 读。CONFIG 被
                // rename/禁用（云托管常见）时按默认 16 兜底——清单只是给用户
                // 选，不准确也不影响实际 SELECT。
                if *cluster {
                    return Ok(vec!["db0".to_string()]);
                }
                let count = redis_database_count(conn).await.unwrap_or(16);
                Ok((0..count.max(1)).map(|i| format!("db{i}")).collect())
            }
            DbEngine::Sqlite(_) => {
                // 已附加的库（PRAGMA database_list 的 name 列：main/temp 及
                // ATTACH 进来的库）。SQLite 无连接级切库，前端对 SQLite 隐藏
                // 该选择器，此处仅为命令兼容
                self.with_sqlite(sqlite_database_names).await
            }
            DbEngine::Dameng(_) => {
                // 达梦的「库」层即 schema，清单取 SYSOBJECTS 的模式对象
                //（TYPE$='SCH'，见 DM_SCHEMAS_SQL 注释）。不滤系统 schema——
                // 工作台需要能看到 SYS/SYSDBA 等库供排查
                self.with_dameng(dm_schema_names).await
            }
            DbEngine::Closed => Err("连接已关闭".to_string()),
        }
    }

    /// MySQL 会话内切库（USE `db``，库名反引号转义）；PG 的切换在会话层走重连。
    pub async fn select_database(&mut self, database: &str) -> Result<(), String> {
        match self {
            DbEngine::MySql(conn) => {
                // OB Oracle：切库 = ALTER SESSION SET CURRENT_SCHEMA（Oracle
                // 语义，只改名字解析不改权限）；标识符校验同原生 Oracle 臂
                if conn.ob_is_oracle_mode() {
                    use mysql_async::prelude::Queryable as _;
                    let valid = !database.is_empty()
                        && database
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '#'));
                    if !valid {
                        return Err("OceanBase Oracle 无效的 schema 名".to_string());
                    }
                    return conn
                        .query_drop(format!("ALTER SESSION SET CURRENT_SCHEMA = {database}"))
                        .await
                        .map_err(|e| format!("OceanBase 切换 schema 失败：{e}"));
                }
                use mysql_async::prelude::Queryable as _;
                let escaped = database.replace('`', "``");
                conn.query_drop(format!("USE `{escaped}`"))
                    .await
                    .map_err(|e| format!("MySQL 切换数据库失败：{e}"))
            }
            DbEngine::Postgres(_) => {
                Err("PostgreSQL 切换数据库需重连，由会话层处理".to_string())
            }
            DbEngine::Oracle(conn) => {
                // CURRENT_SCHEMA 只改名字解析不改权限；拼进 DDL 前必须校验
                // 标识符字符集（Oracle 普通标识符 [A-Za-z0-9_$#]，不带引号）
                let valid = !database.is_empty()
                    && database
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '#'));
                if !valid {
                    return Err("Oracle 无效的 schema 名".to_string());
                }
                let sql = format!("ALTER SESSION SET CURRENT_SCHEMA = {database}");
                conn.exec(&sql)
                    .await
                    .map_err(|e| format!("Oracle 切换 schema 失败：{}", oracle_err(e)))?;
                Ok(())
            }
            DbEngine::Redis { conn, db, cluster } => {
                // SELECT 是连接级状态：multiplexed 连接上所有后续命令都落在这个
                // 库里。接受 "3" 与 "db3" 两种写法（与连接表单的两种习惯一致），
                // 解析失败直接报错而不是静默回落到 db0——切错库比报错更危险。
                // 集群仅 db0：协议不允许 SELECT
                if *cluster {
                    return Err("Redis Cluster 只有一个逻辑库（db0），无法切换".to_string());
                }
                let target = parse_redis_db(database)
                    .ok_or_else(|| format!("无效的 Redis 逻辑库：{database}（应为 0 或 db0 形态）"))?;
                let mut cmd = redis::cmd("SELECT");
                cmd.arg(target);
                redis_run_cmd::<()>(conn, &mut cmd)
                    .await
                    .map_err(|e| format!("Redis 切换逻辑库失败：{e}"))?;
                *db = target;
                Ok(())
            }
            DbEngine::Sqlite(_) => {
                // SQLite 无连接级切库（对象树顶层已是 database_list 的附加库），
                // 会话层只更新元信息；不报错以保持命令语义一致
                Ok(())
            }
            DbEngine::Dameng(_) => {
                // 同 Oracle：标识符字符集校验后拼进 SET SCHEMA（达梦普通标识符
                // 同为 [A-Za-z0-9_$#]）。不加双引号——未引号标识符折叠成大写，
                // 与 Oracle 臂的宽容度一致
                if !dm_valid_identifier(database) {
                    return Err("达梦无效的 schema 名".to_string());
                }
                let sql = format!("SET SCHEMA {database}");
                self.with_dameng(move |client| {
                    client
                        .execute(&sql, &[])
                        .map_err(|e| format!("达梦切换 schema 失败：{e}"))?;
                    Ok(())
                })
                .await
            }
            DbEngine::Closed => Err("连接已关闭".to_string()),
        }
    }
    /// 表/视图详情（kind: "table" | "view"）：MySQL 用 SHOW CREATE +
    /// information_schema；PG 无 SHOW CREATE，DDL 由列元数据+主键拼接（附
    /// pg_indexes 的真实索引 DDL），视图用 pg_get_viewdef。
    /// 各引擎实现拆至 `table_detail/` 子模块。
    pub async fn table_detail(
        &mut self,
        schema: &str,
        table: &str,
        kind: &str,
    ) -> Result<TableDetail, String> {
        match self {
            DbEngine::MySql(conn) => {
                table_detail::mysql::detail(conn, schema, table, kind).await
            }
            DbEngine::Postgres(client) => {
                table_detail::postgres::detail(client, schema, table, kind).await
            }
            DbEngine::Oracle(conn) => {
                table_detail::oracle::detail(conn, schema, table, kind).await
            }
            DbEngine::Redis { conn, db, .. } => {
                table_detail::redis::detail(conn, *db, schema, table).await
            }
            DbEngine::Sqlite(_) => {
                table_detail::sqlite::detail(self, schema, table).await
            }
            DbEngine::Dameng(_) => {
                table_detail::dameng::detail(self, schema, table, kind).await
            }
            DbEngine::Closed => Err("连接已关闭".to_string()),
        }
    }
    /// 拉取函数/过程/序列的完整定义与调用骨架
    ///（kind: "function" | "procedure" | "sequence"；序列模板取 nextval）。
    /// MySQL/PG/Oracle/达梦的实现拆至 `object_definition/` 子模块；
    /// Redis 与 SQLite 臂很短，留在分发处。
    pub async fn object_definition(
        &mut self,
        schema: &str,
        name: &str,
        kind: &str,
    ) -> Result<ObjectDefinition, String> {
        match self {
            DbEngine::MySql(conn) => {
                object_definition::mysql::definition(conn, schema, name, kind).await
            }
            DbEngine::Postgres(client) => {
                object_definition::postgres::definition(client, schema, name, kind).await
            }
            DbEngine::Oracle(conn) => {
                object_definition::oracle::definition(conn, schema, name, kind).await
            }
            DbEngine::Redis { conn, db, .. } => {
                // Redis 树里没有函数/过程/序列（只有 key），本臂只服务「key
                // 详情」入口：定义块 = 与 table_detail 同一份检视文本，
                // call_template = 该类型最自然的读取命令（前端可一键执行）。
                // 同 table_detail：临时切到 key 所属逻辑库再切回。
                let target = redis_db_of(schema, *db)?;
                redis_select(conn, target).await?;
                let inspected = async {
                    let type_name = redis_key_type(conn, name).await.map_err(|e| {
                        redis_key_miss_hint(e, target, schema)
                    })?;
                    let definition = redis_inspect_block(conn, name, &type_name).await;
                    Ok::<_, String>((definition, redis_read_command(name, &type_name)))
                }
                .await;
                redis_select(conn, *db).await.ok();
                let (definition, call_template) = inspected?;
                Ok(ObjectDefinition {
                    definition,
                    call_template,
                    arg_count: 0,
                })
            }
            DbEngine::Sqlite(_) => {
                // SQLite 无函数/过程/序列对象（树上也不会出现这些分组），
                // 此处仅防御性拒绝
                Err(format!("SQLite 不支持 {kind} 对象"))
            }
            DbEngine::Dameng(_) => {
                object_definition::dameng::definition(self, schema, name, kind).await
            }
            DbEngine::Closed => Err("连接已关闭".to_string()),
        }
    }
    /// 目录懒加载一层。
    pub async fn catalog(
        &mut self,
        parent: &CatalogParent,
    ) -> Result<Vec<CatalogNode>, String> {
        match parent {
            CatalogParent::Root => self.catalog_root().await,
            CatalogParent::Schema { schema } => self.catalog_groups(schema).await,
            CatalogParent::Group { schema, group } => self.catalog_objects(schema, *group).await,
            CatalogParent::Table { schema, table } => self.catalog_columns(schema, table).await,
        }
    }

    async fn catalog_root(&mut self) -> Result<Vec<CatalogNode>, String> {
        match self {
            DbEngine::MySql(conn) => {
                // OB Oracle 租户无 information_schema：顶层 = schema（用户），
                // 走 ALL_USERS（与原生 Oracle 臂同源口径）
                if conn.ob_is_oracle_mode() {
                    return Ok(ob_oracle_usernames(conn)
                        .await?
                        .into_iter()
                        .map(|name| CatalogNode {
                            key: name.clone(),
                            name,
                            kind: "schema".to_string(),
                            detail: None,
                        })
                        .collect());
                }
                use mysql_async::prelude::Queryable as _;
                let names: Vec<String> = conn
                    .exec(
                        "SELECT schema_name FROM information_schema.schemata ORDER BY schema_name",
                        (),
                    )
                    .await
                    .map_err(|e| format!("MySQL 查询库清单失败：{e}"))?;
                // OB MySQL 租户：滤掉内建 Oracle 兼容 schema（原生 MySQL 恒等）
                Ok(ob_filter_schemas(conn, names)
                    .into_iter()
                    .map(|name| CatalogNode {
                        key: name.clone(),
                        name,
                        kind: "database".to_string(),
                        detail: None,
                    })
                    .collect())
            }
            DbEngine::Postgres(client) => {
                let rows = client
                    .query(
                        "SELECT nspname FROM pg_namespace \
                         WHERE nspname NOT IN ('pg_catalog', 'information_schema') \
                           AND nspname NOT LIKE 'pg_toast%' AND nspname NOT LIKE 'pg_temp%' \
                         ORDER BY nspname",
                        &[],
                    )
                    .await
                    .map_err(|e| format!("PostgreSQL 查询 schema 清单失败：{e}"))?;
                Ok(rows
                    .iter()
                    .filter_map(|row| row.try_get::<_, String>(0).ok())
                    .map(|name| CatalogNode {
                        key: name.clone(),
                        name,
                        kind: "schema".to_string(),
                        detail: None,
                    })
                    .collect())
            }
            DbEngine::Oracle(conn) => {
                // 顶层 = schema（用户）：版本适配滤系统账号（oracle_usernames）
                Ok(oracle_usernames(conn)
                    .await?
                    .into_iter()
                    .map(|name| CatalogNode {
                        key: name.clone(),
                        name,
                        kind: "schema".to_string(),
                        detail: None,
                    })
                    .collect())
            }
            DbEngine::Redis { conn, .. } => {
                // 顶层 = 逻辑库。keys 计数走 INFO keyspace 一次拿全（形如
                // "db0:keys=3,expires=0,avg_ttl=0"），比逐个 SELECT+DBSIZE
                // 省 N-1 个往返；INFO 失败不影响清单本身。
                let count = redis_database_count(conn).await.unwrap_or(16);
                let key_counts = redis_keyspace_counts(conn).await;
                Ok((0..count.max(1))
                    .map(|i| {
                        let name = format!("db{i}");
                        let detail = key_counts
                            .get(i as usize)
                            .copied()
                            .filter(|n| *n > 0)
                            .map(|n| format!("{n} keys"));
                        CatalogNode {
                            key: name.clone(),
                            name,
                            kind: "database".to_string(),
                            detail,
                        }
                    })
                    .collect())
            }
            DbEngine::Sqlite(_) => {
                // 顶层 = 已附加的库（main/temp + ATTACH），与 PG/Oracle 的
                // schema 顶层同形
                self.with_sqlite(|conn| {
                    Ok(sqlite_database_names(conn)?
                        .into_iter()
                        .map(|name| CatalogNode {
                            key: name.clone(),
                            name,
                            kind: "schema".to_string(),
                            detail: None,
                        })
                        .collect())
                })
                .await
            }
            DbEngine::Dameng(_) => {
                // 达梦「库」层即 schema（SYSOBJECTS 权威清单，见 dm_schema_names
                // 注释），kind 与 Oracle 臂一致
                self.with_dameng(|client| {
                    Ok(dm_schema_names(client)?
                        .into_iter()
                        .map(|name| CatalogNode {
                            key: name.clone(),
                            name,
                            kind: "schema".to_string(),
                            detail: None,
                        })
                        .collect())
                })
                .await
            }
            DbEngine::Closed => Err("连接已关闭".to_string()),
        }
    }

    /// 库/schema 下固定分组（表/视图/物化视图/函数/过程/触发器/序列），
    /// 带对象计数，空组不返回。SQL 口径对齐 DBeaver 开源实现：
    /// - MySQL：information_schema.TRIGGERS（DBeaver MySQLCatalog.TriggerCache 同源）
    /// - PG：pg_class relkind='m' 物化视图 + pg_trigger（PostgreSchema 同源）
    /// - Oracle：all_objects 加 TRIGGER/CLUSTER（OracleSchema 同源）
    /// - 达梦：SYSOBJECTS 基表（DamengMetaModel 同源，原生口径而非 ALL_* 兼容层）
    ///
    /// 对象树分组计数（实现见 catalog/groups.rs；真机测试 CLI 直驱）。
    pub async fn catalog_groups(
        &mut self,
        schema: &str,
    ) -> Result<Vec<CatalogNode>, String> {
        catalog::groups::groups(self, schema).await
    }
    /// 分组内对象清单（实现见 catalog/objects.rs）。
    pub async fn catalog_objects(
        &mut self,
        schema: &str,
        group: ObjectGroup,
    ) -> Result<Vec<CatalogNode>, String> {
        catalog::objects::objects(self, schema, group).await
    }
    /// 表/视图的列清单（树第三层；真机测试 CLI 同驱）
    pub async fn catalog_columns(
        &mut self,
        schema: &str,
        table: &str,
    ) -> Result<Vec<CatalogNode>, String> {
        match self {
            DbEngine::MySql(conn) => {
                // OB Oracle：all_tab_columns（表与视图同表，与原生 Oracle 臂
                // 同源口径）；列类型取 data_type 裸名。`USER_GENERATED='YES'`
                // 排除系统生成列与隐藏列（ODC 同源）；老版本没有该列时降级
                if conn.ob_is_oracle_mode() {
                    use mysql_async::prelude::Queryable as _;
                    let lit_schema = ob_quote_literal(schema);
                    let lit_table = ob_quote_literal(table);
                    let base = format!(
                        "SELECT column_name, data_type FROM {OB_DICT_PREFIX}_tab_columns \
                         WHERE owner = {lit_schema} AND table_name = {lit_table}"
                    );
                    let sql = format!("{base} AND USER_GENERATED = 'YES' ORDER BY column_id");
                    let rows: Vec<(String, String)> = match conn.exec(sql, ()).await {
                        Ok(rows) => rows,
                        Err(_) => conn
                            .exec(format!("{base} ORDER BY column_id"), ())
                            .await
                            .map_err(|e| format!("OceanBase 查询列清单失败：{e}"))?,
                    };
                    return Ok(rows
                        .into_iter()
                        .map(|(name, data_type)| CatalogNode {
                            key: format!("{schema}.{table}.{name}"),
                            name,
                            kind: "column".to_string(),
                            detail: Some(data_type),
                        })
                        .collect());
                }
                use mysql_async::prelude::Queryable as _;
                let rows: Vec<(String, String)> = conn
                    .exec(
                        "SELECT column_name, data_type FROM information_schema.columns \
                         WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
                        (schema, table),
                    )
                    .await
                    .map_err(|e| format!("MySQL 查询列清单失败：{e}"))?;
                Ok(rows
                    .into_iter()
                    .map(|(name, data_type)| CatalogNode {
                        key: format!("{schema}.{table}.{name}"),
                        name,
                        kind: "column".to_string(),
                        detail: Some(data_type),
                    })
                    .collect())
            }
            DbEngine::Postgres(client) => {
                let rows = client
                    .query(
                        "SELECT column_name, data_type FROM information_schema.columns \
                         WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
                        &[&schema, &table],
                    )
                    .await
                    .map_err(|e| format!("PostgreSQL 查询列清单失败：{e}"))?;
                Ok(rows
                    .iter()
                    .filter_map(|row| {
                        Some((
                            row.try_get::<_, String>(0).ok()?,
                            row.try_get::<_, String>(1).ok()?,
                        ))
                    })
                    .map(|(name, data_type)| CatalogNode {
                        key: format!("{schema}.{table}.{name}"),
                        name,
                        kind: "column".to_string(),
                        detail: Some(data_type),
                    })
                    .collect())
            }
            DbEngine::Oracle(conn) => {
                // all_tab_columns 覆盖表与视图；data_type 概括为裸类型名
                //（与另两引擎的 information_schema 口径一致）。
                // 字典视图前缀自适应（DBA 权限 → dba_*）
                let dict_p = oracle_dict_prefix(conn).await;
                let sql = format!(
                    "SELECT column_name, data_type FROM {dict_p}_tab_columns \
                     WHERE owner = {} AND table_name = {} ORDER BY column_id",
                    ob_quote_literal(schema),
                    ob_quote_literal(table)
                );
                let mut rs = conn
                    .query(&sql)
                    .await
                    .map_err(|e| format!("Oracle 查询列清单失败：{}", oracle_err(e)))?;
                let mut nodes = Vec::new();
                while let Some(row) = rs
                    .next_row()
                    .await
                    .map_err(|e| format!("Oracle 读取列清单失败：{}", oracle_err(e)))?
                {
                    let name = oracle_row_string(&row, 0);
                    let data_type = oracle_row_string(&row, 1);
                    nodes.push(CatalogNode {
                        key: format!("{schema}.{table}.{name}"),
                        name,
                        kind: "column".to_string(),
                        detail: Some(data_type),
                    });
                }
                Ok(nodes)
            }
            DbEngine::Redis { conn, db, .. } => {
                // Redis 的「列」= 该 key 内的字段/成员/元素（按值类型取前 200
                // 条）。detail 放元素的角色标签（field / index / member /
                // score / id），前端列表直接可见而不必展开。
                // 同 table_detail：临时切到 key 所属逻辑库再切回。
                let target = redis_db_of(schema, *db)?;
                redis_select(conn, target).await?;
                let entries = async {
                    let type_name = redis_key_type(conn, table)
                        .await
                        .map_err(|e| redis_key_miss_hint(e, target, schema))?;
                    Ok::<_, String>(redis_key_entries(conn, table, &type_name).await)
                }
                .await;
                redis_select(conn, *db).await.ok();
                let (role, rows) = entries?;
                // 注：前端树不再展开 Redis key（key 到顶，点击直接在结果面板
                // 出内容），本臂保留为通用能力（CLI/后续行级操作可用）。
                Ok(rows
                    .into_iter()
                    .map(|row| {
                        let field = row.first().cloned().unwrap_or_default();
                        let value = row.get(1).cloned().unwrap_or_default();
                        // 角色标签与值并排展示（hash 的 field 列名 / list 的
                        // 下标 / set 的成员 / zset 的分数 / stream 的字段组）；
                        // 值本身为空时只留角色标签
                        let detail = if value.is_empty() {
                            role.to_string()
                        } else {
                            format!("{role} · {value}")
                        };
                        CatalogNode {
                            key: format!("{schema}.{table}#{field}"),
                            name: field,
                            kind: "column".to_string(),
                            detail: Some(detail),
                        }
                    })
                    .collect())
            }
            DbEngine::Sqlite(_) => {
                // table_info 覆盖表与视图；列序即 cid 顺序。pragma 的表名参数
                // 是字符串字面量（不是标识符），用 quote_string_literal 转义
                let schema_o = schema.to_string();
                let table_o = table.to_string();
                self.with_sqlite(move |conn| {
                    let schema_q = quote_ident(&schema_o)?;
                    let sql = format!(
                        "PRAGMA {schema_q}.table_info({})",
                        quote_string_literal(&table_o)
                    );
                    // table_info 列序：cid(0) name(1) type(2) …
                    let cols: Vec<(String, String)> = {
                        let mut stmt = conn
                            .prepare(&sql)
                            .map_err(|e| format!("SQLite 查询列清单失败：{e}"))?;
                        let mut rows = stmt
                            .query_map([], |row| {
                                Ok((
                                    row.get::<_, String>(1)?,
                                    row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                                ))
                            })
                            .map_err(|e| format!("SQLite 查询列清单失败：{e}"))?;
                        let mut out: Vec<(String, String)> = Vec::new();
                        for row in rows.by_ref() {
                            out.push(row.map_err(|e| format!("SQLite 读取列清单失败：{e}"))?);
                        }
                        out
                    };
                    Ok(cols
                        .into_iter()
                        .map(|(name, data_type)| CatalogNode {
                            key: format!("{schema_o}.{table_o}.{name}"),
                            name,
                            kind: "column".to_string(),
                            detail: Some(data_type),
                        })
                        .collect())
                })
                .await
            }
            DbEngine::Dameng(_) => {
                // ALL_TAB_COLUMNS 覆盖表与视图（同 Oracle 臂口径）
                let schema_o = schema.to_string();
                let table_o = table.to_string();
                self.with_dameng(move |client| {
                    let rows = dm_query_rows(
                        client,
                        "SELECT COLUMN_NAME, DATA_TYPE FROM ALL_TAB_COLUMNS \
                         WHERE OWNER = ? AND TABLE_NAME = ? ORDER BY COLUMN_ID",
                        &[
                            dm_pure::Param::Str(schema_o.clone()),
                            dm_pure::Param::Str(table_o.clone()),
                        ],
                    )
                    .map_err(|e| format!("达梦查询列清单失败：{e}"))?;
                    Ok(rows
                        .iter()
                        .map(|row| {
                            let name = dm_row_string(row, 0);
                            CatalogNode {
                                key: format!("{schema_o}.{table_o}.{name}"),
                                name,
                                kind: "column".to_string(),
                                detail: Some(dm_row_string(row, 1)),
                            }
                        })
                        .collect())
                })
                .await
            }
            DbEngine::Closed => Err("连接已关闭".to_string()),
        }
    }

    /// 流式执行一条语句：先回调 columns（有结果集时），再按批回调行；
    /// max_rows 截断（truncated=true）；cancel 组合（Notify + AtomicBool，
    /// 语句间也能感知）触发后 cancelled=true 返回。
    /// 回调约束 Send + 'static：Oracle 同步引擎把回调移入 spawn_blocking。
    pub async fn exec_streaming<F1, F2>(
        &mut self,
        sql: &str,
        max_rows: usize,
        cancel_notify: &tokio::sync::Notify,
        mut on_columns: F1,
        mut on_rows: F2,
    ) -> Result<ExecSummary, String>
    where
        F1: FnMut(&[String]) + Send + 'static,
        F2: FnMut(Vec<Vec<String>>) + Send + 'static,
    {
        match self {
            DbEngine::MySql(conn) => {
                use mysql_async::prelude::Queryable as _;
                let result = conn
                    .query_iter(sql)
                    .await
                    .map_err(|e| format!("MySQL：{e}"))?;
                let columns: Vec<String> = result
                    .columns_ref()
                    .iter()
                    .map(|c| c.name_str().to_string())
                    .collect();
                if columns.is_empty() {
                    // DML / DDL：无结果集，只有 affected rows
                    let affected = result.affected_rows();
                    let _ = result.drop_result().await;
                    return Ok(ExecSummary {
                        affected: Some(affected),
                        ..Default::default()
                    });
                }
                on_columns(&columns);
                let mut summary = ExecSummary {
                    columns,
                    ..Default::default()
                };
                // owned 形态：result 被消费，流耗尽/弃置时自动清理（无借用残留）
                let dropped = result
                    .stream_and_drop::<mysql_async::Row>()
                    .await
                    .map_err(|e| format!("MySQL：{e}"))?;
                if let Some(stream) = dropped {
                    // ResultSetStream 带 PhantomPinned（!Unpin）：pin 住再 next
                    let mut stream = std::pin::pin!(stream);
                    let mut cancel_fut = std::pin::pin!(cancel_notify.notified());
                    let mut batch: Vec<Vec<String>> = Vec::with_capacity(ROWS_PER_BATCH);
                    while summary.rows_read < max_rows {
                        let mut s = stream.as_mut();
                        let next_row = tokio::select! {
                            _ = &mut cancel_fut => {
                                summary.cancelled = true;
                                break;
                            }
                            r = s.next() => r,
                        };
                        let Some(row) = next_row else {
                            break;
                        };
                        let row = row.map_err(|e| format!("MySQL：{e}"))?;
                        let cells: Vec<String> = (0..summary.columns.len())
                            .map(|i| {
                                row.get::<mysql_async::Value, usize>(i)
                                    .map(|v| mysql_value_to_string(&v))
                                    .unwrap_or_else(|| "NULL".to_string())
                            })
                            .collect();
                        batch.push(cells);
                        summary.rows_read += 1;
                        if batch.len() >= ROWS_PER_BATCH {
                            on_rows(std::mem::take(&mut batch));
                        }
                    }
                    if !batch.is_empty() {
                        on_rows(std::mem::take(&mut batch));
                    }
                    if !summary.cancelled && summary.rows_read >= max_rows {
                        // 再探一行判定截断
                        let mut s = stream.as_mut();
                        if s.next().await.is_some() {
                            summary.truncated = true;
                        }
                    }
                }
                Ok(summary)
            }
            DbEngine::Postgres(client) => {
                let raw = client
                    .query_raw(
                        sql,
                        std::iter::empty::<&(dyn tokio_postgres::types::ToSql + Sync)>(),
                    )
                    .await
                    .map_err(|e| format!("PostgreSQL：{e}"))?;
                let mut stream = std::pin::pin!(raw);
                let mut summary = ExecSummary::default();
                let mut columns_sent = false;
                let mut cancel_fut = std::pin::pin!(cancel_notify.notified());
                let mut batch: Vec<Vec<String>> = Vec::with_capacity(ROWS_PER_BATCH);
                while summary.rows_read < max_rows {
                    let mut s = stream.as_mut();
                    let next_row = tokio::select! {
                        _ = &mut cancel_fut => {
                            summary.cancelled = true;
                            break;
                        }
                        r = s.next() => r,
                    };
                    let Some(row) = next_row else {
                        break;
                    };
                    let row = row.map_err(|e| format!("PostgreSQL：{e}"))?;
                    if summary.columns.is_empty() {
                        summary.columns = row
                            .columns()
                            .iter()
                            .map(|c| c.name().to_string())
                            .collect();
                    }
                    if !columns_sent && !summary.columns.is_empty() {
                        on_columns(&summary.columns);
                        columns_sent = true;
                    }
                    let cells: Vec<String> = (0..summary.columns.len())
                        .map(|i| pg_cell(&row, i))
                        .collect();
                    batch.push(cells);
                    summary.rows_read += 1;
                    if batch.len() >= ROWS_PER_BATCH {
                        on_rows(std::mem::take(&mut batch));
                    }
                }
                if !batch.is_empty() {
                    on_rows(std::mem::take(&mut batch));
                }
                if !summary.cancelled && summary.rows_read >= max_rows {
                    let mut s = stream.as_mut();
                    if s.next().await.is_some() {
                        summary.truncated = true;
                    }
                }
                // PG extended protocol 拿不到 DML 的 affected rows（空列即视为已执行）
                Ok(summary)
            }
            DbEngine::Oracle(conn) => {
                // 纯 Rust 异步驱动：query 流式取行，PL/SQL 匿名块的隐式结果集
                // 由 QueryResult 自动接续（oracle_async 的 Msg 27 链）
                let mut rs = tokio::select! {
                    res = conn.query(sql) => res.map_err(|e| format!("Oracle：{}", oracle_err(e)))?,
                    _ = cancel_notify.notified() => {
                        let _ = conn.break_connection().await;
                        return Ok(ExecSummary {
                            cancelled: true,
                            ..Default::default()
                        });
                    }
                };

                let columns: Vec<String> = rs
                    .columns()
                    .iter()
                    .map(|c| c.name.clone())
                    .collect();

                on_columns(&columns);
                let mut summary = ExecSummary {
                    columns,
                    ..Default::default()
                };

                // 取行段包进 async 块：块内随便 return/break/上抛，块外统一收尾
                // 关游标（Rust 的 Drop 不能 await，故用「块 + 收尾」代替 defer；
                // async_scoped/scopeguard 类工具解决的是结构化并发或同步 Drop，
                // 都不适用于 async 清理）。与 go-ora Connection.QueryContext 的
                // stmt.Close() 收尾语义一致：成功、截断、取消、出错全关
                let read_outcome: Result<(), String> = async {
                    if summary.columns.is_empty() {
                        // DML / DDL / PLSQL 无结果集：affected 走 rows_affected
                        summary.affected = Some(rs.rows_affected());
                        return Ok(());
                    }
                    let mut batch: Vec<Vec<String>> = Vec::with_capacity(ROWS_PER_BATCH);
                    loop {
                        if summary.rows_read >= max_rows {
                            summary.truncated = true;
                            break;
                        }
                        tokio::select! {
                            _ = cancel_notify.notified() => {
                                let _ = rs.break_connection().await;
                                summary.cancelled = true;
                                break;
                            }
                            row_res = rs.next_row() => {
                                match row_res {
                                    Ok(Some(row)) => {
                                        let cells: Vec<String> =
                                            row.values().iter().map(|v| v.to_string()).collect();
                                        batch.push(cells);
                                        summary.rows_read += 1;
                                        if batch.len() >= ROWS_PER_BATCH {
                                            on_rows(std::mem::take(&mut batch));
                                        }
                                    }
                                    Ok(None) => break,
                                    Err(e) => {
                                        return Err(format!(
                                            "Oracle 行读取错误：{}",
                                            oracle_err(e)
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    if !batch.is_empty() {
                        on_rows(batch);
                    }
                    Ok(())
                }
                .await;

                // 统一收尾：无论上面走哪条路径（含 ? 早退）都关游标
                let _ = rs.close().await;
                read_outcome?;
                Ok(summary)
            }
            DbEngine::Redis { conn, db, .. } => {
                // 单行命令执行：tokenize → 一次往返 → 值转表 → 分批回调。
                // 取消粒度 = 单条命令（select! 只在等待往返期间生效）：
                // Redis 命令都是一次往返，没有行流可中断；一旦发出就必然
                // 收到完整回复（或连接错误），不做逐行取消。
                let args = redis_split_args(sql);
                let Some(raw_name) = args.first() else {
                    return Err("Redis 命令为空".to_string());
                };
                // 命令名统一大写后分发（Redis 命令本身大小写不敏感，但值转换
                // 层要按命令名判断 Array 形态，统一大小写免掉逐处 eq_ignore）
                let name = raw_name.to_uppercase();
                let mut cmd = redis::cmd(&name);
                for arg in args.iter().skip(1) {
                    cmd.arg(arg);
                }
                let value = tokio::select! {
                    _ = cancel_notify.notified() => {
                        return Ok(ExecSummary {
                            cancelled: true,
                            ..Default::default()
                        });
                    }
                    r = redis_run_cmd::<redis::Value>(conn, &mut cmd) => {
                        r.map_err(|e| format!("Redis：{e}"))?
                    }
                };
                // 手敲 SELECT n 也是切库：同步引擎跟踪的 db，否则后续对象树
                // 浏览会「切回」旧库，与实际连接状态不符（命令已成功执行，
                // 只在成功后更新）
                if name == "SELECT"
                    && let Some(target) = args.get(1).and_then(|v| parse_redis_db(v))
                {
                    *db = target;
                }
                // 命令名决定 Array 的解释方式（HGETALL 是 field/value 交替、
                // CONFIG GET 也是；其余 Array 是索引/值列表）
                let (columns, rows) = redis_value_to_table(&value, &name);
                on_columns(&columns);
                let mut summary = ExecSummary {
                    columns,
                    ..Default::default()
                };
                let mut batch: Vec<Vec<String>> = Vec::with_capacity(ROWS_PER_BATCH);
                for row in rows {
                    if summary.rows_read >= max_rows {
                        summary.truncated = true;
                        break;
                    }
                    batch.push(row);
                    summary.rows_read += 1;
                    if batch.len() >= ROWS_PER_BATCH {
                        on_rows(std::mem::take(&mut batch));
                    }
                }
                if !batch.is_empty() {
                    on_rows(batch);
                }
                // affected 恒 None：Redis 没有「影响行数」概念，写命令的结果
                // 本身（OK / 整数 / nil）已经作为一行结果返回
                Ok(summary)
            }
            DbEngine::Sqlite(_) => {
                // 同步引擎：整个执行（含行流迭代与回调发射）移入 spawn_blocking，
                // 回调直接在阻塞线程内调用。取消是协作式的——会话层的
                // db_query_cancel 置 AtomicBool 后，progress handler（每 1000 条
                // VDBE 指令轮询一次）让当前语句以 SQLITE_INTERRUPT 中止；本臂把
                // 该错误分类为 cancelled=true 的正常摘要，而不是执行错误。
                let sql_o = sql.to_string();
                let result = self
                    .with_sqlite(move |conn| {
                        // 每个可能被中断的调用点都过这一层：中断 → 取消摘要（保留
                        // 已读行数与列名），其它 → 带前缀的错误。判据是
                        // sqlite_is_interrupt 的错误码，不靠错误文案匹配。
                        let mut summary = ExecSummary::default();
                        macro_rules! step {
                            ($e:expr) => {
                                match $e {
                                    Ok(v) => v,
                                    Err(e) if sqlite_is_interrupt(&e) => {
                                        summary.cancelled = true;
                                        return Ok(summary);
                                    }
                                    Err(e) => return Err(sqlite_err(e)),
                                }
                            };
                        }
                        let mut stmt = step!(conn.prepare(&sql_o));
                        let col_count = stmt.column_count();
                        if col_count == 0 {
                            // DML / DDL / PRAGMA 赋值：无结果集，affected = changes
                            let affected = step!(stmt.execute([]));
                            return Ok(ExecSummary {
                                affected: Some(affected as u64),
                                ..Default::default()
                            });
                        }
                        summary.columns =
                            stmt.column_names().iter().map(|s| s.to_string()).collect();
                        on_columns(&summary.columns);
                        let mut rows = step!(stmt.query([]));
                        let mut batch: Vec<Vec<String>> = Vec::with_capacity(ROWS_PER_BATCH);
                        while summary.rows_read < max_rows {
                            let next = step!(rows.next());
                            let Some(row) = next else { break };
                            let cells: Vec<String> = (0..summary.columns.len())
                                .map(|i| sqlite_cell(row, i))
                                .collect();
                            batch.push(cells);
                            summary.rows_read += 1;
                            if batch.len() >= ROWS_PER_BATCH {
                                on_rows(std::mem::take(&mut batch));
                            }
                        }
                        if !batch.is_empty() {
                            on_rows(std::mem::take(&mut batch));
                        }
                        if summary.rows_read >= max_rows {
                            // 再探一行判定截断（max_rows 恰好等于总行数时不算截断）
                            if step!(rows.next()).is_some() {
                                summary.truncated = true;
                            }
                        }
                        Ok(summary)
                    })
                    .await;
                // cancel_notify 在 SQLite 臂不参与（取消走 progress handler 的
                // 协作中断，见上）；保留参数以对齐统一签名
                let _ = cancel_notify;
                result
            }
            DbEngine::Dameng(_) => {
                // 同步引擎（同 Oracle 臂）：execute 一次性取全量结果，再在阻塞线程
                // 内按批回调（Channel 发送是 Send 安全的）。取消粒度 = 语句间
                //（会话层 AtomicBool）：dm-pure 阻塞在 socket 读上无协作取消点，
                // 行流中不可中断；max_rows 截断在拿到结果后客户端侧成立
                //（驱动已把整结果集解在内存，大表需用户自加 WHERE/LIMIT）
                let sql_o = sql.to_string();
                let result = self
                    .with_dameng(move |client| {
                        let result = client
                            .execute(&sql_o, &[])
                            .map_err(|e| format!("达梦：{e}"))?;
                        let mut summary = ExecSummary::default();
                        match result {
                            dm_pure::ExecuteResult::Query { columns, rows } => {
                                let names: Vec<String> =
                                    columns.iter().map(|c| c.name.clone()).collect();
                                if names.is_empty() {
                                    // 防御：无列的结果集按 DML 处理（与 Oracle 臂同口径）
                                    return Ok(summary);
                                }
                                on_columns(&names);
                                summary.columns = names;
                                // 全量已在内存，用总行数精确判定截断（不靠多探一行）
                                let total = rows.len();
                                let keep = total.min(max_rows);
                                summary.truncated = total > keep;
                                let mut batch: Vec<Vec<String>> =
                                    Vec::with_capacity(ROWS_PER_BATCH);
                                for row in rows.into_iter().take(keep) {
                                    batch.push(
                                        row.iter().map(dm_value_to_string).collect::<Vec<_>>(),
                                    );
                                    summary.rows_read += 1;
                                    if batch.len() >= ROWS_PER_BATCH {
                                        on_rows(std::mem::take(&mut batch));
                                    }
                                }
                                if !batch.is_empty() {
                                    on_rows(batch);
                                }
                            }
                            dm_pure::ExecuteResult::Affected(n) => {
                                summary.affected = Some(n);
                            }
                            dm_pure::ExecuteResult::Call { affected, out } => {
                                // 工作台是单结果集模型：OUT 参数不单列展示，
                                // 仅当存在输出值时作为一行返回（否则只看 affected）
                                summary.affected = Some(affected);
                                if !out.is_empty() && max_rows > 0 {
                                    let out_header = vec!["OUT".to_string()];
                                    on_columns(&out_header);
                                    summary.columns = out_header;
                                    let cells: Vec<String> =
                                        out.iter().map(dm_value_to_string).collect();
                                    summary.rows_read = 1;
                                    on_rows(vec![cells]);
                                }
                            }
                        }
                        Ok(summary)
                    })
                    .await;
                let _ = cancel_notify;
                result
            }

            DbEngine::Closed => Err("连接已关闭".to_string()),
        }
    }
}

// ── Redis 辅助层 ──────────────────────────────────────────────────────────
//
// Redis 没有 schema/表元信息，对象树与详情全部靠命令探测（SCAN/TYPE/INFO/
// MEMORY USAGE…）。下面这组 helper 把「探测 + 降级」收在一处，各 match 臂
// 只负责组织结果形态。所有 helper 对「命令被禁用/权限不足」都降级而非报错
// ——托管 Redis 常禁用 CONFIG/MEMORY，探测失败不该让整棵树打不开。

/// 对象树里展示的值类型顺序（TYPE 命令的返回词，也是分组名）
pub(crate) const REDIS_VALUE_TYPES: [&str; 6] = ["string", "hash", "list", "set", "zset", "stream"];

/// 扫描 key 上限：工作台是浏览用途，全库枚举在百万 key 实例上会把 UI 拖死
const REDIS_SCAN_KEY_CAP: usize = 5000;

/// 单次 SCAN 的 COUNT 提示值（服务端可返回更多，只是提示）
const REDIS_SCAN_COUNT: usize = 1000;

/// 详情/列清单里单个 key 的元素上限
const REDIS_ELEMENT_CAP: usize = 200;

/// 流详情里 XRANGE 的条数上限（stream 一条消息可能含多个字段）
const REDIS_STREAM_CAP: usize = 100;

/// `db3` / `3` → 3（大小写不敏感的前缀剥离）。空串/不可解析 → None。
#[inline]
pub(crate) fn parse_redis_db(value: &str) -> Option<u32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let digits = trimmed
        .get(..2)
        .filter(|p| p.eq_ignore_ascii_case("db"))
        .map(|_| trimmed.get(2..).unwrap_or_default())
        .unwrap_or(trimmed);
    digits.trim().parse::<u32>().ok()
}

/// key 所属逻辑库：schema 可解析（`db3` / `3`）用之，否则回退会话当前库。
///
/// **为什么不直接 unwrap_or**：schema 解析失败意味着调用方传了非库名的
/// schema（老调用方传裸 key、或树节点 key 形态变更），静默回退当前库会
/// 去错库找 key、最终以「未找到该 key」误报——把「参数不对」伪装成
/// 「key 过期」。这里保留回退（前端老路径兼容），但失败信息经
/// [`redis_key_miss_hint`] 带上实际查找的库名，一眼能看出是库错位。
#[inline]
pub(crate) fn redis_db_of(schema: &str, current_db: u32) -> Result<u32, String> {
    Ok(parse_redis_db(schema).unwrap_or(current_db))
}

/// key 未找到时的诊断增强：附上实际查找的库与传入的 schema 原文。
/// 跨库误用（浏览 db3 的 key 但在 db0 执行）是最常见成因，光说「key 已
/// 过期」会把用户引向错误方向。
pub(crate) fn redis_key_miss_hint(err: String, db: u32, schema: &str) -> String {
    if err.contains("未找到该 key") {
        format!("{err}（在 db{db} 中查找，schema 参数「{schema}」）")
    } else {
        err
    }
}

// ── TLS 构造 ──────────────────────────────────────────────────────────────

/// 按计数生成分组节点（计数为 0 的组不返回）；key 形如 `{schema}#{group}`，
/// 前端展开时从 key 解析 schema 与分组类型。
/// 分组计数载体：多数引擎只有基础五组（触发器/物化视图为 None = 不展示该组），
/// 支持 PG 的物化视图与四引擎的触发器时填 Some。
pub(crate) struct GroupCounts {
    tables: i64,
    views: i64,
    functions: i64,
    procedures: i64,
    sequences: i64,
    /// None = 引擎不支持触发器（如 SQLite 常规认知下）/ 该组不展示
    triggers: Option<i64>,
    /// None = 引擎无物化视图（MySQL / Oracle 普通视图树之外另列）
    mviews: Option<i64>,
    /// None = 引擎无外部表（PG relkind='f'，FDW）
    foreign_tables: Option<i64>,
    /// None = 引擎无扩展（PG pg_extension）
    extensions: Option<i64>,
    /// None = 引擎无 DB Link（Oracle all_db_links）
    db_links: Option<i64>,
    /// None = 引擎无回收站（Oracle/达梦 BIN$ 闪回对象）
    recyclebin: Option<i64>,
    /// None = 引擎无用户自定义类型（Oracle all_objects TYPE）
    datatypes: Option<i64>,
    /// None = 引擎无表空间组（达梦 V$TABLESPACE，挂 SYS schema）
    tablespaces: Option<i64>,
    /// None = 引擎无用户组（达梦实例级，挂 SYS schema）
    dm_users: Option<i64>,
    /// None = 引擎无角色组（达梦实例级，挂 SYS schema）
    dm_roles: Option<i64>,
    /// None = 引擎无同义词（Oracle 系特有）
    synonyms: Option<i64>,
    /// None = 引擎无包（Oracle 系特有）
    packages: Option<i64>,
    /// None = 引擎无事件（MySQL Event Scheduler）
    events: Option<i64>,
    /// None = 引擎无规则（PG pg_rewrite）
    rules: Option<i64>,
}

impl GroupCounts {
    /// 基础五组（触发器与物化视图不展示）——绝大多数引擎的起步形态。
    /// `#[inline]`：结构体字面量构造，七引擎各自调用一次，跨 crate 内联省调用开销
    #[inline]
    pub(crate) fn base(tables: i64, views: i64, functions: i64, procedures: i64, sequences: i64) -> Self {
        Self {
            tables,
            views,
            functions,
            procedures,
            sequences,
            triggers: None,
            mviews: None,
            foreign_tables: None,
            extensions: None,
            db_links: None,
            recyclebin: None,
            datatypes: None,
            tablespaces: None,
            dm_users: None,
            dm_roles: None,
            synonyms: None,
            packages: None,
            events: None,
            rules: None,
        }
    }
}

pub(crate) fn group_nodes(schema: &str, counts: GroupCounts) -> Vec<CatalogNode> {
    // 基础五组按各引擎恒有；可选组（触发器/物化视图/同义词/包/事件/规则）
    // 逐个 Some 即拼入——组合爆炸不再用 match 四臂展开，改为动态拼装
    let mut groups: Vec<(ObjectGroup, &str, i64)> = vec![
        (ObjectGroup::Table, "TABLE", counts.tables),
        (ObjectGroup::View, "VIEW", counts.views),
    ];
    if let Some(m) = counts.mviews {
        groups.push((ObjectGroup::MaterializedView, "MATERIALIZED VIEW", m));
    }
    if let Some(f) = counts.foreign_tables {
        groups.push((ObjectGroup::ForeignTable, "FOREIGN TABLE", f));
    }
    groups.push((ObjectGroup::Function, "FUNCTION", counts.functions));
    groups.push((ObjectGroup::Procedure, "PROCEDURE", counts.procedures));
    if let Some(t) = counts.triggers {
        groups.push((ObjectGroup::Trigger, "TRIGGER", t));
    }
    groups.push((ObjectGroup::Sequence, "SEQUENCE", counts.sequences));
    if let Some(s) = counts.synonyms {
        groups.push((ObjectGroup::Synonym, "SYNONYM", s));
    }
    if let Some(p) = counts.packages {
        groups.push((ObjectGroup::Package, "PACKAGE", p));
    }
    if let Some(e) = counts.events {
        groups.push((ObjectGroup::Event, "EVENT", e));
    }
    if let Some(r) = counts.rules {
        groups.push((ObjectGroup::Rule, "RULE", r));
    }
    if let Some(x) = counts.extensions {
        groups.push((ObjectGroup::Extension, "EXTENSION", x));
    }
    if let Some(d) = counts.db_links {
        groups.push((ObjectGroup::DbLink, "DB LINK", d));
    }
    if let Some(r) = counts.recyclebin {
        groups.push((ObjectGroup::RecycleBin, "RECYCLE BIN", r));
    }
    if let Some(d) = counts.datatypes {
        groups.push((ObjectGroup::DataType, "TYPE", d));
    }
    if let Some(t) = counts.tablespaces {
        groups.push((ObjectGroup::Tablespace, "TABLESPACE", t));
    }
    if let Some(u) = counts.dm_users {
        groups.push((ObjectGroup::DmUser, "USER", u));
    }
    if let Some(r) = counts.dm_roles {
        groups.push((ObjectGroup::DmRole, "ROLE", r));
    }
    groups
        .into_iter()
        .filter(|(_, _, count)| *count > 0)
        .map(|(group, name, count)| CatalogNode {
            key: format!("{schema}#{}", group.as_str()),
            name: name.to_string(),
            kind: "group".to_string(),
            detail: Some(count.to_string()),
        })
        .collect()
}

/// OB Oracle 字典前缀：恒 ALL_*（OB 无 DBA_ 视图的独立权限体系，Oracle 的
/// oracle_dict_prefix 探测式不适用）
pub(crate) const OB_DICT_PREFIX: &str = "all";

/// OB MySQL 租户下隐藏的 schema（ODC `ESCAPE_SCHEMA_SET` 同源）：这四个是
/// OB 内建的 Oracle 兼容对象库，MySQL 模式下对用户无意义。精确大小写匹配
/// （与 ODC 的 Set.contains 一致），避免误伤用户同名的自有 schema。
const OB_HIDDEN_SCHEMAS: &[&str] = &["PUBLIC", "LBACSYS", "ORAAUDITOR", "__public"];

/// OB 物化视图的内部对象前缀（ODC `OdcConstants` 同源）：容器表
/// `__MV_CONTAINER_*` 与日志表 `MLOG$_*` 是 MView 的存储实现，不该出现在
/// 用户的对象树里。
const OB_MV_CONTAINER_PREFIX: &str = "__MV_CONTAINER_";
const OB_MV_LOG_PREFIX: &str = "MLOG$_";

/// 是否 OB MySQL 租户内建、需对用户隐藏的 schema
#[inline(always)]
fn ob_hidden_schema(name: &str) -> bool {
    OB_HIDDEN_SCHEMAS.contains(&name)
}

/// 是否 OB 物化视图的内部对象（容器表 / 日志表），不进对象树
#[inline]
fn ob_internal_object(name: &str) -> bool {
    name.starts_with(OB_MV_CONTAINER_PREFIX) || name.starts_with(OB_MV_LOG_PREFIX)
}

/// OB MySQL 租户的序列清单（OB 专有语法 `SHOW SEQUENCES IN <schema>`，
/// ODC `OBMySQLSchemaAccessor.listSequences` 同源）。
///
/// 返回 `None` 表示「不是 OB MySQL 租户」或该语法不可用——调用方回落
/// `information_schema.sequences`（MariaDB 有、MySQL 8 无），原生 MySQL
/// 行为不变。
pub(crate) async fn ob_mysql_sequences(
    conn: &mut mysql_async::Conn,
    schema: &str,
) -> Option<Vec<String>> {
    if conn.ob_compat_mode() != Some(mysql_async::ObCompatMode::MySql) {
        return None;
    }
    use mysql_async::prelude::Queryable as _;
    let s = schema.replace('`', "``");
    conn.query::<String, _>(format!("SHOW SEQUENCES IN `{s}`"))
        .await
        .ok()
}

/// OB Oracle 表清单（`ALL_TABLES`，ODC `OBOracleSchemaAccessor.listTables`
/// 同源）：用表字典而不是 `all_objects`，视图与回收站对象天然不在内。
///
/// `EXTERNAL` 是 OB 4.3.2+ 才有的扩展列，老版本查它会 ORA-00904——首次失败
/// 即降级为不带该条件的同一查询（外部表会混进表分组，与老版本 ODC 一致）。
pub(crate) async fn ob_oracle_tables(
    conn: &mut mysql_async::Conn,
    schema: &str,
) -> Result<Vec<String>, String> {
    use mysql_async::prelude::Queryable as _;
    let lit = ob_quote_literal(schema);
    let sql = format!(
        "SELECT table_name FROM {OB_DICT_PREFIX}_tables \
         WHERE owner = {lit} AND EXTERNAL = 'NO' ORDER BY table_name"
    );
    match conn.query::<String, _>(sql).await {
        Ok(names) => Ok(names),
        Err(_) => {
            let sql = format!(
                "SELECT table_name FROM {OB_DICT_PREFIX}_tables \
                 WHERE owner = {lit} ORDER BY table_name"
            );
            conn.query::<String, _>(sql)
                .await
                .map_err(|e| format!("OceanBase 查询表清单失败：{e}"))
        }
    }
}

/// OB Oracle 包清单：`PACKAGE` 与 `PACKAGE BODY` 合并按名去重（ODC
/// `listPackages` 的状态合并语义；只有包体的包也必须列出来）。
pub(crate) async fn ob_oracle_packages(
    conn: &mut mysql_async::Conn,
    schema: &str,
) -> Result<Vec<String>, String> {
    use mysql_async::prelude::Queryable as _;
    let sql = format!(
        "SELECT DISTINCT object_name FROM {OB_DICT_PREFIX}_objects \
         WHERE owner = {} AND object_name NOT LIKE 'BIN$%' \
           AND object_type IN ('PACKAGE', 'PACKAGE BODY') ORDER BY object_name",
        ob_quote_literal(schema)
    );
    conn.query::<String, _>(sql)
        .await
        .map_err(|e| format!("OceanBase 查询包清单失败：{e}"))
}

/// OB Oracle 物化视图清单（`ALL_MVIEWS`，ODC `OBOracleSchemaAccessor.listMViews`
/// 同源）。OB Oracle 的物化视图同时有同名容器表，`ALL_TABLES` 也会列出——
/// ODC 未做排除，此处保持一致。
///
/// 字典缺失（老版本无 ALL_MVIEWS）按空集处理：对象树宁可少一个分组，也不该
/// 因为一个可选字典整棵树打不开。
pub(crate) async fn ob_oracle_mviews(
    conn: &mut mysql_async::Conn,
    schema: &str,
) -> Vec<String> {
    use mysql_async::prelude::Queryable as _;
    let sql = format!(
        "SELECT mview_name FROM {OB_DICT_PREFIX}_mviews WHERE owner = {} ORDER BY mview_name",
        ob_quote_literal(schema)
    );
    conn.query::<String, _>(sql).await.unwrap_or_default()
}

/// OB MySQL 物化视图清单（`OCEANBASE.DBA_MVIEWS`，ODC
/// `OBMySQLSchemaAccessor.listMViews` 同源）。
///
/// 该视图 OB 4.3.5.2 起才有；更早版本查询报错 → 返回 `None`，分组不出现
/// （与 ODC 在低版本显式抛 Unsupported 的可见结果一致）。
pub(crate) async fn ob_mysql_mviews(
    conn: &mut mysql_async::Conn,
    schema: &str,
) -> Option<Vec<String>> {
    use mysql_async::prelude::Queryable as _;
    let sql = format!(
        "SELECT mview_name FROM OCEANBASE.DBA_MVIEWS WHERE OWNER = {} ORDER BY mview_name",
        ob_quote_literal(schema)
    );
    conn.query::<String, _>(sql).await.ok()
}

/// OB MySQL 租户：隐藏内建 Oracle 兼容 schema（原生 MySQL 原样返回）。
/// 非 OB 连接 `ob_compat_mode()` 为 None，此函数是恒等变换。
fn ob_filter_schemas(conn: &mysql_async::Conn, names: Vec<String>) -> Vec<String> {
    if conn.ob_compat_mode() == Some(mysql_async::ObCompatMode::MySql) {
        names.into_iter().filter(|n| !ob_hidden_schema(n)).collect()
    } else {
        names
    }
}

/// OB Oracle 系统账号黑名单（与 oracle_usernames 的 <12 分支同源；OB 的
/// all_users 无 12c 的 oracle_maintained 列，统一走黑名单）
const OB_SYS_USERS: &str = "'SYS','SYSTEM','OUTLN','DIP','ORACLE_OCM','DBSNMP',\
     'APPQOSSYS','WMSYS','EXFSYS','CTXSYS','XDB','ANONYMOUS','XS$NULL','MDDATA',\
     'MDSYS','OLAPSYS','ORDDATA','ORDPLUGINS','ORDSYS','SI_INFORMTN_SCHEMA',\
     'LBACSYS','OWBSYS','OWBSYS_AUDIT','SPATIAL_CSW_ADMIN','SPATIAL_WFS_ADMIN',\
     'SYSMAN','MGMT_VIEW','FLOWS_FILES'";

/// OB Oracle 可见 schema 清单（all_users + 系统账号黑名单；连接为 mysql_async
/// 的文本协议，列值按 UTF-8 lossy 转串）
async fn ob_oracle_usernames(conn: &mut mysql_async::Conn) -> Result<Vec<String>, String> {
    use mysql_async::prelude::Queryable as _;
    let sql = format!(
        "SELECT username FROM all_users WHERE username NOT IN ({OB_SYS_USERS}) \
         AND username NOT LIKE 'APEX\\_%' ESCAPE '\\' \
         AND username NOT LIKE 'FLOWS\\_%' ESCAPE '\\' \
         ORDER BY username"
    );
    let names: Vec<String> = conn
        .exec(sql, ())
        .await
        .map_err(|e| format!("OceanBase 查询用户清单失败：{e}"))?;
    Ok(names)
}

/// DBA 字典视图自适应（DBeaver OracleUtils.getAdminAllViewPrefix 同义的
/// 自动探测形态）：探测 dba_objects 可读（SELECT ANY DICTIONARY /
/// SELECT_CATALOG_ROLE）即切 DBA_*（实例全量可见），否则 ALL_*（仅当前
/// 用户可访问对象）。每次目录操作探测一回（一条 ROWNUM=1 的轻量探测）；
/// 失败按无权限回 ALL_*（普通用户查 dba_objects 直接 ORA-00942）
pub(crate) async fn oracle_dict_prefix(conn: &mut oracle_async::Conn) -> &'static str {
    match conn
        .query("SELECT 1 FROM dba_objects WHERE ROWNUM = 1")
        .await
    {
        Ok(mut rs) => match rs.next_row().await {
            Ok(Some(_)) => "dba",
            _ => "all",
        },
        Err(_) => "all",
    }
}

/// Oracle 容错计数：查询失败（权限/视图缺失）返回 None
pub(crate) async fn oracle_count_opt(conn: &mut oracle_async::Conn, sql: &str) -> Option<i64> {
    let mut rs = conn.query(sql).await.ok()?;
    let row = rs.next_row().await.ok()??;
    oracle_row_string(&row, 0).parse::<i64>().ok()
}

/// Oracle 标量字符串（容错：失败/空结果返回空串）
pub(crate) async fn oracle_scalar_string(conn: &mut oracle_async::Conn, sql: &str) -> String {
    match conn.query(sql).await {
        Ok(mut rs) => match rs.next_row().await {
            Ok(Some(row)) => oracle_row_string(&row, 0),
            _ => String::new(),
        },
        Err(_) => String::new(),
    }
}

/// 回收站行 → CatalogNode（OBJECT_NAME/ORIGINAL_NAME/TYPE 三列形态；
/// key 用 BIN$ 实名防同名多版本冲突，显示名用 ORIGINAL_NAME）。
/// `#[inline]`：回收站逐行调用，函数体只有三次列取值 + 一次结构构造
#[inline]
pub(crate) fn recycle_node(row: &oracle_async::Row, schema: &str, kind: &str) -> Option<CatalogNode> {
    let obj = oracle_row_string(row, 0);
    if obj == "NULL" {
        return None;
    }
    let original = oracle_row_string(row, 1);
    let otype = oracle_row_string(row, 2);
    Some(CatalogNode {
        key: format!("{schema}.{obj}"),
        name: if original == "NULL" { obj } else { original },
        kind: kind.to_string(),
        detail: if otype == "NULL" { None } else { Some(otype) },
    })
}

/// 索引聚列的表达式回填：主查询 GROUP_CONCAT/LISTAGG 按序号聚出的列串，
/// 用（索引名, 序号）→ 表达式 的 overlay 按位置替换（表达式/函数索引部件的
/// 列名是系统占位名，真实定义在 overlay）；序号从 1 起与部件序一致。
/// `#[inline]`：MySQL/Oracle 两臂每次表详情各调一次，空 overlay 时是纯
/// 早退（最常见的路径），内联后连函数调用都省掉
#[inline]
pub(crate) fn apply_index_expr_overlay(
    cols: &str,
    index_name: &str,
    overlay: &std::collections::HashMap<(String, i64), String>,
) -> String {
    if overlay.is_empty() || cols.is_empty() {
        return cols.to_string();
    }
    cols.split(", ")
        .enumerate()
        .map(|(i, part)| {
            overlay
                .get(&(index_name.to_string(), i as i64 + 1))
                .map(String::as_str)
                .unwrap_or(part)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// 字节数 → 人类可读（B / KB / MB / GB / TB，一位小数）。表大小展示用。
#[inline]
pub(crate) fn format_float(v: f64) -> String {
    if v.fract() == 0.0 && v.is_finite() && v.abs() < 1e15 {
        // 整数值去掉 .0（与 sqlite_viewer 一致）
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

#[inline]
pub(crate) fn format_bytes(bytes: i64) -> String {
    if bytes < 0 {
        return String::new();
    }
    let mut val = bytes as f64;
    for unit in ["KB", "MB", "GB", "TB"] {
        if val < 1024.0 {
            break;
        }
        val /= 1024.0;
        if unit == "TB" || val < 1024.0 {
            return format!("{val:.1} {unit}");
        }
    }
    format!("{bytes} B")
}


#[cfg(test)]
mod tests {
    use super::*;

    /// OB MySQL 租户的过滤规则（纯函数，无服务端可测的部分）：
    /// - 内建 Oracle 兼容 schema 精确匹配隐藏（大小写敏感，不误伤用户 schema）
    /// - 物化视图的容器表/日志表按前缀排除，普通表不受影响
    #[test]
    fn ob_mysql_filters() {
        for name in ["PUBLIC", "LBACSYS", "ORAAUDITOR", "__public"] {
            assert!(ob_hidden_schema(name), "{name} 应被隐藏");
        }
        for name in ["public", "Public", "APP", "SYS", "mysql", "information_schema"] {
            assert!(!ob_hidden_schema(name), "{name} 不应被隐藏");
        }

        for name in [
            "__MV_CONTAINER_mv1",
            "MLOG$_T1",
            "__MV_CONTAINER_",
            "MLOG$_",
        ] {
            assert!(ob_internal_object(name), "{name} 属于 MV 内部对象");
        }
        for name in ["T1", "my_mlog", "MLOG_T1", "mv_container_x"] {
            assert!(!ob_internal_object(name), "{name} 是普通表");
        }
    }

    /// 系统根证书 → PEM：每张证书必须各自一层 BEGIN/END。
    /// 曾经把所有证书的 base64 塞进同一个块——多证书时中间会出现 `=` 填充，
    /// 或解出「多张 DER 首尾相接」的字节，两种都让 PEM/证书解析失败，
    /// MySQL 的 verify-full 因此稳定连不上（系统根证书必然多于一张）。
    #[test]
    fn native_roots_pem_one_block_per_cert() {
        let native = rustls_native_certs::load_native_certs();
        if native.certs.is_empty() {
            return; // 无系统根证书的环境（精简容器）跳过
        }
        let pem = connect::native_roots_pem().expect("应能生成 PEM");
        let begins = pem.matches("-----BEGIN CERTIFICATE-----").count();
        let ends = pem.matches("-----END CERTIFICATE-----").count();
        assert_eq!(begins, native.certs.len(), "每张证书应各自一层 BEGIN");
        assert_eq!(begins, ends, "BEGIN/END 必须成对");
    }

    /// Redis extra_options 解析：合法 camelCase JSON 取值、非法/空 JSON 按
    /// 单机默认（老记录兼容语义），拓扑标签映射三种形态
    #[test]
    fn redis_extra_options_parse_and_topology() {
        let extra = redis_extra_options(Some(
            r#"{"topology":"cluster","nodes":["10.0.0.1:6379","[::1]:7002"],"masterName":"mymaster","clientCertPath":"/a.pem","clientKeyPath":"/a.key"}"#,
        ));
        assert_eq!(extra.topology, "cluster");
        assert_eq!(extra.nodes.len(), 2);
        assert_eq!(extra.master_name.as_deref(), Some("mymaster"));
        assert_eq!(extra.client_cert_path.as_deref(), Some("/a.pem"));

        // 非法 JSON / 空 → 单机默认；单独 sentinel 拓扑映射标签
        assert_eq!(redis_extra_options(Some("{oops")).topology, "");
        assert_eq!(redis_extra_options(None).nodes.len(), 0);
        assert_eq!(redis_topology_of(Some(r#"{"topology":"sentinel"}"#)), Some("sentinel"));
        assert_eq!(redis_topology_of(Some(r#"{"topology":"cluster"}"#)), Some("cluster"));
        assert_eq!(redis_topology_of(None), None);
    }

    /// 节点清单解析：host:port / [IPv6]:port / 空行忽略；缺端口、缺主机、
    /// 整表为空直接报错
    #[test]
    fn redis_node_list_shapes() {
        let mk = |nodes: &[&str]| RedisExtraOptions {
            nodes: nodes.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        let list = redis_node_list(&mk(&[" h1:6379 ", "", "[::1]:7002"])).unwrap();
        assert_eq!(list, vec![("h1".to_string(), 6379), ("::1".to_string(), 7002)]);
        assert!(redis_node_list(&mk(&["noport"])).is_err());
        assert!(redis_node_list(&mk(&[":6379"])).is_err());
        assert!(redis_node_list(&mk(&["h:xx"])).is_err());
        assert!(redis_node_list(&mk(&[])).is_err());
    }

    /// 键覆写载荷的 serde 判别形态（前端契约：tag=kind、五种小写形态、
    /// hash/zset 的 entries 字段名）
    #[test]
    fn redis_key_write_payload_shapes() {
        let hash: RedisKeyWrite =
            serde_json::from_str(r#"{"kind":"hash","entries":[{"field":"f","value":"v"}]}"#)
                .unwrap();
        assert!(matches!(hash, RedisKeyWrite::Hash { ref entries } if entries.len() == 1));

        let zset: RedisKeyWrite =
            serde_json::from_str(r#"{"kind":"zset","entries":[{"member":"m","score":"1.5"}]}"#)
                .unwrap();
        assert!(matches!(zset, RedisKeyWrite::ZSet { .. }));

        let string: RedisKeyWrite = serde_json::from_str(r#"{"kind":"string","value":"x"}"#).unwrap();
        assert!(matches!(string, RedisKeyWrite::String { .. }));
        // 未知形态必须报错（防前端拼写偏差静默落错臂）
        assert!(serde_json::from_str::<RedisKeyWrite>(r#"{"kind":"stream"}"#).is_err());
    }

    /// INFO 文本与 SCAN 回复的表格化：field/value 拆行（分节行、CRLF）、
    /// cursor/key 两列（游标只在首行、空页保留游标行）
    #[test]
    fn redis_info_and_scan_tables() {
        let (headers, rows) = redis_info_table("# Server\r\nredis_version:7.2\r\n\r\nuptime:1");
        assert_eq!(headers, vec!["field", "value"]);
        assert_eq!(rows.first().expect("section row"), &vec!["◆ Server", ""]);        assert_eq!(rows.get(1).expect("version row"), &vec!["redis_version", "7.2"]);
        assert_eq!(rows.get(2).expect("uptime row"), &vec!["uptime", "1"]);

        let scan = vec![
            redis::Value::BulkString(b"17".to_vec()),
            redis::Value::Array(vec![
                redis::Value::BulkString(b"k1".to_vec()),
                redis::Value::BulkString(b"k2".to_vec()),
            ]),
        ];
        let (headers, rows) = redis_scan_table(&scan);
        assert_eq!(headers, vec!["cursor", "key"]);
        assert_eq!(rows.first().expect("first row"), &vec!["17", "k1"]);        assert_eq!(rows.get(1).expect("second row"), &vec!["", "k2"]);

        let empty = vec![
            redis::Value::BulkString(b"0".to_vec()),
            redis::Value::Array(vec![]),
        ];
        let (_, rows) = redis_scan_table(&empty);
        assert_eq!(rows, vec![vec!["0".to_string(), "(本页无 key)".to_string()]]);
    }

    /// 常见 ORA 码表边界：收录码有文案、未收录码回落 None（连库行为
    /// 无法在无客户端环境测试，这里只锁纯函数映射）
    #[test]
    fn oracle_ora_hint_known_and_unknown_codes() {
        assert_eq!(oracle_ora_hint(904), Some("标识符无效（列名/别名拼写错误，或该列在当前版本不存在）"));
        assert_eq!(oracle_ora_hint(923), Some("未找到 FROM 关键字（常见：SELECT 列表缺逗号）"));
        assert_eq!(oracle_ora_hint(942), Some("表或视图不存在（检查 schema 前缀与权限）"));
        assert_eq!(oracle_ora_hint(1017), Some("用户名或密码无效"));
        assert_eq!(oracle_ora_hint(28000), Some("账户已被锁定"));
        // 新增覆盖：游标/PLSQL/通信
        assert_eq!(oracle_ora_hint(1002), Some("取数越界（fetch out of sequence，游标状态已失效）"));
        assert_eq!(oracle_ora_hint(6510), Some("PL/SQL 编译错误（检查过程和函数体）"));
        assert_eq!(oracle_ora_hint(3135), Some("连接丢失（connection lost contact）"));
        assert_eq!(oracle_ora_hint(99999), None);
        assert_eq!(oracle_ora_hint(0), None);
    }

    /// OCI「缺消息文件」文案清理：go-ora 原样透传服务器文案（如
    /// `ORA-00904: Message 904 not found; No message file for product=RDBMS,
    /// facility=ORA`），本函数剥掉英文尾巴、换内置中文并保留 arguments 段
    #[test]
    fn oracle_err_strips_missing_message_file_noise() {
        use oracle_async::OracleError;
        // 缺消息文件形态（真机服务器返回的原文）
        let e = oracle_async::Error::Server(OracleError {
            code: 904,
            message: "ORA-00904: Message 904 not found; No message file for product=RDBMS, \
                      facility=ORA; arguments: [ORACLE_MAINTAINED]"
                .into(),
        });
        let msg = oracle_err(e);
        assert!(!msg.contains("No message file"), "应剥掉缺消息文件尾巴：{msg}");
        assert!(msg.contains("标识符无效"), "应含内置中文：{msg}");
        assert!(msg.contains("ORACLE_MAINTAINED"), "应保留 arguments 定位信息：{msg}");

        // 正常文案形态：透传并追加提示
        let e = oracle_async::Error::Server(OracleError {
            code: 942,
            message: "ORA-00942: table or view does not exist".into(),
        });
        let msg = oracle_err(e);
        assert!(msg.contains("table or view does not exist"));
        assert!(msg.contains("表或视图不存在"));

        // 未收录码 + 缺消息文件：回落官方文档提示，不出现英文噪音
        let e = oracle_async::Error::Server(OracleError {
            code: 20001,
            message: "ORA-20001: Message 20001 not found; No message file".into(),
        });
        let msg = oracle_err(e);
        assert!(!msg.contains("No message file"), "{msg}");
        assert!(msg.contains("ORA-20001"), "{msg}");
    }

    /// 错误文案接入全量表后的行为：优先用 19c 全量译文（含 cause/action），
    /// 未收录回落 80 条精简表
    #[test]
    fn oracle_error_text_prefers_full_table_then_falls_back() {
        use crate::db_browser::engines::oracle_engine::oracle_error_text;

        // 全量表命中：三段齐全（ORA-00942 在 19c 手册里有完整 cause/action）
        let t = oracle_error_text(942).expect("ORA-00942 应有解释");
        assert_eq!(t.message, "表或视图不存在");
        assert!(!t.cause.is_empty(), "全量表应带 cause");
        assert!(!t.action.is_empty(), "全量表应带 action");

        // 精简表也收录的码：仍能查到且非空
        let t = oracle_error_text(1002).expect("ORA-01002 应有解释");
        assert!(!t.message.is_empty());

        // 两个表都未收录
        assert!(oracle_error_text(99999).is_none());

        // 负数/超 u16 范围码不进全量表（转换失败），只可能走精简表
        assert!(oracle_error_text(-1).is_none());
        assert!(oracle_error_text(70000).is_none());
    }

    /// 三行结构化文案：第一行 `ORA-xxxxx：解释`，第二行 `原因：`，第三行
    /// `处理建议：`；精简表（无 cause/action）退化为单行
    #[test]
    fn oracle_err_formats_three_lines() {
        use oracle_async::OracleError;

        // 缺消息文件形态（服务器没装 .msb）：完整三行
        let e = oracle_async::Error::Server(OracleError {
            code: 942,
            message: "ORA-00942: Message 942 not found; No message file for product=RDBMS, \
                      facility=ORA; arguments: [EMP]"
                .into(),
        });
        let msg = oracle_err(e);
        let lines: Vec<&str> = msg.lines().collect();
        assert!(lines.len() >= 3, "应为三行以上，实际 {} 行：{msg}", lines.len());
        let (first, second, third) = match (lines.first(), lines.get(1), lines.get(2)) {
            (Some(a), Some(b), Some(c)) => (*a, *b, *c),
            _ => unreachable!("len >= 3 已断言"),
        };
        assert!(
            first.starts_with("ORA-00942："),
            "首行应为代码+解释：{first}"
        );
        assert!(first.contains("表或视图不存在"), "首行应含中文解释：{first}");
        assert!(second.starts_with("原因："), "第二行应为原因：{second}");
        assert!(
            third.starts_with("处理建议："),
            "第三行应为处理建议：{third}"
        );
        // arguments 段保留（缺消息文件时唯一的定位信息）
        assert!(msg.contains("EMP"), "应保留 arguments 定位信息：{msg}");
        // 不应残留英文噪音
        assert!(!msg.contains("No message file"), "应剥掉缺消息文件尾巴：{msg}");
    }

    /// 达梦值字符串化/数值归一/标识符校验的纯函数边界（连库行为无法在无
    /// 服务端环境测试，这里只锁展示与防注入逻辑）
    #[test]
    fn dameng_value_helpers() {
        use dm_pure::Value as V;
        assert_eq!(dm_value_to_string(&V::Null), "NULL");
        assert_eq!(dm_value_to_string(&V::BigInt(42)), "42");
        assert_eq!(
            dm_value_to_string(&V::Decimal("123.4500".into())),
            "123.4500"
        );
        assert_eq!(
            dm_value_to_string(&V::Bytes(vec![1, 2, 3])),
            "<BLOB 3 bytes>"
        );
        assert_eq!(dm_value_to_string(&V::Double(2.0)), "2");
        assert_eq!(dm_value_to_i64(&V::Decimal("7".into())), 7);
        assert_eq!(dm_value_to_i64(&V::Decimal("7.0".into())), 7);
        assert_eq!(dm_value_to_i64(&V::Int(5)), 5);
        assert_eq!(dm_value_to_i64(&V::Str("x".into())), 0);
        assert_eq!(dm_value_to_i64(&V::Null), 0);
        // 文本列：CLOB 走 Str，BLOB 形态按 UTF-8 容错解码，NULL 归空串
        let text_row = vec![V::Bytes("CREATE TABLE t".as_bytes().to_vec())];
        assert_eq!(dm_text_at(&text_row, 0), "CREATE TABLE t");
        assert_eq!(dm_text_at(&[V::Null], 0), "");
        assert_eq!(dm_text_at(&[], 0), "");
        assert!(dm_valid_identifier("SYSDBA"));
        assert!(dm_valid_identifier("A_1$#"));
        assert!(!dm_valid_identifier(""));
        assert!(!dm_valid_identifier("a b"));
        assert!(!dm_valid_identifier("a;b"));
    }
}
