/**
 * 数据库工作台连接库：侧栏「数据库主机」页 CRUD + 会话/目录/查询 invoke 封装。
 * 七引擎共用同一套记录与会话接口，引擎差异只落在 database 字段语义与后端实现。
 * 类型与后端 db_browser 模块的 serde camelCase 契约一一对应。
 * SQLite 引擎复用 password 作为加密口令、dbPath 为库文件路径。
 * Redis 的拓扑/节点/哨兵 master/mTLS 证书路径存 extraOptions（JSON 文本）。
 */

import { Channel, invoke } from '@tauri-apps/api/core';

export type DbEngineKind =
  | 'mysql'
  | 'postgres'
  | 'oracle'
  | 'redis'
  | 'oceanbase'
  | 'sqlite'
  | 'dameng';

/**
 * SQLite 加密方式（后端 db.rs 的 encryption_mode 列）：
 * - none：明文 SQLite
 * - sqlcipher4：SQLCipher 4 默认参数（PBKDF2-HMAC-SHA512 / 256000 轮）
 * - sqlcipher3 / sqlcipher2：兼容旧版 SQLCipher 生成的库（cipher_compatibility）
 * - sqlcipher-raw：原始密钥，password 填 64 位十六进制（不经 KDF）
 */
export type SqliteEncryptionMode =
  | 'none'
  | 'sqlcipher4'
  | 'sqlcipher3'
  | 'sqlcipher2'
  | 'sqlcipher-raw';

/** Redis 的 extra_options JSON 结构（后端 RedisExtraOptions 同构）；
 * 其他引擎恒为 null。topology 缺省/空 = 单机 */
export interface RedisExtraOptions {
  topology: 'single' | 'cluster' | 'sentinel';
  /** 集群种子节点 / 哨兵地址（"host:port" 每行一个） */
  nodes?: string[];
  /** 哨兵 master 名（sentinel 必填） */
  masterName?: string;
  /** mTLS 客户端证书 PEM 路径（与 clientKeyPath 成对，仅 SSL ≠ disable 时生效） */
  clientCertPath?: string;
  clientKeyPath?: string;
}

export interface DbConnectionRecord {
  id: number;
  name: string;
  engine: DbEngineKind;
  /** OceanBase 专有：租户模式 'mysql' | 'oracle'（其他引擎恒 null） */
  mode: string | null;
  host: string;
  port: number;
  /** MySQL/PG=库名；Oracle=初始 schema；Redis=逻辑库（db0..dbN，缺省 db0） */
  database: string | null;
  /** Oracle 专有：服务名（连库必需）。database 在 Oracle 语义 = 初始 schema；Redis 恒为 null */
  serviceName: string | null;
  /** SQLite 专有：数据库文件路径（绝对路径或 file: URI）；null = 内存库 */
  dbPath: string | null;
  /** SQLite 专有加密方式（仅 engine='sqlite' 有意义）；其它引擎恒为 'none' */
  encryptionMode: SqliteEncryptionMode;
  /** 仅达梦有意义：客户端侧方言选择（服务器行为由 dm.ini 的 COMPATIBLE_MODE 决定）。
 *  null/undefined = 按 oracle 处理。 */
  compatibleMode: 'oracle' | 'mysql' | null;
  /** 引擎专属扩展配置（JSON 文本，当前仅 Redis 消费，见 RedisExtraOptions）；
 *  其他引擎恒为 null */
  extraOptions: string | null;
  username: string | null;
  /** null = 更新时保留库中旧值（表单未改动不回传）；明文存 SQLCipher 库。
   * engine='sqlite' 时语义 = 加密口令（原始密钥模式为 64 位十六进制） */
  password: string | null;
  sslMode: 'disable' | 'prefer' | 'require' | 'verify-full';
  sshHostId: number | null;
  envLevel: string;
  readonly: boolean;
  color: string | null;
  createdAt: number;
  updatedAt: number;
}

export function dbConnectionList(): Promise<DbConnectionRecord[]> {
  return invoke('db_connection_list');
}

/** id=0 新增、id>0 更新；password=null 的更新保留库中旧密码。 */
export function dbConnectionSave(record: DbConnectionRecord): Promise<DbConnectionRecord> {
  return invoke('db_connection_save', { record });
}

export function dbConnectionDelete(id: number): Promise<void> {
  return invoke('db_connection_delete', { id });
}

export interface DbConnectionTestResult {
  ok: boolean;
  elapsedMs: number;
  serverVersion: string | null;
  message: string | null;
}

/** 连接测试（未保存的草稿也可测）：建连 → 取版本 → 立即断开。 */
export function dbConnectionTest(
  record: DbConnectionRecord,
  connectionId?: number,
): Promise<DbConnectionTestResult> {
  return invoke('db_connection_test', { record, connectionId });
}

export interface DbSessionMeta {
  sessionId: string;
  connectionId: number;
  name: string;
  engine: DbEngineKind;
  /** OceanBase 专有：租户模式（'mysql'/'oracle'）；Redis 携带拓扑
 *  （'cluster'/'sentinel'，单机 null，前端据此禁用集群切库）；其余 null */
  mode: string | null;
  /** 达梦的客户端侧兼容模式；其他引擎为 null（前端据此选方言语法） */
  compatibleMode: 'oracle' | 'mysql' | null;
  /** engine='sqlite' 时固定为 'main'（SQLite 无连接级库概念，对象树顶层为已附加库） */
  database: string | null;
  serverVersion: string;
  readonly: boolean;
  envLevel: string;
}

export function dbConnect(connectionId: number): Promise<DbSessionMeta> {
  return invoke('db_connect', { connectionId });
}

export function dbDisconnect(sessionId: string): Promise<void> {
  return invoke('db_disconnect', { sessionId });
}

export interface CatalogNode {
  key: string;
  name: string;
  /** database（MySQL / Redis 顶层）| schema（PG 顶层）| group | table | view
   *  | mview（物化视图）| function | procedure | sequence | trigger | column；
   *  group 的 key 形如 `{schema}#{group}`；Redis 的 table 节点 detail 承载
   *  值类型（string/hash/list/set/zset/stream） */
  kind:
    | 'database'
    | 'schema'
    | 'group'
    | 'table'
    | 'view'
    | 'mview'
    | 'function'
    | 'procedure'
    | 'sequence'
    | 'trigger'
    | 'column';
  detail: string | null;
}

/** 目录父层参数；null = 根层。 */
export type CatalogParentArg =
  | { kind: 'schema'; schema: string }
  | {
      kind: 'group';
      schema: string;
      group: 'table' | 'view' | 'mview' | 'function' | 'procedure' | 'sequence' | 'trigger';
    }
  | { kind: 'table'; schema: string; table: string };

export function dbCatalog(
  sessionId: string,
  parent: CatalogParentArg | null,
): Promise<CatalogNode[]> {
  return invoke('db_catalog', { sessionId, parent });
}

/** 流式查询事件（后端按 300 行/批发来，与 Rust QueryEvent 的 tag=type 对齐）。 */
export type DbQueryEvent =
  | { type: 'started'; statementCount: number }
  | { type: 'statementStart'; index: number; sql: string }
  | { type: 'columns'; index: number; columns: string[] }
  | { type: 'rows'; index: number; rows: string[][]; total: number }
  | {
      type: 'statementDone';
      index: number;
      affected: number | null;
      rowsRead: number;
      truncated: boolean;
      cancelled: boolean;
      elapsedMs: number;
    }
  | { type: 'statementError'; index: number; message: string }
  | { type: 'done'; elapsedMs: number; cancelled: boolean };

export interface DbQueryOptions {
  maxRows?: number;
  timeoutMs?: number;
}

/** 执行 SQL（可多语句）；事件经 Tauri Channel 流式回传。 */
export function dbQuery(
  sessionId: string,
  sql: string,
  onEvent: (event: DbQueryEvent) => void,
  options?: DbQueryOptions,
): Promise<void> {
  const channel = new Channel<DbQueryEvent>();
  channel.onmessage = onEvent;
  return invoke('db_query', {
    sessionId,
    sql,
    maxRows: options?.maxRows,
    timeoutMs: options?.timeoutMs,
    onEvent: channel,
  });
}

export function dbQueryCancel(sessionId: string): Promise<void> {
  return invoke('db_query_cancel', { sessionId });
}

/** 可切换的数据库清单（MySQL=全部 schema；PG=服务器上可连的库）。 */
export function dbListDatabases(sessionId: string): Promise<string[]> {
  return invoke('db_list_databases', { sessionId });
}

/** 切换当前数据库（MySQL=USE；PG=重连替换引擎），返回更新后的会话元信息。 */
export function dbUseDatabase(sessionId: string, database: string): Promise<DbSessionMeta> {
  return invoke('db_use_database', { sessionId, database });
}

/** 用表单草稿临时建连拉取可选数据库清单（拉完即断）；编辑态密码留空时后端从库补。 */
export function dbListDatabasesForRecord(
  record: DbConnectionRecord,
  connectionId?: number,
): Promise<string[]> {
  return invoke('db_list_databases_for_record', { record, connectionId });
}

/** 函数/过程/序列的定义与调用骨架。argCount > 0 时前端引导补参后再执行（序列恒为 0）。 */
export interface ObjectDefinition {
  definition: string;
  callTemplate: string;
  argCount: number;
}

/** 拉取函数/过程/序列/触发器的完整定义与调用骨架。 */
export function dbObjectDefinition(
  sessionId: string,
  schema: string,
  name: string,
  kind: 'function' | 'procedure' | 'sequence' | 'trigger',
): Promise<ObjectDefinition> {
  return invoke('db_object_definition', { sessionId, schema, name, kind });
}

/** 表/视图详情：DDL + 列结构 + 索引清单 + 约束清单（树右键「表详情」）。 */
export interface TableDetail {
  ddl: string;
  columnHeaders: string[];
  columns: string[][];
  indexHeaders: string[];
  indexes: string[][];
  /** 约束（PK/U/F/C）：name / type / columns | definition / references */
  constraintHeaders: string[];
  constraints: string[][];
}

export function dbTableDetail(
  sessionId: string,
  schema: string,
  table: string,
  kind: 'table' | 'view',
): Promise<TableDetail> {
  return invoke('db_table_detail', { sessionId, schema, table, kind });
}

// ── Redis 键管理与诊断 ─────────────────────────────────────────────────────

/** 键结构化读（键编辑对话框）：entries 按 kind 呈 {field,value}（hash）
 *  或 {member,score}（zset）；truncated=true 表示成员超上限被截断，
 *  前端禁整体覆写（DEL+重写语义下截断保存会丢数据） */
export interface RedisKeyData {
  kind: 'string' | 'hash' | 'list' | 'set' | 'zset' | 'stream' | 'none';
  /** -1 永不过期；-2 key 不存在 */
  ttl: number;
  truncated: boolean;
  value?: string;
  entries?: { field: string; value: string }[] | { member: string; score: string }[];
  items?: string[];
  members?: string[];
}

/** 键整体覆写载荷（与后端 RedisKeyWrite 的 serde 判别形态对齐） */
export type RedisKeyWritePayload =
  | { kind: 'string'; value: string }
  | { kind: 'hash'; entries: { field: string; value: string }[] }
  | { kind: 'list'; items: string[] }
  | { kind: 'set'; members: string[] }
  | { kind: 'zset'; entries: { member: string; score: string }[] };

/** 表格型返回（慢日志）：headers/rows 直入 DataTable */
export interface RedisTableData {
  headers: string[];
  rows: string[][];
}

/** 大 key 分析结果：scanned = 采样范围内实扫 key 数 */
export interface RedisBigkeysData extends RedisTableData {
  scanned: number;
  truncated: boolean;
}

/** 键结构化读；db = 键所属逻辑库（树节点携带的 dbN），缺省用会话当前库 */
export function redisReadKey(
  sessionId: string,
  key: string,
  db?: string,
): Promise<RedisKeyData> {
  return invoke('redis_read_key', { sessionId, key, db });
}

/** 键整体覆写（原子管道；keepTtl 保留原过期时间）；返回新成员数。
 *  只读会话后端拒绝。 */
export function redisWriteKey(
  sessionId: string,
  key: string,
  payload: RedisKeyWritePayload,
  keepTtl: boolean,
  db?: string,
): Promise<number> {
  return invoke('redis_write_key', { sessionId, key, payload, keepTtl, db });
}

/** 慢日志视图（SLOWLOG GET 结构化解析）；count 缺省 100 */
export function redisSlowlog(sessionId: string, count?: number): Promise<RedisTableData> {
  return invoke('redis_slowlog', { sessionId, count });
}

/** 大 key 分析（SCAN 采样 + 批量 MEMORY USAGE，按内存倒序取前 N） */
export function redisBigkeys(
  sessionId: string,
  sample?: number,
  topN?: number,
): Promise<RedisBigkeysData> {
  return invoke('redis_bigkeys', { sessionId, sample, topN });
}

// ── SQLite 数据库文件选择（rfd）──

/** 选择本地 SQLite 数据库文件；取消返回 null */
export function sqliteFilePick(): Promise<string | null> {
  return invoke('sqlite_file_pick');
}
