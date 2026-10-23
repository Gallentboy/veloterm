-- =============================================================================
-- 2026-10-22  db_connections：SQLite 引擎支持（新增 db_path / encryption_mode）
-- =============================================================================
--
-- 用途
--   为数据库工作台接入 SQLite 引擎，给应用自身的连接库表 db_connections 增加
--   两列：
--     db_path         SQLite 数据库文件路径（绝对路径，或 file: URI）；
--                     NULL / 空串 = 内存库 :memory:
--     encryption_mode 加密方式，取值 none | sqlcipher4 | sqlcipher3 |
--                     sqlcipher2 | sqlcipher-raw；NOT NULL DEFAULT 'none'
--
-- 目标库
--   应用自身的 SQLCipher 加密库 veloterm.db（位于系统应用数据目录，
--   见 src-tauri/src/db.rs 的 init_db / init_db_at_path）。这是加密库：
--   外部工具直接打开时须先执行 PRAGMA key = '<主密钥>'，主密钥来自启动环境变量
--   VELOTERM_DB_KEY，未设置时用 db.rs 的 DEFAULT_MASTER_KEY。
--
-- 执行方式
--   正常情况**不需要手工执行**：src-tauri/src/db.rs 的 init_db（内部调用
--   init_db_at_path）在应用启动时以幂等 ALTER 自动应用本变更。本脚本是同一
--   变更的可审计留档，也可用于外部工具对已有库补列（执行前建议先关闭应用：
--   应用启动时会自己补列，且并发写同一加密库会受 busy_timeout 限制）。
--
-- 幂等性
--   SQLite 的 ADD COLUMN 没有 IF NOT EXISTS：重复执行会报
--   "duplicate column name: db_path" / "...: encryption_mode"。
--   应用侧对每个 ALTER 单独判定，错误文案包含 "duplicate column" 即视为已应用
--   并继续（见 init_db_at_path 的列级迁移循环）；手工执行时忽略该报错即可，
--   不会影响已有数据。
--
-- 说明
--   两条语句按下列顺序执行，且刻意不包在显式事务里——与 init_db_at_path 的
--   应用路径一致（逐条 autocommit，重复执行时各自报 duplicate column 而互不
--   影响）。encryption_mode 带非 NULL 默认值，SQLite 允许对已有数据的表直接
--   ADD COLUMN，存量记录自动补 'none'。
--
--   engine 列（TEXT，无 CHECK 约束）新增取值 'sqlite' 不需要任何 DDL 变更：
--   合法引擎集合由应用层校验（db.rs 的 supported_engine /
--   validate_db_connection），不在数据库层约束。
-- =============================================================================

-- ---------------------------------------------------------------------------
-- 迁移正文
-- ---------------------------------------------------------------------------

ALTER TABLE db_connections ADD COLUMN db_path TEXT;

ALTER TABLE db_connections ADD COLUMN encryption_mode TEXT NOT NULL DEFAULT 'none';

-- ---------------------------------------------------------------------------
-- 回滚（SQLite >= 3.35 支持 DROP COLUMN）
-- ---------------------------------------------------------------------------
--
-- 旧版 SQLite（< 3.35）没有 DROP COLUMN，只能重建表：新建不含这两列的同构表 →
-- INSERT INTO ... SELECT 旧列 → DROP 旧表 → RENAME。执行前先用
--   SELECT sqlite_version();
-- 确认版本（应用随包的 SQLCipher 内置 SQLite 较新，通常满足 3.35+）。
--
-- 回滚会丢失 SQLite 连接的文件路径与加密方式配置：连接记录本身保留，但
-- db_path 为空后 SQLite 引擎的连接会退化为内存库（:memory:），加密库也无法
-- 再被打开。此外当前版本应用下次启动时 init_db 会重新补列（幂等 ALTER），
-- 因此回滚只在「同时降级应用版本」时才有持久效果。
--
-- ALTER TABLE db_connections DROP COLUMN encryption_mode;
-- ALTER TABLE db_connections DROP COLUMN db_path;

-- ---------------------------------------------------------------------------
-- 校验
-- ---------------------------------------------------------------------------
--
-- 注意：以下语句要在已经解密的连接上执行——外部工具打开 veloterm.db 前须先
--   PRAGMA key = '<主密钥>';
-- 否则第一条查询就会报 "file is not a database"（与口令错误同形）。

-- 1) 确认两列已存在（类型 / notnull / 默认值）
PRAGMA table_info(db_connections);

-- 2) 确认既有记录已被补默认值（存量记录 db_path 为 NULL、encryption_mode 为 'none'；
--    本查询返回 0 行属正常——新建的库里还没有 engine='sqlite' 的连接记录）
SELECT id, name, engine, db_path, encryption_mode
  FROM db_connections
 WHERE engine = 'sqlite';

-- 3) 确认 NOT NULL DEFAULT 已生效（应为 0）
SELECT COUNT(*) AS missing_encryption_mode
  FROM db_connections
 WHERE encryption_mode IS NULL;
