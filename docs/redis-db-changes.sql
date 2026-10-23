-- =============================================================================
-- Redis 引擎（数据库工作台）—— 数据库变更说明
-- =============================================================================
--
-- 目的
--   veloterm 数据库工作台的 Redis 引擎（crates.io `redis` 1.7.1，
--   tokio-comp + tokio-rustls-comp + tls-rustls-insecure + cluster-async +
--   streams）。本文件说明该接入对 SQLCipher 库（veloterm.db）的表结构影响，
--   并给出核对语句。
--
-- 结论：一条新列 extra_options（幂等 ALTER，应用启动自动补列，零手工迁移）
--   db_connections 新增 `extra_options TEXT`（可空，JSON 文本）：引擎专属
--   扩展配置的通用槽位，当前仅 Redis 消费——拓扑（single/cluster/sentinel）、
--   集群/哨兵节点清单、哨兵 master 名、mTLS 客户端证书/私钥 PEM 路径。
--   结构定义见 src-tauri/src/db_browser/engines.rs 的 RedisExtraOptions
--   （serde camelCase）。非 Redis 记录恒 NULL。
--   其余沿用既有结论：
--     - database      存 Redis 逻辑库，形如 'db0' .. 'dbN'（可空）
--     - service_name  对 Redis 恒为 NULL（该列是 Oracle 服务名专用）
--     - engine        取值 'redis'（列为 TEXT，无 CHECK 约束）
--     - port          表列无默认值；6379 只是表单缺省，直插记录须自带端口
--
-- 对应的 Rust 侧写法（db.rs 的幂等 ALTER 批次已内置，无需手工执行）：
--   if let Err(e) = conn.execute_batch(
--       "ALTER TABLE db_connections ADD COLUMN extra_options TEXT")
--       && !e.to_string().contains("duplicate column")
--   {
--       return Err(format!(
--           "failed to apply db_connections column migration `extra_options`: {e}"));
--   }
--
-- 执行方式：只读核对
--   以下语句全部是 SELECT，可直接在 SQLCipher 库上跑。注意 veloterm.db 是
--   加密库，需带密钥打开（例如 sqlcipher CLI：PRAGMA key = '<密钥>';），
--   或在应用内以只读方式查看。本文件不含任何写语句。
-- =============================================================================


-- -----------------------------------------------------------------------------
-- 1. 核对：Redis 连接记录保存后的形态
-- -----------------------------------------------------------------------------
-- 预期结果（一条 Redis 记录）：
--   engine        = 'redis'
--   port          = 6379（或用户填写的端口）
--   database      = 'db0' .. 'dbN'，或 NULL（= 连接时使用 db0）
--   service_name  = NULL（Redis 不使用该列）
--   ssl_mode      = 'disable' | 'prefer' | 'require' | 'verify-full'
--                   （disable → redis:// 明文；其余三种 → rediss:// TLS）
--   extra_options = NULL 或 JSON 文本，形如：
--                   {"topology":"cluster","nodes":["10.0.0.1:6379"]}
--                   {"topology":"sentinel","nodes":["…"],"masterName":"mymaster"}
--                   {"topology":"single","clientCertPath":"…","clientKeyPath":"…"}
--                   （非法 JSON / 缺 topology 按单机处理——老记录兼容语义）
-- -----------------------------------------------------------------------------
SELECT id, name, engine, host, port, database, service_name, ssl_mode, extra_options
FROM db_connections
WHERE engine = 'redis';


-- -----------------------------------------------------------------------------
-- 2. 核对：确认没有非 Redis 记录被误改（回归用）
-- -----------------------------------------------------------------------------
-- 升级前后各跑一次，行数与内容应完全一致（Redis 接入只新增记录，
-- 不触碰既有 mysql / postgres / oracle 行）。
-- -----------------------------------------------------------------------------
SELECT engine, COUNT(*) AS n
FROM db_connections
GROUP BY engine
ORDER BY engine;


-- -----------------------------------------------------------------------------
-- 3. 核对：Redis 记录的 database 列取值分布（应为 dbN 或 NULL）
-- -----------------------------------------------------------------------------
-- 若出现纯数字（'0' / '1'）或带斜杠（'db0/foo'）等非约定形态，
-- 说明记录是旧版本/手工插入的，连接层按空值处理（回退 db0）前请先修正。
-- -----------------------------------------------------------------------------
SELECT id, name, database
FROM db_connections
WHERE engine = 'redis'
  AND database IS NOT NULL
  AND database NOT GLOB 'db[0-9]*';


-- -----------------------------------------------------------------------------
-- 4. 核对：extra_options 仅 Redis 记录携带（其他引擎恒 NULL）
-- -----------------------------------------------------------------------------
SELECT engine, COUNT(*) AS with_extra
FROM db_connections
WHERE extra_options IS NOT NULL
GROUP BY engine;
-- 预期：只可能出现 engine='redis' 的行


-- -----------------------------------------------------------------------------
-- 5. 核对：extra_options 的 JSON 合法性（在 SQLCipher 内可用 json_valid）
-- -----------------------------------------------------------------------------
-- 任何 valid=0 的行说明被手工写坏；连接层会按单机处理（拓扑信息缺失
-- 不阻断连接），修正后拓扑功能恢复。
-- -----------------------------------------------------------------------------
SELECT id, name, json_valid(extra_options) AS valid
FROM db_connections
WHERE engine = 'redis'
  AND extra_options IS NOT NULL;


-- =============================================================================
-- 备注
-- =============================================================================
-- 1) database 存 'dbN' 文本（如 'db2'），不存裸数字。
-- 2) database 为空 / NULL 时，连接层在建立连接后不执行 SELECT，
--    即使用服务端默认的 db0。这与「显式写 db0」在行为上等价，
--    但记录里保持空值更利于「跟随服务端默认」的语义。
-- 3) service_name 对 Redis 恒为 NULL：该列在 Oracle 引擎下才是必填，
--    Redis 的等价物（逻辑库）放在 database 列。
-- 4) engine 列无枚举约束，新增引擎不需要 DDL；前端下拉与后端引擎分支
--    各自维护取值集合，两处必须同时加 'redis'。
-- 5) extra_options 是引擎通用扩展槽位：未来其他引擎需要专属配置时复用
--    该列（各引擎自定义 JSON 结构），而不是继续加引擎专有列。
-- =============================================================================
