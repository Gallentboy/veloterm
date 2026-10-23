//! 引擎层：MySQL（mysql_async）/ PostgreSQL（tokio-postgres）/ Oracle
//!（vendor rust-oracle，官方 ODPI-C 绑定、同步 API，经 with_oracle 包裹）
//! / Redis（redis-rs 多路复用异步连接）的 enum 分发。不用 dyn trait ——
//! async fn in trait 非 dyn-compatible，多变体 enum 是最轻的分发形态。
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
//! Oracle 侧说明：rust-oracle 同步无 async，with_oracle 以 spawn_blocking
//! 包裹每次调用（连接取出/归还）；运行期需 Oracle Instant Client（11.2–
//! 23ai 全版本支持，编译期仅 cc 编 dpi.c 无需 SDK）。TLS 走完整描述符的
//! TCPS 协议（证书校验粒度由 Instant Client 侧配置决定，四档统一开启
//! TCPS、不回退明文）。
//!
//! rustls provider 全局唯一（aws-lc-rs，见 Cargo.toml 约束），MySQL 侧用
//! mysql_async 的 default-rustls feature 同源。

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use futures_util::StreamExt as _;
use serde::Serialize;

use crate::db::DbConnectionRecord;

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

/// 表/视图详情：DDL + 列结构 + 索引清单（树右键「表详情」）。表头用英文通用词
///（后端不做 i18n，前端 DataTable 直接展示）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDetail {
    pub ddl: String,
    pub column_headers: Vec<String>,
    pub columns: Vec<Vec<String>>,
    pub index_headers: Vec<String>,
    pub indexes: Vec<Vec<String>>,
}

/// MySQL 列结构行：name / column_type / is_nullable / default / comment
type MySqlColumnRow = (String, String, String, Option<String>, Option<String>);

/// 引擎实例：一会话一连接（单查询工作台模型，不做池）。Closed 为优雅关闭后的占位。
/// Oracle 为同步驱动（rust-oracle 无 async），持有连接经 with_oracle 包裹调用。
/// Redis 用 MultiplexedConnection（内部单任务串行化请求/响应，clone 廉价）——
/// 与「单连接工作台」模型天然契合，不需要额外池。
pub enum DbEngine {
    MySql(mysql_async::Conn),
    Postgres(tokio_postgres::Client),
    Oracle(oracle::Connection),
    Redis {
        conn: redis::aio::MultiplexedConnection,
        /// 当前逻辑库索引（SELECT n 的目标；元信息展示为 "db{n}"）
        db: u32,
    },
    Closed,
}

impl DbEngine {
    /// 建连并取服务端版本描述。
    pub async fn connect(record: &DbConnectionRecord) -> Result<(DbEngine, String), String> {
        match record.engine.as_str() {
            "mysql" => Self::connect_mysql(record).await,
            "postgres" => Self::connect_postgres(record).await,
            "oracle" => Self::connect_oracle(record).await,
            "redis" => Self::connect_redis(record).await,
            other => Err(format!("不支持的数据库引擎：{other}")),
        }
    }

    /// 优雅关闭（MySQL 发 COM_QUIT；PG 直接断；Redis 的 multiplexed 连接
    /// 随 Drop 关闭后台任务，无需发命令）——消费自身，置 Closed 占位。
    pub async fn close(&mut self) {
        let taken = std::mem::replace(self, DbEngine::Closed);
        if let DbEngine::MySql(conn) = taken {
            let _ = conn.disconnect().await;
        }
    }

    async fn connect_mysql(record: &DbConnectionRecord) -> Result<(DbEngine, String), String> {
        let mut builder = mysql_async::OptsBuilder::default()
            .ip_or_hostname(record.host.trim().to_string())
            .tcp_port(record.port);
        if let Some(user) = record.username.as_deref().filter(|s| !s.is_empty()) {
            builder = builder.user(Some(user.to_string()));
        }
        if let Some(pass) = record.password.as_deref().filter(|s| !s.is_empty()) {
            builder = builder.pass(Some(pass.to_string()));
        }
        if let Some(db) = record.database.as_deref().filter(|s| !s.is_empty()) {
            builder = builder.db_name(Some(db.to_string()));
        }
        builder = builder.ssl_opts(mysql_ssl_opts(&record.ssl_mode)?);
        let conn = tokio::time::timeout(Duration::from_secs(12), mysql_async::Conn::new(builder))
            .await
            .map_err(|_| "MySQL 连接超时（12s）".to_string())?
            .map_err(|e| format!("MySQL 连接失败：{e}"))?;
        let conn = conn;
        let (major, minor, patch) = conn.server_version();
        Ok((
            DbEngine::MySql(conn),
            format!("MySQL {major}.{minor}.{patch}"),
        ))
    }

    async fn connect_postgres(record: &DbConnectionRecord) -> Result<(DbEngine, String), String> {
        let mut config = tokio_postgres::Config::new();
        config
            .host(record.host.trim())
            .port(record.port)
            .application_name("VeloTerm");
        if let Some(user) = record.username.as_deref().filter(|s| !s.is_empty()) {
            config.user(user);
        }
        if let Some(pass) = record.password.as_deref().filter(|s| !s.is_empty()) {
            config.password(pass);
        }
        if let Some(db) = record.database.as_deref().filter(|s| !s.is_empty()) {
            config.dbname(db);
        }
        let client = if record.ssl_mode == "disable" {
            let (client, connection) = config
                .connect(tokio_postgres::NoTls)
                .await
                .map_err(|e| format!("PostgreSQL 连接失败：{e}"))?;
            tokio::spawn(spawn_pg_connection(connection));
            client
        } else {
            let connector = pg_tls_connector(record.ssl_mode == "verify-full")?;
            let (client, connection) = config
                .connect(connector)
                .await
                .map_err(|e| format!("PostgreSQL 连接失败：{e}"))?;
            tokio::spawn(spawn_pg_connection(connection));
            client
        };
        let version = client
            .query_one("SELECT version()", &[])
            .await
            .ok()
            .and_then(|row| row.try_get::<_, String>(0).ok())
            .unwrap_or_else(|| "PostgreSQL".to_string());
        Ok((DbEngine::Postgres(client), version))
    }

    async fn connect_oracle(record: &DbConnectionRecord) -> Result<(DbEngine, String), String> {
        // Oracle 双字段模型：service_name = 服务名（连库必需）；database = 初始
        // schema（可选，连上后 ALTER SESSION SET CURRENT_SCHEMA）
        let initial_schema = record.database.clone();
        let service = record
            .service_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "Oracle 需要填写服务名（如 ORCLPDB1）".to_string())?;
        let username = record
            .username
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "Oracle 需要填写用户名".to_string())?;
        let password = record
            .password
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "Oracle 需要填写密码".to_string())?;
        let host = record.host.trim().to_string();
        let conn_string = if record.ssl_mode == "disable" {
            format!("{host}:{}/{}", record.port, service)
        } else {
            format!(
                "((ADDRESS=(PROTOCOL=TCPS)(HOST={host})(PORT={port}))\
                 (CONNECT_DATA=(SERVICE_NAME={service})))",
                port = record.port,
            )
        };
        let username = username.to_string();
        let password = password.to_string();
        let schema_o = initial_schema;
        // Oracle 客户端库目录（用户可设，db_browser 的 oracle_client_dir_set）：
        // ODPI-C 运行期 dlopen——进程级一次初始化，本进程此前已用别的参数
        // 初始化过则本次参数被忽略（rust-oracle 的 GLOBAL_CONTEXT 语义）
        let client_dir = oracle_client_dir().ok().flatten();
        let (conn, version) = tokio::task::spawn_blocking(move || {
            // NLS_LANG 补默认（未设时才补，尊重用户显式配置）：客户端字符集
            // 定死 UTF-8——不设则随系统 locale 漂移，非 UTF-8 locale 下中文
            // 数据乱码；语言 AMERICAN 与官方错误文档口径一致。ODPI-C 在 init
            // 时读该变量，必须设在 init 之前；set_var 是进程级全局操作，
            // edition 2024 要求 unsafe（首个 Oracle 初始化前设置一次，此后
            // 本进程不再改环境变量）
            if std::env::var_os("NLS_LANG").is_none() {
                unsafe { std::env::set_var("NLS_LANG", "AMERICAN_AMERICA.AL32UTF8") };
            }
            let mut init = oracle::InitParams::new();
            if let Some(dir) = client_dir.as_deref().filter(|d| !d.is_empty()) {
                init.oracle_client_lib_dir(dir)
                    .map_err(|e| format!("Oracle 客户端目录参数无效：{e}"))?;
            }
            init.init()
                .map_err(|e| format!("Oracle 客户端初始化失败：{}", oracle_err(e)))?;
            let conn = oracle::Connection::connect(&username, &password, &conn_string)
                .map_err(|e| format!("Oracle 连接失败：{}", oracle_err(e)))?;
            // server_version 返回 (Version, banner)：banner 即「Oracle Database
            // 11g Enterprise Edition Release 11.2.0.4.0 - 64bit Production」形态
            let version = conn
                .server_version()
                .map(|(_, banner)| banner)
                .unwrap_or_default();
            // 初始 schema（database 字段，可选）：CURRENT_SCHEMA 只改名字解析
            // 不改权限；标识符字符集校验同 select_database（防注入）
            if let Some(schema) = schema_o.as_deref().filter(|s| !s.trim().is_empty()) {
                if !schema
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '#'))
                {
                    return Err("Oracle 无效的 schema 名".to_string());
                }
                let sql = format!("ALTER SESSION SET CURRENT_SCHEMA = {schema}");
                conn.execute(sql.as_str(), &[])
                    .map_err(|e| format!("Oracle 设置初始 schema 失败：{}", oracle_err(e)))?;
            }
            Ok::<_, String>((conn, version))
        })
        .await
        .map_err(|e| format!("Oracle 连接任务失败：{e}"))??;
        let version = if version.is_empty() {
            "Oracle".to_string()
        } else {
            version
        };
        Ok((DbEngine::Oracle(conn), version))
    }

    /// Redis 建连。直接构造 ConnectionInfo（不经 URL 解析——密码/用户名里的
    /// `@ : / ? #` 等字符会被 URL 语法吃掉，这是 redis-rs 的 `open("redis://…")`
    /// 路径最常见的踩坑点）。
    async fn connect_redis(record: &DbConnectionRecord) -> Result<(DbEngine, String), String> {
        let host = record.host.trim().to_string();
        // TLS 四档：disable 明文；prefer/require 加密但不校验证书（语义与
        // MySQL/PG 臂一致：绝不回退明文）；verify-full 走系统根证书。
        // redis-rs 的 insecure=true 需要 tls-rustls-insecure feature（Cargo.toml
        // 已开），否则连接时直接报 InvalidClientConfig。
        let addr = if record.ssl_mode == "disable" {
            redis::ConnectionAddr::Tcp(host, record.port)
        } else {
            redis::ConnectionAddr::TcpTls {
                host,
                port: record.port,
                insecure: record.ssl_mode != "verify-full",
                tls_params: None,
            }
        };
        let db = redis_db_index(record.database.as_deref());
        let mut redis_info = redis::RedisConnectionInfo::default().set_db(i64::from(db));
        // 空字符串按「未设置」处理（表单留空 = 无 ACL 用户名/无密码）；
        // 不设 protocol 保持 RESP2——老版本 Redis（6 之前无 HELLO）连不上
        // RESP3，值转换层对两种响应形态都兼容。
        if let Some(user) = record.username.as_deref().filter(|s| !s.is_empty()) {
            redis_info = redis_info.set_username(user);
        }
        if let Some(pass) = record.password.as_deref().filter(|s| !s.is_empty()) {
            redis_info = redis_info.set_password(pass);
        }
        // ConnectionInfo 的字段是私有的：经 IntoConnectionInfo（ConnectionAddr
        // 直接构造 → 默认 tcp_settings）再整体替换 redis 段，避免拼 URL。
        let info = redis::IntoConnectionInfo::into_connection_info(addr)
            .map_err(|e| redis_connect_error(&e.to_string()))?
            .set_redis_settings(redis_info);
        let client =
            redis::Client::open(info).map_err(|e| redis_connect_error(&e.to_string()))?;
        let mut conn = tokio::time::timeout(
            Duration::from_secs(12),
            client.get_multiplexed_async_connection(),
        )
        .await
        .map_err(|_| "Redis 连接超时（12s）".to_string())?
        .map_err(|e| redis_connect_error(&e.to_string()))?;
        // 版本探测：INFO server 的 redis_version 行；失败不影响连接
        let version = redis_version(&mut conn)
            .await
            .unwrap_or_else(|| "Redis".to_string());
        Ok((DbEngine::Redis { conn, db }, version))
    }

    /// 同步 Oracle 连接的阻塞包裹：连接取出 → spawn_blocking 内执行闭包 →
    /// 连接归还（错误/取消也归还，避免连接丢失）。闭包内可直接调用同步 API。
    async fn with_oracle<T, F>(&mut self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&mut oracle::Connection) -> Result<T, String> + Send + 'static,
    {
        let taken = std::mem::replace(self, DbEngine::Closed);
        let DbEngine::Oracle(mut conn) = taken else {
            *self = taken;
            return Err("连接已关闭".to_string());
        };
        let result = tokio::task::spawn_blocking(move || {
            let r = f(&mut conn);
            (r, conn)
        })
        .await
        .map_err(|e| format!("Oracle 任务失败：{e}"));
        match result {
            Ok((r, conn)) => {
                *self = DbEngine::Oracle(conn);
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
                use mysql_async::prelude::Queryable as _;
                let names: Vec<String> = conn
                    .exec(
                        "SELECT schema_name FROM information_schema.schemata ORDER BY schema_name",
                        (),
                    )
                    .await
                    .map_err(|e| format!("MySQL 查询库清单失败：{e}"))?;
                Ok(names)
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
            DbEngine::Oracle(_) => {
                // Oracle 无连接级切库：清单 = 可见 schema（版本适配滤系统账号，
                // 见 oracle_usernames），切换走 select_database 的 CURRENT_SCHEMA
                self.with_oracle(|conn| oracle_usernames(conn)).await
            }
            DbEngine::Redis { conn, .. } => {
                // Redis 协议没有「枚举逻辑库」的命令：库数由 redis.conf 的
                // databases 决定（默认 16），只能经 CONFIG GET 读。CONFIG 被
                // rename/禁用（云托管常见）时按默认 16 兜底——清单只是给用户
                // 选，不准确也不影响实际 SELECT。
                let count = redis_database_count(conn).await.unwrap_or(16);
                Ok((0..count.max(1)).map(|i| format!("db{i}")).collect())
            }
            DbEngine::Closed => Err("连接已关闭".to_string()),
        }
    }

    /// MySQL 会话内切库（USE `db``，库名反引号转义）；PG 的切换在会话层走重连。
    pub async fn select_database(&mut self, database: &str) -> Result<(), String> {
        match self {
            DbEngine::MySql(conn) => {
                use mysql_async::prelude::Queryable as _;
                let escaped = database.replace('`', "``");
                conn.query_drop(format!("USE `{escaped}`"))
                    .await
                    .map_err(|e| format!("MySQL 切换数据库失败：{e}"))
            }
            DbEngine::Postgres(_) => {
                Err("PostgreSQL 切换数据库需重连，由会话层处理".to_string())
            }
            DbEngine::Oracle(_) => {
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
                self.with_oracle(move |conn| {
                    conn.execute(&sql, &[])
                        .map_err(|e| format!("Oracle 切换 schema 失败：{}", oracle_err(e)))?;
                    Ok(())
                })
                .await
            }
            DbEngine::Redis { conn, db } => {
                // SELECT 是连接级状态：multiplexed 连接上所有后续命令都落在这个
                // 库里。接受 "3" 与 "db3" 两种写法（与连接表单的两种习惯一致），
                // 解析失败直接报错而不是静默回落到 db0——切错库比报错更危险。
                let target = parse_redis_db(database)
                    .ok_or_else(|| format!("无效的 Redis 逻辑库：{database}（应为 0 或 db0 形态）"))?;
                redis::cmd("SELECT")
                    .arg(target)
                    .query_async::<()>(conn)
                    .await
                    .map_err(|e| format!("Redis 切换逻辑库失败：{e}"))?;
                *db = target;
                Ok(())
            }
            DbEngine::Closed => Err("连接已关闭".to_string()),
        }
    }

    /// 表/视图详情（kind: "table" | "view"）：MySQL 用 SHOW CREATE +
    /// information_schema；PG 无 SHOW CREATE，DDL 由列元数据+主键拼接（附
    /// pg_indexes 的真实索引 DDL），视图用 pg_get_viewdef。
    pub async fn table_detail(
        &mut self,
        schema: &str,
        table: &str,
        kind: &str,
    ) -> Result<TableDetail, String> {
        match self {
            DbEngine::MySql(conn) => {
                use mysql_async::prelude::Queryable as _;
                let s = schema.replace('`', "``");
                let n = table.replace('`', "``");
                let verb = if kind == "view" { "VIEW" } else { "TABLE" };
                let mut result = conn
                    .query_iter(format!("SHOW CREATE {verb} `{s}`.`{n}`"))
                    .await
                    .map_err(|e| format!("MySQL 拉取 DDL 失败：{e}"))?;
                let create_idx = result
                    .columns_ref()
                    .iter()
                    .position(|c| c.name_str().to_lowercase().contains("create"))
                    .ok_or_else(|| "SHOW CREATE 结果中未找到定义列".to_string())?;
                let rows: Vec<mysql_async::Row> = result
                    .collect()
                    .await
                    .map_err(|e| format!("MySQL 读取 DDL 失败：{e}"))?;
                let ddl = rows
                    .into_iter()
                    .next()
                    .and_then(|row| row.get::<String, usize>(create_idx))
                    .unwrap_or_default();

                let col_rows: Vec<MySqlColumnRow> = conn
                    .exec(
                        "SELECT column_name, column_type, is_nullable, column_default, column_comment \
                         FROM information_schema.columns \
                         WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
                        (schema, table),
                    )
                    .await
                    .map_err(|e| format!("MySQL 查询列结构失败：{e}"))?;
                let columns: Vec<Vec<String>> = col_rows
                    .iter()
                    .map(|(name, ty, nullable, default, comment)| {
                        vec![
                            name.clone(),
                            ty.clone(),
                            nullable.clone(),
                            default.clone().unwrap_or_else(|| "NULL".into()),
                            comment.clone().unwrap_or_default(),
                        ]
                    })
                    .collect();

                // 每索引一行：名字 + 唯一性 + 列清单（GROUP_CONCAT 聚合）
                let idx_rows: Vec<(String, i64, String)> = conn
                    .exec(
                        "SELECT index_name, MIN(non_unique), \
                         GROUP_CONCAT(column_name ORDER BY seq_in_index SEPARATOR ', ') \
                         FROM information_schema.statistics \
                         WHERE table_schema = ? AND table_name = ? GROUP BY index_name ORDER BY index_name",
                        (schema, table),
                    )
                    .await
                    .map_err(|e| format!("MySQL 查询索引失败：{e}"))?;
                let indexes: Vec<Vec<String>> = idx_rows
                    .iter()
                    .map(|(name, non_unique, cols)| {
                        vec![
                            name.clone(),
                            if *non_unique == 0 { "UNIQUE".into() } else { "".into() },
                            cols.clone(),
                        ]
                    })
                    .collect();

                Ok(TableDetail {
                    ddl,
                    column_headers: vec![
                        "Column".into(),
                        "Type".into(),
                        "Nullable".into(),
                        "Default".into(),
                        "Comment".into(),
                    ],
                    columns,
                    index_headers: vec!["Index".into(), "Unique".into(), "Columns".into()],
                    indexes,
                })
            }
            DbEngine::Postgres(client) => {
                let q = |v: &str| format!("\"{}\"", v.replace('"', "\"\""));
                let col_rows = client
                    .query(
                        "SELECT column_name, data_type, is_nullable, column_default \
                         FROM information_schema.columns \
                         WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
                        &[&schema, &table],
                    )
                    .await
                    .map_err(|e| format!("PostgreSQL 查询列结构失败：{e}"))?;
                let columns: Vec<Vec<String>> = col_rows
                    .iter()
                    .filter_map(|row| {
                        Some(vec![
                            row.try_get::<_, String>(0).ok()?,
                            row.try_get::<_, String>(1).ok()?,
                            row.try_get::<_, String>(2).ok()?,
                            row.try_get::<_, Option<String>>(3)
                                .ok()
                                .flatten()
                                .unwrap_or_else(|| "NULL".into()),
                        ])
                    })
                    .collect();

                let index_rows = client
                    .query(
                        "SELECT indexname, indexdef FROM pg_indexes \
                         WHERE schemaname = $1 AND tablename = $2 ORDER BY indexname",
                        &[&schema, &table],
                    )
                    .await
                    .map_err(|e| format!("PostgreSQL 查询索引失败：{e}"))?;
                let indexes: Vec<Vec<String>> = index_rows
                    .iter()
                    .filter_map(|row| {
                        Some(vec![
                            row.try_get::<_, String>(0).ok()?,
                            row.try_get::<_, String>(1).ok()?,
                        ])
                    })
                    .collect();

                // DDL：视图用 pg_get_viewdef；表由列元数据 + 主键拼接，并附真实索引 DDL
                let ddl = if kind == "view" {
                    let def = client
                        .query_opt(
                            "SELECT pg_get_viewdef(format('%I.%I', $1, $2)::regclass, true)",
                            &[&schema, &table],
                        )
                        .await
                        .map_err(|e| format!("PostgreSQL 拉取视图定义失败：{e}"))?
                        .and_then(|row| row.try_get::<_, String>(0).ok())
                        .unwrap_or_default();
                    format!("CREATE OR REPLACE VIEW {}.{} AS\n{def}", q(schema), q(table))
                } else {
                    let pk_cols: Vec<String> = client
                        .query(
                            "SELECT kcu.column_name \
                             FROM information_schema.table_constraints tc \
                             JOIN information_schema.key_column_usage kcu \
                               ON tc.constraint_name = kcu.constraint_name \
                              AND tc.constraint_schema = kcu.constraint_schema \
                             WHERE tc.constraint_type = 'PRIMARY KEY' \
                               AND tc.table_schema = $1 AND tc.table_name = $2 \
                             ORDER BY kcu.ordinal_position",
                            &[&schema, &table],
                        )
                        .await
                        .map_err(|e| format!("PostgreSQL 查询主键失败：{e}"))?
                        .iter()
                        .filter_map(|row| row.try_get::<_, String>(0).ok())
                        .collect();
                    let mut lines: Vec<String> = columns.iter().map(|c| {
                        let mut line = format!("  {} {}", q(c.first().unwrap_or(&String::new())), c.get(1).cloned().unwrap_or_default());
                        if c.get(2).map(|v| v == "NO").unwrap_or(false) {
                            line.push_str(" NOT NULL");
                        }
                        line
                    }).collect();
                    if !pk_cols.is_empty() {
                        let joined = pk_cols.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ");
                        lines.push(format!("  PRIMARY KEY ({joined})"));
                    }
                    let body = lines.join(",\n");
                    let mut ddl = format!("CREATE TABLE {}.{} (\n{body}\n);", q(schema), q(table));
                    if !indexes.is_empty() {
                        ddl.push_str("\n\n-- Indexes:\n");
                        for row in &indexes {
                            if let Some(def) = row.get(1) {
                                ddl.push_str(def);
                                ddl.push(';');
                                ddl.push('\n');
                            }
                        }
                    }
                    ddl
                };

                Ok(TableDetail {
                    ddl,
                    column_headers: vec![
                        "Column".into(),
                        "Type".into(),
                        "Nullable".into(),
                        "Default".into(),
                    ],
                    columns,
                    index_headers: vec!["Index".into(), "Definition".into()],
                    indexes,
                })
            }
            DbEngine::Oracle(_) => {
                // DDL 首选 DBMS_METADATA.GET_DDL（需权限，失败降级为列元数据
                // 拼装——PG 同型语义）；列结构 all_tab_columns（data_default 是
                // LONG 类型、驱动不支持，故不带 Default 列）；索引
                // all_indexes + all_ind_columns 经 LISTAGG 聚列。
                // SQL 用真实多行字符串（Oracle 接受任意空白，无 \ 续行）
                let schema_o = schema.to_string();
                let table_o = table.to_string();
                let kind_o = kind.to_string();
                self.with_oracle(move |conn| {
                    let object_type = if kind_o == "view" { "VIEW" } else { "TABLE" };
                    let ddl = conn
                        .query(
                            "SELECT DBMS_METADATA.GET_DDL(:1, :2, :3) FROM dual",
                            &[&object_type, &table_o, &schema_o],
                        )
                        .ok()
                        .and_then(|mut rs| rs.next())
                        .and_then(|r| r.ok())
                        .map(|row| oracle_row_string(&row, 0))
                        .unwrap_or_default();

                    let col_rs = conn
                        .query(
                            "SELECT column_name, data_type, nullable FROM all_tab_columns
                             WHERE owner = :1 AND table_name = :2 ORDER BY column_id",
                            &[&schema_o, &table_o],
                        )
                        .map_err(|e| format!("Oracle 查询列结构失败：{}", oracle_err(e)))?;
                    let columns: Vec<Vec<String>> = col_rs
                        .filter_map(|r| r.ok())
                        .map(|row| {
                            vec![
                                oracle_row_string(&row, 0),
                                oracle_row_string(&row, 1),
                                // 字典的 Y/N 统一为另两引擎的 YES/NO 展示
                                match row.sql_values().get(2).map(|v| v.to_string()) {
                                    Some(s) if s == "Y" => "YES".to_string(),
                                    Some(s) if s == "N" => "NO".to_string(),
                                    _ => String::new(),
                                },
                            ]
                        })
                        .collect();

                    let idx_rs = conn
                        .query(
                            "SELECT i.index_name, i.uniqueness,
                                    LISTAGG(c.column_name, ', ') WITHIN GROUP
                                      (ORDER BY c.column_position)
                             FROM all_indexes i
                             JOIN all_ind_columns c
                               ON c.index_name = i.index_name AND c.index_owner = i.owner
                             WHERE i.table_owner = :1 AND i.table_name = :2
                             GROUP BY i.index_name, i.uniqueness ORDER BY i.index_name",
                            &[&schema_o, &table_o],
                        )
                        .map_err(|e| format!("Oracle 查询索引失败：{}", oracle_err(e)))?;
                    let indexes: Vec<Vec<String>> = idx_rs
                        .filter_map(|r| r.ok())
                        .map(|row| {
                            vec![
                                oracle_row_string(&row, 0),
                                if oracle_row_string(&row, 1) == "UNIQUE" {
                                    "UNIQUE".to_string()
                                } else {
                                    String::new()
                                },
                                oracle_row_string(&row, 2),
                            ]
                        })
                        .collect();

                    // DDL 降级：列元数据拼装（含 NOT NULL）
                    let ddl = if ddl.is_empty() {
                        let q = |v: &str| format!("\"{}\"", v.replace('"', "\"\""));
                        let lines: Vec<String> = columns
                            .iter()
                            .map(|c| {
                                let mut line = format!(
                                    "  {} {}",
                                    q(c.first().unwrap_or(&String::new())),
                                    c.get(1).cloned().unwrap_or_default()
                                );
                                if c.get(2).map(|v| v == "NO").unwrap_or(false) {
                                    line.push_str(" NOT NULL");
                                }
                                line
                            })
                            .collect();
                        format!(
                            "CREATE TABLE {}.{} (\n{}\n);",
                            q(&schema_o),
                            q(&table_o),
                            lines.join(",\n")
                        )
                    } else {
                        ddl
                    };

                    Ok(TableDetail {
                        ddl,
                        column_headers: vec!["Column".into(), "Type".into(), "Nullable".into()],
                        columns,
                        index_headers: vec!["Index".into(), "Unique".into(), "Columns".into()],
                        indexes,
                    })
                })
                .await
            }
            DbEngine::Redis { conn, db } => {
                // Redis 没有 DDL：ddl 字段承载一段人类可读的键检视块（纯文本，
                // 不是可执行语句——前端 DDL Tab 是只读代码块，语义成立）。
                // schema 是逻辑库名（db0 形态）：树里点开的 key 属于该库，
                // 与会话当前库可能不同，故临时切过去再切回。
                // 前置一次 TYPE：不存在的 key 直接报错，避免返回一份全是
                // 占位值的「详情」误导用户（key 过期/被删是常态）。
                let target = parse_redis_db(schema).unwrap_or(*db);
                redis_select(conn, target).await?;
                let inspected = async {
                    let kind = redis_key_type(conn, table).await?;
                    let ddl = redis_inspect_block(conn, table, &kind).await;
                    let (_, rows) = redis_key_entries(conn, table, &kind).await;
                    Ok::<_, String>((ddl, rows))
                }
                .await;
                redis_select(conn, *db).await.ok();
                let (ddl, rows) = inspected?;
                Ok(TableDetail {
                    ddl,
                    column_headers: vec!["field".into(), "value".into()],
                    columns: rows,
                    // 无索引概念：空 headers + 空 rows，前端空表自然不渲染
                    index_headers: Vec::new(),
                    indexes: Vec::new(),
                })
            }
            DbEngine::Closed => Err("连接已关闭".to_string()),
        }
    }

    /// 拉取函数/过程/序列的完整定义与调用骨架
    ///（kind: "function" | "procedure" | "sequence"；序列模板取 nextval）。
    pub async fn object_definition(
        &mut self,
        schema: &str,
        name: &str,
        kind: &str,
    ) -> Result<ObjectDefinition, String> {
        match self {
            DbEngine::MySql(conn) => {
                use mysql_async::prelude::Queryable as _;
                // MySQL 无序列对象（AUTO_INCREMENT 列即其替代形态），树上也
                // 不会出现序列分组，此处仅防御性拒绝
                if kind == "sequence" {
                    return Err("MySQL 没有序列对象".to_string());
                }
                let verb = if kind == "function" { "FUNCTION" } else { "PROCEDURE" };
                let s = schema.replace('`', "``");
                let n = name.replace('`', "``");
                let mut result = conn
                    .query_iter(format!("SHOW CREATE {verb} `{s}`.`{n}`"))
                    .await
                    .map_err(|e| format!("MySQL 拉取定义失败：{e}"))?;
                // 列名随版本漂移（Procedure/sql_mode/Create Procedure/字符集…），
                // 找名字含 "create" 的列取定义文本
                let create_idx = result
                    .columns_ref()
                    .iter()
                    .position(|c| c.name_str().to_lowercase().contains("create"))
                    .ok_or_else(|| "SHOW CREATE 结果中未找到定义列".to_string())?;
                let rows: Vec<mysql_async::Row> = result
                    .collect()
                    .await
                    .map_err(|e| format!("MySQL 读取定义失败：{e}"))?;
                let row = rows
                    .into_iter()
                    .next()
                    .ok_or_else(|| "SHOW CREATE 未返回定义".to_string())?;
                let definition = row
                    .get::<String, usize>(create_idx)
                    .unwrap_or_default();
                // 入参个数（ordinal_position=0 是函数返回值，排除）
                let arg_count: i64 = conn
                    .exec_first(
                        "SELECT COUNT(*) FROM information_schema.parameters \
                         WHERE specific_schema = ? AND specific_name = ? \
                           AND ordinal_position > 0",
                        (schema, name),
                    )
                    .await
                    .map_err(|e| format!("MySQL 查询参数个数失败：{e}"))?
                    .unwrap_or(0);
                let call_template = if kind == "function" {
                    format!("SELECT `{s}`.`{n}`()")
                } else {
                    format!("CALL `{s}`.`{n}`()")
                };
                Ok(ObjectDefinition {
                    definition,
                    call_template,
                    arg_count: arg_count.max(0) as usize,
                })
            }
            DbEngine::Postgres(client) => {
                // 序列：pg_get_sequencedef（PG 10+）直接返回 CREATE SEQUENCE 语句
                if kind == "sequence" {
                    let row = client
                        .query_one(
                            "SELECT pg_get_sequencedef(format('%I.%I', $1, $2)::regclass)",
                            &[&schema, &name],
                        )
                        .await
                        .map_err(|e| format!("PostgreSQL 拉取序列定义失败：{e}"))?;
                    let definition: String = row
                        .try_get(0)
                        .map_err(|e| format!("PostgreSQL 读取序列定义失败：{e}"))?;
                    return Ok(ObjectDefinition {
                        definition,
                        call_template: format!("SELECT nextval('{schema}.{name}');"),
                        arg_count: 0,
                    });
                }
                let row = client
                    .query_opt(
                        "SELECT pg_get_functiondef(p.oid), p.pronargs \
                         FROM pg_proc p JOIN pg_namespace n ON p.pronamespace = n.oid \
                         WHERE n.nspname = $1 AND p.proname = $2 LIMIT 1",
                        &[&schema, &name],
                    )
                    .await
                    .map_err(|e| format!("PostgreSQL 拉取定义失败：{e}"))?
                    .ok_or_else(|| "未找到该函数/过程（可能已被删除）".to_string())?;
                let definition: String = row
                    .try_get(0)
                    .map_err(|e| format!("PostgreSQL 读取定义失败：{e}"))?;
                let arg_count: i16 = row.try_get(1).unwrap_or(0);
                let q = |v: &str| format!("\"{}\"", v.replace('"', "\"\""));
                let call_template = if kind == "function" {
                    // SELECT * FROM f() 对标量/集合/表函数均合法
                    format!("SELECT * FROM {}.{}()", q(schema), q(name))
                } else {
                    format!("CALL {}.{}()", q(schema), q(name))
                };
                Ok(ObjectDefinition {
                    definition,
                    call_template,
                    arg_count: arg_count.max(0) as usize,
                })
            }
            DbEngine::Oracle(_) => {
                // 定义：all_source 按行拼回（CREATE OR REPLACE 前缀在首行内）；
                // 入参个数：all_arguments 的具名参数（argument_name 为 NULL 的
                // 是函数返回值，排除）
                let schema_o = schema.to_string();
                let name_o = name.to_string();
                let kind_o = kind.to_string();
                self.with_oracle(move |conn| {
                    // 序列：DBMS_METADATA 出 CREATE SEQUENCE DDL；模板取 NEXTVAL
                    if kind_o == "sequence" {
                        let ddl = conn
                            .query(
                                "SELECT DBMS_METADATA.GET_DDL('SEQUENCE', :1, :2) FROM dual",
                                &[&name_o, &schema_o],
                            )
                            .map_err(|e| format!("Oracle 拉取序列定义失败：{}", oracle_err(e)))?
                            .filter_map(|r| r.ok())
                            .next()
                            .map(|row| oracle_row_string(&row, 0))
                            .unwrap_or_default();
                        if ddl.trim().is_empty() {
                            return Err("未找到该序列（可能已被删除）".to_string());
                        }
                        return Ok(ObjectDefinition {
                            definition: ddl,
                            call_template: format!("SELECT {schema_o}.{name_o}.NEXTVAL FROM dual;"),
                            arg_count: 0,
                        });
                    }
                    let obj_type = if kind_o == "function" { "FUNCTION" } else { "PROCEDURE" };
                    let src = conn
                        .query(
                            "SELECT text FROM all_source
                             WHERE owner = :1 AND name = :2 AND type = :3
                             ORDER BY line",
                            &[&schema_o, &name_o, &obj_type],
                        )
                        .map_err(|e| format!("Oracle 拉取定义失败：{}", oracle_err(e)))?;
                    let definition = src
                        .filter_map(|r| r.ok())
                        .map(|row| oracle_row_string(&row, 0))
                        .collect::<Vec<_>>()
                        .join("");
                    if definition.trim().is_empty() {
                        return Err("未找到该函数/过程（可能已被删除）".to_string());
                    }
                    let arg_count = conn
                        .query_row_as::<i64>(
                            "SELECT COUNT(*) FROM all_arguments
                             WHERE owner = :1 AND object_name = :2
                               AND argument_name IS NOT NULL",
                            &[&schema_o, &name_o],
                        )
                        .map_err(|e| format!("Oracle 查询参数个数失败：{}", oracle_err(e)))?;
                    let call_template = if kind_o == "function" {
                        format!("SELECT \"{schema_o}\".\"{name_o}\"() FROM dual")
                    } else {
                        format!("BEGIN \"{schema_o}\".\"{name_o}\"(); END;")
                    };
                    Ok(ObjectDefinition {
                        definition,
                        call_template,
                        arg_count: arg_count.max(0) as usize,
                    })
                })
                .await
            }
            DbEngine::Redis { conn, db } => {
                // Redis 树里没有函数/过程/序列（只有 key），本臂只服务「key
                // 详情」入口：定义块 = 与 table_detail 同一份检视文本，
                // call_template = 该类型最自然的读取命令（前端可一键执行）。
                // 同 table_detail：临时切到 key 所属逻辑库再切回。
                let target = parse_redis_db(schema).unwrap_or(*db);
                redis_select(conn, target).await?;
                let inspected = async {
                    let type_name = redis_key_type(conn, name).await?;
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
                use mysql_async::prelude::Queryable as _;
                let names: Vec<String> = conn
                    .exec(
                        "SELECT schema_name FROM information_schema.schemata ORDER BY schema_name",
                        (),
                    )
                    .await
                    .map_err(|e| format!("MySQL 查询库清单失败：{e}"))?;
                Ok(names
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
            DbEngine::Oracle(_) => {
                // 顶层 = schema（用户）：版本适配滤系统账号（oracle_usernames）
                self.with_oracle(|conn| {
                    Ok(oracle_usernames(conn)?
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
            DbEngine::Closed => Err("连接已关闭".to_string()),
        }
    }

    /// 库/schema 下固定分组（表/视图/函数/过程/序列），带对象计数，空组不返回。
    async fn catalog_groups(&mut self, schema: &str) -> Result<Vec<CatalogNode>, String> {
        match self {
            DbEngine::MySql(conn) => {
                use mysql_async::prelude::Queryable as _;
                let tables: Vec<(String, i64)> = conn
                    .exec(
                        "SELECT table_type, COUNT(*) FROM information_schema.tables \
                         WHERE table_schema = ? GROUP BY table_type",
                        (schema,),
                    )
                    .await
                    .map_err(|e| format!("MySQL 查询表清单失败：{e}"))?;
                let routines: Vec<(String, i64)> = conn
                    .exec(
                        "SELECT routine_type, COUNT(*) FROM information_schema.routines \
                         WHERE routine_schema = ? GROUP BY routine_type",
                        (schema,),
                    )
                    .await
                    .map_err(|e| format!("MySQL 查询例程清单失败：{e}"))?;
                // 序列：仅 MariaDB 有 information_schema.sequences，MySQL 8.0 报
                // 表不存在 → 静默计 0（组不出现）
                let sequences: i64 = match conn
                    .exec_first::<i64, _, _>(
                        "SELECT COUNT(*) FROM information_schema.sequences \
                         WHERE sequence_schema = ?",
                        (schema,),
                    )
                    .await
                {
                    Ok(Some(n)) => n,
                    _ => 0,
                };
                let count_of = |kind: &str, rows: &[(String, i64)]| {
                    rows.iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(kind))
                        .map(|(_, c)| *c)
                        .unwrap_or(0)
                };
                Ok(group_nodes(
                    schema,
                    count_of("BASE TABLE", &tables),
                    count_of("VIEW", &tables),
                    count_of("FUNCTION", &routines),
                    count_of("PROCEDURE", &routines),
                    sequences,
                ))
            }
            DbEngine::Postgres(client) => {
                let tables: Vec<(String, i64)> = client
                    .query(
                        "SELECT table_type, COUNT(*) FROM information_schema.tables \
                         WHERE table_schema = $1 GROUP BY table_type",
                        &[&schema],
                    )
                    .await
                    .map_err(|e| format!("PostgreSQL 查询表清单失败：{e}"))?
                    .iter()
                    .filter_map(|row| {
                        Some((
                            row.try_get::<_, String>(0).ok()?,
                            row.try_get::<_, i64>(1).ok()?,
                        ))
                    })
                    .collect();
                let routines: Vec<(String, i64)> = client
                    .query(
                        "SELECT routine_type, COUNT(*) FROM information_schema.routines \
                         WHERE routine_schema = $1 GROUP BY routine_type",
                        &[&schema],
                    )
                    .await
                    .map_err(|e| format!("PostgreSQL 查询例程清单失败：{e}"))?
                    .iter()
                    .filter_map(|row| {
                        Some((
                            row.try_get::<_, String>(0).ok()?,
                            row.try_get::<_, i64>(1).ok()?,
                        ))
                    })
                    .collect();
                let sequences: i64 = client
                    .query_opt(
                        "SELECT COUNT(*) FROM information_schema.sequences \
                         WHERE sequence_schema = $1",
                        &[&schema],
                    )
                    .await
                    .ok()
                    .flatten()
                    .and_then(|row| row.try_get::<_, i64>(0).ok())
                    .unwrap_or(0);
                let count_of = |kind: &str, rows: &[(String, i64)]| {
                    rows.iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(kind))
                        .map(|(_, c)| *c)
                        .unwrap_or(0)
                };
                Ok(group_nodes(
                    schema,
                    count_of("BASE TABLE", &tables),
                    count_of("VIEW", &tables),
                    count_of("FUNCTION", &routines),
                    count_of("PROCEDURE", &routines),
                    sequences,
                ))
            }
            DbEngine::Oracle(_) => {
                // 五类对象在 all_objects 一表内聚合（SEQUENCE 也在）；
                // BIN$ 前缀 = 回收站对象不计数
                let schema_o = schema.to_string();
                self.with_oracle(move |conn| {
                    let rs = conn
                        .query(
                            "SELECT object_type, COUNT(*) FROM all_objects
                             WHERE owner = :1 AND object_name NOT LIKE 'BIN$%'
                               AND object_type IN
                                 ('TABLE','VIEW','FUNCTION','PROCEDURE','SEQUENCE')
                             GROUP BY object_type",
                            &[&schema_o],
                        )
                        .map_err(|e| format!("Oracle 查询对象计数失败：{}", oracle_err(e)))?;
                    let counts: Vec<(String, i64)> = rs
                        .filter_map(|r| r.ok())
                        .map(|row| {
                            (
                                oracle_row_string(&row, 0),
                                row.sql_values()
                                    .get(1)
                                    .and_then(|v| v.to_string().parse::<i64>().ok())
                                    .unwrap_or(0),
                            )
                        })
                        .collect();
                    let count_of = |kind: &str| {
                        counts
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(kind))
                            .map(|(_, c)| *c)
                            .unwrap_or(0)
                    };
                    Ok(group_nodes(
                        &schema_o,
                        count_of("TABLE"),
                        count_of("VIEW"),
                        count_of("FUNCTION"),
                        count_of("PROCEDURE"),
                        count_of("SEQUENCE"),
                    ))
                })
                .await
            }
            DbEngine::Redis { conn, db } => {
                // 逻辑库 → 值类型分组：SCAN 抓 key 再批量 TYPE 归类。key 上限
                // 5000（工作台是浏览用途，全库枚举在百万 key 实例上会把 UI
                // 拖死）；分组计数是「已扫描范围内」的口径，不是 DBSIZE。
                let target = parse_redis_db(schema)
                    .ok_or_else(|| format!("无效的 Redis 逻辑库：{schema}"))?;
                // 浏览别的库不该改掉会话的执行上下文：切过去、扫完切回来
                // （调用方持有 engine 的 tokio Mutex，这段期间无并发命令）
                redis_select(conn, target).await?;
                let scanned = redis_scan_typed_keys(conn).await;
                redis_select(conn, *db).await.ok();
                let typed = scanned?;
                let mut counts = [0usize; REDIS_VALUE_TYPES.len()];
                for (_, type_name) in &typed {
                    if let Some(i) = REDIS_VALUE_TYPES.iter().position(|t| t == type_name)
                        && let Some(c) = counts.get_mut(i)
                    {
                        *c += 1;
                    }
                }
                Ok(REDIS_VALUE_TYPES
                    .iter()
                    .zip(counts.iter())
                    .filter(|(_, count)| **count > 0)
                    .filter_map(|(type_name, count)| {
                        let group = ObjectGroup::from_redis_type(type_name)?;
                        Some(CatalogNode {
                            key: format!("{schema}#{}", group.as_str()),
                            name: (*type_name).to_string(),
                            kind: "group".to_string(),
                            detail: Some(count.to_string()),
                        })
                    })
                    .collect())
            }
            DbEngine::Closed => Err("连接已关闭".to_string()),
        }
    }

    /// 分组内的对象清单。
    async fn catalog_objects(
        &mut self,
        schema: &str,
        group: ObjectGroup,
    ) -> Result<Vec<CatalogNode>, String> {
        let kind = group.as_str().to_string();
        // Redis 分组没有 SQL 形态：先进 Redis 臂（key 是「对象」，detail 带
        // 值类型供前端生成读命令），SQL 引擎的 group→SQL 映射留在下面。
        if let DbEngine::Redis { conn, db } = self {
            let type_name = group
                .redis_type()
                .ok_or_else(|| format!("Redis 不支持的对象分组：{kind}"))?;
            let target =
                parse_redis_db(schema).ok_or_else(|| format!("无效的 Redis 逻辑库：{schema}"))?;
            redis_select(conn, target).await?;
            let scanned = redis_scan_typed_keys(conn).await;
            redis_select(conn, *db).await.ok();
            return Ok(scanned?
                .into_iter()
                .filter(|(_, t)| t == type_name)
                .map(|(key, t)| CatalogNode {
                    // key 里可能含 `.`，故用 `{db}.{key}` 仅作唯一标识（前端
                    // Redis 分支按 node.name 取裸 key，不切 key）
                    key: format!("{schema}.{key}"),
                    name: key,
                    // kind = "table"：复用前端「表节点可展开/可右键」的现成通路
                    kind: "table".to_string(),
                    detail: Some(t),
                })
                .collect());
        }
        let (sql_mysql, sql_pg, name_col): (&str, &str, &str) = match group {
            ObjectGroup::Table => (
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = ? AND table_type = 'BASE TABLE' ORDER BY table_name",
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = $1 AND table_type = 'BASE TABLE' ORDER BY table_name",
                "table_name",
            ),
            ObjectGroup::View => (
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = ? AND table_type = 'VIEW' ORDER BY table_name",
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = $1 AND table_type = 'VIEW' ORDER BY table_name",
                "table_name",
            ),
            ObjectGroup::Function => (
                "SELECT routine_name FROM information_schema.routines \
                 WHERE routine_schema = ? AND routine_type = 'FUNCTION' ORDER BY routine_name",
                "SELECT routine_name FROM information_schema.routines \
                 WHERE routine_schema = $1 AND routine_type = 'FUNCTION' ORDER BY routine_name",
                "routine_name",
            ),
            ObjectGroup::Procedure => (
                "SELECT routine_name FROM information_schema.routines \
                 WHERE routine_schema = ? AND routine_type = 'PROCEDURE' ORDER BY routine_name",
                "SELECT routine_name FROM information_schema.routines \
                 WHERE routine_schema = $1 AND routine_type = 'PROCEDURE' ORDER BY routine_name",
                "routine_name",
            ),
            ObjectGroup::Sequence => (
                "SELECT sequence_name FROM information_schema.sequences \
                 WHERE sequence_schema = ? ORDER BY sequence_name",
                "SELECT sequence_name FROM information_schema.sequences \
                 WHERE sequence_schema = $1 ORDER BY sequence_name",
                "sequence_name",
            ),
            // Redis 六型在上面的 early return 里已消化；走到这里说明是
            // 非 Redis 引擎收到了 Redis 分组（前端/调用方拼错），直接拒绝
            // 而不是编造一条查不出东西的 SQL。
            ObjectGroup::RedisString
            | ObjectGroup::RedisHash
            | ObjectGroup::RedisList
            | ObjectGroup::RedisSet
            | ObjectGroup::RedisZset
            | ObjectGroup::RedisStream => {
                return Err(format!("当前引擎不支持对象分组：{kind}"));
            }
        };
        let _ = name_col;
        match self {
            DbEngine::MySql(conn) => {
                use mysql_async::prelude::Queryable as _;
                let names: Vec<String> = conn
                    .exec(sql_mysql, (schema,))
                    .await
                    .map_err(|e| format!("MySQL 查询对象清单失败：{e}"))?;
                Ok(names
                    .into_iter()
                    .map(|name| CatalogNode {
                        key: format!("{schema}.{name}"),
                        name,
                        kind: kind.clone(),
                        detail: None,
                    })
                    .collect())
            }
            DbEngine::Postgres(client) => {
                let rows = client
                    .query(sql_pg, &[&schema])
                    .await
                    .map_err(|e| format!("PostgreSQL 查询对象清单失败：{e}"))?;
                Ok(rows
                    .iter()
                    .filter_map(|row| row.try_get::<_, String>(0).ok())
                    .map(|name| CatalogNode {
                        key: format!("{schema}.{name}"),
                        name,
                        kind: kind.clone(),
                        detail: None,
                    })
                    .collect())
            }
            DbEngine::Oracle(_) => {
                // all_objects 按对象类型列（Oracle 无 information_schema）；
                // 对象类型大写、BIN$ 回收站滤除
                let schema_o = schema.to_string();
                let obj_type = match group {
                    ObjectGroup::Table => "TABLE",
                    ObjectGroup::View => "VIEW",
                    ObjectGroup::Function => "FUNCTION",
                    ObjectGroup::Procedure => "PROCEDURE",
                    ObjectGroup::Sequence => "SEQUENCE",
                    // Redis 分组已在函数入口的 early return 拦掉，Oracle
                    // 永远收不到；给个不存在的类型名让服务端返回空集，
                    // 保持本臂签名不变（不必为死路径引入 Result）
                    _ => "VELOTERM_UNREACHABLE",
                }
                .to_string();
                let kind_o = kind.clone();
                self.with_oracle(move |conn| {
                    let rs = conn
                        .query(
                            "SELECT object_name FROM all_objects
                             WHERE owner = :1 AND object_type = :2
                               AND object_name NOT LIKE 'BIN$%' ORDER BY object_name",
                            &[&schema_o, &obj_type],
                        )
                        .map_err(|e| format!("Oracle 查询对象清单失败：{}", oracle_err(e)))?;
                    Ok(rs
                        .filter_map(|r| r.ok())
                        .map(|row| oracle_row_string(&row, 0))
                        .map(|name| CatalogNode {
                            key: format!("{schema_o}.{name}"),
                            name,
                            kind: kind_o.clone(),
                            detail: None,
                        })
                        .collect())
                })
                .await
            }
            // 函数入口的 early return 已消费全部 Redis 分组；这里只是让
            // match 穷尽（借用检查器不会因 early return 收窄 self 类型）
            DbEngine::Redis { .. } => Err(format!("当前引擎不支持对象分组：{kind}")),
            DbEngine::Closed => Err("连接已关闭".to_string()),
        }
    }

    async fn catalog_columns(
        &mut self,
        schema: &str,
        table: &str,
    ) -> Result<Vec<CatalogNode>, String> {
        match self {
            DbEngine::MySql(conn) => {
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
            DbEngine::Oracle(_) => {
                // all_tab_columns 覆盖表与视图；data_type 概括为裸类型名
                //（与另两引擎的 information_schema 口径一致）
                let schema_o = schema.to_string();
                let table_o = table.to_string();
                self.with_oracle(move |conn| {
                    let rs = conn
                        .query(
                            "SELECT column_name, data_type FROM all_tab_columns
                             WHERE owner = :1 AND table_name = :2 ORDER BY column_id",
                            &[&schema_o, &table_o],
                        )
                        .map_err(|e| format!("Oracle 查询列清单失败：{}", oracle_err(e)))?;
                    Ok(rs
                        .filter_map(|r| r.ok())
                        .map(|row| {
                            (
                                oracle_row_string(&row, 0),
                                oracle_row_string(&row, 1),
                            )
                        })
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
            DbEngine::Redis { conn, db } => {
                // Redis 的「列」= 该 key 内的字段/成员/元素（按值类型取前 200
                // 条）。detail 放元素的角色标签（field / index / member /
                // score / id），前端列表直接可见而不必展开。
                // 同 table_detail：临时切到 key 所属逻辑库再切回。
                let target = parse_redis_db(schema).unwrap_or(*db);
                redis_select(conn, target).await?;
                let entries = async {
                    let type_name = redis_key_type(conn, table).await?;
                    Ok::<_, String>(redis_key_entries(conn, table, &type_name).await)
                }
                .await;
                redis_select(conn, *db).await.ok();
                let (role, rows) = entries?;
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
                            key: format!("{schema}.{table}.{field}"),
                            name: field,
                            kind: "column".to_string(),
                            detail: Some(detail),
                        }
                    })
                    .collect())
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
            DbEngine::Oracle(_) => {
                // 同步引擎：整个执行（含行流迭代与回调发射）移入 spawn_blocking，
                // 回调直接在阻塞线程内调用（Channel 发送是 Send 安全的）。
                // 取消粒度 = 语句间（会话层 AtomicBool）；行流中不可中断——
                // rust-oracle 的 fetch 阻塞在 OCI 上无协作取消点
                let sql_o = sql.to_string();
                let result = self
                    .with_oracle(move |conn| {
                        // PL/SQL 匿名块（BEGIN/DECLARE 起）：execute + 隐式结果集
                        //（dbms_sql.return_result，客户端与服务器均 12.1+）。
                        // 工作台是单结果集模型，多组隐式结果只输出第一组
                        let trimmed = sql_o.trim_start();
                        let upper_start = trimmed
                            .get(..32)
                            .map(|s| s.to_uppercase())
                            .unwrap_or_default();
                        if upper_start.starts_with("BEGIN")
                            || upper_start.starts_with("DECLARE")
                        {
                            let mut stmt = conn
                                .statement(&sql_o)
                                .build()
                                .map_err(|e| format!("Oracle：{}", oracle_err(e)))?;
                            stmt.execute(&[])
                                .map_err(|e| format!("Oracle：{}", oracle_err(e)))?;
                            let affected = stmt
                                .row_count()
                                .map_err(|e| format!("Oracle：{}", oracle_err(e)))?;
                            let mut summary = ExecSummary {
                                affected: Some(affected),
                                ..Default::default()
                            };
                            if let Ok(Some(mut cursor)) = stmt.implicit_result() {
                                let mut rs = cursor
                                    .query()
                                    .map_err(|e| format!("Oracle：{}", oracle_err(e)))?;
                                emit_oracle_rows(
                                    &mut rs, max_rows, &mut on_columns, &mut on_rows,
                                    &mut summary,
                                );
                            }
                            return Ok(summary);
                        }

                        // 常规语句：query 逐行流式 fetch（OCI 每行一次，
                        // max_rows 防护客户端侧成立，大表不整载）
                        let mut rs = conn
                            .query(&sql_o, &[])
                            .map_err(|e| format!("Oracle：{}", oracle_err(e)))?;
                        let columns: Vec<String> = rs
                            .column_info()
                            .iter()
                            .map(|c| c.name().to_string())
                            .collect();
                        if columns.is_empty() {
                            // DML / DDL：无结果集（affected 拿不到——PG 同型语义，
                            // 空列即视为已执行）
                            return Ok(ExecSummary::default());
                        }
                        on_columns(&columns);
                        let mut summary = ExecSummary {
                            columns,
                            ..Default::default()
                        };
                        emit_oracle_rows(&mut rs, max_rows, &mut on_columns, &mut on_rows, &mut summary);
                        Ok(summary)
                    })
                    .await;
                // 取消信号在语句间由会话层判定；本语句执行中收到通知时
                // 无法中断阻塞线程，下一轮语句开始前生效
                let _ = cancel_notify;
                result
            }
            DbEngine::Redis { conn, db } => {
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
                    r = cmd.query_async::<redis::Value>(&mut *conn) => {
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
const REDIS_VALUE_TYPES: [&str; 6] = ["string", "hash", "list", "set", "zset", "stream"];

/// 扫描 key 上限：工作台是浏览用途，全库枚举在百万 key 实例上会把 UI 拖死
const REDIS_SCAN_KEY_CAP: usize = 5000;

/// 单次 SCAN 的 COUNT 提示值（服务端可返回更多，只是提示）
const REDIS_SCAN_COUNT: usize = 1000;

/// 详情/列清单里单个 key 的元素上限
const REDIS_ELEMENT_CAP: usize = 200;

/// 流详情里 XRANGE 的条数上限（stream 一条消息可能含多个字段）
const REDIS_STREAM_CAP: usize = 100;

/// `db3` / `3` → 3（大小写不敏感的前缀剥离）。空串/不可解析 → None。
fn parse_redis_db(value: &str) -> Option<u32> {
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

/// 连接表单的 database 字段 → 逻辑库索引（空/不可解析按 db0）
fn redis_db_index(value: Option<&str>) -> u32 {
    value.and_then(parse_redis_db).unwrap_or(0)
}

/// Redis 连接错误文案：认证类错误追加排查提示（NOAUTH 未提供密码 /
/// WRONGPASS 密码错 / AUTH 一般性失败；云实例还会给 NOAUTH Authentication
/// required 或 NOPERM）。
fn redis_connect_error(message: &str) -> String {
    let auth_hint = ["NOAUTH", "WRONGPASS", "AUTH", "NOPERM"]
        .iter()
        .any(|k| message.to_uppercase().contains(k));
    if auth_hint {
        format!("Redis 连接失败：{message}（检查用户名/ACL 与密码）")
    } else {
        format!("Redis 连接失败：{message}")
    }
}

/// INFO server 里取 `redis_version:`（bulk string 逐行扫描）；失败返回 None
async fn redis_version(conn: &mut redis::aio::MultiplexedConnection) -> Option<String> {
    let info: String = redis::cmd("INFO")
        .arg("server")
        .query_async(conn)
        .await
        .ok()?;
    redis_info_field(&info, "redis_version").map(|v| format!("Redis {v}"))
}

/// INFO 文本里取 `field:value`（INFO 是 CRLF 分隔的纯文本）
fn redis_info_field(info: &str, field: &str) -> Option<String> {
    info.lines()
        .filter_map(|line| line.trim().split_once(':'))
        .find(|(k, _)| *k == field)
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// CONFIG GET databases → 逻辑库个数（CONFIG 被 rename/禁用时返回 None，
/// 调用方按默认 16 兜底）。返回形态随版本/协议而异（RESP2 是扁平数组、
/// RESP3 是 map），统一走值转换。
async fn redis_database_count(conn: &mut redis::aio::MultiplexedConnection) -> Option<u32> {
    let value: redis::Value = redis::cmd("CONFIG")
        .arg("GET")
        .arg("databases")
        .query_async(conn)
        .await
        .ok()?;
    let (_, rows) = redis_value_to_table(&value, "CONFIG");
    rows.first()
        .and_then(|row| row.get(1))
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|n| *n > 0)
}

/// INFO keyspace → 每个非空逻辑库的 key 数（下标 = db 索引）。
/// 形如 `db0:keys=3,expires=0,avg_ttl=0`；空库不出现在 INFO 里。
async fn redis_keyspace_counts(conn: &mut redis::aio::MultiplexedConnection) -> Vec<usize> {
    let Ok(info): Result<String, _> = redis::cmd("INFO")
        .arg("keyspace")
        .query_async(conn)
        .await
    else {
        return Vec::new();
    };
    let mut counts: Vec<usize> = Vec::new();
    for line in info.lines() {
        let Some((db, rest)) = line.trim().split_once(':') else {
            continue;
        };
        let Some(idx) = parse_redis_db(db) else {
            continue;
        };
        let keys = rest
            .split(',')
            .filter_map(|part| part.split_once('='))
            .find(|(k, _)| *k == "keys")
            .and_then(|(_, v)| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let idx = idx as usize;
        if counts.len() <= idx {
            counts.resize(idx + 1, 0);
        }
        if let Some(slot) = counts.get_mut(idx) {
            *slot = keys;
        }
    }
    counts
}

/// SELECT 切库（不改引擎的 db 字段——调用方负责上下文还原）
async fn redis_select(
    conn: &mut redis::aio::MultiplexedConnection,
    db: u32,
) -> Result<(), String> {
    redis::cmd("SELECT")
        .arg(db)
        .query_async::<()>(conn)
        .await
        .map_err(|e| format!("Redis 切换逻辑库失败：{e}"))
}

/// SCAN 抓 key（上限 REDIS_SCAN_KEY_CAP），再按 REDIS_SCAN_COUNT 条一批
/// 用 pipeline 发 TYPE 归类。返回 (key, type) 序列，顺序即 SCAN 顺序
///（Redis 不保证有序，这里也不再排——保持零额外开销）。
/// 单条 TYPE 失败（key 在 SCAN 与 TYPE 之间过期）整批丢弃该条。
async fn redis_scan_typed_keys(
    conn: &mut redis::aio::MultiplexedConnection,
) -> Result<Vec<(String, String)>, String> {
    let keys = redis_scan_keys(conn).await?;
    let mut typed = Vec::with_capacity(keys.len());
    for chunk in keys.chunks(REDIS_SCAN_COUNT) {
        let mut pipe = redis::pipe();
        for key in chunk {
            pipe.cmd("TYPE").arg(key);
        }
        // 单条失败不炸整批：pipeline 的 ignore_errors 让错误项返回 Nil，
        // 下面按位置对齐时跳过即可
        pipe.ignore_errors();
        let values: Vec<redis::Value> = pipe
            .query_async(conn)
            .await
            .map_err(|e| format!("Redis 探测 key 类型失败：{e}"))?;
        for (key, value) in chunk.iter().zip(values.iter()) {
            // ignore_errors 的 pipeline 会把单条错误留在数组原位（ServerError），
            // nil 表示该 key 在 SCAN 与 TYPE 之间已过期——两者都不该当类型名
            if matches!(value, redis::Value::ServerError(_) | redis::Value::Nil) {
                continue;
            }
            if let Some(type_name) = redis_value_string(value) {
                typed.push((key.clone(), type_name));
            }
        }
    }
    Ok(typed)
}

/// SCAN 全库抓 key（游标循环，到 0 或上限为止）
async fn redis_scan_keys(
    conn: &mut redis::aio::MultiplexedConnection,
) -> Result<Vec<String>, String> {
    let mut cursor = 0u64;
    let mut keys: Vec<String> = Vec::new();
    loop {
        // SCAN 返回 [cursor, [key...]]；RESP3 下也是 Array 两元素。
        // 按 Value 取而不是 (u64, Vec<String>)：key 允许任意字节，
        // Vec<String> 遇到非 UTF-8 key 会让整页反序列化失败。
        let value: redis::Value = redis::cmd("SCAN")
            .arg(cursor)
            .arg("COUNT")
            .arg(REDIS_SCAN_COUNT)
            .query_async(conn)
            .await
            .map_err(|e| format!("Redis 扫描 key 失败：{e}"))?;
        let redis::Value::Array(parts) = value else {
            break;
        };
        let next = parts
            .first()
            .and_then(redis_value_string)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        if let Some(redis::Value::Array(batch)) = parts.get(1) {
            keys.extend(batch.iter().filter_map(redis_value_string));
        }
        if next == 0 || keys.len() >= REDIS_SCAN_KEY_CAP {
            break;
        }
        cursor = next;
    }
    keys.truncate(REDIS_SCAN_KEY_CAP);
    Ok(keys)
}

/// TYPE 命令取 key 的类型；key 不存在（none）直接报错——调用方（详情/
/// 列清单/定义）都只对存在的 key 有意义。
async fn redis_key_type(
    conn: &mut redis::aio::MultiplexedConnection,
    key: &str,
) -> Result<String, String> {
    let type_name: String = redis::cmd("TYPE")
        .arg(key)
        .query_async(conn)
        .await
        .map_err(|e| format!("Redis 查询 key 类型失败：{e}"))?;
    if type_name == "none" {
        return Err("未找到该 key（可能已过期或被删除）".to_string());
    }
    Ok(type_name)
}

/// 键检视块（table_detail 的 ddl 字段 / object_definition 的 definition）：
/// 纯文本元信息，不是可执行语句。MEMORY USAGE 与 OBJECT ENCODING 是
/// best-effort（托管实例常禁用），失败就整行省略而不是写 "N/A"。
async fn redis_inspect_block(
    conn: &mut redis::aio::MultiplexedConnection,
    key: &str,
    type_name: &str,
) -> String {
    let mut lines = vec![
        format!("KEY {key}"),
        format!("TYPE {type_name}"),
    ];
    // TTL：-1 永不过期、-2 不存在（此时前面 TYPE 已经报错了，不会到这）
    let ttl: i64 = redis::cmd("TTL")
        .arg(key)
        .query_async(conn)
        .await
        .unwrap_or(-1);
    lines.push(format!("TTL {ttl}"));
    // MEMORY USAGE 是 4.0+ 命令，且可被 rename/禁用
    if let Ok(bytes) = redis::cmd("MEMORY")
        .arg("USAGE")
        .arg(key)
        .query_async::<i64>(conn)
        .await
    {
        lines.push(format!("MEMORY USAGE {bytes}"));
    }
    // OBJECT ENCODING（2.2+）：展示内部编码（ziplist/quicklist/hashtable…）
    if let Ok(encoding) = redis::cmd("OBJECT")
        .arg("ENCODING")
        .arg(key)
        .query_async::<String>(conn)
        .await
    {
        lines.push(format!("ENCODING {encoding}"));
    }
    // 类型专属长度：STRLEN/HLEN/LLEN/SCARD/ZCARD/XLEN
    if let Some(len_cmd) = redis_length_command(type_name)
        && let Ok(len) = redis::cmd(len_cmd)
            .arg(key)
            .query_async::<i64>(conn)
            .await
    {
        lines.push(format!("LENGTH {len}"));
    }
    lines.join("\n")
}

/// 值类型 → 长度命令（未知类型无长度概念）
fn redis_length_command(type_name: &str) -> Option<&'static str> {
    Some(match type_name {
        "string" => "STRLEN",
        "hash" => "HLEN",
        "list" => "LLEN",
        "set" => "SCARD",
        "zset" => "ZCARD",
        "stream" => "XLEN",
        _ => return None,
    })
}

/// 该类型最自然的读取命令（object_definition 的 call_template；key 按
/// Redis 命令行习惯加引号转义，含空格/引号也能直接执行）
fn redis_read_command(key: &str, type_name: &str) -> String {
    let k = redis_quote_arg(key);
    match type_name {
        "hash" => format!("HGETALL {k}"),
        "list" => format!("LRANGE {k} 0 -1"),
        "set" => format!("SMEMBERS {k}"),
        "zset" => format!("ZRANGE {k} 0 -1 WITHSCORES"),
        "stream" => format!("XRANGE {k} - +"),
        _ => format!("GET {k}"),
    }
}

/// 命令行参数引号（与前端 redisQuote 同规则：含空白/引号/反斜杠时用双引号
/// 包裹并转义 `"` 与 `\`）
fn redis_quote_arg(value: &str) -> String {
    if value
        .chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '\'' || c == '\\')
    {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        value.to_string()
    }
}

/// key 内元素清单：返回 (元素角色, 行)。行形态统一两列 [field, value]，
/// 与 table_detail 的 columns 表头 ["field","value"] 对齐。
/// - string → [["value", <val>]]
/// - hash   → [["<field>", "<value>"]]（HGETALL 交替）
/// - list   → [["<index>", "<value>"]]（LRANGE 0..199）
/// - set    → [["<member>", ""]]（SMEMBERS 无序，索引无意义）
/// - zset   → [["<member>", "<score>"]]（ZRANGE WITHSCORES 交替）
/// - stream → [["<id>", "<fields json-ish>"]]（XRANGE - + COUNT 100）
///
/// 一律按 redis::Value 取值再走 redis_value_string：直接反序列化成
/// Vec<String> 时，任一元素不是合法 UTF-8（Redis 值允许任意字节）会让
/// **整个** 数组反序列化失败，结果退化成空表——比显示 `<N bytes>` 更糟。
async fn redis_key_entries(
    conn: &mut redis::aio::MultiplexedConnection,
    key: &str,
    type_name: &str,
) -> (&'static str, Vec<Vec<String>>) {
    match type_name {
        "string" => {
            let value: redis::Value = redis::cmd("GET")
                .arg(key)
                .query_async(conn)
                .await
                .unwrap_or(redis::Value::Nil);
            (
                "value",
                vec![vec!["value".to_string(), redis_value_text(&value)]],
            )
        }
        "hash" => {
            let mut cmd = redis::cmd("HGETALL");
            cmd.arg(key);
            let flat = redis_flat_strings(conn, &cmd).await;
            ("field", redis_pair_rows(flat, REDIS_ELEMENT_CAP))
        }
        "list" => {
            let mut cmd = redis::cmd("LRANGE");
            cmd.arg(key).arg(0).arg(REDIS_ELEMENT_CAP as i64 - 1);
            let items = redis_flat_strings(conn, &cmd).await;
            (
                "index",
                items
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| vec![i.to_string(), v])
                    .collect(),
            )
        }
        "set" => {
            let mut cmd = redis::cmd("SMEMBERS");
            cmd.arg(key);
            let members = redis_flat_strings(conn, &cmd).await;
            (
                "member",
                members
                    .into_iter()
                    .take(REDIS_ELEMENT_CAP)
                    .map(|m| vec![m, String::new()])
                    .collect(),
            )
        }
        "zset" => {
            let mut cmd = redis::cmd("ZRANGE");
            cmd.arg(key)
                .arg(0)
                .arg(REDIS_ELEMENT_CAP as i64 - 1)
                .arg("WITHSCORES");
            let flat = redis_flat_strings(conn, &cmd).await;
            ("member", redis_pair_rows(flat, REDIS_ELEMENT_CAP))
        }
        "stream" => {
            // XRANGE 返回 [[id, [field, value, ...]], ...]；字段序列化成
            // `field=value field=value` 的紧凑串（一条消息多字段，表格
            // 一行放不下结构化展示）
            let value: redis::Value = redis::cmd("XRANGE")
                .arg(key)
                .arg("-")
                .arg("+")
                .arg("COUNT")
                .arg(REDIS_STREAM_CAP)
                .query_async(conn)
                .await
                .unwrap_or(redis::Value::Nil);
            let redis::Value::Array(messages) = value else {
                return ("id", Vec::new());
            };
            let rows = messages
                .iter()
                .filter_map(|msg| {
                    let redis::Value::Array(parts) = msg else {
                        return None;
                    };
                    let id = parts.first().and_then(redis_value_string)?;
                    let fields = parts.get(1).and_then(redis_stream_fields)?;
                    Some(vec![id, fields])
                })
                .collect();
            ("id", rows)
        }
        // 未知/新类型（Redis 7.4 的 vector set 等）：无元素概念
        _ => ("element", Vec::new()),
    }
}

/// 扁平 [k1,v1,k2,v2,…] → [[k1,v1],[k2,v2],…]（HGETALL/ZRANGE WITHSCORES
/// 的交替形态）。奇数长度末尾多出的一列单独成行、value 留空。
fn redis_pair_rows(flat: Vec<String>, cap: usize) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut iter = flat.into_iter();
    while rows.len() < cap {
        let Some(first) = iter.next() else {
            break;
        };
        let second = iter.next().unwrap_or_default();
        rows.push(vec![first, second]);
    }
    rows
}

/// 执行一条返回数组的命令，把元素逐个转成文本（跳过无法表达的项）。
/// 不走 Vec<String> 反序列化：任一元素非 UTF-8 会让整批失败（见
/// redis_key_entries 的说明）；这里只丢掉不可表达的那一项。
async fn redis_flat_strings(
    conn: &mut redis::aio::MultiplexedConnection,
    cmd: &redis::Cmd,
) -> Vec<String> {
    let value: redis::Value = match cmd.query_async(conn).await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    match value {
        redis::Value::Array(items) | redis::Value::Set(items) => {
            items.iter().filter_map(redis_value_string).collect()
        }
        // RESP3 的 HGETALL 是 Map：按 field/value 展开成扁平序列，
        // 保持与 RESP2 交替数组同形（调用方 redis_pair_rows 无需分叉）
        redis::Value::Map(pairs) => {
            let mut flat = Vec::with_capacity(pairs.len() * 2);
            for (k, v) in &pairs {
                if let Some(k) = redis_value_string(k) {
                    flat.push(k);
                }
                if let Some(v) = redis_value_string(v) {
                    flat.push(v);
                }
            }
            flat
        }
        redis::Value::Nil => Vec::new(),
        other => redis_value_string(&other).into_iter().collect(),
    }
}

/// 单个值 → 展示文本（nil 渲染成 "(nil)"，其余同 redis_value_string）
fn redis_value_text(value: &redis::Value) -> String {
    redis_value_string(value).unwrap_or_else(|| "(nil)".to_string())
}

/// stream 消息的字段数组 [f1,v1,f2,v2,…] → "f1=v1 f2=v2"
fn redis_stream_fields(value: &redis::Value) -> Option<String> {
    let redis::Value::Array(fields) = value else {
        return None;
    };
    let pairs: Vec<String> = fields
        .chunks(2)
        .filter_map(|pair| {
            let name = pair.first().and_then(redis_value_string)?;
            let val = pair.get(1).and_then(redis_value_string).unwrap_or_default();
            Some(format!("{name}={val}"))
        })
        .collect();
    Some(pairs.join(" "))
}

/// 单个 redis::Value → 字符串（None 表示该值无法表达成文本，调用方跳过）。
/// BulkString 非法 UTF-8（二进制值，Redis 允许任意字节）渲染为 `<N bytes>`
/// ——直接 lossy 会把二进制内容替换成一片 U+FFFD，反而不可读。
fn redis_value_string(value: &redis::Value) -> Option<String> {
    Some(match value {
        redis::Value::Nil => return None,
        redis::Value::Int(i) => i.to_string(),
        redis::Value::BulkString(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => text.to_string(),
            Err(_) => format!("<{} bytes>", bytes.len()),
        },
        redis::Value::SimpleString(s) => s.clone(),
        redis::Value::VerbatimString { text, .. } => text.clone(),
        redis::Value::Double(f) => format_float(*f),
        redis::Value::Boolean(b) => b.to_string(),
        redis::Value::Okay => "OK".to_string(),
        redis::Value::BigNumber(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        redis::Value::ServerError(err) => err.to_string(),
        // 嵌套结构（Array/Map/Set/Push/Attribute）在单值位置少见，退化为
        // 结构摘要；具体展开由 redis_value_to_table 负责
        other => format!("{other:?}"),
    })
}

/// 命令名 → 其 Array 回复是否为 field/value 交替形态。
/// RESP2 下只有极少数命令用「扁平交替数组」表达键值对：HGETALL 是典型，
/// CONFIG GET 与 XINFO 同型。刻意只收这三个——把 CLIENT/SLOWLOG/COMMAND
/// 之类一并算进来会把它们「偶数长度的普通列表」误读成键值对（如
/// COMMAND COUNT 之外的结果），宁可平铺成 index/value 也不误配。
fn redis_pairs_by_command(command: &str) -> bool {
    matches!(command, "HGETALL" | "CONFIG" | "XINFO")
}

/// 单元格渲染：嵌套结构压成一行紧凑文本（表格里展开多层不可读）。
/// 刻意不做「偶数长度数组按 k=v 配对」的猜测——MGET/LRANGE 的两元素结果
/// 会被误读成键值对；只有明确知道是字段组的调用方（XRANGE 消息体，见
/// redis_stream_fields）才做配对。
fn redis_cell(value: &redis::Value) -> String {
    match value {
        redis::Value::Array(items) | redis::Value::Set(items) => {
            let parts: Vec<String> = items.iter().map(redis_cell).collect();
            format!("[{}]", parts.join(", "))
        }
        redis::Value::Map(pairs) => {
            let parts: Vec<String> = pairs
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}={}",
                        redis_value_string(k).unwrap_or_else(|| "(nil)".to_string()),
                        redis_cell(v)
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        redis::Value::Attribute { data, .. } => redis_cell(data),
        other => redis_value_text(other),
    }
}

/// redis::Value → (列头, 行)。值形态与命令名共同决定表格形状：
/// - 标量（Nil/Int/BulkString/SimpleString/Verbatim/Double/Boolean/Okay/
///   BigNumber）→ 单列 ["result"]、单行
/// - Array：全元素都是二元素数组 → 两列（RESP3 的 HGETALL 形态、XRANGE 的
///   [id, fields] 形态）；命令属交替对命令（RESP2 的 HGETALL/CONFIG GET）
///   → 两两配对成 [field, value]；否则平铺成 ["index","value"] 每项一行
/// - Map → ["key","value"]；Set → ["member"]；Push 按 Array；
///   Attribute 拆掉包装取 data
fn redis_value_to_table(value: &redis::Value, command: &str) -> (Vec<String>, Vec<Vec<String>>) {
    match value {
        redis::Value::Nil => single_result("(nil)"),
        redis::Value::Int(i) => single_result(&i.to_string()),
        redis::Value::BulkString(_)
        | redis::Value::SimpleString(_)
        | redis::Value::VerbatimString { .. }
        | redis::Value::Double(_)
        | redis::Value::Boolean(_)
        | redis::Value::Okay
        | redis::Value::BigNumber(_) => single_result(&redis_cell(value)),
        redis::Value::ServerError(err) => single_result(&err.to_string()),
        redis::Value::Array(items) => redis_array_table(items, command),
        redis::Value::Push { data, .. } => redis_array_table(data, command),
        redis::Value::Set(items) => (
            vec!["member".to_string()],
            items.iter().map(redis_cell).map(|m| vec![m]).collect(),
        ),
        redis::Value::Map(pairs) => (
            vec!["key".to_string(), "value".to_string()],
            pairs
                .iter()
                .map(|(k, v)| vec![redis_cell(k), redis_cell(v)])
                .collect(),
        ),
        redis::Value::Attribute { data, .. } => redis_value_to_table(data, command),
        // non_exhaustive：未来新增的值类型退化为 Debug 单值，不让编译
        // 期漏臂变成运行期 panic
        other => single_result(&format!("{other:?}")),
    }
}

/// Array 回复的表格化（RESP2/RESP3 两种形态都兼容）
fn redis_array_table(items: &[redis::Value], command: &str) -> (Vec<String>, Vec<Vec<String>>) {
    if items.is_empty() {
        // 空数组（如 SMEMBERS 空集）：保留单列头，前端显示「0 行」
        return (vec!["value".to_string()], Vec::new());
    }
    // 全元素都是二元素数组：键值对形态（RESP3 HGETALL / XRANGE [id, fields]）
    if items
        .iter()
        .all(|item| matches!(item, redis::Value::Array(pair) if pair.len() == 2))
    {
        // XRANGE 的两元素是 [id, [field, value…]]，用 id/fields 表头更准确，
        // 且字段组按 field=value 配对（其余二元素数组的语义未知，用 redis_cell）
        let is_stream_range = matches!(command, "XRANGE" | "XREVRANGE");
        let headers = if is_stream_range {
            vec!["id".to_string(), "fields".to_string()]
        } else {
            vec!["field".to_string(), "value".to_string()]
        };
        let rows = items
            .iter()
            .filter_map(|item| {
                let redis::Value::Array(pair) = item else {
                    return None;
                };
                let second = match pair.get(1) {
                    Some(fields) if is_stream_range => {
                        redis_stream_fields(fields).unwrap_or_else(|| redis_cell(fields))
                    }
                    Some(other) => redis_cell(other),
                    None => String::new(),
                };
                Some(vec![
                    pair.first().map(redis_cell).unwrap_or_default(),
                    second,
                ])
            })
            .collect();
        return (headers, rows);
    }
    if redis_pairs_by_command(command) && items.len().is_multiple_of(2) {
        let flat: Vec<String> = items.iter().map(redis_cell).collect();
        return (
            vec!["field".to_string(), "value".to_string()],
            redis_pair_rows(flat, usize::MAX),
        );
    }
    // 普通列表：索引 + 值。元素本身是数组时（XRANGE/嵌套结果）渲染成
    // 单行摘要，避免递归展开把表格撑成不可读的深层结构
    (
        vec!["index".to_string(), "value".to_string()],
        items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let text = match item {
                    redis::Value::Array(nested) => {
                        let parts: Vec<String> = nested
                            .iter()
                            .map(|v| redis_value_string(v).unwrap_or_default())
                            .collect();
                        format!("[{}]", parts.join(", "))
                    }
                    other => redis_value_string(other).unwrap_or_else(|| "(nil)".to_string()),
                };
                vec![i.to_string(), text]
            })
            .collect(),
    )
}

/// 单列 ["result"] + 单行
fn single_result(value: &str) -> (Vec<String>, Vec<Vec<String>>) {
    (
        vec!["result".to_string()],
        vec![vec![value.to_string()]],
    )
}

/// Redis 命令行分词（对齐 redis-cli 的 sdssplitargs 语义）：
/// - 空白分隔参数；`"` / `'` 开始引号段（段内可含空白），引号本身不保留
/// - **引号外的反斜杠是普通字符**（redis-cli 同此）——否则未加引号的
///   `SET k C:\path\new` 会被吃成 `C:path` + 换行，Windows 路径必踩
/// - 双引号内：`\n` `\r` `\t` `\b` `\a` 按 C 风格解释，其余 `\x` 取 x 本身
///   （`\"` 与 `\\` 因此自然得到引号与反斜杠）
/// - 单引号内：只有 `\'` 是转义，其余反斜杠原样保留（redis-cli 同此）
/// - 未闭合引号按「到行尾」处理：工作台是随手敲命令的地方，半截输入
///   应当照常发出并由服务端裁决，而不是在本地报语法错
fn redis_split_args(input: &str) -> Vec<String> {
    #[derive(Clone, Copy, PartialEq)]
    enum Quote {
        None,
        Double,
        Single,
    }
    let mut args: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_arg = false;
    let mut quote = Quote::None;
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        match quote {
            Quote::Double => {
                if c == '"' {
                    quote = Quote::None;
                } else if c == '\\' {
                    match chars.next() {
                        Some('n') => current.push('\n'),
                        Some('r') => current.push('\r'),
                        Some('t') => current.push('\t'),
                        Some('b') => current.push('\u{8}'),
                        Some('a') => current.push('\u{7}'),
                        Some(other) => current.push(other),
                        None => current.push('\\'),
                    }
                } else {
                    current.push(c);
                }
            }
            Quote::Single => {
                if c == '\'' {
                    quote = Quote::None;
                } else if c == '\\' && matches!(chars.clone().next(), Some('\'')) {
                    // 单引号内只有 \' 是转义（redis-cli 同此）；其余反斜杠
                    // 原样保留，故不能像双引号那样吃掉下一个字符
                    let _ = chars.next();
                    current.push('\'');
                } else {
                    current.push(c);
                }
            }
            Quote::None => match c {
                '"' => {
                    quote = Quote::Double;
                    in_arg = true;
                }
                '\'' => {
                    quote = Quote::Single;
                    in_arg = true;
                }
                c if c.is_whitespace() => {
                    if in_arg {
                        args.push(std::mem::take(&mut current));
                        in_arg = false;
                    }
                }
                _ => {
                    current.push(c);
                    in_arg = true;
                }
            },
        }
    }
    if in_arg {
        args.push(current);
    }
    args
}

// ── TLS 构造 ──────────────────────────────────────────────────────────────

/// 按计数生成分组节点（计数为 0 的组不返回）；key 形如 `{schema}#{group}`，
/// 前端展开时从 key 解析 schema 与分组类型。
fn group_nodes(
    schema: &str,
    tables: i64,
    views: i64,
    functions: i64,
    procedures: i64,
    sequences: i64,
) -> Vec<CatalogNode> {
    [
        (ObjectGroup::Table, "TABLE", tables),
        (ObjectGroup::View, "VIEW", views),
        (ObjectGroup::Function, "FUNCTION", functions),
        (ObjectGroup::Procedure, "PROCEDURE", procedures),
        (ObjectGroup::Sequence, "SEQUENCE", sequences),
    ]
    .iter()
    .filter(|(_, _, count)| *count > 0)
    .map(|(group, name, count)| CatalogNode {
        key: format!("{schema}#{}", group.as_str()),
        name: name.to_string(),
        kind: "group".to_string(),
        detail: Some(count.to_string()),
    })
    .collect()
}

/// 连接任务泛型 helper：不同 TLS 流类型（NoTlsStream / RustlsStream）经
/// dyn Future 擦除后统一拉起后台驱动。
fn spawn_pg_connection<C>(
    connection: tokio_postgres::Connection<tokio_postgres::Socket, C>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
where
    C: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    Box::pin(async move {
        if let Err(e) = connection.await {
            log::debug!("[DB] postgres connection task ended: {e}");
        }
    })
}

fn mysql_ssl_opts(mode: &str) -> Result<Option<mysql_async::SslOpts>, String> {
    match mode {
        "disable" => Ok(None),
        "prefer" | "require" => Ok(Some(
            mysql_async::SslOpts::default().with_danger_accept_invalid_certs(true),
        )),
        "verify-full" => {
            let pem = native_roots_pem()?;
            // PathOrBuf 未从 crate root 导出，经 collect 的目标类型推断构造
            let certs: Vec<_> = std::iter::once(pem.into_bytes())
                .map(std::convert::Into::into)
                .collect();
            Ok(Some(mysql_async::SslOpts::default().with_root_certs(certs)))
        }
        other => Err(format!("无效的 SSL 模式：{other}")),
    }
}

fn pg_tls_connector(verify: bool) -> Result<tokio_postgres_rustls::MakeRustlsConnect, String> {
    // 显式 aws-lc-rs provider（builder() 无参版走进程默认探测，依赖图双
    // provider 并存时会 panic——见 lib.rs 启动处的 install_default 注释）
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("rustls 协议版本协商失败：{e}"))?;
    let config = if verify {
        let mut roots = rustls::RootCertStore::empty();
        let native = rustls_native_certs::load_native_certs();
        for cert in native.certs {
            let _ = roots.add(cert);
        }
        builder.with_root_certificates(roots).with_no_client_auth()
    } else {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert))
            .with_no_client_auth()
    };
    Ok(tokio_postgres_rustls::MakeRustlsConnect::new(config))
}

/// require 模式的全放行证书校验器（仍校验握手签名，只跳过证书链验证）。
#[derive(Debug)]
struct AcceptAnyServerCert;

impl rustls::client::danger::ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::aws_lc_rs::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::aws_lc_rs::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// 系统根证书 → PEM 文本（mysql_async 的 Ca::Inline 吃 PEM 字节）。
fn native_roots_pem() -> Result<String, String> {
    let native = rustls_native_certs::load_native_certs();
    let mut pem = String::new();
    for cert in native.certs {
        let b64 = base64::engine::general_purpose::STANDARD.encode(cert.as_ref());
        let mut rest = b64.as_str();
        while rest.len() > 64 {
            let (line, tail) = rest.split_at(64);
            pem.push_str(line);
            pem.push('\n');
            rest = tail;
        }
        pem.push_str(rest);
        pem.push('\n');
    }
    if pem.is_empty() {
        return Err("未找到系统根证书（verify-full 模式需要）".to_string());
    }
    Ok(format!(
        "-----BEGIN CERTIFICATE-----\n{pem}-----END CERTIFICATE-----\n"
    ))
}

// ── 值字符串化（与 sqlite_viewer 的展示语义对齐：NULL/BLOB 占位、整数去 .0）──

/// 读 Oracle 客户端库目录设置（app_kv；读失败按未设置处理，不阻塞连接）
fn oracle_client_dir() -> Result<Option<String>, String> {
    let Some(pool) = crate::db::global_pool() else {
        return Ok(None);
    };
    let conn = pool.get().map_err(|e| e.to_string())?;
    let dir: Option<String> = conn
        .query_row(
            "SELECT value FROM app_kv WHERE key = 'oracle_client_lib_dir'",
            [],
            |row| row.get(0),
        )
        .ok();
    Ok(dir.filter(|s| !s.is_empty()))
}

/// oracle::Error → 展示文案。DPI-1047 = Oracle 客户端库缺失（运行期依赖
/// Instant Client），追加安装提示——这是 rust-oracle 形态下最常见的环境错。
/// 缺消息文件形态（macOS 包现实，见 oracle_missing_message_hint）先兜底。
fn oracle_err(e: oracle::Error) -> String {
    if let Some(mapped) = oracle_missing_message_hint(&e) {
        return mapped;
    }
    let msg = e.to_string();
    if msg.contains("DPI-1047") {
        format!(
            "{msg}（未找到 Oracle 客户端库：请安装 Oracle Instant Client 并在\
             「数据库」页设置客户端目录；也可依赖系统搜索路径）"
        )
    } else {
        msg
    }
}

/// macOS Instant Client 不随包提供 ORA 消息文件（.msb，包内只有 dylib；
/// Linux basic 包才有 oraus.msb），OCI 本地查文案失败时给出
/// "Message NNN not found; No message file ..."。错误码本体在 DbError
/// 里始终可得，用内置常见码表 + 官方文档链接兜底成可读信息。
fn oracle_missing_message_hint(e: &oracle::Error) -> Option<String> {
    let db = e.db_error()?;
    if !db.message().contains("No message file") {
        return None;
    }
    let code = db.code();
    let hint = oracle_ora_hint(code)
        .map(|h| format!("：{h}"))
        .unwrap_or_default();
    // "arguments: [...]" 段有真实定位价值（如 904 的出错标识符），保留
    let args = db
        .message()
        .find("arguments: [")
        .and_then(|i| db.message().get(i + "arguments: [".len()..))
        .and_then(|s| s.strip_suffix(']'))
        .filter(|s| !s.is_empty())
        .map(|s| format!("，参数 {s}"))
        .unwrap_or_default();
    Some(format!(
        "ORA-{code:05}{hint}{args}（客户端未随包提供错误文案文件，详见 \
         https://docs.oracle.com/error-help/db/ora-{code:05}/）"
    ))
}

/// 常见 ORA 错误码内置文案（仅覆盖本应用高频路径：连接/认证/SQL 执行）。
/// 未收录的码在 oracle_err 里回落为纯码 + 官方文档链接，不会误导。
fn oracle_ora_hint(code: i32) -> Option<&'static str> {
    Some(match code {
        1 => "违反唯一约束",
        1013 => "用户取消操作",
        1017 => "用户名或密码无效",
        1033 => "Oracle 正在启动或关闭中",
        1034 => "Oracle 不可用（实例未启动）",
        1045 => "用户缺少 CREATE SESSION 权限",
        3113 => "通信通道文件结束（连接被服务端断开）",
        3114 => "未连接到 Oracle",
        904 => "标识符无效",
        911 => "无效字符（常见：多余分号或中文标点）",
        918 => "列名有歧义（多表同名列未限定）",
        920 => "无效的关系运算符",
        923 => "未找到 FROM 关键字（常见：SELECT 列表缺逗号）",
        933 => "SQL 命令未正确结束（常见：末尾多余分号）",
        936 => "缺少表达式",
        942 => "表或视图不存在",
        1722 => "无效数字",
        1400 => "无法插入 NULL（非空列缺值）",
        1403 => "未找到数据",
        2292 => "违反完整性约束（存在引用该行的子记录）",
        12154 => "无法解析连接标识符（检查服务名）",
        12170 => "连接超时",
        12505 => "监听器不认识该 SID/服务（检查服务名）",
        12514 => "监听器当前不认识请求的服务（检查服务名是否正确）",
        12541 => "无监听器（目标端口未监听）",
        12543 => "目标主机不可达",
        12545 => "连接失败（目标主机不存在）",
        12899 => "列值过大（超出字段长度）",
        28000 => "账户已被锁定",
        28001 => "密码已过期",
        28002 => "密码即将过期",
        _ => return None,
    })
}

/// 用户（schema）清单，按服务器版本适配 SQL：ORACLE_MAINTAINED 列是
/// 12.1 才加的（11.2 及更早查询直接 ORA-00904），故 <12 走内置账号黑名单
/// 加 APEX_/FLOWS_ 前缀排除。server_version 走 ODPI-C 连接内缓存
/// （connect 时已取过，此处零额外往返）。
fn oracle_usernames(conn: &oracle::Connection) -> Result<Vec<String>, String> {
    let (version, _) = conn
        .server_version()
        .map_err(|e| format!("Oracle 获取服务器版本失败：{}", oracle_err(e)))?;
    let sql = if version.major() >= 12 {
        "SELECT username FROM all_users WHERE oracle_maintained = 'N' ORDER BY username"
    } else {
        "SELECT username FROM all_users WHERE username NOT IN \
         ('SYS','SYSTEM','OUTLN','DIP','ORACLE_OCM','DBSNMP','APPQOSSYS','WMSYS',\
          'EXFSYS','CTXSYS','XDB','ANONYMOUS','XS$NULL','MDDATA','MDSYS','OLAPSYS',\
          'ORDDATA','ORDPLUGINS','ORDSYS','SI_INFORMTN_SCHEMA','LBACSYS','OWBSYS',\
          'OWBSYS_AUDIT','SPATIAL_CSW_ADMIN','SPATIAL_WFS_ADMIN','SYSMAN',\
          'MGMT_VIEW','FLOWS_FILES') \
         AND username NOT LIKE 'APEX\\_%' ESCAPE '\\' \
         AND username NOT LIKE 'FLOWS\\_%' ESCAPE '\\' \
         ORDER BY username"
    };
    let rs = conn
        .query(sql, &[])
        .map_err(|e| format!("Oracle 查询用户清单失败：{}", oracle_err(e)))?;
    Ok(rs
        .filter_map(|r| r.ok())
        .map(|row| oracle_row_string(&row, 0))
        .collect())
}

/// Row 按列取字符串（SqlValue 的 Display 内置 NULL 处理与各类型形态；越界列回 NULL）
fn oracle_row_string(row: &oracle::Row, i: usize) -> String {
    row.sql_values()
        .get(i)
        .map(|v| v.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

/// 逐行流式发射一个 Oracle 结果集：批量回调（ROWS_PER_BATCH 粒度）、
/// max_rows 截断（truncated 探测一行）。columns 未初始化时从结果集补齐
/// 并回调（PL/SQL 隐式结果集路径复用）
fn emit_oracle_rows<F1, F2>(
    rs: &mut oracle::ResultSet<oracle::Row>,
    max_rows: usize,
    on_columns: &mut F1,
    on_rows: &mut F2,
    summary: &mut ExecSummary,
) where
    F1: FnMut(&[String]),
    F2: FnMut(Vec<Vec<String>>),
{
    if summary.columns.is_empty() {
        summary.columns = rs
            .column_info()
            .iter()
            .map(|c| c.name().to_string())
            .collect();
        on_columns(&summary.columns);
    }
    let mut batch: Vec<Vec<String>> = Vec::with_capacity(ROWS_PER_BATCH);
    let mut maybe_more = false;
    for row_r in &mut *rs {
        if summary.rows_read >= max_rows {
            maybe_more = true;
            break;
        }
        let row = match row_r {
            Ok(r) => r,
            Err(e) => {
                // 行级错误不中断：已收行照常发射，错误以截断形式呈现——
                // 单元格取值失败置 NULL（SqlValue Display 已兜底）
                let _ = e;
                continue;
            }
        };
        let cells: Vec<String> = row.sql_values().iter().map(|v| v.to_string()).collect();
        batch.push(cells);
        summary.rows_read += 1;
        if batch.len() >= ROWS_PER_BATCH {
            on_rows(std::mem::take(&mut batch));
        }
    }
    if !batch.is_empty() {
        on_rows(batch);
    }
    if maybe_more {
        summary.truncated = true;
    }
}

fn format_float(v: f64) -> String {
    if v.fract() == 0.0 && v.is_finite() && v.abs() < 1e15 {
        // 整数值去掉 .0（与 sqlite_viewer 一致）
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn mysql_value_to_string(value: &mysql_async::Value) -> String {
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

/// PG 单元格取值：类型驱动的多级降级链（Option 统一处理 NULL）。
fn pg_cell(row: &tokio_postgres::Row, i: usize) -> String {
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
        return v
            .map(|x| x.to_string())
            .unwrap_or_else(|| "NULL".into());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 常见 ORA 码表边界：收录码有文案、未收录码回落 None（连库行为
    /// 无法在无客户端环境测试，这里只锁纯函数映射）
    #[test]
    fn oracle_ora_hint_known_and_unknown_codes() {
        assert_eq!(oracle_ora_hint(904), Some("标识符无效"));
        assert_eq!(oracle_ora_hint(923), Some("未找到 FROM 关键字（常见：SELECT 列表缺逗号）"));
        assert_eq!(oracle_ora_hint(942), Some("表或视图不存在"));
        assert_eq!(oracle_ora_hint(1017), Some("用户名或密码无效"));
        assert_eq!(oracle_ora_hint(28000), Some("账户已被锁定"));
        assert_eq!(oracle_ora_hint(99999), None);
        assert_eq!(oracle_ora_hint(0), None);
    }
}
