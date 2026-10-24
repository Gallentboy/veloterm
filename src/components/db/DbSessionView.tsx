/**
 * 单个数据库会话视图：连接工具栏 + 左侧对象树（三层懒加载：库/schema →
 * 表/视图 → 列；连接/切库后自动预载当前库的分组与对象层，树内搜索为纯内存过滤）+
 * Monaco SQL 编辑器 + 流式结果面板（多结果集 Tab）。
 * 挂载时消费 pendingConnectionId 自动连接（仅一次，防 StrictMode 双建）；
 * 卸载即断开会话。全部会话状态内部留存（tab 常驻隐藏切换不丢）。
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  AlignLeft,
  AlertCircle,
  Boxes,
  Braces,
  Cable,
  ChevronRight,
  Clock,
  Copy,
  Database,
  Eye,
  Folder,
  Globe,
  HardDrive,
  Layers,
  Link,
  ListOrdered,
  Loader2,
  Package,
  Play,
  Puzzle,
  RefreshCw,
  ScrollText,
  Search,
  Sigma,
  Square,
  Table2,
  ShieldCheck,
  Trash2,
  Type,
  User,
  Unplug,
  X,
  Zap,
} from 'lucide-react';

import { Button } from '@/components/ui/button';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { DataTable } from '@/components/ui/DataTable';
import { SelectMenu } from '@/components/ui/SelectMenu';
import { SqlEditor } from '@/components/db/SqlEditor';
import {
  RedisBigkeysModal,
  RedisKeyDeleteModal,
  RedisKeyEditorModal,
  RedisSlowlogModal,
  RedisTtlModal,
  redisQuote,
} from '@/components/db/RedisKeyDialogs';
import { useDismissOnOutsidePointer } from '@/hooks/useDismissOnOutsidePointer';
import { createPortal } from 'react-dom';
import {
  dbCatalog,
  dbConnect,
  dbDisconnect,
  dbListDatabases,
  dbObjectDefinition,
  dbQuery,
  dbQueryCancel,
  dbTableDetail,
  dbUseDatabase,
  type CatalogNode,
  type CatalogParentArg,
  type DbConnectionRecord,
  type DbQueryEvent,
  type DbSessionMeta,
  type ObjectDefinition,
  type TableDetail,
} from '@/services/dbConnectionsService';
import { cn } from '@/lib/utils';

/** 树栏宽度（同 ZK 树的可拖语义）：默认 280、最小 200、最大 480 */
const TREE_DEFAULT_WIDTH = 280;
const TREE_MIN_WIDTH = 200;
const TREE_MAX_WIDTH = 480;

interface TreeBranch {
  node: CatalogNode;
  expanded: boolean;
  /** null = 未加载（展开时懒拉） */
  children: CatalogNode[] | null;
  loading: boolean;
}

/** 一条语句的结果（结果集或 DML 摘要/错误） */
interface StatementResult {
  index: number;
  sql: string;
  columns: string[];
  rows: string[][];
  affected: number | null;
  truncated: boolean;
  elapsedMs: number;
  error: string | null;
}

const ROW_CAP_OPTIONS = [1000, 5000, 20000, 50000];
const DEFAULT_TIMEOUT_MS = 60_000;

/** Redis key → 最自然的查看命令（树双击、编辑保存后的回看共用） */
function redisViewCommand(key: string, type: string): string {
  const k = redisQuote(key);
  switch (type) {
    case 'hash':
      return `HGETALL ${k}`;
    case 'list':
      return `LRANGE ${k} 0 -1`;
    case 'set':
      return `SMEMBERS ${k}`;
    case 'zset':
      return `ZRANGE ${k} 0 -1 WITHSCORES`;
    case 'stream':
      return `XRANGE ${k} - +`;
    default:
      return `GET ${k}`;
  }
}

/** 可展开子层的节点类型（分组/表/视图可展开；函数/过程/序列/列是叶子） */
const EXPANDABLE_KINDS = new Set(['database', 'schema', 'group', 'table', 'view', 'mview']);

/** 搜索匹配的对象类节点（分组/库节点经由子树内命中间接显示） */
const OBJECT_KINDS = new Set([
  'table',
  'view',
  'mview',
  'ftable',
  'extension',
  'dblink',
  'recycle',
  'datatype',
  'tablespace',
  'user',
  'role',
  'function',
  'procedure',
  'sequence',
  'trigger',
  'synonym',
  'package',
  'event',
  'rule',
]);

/** OB 的 Oracle 租户模式与 Oracle 同方言：SELECT 需 FROM DUAL、无 LIMIT
 * （用 ROWNUM）、标识符不用反引号；OB MySQL 租户模式则完全走 MySQL 方言。 */
function isOracleDialect(
  engine: DbSessionMeta['engine'],
  mode: string | null,
  compatibleMode: string | null = null,
): boolean {
  // 达梦的兼容模式在服务器侧由 dm.ini 决定、客户端不感知，这里按表单选择的
  // compatibleMode 判定（null/非 mysql 均归 oracle——达梦默认部署多为 Oracle 兼容）
  return (
    engine === 'oracle' ||
    (engine === 'oceanbase' && mode === 'oracle') ||
    (engine === 'dameng' && compatibleMode !== 'mysql')
  );
}

function nodeIcon(kind: CatalogNode['kind'], className?: string) {
  switch (kind) {
    case 'database':
      return <Database className={cn('size-3.5 shrink-0 text-terminal-cyan', className)} />;
    case 'schema':
      return <Boxes className={cn('size-3.5 shrink-0 text-terminal-cyan', className)} />;
    case 'group':
      return <Folder className={cn('size-3.5 shrink-0 text-terminal-blue', className)} />;
    case 'view':
      return <Eye className={cn('size-3.5 shrink-0 text-terminal-blue', className)} />;
    case 'mview':
      return <Layers className={cn('size-3.5 shrink-0 text-terminal-blue', className)} />;
    case 'ftable':
      return <Globe className={cn('size-3.5 shrink-0 text-terminal-blue', className)} />;
    case 'extension':
      return <Puzzle className={cn('size-3.5 shrink-0 text-terminal-cyan', className)} />;
    case 'dblink':
      return <Cable className={cn('size-3.5 shrink-0 text-terminal-yellow', className)} />;
    case 'recycle':
      return <Trash2 className={cn('size-3.5 shrink-0 text-foreground-subtlest', className)} />;
    case 'datatype':
      return <Type className={cn('size-3.5 shrink-0 text-terminal-magenta', className)} />;
    case 'tablespace':
      return <HardDrive className={cn('size-3.5 shrink-0 text-terminal-cyan', className)} />;
    case 'user':
      return <User className={cn('size-3.5 shrink-0 text-terminal-green', className)} />;
    case 'role':
      return <ShieldCheck className={cn('size-3.5 shrink-0 text-terminal-yellow', className)} />;
    case 'function':
      return <Sigma className={cn('size-3.5 shrink-0 text-terminal-magenta', className)} />;
    case 'procedure':
      return <Braces className={cn('size-3.5 shrink-0 text-terminal-magenta', className)} />;
    case 'sequence':
      return <ListOrdered className={cn('size-3.5 shrink-0 text-terminal-yellow', className)} />;
    case 'trigger':
      return <Zap className={cn('size-3.5 shrink-0 text-terminal-red', className)} />;
    case 'synonym':
      return <Link className={cn('size-3.5 shrink-0 text-terminal-cyan', className)} />;
    case 'package':
      return <Package className={cn('size-3.5 shrink-0 text-terminal-yellow', className)} />;
    case 'event':
      return <Clock className={cn('size-3.5 shrink-0 text-terminal-blue', className)} />;
    case 'rule':
      return <ScrollText className={cn('size-3.5 shrink-0 text-terminal-magenta', className)} />;
    case 'column':
      return <AlignLeft className={cn('size-3 shrink-0 text-foreground-subtlest', className)} />;
    default:
      return <Table2 className={cn('size-3.5 shrink-0 text-terminal-green', className)} />;
  }
}

export function DbSessionView({
  pendingConnection,
  onBaseTitle,
}: {
  /** 建 tab 时携带的待连连接（数据库主机页「连接」入口）；挂载时消费一次 */
  pendingConnection: DbConnectionRecord | null;
  onBaseTitle: (title: string | null) => void;
}) {
  const { t } = useTranslation();
  const [meta, setMeta] = useState<DbSessionMeta | null>(null);
  const [connecting, setConnecting] = useState(false);
  const [connectError, setConnectError] = useState<string | null>(null);

  // 对象树：扁平 key -> branch + 根层 key 序列
  const [roots, setRoots] = useState<string[]>([]);
  const [branches, setBranches] = useState<Map<string, TreeBranch>>(new Map());

  const [sql, setSql] = useState('SELECT 1;');
  const [running, setRunning] = useState(false);
  const [results, setResults] = useState<StatementResult[]>([]);
  const [activeResult, setActiveResult] = useState(0);
  const [rowCap, setRowCap] = useState(5000);

  // 当前数据库下拉（可切换的库清单 + 切换中状态）
  const [databases, setDatabases] = useState<string[]>([]);
  const [switchingDb, setSwitchingDb] = useState(false);

  // 对象树搜索词（纯内存过滤，仅作用于当前选中库的已加载对象）；
  // searchCollapsed 记录搜索态下用户手动收起的分支（仅展示层，清空搜索即复位）
  const [treeSearch, setTreeSearch] = useState('');
  const [searchCollapsed, setSearchCollapsed] = useState<Set<string>>(new Set());

  // 函数/过程定义视图（对象树双击拉取；新查询执行时让位给结果面板）
  const [objectDef, setObjectDef] = useState<{
    name: string;
    kind:
      | 'function'
      | 'procedure'
      | 'sequence'
      | 'trigger'
      | 'synonym'
      | 'package'
      | 'event'
      | 'rule'
      | 'ftable'
      | 'extension'
      | 'dblink'
      | 'recycle'
      | 'datatype'
      | 'tablespace'
      | 'user'
      | 'role';
    def: ObjectDefinition;
  } | null>(null);
  const [defLoading, setDefLoading] = useState(false);
  const [defCopied, setDefCopied] = useState(false);
  const [defNotice, setDefNotice] = useState<string | null>(null);

  // 表/视图详情视图（DDL / 列 / 索引三 Tab；树右键打开）
  const [tableDetail, setTableDetail] = useState<{
    name: string;
    kind: 'table' | 'view';
    detail: TableDetail;
    tab: 'ddl' | 'columns' | 'indexes' | 'constraints' | 'references';
  } | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [ddlCopied, setDdlCopied] = useState(false);

  // 树节点右键菜单（fixed 定位，坐标取 contextmenu 事件）
  const [contextMenu, setContextMenu] = useState<{
    x: number;
    y: number;
    node: CatalogNode;
  } | null>(null);

  // Redis 键管理/诊断对话框（editor/ttl/delete 携带键作用域；node.key 的
  // `.` 前段是逻辑库 dbN——key 本身可含点，但 schema 不含）
  const [redisDialog, setRedisDialog] = useState<
    | { kind: 'editor' | 'ttl' | 'delete'; keyName: string; keyDb: string; keyType: string }
    | { kind: 'slowlog' | 'bigkeys' }
    | null
  >(null);
  const contextMenuRef = useRef<HTMLDivElement | null>(null);
  useDismissOnOutsidePointer(
    contextMenu != null,
    [contextMenuRef],
    () => setContextMenu(null),
  );

  const [treeWidth, setTreeWidth] = useState(TREE_DEFAULT_WIDTH);
  const resizeRef = useRef<{ startX: number; startWidth: number } | null>(null);

  const sessionIdRef = useRef<string | null>(null);

  const catalogLoad = useCallback(async (sessionId: string, currentDb?: string | null) => {
    setBranches(new Map());
    setRoots([]);
    setTreeSearch('');
    try {
      const nodes = await dbCatalog(sessionId, null);
      const map = new Map<string, TreeBranch>();
      const keys: string[] = [];
      for (const node of nodes) {
        map.set(node.key, { node, expanded: false, children: null, loading: false });
        keys.push(node.key);
      }
      setBranches(map);
      setRoots(keys);

      // 预载当前库（连接 / 切库 / 刷新后）：库 → 分组 → 对象两层拉全进内存供搜索过滤，
      // 分支保持折叠（展开态不变）；单组拉取失败降级为该组懒加载，整层失败静默
      const dbNode =
        currentDb &&
        nodes.find(
          (n) => (n.kind === 'database' || n.kind === 'schema') && n.name === currentDb,
        );
      if (dbNode) {
        try {
          const groups = await dbCatalog(sessionId, { kind: 'schema', schema: dbNode.name });
          // group 取 key 的 `#` 后段（与手动展开同源）：SQL 引擎分组节点的 name 是大写
          // TABLE 而 ObjectGroup::parse 只认小写，直接传 name 会整组拉空
          const results = await Promise.all(
            groups.map((g) => {
              const group = g.key.split('#')[1] ?? g.name;
              return dbCatalog(
                sessionId,
                { kind: 'group', schema: dbNode.name, group } as CatalogParentArg,
              )
                .then((children) => ({ g, children }))
                .catch(() => null);
            }),
          );
          setBranches((current) => {
            // 树已被后续操作重建或用户已手动展开（切库/刷新竞态）则放弃合并
            const dbBranch = current.get(dbNode.key);
            if (!dbBranch || dbBranch.children != null) return current;
            const next = new Map(current);
            next.set(dbNode.key, { ...dbBranch, children: groups });
            for (const r of results) {
              // 单组预载失败：children 保持 null，展开时走懒加载兜底（不写入空数组短路）
              if (!r) continue;
              const { g, children } = r;
              const existing = next.get(g.key);
              if (existing) {
                next.set(g.key, { ...existing, children });
              } else {
                next.set(g.key, { node: g, expanded: false, children, loading: false });
              }
              for (const obj of children) {
                if (!next.has(obj.key)) {
                  next.set(obj.key, { node: obj, expanded: false, children: null, loading: false });
                }
              }
            }
            return next;
          });
        } catch {
          // 预载失败不阻塞：分支保持懒加载语义
        }
      }
    } catch (e) {
      setConnectError(String(e));
    }
  }, []);

  const connect = useCallback(
    async (connection: DbConnectionRecord) => {
      if (connecting) return;
      setConnecting(true);
      setConnectError(null);
      try {
        const sessionMeta = await dbConnect(connection.id);
        sessionIdRef.current = sessionMeta.sessionId;
        setMeta(sessionMeta);
        // 引擎适配的欢迎语句：Oracle 方言无 FROM 不可查（SELECT 1 报 ORA-00923），
        // 换 SELECT 1 FROM DUAL（含 OB Oracle 租户模式与达梦 Oracle 兼容模式）；
        // Redis 不认 SQL，换 PING。函数式 setSql 读最新值，仅初始占位未动时替换。
        // SQLite/MySQL/PG 的 SELECT 1 均合法，沿用默认占位
        if (
          isOracleDialect(
            sessionMeta.engine,
            sessionMeta.mode,
            sessionMeta.compatibleMode,
          )
        ) {
          setSql((prev) => (prev === 'SELECT 1;' ? 'SELECT 1 FROM DUAL;' : prev));
        } else if (sessionMeta.engine === 'redis') {
          setSql((prev) => (prev === 'SELECT 1;' ? 'PING' : prev));
        }
        onBaseTitle(`${sessionMeta.name} · ${connection.host}`);
        // 可切换库清单与对象树并行拉取
        void dbListDatabases(sessionMeta.sessionId)
          .then(setDatabases)
          .catch(() => {
            // 清单拉取失败不阻塞会话（下拉退化为仅当前库）
            setDatabases(sessionMeta.database ? [sessionMeta.database] : []);
          });
        await catalogLoad(sessionMeta.sessionId, sessionMeta.database);
      } catch (e) {
        setConnectError(String(e));
        onBaseTitle(connection.name);
      } finally {
        setConnecting(false);
      }
    },
    [catalogLoad, connecting, onBaseTitle],
  );

  // 挂载时消费一次待连连接（StrictMode 双 effect 防重：以 ref 占位）
  const consumedRef = useRef(false);
  useEffect(() => {
    if (consumedRef.current || !pendingConnection) return;
    consumedRef.current = true;
    void connect(pendingConnection);
    return undefined;
  }, [pendingConnection, connect]);

  // 卸载断开
  useEffect(() => {
    return () => {
      const sid = sessionIdRef.current;
      if (sid) {
        void dbDisconnect(sid).catch(() => undefined);
        sessionIdRef.current = null;
      }
    };
  }, []);

  const toggleBranch = useCallback(
    async (key: string) => {
      const sessionId = sessionIdRef.current;
      if (!sessionId) return;
      const branch = branches.get(key);
      if (!branch) return;

      if (!branch.expanded && branch.children == null && !branch.loading) {
        // 懒加载子层
        setBranches((current) => {
          const next = new Map(current);
          const b = next.get(key);
          if (b) next.set(key, { ...b, loading: true });
          return next;
        });
        try {
          let parent: Parameters<typeof dbCatalog>[1] = null;
          if (branch.node.kind === 'database' || branch.node.kind === 'schema') {
            parent = { kind: 'schema', schema: branch.node.name };
          } else if (branch.node.kind === 'group') {
            // group 的 key 形如 `{schema}#{group}`（后端约定）
            const [schema, group] = branch.node.key.split('#');
            if (!schema || !group) return;
            parent = { kind: 'group', schema, group } as CatalogParentArg;
          } else {
            // 表/视图 → 列：key 是 "schema.table"
            const schema = branch.node.key.split('.')[0] ?? branch.node.name;
            parent = { kind: 'table', schema, table: branch.node.name };
          }
          const children = await dbCatalog(sessionId, parent);
          setBranches((current) => {
            const next = new Map(current);
            const b = next.get(key);
            if (b) {
              next.set(key, { ...b, expanded: true, children, loading: false });
              for (const child of children) {
                if (!next.has(child.key)) {
                  next.set(child.key, {
                    node: child,
                    expanded: false,
                    children: null,
                    loading: false,
                  });
                }
              }
            }
            return next;
          });
        } catch (e) {
          setConnectError(String(e));
          setBranches((current) => {
            const next = new Map(current);
            const b = next.get(key);
            if (b) next.set(key, { ...b, loading: false });
            return next;
          });
        }
      } else {
        setBranches((current) => {
          const next = new Map(current);
          const b = next.get(key);
          if (b) next.set(key, { ...b, expanded: !b.expanded });
          return next;
        });
      }
    },
    [branches],
  );

  // 点击表/视图节点 → 生成查询语句；双击函数/过程 → 拉取定义展示
  // 方言差异：标识符引用 MySQL 反引号、SQLite 双引号（PG/Oracle 保持不加引号的
  // 既有手感）；限行子句 Oracle 用 ROWNUM、MySQL/PG/SQLite 均用 LIMIT 200
  const handleNodeClick = useCallback(
    (node: CatalogNode) => {
      // Redis：table 节点即 key，detail 携带值类型（string/hash/...），据此出命令。
      // node.name 是裸 key（node.key 形如 "db0.{key}"，不能按 . 切分）
      if (meta?.engine === 'redis' && node.kind === 'table') {
        setSql(redisViewCommand(node.name, node.detail ?? 'string'));
        return;
      }
      if (node.kind === 'table' || node.kind === 'view') {
        const schema = node.key.split('.')[0] ?? '';
        const name = node.name;
        // 标识符引用按方言：MySQL（含 OB MySQL 租户模式与达梦 MySQL 兼容模式）
        // 反引号；SQLite 双引号（PG/Oracle 现有行为不加引号，保持原样不动，
        // 避免影响既有会话手感）
        const mysqlDialect =
          meta?.engine === 'mysql' ||
          (meta?.engine === 'oceanbase' && meta?.mode !== 'oracle') ||
          (meta?.engine === 'dameng' && meta?.compatibleMode === 'mysql');
        const quote = (ident: string) =>
          mysqlDialect
            ? `\`${ident.replace(/`/g, '``')}\``
            : meta?.engine === 'sqlite'
              ? `"${ident.replace(/"/g, '""')}"`
              : ident;
        const target = schema ? `${quote(schema)}.${quote(name)}` : quote(name);
        // 限行子句同样按方言：Oracle 方言（含 OB Oracle 租户模式与达梦 Oracle
        // 兼容模式）无 LIMIT 且 11.x 只认 ROWNUM（FETCH FIRST 需 12c+）
        const limit =
          meta && isOracleDialect(meta.engine, meta.mode, meta.compatibleMode)
            ? 'WHERE ROWNUM <= 200'
            : 'LIMIT 200';
        setSql(`SELECT * FROM ${target} ${limit};`);
        return;
      }
      // 物化视图：双击生成与普通视图同型的 SELECT
      if (node.kind === 'mview') {
        const schema = node.key.split('.')[0] ?? '';
        const name = node.name;
        const mysqlDialect = meta?.engine === 'dameng' && meta?.compatibleMode === 'mysql';
        const quote = (ident: string) =>
          mysqlDialect ? `\`${ident.replace(/`/g, '``')}\`` : ident;
        const target = schema ? `${quote(schema)}.${quote(name)}` : quote(name);
        setSql(`SELECT * FROM ${target} LIMIT 200;`);
        return;
      }
      if (
        node.kind === 'function' ||
        node.kind === 'procedure' ||
        node.kind === 'sequence' ||
        node.kind === 'trigger' ||
        node.kind === 'synonym' ||
        node.kind === 'package' ||
        node.kind === 'event' ||
        node.kind === 'rule' ||
        node.kind === 'ftable' ||
        node.kind === 'extension' ||
        node.kind === 'dblink' ||
        node.kind === 'recycle' ||
        node.kind === 'datatype' ||
        node.kind === 'tablespace' ||
        node.kind === 'user' ||
        node.kind === 'role'
      ) {
        const sessionId = sessionIdRef.current;
        if (!sessionId || defLoading) return;
        const schema = node.key.split('.')[0] ?? '';
        // const 快照：闭包内保持类型收窄（参数 narrowing 不进闭包）
        const kind = node.kind;
        const name = node.name;
        setDefLoading(true);
        setObjectDef(null);
        dbObjectDefinition(sessionId, schema, name, kind)
          .then((def) => setObjectDef({ name, kind, def }))
          .catch((e) => setConnectError(String(e)))
          .finally(() => setDefLoading(false));
      }
    },
    [defLoading, meta],
  );

  /** 右键菜单：表详情（DDL/列/索引三 Tab） */
  const openTableDetail = useCallback(
    (node: CatalogNode) => {
      const sessionId = sessionIdRef.current;
      if (!sessionId || detailLoading) return;
      if (node.kind !== 'table' && node.kind !== 'view') return;
      const schema = node.key.split('.')[0] ?? '';
      // const 快照：闭包内保持类型收窄（参数 narrowing 不进闭包）
      const kind = node.kind;
      const name = node.name;
      setDetailLoading(true);
      setTableDetail(null);
      setObjectDef(null);
      dbTableDetail(sessionId, schema, name, kind)
        .then((detail) => setTableDetail({ name, kind, detail, tab: 'ddl' }))
        .catch((e) => setConnectError(String(e)))
        .finally(() => setDetailLoading(false));
    },
    [detailLoading],
  );

  /**
   * 右键菜单：创建索引骨架（按引擎方言生成，插入编辑器补列名后执行）
   * 方言差异：MySQL 全反引号；SQLite 双引号（索引名也加引号，避免与关键字或
   * 大小写混用时的解析歧义）；PG/Oracle 沿用原有形态
   */
  const insertCreateIndexTemplate = useCallback(
    (node: CatalogNode) => {
      if (node.kind !== 'table' || !meta) return;
      const schema = node.key.split('.')[0] ?? '';
      // OB MySQL 租户模式与 MySQL 同走反引号限定符；其余（PG / Oracle /
      // OB Oracle 租户模式）走双引号
      const mysqlDialect =
        meta.engine === 'mysql' ||
        (meta.engine === 'oceanbase' && meta.mode !== 'oracle');
      const sql =
        mysqlDialect
          ? `CREATE INDEX \`idx_${node.name}\` ON \`${schema}\`.\`${node.name}\` (\`col\`);`
          : meta.engine === 'sqlite'
            ? `CREATE INDEX "idx_${node.name}" ON "${schema}"."${node.name}" (col);`
            : `CREATE INDEX idx_${node.name} ON "${schema}"."${node.name}" (col);`;
      setSql(sql);
    },
    [meta],
  );

  const runQuery = useCallback(
    async (overrideSql?: string) => {
      const sessionId = sessionIdRef.current;
      const text = (overrideSql ?? sql).trim();
      if (!sessionId || running || !text) return;
      setRunning(true);
      setResults([]);
      setActiveResult(0);
      setObjectDef(null); // 新查询让位：定义/表详情视图隐藏
      setTableDetail(null);
      try {
        await dbQuery(
          sessionId,
          text,
        (event: DbQueryEvent) => {
          setResults((current) => {
            const next = [...current];
            const findIdx = (i: number) => next.findIndex((r) => r.index === i);
            switch (event.type) {
              case 'statementStart': {
                next.push({
                  index: event.index,
                  sql: event.sql,
                  columns: [],
                  rows: [],
                  affected: null,
                  truncated: false,
                  elapsedMs: 0,
                  error: null,
                });
                break;
              }
              case 'columns': {
                const i = findIdx(event.index);
                if (i >= 0) next[i] = { ...next[i], columns: event.columns };
                break;
              }
              case 'rows': {
                const i = findIdx(event.index);
                if (i >= 0) {
                  next[i] = { ...next[i], rows: [...next[i].rows, ...event.rows] };
                }
                break;
              }
              case 'statementDone': {
                const i = findIdx(event.index);
                if (i >= 0) {
                  next[i] = {
                    ...next[i],
                    affected: event.affected,
                    truncated: event.truncated,
                    elapsedMs: event.elapsedMs,
                  };
                }
                break;
              }
              case 'statementError': {
                const i = findIdx(event.index);
                if (i >= 0) {
                  next[i] = { ...next[i], error: event.message };
                } else {
                  next.push({
                    index: event.index,
                    sql: '',
                    columns: [],
                    rows: [],
                    affected: null,
                    truncated: false,
                    elapsedMs: 0,
                    error: event.message,
                  });
                }
                break;
              }
              default:
                break;
            }
            return next;
          });
          if (event.type === 'statementDone' || event.type === 'statementError') {
            setActiveResult((cur) => cur); // 保持当前
          }
        },
        { maxRows: rowCap, timeoutMs: DEFAULT_TIMEOUT_MS },
      );
    } catch (e) {
      setResults([
        {
          index: 0,
          sql: text,
          columns: [],
          rows: [],
          affected: null,
          truncated: false,
          elapsedMs: 0,
          error: String(e),
        },
      ]);
    } finally {
      setRunning(false);
    }
  }, [running, rowCap, sql]);

  // 切换当前数据库：MySQL=USE；PG=重连（后端换引擎）；切换后刷新对象树。
  // SQLite 无连接级切库（后端为空操作），UI 侧下拉已禁用，此函数不会被触发
  const handleUseDatabase = useCallback(
    async (database: string) => {
      const sessionId = sessionIdRef.current;
      if (!sessionId || switchingDb || !database.trim()) return;
      setSwitchingDb(true);
      try {
        const nextMeta = await dbUseDatabase(sessionId, database);
        setMeta(nextMeta);
        if (!databases.includes(database)) {
          setDatabases((current) => [...current, database].sort());
        }
        await catalogLoad(sessionId, database);
      } catch (e) {
        setConnectError(String(e));
      } finally {
        setSwitchingDb(false);
      }
    },
    [catalogLoad, databases, switchingDb],
  );

  // ── 树拖宽 ──
  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      if (!resizeRef.current) return;
      const delta = e.clientX - resizeRef.current.startX;
      setTreeWidth(
        Math.max(TREE_MIN_WIDTH, Math.min(TREE_MAX_WIDTH, resizeRef.current.startWidth + delta)),
      );
    };
    const onUp = () => {
      resizeRef.current = null;
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    return () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
  }, []);

  const treeFilter = treeSearch.trim().toLowerCase();

  // 清空搜索后复位手动收起集
  useEffect(() => {
    if (!treeSearch.trim()) setSearchCollapsed(new Set());
  }, [treeSearch]);

  /** 搜索匹配：对象类节点按名称命中；分组/库节点看其已加载子树内是否有命中（未加载子层不参与） */
  const subtreeMatches = (branch: TreeBranch, q: string): boolean => {
    if (OBJECT_KINDS.has(branch.node.kind) && branch.node.name.toLowerCase().includes(q)) {
      return true;
    }
    return (branch.children ?? []).some((c) => {
      const child = branches.get(c.key);
      return child
        ? subtreeMatches(child, q)
        : OBJECT_KINDS.has(c.kind) && c.name.toLowerCase().includes(q);
    });
  };

  const renderTree = (keys: string[], depth: number) => {
    const out: React.ReactNode[] = [];
    for (const key of keys) {
      const branch = branches.get(key);
      if (!branch) continue;
      // 搜索即剪枝：命中的对象及其祖先路径显示，其余整枝隐藏
      if (treeFilter && !subtreeMatches(branch, treeFilter)) continue;
      const hasChildren = EXPANDABLE_KINDS.has(branch.node.kind);
      // 搜索时命中路径默认展开、点击可收起（仅展示层，不改 expanded 状态，清空搜索即恢复原状）
      const effectiveExpanded = treeFilter ? !searchCollapsed.has(key) : branch.expanded;
      // 树根层的「当前库」高亮：各引擎根层形态不同——MySQL（含 OB MySQL 模式）
      // 根层是 database 节点，PG / Oracle（含 OB Oracle 模式）/ 达梦 / SQLite
      // 根层是 schema 节点，所以按名字匹配而非限定 kind。大小写不敏感：达梦
      // Oracle 兼容模式与 Oracle 的标识符默认折叠大写，而连接记录里用户可能
      // 填小写。
      const currentDbName = meta?.database ?? '';
      const isCurrentDb =
        depth === 0 &&
        currentDbName.length > 0 &&
        branch.node.name.toLowerCase() === currentDbName.toLowerCase();
      // 分组名后端给标准英文大写（TABLE/VIEW/...），前端 i18n 映射显示
      const displayLabel =
        branch.node.kind === 'group'
          ? t(`db.group_${branch.node.name.toLowerCase()}`, branch.node.name)
          : branch.node.name;
      out.push(
        <button
          key={key}
          type="button"
          onClick={() => {
            if (treeFilter) {
              // 搜索态：折叠/展开只作用于展示层，不动真实展开态（列层不参与匹配，无需懒加载）
              setSearchCollapsed((cur) => {
                const next = new Set(cur);
                if (next.has(key)) next.delete(key);
                else next.add(key);
                return next;
              });
            } else {
              void toggleBranch(key);
            }
          }}
          onDoubleClick={() => handleNodeClick(branch.node)}
          onContextMenu={(e) => {
            // 表/视图/物化视图/函数/过程/序列/触发器提供右键菜单；其余用系统默认
            if (
              [
                'table',
                'view',
                'mview',
                'ftable',
                'extension',
                'dblink',
                'recycle',
                'datatype',
                'tablespace',
                'user',
                'role',
                'function',
                'procedure',
                'sequence',
                'trigger',
                'synonym',
                'package',
                'event',
                'rule',
              ].includes(branch.node.kind)
            ) {
              e.preventDefault();
              setContextMenu({ x: e.clientX, y: e.clientY, node: branch.node });
            }
          }}
          style={{ paddingLeft: 8 + depth * 14 }}
          className={cn(
            'flex w-full items-center gap-1.5 rounded-md py-1 pr-2 text-left text-ui-xs text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground',
            isCurrentDb && 'bg-selected/70 font-semibold text-foreground',
          )}
          title={
            branch.node.detail ? `${displayLabel} · ${branch.node.detail}` : displayLabel
          }
        >
          {hasChildren ? (
            <ChevronRight
              className={cn(
                'size-3 shrink-0 text-foreground-subtlest transition-transform duration-150',
                effectiveExpanded && 'rotate-90',
              )}
            />
          ) : (
            <span className="w-3 shrink-0" />
          )}
          {nodeIcon(branch.node.kind)}
          <span className="min-w-0 flex-1 truncate">{displayLabel}</span>
          {isCurrentDb ? (
            <span className="shrink-0 rounded bg-primary/15 px-1 py-px text-[9px] font-semibold text-primary">
              {t('db.currentBadge')}
            </span>
          ) : branch.node.detail ? (
            <span className="shrink-0 font-mono text-[9px] text-foreground-subtlest">
              {branch.node.detail}
            </span>
          ) : null}
        </button>,
      );
      if (effectiveExpanded && branch.children) {
        out.push(
          renderTree(
            branch.children.map((c) => c.key),
            depth + 1,
          ),
        );
      }
    }
    return out;
  };

  if (connecting) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 select-none">
        <Loader2 className="size-5 animate-spin text-foreground-subtle" />
        <span className="text-ui-sm text-foreground-subtle">{t('db.connecting')}</span>
      </div>
    );
  }

  if (!meta) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 select-none px-6">
        <Unplug className="size-5 text-foreground-subtlest" />
        <span className="text-ui-sm text-foreground-subtle">{t('db.notConnected')}</span>
        {connectError ? (
          <p className="max-w-md break-all text-center font-mono text-ui-xs text-destructive">
            {connectError}
          </p>
        ) : null}
      </div>
    );
  }

  const currentResult = results[activeResult] ?? results[results.length - 1] ?? null;
  // Redis Cluster：仅 db0，切库下拉禁用（SELECT 在集群协议下不可用）
  const redisCluster = meta.engine === 'redis' && meta.mode === 'cluster';

  // Redis SCAN 翻页：后端保证 SCAN 结果 headers 为 [cursor, key] 且游标只在
  // 首行首列；'0' 表示迭代完毕。翻页沿用原命令的 COUNT（缺省 100）
  const scanPaging = (() => {
    if (meta.engine !== 'redis' || !currentResult) return null;
    const text = currentResult.sql.trim();
    if (!/^scan\b/i.test(text) || currentResult.columns[0] !== 'cursor') return null;
    const cursor = currentResult.rows[0]?.[0] ?? '0';
    if (!cursor || cursor === '0') return null;
    const countMatch = text.match(/COUNT\s+(\d+)/i);
    const count = countMatch ? Number(countMatch[1]) : 100;
    return { cursor, command: `SCAN ${cursor} COUNT ${count}` };
  })();

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* 连接工具栏 */}
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-border/50 px-3 select-none">
        <Database className="size-3.5 shrink-0 text-terminal-cyan" />
        <span className="max-w-44 truncate text-ui-sm font-medium text-foreground">
          {meta.name}
        </span>
        {/* OceanBase：引擎 + 租户模式标识（meta.mode 驱动）；server_version 原样直显 */}
        {meta.engine === 'oceanbase' ? (
          <span className="hidden shrink-0 rounded bg-surface px-1.5 py-px font-mono text-[9px] text-foreground-subtle lg:inline">
            {t('dbHosts.engineOceanbase')} ·{' '}
            {t(meta.mode === 'oracle' ? 'dbHosts.obModeOracle' : 'dbHosts.obModeMysql')}
          </span>
        ) : null}
        <span className="hidden max-w-64 truncate font-mono text-ui-xs text-foreground-subtle lg:block">
          {meta.serverVersion}
        </span>
        {meta.readonly ? (
          <span className="rounded bg-surface px-1.5 py-px font-mono text-[9px] text-foreground-subtle uppercase">
            {t('dbHosts.readonlyBadge')}
          </span>
        ) : (
          <span className="rounded bg-amber-500/15 px-1.5 py-px font-mono text-[9px] text-amber-500 uppercase">
            {t('db.writable')}
          </span>
        )}
        {/* 当前数据库切换：MySQL=USE / PG=重连（自绘下拉，原生 select 弹层由系统绘制观感不可控）；
            SQLite 无连接级库概念（固定 main，ATTACH 库由对象树呈现），切换无意义故禁用但保留展示 */}
        <div className="flex items-center gap-1">
          <span className="hidden text-ui-xs text-foreground-subtlest select-none sm:inline">
            {t('db.currentDatabase')}
          </span>
          {switchingDb ? (
            <Loader2 className="mx-1 size-3 animate-spin text-foreground-subtle" />
          ) : (
            <span
              title={redisCluster ? t('db.redisClusterDbLocked') : undefined}
              className={redisCluster ? 'cursor-not-allowed opacity-60' : undefined}
            >
              <SelectMenu
                value={meta.database ?? ''}
                options={(
                  meta.database && !databases.includes(meta.database)
                    ? [meta.database, ...databases]
                    : databases
                ).map((db) => ({ value: db, label: db }))}
                onChange={(db) => void handleUseDatabase(db)}
                disabled={
                  databases.length === 0 ||
                  meta.engine === 'sqlite' ||
                  (meta.engine === 'redis' && meta.mode === 'cluster')
                }
                placeholder="—"
                aria-label={t('db.currentDatabase')}
                className="max-w-40"
                triggerClassName="h-6 rounded border-border/60 bg-background px-1.5 font-mono text-ui-xs hover:border-border"
              />
            </span>
          )}
        </div>
        <div className="flex-1" />
        <ControlHintTooltip title={t('db.refreshTree')} side="bottom">
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-7 rounded-lg text-foreground-subtle hover:text-foreground"
            onClick={() => {
              const sid = sessionIdRef.current;
              if (sid) void catalogLoad(sid, meta.database);
            }}
          >
            <RefreshCw className="size-3.5" />
          </Button>
        </ControlHintTooltip>
        <ControlHintTooltip title={t('db.disconnect')} side="bottom">
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-7 rounded-lg text-foreground-subtle hover:text-destructive"
            onClick={() => {
              const sid = sessionIdRef.current;
              if (sid) {
                void dbDisconnect(sid).catch(() => undefined);
                sessionIdRef.current = null;
              }
              setMeta(null);
              onBaseTitle(null);
            }}
          >
            <Unplug className="size-3.5" />
          </Button>
        </ControlHintTooltip>
      </div>

      {connectError ? (
        <div className="flex shrink-0 items-start gap-1.5 border-b border-destructive/20 bg-destructive/5 px-3 py-1 text-ui-xs text-destructive">
          <AlertCircle className="mt-0.5 size-3 shrink-0" />
          {/* Oracle 错误是三行结构化文案（代码解释 / 原因 / 处理建议），
              用 pre-wrap 保留换行；其余引擎的单行错误也自然兼容 */}
          <span className="min-w-0 flex-1 whitespace-pre-wrap break-all">{connectError}</span>
          <button
            type="button"
            onClick={() => setConnectError(null)}
            className="shrink-0 opacity-70 hover:opacity-100"
          >
            ✕
          </button>
        </div>
      ) : null}

      {/* 树节点右键菜单（fixed 定位 + portal，坐标取 contextmenu 事件） */}
      {contextMenu
        ? createPortal(
            <div
              ref={contextMenuRef}
              role="menu"
              style={{ position: 'fixed', top: contextMenu.y, left: contextMenu.x }}
              className="z-[120] min-w-44 overflow-hidden rounded-lg border border-border bg-menu p-1 shadow-lg select-none"
            >
              {(() => {
                // Redis 键：table 节点即 key（detail=值类型）——「表详情」语义即
                // 键详情，另附编辑/TTL/删除三个写入口（只读会话隐藏）；创建索引
                // 对 Redis 无意义，只对 SQL 表保留
                const node = contextMenu.node;
                const isRedisKey = meta.engine === 'redis' && node.kind === 'table';
                const items: { key: string; label: string; action: () => void }[] = [];
                if (node.kind === 'table' || node.kind === 'view' || node.kind === 'mview') {
                  items.push(
                    {
                      key: 'viewData',
                      label: t('db.viewData'),
                      action: () => handleNodeClick(node),
                    },
                    {
                      key: 'detail',
                      label: isRedisKey ? t('db.objectDefinition') : t('db.tableDetail'),
                      action: () => openTableDetail(node),
                    },
                  );
                } else {
                  items.push({
                    key: 'definition',
                    label: t('db.objectDefinition'),
                    action: () => handleNodeClick(node),
                  });
                }
                if (isRedisKey && !meta.readonly) {
                  const keyDb = node.key.split('.')[0] ?? '';
                  const keyType = node.detail ?? 'string';
                  items.push(
                    {
                      key: 'editKey',
                      label: t('db.redisEditKey'),
                      action: () =>
                        setRedisDialog({ kind: 'editor', keyName: node.name, keyDb, keyType }),
                    },
                    {
                      key: 'ttl',
                      label: t('db.redisTtlTitle'),
                      action: () =>
                        setRedisDialog({ kind: 'ttl', keyName: node.name, keyDb, keyType }),
                    },
                    {
                      key: 'deleteKey',
                      label: t('db.redisDeleteKeyTitle'),
                      action: () =>
                        setRedisDialog({ kind: 'delete', keyName: node.name, keyDb, keyType }),
                    },
                  );
                }
                if (node.kind === 'table' && meta.engine !== 'redis') {
                  items.push({
                    key: 'createIndex',
                    label: t('db.createIndex'),
                    action: () => insertCreateIndexTemplate(node),
                  });
                }
                items.push({
                  key: 'copyName',
                  label: t('db.copyQualifiedName'),
                  action: () => {
                    void navigator.clipboard.writeText(node.name).catch(() => undefined);
                  },
                });
                return items.map((item) => (
                  <button
                    key={item.key}
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      setContextMenu(null);
                      item.action();
                    }}
                    className="flex w-full items-center rounded-md px-2.5 py-1.5 text-left text-ui-sm text-foreground transition-colors hover:bg-menu-hover"
                  >
                    {item.label}
                  </button>
                ));
              })()}
            </div>,
            document.body,
          )
        : null}

      {/* Redis 键管理/诊断对话框（键操作完成即刷新对象树；保存后自动回看该键） */}
      {meta && redisDialog?.kind === 'editor' ? (
        <RedisKeyEditorModal
          sessionId={meta.sessionId}
          keyName={redisDialog.keyName}
          keyDb={redisDialog.keyDb || undefined}
          onClose={() => setRedisDialog(null)}
          onSaved={() => {
            void catalogLoad(meta.sessionId, meta.database);
            const cmd = redisViewCommand(redisDialog.keyName, redisDialog.keyType);
            setSql(cmd);
            void runQuery(cmd);
          }}
        />
      ) : null}
      {meta && redisDialog?.kind === 'ttl' ? (
        <RedisTtlModal
          sessionId={meta.sessionId}
          keyName={redisDialog.keyName}
          keyDb={redisDialog.keyDb || undefined}
          currentDb={meta.database}
          onClose={() => setRedisDialog(null)}
          onChanged={() => void catalogLoad(meta.sessionId, meta.database)}
        />
      ) : null}
      {meta && redisDialog?.kind === 'delete' ? (
        <RedisKeyDeleteModal
          sessionId={meta.sessionId}
          keyName={redisDialog.keyName}
          keyDb={redisDialog.keyDb || undefined}
          currentDb={meta.database}
          onClose={() => setRedisDialog(null)}
          onDeleted={() => void catalogLoad(meta.sessionId, meta.database)}
        />
      ) : null}
      {meta && redisDialog?.kind === 'slowlog' ? (
        <RedisSlowlogModal sessionId={meta.sessionId} onClose={() => setRedisDialog(null)} />
      ) : null}
      {meta && redisDialog?.kind === 'bigkeys' ? (
        <RedisBigkeysModal sessionId={meta.sessionId} onClose={() => setRedisDialog(null)} />
      ) : null}

      <div className="flex min-h-0 flex-1">
        {/* 对象树 */}
        <aside
          style={{ width: `${treeWidth}px` }}
          className="flex shrink-0 flex-col border-r border-border/50 bg-background/40"
        >
          {/* 当前库指示条：与会话的当前数据库（下拉所选）实时联动，切库即更新 */}
          <div className="flex h-8 shrink-0 items-center gap-1.5 border-b border-border/40 px-2.5 select-none">
            <Database className="size-3 shrink-0 text-primary" />
            <span className="shrink-0 text-[10px] text-foreground-subtlest uppercase tracking-wide">
              {t('db.currentDatabase')}
            </span>
            <span className="min-w-0 flex-1 truncate text-right font-mono text-ui-xs font-semibold text-primary">
              {meta.database ?? '—'}
            </span>
          </div>
          {/* 对象树搜索：纯内存过滤，只作用于当前选中库的已加载对象（连接/切库后自动预载） */}
          <div className="flex h-8 shrink-0 items-center gap-1.5 border-b border-border/40 px-2">
            <Search className="size-3 shrink-0 text-foreground-subtlest" />
            <div className="relative min-w-0 flex-1">
              <input
                type="text"
                value={treeSearch}
                onChange={(e) => setTreeSearch(e.target.value)}
                placeholder={t('db.searchObjects')}
                className="h-6 w-full rounded-md border border-border/60 bg-background pr-5 pl-2 text-ui-xs text-foreground outline-none placeholder:text-foreground-subtlest focus:border-border"
              />
              {treeSearch ? (
                <button
                  type="button"
                  onClick={() => setTreeSearch('')}
                  className="absolute inset-y-0 right-1 flex w-4 items-center justify-center text-foreground-subtlest transition-colors hover:text-foreground"
                  aria-label={t('common.clear', '清空')}
                >
                  <X className="size-3" />
                </button>
              ) : null}
            </div>
          </div>
          <div className="scrollbar-slim min-h-0 flex-1 overflow-y-auto p-1.5 select-none">
            {renderTree(roots, 0)}
          </div>
          <div className="border-t border-border/40 px-2 py-1 font-mono text-[10px] text-foreground-subtlest select-none">
            {t('db.treeHint')}
          </div>
        </aside>

        {/* 拖宽手柄 */}
        <div
          role="separator"
          aria-orientation="vertical"
          tabIndex={0}
          onPointerDown={(e) => {
            if (e.button === 0) {
              resizeRef.current = { startX: e.clientX, startWidth: treeWidth };
              e.preventDefault();
            }
          }}
          className="group/handle relative z-10 flex h-full w-1 shrink-0 cursor-ew-resize touch-none items-center justify-center bg-transparent outline-none after:pointer-events-none after:absolute after:w-0.5 after:rounded-full after:bg-foreground-subtlest/50 after:opacity-0 after:transition-opacity after:content-[''] after:inset-y-4 hover:after:opacity-100"
        />

        {/* 编辑器 + 结果 */}
        <div className="flex min-w-0 flex-1 flex-col">
          {/* SQL 编辑器区 */}
          <div className="h-44 shrink-0 border-b border-border/50">
            <SqlEditor
              value={sql}
              onChange={setSql}
              onRun={() => void runQuery()}
              redisMode={meta.engine === 'redis'}
            />
          </div>

          {/* 执行工具栏 */}
          <div className="flex h-8 shrink-0 items-center gap-1.5 border-b border-border/50 bg-surface/40 px-2 select-none">
            <button
              type="button"
              disabled={running || !sql.trim()}
              onClick={() => void runQuery()}
              className="flex items-center gap-1 rounded-md bg-surface px-2 py-0.5 text-ui-xs font-medium text-foreground hover:bg-hover disabled:opacity-40"
            >
              <Play className="size-3 text-terminal-green fill-terminal-green" />
              {t('db.run')}
            </button>
            {running ? (
              <button
                type="button"
                onClick={() => {
                  const sid = sessionIdRef.current;
                  if (sid) void dbQueryCancel(sid).catch(() => undefined);
                }}
                className="flex items-center gap-1 rounded-md bg-surface px-2 py-0.5 text-ui-xs font-medium text-destructive hover:bg-hover"
              >
                <Square className="size-3 fill-destructive" />
                {t('db.cancel')}
              </button>
            ) : null}
            <span className="mx-1 h-4 w-px bg-border/60" />
            <span className="text-ui-xs text-foreground-subtle">{t('db.rowCap')}</span>
            <SelectMenu
              value={String(rowCap)}
              options={ROW_CAP_OPTIONS.map((n) => ({ value: String(n), label: n.toLocaleString() }))}
              onChange={(v) => setRowCap(Number(v))}
              aria-label={t('db.rowCap')}
              className="w-[4.5rem]"
              triggerClassName="h-6 rounded border-border/60 bg-background px-1.5 text-ui-xs hover:border-border"
            />
            {/* Redis 专属：慢日志 / 大 key 分析（只读会话也可用，纯读操作） */}
            {meta.engine === 'redis' ? (
              <>
                <span className="mx-1 h-4 w-px bg-border/60" />
                <button
                  type="button"
                  onClick={() => setRedisDialog({ kind: 'slowlog' })}
                  className="flex items-center gap-1 rounded-md bg-surface px-2 py-0.5 text-ui-xs font-medium text-foreground hover:bg-hover"
                >
                  {t('db.redisSlowlogTitle')}
                </button>
                <button
                  type="button"
                  onClick={() => setRedisDialog({ kind: 'bigkeys' })}
                  className="flex items-center gap-1 rounded-md bg-surface px-2 py-0.5 text-ui-xs font-medium text-foreground hover:bg-hover"
                >
                  {t('db.redisBigkeysTitle')}
                </button>
              </>
            ) : null}
            {running ? (
              <span className="ml-auto flex items-center gap-1 text-ui-xs text-foreground-subtle">
                <Loader2 className="size-3 animate-spin" />
                {t('db.executing')}
              </span>
            ) : null}
          </div>

          {/* 结果面板：表详情 > 定义视图 > 多结果集 Tab */}
          <div className="flex min-h-0 flex-1 flex-col">
            {detailLoading ? (
              <div className="flex flex-1 items-center justify-center gap-2 text-ui-sm text-foreground-subtle select-none">
                <Loader2 className="size-4 animate-spin" />
                {t('db.loadingDefinition')}
              </div>
            ) : tableDetail ? (
              <>
                <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border/50 bg-surface/30 px-2 select-none">
                  {nodeIcon(tableDetail.kind)}
                  <span className="max-w-44 truncate font-mono text-ui-xs font-semibold text-foreground">
                    {tableDetail.name}
                  </span>
                  {/* 表统计徽标：注释截断展示在 title 悬停；大小/行数即时可见 */}
                  {tableDetail.detail.comment ? (
                    <span
                      className="hidden max-w-52 truncate rounded bg-surface px-1.5 py-px text-ui-xs text-foreground-subtle lg:inline-block"
                      title={tableDetail.detail.comment}
                    >
                      {tableDetail.detail.comment}
                    </span>
                  ) : null}
                  {tableDetail.detail.size ? (
                    <span className="shrink-0 rounded bg-primary/10 px-1.5 py-px font-mono text-[9px] font-semibold text-primary">
                      {tableDetail.detail.size}
                    </span>
                  ) : null}
                  {tableDetail.detail.rowCount ? (
                    <span className="shrink-0 rounded bg-surface px-1.5 py-px font-mono text-[9px] text-foreground-subtle">
                      {tableDetail.detail.rowCount} rows
                    </span>
                  ) : null}
                  <span className="flex items-center gap-0.5">
                    {(['ddl', 'columns', 'indexes', 'constraints', 'references'] as const)
                      // 无数据（headers 空）不展示约束/引用 Tab
                      .filter(
                        (tab) =>
                          (tab !== 'constraints' || tableDetail.detail.constraintHeaders.length > 0) &&
                          (tab !== 'references' || tableDetail.detail.referenceHeaders.length > 0),
                      )
                      .map((tab) => (
                      <button
                        key={tab}
                        type="button"
                        onClick={() => setTableDetail((cur) => (cur ? { ...cur, tab } : cur))}
                        className={cn(
                          'rounded-md border px-2 py-0.5 text-ui-xs transition-colors',
                          tableDetail.tab === tab
                            ? 'border-border bg-surface font-semibold text-foreground'
                            : 'border-transparent text-foreground-subtle hover:bg-hover hover:text-foreground',
                        )}
                      >
                        {t(`db.tab_${tab}`)}
                      </button>
                    ))}
                  </span>
                  <div className="flex-1" />
                  {tableDetail.tab === 'ddl' ? (
                    <button
                      type="button"
                      onClick={() => {
                        void navigator.clipboard
                          .writeText(tableDetail.detail.ddl)
                          .then(() => {
                            setDdlCopied(true);
                            setTimeout(() => setDdlCopied(false), 2000);
                          })
                          .catch(() => undefined);
                      }}
                      className="flex items-center gap-1 rounded-md px-2 py-0.5 text-ui-xs text-foreground-subtle hover:bg-hover hover:text-foreground"
                    >
                      {ddlCopied ? (
                        <span className="text-terminal-green">{t('files.copied', '已复制')}</span>
                      ) : (
                        <>
                          <Copy className="size-3" />
                          <span>{t('db.copyDefinition')}</span>
                        </>
                      )}
                    </button>
                  ) : null}
                  <button
                    type="button"
                    onClick={() => setTableDetail(null)}
                    className="rounded p-1 text-foreground-subtlest hover:text-foreground"
                    aria-label={t('common.close', '关闭')}
                  >
                    ✕
                  </button>
                </div>
                {tableDetail.tab === 'ddl' ? (
                  <div className="scrollbar-slim min-h-0 flex-1 overflow-auto p-3">
                    <pre className="whitespace-pre-wrap break-all font-mono text-ui-xs leading-relaxed text-foreground select-text">
                      {tableDetail.detail.ddl}
                    </pre>
                  </div>
                ) : tableDetail.tab === 'columns' ? (
                  <DataTable
                    className="flex-1"
                    columns={tableDetail.detail.columnHeaders}
                    rows={tableDetail.detail.columns}
                  />
                ) : tableDetail.tab === 'indexes' ? (
                  <DataTable
                    className="flex-1"
                    columns={tableDetail.detail.indexHeaders}
                    rows={tableDetail.detail.indexes}
                    emptyLabel={t('db.noIndexes')}
                  />
                ) : tableDetail.tab === 'references' ? (
                  <DataTable
                    className="flex-1"
                    columns={tableDetail.detail.referenceHeaders}
                    rows={tableDetail.detail.references}
                    emptyLabel={t('db.noReferences')}
                  />
                ) : (
                  <DataTable
                    className="flex-1"
                    columns={tableDetail.detail.constraintHeaders}
                    rows={tableDetail.detail.constraints}
                    emptyLabel={t('db.noConstraints')}
                  />
                )}
              </>
            ) : defLoading ? (
              <div className="flex flex-1 items-center justify-center gap-2 text-ui-sm text-foreground-subtle select-none">
                <Loader2 className="size-4 animate-spin" />
                {t('db.loadingDefinition')}
              </div>
            ) : objectDef ? (
              <>
                <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border/50 bg-surface/30 px-2 select-none">
                  {objectDef.kind === 'function' ? (
                    <Sigma className="size-3.5 shrink-0 text-terminal-magenta" />
                  ) : objectDef.kind === 'trigger' ? (
                    <Zap className="size-3.5 shrink-0 text-terminal-red" />
                  ) : objectDef.kind === 'synonym' ? (
                    <Link className="size-3.5 shrink-0 text-terminal-cyan" />
                  ) : objectDef.kind === 'package' ? (
                    <Package className="size-3.5 shrink-0 text-terminal-yellow" />
                  ) : objectDef.kind === 'event' ? (
                    <Clock className="size-3.5 shrink-0 text-terminal-blue" />
                  ) : objectDef.kind === 'rule' ? (
                    <ScrollText className="size-3.5 shrink-0 text-terminal-magenta" />
                  ) : objectDef.kind === 'ftable' ? (
                    <Globe className="size-3.5 shrink-0 text-terminal-blue" />
                  ) : objectDef.kind === 'extension' ? (
                    <Puzzle className="size-3.5 shrink-0 text-terminal-cyan" />
                  ) : objectDef.kind === 'dblink' ? (
                    <Cable className="size-3.5 shrink-0 text-terminal-yellow" />
                  ) : objectDef.kind === 'recycle' ? (
                    <Trash2 className="size-3.5 shrink-0 text-foreground-subtlest" />
                  ) : objectDef.kind === 'datatype' ? (
                    <Type className="size-3.5 shrink-0 text-terminal-magenta" />
                  ) : objectDef.kind === 'tablespace' ? (
                    <HardDrive className="size-3.5 shrink-0 text-terminal-cyan" />
                  ) : objectDef.kind === 'user' ? (
                    <User className="size-3.5 shrink-0 text-terminal-green" />
                  ) : objectDef.kind === 'role' ? (
                    <ShieldCheck className="size-3.5 shrink-0 text-terminal-yellow" />
                  ) : (
                    <Braces className="size-3.5 shrink-0 text-terminal-magenta" />
                  )}
                  <span className="max-w-44 truncate font-mono text-ui-xs font-semibold text-foreground">
                    {objectDef.name}
                  </span>
                  <span className="text-ui-xs text-foreground-subtle">{t('db.objectDefinition')}</span>
                  {objectDef.def.argCount > 0 ? (
                    <span className="rounded bg-surface px-1.5 py-px font-mono text-[9px] text-foreground-subtle">
                      {t('db.argCount', { count: objectDef.def.argCount })}
                    </span>
                  ) : null}
                  <div className="flex-1" />
                  <button
                    type="button"
                    onClick={() => {
                      void navigator.clipboard
                        .writeText(objectDef.def.definition)
                        .then(() => {
                          setDefCopied(true);
                          setTimeout(() => setDefCopied(false), 2000);
                        })
                        .catch(() => undefined);
                    }}
                    className="flex items-center gap-1 rounded-md px-2 py-0.5 text-ui-xs text-foreground-subtle hover:bg-hover hover:text-foreground"
                    title={t('files.copyCode', '复制')}
                  >
                    {defCopied ? (
                      <span className="text-terminal-green">{t('files.copied', '已复制')}</span>
                    ) : (
                      <>
                        <Copy className="size-3" />
                        <span>{t('db.copyDefinition')}</span>
                      </>
                    )}
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      setSql(objectDef.def.callTemplate);
                      setDefNotice(t('db.templateInserted'));
                    }}
                    className="rounded-md px-2 py-0.5 text-ui-xs text-foreground-subtle hover:bg-hover hover:text-foreground"
                  >
                    {t('db.insertCallTemplate')}
                  </button>
                  <button
                    type="button"
                    disabled={running}
                    onClick={() => {
                      const { callTemplate, argCount } = objectDef.def;
                      setSql(callTemplate);
                      if (argCount > 0) {
                        // 有参数：先插入编辑器引导补参，不首接执行（空参会直接报错）
                        setDefNotice(t('db.needsArgs', { count: argCount }));
                        return;
                      }
                      setDefNotice(null);
                      void runQuery(callTemplate);
                    }}
                    className="flex items-center gap-1 rounded-md bg-surface px-2 py-0.5 text-ui-xs font-medium text-foreground hover:bg-hover disabled:opacity-40"
                  >
                    <Play className="size-3 text-terminal-green fill-terminal-green" />
                    {objectDef.def.argCount > 0 ? t('db.fillArgsAndRun') : t('db.runCall')}
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      setObjectDef(null);
                      setDefNotice(null);
                    }}
                    className="rounded p-1 text-foreground-subtlest hover:text-foreground"
                    aria-label={t('common.close', '关闭')}
                  >
                    ✕
                  </button>
                </div>
                {defNotice ? (
                  <div className="shrink-0 border-b border-border/40 bg-surface/20 px-3 py-1 text-ui-xs text-foreground-subtle select-none">
                    {defNotice}
                  </div>
                ) : null}
                <div className="scrollbar-slim min-h-0 flex-1 overflow-auto p-3">
                  <pre className="whitespace-pre-wrap break-all font-mono text-ui-xs leading-relaxed text-foreground select-text">
                    {objectDef.def.definition}
                  </pre>
                </div>
              </>
            ) : results.length === 0 ? (
              <div className="flex flex-1 items-center justify-center text-ui-sm text-foreground-subtlest select-none">
                {t('db.noResults')}
              </div>
            ) : (
              <>
                <div className="flex h-8 shrink-0 items-center gap-1 overflow-x-auto border-b border-border/50 bg-surface/30 px-2 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden select-none">
                  {results.map((result, i) => (
                    <button
                      key={result.index}
                      type="button"
                      onClick={() => setActiveResult(i)}
                      className={cn(
                        'flex shrink-0 items-center gap-1 rounded-md border px-2 py-0.5 font-mono text-ui-xs transition-colors',
                        i === activeResult
                          ? 'border-border bg-surface font-semibold text-foreground'
                          : 'border-transparent text-foreground-subtle hover:bg-hover hover:text-foreground',
                      )}
                    >
                      {result.error ? (
                        <AlertCircle className="size-3 text-destructive" />
                      ) : result.columns.length > 0 ? (
                        <Table2 className="size-3 text-terminal-green" />
                      ) : (
                        <Square className="size-2.5 fill-foreground-subtlest text-foreground-subtlest" />
                      )}
                      #{result.index + 1}
                    </button>
                  ))}
                </div>
                {currentResult ? (
                  currentResult.error ? (
                    <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-auto p-3 select-text">
                      <div className="flex items-center gap-1.5 text-ui-sm text-destructive">
                        <AlertCircle className="size-4 shrink-0" />
                        <span>{t('db.statementFailed')}</span>
                      </div>
                      <pre className="whitespace-pre-wrap break-all rounded-md border border-destructive/30 bg-destructive/5 p-2.5 font-mono text-ui-xs text-destructive">
                        {currentResult.error}
                      </pre>
                      {currentResult.sql ? (
                        <p className="font-mono text-ui-xs text-foreground-subtlest">
                          {currentResult.sql.slice(0, 200)}
                        </p>
                      ) : null}
                    </div>
                  ) : currentResult.columns.length > 0 ? (
                    <DataTable
                      className="flex-1"
                      columns={currentResult.columns}
                      rows={currentResult.rows}
                      statusFooter={
                        <span className="flex items-center gap-2">
                          <span>
                            {currentResult.rows.length} 行 · {currentResult.columns.length} 列 ·{' '}
                            {currentResult.elapsedMs} ms
                            {currentResult.truncated ? ` · ${t('db.truncated')}` : ''}
                          </span>
                          {scanPaging ? (
                            <button
                              type="button"
                              onClick={() => {
                                setSql(scanPaging.command);
                                void runQuery(scanPaging.command);
                              }}
                              className="rounded border border-border/60 px-1.5 py-px font-mono text-ui-xs text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
                              title={t('db.redisScanNextHint')}
                            >
                              {t('db.redisScanNext')} ▸
                            </button>
                          ) : null}
                        </span>
                      }
                    />
                  ) : (
                    <div className="flex flex-1 items-center justify-center gap-2 text-ui-sm text-foreground-subtle select-none">
                      <Square className="size-3 fill-current opacity-40" />
                      {currentResult.affected != null
                        ? t('db.affectedRows', { count: currentResult.affected })
                        : t('db.executed')}
                      <span className="font-mono text-ui-xs text-foreground-subtlest">
                        {currentResult.elapsedMs} ms
                      </span>
                    </div>
                  )
                ) : null}
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
