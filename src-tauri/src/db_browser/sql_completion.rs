//! SQL 编辑器 schema 感知补全（表 / 列 / 别名 t.xx）的服务侧实现。
//!
//! 前端 `db_sql_completions(sessionId, sql, cursorOffset)` 一条 invoke 拿全部
//! 建议：光标上下文解析（限定符 / 表名位 / 别名表收集）与目录数据解析都在
//! 本模块完成，前端只做 CompletionItem 薄映射。解析器是行级 token 扫描
//! （非 AST——补全触发的时机恰恰是语句敲到一半、AST 大概率解析不了的形态），
//! 语义与已被真机验证的前端初版一致，单测按同口径移植。
//!
//! 目录数据三层缓存挂在 DbSession 上（顶层 schema 名单 / 每库表清单 / 每表
//! 列清单，键一律小写）：建议框每次触发至多一趟 IPC，缓存命中后纯内存。
//! 缓存不设 TTL——失效由外部显式触发：`db_sql_completions_invalidate`
//! （前端对象树 catalogLoad：连接 / 切库 / 刷新时调用）与 `db_use_database`
//! （切库内联失效）。std Mutex 不跨 await 持有（查目录在锁外，回来再写回），
//! 并发补全可能重复拉一趟（幂等查询，无害）。
//!
//! 解析语义（与前端初版 sqlCompletion.ts 对齐，已知边界一致）：
//! - 语句边界 = 光标前最后一个 `;` 之后；字符串字面量 / `--` / `/* */`
//!   注释整段吞掉，其内的 from 等关键字不参与扫描；
//! - 表引用 = FROM / JOIN / UPDATE / INTO 后的标识符（支持 schema.table
//!   两段限定与 `FROM a, b` 旧式逗号联表），其后一个非停用词 token 视为
//!   别名（`AS x` 显式形态亦认）；
//! - 括号深度隔离子查询作用域：内层 FROM 的表不进外层引用集（其列在外层
//!   不可见，泄入会给出错误列建议）；
//! - 光标三态：限定形态（t. / 表名. / schema. 按 别名 → 裸表 → schema
//!   解析）、表名位（当前库表 + 其它 schema 名带 `.` 引导二段）、列名位
//!   （引用表列并集去重，无引用退化为表清单）；
//! - 排序前缀 `!1/!2/!3`（`!` < `0`，压过静态关键字 provider 的 0/1/2 段）。

use std::collections::HashMap;

use serde::Serialize;

use super::engines::{CatalogParent, DbEngine, ObjectGroup};
use super::{CatalogNode, DbSession};

// ── 光标上下文解析（纯函数） ───────────────────────────────────────────────

/// 语句内一张被引用的表（含别名指向的表）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefTable {
    pub schema: String,
    pub table: String,
}

/// 光标处上下文（`parse_cursor_context` 的产物）
#[derive(Debug, Default)]
pub struct CursorContext {
    /// `t.` 形态的限定符（已剥引号、原样大小写）；None = 非限定形态
    pub qualifier: Option<String>,
    /// word 在光标前文本中的字节偏移（换算替换范围用；词本身不需要——
    /// 前端拿 wordStart 到光标就是被替换范围）
    pub word_start: usize,
    /// 光标是否在表名位（FROM/JOIN/INTO/UPDATE 后敲标识符）
    pub table_position: bool,
    /// 别名表：alias 小写 → 表（schema + 裸表名）
    pub aliases: HashMap<String, RefTable>,
    /// 当前语句引用的全部表（含别名表，按 schema+table 小写去重）
    pub referenced: Vec<RefTable>,
}

const TABLE_KEYWORDS: [&str; 4] = ["from", "join", "update", "into"];

/// 别名位的停用词：表名后紧跟这些词时不是别名（SQL 常见子句/联结词）
const ALIAS_STOPWORDS: &[&str] = &[
    "on",
    "using",
    "where",
    "set",
    "values",
    "group",
    "order",
    "by",
    "limit",
    "offset",
    "having",
    "left",
    "right",
    "inner",
    "outer",
    "cross",
    "full",
    "natural",
    "join",
    "union",
    "minus",
    "intersect",
    "select",
    "from",
    "for",
    "with",
    "when",
    "then",
    "else",
    "end",
    "and",
    "or",
    "not",
    "as",
    "lateral",
    "partition",
    "duplicate",
    "returning",
    "wait",
    "nowait",
    "where",
    "into",
    "update",
];

struct Tok {
    start: usize,
    end: usize,
    ident: bool,
}

#[inline(always)]
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

#[inline(always)]
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

/// 语句粗分词：标识符 / 引号标识符 / 字符串 / 注释 / 单字符其它。
/// 字符串与注释整段吞成非标识 token（其内的 from 等不触发表扫描）。
/// 字节级扫描对非 ASCII 安全：字符类全 ASCII，边界必落在字符边界。
/// 索引全部走 `.get()`（仓库 deny indexing_slicing）。
fn tokenize(stmt: &str) -> Vec<Tok> {
    let b = stmt.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let Some(&c) = b.get(i) else { break };
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'-' && b.get(i.wrapping_add(1)) == Some(&b'-') {
            let start = i;
            while b.get(i).is_some_and(|&x| x != b'\n') {
                i += 1;
            }
            out.push(Tok {
                start,
                end: i,
                ident: false,
            });
            continue;
        }
        if c == b'/' && b.get(i.wrapping_add(1)) == Some(&b'*') {
            let start = i;
            i += 2;
            while i + 1 < b.len() && !(b.get(i) == Some(&b'*') && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(b.len());
            out.push(Tok {
                start,
                end: i,
                ident: false,
            });
            continue;
        }
        if c == b'\'' {
            let start = i;
            i += 1;
            while b.get(i).is_some_and(|&x| x != b'\'') {
                i += 1;
            }
            i = (i + 1).min(b.len());
            out.push(Tok {
                start,
                end: i,
                ident: false,
            });
            continue;
        }
        if c == b'"' || c == b'`' || c == b'[' {
            let close = if c == b'[' { b']' } else { c };
            let start = i;
            i += 1;
            while b.get(i).is_some_and(|&x| x != close) {
                // `""` 是双引号形态的转义（"a""b" 整体一个标识符）
                if close == b'"'
                    && b.get(i) == Some(&b'"')
                    && b.get(i.wrapping_add(1)) == Some(&b'"')
                {
                    i += 2;
                    continue;
                }
                i += 1;
            }
            i = (i + 1).min(b.len());
            out.push(Tok {
                start,
                end: i,
                ident: true,
            });
            continue;
        }
        if is_ident_start(c) {
            let start = i;
            while b.get(i).is_some_and(|&x| is_word_byte(x)) {
                i += 1;
            }
            out.push(Tok {
                start,
                end: i,
                ident: true,
            });
            continue;
        }
        out.push(Tok {
            start: i,
            end: i + 1,
            ident: false,
        });
        i += 1;
    }
    out
}

#[inline]
fn strip_ident_quotes(s: &str) -> &str {
    match (s.as_bytes().first(), s.as_bytes().last()) {
        (Some(b'"'), Some(b'"')) | (Some(b'`'), Some(b'`')) | (Some(b'['), Some(b']'))
            if s.len() >= 2 =>
        {
            &s[1..s.len() - 1]
        }
        _ => s,
    }
}

/// 解析光标处上下文，产出补全上下文（详见模块头注释）。
///
/// 入参是**全文 + 光标字节偏移**（不是光标前缀）：表引用与别名扫描跑在
/// 「包含光标的完整语句」上（前后分号定界）——补全常发生在语句中段，
/// 典型如 `SELECT t.u| FROM TUSER t`：别名 t 的定义在光标**之后**，只看
/// 前缀就永远对不上。光标词 / 限定符 / 表名位判定仍只看光标前缀。
///
/// 表引用/别名提取双路（sqlsplit 同型）：
/// - **主路（完美模式）**：整句语法完整时（中段编辑的典型态——语句全部
///   写完、光标只是停在中间）sqlparser 解析完整 AST，精准提取表因子与
///   别名——无停用词猜测、天然处理嵌套 JOIN/派生表/UNION 各臂；
/// - **降级路**：解析失败（语句真敲到一半：尾部悬空点/缺 FROM 等，或
///   方言特有语法）回退行级 token 扫描启发式，语义同前端初版。
pub fn parse_cursor_context(text: &str, cursor: usize, engine: &str) -> CursorContext {
    // 语句边界：光标前最后一个分号之后 → 光标后第一个分号之前
    let stmt_start = text
        .get(..cursor)
        .unwrap_or(text)
        .rfind(';')
        .map(|p| p + 1)
        .unwrap_or(0);
    let stmt_end = text
        .get(cursor..)
        .unwrap_or_default()
        .find(';')
        .map(|p| cursor + p)
        .unwrap_or(text.len());
    let stmt = text.get(stmt_start..stmt_end).unwrap_or_default();

    // 主路（完美模式）：整句语法完整 → AST 精准提取；降级路：解析失败
    //（真敲到一半/方言特有语法）→ 行级 token 扫描启发式
    let (aliases, referenced) =
        match ast_tables_and_aliases(stmt, super::sqlsplit::dialect_for_engine(engine)) {
            Some(v) => v,
            None => token_scan_tables(stmt),
        };

    // 光标词与限定符：只看光标前缀（语句内 stmt_start..cursor），从尾部回扫
    // `[标识符] 空白? . 空白? [词]`；word_start 补回语句起点得全文绝对偏移
    let prefix = text.get(stmt_start..cursor).unwrap_or_default();
    let (qualifier, _word, word_start_rel) = scan_word_and_qualifier(prefix);
    let word_start = stmt_start + word_start_rel;

    // 表名位：光标前缀的尾部是 `[表名位字符]*`，其前有空白，再前是表关键字。
    // 限定形态不走此标志——建议解析的限定分支先行返回；含空白后的位置
    // （FROM t |）视为列名位（正在敲别名/列）
    let table_position = ends_with_table_position(prefix);

    CursorContext {
        qualifier,
        word_start,
        table_position,
        aliases,
        referenced,
    }
}

// ── 主路：sqlparser AST 精准提取（解析成功 = 语句语法完整） ───────────────

/// 整句解析成功时返回表引用与别名；解析失败（语句敲到一半/方言特有语法）
/// 返回 None 由降级路接管
fn ast_tables_and_aliases(
    stmt: &str,
    dialect: &'static dyn sqlparser::dialect::Dialect,
) -> Option<(HashMap<String, RefTable>, Vec<RefTable>)> {
    let stmts = sqlparser::parser::Parser::parse_sql(dialect, stmt).ok()?;
    let mut aliases: HashMap<String, RefTable> = HashMap::new();
    let mut referenced: Vec<RefTable> = Vec::new();
    // 只取第一条语句（语句切片按分号定界本就单条）
    if let Some(stmt) = stmts.first() {
        collect_statement(stmt, &mut aliases, &mut referenced);
    }
    Some((aliases, referenced))
}

#[inline]
fn push_ref(r: RefTable, referenced: &mut Vec<RefTable>) {
    let dup = referenced.iter().any(|x| {
        x.table.eq_ignore_ascii_case(&r.table) && x.schema.eq_ignore_ascii_case(&r.schema)
    });
    if !dup {
        referenced.push(r);
    }
}

fn ref_from_object_name(name: &sqlparser::ast::ObjectName) -> RefTable {
    // 一段 = 裸表名（schema 空，建议端用当前库兜底）；两段 = schema.table
    let idents = &name.0;
    match (idents.first(), idents.get(1)) {
        (Some(t), Some(s)) => RefTable {
            schema: s.value.clone(),
            table: t.value.clone(),
        },
        (Some(t), None) => RefTable {
            schema: String::new(),
            table: t.value.clone(),
        },
        _ => RefTable {
            schema: String::new(),
            table: String::new(),
        },
    }
}

fn collect_factor(
    factor: &sqlparser::ast::TableFactor,
    aliases: &mut HashMap<String, RefTable>,
    referenced: &mut Vec<RefTable>,
) {
    use sqlparser::ast::TableFactor;
    if let TableFactor::Table { name, alias, .. } = factor {
        let r = ref_from_object_name(name);
        if !r.table.is_empty() {
            push_ref(r.clone(), referenced);
            if let Some(a) = alias {
                aliases.insert(a.name.value.to_ascii_lowercase(), r);
            }
        }
    }
    // Derived（派生表/子查询）与表函数：列不可知，不进引用集——
    // 其作用域隔离由 AST 结构天然保证（子查询内的表根本不在本层 from 里）
}

fn collect_twj(
    twj: &sqlparser::ast::TableWithJoins,
    aliases: &mut HashMap<String, RefTable>,
    referenced: &mut Vec<RefTable>,
) {
    collect_factor(&twj.relation, aliases, referenced);
    for j in &twj.joins {
        collect_factor(&j.relation, aliases, referenced);
    }
}

fn collect_body(
    body: &sqlparser::ast::SetExpr,
    aliases: &mut HashMap<String, RefTable>,
    referenced: &mut Vec<RefTable>,
) {
    use sqlparser::ast::SetExpr;
    match body {
        // UNION/EXCEPT/INTERSECT：两臂的表都在结果作用域内
        SetExpr::SetOperation { left, right, .. } => {
            collect_body(left, aliases, referenced);
            collect_body(right, aliases, referenced);
        }
        SetExpr::Query(q) => collect_query(q, aliases, referenced),
        SetExpr::Select(select) => {
            for twj in &select.from {
                collect_twj(twj, aliases, referenced);
            }
        }
        _ => {}
    }
    // SELECT 列表/WHERE 里的标量子查询不在本层作用域（列不可见），不递归——
    // 这正是 AST 相对 token 扫描的作用域优势
}

fn collect_query(
    q: &sqlparser::ast::Query,
    aliases: &mut HashMap<String, RefTable>,
    referenced: &mut Vec<RefTable>,
) {
    // CTE（WITH 子句）体在独立作用域，不进本层引用集
    collect_body(&q.body, aliases, referenced);
}

fn collect_statement(
    stmt: &sqlparser::ast::Statement,
    aliases: &mut HashMap<String, RefTable>,
    referenced: &mut Vec<RefTable>,
) {
    use sqlparser::ast::Statement;
    match stmt {
        Statement::Query(q) => collect_query(q, aliases, referenced),
        Statement::Insert(insert) => {
            let r = ref_from_object_name(&insert.table_name);
            if !r.table.is_empty() {
                push_ref(r, referenced);
            }
        }
        Statement::Update { table, .. } => collect_twj(table, aliases, referenced),
        Statement::Delete(delete) => {
            // 0.53 的 FROM 形态：显式 FROM 关键字 / BigQuery 省略形态，
            // 两分支都是 TableWithJoins 列表
            use sqlparser::ast::FromTable;
            let twjs: Vec<&sqlparser::ast::TableWithJoins> = match &delete.from {
                FromTable::WithFromKeyword(f) => f.iter().collect(),
                FromTable::WithoutKeyword(t) => t.iter().collect(),
            };
            for twj in twjs {
                collect_twj(twj, aliases, referenced);
            }
        }
        _ => {}
    }
}

// ── 降级路：行级 token 扫描启发式（解析失败时接管，语义同前端初版） ────────

fn token_scan_tables(stmt: &str) -> (HashMap<String, RefTable>, Vec<RefTable>) {
    let toks = tokenize(stmt);
    let mut aliases: HashMap<String, RefTable> = HashMap::new();
    let mut referenced: Vec<RefTable> = Vec::new();

    // 消费一个表引用：ident [. ident] [AS] [alias]，返回扫描推进到的下标
    fn consume_ref(
        start: usize,
        toks: &[Tok],
        stmt: &str,
        aliases: &mut HashMap<String, RefTable>,
        referenced: &mut Vec<RefTable>,
    ) -> usize {
        let text = |t: &Tok| stmt.get(t.start..t.end).unwrap_or_default();
        let mut schema = String::new();
        let mut table =
            strip_ident_quotes(toks.get(start).map(text).unwrap_or_default()).to_string();
        let mut end = start + 1;
        if let (Some(dot), Some(name)) = (toks.get(end), toks.get(end + 1).filter(|t| t.ident))
            && !dot.ident
            && dot.text_is(stmt, ".")
        {
            schema = table;
            table = strip_ident_quotes(text(name)).to_string();
            end += 2;
        }
        // 裸表名缺 schema 记空串，由建议端用当前库兑底匹配
        let dup = referenced.iter().any(|x| {
            x.table.eq_ignore_ascii_case(&table) && x.schema.eq_ignore_ascii_case(&schema)
        });
        if !dup {
            referenced.push(RefTable {
                schema: schema.clone(),
                table: table.clone(),
            });
        }
        if let Some(next) = toks.get(end).filter(|t| t.ident) {
            let w = text(next).to_ascii_lowercase();
            if w == "as"
                && let Some(alias_tok) = toks.get(end + 1).filter(|t| t.ident)
            {
                aliases.insert(
                    strip_ident_quotes(text(alias_tok)).to_ascii_lowercase(),
                    RefTable { schema, table },
                );
                end += 2;
            } else if !ALIAS_STOPWORDS.contains(&w.as_str()) {
                aliases.insert(
                    strip_ident_quotes(text(next)).to_ascii_lowercase(),
                    RefTable { schema, table },
                );
                end += 1;
            }
        }
        end
    }

    let mut depth: u32 = 0;
    let mut i = 0;
    while i < toks.len() {
        let Some(t) = toks.get(i) else { break };
        let text = stmt.get(t.start..t.end).unwrap_or_default();
        if !t.ident && text == "(" {
            depth += 1;
        } else if !t.ident && text == ")" {
            depth = depth.saturating_sub(1);
        }
        // 括号内的表（子查询）不进外层作用域：其列在外层不可见
        if depth > 0 || !t.ident || !TABLE_KEYWORDS.contains(&text.to_ascii_lowercase().as_str()) {
            i += 1;
            continue;
        }
        // FROM ( 子查询：跳过（派生表列不可知）
        if toks
            .get(i + 1)
            .is_some_and(|n| !n.ident && n.text_is(stmt, "("))
        {
            i += 1;
            continue;
        }
        if !toks.get(i + 1).is_some_and(|n| n.ident) {
            i += 1;
            continue;
        }
        // FROM a, b 旧式逗号联表：每段都是表引用（各可带别名）
        let mut end = consume_ref(i + 1, &toks, stmt, &mut aliases, &mut referenced);
        while toks
            .get(end)
            .is_some_and(|n| !n.ident && n.text_is(stmt, ","))
            && toks.get(end + 1).is_some_and(|n| n.ident)
        {
            end = consume_ref(end + 1, &toks, stmt, &mut aliases, &mut referenced);
        }
        i = end;
    }
    (aliases, referenced)
}

impl Tok {
    #[inline(always)]
    fn text_is(&self, stmt: &str, expect: &str) -> bool {
        &stmt[self.start..self.end] == expect
    }
}

/// 从语句尾部回扫：`(qualifier . )? word`，返回（限定符, 词, 词的字节起点）。
/// 词与限定符均允许敲到一半（空串 / 局部引号按引号字符并入扫描）
fn scan_word_and_qualifier(stmt: &str) -> (Option<String>, String, usize) {
    let b = stmt.as_bytes();
    let n = b.len();
    let mut i = n;
    while i > 0 && b.get(i - 1).is_some_and(|&x| is_word_byte(x)) {
        i -= 1;
    }
    let word = stmt.get(i..n).unwrap_or_default().to_string();
    let word_start = i;
    let mut j = i;
    while j > 0 && b.get(j - 1).is_some_and(u8::is_ascii_whitespace) {
        j -= 1;
    }
    let qualifier = (j > 0 && b.get(j - 1) == Some(&b'.')).then(|| {
        let mut k = j - 1;
        while k > 0 && b.get(k - 1).is_some_and(u8::is_ascii_whitespace) {
            k -= 1;
        }
        let mut m = k;
        while m > 0
            && b.get(m - 1)
                .is_some_and(|&x| is_word_byte(x) || matches!(x, b'"' | b'`' | b'[' | b']'))
        {
            m -= 1;
        }
        strip_ident_quotes(stmt.get(m..k).unwrap_or_default()).to_string()
    });
    (qualifier, word, word_start)
}

/// 表名位判定：尾部是 `[表名位字符]*`，其前有空白，再前是表关键字
fn ends_with_table_position(stmt: &str) -> bool {
    let b = stmt.as_bytes();
    let mut i = b.len();
    // 表名位字符：词字符 + 限定符字符（. " ` [ ]）
    while i > 0
        && b.get(i - 1)
            .is_some_and(|&x| is_word_byte(x) || matches!(x, b'.' | b'"' | b'`' | b'[' | b']'))
    {
        i -= 1;
    }
    let mut j = i;
    let mut saw_ws = false;
    while j > 0 && b.get(j - 1).is_some_and(u8::is_ascii_whitespace) {
        j -= 1;
        saw_ws = true;
    }
    if !saw_ws {
        return false;
    }
    let mut k = j;
    while k > 0 && b.get(k - 1).is_some_and(|&x| is_word_byte(x)) {
        k -= 1;
    }
    // 词边界：关键字前不得还是词字符（防 xxxfrom 误判）
    if k > 0 && b.get(k - 1).is_some_and(|&x| is_word_byte(x)) {
        return false;
    }
    let kw = stmt.get(k..j).unwrap_or_default().to_ascii_lowercase();
    TABLE_KEYWORDS.contains(&kw.as_str())
}

// ── 目录缓存（挂 DbSession） ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TableEntry {
    pub schema: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone)]
pub struct ColumnEntry {
    pub name: String,
    pub data_type: String,
}

/// 补全目录缓存（详见模块头：失效只走显式 invalidate / 切库）
#[derive(Debug, Default)]
pub struct SqlCompletionCache {
    /// 顶层 schema / database 名单；None = 未拉过
    pub schemas: Option<Vec<String>>,
    /// 每库表类对象，键 = 库名小写
    pub tables: HashMap<String, Vec<TableEntry>>,
    /// 每表列，键 = `schema.table` 小写
    pub columns: HashMap<String, Vec<ColumnEntry>>,
}

impl SqlCompletionCache {
    #[inline]
    pub fn clear(&mut self) {
        self.schemas = None;
        self.tables.clear();
        self.columns.clear();
    }
}

// ── 建议产物（前端薄映射为 monaco CompletionItem） ─────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlCompletionItem {
    pub label: String,
    /// column | table | schema（前端映射 Field / Class / Module 图标）
    pub kind: String,
    pub detail: String,
    pub insert_text: String,
    /// 排序前缀：`!1`(列)/`!2`(表)/`!3`(schema)，`!` 压过静态关键字段
    pub sort: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlCompletionResult {
    pub suggestions: Vec<SqlCompletionItem>,
    /// 被替换词的起始偏移（UTF-16 码元，monaco getPositionAt 语义）
    pub word_start: usize,
}

fn kind_label(kind: &str) -> String {
    match kind {
        "table" => "TABLE".into(),
        "view" => "VIEW".into(),
        "mview" => "MATERIALIZED VIEW".into(),
        "ftable" => "FOREIGN TABLE".into(),
        "synonym" => "SYNONYM".into(),
        other => other.to_ascii_uppercase(),
    }
}

fn column_drafts(cols: &[ColumnEntry], table: &str) -> Vec<SqlCompletionItem> {
    cols.iter()
        .map(|c| SqlCompletionItem {
            label: c.name.clone(),
            kind: "column".into(),
            detail: format!("{table}.{} · {}", c.name, c.data_type),
            insert_text: c.name.clone(),
            sort: format!("!1{}", c.name.to_ascii_lowercase()),
        })
        .collect()
}

fn table_drafts(tables: &[TableEntry]) -> Vec<SqlCompletionItem> {
    tables
        .iter()
        .map(|t| SqlCompletionItem {
            label: t.name.clone(),
            kind: "table".into(),
            detail: format!("{} · {}", t.schema, kind_label(&t.kind)),
            insert_text: t.name.clone(),
            sort: format!("!2{}", t.name.to_ascii_lowercase()),
        })
        .collect()
}

// ── 解析服务（async：缓存未命中时经引擎查目录） ─────────────────────────────

/// UTF-16 码元偏移 → 字节偏移（前端 cursorOffset / monaco getOffsetAt 均为
/// UTF-16 语义；非 ASCII 的 SQL 标识符两侧需换算）
#[inline]
fn utf16_to_byte(s: &str, off: usize) -> usize {
    let mut units = 0usize;
    for (i, ch) in s.char_indices() {
        if units >= off {
            return i;
        }
        units += ch.len_utf16();
    }
    s.len()
}

#[inline(always)]
fn byte_to_utf16(s: &str, byte: usize) -> usize {
    s[..byte].chars().map(char::len_utf16).sum()
}

async fn fetch_schemas(engine: &mut DbEngine) -> Result<Vec<String>, String> {
    let nodes: Vec<CatalogNode> = engine.catalog(&CatalogParent::Root).await?;
    Ok(nodes
        .into_iter()
        .filter(|n| n.kind == "database" || n.kind == "schema")
        .map(|n| n.name)
        .collect())
}

async fn fetch_tables(engine: &mut DbEngine, schema: &str) -> Result<Vec<TableEntry>, String> {
    let groups = engine
        .catalog(&CatalogParent::Schema {
            schema: schema.to_string(),
        })
        .await?;
    let mut out = Vec::new();
    for g in &groups {
        // group 规范名在 key 的 `#` 后段（小写；name 是大写展示名）
        let group_str = g.key.split('#').nth(1).unwrap_or_default();
        let Some(group) = ObjectGroup::parse(group_str) else {
            continue;
        };
        if !matches!(
            group,
            ObjectGroup::Table
                | ObjectGroup::View
                | ObjectGroup::MaterializedView
                | ObjectGroup::ForeignTable
                | ObjectGroup::Synonym
        ) {
            continue;
        }
        let nodes = engine
            .catalog(&CatalogParent::Group {
                schema: schema.to_string(),
                group,
            })
            .await?;
        for n in nodes {
            out.push(TableEntry {
                schema: schema.to_string(),
                name: n.name,
                kind: n.kind,
            });
        }
    }
    Ok(out)
}

async fn fetch_columns(
    engine: &mut DbEngine,
    schema: &str,
    table: &str,
) -> Result<Vec<ColumnEntry>, String> {
    let nodes = engine
        .catalog(&CatalogParent::Table {
            schema: schema.to_string(),
            table: table.to_string(),
        })
        .await?;
    Ok(nodes
        .into_iter()
        .filter(|n| n.kind == "column")
        .map(|n| ColumnEntry {
            name: n.name,
            data_type: n.detail.unwrap_or_default(),
        })
        .collect())
}

/// 缓存包装：命中直接返回；未命中在锁外查目录、回来写回（std Mutex 不跨
/// await 持有；并发未命中可能重复拉一趟，幂等无害）
async fn cached_schemas(
    engine: &mut DbEngine,
    cache: &std::sync::Mutex<SqlCompletionCache>,
) -> Result<Vec<String>, String> {
    if let Some(v) = cache.lock().ok().and_then(|c| c.schemas.clone()) {
        return Ok(v);
    }
    let v = fetch_schemas(engine).await?;
    if let Ok(mut c) = cache.lock() {
        c.schemas = Some(v.clone());
    }
    Ok(v)
}

async fn cached_tables(
    engine: &mut DbEngine,
    cache: &std::sync::Mutex<SqlCompletionCache>,
    schema: &str,
) -> Result<Vec<TableEntry>, String> {
    let key = schema.to_ascii_lowercase();
    if let Some(v) = cache.lock().ok().and_then(|c| c.tables.get(&key).cloned()) {
        return Ok(v);
    }
    let v = fetch_tables(engine, schema).await?;
    if let Ok(mut c) = cache.lock() {
        c.tables.insert(key, v.clone());
    }
    Ok(v)
}

async fn cached_columns(
    engine: &mut DbEngine,
    cache: &std::sync::Mutex<SqlCompletionCache>,
    schema: &str,
    table: &str,
) -> Result<Vec<ColumnEntry>, String> {
    let key = format!(
        "{}.{}",
        schema.to_ascii_lowercase(),
        table.to_ascii_lowercase()
    );
    if let Some(v) = cache.lock().ok().and_then(|c| c.columns.get(&key).cloned()) {
        return Ok(v);
    }
    let v = fetch_columns(engine, schema, table).await?;
    if let Ok(mut c) = cache.lock() {
        c.columns.insert(key, v.clone());
    }
    Ok(v)
}

/// 一条表引用的列：先对目录规范名再查——大小写敏感引擎（Oracle/达梦目录存
/// 大写 EMP，用户敲小写 emp；PG 反向存小写）拿原样名查列会零行返回空建议。
/// 显式两段限定的库名也先对顶层名单规范（sc → SCOTT）；都不中（非表类对象/
/// 清单外对象）按原样兑底试一趟
async fn ref_columns(
    engine: &mut DbEngine,
    cache: &std::sync::Mutex<SqlCompletionCache>,
    current_db: &str,
    r: &RefTable,
) -> Result<Vec<ColumnEntry>, String> {
    let schema = if r.schema.is_empty() {
        current_db.to_string()
    } else {
        let schemas = cached_schemas(engine, cache).await.unwrap_or_default();
        schemas
            .iter()
            .find(|s| s.eq_ignore_ascii_case(&r.schema))
            .cloned()
            .unwrap_or_else(|| r.schema.clone())
    };
    let tables = cached_tables(engine, cache, &schema)
        .await
        .unwrap_or_default();
    let (cs, ct) = tables
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(&r.table))
        .map(|t| (t.schema.clone(), t.name.clone()))
        .unwrap_or((schema, r.table.clone()));
    cached_columns(engine, cache, &cs, &ct).await
}

/// 主入口：解析光标上下文并产出建议（Redis 引擎由命令层提前拦下，不进此处）
pub async fn sql_completions(
    session: &DbSession,
    sql: &str,
    cursor_offset_utf16: usize,
) -> Result<SqlCompletionResult, String> {
    let cursor_byte = utf16_to_byte(sql, cursor_offset_utf16);
    // 元信息一次读取：engine 决定 AST 方言，database 是裸表名的 schema 兜底
    let (engine, current_db) = {
        let meta = session
            .meta
            .read()
            .map_err(|_| "DB 会话元信息锁中毒".to_string())?;
        (
            meta.engine.clone(),
            meta.database.clone().unwrap_or_default(),
        )
    };
    let ctx = parse_cursor_context(sql, cursor_byte, &engine);
    let word_start_u16 = byte_to_utf16(sql, ctx.word_start);
    let empty = || SqlCompletionResult {
        suggestions: vec![],
        word_start: word_start_u16,
    };

    if current_db.is_empty() {
        // 无当前库（理论不可达：各引擎连接后都带默认库）——空建议保守返回
        return Ok(empty());
    }
    let mut engine = session.engine.lock().await;
    let cache = &session.completion_cache;

    // —— 限定形态 t.｜table.｜schema. ——
    if let Some(q) = &ctx.qualifier {
        let ql = q.to_ascii_lowercase();
        // ① 别名
        if let Some(t) = ctx.aliases.get(&ql) {
            let cols = ref_columns(&mut engine, cache, &current_db, t).await?;
            return Ok(SqlCompletionResult {
                suggestions: column_drafts(&cols, &t.table),
                word_start: word_start_u16,
            });
        }
        // ② 裸表名（当前库）
        let tables = cached_tables(&mut engine, cache, &current_db).await?;
        if let Some(bare) = tables.iter().find(|t| t.name.eq_ignore_ascii_case(q)) {
            let cols = cached_columns(&mut engine, cache, &bare.schema, &bare.name).await?;
            return Ok(SqlCompletionResult {
                suggestions: column_drafts(&cols, &bare.name),
                word_start: word_start_u16,
            });
        }
        // ③ schema 名 → 该库的表
        let schemas = cached_schemas(&mut engine, cache).await?;
        if let Some(hit) = schemas.iter().find(|s| s.eq_ignore_ascii_case(q)) {
            let st = cached_tables(&mut engine, cache, hit).await?;
            return Ok(SqlCompletionResult {
                suggestions: table_drafts(&st),
                word_start: word_start_u16,
            });
        }
        return Ok(empty());
    }

    // —— 表名位：当前库的表 + 其它 schema（带 `.` 引导二段）——
    if ctx.table_position {
        let tables = cached_tables(&mut engine, cache, &current_db).await?;
        let schemas = cached_schemas(&mut engine, cache).await?;
        let mut items = table_drafts(&tables);
        items.extend(
            schemas
                .iter()
                .filter(|s| !s.eq_ignore_ascii_case(&current_db))
                .map(|s| SqlCompletionItem {
                    label: s.clone(),
                    kind: "schema".into(),
                    detail: "SCHEMA".into(),
                    insert_text: format!("{s}."),
                    sort: format!("!3{}", s.to_ascii_lowercase()),
                }),
        );
        return Ok(SqlCompletionResult {
            suggestions: items,
            word_start: word_start_u16,
        });
    }

    // —— 列名位：引用表的列并集去重；无引用退化为表清单 ——
    if ctx.referenced.is_empty() {
        let tables = cached_tables(&mut engine, cache, &current_db).await?;
        return Ok(SqlCompletionResult {
            suggestions: table_drafts(&tables),
            word_start: word_start_u16,
        });
    }
    let mut merged: Vec<SqlCompletionItem> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for r in &ctx.referenced {
        let cols = ref_columns(&mut engine, cache, &current_db, r).await?;
        for c in &cols {
            let key = c.name.to_ascii_lowercase();
            if seen.contains(&key) {
                // 同名列：detail 追加第二个出处（表名区分）
                if let Some(prev) = merged.iter_mut().find(|d| d.sort == format!("!1{key}")) {
                    prev.detail = format!("{} · {}.{}", prev.detail, r.table, c.name);
                }
                continue;
            }
            seen.push(key.clone());
            merged.push(SqlCompletionItem {
                label: c.name.clone(),
                kind: "column".into(),
                detail: format!("{}.{} · {}", r.table, c.name, c.data_type),
                insert_text: c.name.clone(),
                sort: format!("!1{key}"),
            });
        }
    }
    Ok(SqlCompletionResult {
        suggestions: merged,
        word_start: word_start_u16,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(s: &str) -> CursorContext {
        parse_cursor_context(s, s.len(), "postgres")
    }

    fn ctx_at(s: &str, cursor: usize) -> CursorContext {
        parse_cursor_context(s, cursor, "postgres")
    }

    #[test]
    fn table_refs_and_aliases() {
        let c = ctx("SELECT * FROM emp e JOIN dept d ON e.deptno = d.deptno WHERE ");
        assert_eq!(c.referenced.len(), 2);
        assert_eq!(c.aliases.get("e").map(|t| t.table.as_str()), Some("emp"));
        assert_eq!(c.aliases.get("d").map(|t| t.table.as_str()), Some("dept"));
        assert!(!c.table_position);
    }

    #[test]
    fn explicit_as_alias() {
        let c = ctx("SELECT * FROM emp AS e WHERE ");
        assert_eq!(c.aliases.get("e").map(|t| t.table.as_str()), Some("emp"));
    }

    #[test]
    fn update_no_alias_after_set_stopword() {
        let c = ctx("UPDATE emp SET ");
        assert_eq!(c.referenced.first().map(|t| t.table.as_str()), Some("emp"));
        assert!(c.aliases.is_empty());
    }

    #[test]
    fn statement_boundary_after_semicolon() {
        let c = ctx("INSERT INTO orders (id) VALUES (1); SELECT * FROM ");
        assert!(c.referenced.is_empty());
        assert!(c.table_position);
    }

    #[test]
    fn table_position_lowercase_keyword() {
        assert!(ctx("select * from ").table_position);
        assert!(!ctx("SELECT * FROM t WHERE x ").table_position);
        assert!(!ctx("SELECT * FROM txxxfrom ").table_position);
    }

    #[test]
    fn two_part_qualified_table_with_alias() {
        let c = ctx("SELECT * FROM sc.emp x WHERE ");
        let r = c.referenced.first().expect("两段限定表");
        assert_eq!(r.schema, "sc");
        assert_eq!(r.table, "emp");
        assert_eq!(c.aliases.get("x").map(|t| t.schema.as_str()), Some("sc"));
    }

    #[test]
    fn comma_join_lists_all_tables() {
        let c = ctx("SELECT * FROM emp e, dept d WHERE ");
        assert_eq!(c.referenced.len(), 2);
        assert_eq!(c.aliases.get("d").map(|t| t.table.as_str()), Some("dept"));
    }

    #[test]
    fn subquery_scope_isolated() {
        let c = ctx("SELECT * FROM (SELECT 1 FROM dual) sub WHERE ");
        assert!(c.referenced.is_empty());
        assert!(c.aliases.is_empty());
    }

    #[test]
    fn keywords_inside_string_literal_ignored() {
        let c = ctx("SELECT * FROM t WHERE note = 'from and join' ");
        assert_eq!(c.referenced.len(), 1);
    }

    #[test]
    fn comments_do_not_trigger_scan() {
        let c = ctx("SELECT 1 -- from secret\nFROM t ");
        assert_eq!(c.referenced.len(), 1);
        assert_eq!(c.referenced.first().map(|t| t.table.as_str()), Some("t"));
    }

    #[test]
    fn qualifier_and_word_detection() {
        let c = ctx("SELECT e.de");
        assert_eq!(c.qualifier.as_deref(), Some("e"));
        assert_eq!(c.word_start, 9);
        let c = ctx("SELECT e.");
        assert_eq!(c.qualifier.as_deref(), Some("e"));
        let c = ctx("SELECT * FROM employees WHERE first_na");
        assert!(c.qualifier.is_none());
        assert_eq!(ctx("SELECT * FROM employees WHERE first_na").word_start, 30);
    }

    #[test]
    fn dot_inside_string_literal_is_not_qualifier() {
        assert!(
            ctx("SELECT * FROM emp WHERE c1 = 'ab.'")
                .qualifier
                .is_none()
        );
    }

    #[test]
    fn quoted_identifier_table() {
        let c = ctx(r#"SELECT * FROM "My Table" x WHERE "#);
        let r = c.referenced.first().expect("引号表名");
        assert_eq!(r.table, "My Table");
        assert_eq!(
            c.aliases.get("x").map(|t| t.table.as_str()),
            Some("My Table")
        );
    }

    #[test]
    fn mid_statement_alias_defined_after_cursor() {
        // SELECT 列表在 FROM 之前：别名定义在光标之后也要能解析（补全最常见
        // 的中段编辑场景）
        let c = ctx_at("SELECT t.u FROM TUSER t", 10);
        assert_eq!(c.qualifier.as_deref(), Some("t"));
        assert_eq!(c.aliases.get("t").map(|t| t.table.as_str()), Some("TUSER"));
        assert_eq!(c.word_start, 9);

        // 裸列名位同样受益：语句后半的 FROM 收集得到引用表
        let c = ctx_at("SELECT u FROM TUSER t WHERE ", 8);
        assert!(c.qualifier.is_none());
        assert_eq!(
            c.referenced.first().map(|t| t.table.as_str()),
            Some("TUSER")
        );

        // 光标后的分号仍是语句边界：尾巴的下一句不污染本句
        let c = ctx_at("SELECT t.u FROM TUSER t; SELECT * FROM dual", 10);
        assert_eq!(c.aliases.len(), 1);
        assert!(c.referenced.iter().all(|r| r.table != "dual"));
    }

    #[test]
    fn ast_path_joins_and_nested_scope() {
        // 语法完整的语句走 AST 主路：JOIN 链别名、标量子查询隔离、
        // UNION 两臂都在作用域内
        let c =
            ctx("SELECT (SELECT max(1) FROM secret_tab) FROM emp e UNION ALL SELECT * FROM dept d");
        let tables: Vec<&str> = c.referenced.iter().map(|r| r.table.as_str()).collect();
        assert!(
            tables.contains(&"emp") && tables.contains(&"dept"),
            "{tables:?}"
        );
        assert!(
            !tables.contains(&"secret_tab"),
            "标量子查询内的表不得进外层"
        );
        assert_eq!(c.aliases.get("e").map(|t| t.table.as_str()), Some("emp"));
        assert_eq!(c.aliases.get("d").map(|t| t.table.as_str()), Some("dept"));

        // 引号别名（token 启发式同判，AST 主路同判）
        let c = ctx(r#"SELECT * FROM emp AS "E" "#);
        assert_eq!(c.aliases.get("e").map(|t| t.table.as_str()), Some("emp"));

        // UPDATE/DELETE 的目标表
        let c = ctx("DELETE FROM log_table WHERE 1=1");
        assert_eq!(
            c.referenced.first().map(|t| t.table.as_str()),
            Some("log_table")
        );
    }

    #[test]
    fn fallback_path_on_incomplete_statement() {
        // 残缺语句降级 token 扫描，既有语义保持；注意 sqlparser 的接受面比
        // 想象宽（JOIN 无 ON 也能解析）——真正的降级触发是悬空点/缺子句等
        let c = ctx("SELECT * FROM emp e JOIN dep");
        let tables: Vec<&str> = c.referenced.iter().map(|r| r.table.as_str()).collect();
        assert!(
            tables.contains(&"emp") && tables.contains(&"dep"),
            "{tables:?}"
        );
        assert_eq!(c.aliases.get("e").map(|t| t.table.as_str()), Some("emp"));

        // 真正解析不了的形态：尾部悬空点（光标词所在）→ 降级路接住
        let c = ctx("SELECT * FROM emp e WHERE e.");
        assert_eq!(c.aliases.get("e").map(|t| t.table.as_str()), Some("emp"));
        assert_eq!(c.qualifier.as_deref(), Some("e"));
    }

    #[test]
    fn utf16_offset_conversion() {
        // "SELECT 中 FROM t"：中 是 1 个 UTF-16 码元、3 字节；光标停在 中 之后
        // 的 UTF-16 偏移是 8（SELECT␣ = 7 码元 + 中 1 码元）
        let s = "SELECT 中 FROM t";
        assert_eq!(utf16_to_byte(s, 8), 10);
        assert_eq!(byte_to_utf16(s, 10), 8);
        assert_eq!(utf16_to_byte(s, 999), s.len());
        // 代理对（𝄞 = 2 码元 4 字节）
        let s2 = "a𝄞b";
        assert_eq!(byte_to_utf16(s2, 5), 3);
        assert_eq!(utf16_to_byte(s2, 3), 5);
    }
}
