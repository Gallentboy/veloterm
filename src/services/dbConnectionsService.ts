/**
 * 数据库工作台连接库：侧栏「数据库主机」页 CRUD + 会话/目录/查询 invoke 封装。
 * 类型与后端 db_browser 模块的 serde camelCase 契约一一对应。
 */

import { Channel, invoke } from '@tauri-apps/api/core';

export type DbEngineKind = 'mysql' | 'postgres' | 'oracle' | 'oceanbase';

export interface DbConnectionRecord {
  id: number;
  name: string;
  engine: DbEngineKind;
  /** OceanBase 专有：租户模式 'mysql' | 'oracle'（其他引擎恒 null） */
  mode: string | null;
  host: string;
  port: number;
  database: string | null;
  /** Oracle 专有：服务名（连库必需）。database 在 Oracle 语义 = 初始 schema */
  serviceName: string | null;
  username: string | null;
  /** null = 更新时保留库中旧值（表单未改动不回传）；明文存 SQLCipher 库 */
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
  /** OceanBase 专有：租户模式 'mysql' | 'oracle'（其他引擎 null） */
  mode: string | null;
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
  /** database（MySQL 顶层）| schema（PG 顶层）| group | table | view
   *  | function | procedure | sequence | column；group 的 key 形如 `{schema}#{group}` */
  kind: 'database' | 'schema' | 'group' | 'table' | 'view' | 'function' | 'procedure' | 'sequence' | 'column';
  detail: string | null;
}

/** 目录父层参数；null = 根层。 */
export type CatalogParentArg =
  | { kind: 'schema'; schema: string }
  | { kind: 'group'; schema: string; group: 'table' | 'view' | 'function' | 'procedure' | 'sequence' }
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

/** 拉取函数/过程/序列（kind: 'function' | 'procedure' | 'sequence'）的完整定义与调用骨架。 */
export function dbObjectDefinition(
  sessionId: string,
  schema: string,
  name: string,
  kind: 'function' | 'procedure' | 'sequence',
): Promise<ObjectDefinition> {
  return invoke('db_object_definition', { sessionId, schema, name, kind });
}

/** 表/视图详情：DDL + 列结构 + 索引清单（树右键「表详情」）。 */
export interface TableDetail {
  ddl: string;
  columnHeaders: string[];
  columns: string[][];
  indexHeaders: string[];
  indexes: string[][];
}

export function dbTableDetail(
  sessionId: string,
  schema: string,
  table: string,
  kind: 'table' | 'view',
): Promise<TableDetail> {
  return invoke('db_table_detail', { sessionId, schema, table, kind });
}

// ── Oracle 客户端库目录（运行期 dlopen 的动态绑定；进程级一次初始化）──

/** 读 Oracle Instant Client 目录（null = 未设置，走系统搜索路径） */
export function oracleClientDirGet(): Promise<string | null> {
  return invoke('oracle_client_dir_get');
}

/** 写/清除目录（null = 清除）。注意：本进程已初始化过 Oracle 客户端时，
 * 新目录要重启应用才生效（rust-oracle 的 GLOBAL_CONTEXT 进程级一次语义） */
export function oracleClientDirSet(dir: string | null): Promise<void> {
  return invoke('oracle_client_dir_set', { dir });
}

/** 目录选择对话框（rfd）：取消返回 null */
export function oracleClientDirPick(): Promise<string | null> {
  return invoke('oracle_client_dir_pick');
}
