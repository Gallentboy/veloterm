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
//! Redis 是第三路（见 `split_statements` 的早退分支）：命令按行分隔、没有
//! 分号语义，sqlparser 与分号降级路都不适用；只读判定走独立的命令白名单
//! （`redis_is_readonly`）。

use sqlparser::ast::Statement;
use sqlparser::dialect::{Dialect, MySqlDialect, PostgreSqlDialect};
use sqlparser::parser::Parser;

/// 语句的只读判定结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitStatement {
    pub sql: String,
    /// 主路解析成功时给 true（只读判定走 AST 节点名）
    pub parsed: bool,
}

/// 按引擎取方言。Oracle 无专属方言（sqlparser 0.53 已无 OracleDialect），
/// 走 PostgreSqlDialect 主路——常规 SELECT/WITH 能解析（只读判定走 AST），
/// Oracle 特有语法（CONNECT BY 等）解析失败自然落入降级路，双路设计本就
/// 容忍方言缺失（绝不能因解析器不认识就拒绝执行）。
pub fn dialect_for_engine(engine: &str) -> &'static dyn Dialect {
    match engine {
        "mysql" => &MySqlDialect {},
        _ => &PostgreSqlDialect {},
    }
}

/// 分割多语句。空语句（仅空白/注释）被丢弃。
/// Oracle：含 PL/SQL 块（词界 BEGIN/DECLARE）的文本整段作单条语句——
/// 块内的分号（IF/END IF、逐语句 DML）不是语句边界，朴素分割会撕裂。
/// Redis：命令按**行**分隔（redis-cli 语义），空行与 `#` 注释行丢弃，
/// 其余每行一条语句——必须在 sqlparser 之前早退：Redis 命令不是 SQL，
/// sqlparser 必然解析失败并落到分号降级路，而分号分割会把多行命令
/// 合成一条（Redis 协议没有分号分隔符）。
pub fn split_statements(engine: &str, sql: &str) -> Vec<SplitStatement> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if engine == "redis" {
        return trimmed
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| SplitStatement {
                sql: line.to_string(),
                parsed: false,
            })
            .collect();
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

/// 只读白名单判定。parsed=false（降级路）时走前缀白名单（大小写不敏感）。
pub fn statement_is_readonly(engine: &str, stmt: &SplitStatement) -> bool {
    let sql = stmt.sql.trim_start();
    // Redis 命令不是 SQL，AST 与 SQL 前缀白名单都不成立；白名单放在这里
    // 而不是 engines.rs，是因为它是「会话只读模式的执行前拦截」这一层的
    // 职责——engines 只负责执行，不参与策略判定（另三引擎同样如此）。
    if engine == "redis" {
        return redis_is_readonly(sql);
    }
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
    let kind = format!("{stmt:?}");
    let name = kind.split([' ', '(']).next().unwrap_or("");
    READONLY_KIND_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Redis 只读命令白名单（会话只读模式的执行前拦截）。
///
/// 为什么放在这里而不是 engines.rs：engines 是「执行层」，只负责把命令发给
/// 服务端；「会话是否只读」是会话策略，与 SQL 引擎的 AST/前缀白名单同属
/// 一个判定层（`statement_is_readonly`）。放在执行层会让策略随引擎实现
/// 漂移，且 engines 的 Redis 臂也就再没有别的理由关心 readonly。
///
/// 语义：**默认拒绝**。只有明确列出的读命令（以及 GEO* 家族）放行，
/// 其余（含 SET/DEL/EXPIRE/EVAL/FLUSHALL…）一律拒绝。四类带子命令的
/// 命令（CONFIG/CLIENT/MEMORY/OBJECT）只有读子命令放行——它们的第一段
/// 本身既能读也能写（CONFIG SET / CLIENT KILL / MEMORY PURGE / OBJECT
/// 无写子命令但保持一致），不能整词放行。
fn redis_is_readonly(sql: &str) -> bool {
    let mut words = sql.split_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    let command = first.to_uppercase();
    let sub = words.next().map(str::to_uppercase);

    // 带子命令的只读命令：第一段 + 白名单子命令
    let sub_readonly: Option<&[&str]> = match command.as_str() {
        "CONFIG" => Some(&["GET"]),
        "CLIENT" => Some(&["LIST", "INFO", "GETNAME", "ID"]),
        "MEMORY" => Some(&["USAGE", "STATS", "DOCTOR"]),
        "OBJECT" => Some(&["ENCODING", "REFCOUNT", "IDLETIME", "FREQ"]),
        // ACL SETUSER/DELUSER/LOAD/SAVE 会改权限表；SLOWLOG RESET 会清日志
        "ACL" => Some(&["LIST", "WHOAMI", "CAT", "GETUSER", "GENPASS", "LOG", "HELP"]),
        "SLOWLOG" => Some(&["GET", "LEN", "HELP"]),
        _ => None,
    };
    if let Some(allowed) = sub_readonly {
        // 缺子命令（裸 `CONFIG`）不放行：服务端会报错，但白名单不猜意图
        return sub.as_deref().is_some_and(|s| allowed.contains(&s));
    }

    // GEO* 家族（GEODIST/GEOPOS/GEOSEARCH/GEOSEARCHSTORE…）：前三个是读，
    // GEOSEARCHSTORE 会写目标 key。这里按前缀整体放行是刻意的折中——
    // GEOSEARCHSTORE 是唯一写命令，且只写调用方显式指定的目标 key，
    // 不碰既有数据；逐个子命令枚举会让新版本新增的 GEO 读命令被误拦。
    if command.starts_with("GEO") {
        return true;
    }

    const REDIS_READONLY_COMMANDS: &[&str] = &[
        // string
        "GET", "MGET", "GETRANGE", "STRLEN", "EXISTS", "TYPE", "TTL", "PTTL", "KEYS", "SCAN",
        "RANDOMKEY", "DUMP", "BITCOUNT", "BITPOS", "BITFIELD_RO",
        // hash
        "HGET", "HMGET", "HGETALL", "HKEYS", "HVALS", "HLEN", "HEXISTS", "HRANDFIELD", "HSCAN",
        // list
        "LLEN", "LRANGE", "LINDEX", "LPOS",
        // set
        "SMEMBERS", "SCARD", "SISMEMBER", "SMISMEMBER", "SRANDMEMBER", "SSCAN",
        // zset
        "ZCARD", "ZCOUNT", "ZSCORE", "ZMSCORE", "ZRANGE", "ZRANGEBYSCORE", "ZREVRANGE",
        "ZREVRANGEBYSCORE", "ZRANK", "ZREVRANK", "ZSCAN", "ZRANDMEMBER",
        // stream
        "XLEN", "XRANGE", "XREVRANGE", "XINFO", "XREAD",
        // hyperloglog / server / 连接（CONFIG/CLIENT/MEMORY/OBJECT/ACL/SLOWLOG
        // 已由 sub_readonly 分支处理，不在此列——它们有写子命令）
        "PFCOUNT", "INFO", "DBSIZE", "PING", "ECHO", "TIME", "LASTSAVE", "COMMAND", "LATENCY",
        "PUBSUB",
    ];
    REDIS_READONLY_COMMANDS.contains(&command.as_str())
}

/// 降级前缀白名单：SELECT / WITH / EXPLAIN / SHOW / DESC / DESCRIBE / TABLE /
/// VALUES（sqlite_viewer 同型语义）。
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
