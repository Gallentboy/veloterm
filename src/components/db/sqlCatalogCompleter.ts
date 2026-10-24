/**
 * SqlSchemaCompleter 的 dbCatalog 数据源实现（表/列 schema 感知补全）。
 *
 * 三层缓存（Promise 级：in-flight 去重与成功结果共用一个槽位）：
 * - 顶层 schema/database 名单：单值缓存；
 * - 每库表清单（table/view/mview/synonym 分组并集）：按库名缓存——切库后
 *   getTables() 缺省参数即新库，自动另拉不误；
 * - 每表列清单：按 `schema.table`（小写）缓存，补全首次触达某表才拉。
 * 失败不缓存（catch 里回滚槽位），下次补全重试；invalidate() 清全部，
 * 由 DbSessionView 在对象树 catalogLoad（连接/切库/刷新）时调用。
 *
 * 与对象树的预载（树自己也会拉全当前库）刻意不共享：树数据是视图态
 * （搜索过滤/剪枝/竞态守卫），补全要的是稳定快照，两份请求各查一次
 * 各自缓存，代价一次多余往返，换来互不牵连。
 */

import { dbCatalog } from '@/services/dbConnectionsService';
import type {
  SchemaColumn,
  SchemaTable,
  SqlSchemaCompleter,
} from './sqlCompletion';

/** 表类分组（CatalogParentArg 的 group 并集子集；其余分组不参与表补全） */
const TABLE_GROUPS = new Set(['table', 'view', 'mview', 'synonym']);

export class CatalogCompleter implements SqlSchemaCompleter {
  private sessionId: () => string | null;
  private currentDb: () => string;
  private schemas: Promise<string[]> | null = null;
  private readonly tables = new Map<string, Promise<SchemaTable[]>>();
  private readonly columns = new Map<string, Promise<SchemaColumn[]>>();

  constructor(sessionId: () => string | null, currentDb: () => string) {
    this.sessionId = sessionId;
    this.currentDb = currentDb;
  }

  currentSchema(): string {
    return this.currentDb();
  }

  getSchemas(): Promise<string[]> {
    const sid = this.sessionId();
    if (!sid) return Promise.resolve([]);
    if (!this.schemas) {
      this.schemas = dbCatalog(sid, null)
        .then((nodes) =>
          nodes
            .filter((n) => n.kind === 'database' || n.kind === 'schema')
            .map((n) => n.name),
        )
        .catch(() => {
          // 失败不缓存：下次补全重试
          this.schemas = null;
          return [] as string[];
        });
    }
    return this.schemas;
  }

  getTables(schema?: string): Promise<SchemaTable[]> {
    const sid = this.sessionId();
    const db = schema ?? this.currentDb();
    if (!sid || !db) return Promise.resolve([]);
    const key = db.toLowerCase();
    let cached = this.tables.get(key);
    if (!cached) {
      cached = (async () => {
        const groups = await dbCatalog(sid, { kind: 'schema', schema: db });
        // group 规范名在 key 的 `#` 后段（小写；name 是大写展示名）
        const wanted = groups
          .map((g) => (g.key.split('#')[1] ?? '').toLowerCase())
          .filter((g) => TABLE_GROUPS.has(g));
        const lists = await Promise.all(
          wanted.map((group) =>
            dbCatalog(sid, {
              kind: 'group',
              schema: db,
              group: group as 'table' | 'view' | 'mview' | 'synonym',
            }).catch(() => []),
          ),
        );
        return lists.flatMap((nodes) =>
          nodes.map((n) => ({ schema: db, name: n.name, kind: n.kind })),
        );
      })().catch(() => {
        this.tables.delete(key);
        return [] as SchemaTable[];
      });
      this.tables.set(key, cached);
    }
    return cached;
  }

  getColumns(schema: string, table: string): Promise<SchemaColumn[]> {
    const sid = this.sessionId();
    if (!sid || !table) return Promise.resolve([]);
    const key = `${(schema || this.currentDb()).toLowerCase()}.${table.toLowerCase()}`;
    let cached = this.columns.get(key);
    if (!cached) {
      cached = dbCatalog(sid, {
        kind: 'table',
        schema: schema || this.currentDb(),
        table,
      })
        .then((nodes) =>
          nodes
            .filter((n) => n.kind === 'column')
            .map((n) => ({ name: n.name, dataType: n.detail ?? '' })),
        )
        .catch(() => {
          this.columns.delete(key);
          return [] as SchemaColumn[];
        });
      this.columns.set(key, cached);
    }
    return cached;
  }

  /** 清全部缓存（对象树 catalogLoad：连接/切库/刷新时调用） */
  invalidate(): void {
    this.schemas = null;
    this.tables.clear();
    this.columns.clear();
  }
}
