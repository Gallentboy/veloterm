//! 各引擎建连逻辑（`DbEngine::connect` 的实现体 + TLS 辅助）。
//! 拆自 engines/mod.rs：连接管理与引擎操作两类职责分离。
//! 各 `connect_*` 为自由函数，由 `DbEngine::connect` 分派；与拆分前逐行一致。

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use base64::Engine as _;
use oracle_async::Queryable as _;

use super::{
    DbConnectionRecord, DbEngine, RedisConn, RedisExtraOptions, SqliteEngine, dm_server_version, dm_valid_identifier,
    redis_addr, redis_connect_error, redis_connection_info, redis_db_index,
    oracle_err, redis_extra_options, redis_node_list, redis_tls_certs, redis_value_string,
    redis_version,
};

/// 建连并取服务端版本描述。OceanBase 按 mode 分双模式：MySQL 模式复用
/// mysql_async 臂（engine 变体仍是 MySql），Oracle 模式固定走纯 Rust
/// 分叉驱动（`connect_ob_rust`，结果同样落在 MySql 变体）。
pub(crate) async fn connect(record: &DbConnectionRecord) -> Result<(DbEngine, String), String> {
    match (record.engine.as_str(), record.mode.as_deref()) {
        ("mysql", _) => connect_mysql(record).await,
        ("postgres", _) => connect_postgres(record).await,
        ("oracle", _) => connect_oracle(record).await,
        ("redis", _) => connect_redis(record).await,
        ("oceanbase", Some("mysql")) => connect_ob_mysql(record).await,
        ("oceanbase", Some("oracle")) => connect_ob_rust(record).await,
        ("oceanbase", _) => Err("OceanBase 需要选择租户模式（MySQL/Oracle）".to_string()),
        ("sqlite", _) => connect_sqlite(record).await,
        ("dameng", _) => connect_dameng(record).await,
        (other, _) => Err(format!("不支持的数据库引擎：{other}")),
    }
}

/// mysql_async 建连（mysql 与 OceanBase MySQL 模式共用：builder 构造 +
/// 12s 超时 + TLS 映射）。
pub(crate) async fn mysql_conn(
    record: &DbConnectionRecord,
    ob_mode: Option<mysql_async::ObCompatMode>,
) -> Result<mysql_async::Conn, String> {
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
    // OB 租户兼容模式由连接方声明（驱动无法从线缆区分 OB MySQL 租户与
    // 原生 MySQL）；原生 MySQL 传 None，目录查询保持原口径不变
    builder = builder.ob_compat_mode(ob_mode);
    let conn = tokio::time::timeout(Duration::from_secs(12), mysql_async::Conn::new(builder))
        .await
        .map_err(|_| "MySQL 连接超时（12s）".to_string())?
        .map_err(|e| format!("MySQL 连接失败：{e}"))?;
    Ok(conn)
}

pub(crate) async fn connect_mysql(record: &DbConnectionRecord) -> Result<(DbEngine, String), String> {
    let conn = mysql_conn(record, None).await?;
    let (major, minor, patch) = conn.server_version();
    Ok((
        DbEngine::MySql(conn),
        format!("MySQL {major}.{minor}.{patch}"),
    ))
}

/// OB MySQL 模式：标准 MySQL 线缆协议，复用 mysql_async 建连；差异仅两处
///——连接后 SET NAMES utf8mb4（显式统一字符集，不随服务端缺省漂移）、
/// 版本串直显 SELECT version() 原文（"5.7.25-OceanBase-…" 含发行版信息；
/// 查询失败回退 server_version 拼法）。
pub(crate) async fn connect_ob_mysql(record: &DbConnectionRecord) -> Result<(DbEngine, String), String> {
    let mut conn =
        mysql_conn(record, Some(mysql_async::ObCompatMode::MySql)).await?;
    use mysql_async::prelude::Queryable as _;
    conn.query_drop("SET NAMES utf8mb4")
        .await
        .map_err(|e| format!("OceanBase 设置字符集失败：{e}"))?;
    let version = match conn
        .query_first::<String, _>("SELECT version()")
        .await
    {
        Ok(Some(v)) if !v.is_empty() => v,
        _ => {
            let (major, minor, patch) = conn.server_version();
            format!("OceanBase（MySQL 模式）{major}.{minor}.{patch}")
        }
    };
    Ok((DbEngine::MySql(conn), version))
}

pub(crate) async fn connect_postgres(record: &DbConnectionRecord) -> Result<(DbEngine, String), String> {
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

pub(crate) async fn connect_oracle(record: &DbConnectionRecord) -> Result<(DbEngine, String), String> {
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

    // 纯 Rust 驱动未实现 TCPS：非 disable 直接报错，不做静默明文回退
    // （同 OB Oracle 租户模式的 P0 约定）。前端 Oracle 表单已隐藏 SSL
    // 选项，此处拦截的是历史记录里残留的旧档位。
    if record.ssl_mode != "disable" {
        return Err(
            "Oracle 连接暂不支持 TLS/TCPS（纯 Rust 驱动未实现），请把 SSL 模式改为「不加密」"
                .to_string(),
        );
    }

    let opts = oracle_async::Opts::builder()
        .host(host)
        .port(record.port)
        .service_name(service)
        .username(username)
        .password(password)
        .build();

    let mut conn = oracle_async::Conn::connect(opts)
        .await
        .map_err(|e| format!("Oracle 连接失败：{}", oracle_err(e)))?;

    // 版本展示与原生 OCI / go-ora 严格一致：用 0x3B 回包的完整 banner
    // （如 "Oracle Database 11g Enterprise Edition Release …"），
    // 兜底数字版本号；TTC 协商串只是平台尾巴不可用
    let version = {
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
    };

    // 初始 schema（database 字段，可选）：CURRENT_SCHEMA 只改名字解析
    // 不改权限；标识符字符集校验同 select_database（防注入）
    if let Some(schema) = initial_schema.as_deref().filter(|s| !s.trim().is_empty()) {
        if !schema
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '#'))
        {
            return Err("Oracle 无效的 schema 名".to_string());
        }
        let sql = format!("ALTER SESSION SET CURRENT_SCHEMA = {schema}");
        conn.exec(&sql)
            .await
            .map_err(|e| format!("Oracle 设置初始 schema 失败：{}", oracle_err(e)))?;
    }

    Ok((DbEngine::Oracle(conn), version))
}

/// Redis 建连：按 extra_options.topology 分单机 / 集群 / 哨兵三条路径。
/// 地址一律直接构造 ConnectionInfo（不经 URL 解析——密码/用户名里的
/// `@ : / ? #` 等字符会被 URL 语法吃掉，这是 redis-rs 的
/// `open("redis://…")` 路径最常见的踩坑点）。
pub(crate) async fn connect_redis(record: &DbConnectionRecord) -> Result<(DbEngine, String), String> {
    let extra = redis_extra_options(record.extra_options.as_deref());
    match extra.topology.as_str() {
        "cluster" => connect_redis_cluster(record, &extra).await,
        "sentinel" => connect_redis_sentinel(record, &extra).await,
        _ => connect_redis_single(record, &extra).await,
    }
}

/// 单机拓扑（默认；老记录/无 extra_options 同此）
pub(crate) async fn connect_redis_single(
    record: &DbConnectionRecord,
    extra: &RedisExtraOptions,
) -> Result<(DbEngine, String), String> {
    let host = record.host.trim().to_string();
    connect_redis_endpoint(record, extra, &host, record.port).await
}

/// 对单个端点建连（单机与哨兵解析出的 master 共用）。
/// TLS 四档：disable 明文；prefer/require 加密但不校验证书（语义与
/// MySQL/PG 臂一致：绝不回退明文）；verify-full 走系统根证书。
/// redis-rs 的 insecure=true 需要 tls-rustls-insecure feature（Cargo.toml
/// 已开），否则连接时直接报 InvalidClientConfig。mTLS 证书存在时经
/// Client::build_with_tls 挂进地址（TlsConnParams 不对外构造，这是唯一
/// 通道），要求 SSL 档位非 disable。
pub(crate) async fn connect_redis_endpoint(
    record: &DbConnectionRecord,
    extra: &RedisExtraOptions,
    host: &str,
    port: u16,
) -> Result<(DbEngine, String), String> {
    let addr = redis_addr(host, port, &record.ssl_mode);
    let db = redis_db_index(record.database.as_deref());
    let info = redis_connection_info(addr, record, db, true)?;
    let certs = redis_tls_certs(extra)?;
    let client = match certs {
        Some(certs) => {
            if record.ssl_mode == "disable" {
                return Err(
                    "Redis mTLS 需要将 SSL 设为 require/verify-full（disable 无法携带客户端证书）"
                        .to_string(),
                );
            }
            redis::Client::build_with_tls(info, certs)
        }
        None => redis::Client::open(info),
    }
    .map_err(|e| redis_connect_error(&e.to_string()))?;
    let conn = tokio::time::timeout(
        Duration::from_secs(12),
        client.get_multiplexed_async_connection(),
    )
    .await
    .map_err(|_| "Redis 连接超时（12s）".to_string())?
    .map_err(|e| redis_connect_error(&e.to_string()))?;
    // 版本探测：INFO server 的 redis_version 行；失败不影响连接
    let mut rconn = RedisConn::Single(conn);
    let version = redis_version(&mut rconn)
        .await
        .unwrap_or_else(|| "Redis".to_string());
    Ok((
        DbEngine::Redis {
            conn: rconn,
            db,
            cluster: false,
        },
        version,
    ))
}

/// 集群拓扑：extra_options.nodes 全部作为初始节点交给 ClusterClient
///（MOVED/ASK 重定向与按 slot 路由由 cluster-async 内部处理）；只认 db0。
/// SCAN/CONFIG 这类无 key 命令只路由到单个节点——对象树/大 key 分析在
/// 集群下是「单节点口径」，键操作命令则全集群正确路由（文档边界）。
pub(crate) async fn connect_redis_cluster(
    record: &DbConnectionRecord,
    extra: &RedisExtraOptions,
) -> Result<(DbEngine, String), String> {
    let nodes = redis_node_list(extra)?;
    let certs = redis_tls_certs(extra)?;
    let mut infos = Vec::with_capacity(nodes.len());
    for (host, port) in nodes {
        let addr = redis_addr(&host, port, &record.ssl_mode);
        let info = redis_connection_info(addr, record, 0, true)?;
        // mTLS：先经单机 build_with_tls 把 TlsConnParams 挂进地址，再取回
        // ConnectionInfo 喂给 ClusterClient（集群侧无独立的证书入口）
        let info = match &certs {
            Some(certs) if record.ssl_mode != "disable" => {
                redis::Client::build_with_tls(info, certs.clone())
                    .map_err(|e| redis_connect_error(&e.to_string()))?
                    .get_connection_info()
                    .clone()
            }
            Some(_) => {
                return Err(
                    "Redis mTLS 需要将 SSL 设为 require/verify-full（disable 无法携带客户端证书）"
                        .to_string(),
                );
            }
            None => info,
        };
        infos.push(info);
    }
    let client = redis::cluster::ClusterClient::new(infos)
        .map_err(|e| redis_connect_error(&e.to_string()))?;
    let conn = tokio::time::timeout(
        Duration::from_secs(15),
        client.get_async_connection(),
    )
    .await
    .map_err(|_| "Redis Cluster 连接超时（15s，含 slot 表发现）".to_string())?
    .map_err(|e| redis_connect_error(&e.to_string()))?;
    let mut rconn = RedisConn::Cluster(conn);
    let version = redis_version(&mut rconn)
        .await
        .unwrap_or_else(|| "Redis Cluster".to_string());
    Ok((
        DbEngine::Redis {
            conn: rconn,
            db: 0,
            cluster: true,
        },
        version,
    ))
}

/// 哨兵拓扑：逐个哨兵节点问 SENTINEL get-master-addr-by-name 解析当前
/// master 地址，先答先得；解析成功后按单机拓扑连 master（TLS 档位与
/// 凭据沿用连接记录）。主从切换后重连即自动指向新 master（不做运行期
/// 跟踪）。哨兵通常无认证：带凭据连失败时再按无凭据重试一轮（master
/// 的认证始终用连接记录的凭据）。
pub(crate) async fn connect_redis_sentinel(
    record: &DbConnectionRecord,
    extra: &RedisExtraOptions,
) -> Result<(DbEngine, String), String> {
    let master_name = extra
        .master_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "Redis 哨兵拓扑需要提供 master 名称（extra_options.masterName）".to_string()
        })?;
    let nodes = redis_node_list(extra)?;
    let has_password = record
        .password
        .as_deref()
        .map(|p| !p.trim().is_empty())
        .unwrap_or(false);
    let mut last_err = "没有可用的哨兵节点".to_string();
    for (host, port) in nodes {
        let addr = redis_addr(&host, port, &record.ssl_mode);
        // 两轮尝试：带记录凭据（哨兵配了 ACL）→ 无凭据（哨兵裸奔、
        // master 才有密码的常见部署）。密码为空只试一轮。
        for with_creds in [true, false] {
            if !with_creds && !has_password {
                break;
            }
            let Ok(info) = redis_connection_info(addr.clone(), record, 0, with_creds) else {
                break;
            };
            let Ok(client) = redis::Client::open(info) else {
                break;
            };
            let mut conn = match tokio::time::timeout(
                Duration::from_secs(8),
                client.get_multiplexed_async_connection(),
            )
            .await
            {
                Ok(Ok(conn)) => conn,
                Ok(Err(e)) => {
                    last_err = redis_connect_error(&e.to_string());
                    continue;
                }
                Err(_) => {
                    last_err = format!("哨兵 {host}:{port} 连接超时（8s）");
                    break;
                }
            };
            let mut cmd = redis::cmd("SENTINEL");
            cmd.arg("get-master-addr-by-name").arg(master_name);
            let value: redis::Value = match cmd.query_async(&mut conn).await {
                Ok(v) => v,
                Err(e) => {
                    last_err = format!("SENTINEL 查询失败（{host}:{port}）：{e}");
                    break;
                }
            };
            let redis::Value::Array(parts) = &value else {
                last_err =
                    format!("SENTINEL get-master-addr-by-name 返回异常形态：{value:?}");
                break;
            };
            let Some(master_host) = parts.first().and_then(redis_value_string) else {
                last_err = format!(
                    "哨兵未找到 master「{master_name}」（检查 masterName 与 SENTINEL monitor 配置）"
                );
                break;
            };
            // host 可能是哨兵 advertise 的内网名（replica-announce-ip），
            // 直接连；端口缺省 6379
            let master_port = parts
                .get(1)
                .and_then(redis_value_string)
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(6379);
            return connect_redis_endpoint(record, extra, &master_host, master_port)
                .await;
        }
    }
    Err(format!(
        "Redis 哨兵解析 master 失败（master={master_name}）：{last_err}"
    ))
}

/// 建 SQLite 连接：本地文件库（db_path 空 = :memory:），可选 SQLCipher
/// 加密。打开不带 CREATE 标志（文件不存在直接报错，不静默建空库）。
/// 加密参数顺序以 SQLCipher 源码为准：先 key（附加 codec 上下文）再
/// cipher_compatibility（该 pragma 只在 codec 已存在时生效）；KDF 惰性，
/// 首次读页才派生，故 key 之后、任何页访问之前设档位仍然有效。打开 +
/// 密钥校验会做 PBKDF2（SQLCipher 4 默认 256000 轮），整体放进
/// spawn_blocking。
pub(crate) async fn connect_sqlite(record: &DbConnectionRecord) -> Result<(DbEngine, String), String> {
    let path = record
        .db_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(":memory:")
        .to_string();
    let readonly = record.readonly;
    let mode = record.encryption_mode.trim().to_string();
    let password = record
        .password
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    // 口令校验前置（不进阻塞线程，错误信息与加密方式一并给出）
    let key_material = if mode == "none" {
        None
    } else if !matches!(
        mode.as_str(),
        "sqlcipher4" | "sqlcipher3" | "sqlcipher2" | "sqlcipher-raw"
    ) {
        return Err(format!("不支持的 SQLite 加密方式：{mode}"));
    } else if password.is_empty() {
        return Err("SQLite 加密数据库需要提供口令".to_string());
    } else if mode == "sqlcipher-raw" {
        // 原始密钥：SQLCipher 只认 x'<hex>' 字面量形态（长度恰为 key_sz*2+3
        // 才走直用分支，否则会被当成口令做 PBKDF2）。要求 64 位十六进制
        //（256-bit 密钥），不接受 96 位（key+salt）形态以免误解。
        let hex = password
            .strip_prefix("x'")
            .and_then(|s| s.strip_suffix('\''))
            .unwrap_or(password.as_str())
            .trim();
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("SQLite 原始密钥必须是 64 位十六进制字符串".to_string());
        }
        Some(format!("x'{hex}'"))
    } else {
        Some(password)
    };
    let encrypted = key_material.is_some();
    let (conn, version) = tokio::task::spawn_blocking(move || {
        let flags = if readonly {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_URI
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
        } else {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_URI
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
        };
        let conn = rusqlite::Connection::open_with_flags(&path, flags)
            .map_err(|e| format!("SQLite 打开失败：{e}（{path}）"))?;
        // 加密设置。顺序以 SQLCipher 源码为准（sqlite3.c 的
        // sqlcipher_codec_pragma）：cipher_compatibility 分支要求 codec 上下文
        // 已存在（`if(ctx)`），而 codec 由 PRAGMA key 经 sqlcipherCodecAttach
        // 建立——所以必须先 key 再 compatibility，反过来 compatibility 会被
        // 静默忽略（KDF 是惰性的：attach 只存口令，首次读页才派生密钥，故
        // key 之后、任何页访问之前设 compatibility 仍然生效）。
        if let Some(key) = key_material {
            conn.pragma_update(None, "key", key.as_str())
                .map_err(|e| format!("SQLite 设置加密口令失败：{e}"))?;
            let compat = match mode.as_str() {
                "sqlcipher2" => Some(2),
                "sqlcipher3" => Some(3),
                // sqlcipher4 即 SQLCipher 4 默认档位；raw 由密钥字面量决定，
                // 两者都不动 compatibility
                _ => None,
            };
            if let Some(v) = compat {
                conn.pragma_update(None, "cipher_compatibility", v)
                    .map_err(|e| format!("SQLite 设置加密兼容档位失败：{e}"))?;
            }
        }
        // 并发写锁等待（失败无副作用，忽略即可）
        let _ = conn.busy_timeout(Duration::from_secs(5));
        // 密钥校验：读 sqlite_master 即触发首块解密——口令错/非加密库在此
        // 暴露（SQLITE_NOTADB），不留到用户第一条查询才报
        if let Err(e) = conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(())) {
            if matches!(
                e.sqlite_error_code(),
                Some(rusqlite::ffi::ErrorCode::NotADatabase)
            ) {
                return Err(format!(
                    "SQLite 打开失败：口令错误或不是加密数据库（{path}）"
                ));
            }
            return Err(format!("SQLite 打开失败：{e}（{path}）"));
        }
        let mut version = conn
            .query_row("SELECT sqlite_version()", [], |row| row.get::<_, String>(0))
            .map(|v| format!("SQLite {v}"))
            .unwrap_or_else(|_| "SQLite".to_string());
        if encrypted {
            version.push_str(" · SQLCipher");
        }
        Ok::<_, String>((conn, version))
    })
    .await
    .map_err(|e| format!("SQLite 连接任务失败：{e}"))??;
    Ok((
        DbEngine::Sqlite(SqliteEngine {
            conn,
            cancelled: Arc::new(AtomicBool::new(false)),
        }),
        version,
    ))
}


/// OB Oracle 纯 Rust 臂：mysql_async 分叉（vendor/mysql-async-ob）+ ob20。
/// 与 connect_mysql 同构（标准 MySQL 握手/认证），额外打开 ob_protocol20
/// 并支持 TLS 四档。结果引擎是 MySql 变体——同一个 crate 同时充当两角色
/// （分叉即上游完整副本，ob_protocol20 只是握手选项），查询/目录/切库
/// 全部复用 MySql 臂。database 为初始 schema（OB Oracle 语义下经
/// db_name 下发，服务端按租户默认 schema 解析）。
pub(crate) async fn connect_ob_rust(
    record: &DbConnectionRecord,
) -> Result<(DbEngine, String), String> {
    let username = record
        .username
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "OceanBase 需要填写用户名".to_string())?;
    let mut builder = mysql_async::OptsBuilder::default()
        .ip_or_hostname(record.host.trim())
        .tcp_port(record.port)
        .user(Some(username))
        .ob_protocol20(true)
        // OB Oracle 租户：对象目录全部走 ALL_* 数据字典
        .ob_compat_mode(Some(mysql_async::ObCompatMode::Oracle));
    if let Some(pass) = record.password.as_deref().filter(|s| !s.is_empty()) {
        builder = builder.pass(Some(pass));
    }
    if let Some(db) = record.database.as_deref().filter(|s| !s.trim().is_empty()) {
        builder = builder.db_name(Some(db.trim()));
    }
    if record.ssl_mode != "disable" {
        let mut ssl = mysql_async::SslOpts::default();
        if record.ssl_mode == "verify-full" {
            ssl = ssl
                .with_danger_accept_invalid_certs(false)
                .with_danger_skip_domain_validation(false);
        } else {
            ssl = ssl
                .with_danger_accept_invalid_certs(true)
                .with_danger_skip_domain_validation(true);
        }
        builder = builder.ssl_opts(ssl);
    }
    let opts = mysql_async::Opts::from(builder);
    let mut conn = tokio::time::timeout(
        Duration::from_secs(12),
        mysql_async::Conn::new(opts),
    )
    .await
    .map_err(|_| "OceanBase 连接超时（12s）".to_string())?
    .map_err(|e| format!("OceanBase（Rust 驱动）连接失败：{e}"))?;
    // 版本串：VERSION() 在 OB Oracle 租户下返回 Oracle 兼容版本
    let version: Option<String> = {
        use mysql_async::prelude::Queryable as _;
        conn.query_first::<String, _>("SELECT VERSION() FROM DUAL")
            .await
            .ok()
            .flatten()
    };
    Ok((
        DbEngine::MySql(conn),
        version.unwrap_or_else(|| "OceanBase".to_string()),
    ))
}

pub(crate) async fn connect_dameng(record: &DbConnectionRecord) -> Result<(DbEngine, String), String> {
    // 达梦沿用 Oracle 的双字段语义：database = 初始 schema（可选）
    if record.ssl_mode != "disable" {
        return Err(
            "达梦引擎暂不支持 TLS（dm-pure 为明文 TCP），请将 SSL 模式设为 disable"
                .to_string(),
        );
    }
    let host = record.host.trim().to_string();
    let port = record.port;
    let username = record
        .username
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("SYSDBA")
        .to_string();
    // 密码允许为空（部分测试实例无密码），由服务器裁决
    let password = record.password.clone().unwrap_or_default();
    let initial_schema = record
        .database
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Some(schema) = initial_schema.as_deref()
        && !dm_valid_identifier(schema)
    {
        return Err("达梦无效的 schema 名".to_string());
    }
    let (client, version) = tokio::task::spawn_blocking(move || {
        let cfg = dm_pure::ClientConfig {
            host,
            port,
            user: username,
            password,
            app_name: "VeloTerm".to_string(),
            os_name: std::env::consts::OS.to_string(),
            // 驱动的 timeout 是 socket 读写「空闲」超时（不是语句总时长）：
            // 取 60s 对齐工作台默认语句预算，长查询只要服务端持续有帧
            // 往来就不会被提前掐断
            timeout: Duration::from_secs(60),
            // 静默重连会新建会话、丢掉 SET SCHEMA 的会话状态，后续查询
            // 悄悄落到默认 schema——工作台宁可报错让用户显式重连
            auto_reconnect: false,
            ..Default::default()
        };
        let mut client = dm_pure::DmClient::with_config(cfg);
        client
            .connect()
            .map_err(|e| format!("达梦连接失败：{e}"))?;
        // 先设 schema 再查版本：版本查询是「能拿就拿、拿不到回 DM8」的
        // 尽力而为查询，放在最后，它的失败不影响会话初始化
        if let Some(schema) = initial_schema {
            // SET SCHEMA 只改名字解析（同 Oracle 的 CURRENT_SCHEMA）。
            // 不加双引号：未加引号的标识符在达梦 Oracle 兼容模式下折叠成
            // 大写（与 Oracle 一致），用户手输小写也能命中；加了引号则
            // 大小写敏感（Oracle 臂同此取舍）
            let sql = format!("SET SCHEMA {schema}");
            client
                .execute(&sql, &[])
                .map_err(|e| format!("达梦设置初始 schema 失败：{e}"))?;
        }
        let version = dm_server_version(&mut client);
        Ok::<_, String>((client, version))
    })
    .await
    .map_err(|e| format!("达梦连接任务失败：{e}"))??;
    Ok((DbEngine::Dameng(client), version))
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
///
/// **每张证书必须各自一层 `BEGIN/END CERTIFICATE`**：把所有证书的 base64
/// 塞进同一个块里时，多张证书之间会出现 `=` 填充导致 PEM 解码失败；即使
/// 恰好无填充，解出来的也是「多张 DER 首尾相接」的字节，webpki 会按单张
/// 证书解析并拒收——系统根证书必然 ≥2 张，verify-full 因此稳定连不上。
pub(crate) fn native_roots_pem() -> Result<String, String> {
    let native = rustls_native_certs::load_native_certs();
    let mut pem = String::new();
    for cert in native.certs {
        let b64 = base64::engine::general_purpose::STANDARD.encode(cert.as_ref());
        pem.push_str("-----BEGIN CERTIFICATE-----\n");
        let mut rest = b64.as_str();
        while rest.len() > 64 {
            let (line, tail) = rest.split_at(64);
            pem.push_str(line);
            pem.push('\n');
            rest = tail;
        }
        pem.push_str(rest);
        pem.push('\n');
        pem.push_str("-----END CERTIFICATE-----\n");
    }
    if pem.is_empty() {
        return Err("未找到系统根证书（verify-full 模式需要）".to_string());
    }
    Ok(pem)
}

