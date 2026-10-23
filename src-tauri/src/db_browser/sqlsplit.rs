//! SQL 语句分割与只读白名单判定（数据库工作台的写防护前置层）。
//!
//! 双路策略：
//! - **主路**：sqlparser 按方言解析整段 SQL → 精确得到语句列表 + 每条的首节点
//!   类型（用于只读白名单判定）。
//! - **降级路**：sqlparser 解析失败（方言特有语法 / 存储过程体等）时回退到
//!   手写状态机分号分割（字符串 / 注释 / 反引号内的分号不算），只读判定退回
//!   前缀白名单（sqlite_viewer 同型）。sqlparser 只是增强路径而非硬依赖——
//!   工作台绝不能因为「解析器不认识这条方言」就拒绝执行。
//!
//! SQLite 方言经 SQLiteDialect 解析；PRAGMA 的只读判定按「有无赋值」区分
//! （无值 = 只读查询，带值 = 写操作）。

use sqlparser::ast::Statement;
use sqlparser::dialect::{Dialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect};
use sqlparser::parser::Parser;

/// 语句的只读判定结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitStatement {
    pub sql: String,
    /// 主路解析成功时给 true（只读判定走 AST 节点名）
    pub parsed: bool,
}

/// 按引擎取方言。SQLite 走 SQLiteDialect（PRAGMA / AUTOINCREMENT / 双引号
/// 标识符都能正确解析）。Oracle 仍无专属方言（sqlparser 0.53 已无
/// OracleDialect），走 PostgreSqlDialect 主路——常规 SELECT/WITH 能解析
/// （只读判定走 AST），Oracle 特有语法（CONNECT BY 等）解析失败自然落入
/// 降级路，双路设计本就容忍方言缺失（绝不能因解析器不认识就拒绝执行）。
pub fn dialect_for_engine(engine: &str) -> &'static dyn Dialect {
    match engine {
        "mysql" => &MySqlDialect {},
        "sqlite" => &SQLiteDialect {},
        _ => &PostgreSqlDialect {},
    }
}

/// 分割多语句。空语句（仅空白/注释）被丢弃。
/// Oracle：含 PL/SQL 块（词界 BEGIN/DECLARE）的文本整段作单条语句——
/// 块内的分号（IF/END IF、逐语句 DML）不是语句边界，朴素分割会撕裂。
/// SQLite：触发器体（BEGIN...END）目前按分号分割，多语句编辑器里请整段
/// 单独执行。
pub fn split_statements(engine: &str, sql: &str) -> Vec<SplitStatement> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if engine == "oracle" && contains_plsql_block(trimmed) {
        return vec![SplitStatement {
            sql: trimmed.to_string(),
            parsed: false,
        }];
    }
    match Parser::parse_sql(dialect_for_engine(engine), trimmed) {
        Ok(statements) if !statements.is_empty() => {
            let parts = split_by_semicolon(trimmed);
            // 主路成功：语句数应与朴素分割一致（不一致说明有非分号结尾的复合语句，
            // 回退整段一条执行，避免错位）
            if parts.len() == statements.len() {
                parts
                    .into_iter()
                    .map(|sql| SplitStatement { sql, parsed: true })
                    .collect()
            } else {
                vec![SplitStatement {
                    sql: trimmed.to_string(),
                    parsed: false,
                }]
            }
        }
        _ => split_by_semicolon(trimmed)
            .into_iter()
            .map(|sql| SplitStatement { sql, parsed: false })
            .collect(),
    }
}

/// 词边界 PL/SQL 块检测（仅 Oracle 引擎使用）：跳过字符串/注释后扫描，
/// 出现独立的 BEGIN 或 DECLARE 关键词即视为含块。普通 SQL 几乎不含裸
/// BEGIN（MySQL 事务缩写也仅限 mysql 引擎）；字符串/注释里的字面量
/// 'BEGIN' 不误判。CREATE OR REPLACE PROCEDURE/FUNCTION 体含 BEGIN，
/// 同样整段化——从工作台执行存储过程定义是合法场景。
fn contains_plsql_block(sql: &str) -> bool {
    #[derive(PartialEq)]
    enum St {
        Code,
        Str,
        LineCmt,
        BlockCmt,
    }
    let chars: Vec<char> = sql.chars().collect();
    let mut st = St::Code;
    let mut i = 0usize;
    while i < chars.len() {
        let c = *chars.get(i).unwrap_or(&'\0');
        let next = chars.get(i + 1).copied();
        match st {
            St::Code => match c {
                '\'' => st = St::Str,
                '-' if next == Some('-') => st = St::LineCmt,
                '/' if next == Some('*') => st = St::BlockCmt,
                _ => {
                    // 词首且是 BEGIN/DECLARE 关键词（前一个非词字符、后续词界）
                    let word_start = i == 0
                        || !chars
                            .get(i - 1)
                            .is_some_and(|p| p.is_ascii_alphanumeric() || *p == '_');
                    if word_start {
                        for kw in ["BEGIN", "DECLARE"] {
                            let rest: String = chars
                                .get(i..)
                                .unwrap_or_default()
                                .iter()
                                .take(kw.len())
                                .collect::<String>()
                                .to_uppercase();
                            if rest == kw {
                                let after = chars.get(i + kw.len()).copied();
                                let word_end =
                                    !after.is_some_and(|a| a.is_ascii_alphanumeric() || a == '_');
                                if word_end {
                                    return true;
                                }
                            }
                        }
                    }
                }
            },
            St::Str => {
                if c == '\'' {
                    if next == Some('\'') {
                        i += 1; // '' 转义
                    } else {
                        st = St::Code;
                    }
                }
            }
            St::LineCmt => {
                if c == '\n' {
                    st = St::Code;
                }
            }
            St::BlockCmt => {
                if c == '*' && next == Some('/') {
                    st = St::Code;
                    i += 1;
                }
            }
        }
        i += 1;
    }
    false
}

/// 手写分号分割状态机：跳过 '...'、"..."、`...`、--行注释、#行注释（MySQL）、
/// / * 块注释 * / 内的分号；转义（'' 与 \\）正确跳过。
pub fn split_by_semicolon(sql: &str) -> Vec<String> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Code,
        SingleQuoted,
        DoubleQuoted,
        BackQuoted,
        LineComment,
        BlockComment,
    }

    let mut parts = Vec::new();
    let mut current = String::new();
    let mut state = State::Code;
    let bytes: Vec<char> = sql.chars().collect();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = *bytes.get(i).unwrap_or(&'\0');
        let next = bytes.get(i + 1).copied();
        match state {
            State::Code => match c {
                '\'' => {
                    state = State::SingleQuoted;
                    current.push(c);
                }
                '"' => {
                    state = State::DoubleQuoted;
                    current.push(c);
                }
                '`' => {
                    state = State::BackQuoted;
                    current.push(c);
                }
                '-' if next == Some('-') => {
                    state = State::LineComment;
                    current.push(c);
                }
                '#' => {
                    state = State::LineComment;
                    current.push(c);
                }
                '/' if next == Some('*') => {
                    state = State::BlockComment;
                    current.push(c);
                }
                ';' => {
                    if !current.trim().is_empty() {
                        parts.push(current.trim().to_string());
                    }
                    current = String::new();
                }
                _ => current.push(c),
            },
            State::SingleQuoted => {
                // '' 转义与 \' 转义都消费两字符
                if c == '\\' && next.is_some() {
                    current.push(c);
                    if let Some(n) = next {
                        current.push(n);
                    }
                    i += 1;
                } else if c == '\'' && next == Some('\'') {
                    current.push(c);
                    current.push('\'');
                    i += 1;
                } else {
                    if c == '\'' {
                        state = State::Code;
                    }
                    current.push(c);
                }
            }
            State::DoubleQuoted => {
                if c == '\\' && next.is_some() {
                    current.push(c);
                    if let Some(n) = next {
                        current.push(n);
                    }
                    i += 1;
                } else {
                    if c == '"' {
                        state = State::Code;
                    }
                    current.push(c);
                }
            }
            State::BackQuoted => {
                if c == '`' {
                    state = State::Code;
                }
                current.push(c);
            }
            State::LineComment => {
                if c == '\n' {
                    state = State::Code;
                }
                current.push(c);
            }
            State::BlockComment => {
                if c == '*' && next == Some('/') {
                    current.push(c);
                    current.push('/');
                    i += 1;
                    state = State::Code;
                } else {
                    current.push(c);
                }
            }
        }
        i += 1;
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}

/// AST 节点白名单前缀（sqlparser Statement 的 Debug 形如 `Query(Query { .. })`，
/// 取首个词做节点名；用 Debug 前缀而非 matches! 巨型枚举，是为免受 sqlparser
/// 版本间变体增删的漂移影响）。
const READONLY_KIND_PREFIXES: &[&str] = &["Query", "Explain", "Show", "Analyze"];

/// 纯查询型 PRAGMA（无副作用，即使带括号参数也放行）。只收**不可设置**的
/// 内省指令——可设置的（cache_size / schema_version / page_size / user_version /
/// application_id / mmap_size / max_page_count / encoding …）不在此列，它们
/// 落到下面的 `value.is_none()` 判定：无值当查询放行、带值当写拒绝。
const READONLY_PRAGMAS: &[&str] = &[
    "table_info",
    "table_xinfo",
    "table_list",
    "index_info",
    "index_xinfo",
    "index_list",
    "foreign_key_list",
    "database_list",
    "collation_list",
    "function_list",
    "module_list",
    "pragma_list",
    "compile_options",
    "integrity_check",
    "quick_check",
    "freelist_count",
    "data_version",
    "sqlite_version",
];

/// 无条件判写的 PRAGMA：
/// - 加密族：只读会话里没有合法用途，且 `PRAGMA key` 的裸形态语义不明确
/// - 维护动作族：即便无值也会落盘（`optimize` 写 sqlite_stat1、
///   `incremental_vacuum` 释放页、`wal_checkpoint` 回写 WAL）
///
/// 其余双态指令（`foreign_keys` / `journal_mode` / `synchronous` …）交给
/// `value.is_none()` 判定：无值是查询、带值是写。
const WRITE_PRAGMAS: &[&str] = &[
    "key",
    "rekey",
    "cipher",
    "cipher_migrate",
    "cipher_compatibility",
    "cipher_page_size",
    "cipher_kdf_iter",
    "cipher_hmac_algorithm",
    "cipher_kdf_algorithm",
    "cipher_plaintext_header_size",
    "cipher_salt",
    "cipher_default_kdf_iter",
    "optimize",
    "incremental_vacuum",
    "wal_checkpoint",
];

/// 只读白名单判定。parsed=false（降级路）时走前缀白名单（大小写不敏感）。
pub fn statement_is_readonly(engine: &str, stmt: &SplitStatement) -> bool {
    let sql = stmt.sql.trim_start();
    if stmt.parsed {
        match Parser::parse_sql(dialect_for_engine(engine), sql) {
            Ok(statements) => {
                let Some(first) = statements.first() else {
                    return false;
                };
                statement_kind_readonly(first)
            }
            Err(_) => prefix_readonly(sql),
        }
    } else {
        prefix_readonly(sql)
    }
}

fn statement_kind_readonly(stmt: &Statement) -> bool {
    // PRAGMA 双态：无值 = 查询型（只读）；带值 = 设置型（写）。
    // 例外是「内省型」PRAGMA——它们的括号形态（PRAGMA table_info('t')）在
    // sqlparser 里同样落进 value，但语义仍是查询，故按名字白名单放行。
    // 写型 PRAGMA 的括号形态（PRAGMA journal_mode('WAL')）不在白名单内，
    // 仍判为写，不给只读会话留后门。
    if let Statement::Pragma { name, value, .. } = stmt {
        let pragma = pragma_name(name);
        // 内省型：括号形态也是查询，直接放行
        if READONLY_PRAGMAS.contains(&pragma.as_str()) {
            return true;
        }
        // 加密族：无论有无赋值一律拒绝——只读会话里没有合法用途，
        // 且 PRAGMA key 的裸形态语义不明确，保守处理
        if WRITE_PRAGMAS.contains(&pragma.as_str()) {
            return false;
        }
        // 其余双态指令：无值 = 查询（PRAGMA foreign_keys），带值 = 写
        return value.is_none();
    }
    let kind = format!("{stmt:?}");
    let name = kind.split([' ', '(']).next().unwrap_or("");
    READONLY_KIND_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// PRAGMA 名（`PRAGMA [schema.]name` 的末段，小写；ObjectName 是 Vec<Ident>）。
fn pragma_name(name: &sqlparser::ast::ObjectName) -> String {
    name.0
        .last()
        .map(|ident| ident.value.to_lowercase())
        .unwrap_or_default()
}

/// 降级前缀白名单：SELECT / WITH / EXPLAIN / SHOW / DESC / DESCRIBE / TABLE /
/// VALUES（sqlite_viewer 同型语义）。
/// 有意不含 PRAGMA：降级路无法区分查询型（PRAGMA table_info(t)）与设置型
/// （PRAGMA journal_mode=WAL），宁可解析失败时拒绝；SQLite 的 PRAGMA 基本
/// 都能被 SQLiteDialect 解析，走 AST 主路即可。
fn prefix_readonly(sql: &str) -> bool {
    let upper = sql.to_uppercase();
    ["SELECT", "WITH", "EXPLAIN", "SHOW", "DESC", "DESCRIBE", "TABLE", "VALUES"]
        .iter()
        .any(|p| upper.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_multi_statements() {
        let parts = split_by_semicolon("SELECT 1; SELECT 2;;  ");
        assert_eq!(parts, vec!["SELECT 1", "SELECT 2"]);
    }

    #[test]
    fn split_ignores_semicolon_in_strings_and_comments() {
        let sql = "SELECT ';' AS a; -- comment; here\nSELECT `x;y` FROM t; /* block; */ SELECT 3";
        let parts = split_by_semicolon(sql);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts.first().map(String::as_str), Some("SELECT ';' AS a"));
        assert!(parts.get(1).map(|s| s.contains("comment")).unwrap_or(false));
        assert_eq!(parts.get(2).map(String::as_str), Some("/* block; */ SELECT 3"));
    }

    #[test]
    fn split_escaped_quote() {
        let parts = split_by_semicolon("SELECT 'it''s; fine'; SELECT 2");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts.first().map(String::as_str), Some("SELECT 'it''s; fine'"));
    }

    #[test]
    fn parser_route_splits_clean_sql() {
        let parts = split_statements("postgres", "SELECT 1; SELECT 2");
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|p| p.parsed));
    }

    #[test]
    fn parser_route_falls_back_on_dialect_error() {
        // PostgreSqlDialect 不认 MySQL 反引号 → 降级路仍按分号分割
        let parts = split_statements("postgres", "SELECT `a` FROM t; SELECT 2");
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|p| !p.parsed));
    }

    #[test]
    fn readonly_ast_route() {
        let ok = split_statements("postgres", "SELECT 1");
        assert!(ok.iter().all(|s| statement_is_readonly("postgres", s)));
        let with = split_statements("postgres", "WITH t AS (SELECT 1) SELECT * FROM t");
        assert!(with.iter().all(|s| statement_is_readonly("postgres", s)));
        let bad = split_statements("postgres", "DELETE FROM users");
        assert!(!bad.iter().all(|s| statement_is_readonly("postgres", s)));
    }

    #[test]
    fn readonly_prefix_route() {
        let stmt = SplitStatement {
            sql: "update t set x=1".to_string(),
            parsed: false,
        };
        assert!(!statement_is_readonly("mysql", &stmt));
        let show = SplitStatement {
            sql: "show databases".to_string(),
            parsed: false,
        };
        assert!(statement_is_readonly("mysql", &show));
    }

    #[test]
    fn readonly_mysql_show_ast() {
        let show = split_statements("mysql", "SHOW TABLES");
        assert!(show.iter().all(|s| statement_is_readonly("mysql", s)));
        let insert = split_statements("mysql", "INSERT INTO t VALUES (1)");
        assert!(!insert.iter().all(|s| statement_is_readonly("mysql", s)));
        let explain = split_statements("postgres", "EXPLAIN SELECT 1");
        assert!(explain.iter().all(|s| statement_is_readonly("postgres", s)));
    }

    #[test]
    fn oracle_plsql_block_is_single_statement() {
        // 块内分号（逐条 DML、IF/END IF）不是语句边界
        let plsql = "BEGIN\n  UPDATE t SET x = 1;\n  IF SQL%ROWCOUNT > 0 THEN\n    INSERT INTO log VALUES (1);\n  END IF;\n  COMMIT;\nEND;";
        let parts = split_statements("oracle", plsql);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts.first().map(|s| s.sql.as_str()), Some(plsql));
        // PL/SQL 块降级路只读判定为否（前缀白名单不命中 BEGIN）
        assert!(!parts.iter().all(|s| statement_is_readonly("oracle", s)));
    }

    #[test]
    fn oracle_declare_block_and_create_procedure() {
        let declare = "DECLARE\n  v INT := 1;\nBEGIN\n  NULL;\nEND;";
        assert_eq!(split_statements("oracle", declare).len(), 1);
        // 存储过程定义体含 BEGIN：整段单条
        let create = "CREATE OR REPLACE PROCEDURE p IS\nBEGIN\n  NULL;\nEND;";
        assert_eq!(split_statements("oracle", create).len(), 1);
    }

    #[test]
    fn oracle_plain_sql_still_splits() {
        // 不含块的普通多语句照常分割
        let parts = split_statements("oracle", "SELECT 1 FROM dual; SELECT 2 FROM dual");
        assert_eq!(parts.len(), 2);
        // CASE 表达式含 END 但无 BEGIN/DECLARE：不触发整段化
        let case_sql = "SELECT CASE WHEN 1 = 1 THEN 'a' ELSE 'b' END FROM dual; SELECT 2 FROM dual";
        assert_eq!(split_statements("oracle", case_sql).len(), 2);
    }

    #[test]
    fn oracle_block_keyword_literals_not_false_positive() {
        // 字符串/注释里的 BEGIN 字面量不误判
        let sql = "SELECT 'BEGIN transaction' FROM dual; -- DECLARE something\nSELECT 2 FROM dual";
        assert_eq!(split_statements("oracle", sql).len(), 2);
        // 非 Oracle 引擎不受 PL/SQL 整段化影响
        let mysql_sql = "SELECT 'BEGIN' ; SELECT 2";
        assert_eq!(split_statements("mysql", mysql_sql).len(), 2);
    }
}
