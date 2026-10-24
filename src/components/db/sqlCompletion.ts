/**
 * 数据库工作台 SQL 编辑器的 schema 感知补全（表 / 列 / 别名 t.xx）。
 *
 * 纯逻辑模块：不 import monaco 也不 import 服务层——补全数据源以
 * SqlSchemaCompleter 接口注入（DbSessionView 用 dbCatalog 目录接口实现，
 * 带缓存），建议产物是中性的 SuggestionDraft，由 SqlEditor 映射成
 * monaco CompletionItem。纯函数形态可用 `node --experimental-strip-types`
 * 一次性冒烟（命令记于文件尾注释）。
 *
 * 解析策略（行级 token 扫描，无 AST——补全只求「够准不求全」）：
 * - 语句边界：光标前最后一个 `;` 之后视为当前语句；
 * - 表引用：FROM / JOIN / UPDATE / INTO 后的标识符（支持 schema.table
 *   两段限定），其后一个非保留字 token 视为别名（`AS x` 显式形态亦认）；
 * - 光标上下文三种：
 *   ① 限定符形态 `t.xx|`（含刚敲完点 `t.|`）——按 别名 → 裸表名 → schema
 *     的顺序解析限定符，给出对应列（或该 schema 的表）；
 *   ② 表名位（FROM/JOIN/INTO/UPDATE 后正在敲标识符）——给当前库的表/视图
 *     + 顶层 schema 清单（insertText 带 `.` 引导二段补全）；
 *   ③ 列名位（SELECT 列表 / WHERE / ON / GROUP BY 等）——给当前语句引用的
 *     全部表的列并集（去重），无引用时退化为表清单。
 *
 * 已知边界（有意不做）：引号内局部输入（`"US|`）不剥离引号补全；子查询
 * 派生表别名（`FROM (SELECT..) x`）的列不可知，不给建议；字符串字面量与
 * 注释中的关键字已被 tokenizer 跳过不会误触发。
 */

/** 补全数据源的一个表/视图条目 */
export interface SchemaTable {
  /** 所属 schema（MySQL 为 database 名） */
  schema: string;
  /** 裸名（不带引号） */
  name: string;
  /** table / view / mview / synonym 等（目录对象的 kind 原样） */
  kind: string;
}

/** 补全数据源的一列 */
export interface SchemaColumn {
  name: string;
  /** 裸类型名（VARCHAR / NUMBER …，做 detail 提示） */
  dataType: string;
}

/** 补全数据源（DbSessionView 侧用 dbCatalog + 缓存实现） */
export interface SqlSchemaCompleter {
  /** 当前库名（切库后返回新值；裸表名的 schema 兑底用它） */
  currentSchema(): string;
  /** 顶层 schema / database 名单（供 FROM schema. 二段补全） */
  getSchemas(): Promise<string[]>;
  /** 某库的表类对象（table/view/mview/synonym）；缺省 = 当前库 */
  getTables(schema?: string): Promise<SchemaTable[]>;
  /** 某表的列（缓存由实现方负责） */
  getColumns(schema: string, table: string): Promise<SchemaColumn[]>;
}

/** 建议草稿（中性结构，SqlEditor 映射为 monaco CompletionItem） */
export interface SuggestionDraft {
  label: string;
  kind: 'column' | 'table' | 'schema';
  detail: string;
  insertText: string;
  /** 稳定排序前缀。静态关键字 provider（sqlLanguage.ts）用 `0`(snippet)/`1`(关键字)
   * /`2`(函数)，schema 建议必须永远排在它们前：取 `!`（ASCII 0x21 < '0'），
   * 段内再分 列 !1 / 表 !2 / schema !3 */
  sort: string;
}

/** 语句内一张被引用的表（含别名指向的表） */
interface RefTable {
  schema: string;
  table: string;
}

/** 光标处上下文（parseSqlCompletionContext 的产物） */
export interface SqlCompletionContext {
  /** `t.` 形态的限定符（已剥引号、原样大小写）；null = 非限定形态 */
  qualifier: string | null;
  /** 正在输入的词（限定形态=点后部分），可为空串（刚敲完触发字符） */
  word: string;
  /** word 在全文（=光标前文本）中的起始偏移（换算 range 用） */
  wordStart: number;
  /** 光标是否在表名位（FROM/JOIN/INTO/UPDATE 后敲标识符） */
  tablePosition: boolean;
  /** 别名表：lower(alias) → 表（schema + 裸表名） */
  aliases: Map<string, RefTable>;
  /** 当前语句引用的全部表（FROM/JOIN/UPDATE/INTO，含别名表，去重） */
  referenced: RefTable[];
}

const TABLE_KEYWORDS = new Set(['from', 'join', 'update', 'into']);

/** 别名位的停用词：表名后紧跟这些词时不是别名（SQL 常见子句/联结词） */
const ALIAS_STOPWORDS = new Set([
  'on', 'using', 'where', 'set', 'values', 'group', 'order', 'by', 'limit',
  'offset', 'having', 'left', 'right', 'inner', 'outer', 'cross', 'full',
  'natural', 'join', 'union', 'minus', 'intersect', 'select', 'from', 'for',
  'with', 'when', 'then', 'else', 'end', 'and', 'or', 'not', 'as', 'lateral',
  'partition', 'duplicate', 'returning', 'wait', 'nowait',
]);

interface Token {
  text: string;
  start: number;
  /** 标识符形态（可作表名/别名/限定符参与解析）；标点与字面量为 false */
  ident: boolean;
}

/** 语句粗分词：标识符 / 引号标识符 / 字符串 / 注释 / 单字符其它。
 * 字符串字面量与注释整段吞掉（其内的 from 等不触发表扫描）。 */
function tokenize(text: string): Token[] {
  const tokens: Token[] = [];
  const re =
    /'[^']*'|"(?:[^"]|"")*"|`[^`]*`|\[[^\]]*\]|--[^\n]*|\/\*[\s\S]*?\*\/|[A-Za-z_][\w$]*|\$\d+|./g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    const t = m[0];
    const isIdent =
      /^[A-Za-z_$]/.test(t) || t.startsWith('"') || t.startsWith('`') || t.startsWith('[');
    if (isIdent) tokens.push({ text: t, start: m.index, ident: true });
    else if (!/^\s$/.test(t)) tokens.push({ text: t, start: m.index, ident: false });
  }
  return tokens;
}

function stripIdentQuotes(s: string): string {
  if (
    s.length >= 2 &&
    ((s.startsWith('"') && s.endsWith('"')) ||
      (s.startsWith('`') && s.endsWith('`')) ||
      (s.startsWith('[') && s.endsWith(']')))
  ) {
    return s.slice(1, -1);
  }
  return s;
}

/** 解析光标前文本，产出补全上下文（详见接口注释） */
export function parseSqlCompletionContext(textBeforeCursor: string): SqlCompletionContext {
  // 当前语句 = 最后一个分号之后（分号不会出现在字符串里被误切——字符串
  // 里有没有分号对「取更靠后的一段」无碍：最坏把字面量前的语句并入）
  const semi = textBeforeCursor.lastIndexOf(';');
  const stmt = semi >= 0 ? textBeforeCursor.slice(semi + 1) : textBeforeCursor;

  const tokens = tokenize(stmt);
  const aliases = new Map<string, RefTable>();
  const referenced: RefTable[] = [];
  let depth = 0;
  const pushRef = (r: RefTable) => {
    if (
      !referenced.some(
        (x) =>
          x.table.toLowerCase() === r.table.toLowerCase() &&
          x.schema.toLowerCase() === r.schema.toLowerCase(),
      )
    ) {
      referenced.push(r);
    }
  };

  // 消费一个表引用：ident [. ident] [AS] [alias]，返回扫描推进位置
  const consumeRef = (startIdx: number): number => {
    let schema = '';
    let table = stripIdentQuotes(tokens[startIdx].text);
    let end = startIdx + 1;
    if (tokens[end] && tokens[end].text === '.' && tokens[end + 1] && tokens[end + 1].ident) {
      schema = table;
      table = stripIdentQuotes(tokens[end + 1].text);
      end += 2;
    }
    // 裸表名缺 schema 记空串，由建议端用当前库兜底匹配
    pushRef({ schema, table });
    if (tokens[end] && tokens[end].ident) {
      const w = tokens[end].text.toLowerCase();
      if (w === 'as' && tokens[end + 1] && tokens[end + 1].ident) {
        aliases.set(stripIdentQuotes(tokens[end + 1].text).toLowerCase(), { schema, table });
        end += 2;
      } else if (!ALIAS_STOPWORDS.has(w)) {
        aliases.set(stripIdentQuotes(tokens[end].text).toLowerCase(), { schema, table });
        end += 1;
      }
    }
    return end;
  };

  for (let i = 0; i < tokens.length; i++) {
    if (tokens[i].text === '(') depth++;
    else if (tokens[i].text === ')') depth = Math.max(0, depth - 1);
    // 括号内的表（子查询）不进外层作用域：其列在外层不可见，
    // 泄入会在外层 WHERE 处给出错误列建议
    if (depth > 0) continue;
    const kw = tokens[i].ident ? tokens[i].text.toLowerCase() : '';
    if (!TABLE_KEYWORDS.has(kw)) continue;
    // FROM ( 子查询：跳过（派生表列不可知）
    if (tokens[i + 1] && tokens[i + 1].text === '(') continue;
    if (!tokens[i + 1] || !tokens[i + 1].ident) continue;
    // FROM a, b 旧式逗号联表：每段都是表引用（各可带别名）
    let end = consumeRef(i + 1);
    while (tokens[end] && tokens[end].text === ',' && tokens[end + 1] && tokens[end + 1].ident) {
      end = consumeRef(end + 1);
    }
    i = end - 1;
  }

  // 光标词与限定符：tail 末尾匹配 `qualifier . [word]`
  const qualifierMatch = stmt.match(
    /(?:[A-Za-z_$][\w$]*|"[^"]*"|`[^`]*`|\[[^\]]*\])\s*\.\s*([\w$]*)$/,
  );
  let qualifier: string | null = null;
  let word = '';
  let wordStart = textBeforeCursor.length;
  if (qualifierMatch) {
    qualifier = stripIdentQuotes(qualifierMatch[0].replace(/\s*\.\s*[\w$]*$/, ''));
    word = qualifierMatch[1] ?? '';
    wordStart = textBeforeCursor.length - word.length;
  } else {
    const wordMatch = stmt.match(/[\w$]*$/);
    if (wordMatch) {
      word = wordMatch[0];
      wordStart = textBeforeCursor.length - word.length;
    }
  }

  // 表名位：最后一个表关键字后只有标识符/限定符字符（表名可能敲到一半）。
  // 限定形态（t. / sc.）不走此标志——resolveSchemaSuggestions 的限定分支
  // 先行返回；含空白后的位置（FROM t |）视为列名位（正在敲别名/列）
  const tablePosition = /\b(?:from|join|into|update)\s+[\w$."`\[\]]*$/i.test(stmt);

  return { qualifier, word, wordStart, tablePosition, aliases, referenced };
}

const KIND_LABEL: Record<string, string> = {
  table: 'TABLE',
  view: 'VIEW',
  mview: 'MATERIALIZED VIEW',
  ftable: 'FOREIGN TABLE',
  synonym: 'SYNONYM',
};

/** 按上下文解析建议（async：首次触达某表列时会向数据源取一趟） */
export async function resolveSchemaSuggestions(
  ctx: SqlCompletionContext,
  completer: SqlSchemaCompleter,
): Promise<SuggestionDraft[]> {
  const currentSchema = completer.currentSchema();
  // —— 限定形态 t.｜table.｜schema. ——
  if (ctx.qualifier !== null) {
    const q = ctx.qualifier.toLowerCase();
    // ① 别名
    const aliased = ctx.aliases.get(q);
    if (aliased) {
      return columnDrafts(
        await completer.getColumns(aliased.schema || currentSchema, aliased.table),
        aliased.table,
      );
    }
    // ② 裸表名（当前库）
    const tables = await completer.getTables();
    const bare = tables.find((t) => t.name.toLowerCase() === q);
    if (bare) {
      return columnDrafts(await completer.getColumns(bare.schema, bare.name), bare.name);
    }
    // ③ schema 名 → 该库的表
    const schemas = await completer.getSchemas();
    const schemaHit = schemas.find((s) => s.toLowerCase() === q);
    if (schemaHit) {
      const schemaTables = await completer.getTables(schemaHit);
      return tableDrafts(schemaTables);
    }
    return [];
  }

  // —— 表名位：当前库的表 + 顶层 schema（带 `.` 引导二段）——
  if (ctx.tablePosition) {
    const tables = await completer.getTables();
    const schemas = await completer.getSchemas();
    const lowerCurrent = currentSchema.toLowerCase();
    return [
      ...tableDrafts(tables),
        ...schemas
        .filter((s) => s.toLowerCase() !== lowerCurrent)
        .map((s) => ({
          label: s,
          kind: 'schema' as const,
          detail: 'SCHEMA',
          insertText: `${s}.`,
          sort: `!3${s.toLowerCase()}`,
        })),
    ];
  }

  // —— 列名位：当前语句引用表的列并集；无引用退化为表清单 ——
  if (ctx.referenced.length === 0) {
    return tableDrafts(await completer.getTables());
  }
  const merged = new Map<string, SuggestionDraft>();
  for (const ref of ctx.referenced) {
    const cols = await completer.getColumns(ref.schema || currentSchema, ref.table);
    for (const c of cols) {
      const key = c.name.toLowerCase();
      const draft: SuggestionDraft = {
        label: c.name,
        kind: 'column',
        detail: merged.has(key)
          ? `${merged.get(key)!.detail} · ${ref.table}.${c.name}`
          : `${ref.table}.${c.name} · ${c.dataType}`,
        insertText: c.name,
        sort: `!1${key}`,
      };
      merged.set(key, draft);
    }
  }
  return [...merged.values()];
}

function columnDrafts(cols: SchemaColumn[], table: string): SuggestionDraft[] {
  return cols.map((c) => ({
    label: c.name,
    kind: 'column' as const,
    detail: `${table}.${c.name} · ${c.dataType}`,
    insertText: c.name,
    sort: `!1${c.name.toLowerCase()}`,
  }));
}

function tableDrafts(tables: SchemaTable[]): SuggestionDraft[] {
  return tables.map((t) => ({
    label: t.name,
    kind: 'table' as const,
    detail: `${t.schema} · ${KIND_LABEL[t.kind] ?? t.kind.toUpperCase()}`,
    insertText: t.name,
    sort: `!2${t.name.toLowerCase()}`,
  }));
}

/*
 * 冒烟驱动（纯函数，无 DOM/monaco 依赖，inline 形态同 crontabLanguage 惯例）：
 *   node --experimental-strip-types --input-type=module -e '...'
 * （覆盖：表/别名/限定符/表名位/字符串字面量免疫/分号语句边界，断言清单
 * 随命令一并留档于提交记录）
 */
