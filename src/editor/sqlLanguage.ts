/**
 * SQL 语言支持：关键字 / 方言关键字 / 函数补全 + 骨架 snippet。
 *
 * 数据静态提取自 sqls-server/sqls（MIT，commit 920fde7，见
 * ./sqlData/README.md）——只取其静态层（基础 ANSI 关键字带 DML/DDL/DCL
 * 分类、mysql8/postgresql13/sqlite3 方言表、MySQL 8 函数表）；其核心的
 * schema 感知补全依赖真实数据库连接内省，不在纯离线范围（后续可经
 * SqlitePreview 的 rusqlite 连接做 SQLite 子集增强）。
 *
 * 上下文过滤（移植 sqls getCompletionTypes 的最表层规则，行级扫描无
 * parser）：光标前自上一分号以来的语句文本中，最后出现的子句关键字若为
 * 表引用类（FROM / JOIN / INTO / UPDATE）→ 只出关键字与方言关键字
 * （表名位不出函数）；其余（SELECT / WHERE / SET / VALUES …）→ 全量。
 *
 * 挂在 Monaco 内置 `sql` 语言上（Monarch 高亮零改动）；关键字大写插入
 * （SQL 惯例，对齐上游 uppercase 数据形态）。hover 不做：上游关键字表
 * 只有名字与分类、无文档文本。数据经 dynamic import 打成独立 async chunk。
 */

import type * as MonacoNs from 'monaco-editor';

// ─── 数据形状（sqlData/*.json，提取方式见该目录 README.md）───

type KeywordClass = 'matched' | 'dml' | 'ddl' | 'dcl';

interface SqlDataIndex {
  /** 基础关键字 → 分类 */
  keywords: Map<string, KeywordClass>;
  /** 方言关键字 → 方言标签（与基础表去重，基础表优先保分类） */
  dialect: Map<string, string>;
  functions: string[];
}

interface RawSqlData {
  keywords: Record<string, KeywordClass>;
  dialects: Record<string, string[]>;
  functions: string[];
}

// ─── 骨架 snippet（项目自写；InsertAsSnippet 语义明确受控，非上游数据）───

interface SqlSnippet {
  label: string;
  detail: string;
  insert: string;
}

const SQL_SNIPPETS: SqlSnippet[] = [
  {
    label: 'SELECT * FROM',
    detail: 'snippet',
    insert: 'SELECT *\nFROM ${1:table};',
  },
  {
    label: 'SELECT … WHERE',
    detail: 'snippet',
    insert: 'SELECT ${1:columns}\nFROM ${2:table}\nWHERE ${3:condition};',
  },
  {
    label: 'INNER JOIN',
    detail: 'snippet',
    insert: 'INNER JOIN ${1:table} ${2:alias}\n  ON ${3:condition}',
  },
  {
    label: 'LEFT JOIN',
    detail: 'snippet',
    insert: 'LEFT JOIN ${1:table} ${2:alias}\n  ON ${3:condition}',
  },
  {
    label: 'WITH … AS (CTE)',
    detail: 'snippet',
    insert: 'WITH ${1:cte_name} AS (\n  SELECT ${2:columns} FROM ${3:table}\n)\nSELECT *\nFROM ${1:cte_name};',
  },
  {
    label: 'INSERT INTO',
    detail: 'snippet',
    insert: 'INSERT INTO ${1:table} (${2:columns})\nVALUES (${3:values});',
  },
  {
    label: 'UPDATE … SET',
    detail: 'snippet',
    insert: 'UPDATE ${1:table}\nSET ${2:column} = ${3:value}\nWHERE ${4:condition};',
  },
  {
    label: 'CREATE TABLE',
    detail: 'snippet',
    insert: 'CREATE TABLE ${1:table} (\n  id BIGINT PRIMARY KEY,\n  ${2:column} ${3:TYPE}\n);',
  },
];

// ─── 上下文判定（表引用位不出函数；对齐 sqls 补全类型的最表层规则）───

/** 子句关键字 → 是否表引用位（FROM/JOIN/INTO/UPDATE 后面跟的是表名） */
const TABLE_REF_RE = /\b(FROM|JOIN|INTO|UPDATE)\b/gi;
const GENERAL_CLAUSE_RE =
  /\b(SELECT|WHERE|SET|VALUES|GROUP|ORDER|HAVING|LIMIT|ON|AND|OR|BY|AS|CASE|WHEN|THEN|ELSE|END|NOT|IN|EXISTS|BETWEEN|LIKE|DISTINCT|UNION|INSERT|DELETE|CREATE|ALTER|DROP|TRUNCATE|WITH|RETURNING)\b/gi;

export type SqlClauseContext = 'table-ref' | 'general';

/**
 * 判定光标所处子句：取自上一分号以来语句文本中最后出现的子句关键字，
 * 属表引用类则 table-ref。两个集合都无命中按 general（含空语句）。
 */
export function sqlClauseContext(statementText: string): SqlClauseContext {
  let tableRefAt = -1;
  let generalAt = -1;
  for (const m of statementText.matchAll(TABLE_REF_RE)) {
    tableRefAt = Math.max(tableRefAt, m.index ?? -1);
  }
  for (const m of statementText.matchAll(GENERAL_CLAUSE_RE)) {
    generalAt = Math.max(generalAt, m.index ?? -1);
  }
  return tableRefAt > generalAt ? 'table-ref' : 'general';
}

/** 截取自上一分号（语句边界）以来的文本 */
export function currentStatement(textBeforeCursor: string): string {
  const idx = textBeforeCursor.lastIndexOf(';');
  return idx >= 0 ? textBeforeCursor.slice(idx + 1) : textBeforeCursor;
}

// ─── 索引懒加载 ───

let dataIndex: SqlDataIndex | null = null;
let dataIndexLoading: Promise<SqlDataIndex> | null = null;

function buildIndex(raw: RawSqlData): SqlDataIndex {
  const keywords = new Map(Object.entries(raw.keywords));
  const dialect = new Map<string, string>();
  for (const [dialectName, names] of Object.entries(raw.dialects)) {
    for (const name of names) {
      // 基础表已有同名条目时保留其分类，不重复出（补全并集去重）
      if (!keywords.has(name) && !dialect.has(name)) {
        dialect.set(name, dialectName);
      }
    }
  }
  return { keywords, dialect, functions: raw.functions };
}

function ensureDataIndex(): Promise<SqlDataIndex> {
  if (dataIndex) return Promise.resolve(dataIndex);
  if (!dataIndexLoading) {
    dataIndexLoading = Promise.all([
      import('./sqlData/keywords.json'),
      import('./sqlData/dialects.json'),
      import('./sqlData/functions.json'),
    ]).then(([k, d, f]) => {
      dataIndex = buildIndex({
        keywords: k.default as Record<string, KeywordClass>,
        dialects: d.default as Record<string, string[]>,
        functions: f.default as string[],
      });
      return dataIndex;
    });
  }
  return dataIndexLoading;
}

// ─── 注册入口（幂等；monaco 实例由宿主传入）───

let registered = false;

export function ensureSqlLanguage(monaco: typeof MonacoNs): void {
  if (registered) return;
  registered = true;

  monaco.languages.registerCompletionItemProvider('sql', {
    triggerCharacters: [' '],
    async provideCompletionItems(model, position) {
      const index = await ensureDataIndex();
      const word = model.getWordUntilPosition(position);
      const replace = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn || position.column,
        endColumn: word.endColumn || position.column,
      };
      const before = model.getValueInRange({
        startLineNumber: 1,
        startColumn: 1,
        endLineNumber: position.lineNumber,
        endColumn: position.column,
      });
      // 限定符位置（标识符. 后，含刚敲完点）：只可能是列名/限定表名二段，
      // 关键字/函数/片段在这里全是噪音——让位给 schema 感知补全
      //（db_sql_completions 后端服务），保证 t. 后列表只剩目标字段
      if (/(?:[A-Za-z_$][\w$]*|"[^"]*"|`[^`]*`)\s*\.\s*[\w$]*$/.test(before)) {
        return { suggestions: [] };
      }
      const clause = sqlClauseContext(currentStatement(before));

      const items: MonacoNs.languages.CompletionItem[] = [];
      const keywordKind = monaco.languages.CompletionItemKind.Keyword;
      const classLabel: Record<KeywordClass, string | undefined> = {
        matched: undefined,
        dml: 'DML',
        ddl: 'DDL',
        dcl: 'DCL',
      };
      for (const [name, cls] of index.keywords) {
        items.push({
          label: name,
          kind: keywordKind,
          detail: classLabel[cls],
          insertText: name,
          sortText: `1${name}`,
          range: replace,
        });
      }
      for (const [name, tag] of index.dialect) {
        items.push({
          label: name,
          kind: keywordKind,
          detail: tag,
          insertText: name,
          sortText: `1${name}`,
          range: replace,
        });
      }
      // 表引用位（FROM/JOIN/INTO/UPDATE 后）出表名——函数不掺入
      if (clause === 'general') {
        const functionKind = monaco.languages.CompletionItemKind.Function;
        for (const name of index.functions) {
          items.push({
            label: name,
            kind: functionKind,
            detail: 'function',
            insertText: name,
            sortText: `2${name}`,
            range: replace,
          });
        }
      }
      const snippetKind = monaco.languages.CompletionItemKind.Snippet;
      for (const snippet of SQL_SNIPPETS) {
        items.push({
          label: snippet.label,
          kind: snippetKind,
          detail: snippet.detail,
          insertText: snippet.insert,
          insertTextRules:
            monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          sortText: `0${snippet.label}`,
          range: replace,
        });
      }
      return { suggestions: items };
    },
  });
}
