//! Redis 引擎专属实现：连接拓扑（单机/集群/sentinel）、命令执行、键检视与
//! 写回、慢日志、大 key 分析、命令行分词。
//!
//! 从 engines.rs 拆出（纯移动，零行为变化）。所有助手面向 `RedisConn` 枚举，
//! 经 `redis_run_cmd` / `redis_run_pipe` 两个单点派发，新增拓扑不改助手签名。
//!
//! 依赖 `super` 的项：连接记录类型、通用值格式化，以及散在 mod.rs 里被
//! match 臂与测试共用的 Redis 常量/解析函数（`REDIS_*`、`parse_redis_db`、
//! `redis_extra_options`、`format_float`）——这些留在 mod.rs 以避免两边
//! 各存一份。

use serde::{Deserialize, Serialize};

use crate::db::DbConnectionRecord;

use super::{
    REDIS_ELEMENT_CAP, REDIS_SCAN_COUNT, REDIS_SCAN_KEY_CAP, REDIS_STREAM_CAP, format_float,
    parse_redis_db,
};

pub enum RedisConn {
    Single(redis::aio::MultiplexedConnection),
    Cluster(redis::cluster_async::ClusterConnection),
}

/// 命令单点派发（错误文案由调用方包装）
pub(crate) async fn redis_run_cmd<T: redis::FromRedisValue>(
    conn: &mut RedisConn,
    cmd: &mut redis::Cmd,
) -> Result<T, redis::RedisError> {
    match conn {
        RedisConn::Single(c) => cmd.query_async(c).await,
        RedisConn::Cluster(c) => cmd.query_async(c).await,
    }
}

/// pipeline 单点派发（原子/非原子由构造 Pipeline 时的 atomic() 决定）
pub async fn redis_run_pipe<T: redis::FromRedisValue>(
    conn: &mut RedisConn,
    pipe: &mut redis::Pipeline,
) -> Result<T, redis::RedisError> {
    match conn {
        RedisConn::Single(c) => pipe.query_async(c).await,
        RedisConn::Cluster(c) => pipe.query_async(c).await,
    }
}

/// db_connections.extra_options 的 Redis 结构（JSON 文本）。列是引擎通用
/// 扩展槽位，当前仅 Redis 消费；字段缺失/非法 JSON 一律按单机处理（拓扑
/// 信息缺失不阻断连接——extra_options 是后加列，老记录全是单机语义）。
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RedisExtraOptions {
    /// single（默认/缺省）| cluster | sentinel
    #[serde(default)]
    pub topology: String,
    /// 集群的种子节点 / 哨兵地址清单（"host:port"）
    #[serde(default)]
    pub nodes: Vec<String>,
    /// 哨兵 master 名（sentinel 拓扑必填）
    #[serde(default)]
    pub master_name: Option<String>,
    /// mTLS 客户端证书 PEM 路径（与 client_key_path 成对）
    #[serde(default)]
    pub client_cert_path: Option<String>,
    #[serde(default)]
    pub client_key_path: Option<String>,
}

/// 解析 extra_options（空/非法 → 单机默认）
pub(crate) fn redis_extra_options(raw: Option<&str>) -> RedisExtraOptions {
    raw.and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default()
}

/// 会话元信息捎带的拓扑标签（db_connect 消费，前端据此禁用集群切库）
#[inline]
pub fn redis_topology_of(raw: Option<&str>) -> Option<&'static str> {
    match redis_extra_options(raw).topology.as_str() {
        "cluster" => Some("cluster"),
        "sentinel" => Some("sentinel"),
        _ => None,
    }
}

/// extra_options.nodes → (host, port) 清单；空清单报错（cluster/sentinel
/// 都至少要一个地址）。按最后一个冒号切（兼容 [IPv6]:port 的括号形态）
pub(crate) fn redis_node_list(extra: &RedisExtraOptions) -> Result<Vec<(String, u16)>, String> {
    let mut out = Vec::new();
    for raw in &extra.nodes {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (host, port) = trimmed
            .rsplit_once(':')
            .ok_or_else(|| format!("Redis 节点地址缺端口（应为 host:port）：{trimmed}"))?;
        let port: u16 = port
            .parse()
            .map_err(|_| format!("Redis 节点端口不合法：{trimmed}"))?;
        let host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        if host.is_empty() {
            return Err(format!("Redis 节点地址缺主机：{trimmed}"));
        }
        out.push((host, port));
    }
    if out.is_empty() {
        return Err(
            "Redis 集群/哨兵拓扑需要在节点列表提供至少一个地址（host:port）".to_string(),
        );
    }
    Ok(out)
}

/// TLS 四档 → ConnectionAddr（prefer/require 加密但不校验证书，语义与
/// MySQL/PG 臂一致：绝不回退明文；verify-full 走系统根证书）
#[inline]
pub(crate) fn redis_addr(host: &str, port: u16, ssl_mode: &str) -> redis::ConnectionAddr {
    if ssl_mode == "disable" {
        redis::ConnectionAddr::Tcp(host.to_string(), port)
    } else {
        redis::ConnectionAddr::TcpTls {
            host: host.to_string(),
            port,
            insecure: ssl_mode != "verify-full",
            tls_params: None,
        }
    }
}

/// ConnectionInfo 组装：直接构造（不经 URL——密码/用户名里的特殊字符会被
/// URL 语法吃掉）；with_creds=false 供哨兵无认证重试（master 始终带凭据）。
/// 不设 protocol 保持 RESP2——老版本 Redis（6 之前无 HELLO）连不上 RESP3，
/// 值转换层对两种响应形态都兼容。
pub(crate) fn redis_connection_info(
    addr: redis::ConnectionAddr,
    record: &DbConnectionRecord,
    db: u32,
    with_creds: bool,
) -> Result<redis::ConnectionInfo, String> {
    let mut redis_info = redis::RedisConnectionInfo::default().set_db(i64::from(db));
    if with_creds {
        // 空字符串按「未设置」处理（表单留空 = 无 ACL 用户名/无密码）
        if let Some(user) = record.username.as_deref().filter(|s| !s.is_empty()) {
            redis_info = redis_info.set_username(user);
        }
        if let Some(pass) = record.password.as_deref().filter(|s| !s.is_empty()) {
            redis_info = redis_info.set_password(pass);
        }
    }
    // ConnectionInfo 的字段是私有的：经 IntoConnectionInfo（ConnectionAddr
    // 直接构造 → 默认 tcp_settings）再整体替换 redis 段，避免拼 URL。
    Ok(redis::IntoConnectionInfo::into_connection_info(addr)
        .map_err(|e| redis_connect_error(&e.to_string()))?
        .set_redis_settings(redis_info))
}

/// mTLS 证书（PEM 路径 → 字节）；两路径只给其一直接报错。root 证书不单独
/// 开路径：自建 CA 请装进系统信任库。
pub(crate) fn redis_tls_certs(extra: &RedisExtraOptions) -> Result<Option<redis::TlsCertificates>, String> {
    let cert = extra
        .client_cert_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let key = extra
        .client_key_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match (cert, key) {
        (None, None) => Ok(None),
        (Some(cert), Some(key)) => Ok(Some(redis::TlsCertificates {
            client_tls: Some(redis::ClientTlsConfig {
                client_cert: std::fs::read(cert)
                    .map_err(|e| format!("Redis 客户端证书读取失败（{cert}）：{e}"))? ,
                client_key: std::fs::read(key)
                    .map_err(|e| format!("Redis 客户端私钥读取失败（{key}）：{e}"))?,
            }),
            root_cert: None,
        })),
        _ => Err("Redis mTLS 需要同时提供客户端证书与私钥的 PEM 路径".to_string()),
    }
}

#[inline(always)]
pub(crate) fn redis_db_index(value: Option<&str>) -> u32 {
    value.and_then(parse_redis_db).unwrap_or(0)
}

/// Redis 连接错误文案：认证类错误追加排查提示（NOAUTH 未提供密码 /
/// WRONGPASS 密码错 / AUTH 一般性失败；云实例还会给 NOAUTH Authentication
/// required 或 NOPERM）。
#[inline(always)]
pub(crate) fn redis_connect_error(message: &str) -> String {
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
pub(crate) async fn redis_version(conn: &mut RedisConn) -> Option<String> {
    let mut cmd = redis::cmd("INFO");
    cmd.arg("server");
    let info: String = redis_run_cmd(conn, &mut cmd).await.ok()?;
    redis_info_field(&info, "redis_version").map(|v| format!("Redis {v}"))
}

/// INFO 文本里取 `field:value`（INFO 是 CRLF 分隔的纯文本）
#[inline]
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
pub(crate) async fn redis_database_count(conn: &mut RedisConn) -> Option<u32> {
    let mut cmd = redis::cmd("CONFIG");
    cmd.arg("GET").arg("databases");
    let value: redis::Value = redis_run_cmd(conn, &mut cmd).await.ok()?;
    let (_, rows) = redis_value_to_table(&value, "CONFIG");
    rows.first()
        .and_then(|row| row.get(1))
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|n| *n > 0)
}

/// INFO keyspace → 每个非空逻辑库的 key 数（下标 = db 索引）。
/// 形如 `db0:keys=3,expires=0,avg_ttl=0`；空库不出现在 INFO 里。
pub(crate) async fn redis_keyspace_counts(conn: &mut RedisConn) -> Vec<usize> {
    let mut cmd = redis::cmd("INFO");
    cmd.arg("keyspace");
    let Ok(info): Result<String, _> = redis_run_cmd(conn, &mut cmd).await else {
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

/// SELECT 切库（不改引擎的 db 字段——调用方负责上下文还原）；
/// Cluster 只有 db0：对象树/详情的「临时切库」在集群下是空操作
pub(crate) async fn redis_select(conn: &mut RedisConn, db: u32) -> Result<(), String> {
    if matches!(conn, RedisConn::Cluster(_)) {
        return Ok(());
    }
    let mut cmd = redis::cmd("SELECT");
    cmd.arg(db);
    redis_run_cmd::<()>(conn, &mut cmd)
        .await
        .map_err(|e| format!("Redis 切换逻辑库失败：{e}"))
}

/// SCAN 抓 key（上限 REDIS_SCAN_KEY_CAP），再按 REDIS_SCAN_COUNT 条一批
/// 用 pipeline 发 TYPE 归类。返回 (key, type) 序列，顺序即 SCAN 顺序
///（Redis 不保证有序，这里也不再排——保持零额外开销）。
/// 单条 TYPE 失败（key 在 SCAN 与 TYPE 之间过期）整批丢弃该条。
pub(crate) async fn redis_scan_typed_keys(
    conn: &mut RedisConn,
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
        let values: Vec<redis::Value> = redis_run_pipe(conn, &mut pipe)
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
async fn redis_scan_keys(conn: &mut RedisConn) -> Result<Vec<String>, String> {
    let mut cursor = 0u64;
    let mut keys: Vec<String> = Vec::new();
    loop {
        // SCAN 返回 [cursor, [key...]]；RESP3 下也是 Array 两元素。
        // 按 Value 取而不是 (u64, Vec<String>)：key 允许任意字节，
        // Vec<String> 遇到非 UTF-8 key 会让整页反序列化失败。
        let mut cmd = redis::cmd("SCAN");
        cmd.arg(cursor).arg("COUNT").arg(REDIS_SCAN_COUNT);
        let value: redis::Value = redis_run_cmd(conn, &mut cmd)
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
pub(crate) async fn redis_key_type(conn: &mut RedisConn, key: &str) -> Result<String, String> {
    let mut cmd = redis::cmd("TYPE");
    cmd.arg(key);
    let type_name: String = redis_run_cmd(conn, &mut cmd)
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
pub(crate) async fn redis_inspect_block(
    conn: &mut RedisConn,
    key: &str,
    type_name: &str,
) -> String {
    let mut lines = vec![
        format!("KEY {key}"),
        format!("TYPE {type_name}"),
    ];
    // TTL：-1 永不过期、-2 不存在（此时前面 TYPE 已经报错了，不会到这）
    let mut cmd = redis::cmd("TTL");
    cmd.arg(key);
    let ttl: i64 = redis_run_cmd(conn, &mut cmd).await.unwrap_or(-1);
    lines.push(format!("TTL {ttl}"));
    // MEMORY USAGE 是 4.0+ 命令，且可被 rename/禁用
    let mut cmd = redis::cmd("MEMORY");
    cmd.arg("USAGE").arg(key);
    if let Ok(bytes) = redis_run_cmd::<i64>(conn, &mut cmd).await {
        lines.push(format!("MEMORY USAGE {bytes}"));
    }
    // OBJECT ENCODING（2.2+）：展示内部编码（ziplist/quicklist/hashtable…）
    let mut cmd = redis::cmd("OBJECT");
    cmd.arg("ENCODING").arg(key);
    if let Ok(encoding) = redis_run_cmd::<String>(conn, &mut cmd).await {
        lines.push(format!("ENCODING {encoding}"));
    }
    // 类型专属长度：STRLEN/HLEN/LLEN/SCARD/ZCARD/XLEN
    if let Some(len_cmd) = redis_length_command(type_name) {
        let mut cmd = redis::cmd(len_cmd);
        cmd.arg(key);
        if let Ok(len) = redis_run_cmd::<i64>(conn, &mut cmd).await {
            lines.push(format!("LENGTH {len}"));
        }
    }
    lines.join("\n")
}

/// 值类型 → 长度命令（未知类型无长度概念）
#[inline(always)]
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
#[inline]
pub(crate) fn redis_read_command(key: &str, type_name: &str) -> String {
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
#[inline(always)]
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
pub(crate) async fn redis_key_entries(
    conn: &mut RedisConn,
    key: &str,
    type_name: &str,
) -> (&'static str, Vec<Vec<String>>) {
    match type_name {
        "string" => {
            let mut cmd = redis::cmd("GET");
            cmd.arg(key);
            let value: redis::Value =
                redis_run_cmd(conn, &mut cmd).await.unwrap_or(redis::Value::Nil);
            (
                "value",
                vec![vec!["value".to_string(), redis_value_text(&value)]],
            )
        }
        "hash" => {
            let mut cmd = redis::cmd("HGETALL");
            cmd.arg(key);
            let flat = redis_flat_strings(conn, &mut cmd).await;
            ("field", redis_pair_rows(flat, REDIS_ELEMENT_CAP))
        }
        "list" => {
            let mut cmd = redis::cmd("LRANGE");
            cmd.arg(key).arg(0).arg(REDIS_ELEMENT_CAP as i64 - 1);
            let items = redis_flat_strings(conn, &mut cmd).await;
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
            let members = redis_flat_strings(conn, &mut cmd).await;
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
            let flat = redis_flat_strings(conn, &mut cmd).await;
            ("member", redis_pair_rows(flat, REDIS_ELEMENT_CAP))
        }
        "stream" => {
            // XRANGE 返回 [[id, [field, value, ...]], ...]；字段序列化成
            // `field=value field=value` 的紧凑串（一条消息多字段，表格
            // 一行放不下结构化展示）
            let mut cmd = redis::cmd("XRANGE");
            cmd.arg(key).arg("-").arg("+").arg("COUNT").arg(REDIS_STREAM_CAP);
            let value: redis::Value =
                redis_run_cmd(conn, &mut cmd).await.unwrap_or(redis::Value::Nil);
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
    conn: &mut RedisConn,
    cmd: &mut redis::Cmd,
) -> Vec<String> {
    let value: redis::Value = match redis_run_cmd(conn, cmd).await {
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
#[inline(always)]
fn redis_value_text(value: &redis::Value) -> String {
    redis_value_string(value).unwrap_or_else(|| "(nil)".to_string())
}

/// stream 消息的字段数组 [f1,v1,f2,v2,…] → "f1=v1 f2=v2"
#[inline]
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
#[inline]
pub(crate) fn redis_value_string(value: &redis::Value) -> Option<String> {
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
#[inline(always)]
fn redis_pairs_by_command(command: &str) -> bool {
    matches!(command, "HGETALL" | "CONFIG" | "XINFO")
}

/// 单元格渲染：嵌套结构压成一行紧凑文本（表格里展开多层不可读）。
/// 刻意不做「偶数长度数组按 k=v 配对」的猜测——MGET/LRANGE 的两元素结果
/// 会被误读成键值对；只有明确知道是字段组的调用方（XRANGE 消息体，见
/// redis_stream_fields）才做配对。
#[inline]
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
pub(crate) fn redis_value_to_table(value: &redis::Value, command: &str) -> (Vec<String>, Vec<Vec<String>>) {
    // INFO 的 bulk 文本按行拆 field/value（整块塞一格不可读；# Section
    // 分节行占 field 列作视觉分隔）
    if command == "INFO"
        && let redis::Value::BulkString(bytes) = value
        && let Ok(text) = std::str::from_utf8(bytes)
    {
        return redis_info_table(text);
    }
    // SCAN：[cursor, [key…]] → cursor/key 两列，游标只在首行出现（前端翻页
    // 按钮读首行首列；'0' 表示迭代完毕）。key 全空时保留一行游标供翻页判断
    if command == "SCAN"
        && let redis::Value::Array(parts) = value
    {
        return redis_scan_table(parts);
    }
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

/// INFO 文本 → field/value 两列表（# Section 分节行占 field 列、value 留空；
/// CRLF 是 INFO 的行分隔原生形态）
pub(crate) fn redis_info_table(text: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if let Some(section) = line.strip_prefix("# ") {
            rows.push(vec![format!("◆ {}", section.trim()), String::new()]);
        } else if let Some((k, v)) = line.split_once(':') {
            rows.push(vec![k.trim().to_string(), v.trim().to_string()]);
        }
    }
    (vec!["field".to_string(), "value".to_string()], rows)
}

/// SCAN 的 [cursor, [key…]] → cursor/key 两列（游标只在首行；空页保留
/// 游标行供前端判断翻页终点）
pub(crate) fn redis_scan_table(parts: &[redis::Value]) -> (Vec<String>, Vec<Vec<String>>) {
    let cursor = parts
        .first()
        .and_then(redis_value_string)
        .unwrap_or_else(|| "0".to_string());
    let keys: Vec<String> = parts
        .get(1)
        .map(|v| match v {
            redis::Value::Array(items) => {
                items.iter().filter_map(redis_value_string).collect()
            }
            redis::Value::Nil => Vec::new(),
            other => redis_value_string(other).into_iter().collect(),
        })
        .unwrap_or_default();
    let rows = if keys.is_empty() {
        vec![vec![cursor, "(本页无 key)".to_string()]]
    } else {
        keys.into_iter()
            .enumerate()
            .map(|(i, k)| {
                vec![
                    if i == 0 { cursor.clone() } else { String::new() },
                    k,
                ]
            })
            .collect()
    };
    (vec!["cursor".to_string(), "key".to_string()], rows)
}

// ── Redis 键管理与诊断（redis_read_key / redis_write_key / redis_slowlog /
// redis_bigkeys 四个命令的引擎层实现）──────────────────────────────────

/// 键编辑的成员上限：超限置 truncated，前端禁整体覆写（DEL+重写的语义下
/// 截断保存会丢数据，宁可不给保存）
const REDIS_EDIT_CAP: usize = 5000;

/// 键结构化读结果（redis_read_key 命令返回；entries 按 kind 呈
/// {field,value}（hash）或 {member,score}（zset）两种 JSON 形态）
#[derive(Serialize)]
pub struct RedisKeyData {
    pub kind: String,
    /// -1 永不过期；-2 key 不存在（redis_key_type 已先行报错，防御性兜底）
    pub ttl: i64,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
}

/// 键整体覆写载荷（redis_write_key 命令参数；kind 是判别字段）
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RedisKeyWrite {
    String { value: String },
    Hash { entries: Vec<RedisHashEntry> },
    List { items: Vec<String> },
    Set { members: Vec<String> },
    ZSet { entries: Vec<RedisZsetEntry> },
}

#[derive(Debug, Deserialize)]
pub struct RedisHashEntry {
    pub field: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct RedisZsetEntry {
    pub member: String,
    pub score: String,
}

/// 表格型返回（redis_slowlog 命令；headers/rows 直入前端 DataTable）
#[derive(Serialize)]
pub struct RedisTableData {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// 大 key 分析结果（redis_bigkeys 命令）
#[derive(Serialize)]
pub struct RedisBigkeysData {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// 实际扫描到的 key 数（采样上限内的全量）
    pub scanned: usize,
    /// 采样是否触及上限（true = 库里还有更多 key 未参与统计）
    pub truncated: bool,
}

/// 键结构化读：TYPE + TTL + 按类型的成员明细（REDIS_EDIT_CAP 截断保护）。
/// 与对象树检视同型：切到目标库读，非当前库读完切回。
pub async fn redis_read_key_full(
    conn: &mut RedisConn,
    current_db: u32,
    target_db: u32,
    key: &str,
) -> Result<RedisKeyData, String> {
    redis_select(conn, target_db).await?;
    let result = redis_read_key_at(conn, key).await;
    if target_db != current_db {
        redis_select(conn, current_db).await.ok();
    }
    result
}

async fn redis_read_key_at(conn: &mut RedisConn, key: &str) -> Result<RedisKeyData, String> {
    let kind = redis_key_type(conn, key).await?;
    let mut cmd = redis::cmd("TTL");
    cmd.arg(key);
    let ttl: i64 = redis_run_cmd(conn, &mut cmd).await.unwrap_or(-1);
    let mut data = RedisKeyData {
        kind: kind.clone(),
        ttl,
        truncated: false,
        value: None,
        entries: Vec::new(),
        items: Vec::new(),
        members: Vec::new(),
    };
    match kind.as_str() {
        "string" => {
            let mut cmd = redis::cmd("GET");
            cmd.arg(key);
            let value: redis::Value =
                redis_run_cmd(conn, &mut cmd).await.unwrap_or(redis::Value::Nil);
            data.value = Some(redis_value_text(&value));
        }
        "hash" => {
            let mut cmd = redis::cmd("HGETALL");
            cmd.arg(key);
            let flat = redis_flat_strings(conn, &mut cmd).await;
            data.truncated = flat.len() > REDIS_EDIT_CAP * 2;
            data.entries = redis_pair_rows(flat, REDIS_EDIT_CAP)
                .into_iter()
                .map(|row| {
                    serde_json::json!({
                        "field": row.first().cloned().unwrap_or_default(),
                        "value": row.get(1).cloned().unwrap_or_default(),
                    })
                })
                .collect();
        }
        "list" => {
            let mut cmd = redis::cmd("LLEN");
            cmd.arg(key);
            let len: i64 = redis_run_cmd(conn, &mut cmd).await.unwrap_or(0);
            data.truncated = len as usize > REDIS_EDIT_CAP;
            let end = if data.truncated {
                REDIS_EDIT_CAP as i64 - 1
            } else {
                -1
            };
            let mut cmd = redis::cmd("LRANGE");
            cmd.arg(key).arg(0).arg(end);
            data.items = redis_flat_strings(conn, &mut cmd).await;
        }
        "set" => {
            let mut cmd = redis::cmd("SCARD");
            cmd.arg(key);
            let len: i64 = redis_run_cmd(conn, &mut cmd).await.unwrap_or(0);
            data.truncated = len as usize > REDIS_EDIT_CAP;
            let mut cmd = redis::cmd("SMEMBERS");
            cmd.arg(key);
            let all = redis_flat_strings(conn, &mut cmd).await;
            data.members = all.into_iter().take(REDIS_EDIT_CAP).collect();
        }
        "zset" => {
            let mut cmd = redis::cmd("ZCARD");
            cmd.arg(key);
            let len: i64 = redis_run_cmd(conn, &mut cmd).await.unwrap_or(0);
            data.truncated = len as usize > REDIS_EDIT_CAP;
            let end = if data.truncated {
                REDIS_EDIT_CAP as i64 - 1
            } else {
                -1
            };
            let mut cmd = redis::cmd("ZRANGE");
            cmd.arg(key).arg(0).arg(end).arg("WITHSCORES");
            let flat = redis_flat_strings(conn, &mut cmd).await;
            data.entries = redis_pair_rows(flat, REDIS_EDIT_CAP)
                .into_iter()
                .map(|row| {
                    serde_json::json!({
                        "member": row.first().cloned().unwrap_or_default(),
                        "score": row.get(1).cloned().unwrap_or_default(),
                    })
                })
                .collect();
        }
        // stream / 未知类型：无成员编辑语义（前端按不支持处理；TTL 仍可读）
        _ => {}
    }
    Ok(data)
}

/// 键整体覆写：先读原 TTL，DEL + 按类型写入 +（可选）恢复 TTL 进一条
/// MULTI/EXEC 原子管道（同 key 同 slot，集群下也成立）。清空所有成员 =
/// 删除该 key（DEL 后无写入）。返回写入的成员数。
pub async fn redis_write_key_full(
    conn: &mut RedisConn,
    current_db: u32,
    target_db: u32,
    key: &str,
    payload: RedisKeyWrite,
    keep_ttl: bool,
) -> Result<i64, String> {
    redis_select(conn, target_db).await?;
    let result = redis_write_key_at(conn, key, payload, keep_ttl).await;
    if target_db != current_db {
        redis_select(conn, current_db).await.ok();
    }
    result
}

async fn redis_write_key_at(
    conn: &mut RedisConn,
    key: &str,
    payload: RedisKeyWrite,
    keep_ttl: bool,
) -> Result<i64, String> {
    let mut cmd = redis::cmd("TTL");
    cmd.arg(key);
    let old_ttl: i64 = redis_run_cmd(conn, &mut cmd).await.unwrap_or(-1);

    let mut pipe = redis::pipe();
    pipe.atomic();
    pipe.cmd("DEL").arg(key);
    let written = match payload {
        RedisKeyWrite::String { value } => {
            pipe.cmd("SET").arg(key).arg(value);
            1
        }
        RedisKeyWrite::Hash { entries } => {
            if !entries.is_empty() {
                pipe.cmd("HSET").arg(key);
                for entry in &entries {
                    pipe.arg(&entry.field).arg(&entry.value);
                }
            }
            entries.len() as i64
        }
        RedisKeyWrite::List { items } => {
            for item in &items {
                pipe.cmd("RPUSH").arg(key).arg(item);
            }
            items.len() as i64
        }
        RedisKeyWrite::Set { members } => {
            if !members.is_empty() {
                pipe.cmd("SADD").arg(key);
                for member in &members {
                    pipe.arg(member);
                }
            }
            members.len() as i64
        }
        RedisKeyWrite::ZSet { entries } => {
            if !entries.is_empty() {
                pipe.cmd("ZADD").arg(key);
                for entry in &entries {
                    let score: f64 = entry.score.trim().parse().map_err(|_| {
                        format!("zset 分数不合法：{}（成员 {}）", entry.score, entry.member)
                    })?;
                    pipe.arg(score).arg(&entry.member);
                }
            }
            entries.len() as i64
        }
    };
    if keep_ttl && old_ttl > 0 {
        pipe.cmd("EXPIRE").arg(key).arg(old_ttl);
    }
    let _: () = redis_run_pipe(conn, &mut pipe)
        .await
        .map_err(|e| format!("Redis 写键失败：{e}"))?;
    Ok(written)
}

/// SLOWLOG GET n → 结构化行。RESP2 数组形态：每条 [id, ts(秒), 耗时μs,
/// [命令…], 客户端地址, 客户端名]（后两项 4.0+；旧版缺位留空）。
/// SLOWLOG 是实例级数据，不切库。
pub async fn redis_slowlog_rows(
    conn: &mut RedisConn,
    count: usize,
) -> Result<RedisTableData, String> {
    let mut cmd = redis::cmd("SLOWLOG");
    cmd.arg("GET").arg(count as i64);
    let value: redis::Value = redis_run_cmd(conn, &mut cmd)
        .await
        .map_err(|e| format!("SLOWLOG 查询失败：{e}"))?;
    let redis::Value::Array(entries) = value else {
        return Ok(RedisTableData {
            headers: Vec::new(),
            rows: Vec::new(),
        });
    };
    let headers = vec!["id", "时间", "耗时", "命令", "客户端"]
        .into_iter()
        .map(String::from)
        .collect();
    let mut rows = Vec::new();
    for entry in &entries {
        let redis::Value::Array(parts) = entry else {
            continue;
        };
        let id = parts.first().map(redis_value_text).unwrap_or_default();
        let ts = parts
            .get(1)
            .and_then(redis_value_string)
            .and_then(|s| s.parse::<i64>().ok())
            .and_then(|secs| {
                chrono::DateTime::from_timestamp(secs, 0)
                    .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            })
            .unwrap_or_default();
        let us = parts
            .get(2)
            .and_then(|v| redis_value_string(v).and_then(|s| s.parse::<i64>().ok()))
            .unwrap_or(0);
        let elapsed = if us >= 1000 {
            format!("{} ms", us / 1000)
        } else {
            format!("{us} μs")
        };
        let command = parts
            .get(3)
            .map(|v| match v {
                redis::Value::Array(tokens) => tokens
                    .iter()
                    .filter_map(redis_value_string)
                    .collect::<Vec<_>>()
                    .join(" "),
                other => redis_value_text(other),
            })
            .unwrap_or_default();
        let client = parts
            .get(4)
            .and_then(redis_value_string)
            .unwrap_or_default();
        rows.push(vec![id, ts, elapsed, command, client]);
    }
    Ok(RedisTableData { headers, rows })
}

/// SCAN 采样 + 批量 MEMORY USAGE/TYPE → 按内存倒序取前 top_n。
/// MEMORY USAGE 是采样估算且托管实例常禁用：单 key 失败按 0 字节计（排序
/// 靠后）。SCAN 在集群拓扑下只命中被路由到的单个节点（连接层边界）。
pub async fn redis_bigkeys_rows(
    conn: &mut RedisConn,
    sample: usize,
    top_n: usize,
) -> Result<RedisBigkeysData, String> {
    let mut cursor = 0u64;
    let mut keys: Vec<String> = Vec::new();
    let mut truncated = false;
    loop {
        let mut cmd = redis::cmd("SCAN");
        cmd.arg(cursor).arg("COUNT").arg(REDIS_SCAN_COUNT);
        let value: redis::Value = redis_run_cmd(conn, &mut cmd)
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
        if next == 0 || keys.len() >= sample {
            truncated = next != 0 && keys.len() >= sample;
            break;
        }
        cursor = next;
    }
    keys.truncate(sample);
    let scanned = keys.len();
    // 批量 MEMORY USAGE + TYPE（同 pipeline 按位对齐；USAGE 失败位不是 Int
    //（Nil/ServerError），按 0 字节计）
    let mut entries: Vec<(String, String, i64)> = Vec::with_capacity(keys.len());
    for chunk in keys.chunks(REDIS_SCAN_COUNT) {
        let mut pipe = redis::pipe();
        for key in chunk {
            pipe.cmd("MEMORY").arg("USAGE").arg(key);
            pipe.cmd("TYPE").arg(key);
        }
        pipe.ignore_errors();
        let values: Vec<redis::Value> = redis_run_pipe(conn, &mut pipe)
            .await
            .map_err(|e| format!("Redis 探测内存占用失败：{e}"))?;
        for (i, key) in chunk.iter().enumerate() {
            let bytes = values
                .get(i * 2)
                .and_then(|v| match v {
                    redis::Value::Int(n) => Some(*n),
                    _ => None,
                })
                .unwrap_or(0);
            let type_name = values
                .get(i * 2 + 1)
                .and_then(redis_value_string)
                .unwrap_or_else(|| "?".to_string());
            entries.push((key.clone(), type_name, bytes));
        }
    }
    entries.sort_by_key(|(_, _, bytes)| std::cmp::Reverse(*bytes));
    let rows = entries
        .into_iter()
        .take(top_n)
        .map(|(key, type_name, bytes)| vec![key, type_name, bytes.to_string()])
        .collect();
    Ok(RedisBigkeysData {
        headers: vec![
            "key".to_string(),
            "类型".to_string(),
            "内存(字节)".to_string(),
        ],
        rows,
        scanned,
        truncated,
    })
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
pub(crate) fn redis_split_args(input: &str) -> Vec<String> {
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
