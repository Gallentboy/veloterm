//! 执行计划的结构化解析：各引擎 EXPLAIN 的结果集 → 统一的（id, parent_id）
//! 节点表。前端按节点/边渲染图（React Flow），后端保证树语义。
//!
//! 形态分发按结果集列名特征（列名大小写不敏感）：
//! - Oracle PLAN_TABLE：OPERATION/ID/PARENT_ID/COST/CARDINALITY/BYTES…
//! - SQLite EXPLAIN QUERY PLAN：id/parent/notused/detail
//! - MySQL 经典表：id/select_type/table/type/key/rows/Extra（同 id 行按
//!   管道顺序串成链）
//! - PG / 达梦文本行：单列缩进文本（PG 的 "-> " 前缀与 "(cost=…)" 括号段）
//! - 兜底：每行一个节点平铺
//!
//! 单元格值经各引擎的字符串化通道（NULL 渲染为 "NULL" 或空串），数值列
//! 容忍千分位逗号。

use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// 计划树节点。id 全局唯一（多数引擎直接用行号或 PLAN_TABLE.ID），
/// parent_id = None 即根；节点对即边表。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlanNode {
    pub id: i64,
    pub parent_id: Option<i64>,
    /// 操作名（TABLE ACCESS FULL / Sort / Seq Scan / SIMPLE…）
    pub title: String,
    /// 次要信息（对象名、访问方式、Extra 等）
    pub detail: String,
    pub cost: Option<f64>,
    pub rows: Option<f64>,
    pub bytes: Option<f64>,
}

/// 一条语句的执行计划（多语句选中时逐条各一份）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainPlan {
    /// 被解释的语句原文（截断，前端做 Tab 标签）
    pub statement: String,
    pub nodes: Vec<PlanNode>,
}

/// 单元格取值：列号越界/缺列返回空串
fn cell(row: &[String], col: Option<usize>) -> &str {
    col.and_then(|c| row.get(c).map(String::as_str))
        .unwrap_or("")
}

/// 引擎空值渲染（"NULL"/"(null)"/空串）归一为 false——标题与详情拼接时跳过
fn nz(s: &str) -> bool {
    s.is_empty() || s == "NULL" || s == "(null)"
}

/// 数值列解析：NULL/空/非数返回 None，容忍千分位逗号
fn num(s: &str) -> Option<f64> {
    let t = s.trim().replace(',', "");
    if t.is_empty() || t == "NULL" || t == "(null)" {
        return None;
    }
    t.parse::<f64>().ok()
}

fn num_cell(row: &[String], col: Option<usize>) -> Option<f64> {
    num(cell(row, col))
}

fn int_cell(row: &[String], col: Option<usize>) -> Option<i64> {
    num_cell(row, col).map(|f| f as i64)
}

/// 列名定位（大小写不敏感）
fn col_of(columns: &[String], name: &str) -> Option<usize> {
    columns
        .iter()
        .position(|c| c.trim_end_matches('\u{0}').eq_ignore_ascii_case(name))
}

/// 解析入口：按列名特征分发到具体形态
pub fn parse_plan(columns: &[String], rows: &[Vec<String>], statement: &str) -> ExplainPlan {
    let nodes = if col_of(columns, "OPERATION").is_some() && col_of(columns, "ID").is_some() {
        parse_id_parent(columns, rows)
    } else if col_of(columns, "DETAIL").is_some()
        && col_of(columns, "PARENT").is_some()
        && col_of(columns, "ID").is_some()
    {
        // SQLite EXPLAIN QUERY PLAN 四列：id/parent/notused/detail
        parse_sqlite(columns, rows)
    } else if col_of(columns, "SELECT_TYPE").is_some() && col_of(columns, "ID").is_some() {
        parse_mysql(columns, rows)
    } else if columns.len() == 1 {
        parse_text_lines(rows)
    } else {
        parse_flat(rows)
    };
    ExplainPlan {
        statement: statement.chars().take(120).collect(),
        nodes,
    }
}

/// Oracle PLAN_TABLE 形态：OPERATION[+OPTIONS] / OBJECT_NAME / ID / PARENT_ID /
/// COST / CARDINALITY / BYTES（列名不区分大小写）
fn parse_id_parent(columns: &[String], rows: &[Vec<String>]) -> Vec<PlanNode> {
    let op = col_of(columns, "OPERATION");
    let options = col_of(columns, "OPTIONS");
    let object = col_of(columns, "OBJECT_NAME");
    let id = col_of(columns, "ID");
    let parent = col_of(columns, "PARENT_ID");
    let cost = col_of(columns, "COST");
    let card = col_of(columns, "CARDINALITY");
    let bytes = col_of(columns, "BYTES");
    let mut nodes: Vec<PlanNode> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            // OPERATION 为空时 OPTIONS 顶当标题；两者均空（Oracle 11g+
            // NESTED LOOPS 批读伪步骤，DBMS_XPLAN.DISPLAY 同样滤除）先空置，
            // 后续 splice 剔除
            let op = cell(r, op);
            let opt = cell(r, options);
            let title = if nz(op) {
                if nz(opt) {
                    String::new()
                } else {
                    opt.to_string()
                }
            } else if nz(opt) {
                op.to_string()
            } else {
                format!("{op} {opt}")
            };
            let obj = cell(r, object);
            PlanNode {
                // PLAN_TABLE.ID 唯一；缺列时退化为行号
                id: int_cell(r, id).unwrap_or(i as i64),
                parent_id: int_cell(r, parent),
                title,
                detail: if nz(obj) { String::new() } else { obj.to_string() },
                cost: num_cell(r, cost),
                rows: num_cell(r, card),
                bytes: num_cell(r, bytes),
            }
        })
        .collect();
    splice_empty_steps(&mut nodes);
    nodes
}

/// 剔除空标题伪步骤（Oracle 11g+ NESTED LOOPS 批读的两行形态：第二行
/// OPERATION/OPTIONS 均空；DBMS_XPLAN.DISPLAY 渲染时同样滤除）并把其
/// 子节点上挂到被删节点的父，保持树连通；伪步骤的父也可能仍是伪步骤，
/// 逐层上探（步数上限防环）
fn splice_empty_steps(nodes: &mut Vec<PlanNode>) {
    let empty_ids: HashSet<i64> = nodes
        .iter()
        .filter(|n| n.title.is_empty())
        .map(|n| n.id)
        .collect();
    if empty_ids.is_empty() {
        return;
    }
    let parent_of: HashMap<i64, Option<i64>> =
        nodes.iter().map(|n| (n.id, n.parent_id)).collect();
    let len = nodes.len();
    nodes.retain(|n| !empty_ids.contains(&n.id));
    for n in nodes.iter_mut() {
        if !n.parent_id.is_some_and(|p| empty_ids.contains(&p)) {
            continue;
        }
        let mut parent = n.parent_id;
        let mut steps = 0;
        while let Some(p) = parent {
            if empty_ids.contains(&p) {
                parent = parent_of.get(&p).copied().flatten();
                steps += 1;
                if steps > len {
                    parent = None;
                    break;
                }
            } else {
                break;
            }
        }
        n.parent_id = parent;
    }
}

/// SQLite EXPLAIN QUERY PLAN：结构走 id/parent 列，标题取 detail 文本
fn parse_sqlite(columns: &[String], rows: &[Vec<String>]) -> Vec<PlanNode> {
    let detail = col_of(columns, "DETAIL");
    let id = col_of(columns, "ID");
    let parent = col_of(columns, "PARENT");
    rows.iter()
        .enumerate()
        .map(|(i, r)| PlanNode {
            id: int_cell(r, id).unwrap_or(i as i64),
            parent_id: int_cell(r, parent),
            title: cell(r, detail).to_string(),
            detail: String::new(),
            cost: None,
            rows: None,
            bytes: None,
        })
        .collect()
}

/// MySQL 经典表：同 id 的行是一个查询块的流水线——首行为组根，其余按
/// 出现顺序串成链；节点 id 用行号保证唯一（id 列的值会重复）
fn parse_mysql(columns: &[String], rows: &[Vec<String>]) -> Vec<PlanNode> {
    let id = col_of(columns, "ID");
    let select_type = col_of(columns, "SELECT_TYPE");
    let table = col_of(columns, "TABLE");
    let access = col_of(columns, "TYPE");
    let key = col_of(columns, "KEY");
    let rows_n = col_of(columns, "ROWS");
    let extra = col_of(columns, "EXTRA");
    let mut out = Vec::with_capacity(rows.len());
    // 组内上一行的行号（同 id 链）
    let mut last_in_group: Option<(Option<i64>, i64)> = None;
    for (i, r) in rows.iter().enumerate() {
        let group = int_cell(r, id);
        let parent = match last_in_group {
            Some((g, prev)) if g == group => Some(prev),
            _ => None,
        };
        last_in_group = Some((group, i as i64));
        let st = cell(r, select_type);
        let tbl = cell(r, table);
        let mut title = st.to_string();
        if !tbl.is_empty() && tbl != "NULL" {
            title.push_str(" · ");
            title.push_str(tbl);
        }
        let mut parts: Vec<String> = Vec::new();
        for seg in [cell(r, access), cell(r, key)] {
            if !seg.is_empty() && seg != "NULL" {
                parts.push(seg.to_string());
            }
        }
        if let Some(n) = num_cell(r, rows_n) {
            parts.push(format!("rows={n}"));
        }
        let ex = cell(r, extra);
        if !ex.is_empty() && ex != "NULL" {
            parts.push(ex.to_string());
        }
        out.push(PlanNode {
            id: i as i64,
            parent_id: parent,
            title,
            detail: parts.join(" · "),
            cost: None,
            rows: num_cell(r, rows_n),
            bytes: None,
        });
    }
    out
}

/// PG / 达梦的文本行形态（单列，行内缩进表达层级；PG 的 "-> " 前缀与
/// "(cost=a..b rows=n width=w…)" 括号段解析出数值）。缩进深度按行首空格数，
/// 与上一浅层行比较建树。
/// 达梦原生 EXPLAIN 形态（cmd=149 帧的计划文本，每行形如
/// `4         #CSCN2: [1, 1, 0]; INDEX33555437(T); btr_scan(1)`）——
/// 层级的判定不用空格（列对齐）而用行号前缀 + 操作符列起始位置：
/// 操作符 `#XXX:` 在行内的列位置越小层级越浅，同位置即同层
fn parse_text_lines(rows: &[Vec<String>]) -> Vec<PlanNode> {
    let dm = rows.iter().any(|r| {
        r.first()
            .is_some_and(|l| l.contains('#') && l.contains(": ["))
    });
    if dm {
        return parse_dm_lines(rows);
    }
    let mut stack: Vec<(usize, i64)> = Vec::new();
    let mut out = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let line = r.first().map(String::as_str).unwrap_or("");
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        let text = line.trim_start_matches(' ');
        // PG 的 "-> Op (...)"：箭头只是树形装饰，层级由缩进表达
        let text = text.strip_prefix("-> ").unwrap_or(text).trim();
        let (title, detail, cost, rows_n) = split_pg_paren(text);
        while stack
            .last()
            .is_some_and(|(ind, _)| *ind >= indent)
        {
            stack.pop();
        }
        let parent = stack.last().map(|(_, id)| *id);
        stack.push((indent, i as i64));
        out.push(PlanNode {
            id: i as i64,
            parent_id: parent,
            title,
            detail,
            cost,
            rows: rows_n,
            bytes: None,
        });
    }
    out
}

/// 达梦原生计划行：`<行号>  <缩进>#<操作符>: [cost, rows, bytes]; <参数>`
/// - 行号前缀（首个空格前）丢弃，层级由 `#` 的列位置决定（列对齐树形）
/// - 方括号三元组按达梦语义映射 cost / rows / bytes
/// - 分号后的操作符参数（表名、btr_scan 等）进 detail
fn parse_dm_lines(rows: &[Vec<String>]) -> Vec<PlanNode> {
    let mut stack: Vec<(usize, i64)> = Vec::new();
    let mut out = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let raw = r.first().map(String::as_str).unwrap_or("");
        // 去行号前缀（数字 + 空白）
        let line = raw.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start();
        let Some(hash_pos) = line.find('#') else {
            continue;
        };
        // 层级 = 操作符在行内的列位置（含行号前缀的原始位置）
        let indent = raw.find('#').unwrap_or(hash_pos);
        let rest = &line[hash_pos..];
        let (head, tail) = rest.split_once(';').unwrap_or((rest, ""));
        // head: "#CSCN2: [1, 1, 0]"；操作符名与三元组分开
        let (op_part, tri) = head.split_once(": ").unwrap_or((head, ""));
        let title = op_part.trim_start_matches('#').trim().to_string();
        let nums: Vec<f64> = tri
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .filter_map(|s| num(s))
            .collect();
        while stack.last().is_some_and(|(ind, _)| *ind >= indent) {
            stack.pop();
        }
        let parent = stack.last().map(|(_, id)| *id);
        stack.push((indent, i as i64));
        out.push(PlanNode {
            id: i as i64,
            parent_id: parent,
            title,
            detail: tail.trim().to_string(),
            cost: nums.first().copied(),
            rows: nums.get(1).copied(),
            bytes: nums.get(2).copied(),
        });
    }
    out
}

/// 拆 "Op name  (k=v, …) [(actual …)]"：标题取首个 "(" 前；括号内解析
/// cost=A..B（取上界）、rows=N（多次出现时取最后一次——ANALYZE 的 actual
/// rows 在后），其余键值留在 detail；无括号整行做标题
fn split_pg_paren(text: &str) -> (String, String, Option<f64>, Option<f64>) {
    let Some(open) = text.find('(') else {
        return (text.to_string(), String::new(), None, None);
    };
    if !text.ends_with(')') {
        return (text.to_string(), String::new(), None, None);
    }
    let title = text[..open].trim().to_string();
    // 括号内空格分隔的键值 token（PG 文本形态：cost=a..b rows=n width=w，
    // ANALYZE 另有 actual time=a..b rows=n loops=n）；非键值 token 留 detail
    let inner = &text[open + 1..text.len() - 1];
    let mut cost = None;
    let mut rows = None;
    let mut rest: Vec<&str> = Vec::new();
    let mut has_kv = false;
    for tok in inner.split_whitespace() {
        let Some((k, v)) = tok.split_once('=') else {
            rest.push(tok);
            continue;
        };
        has_kv = true;
        match k {
            "cost" => {
                // "0..8.15" 取区间上界（总代价）
                if let Some((_, hi)) = v.split_once("..") {
                    cost = num(hi);
                }
            }
            // ANALYZE 的 actual rows 在 cost 括号后出现，取最后一次
            "rows" => rows = num(v),
            _ => rest.push(tok),
        }
    }
    // 括号内没有任何键值（如 "Filter: ((a) > (1))"）——不是统计括号，
    // 整行做标题
    if !has_kv {
        return (text.to_string(), String::new(), None, None);
    }
    (title, rest.join(" "), cost, rows)
}

/// 兜底：每行一个平铺节点（首格做标题，其余拼 detail）
fn parse_flat(rows: &[Vec<String>]) -> Vec<PlanNode> {
    rows.iter()
        .enumerate()
        .map(|(i, r)| {
            let title = r
                .first()
                .map(|s| s.chars().take(60).collect::<String>())
                .unwrap_or_default();
            let detail = r
                .iter()
                .skip(1)
                .filter(|s| !s.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" · ");
            PlanNode {
                id: i as i64,
                parent_id: None,
                title,
                detail,
                cost: None,
                rows: None,
                bytes: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Oracle PLAN_TABLE：ID/PARENT_ID 建树 + COST/CARDINALITY 数值
    #[test]
    fn parse_oracle_plan_table() {
        let cols: Vec<String> = ["ID", "PARENT_ID", "OPERATION", "OPTIONS", "OBJECT_NAME",
            "COST", "CARDINALITY", "BYTES"]
            .iter().map(|s| s.to_string()).collect();
        let rows = vec![
            vec!["0", "NULL", "SELECT STATEMENT", "", "NULL", "12", "10", "200"],
            vec!["1", "0", "SORT", "ORDER BY", "", "12", "10", "200"],
            vec!["2", "1", "TABLE ACCESS", "FULL", "T1", "10", "100", "1200"],
            vec!["3", "2", "NULL", "FULL", "T2", "9", "40", "1600"],
            // 11g+ NESTED LOOPS 批读伪步骤（OPERATION/OPTIONS 均空）+ 挂其下的真步骤
            vec!["4", "2", "NULL", "NULL", "", "1", "1", "10"],
            vec!["5", "4", "INDEX", "UNIQUE SCAN", "PK_X", "0", "1", ""],
        ];
        let rows: Vec<Vec<String>> = rows
            .into_iter()
            .map(|r| r.into_iter().map(String::from).collect())
            .collect();
        let plan = parse_plan(&cols, &rows, "SELECT 1");
        // 伪步骤（id=4）被剔除：6 行 → 5 节点
        assert_eq!(plan.nodes.len(), 5);
        let [n0, n1, n2, n3, n5] = plan.nodes.as_slice() else {
            unreachable!()
        };
        assert_eq!(n0.parent_id, None);
        assert_eq!(n1.parent_id, Some(0));
        assert_eq!(n2.parent_id, Some(1));
        assert_eq!(n2.title, "TABLE ACCESS FULL");
        assert_eq!(n2.detail, "T1");
        assert_eq!(n2.cost, Some(10.0));
        assert_eq!(n2.rows, Some(100.0));
        // 空值渲染不进标题与详情：OBJECT_NAME=NULL → detail 空；
        // OPERATION=NULL 时 OPTIONS 顶当标题
        assert_eq!(n0.detail, "");
        assert_eq!(n3.title, "FULL");
        assert_eq!(n3.detail, "T2");
        // 伪步骤剔除后子节点上挂到其父（5 的父从 4 改挂 2）
        assert_eq!(n5.id, 5);
        assert_eq!(n5.parent_id, Some(2));
        assert_eq!(n5.title, "INDEX UNIQUE SCAN");
    }

    /// SQLite EXPLAIN QUERY PLAN：id/parent 结构 + detail 标题
    #[test]
    fn parse_sqlite_plan() {
        let cols: Vec<String> = ["id", "parent", "notused", "detail"]
            .iter().map(|s| s.to_string()).collect();
        let rows: Vec<Vec<String>> = vec![
            vec!["3", "0", "", "SEARCH T USING INTEGER PRIMARY KEY (rowid=?)"],
            vec!["7", "3", "", "USE TEMP B-TREE FOR ORDER BY"],
        ]
        .into_iter()
        .map(|r| r.into_iter().map(String::from).collect())
        .collect();
        let plan = parse_plan(&cols, &rows, "SELECT 1");
        assert_eq!(plan.nodes.len(), 2);
        let [n0, n1] = plan.nodes.as_slice() else {
            unreachable!()
        };
        assert_eq!(n0.parent_id, Some(0)); // parent=0（拓扑根链）
        assert_eq!(n1.parent_id, Some(3));
        assert!(n0.title.contains("SEARCH T"));
    }

    /// MySQL 经典表：同 id 串链、跨 id 断链
    #[test]
    fn parse_mysql_plan() {
        let cols: Vec<String> = ["id", "select_type", "table", "type", "key", "rows", "Extra"]
            .iter().map(|s| s.to_string()).collect();
        let rows: Vec<Vec<String>> = vec![
            vec!["1", "PRIMARY", "t1", "ALL", "NULL", "100", "Using where"],
            vec!["1", "PRIMARY", "t2", "ref", "idx_a", "10", ""],
            vec!["2", "SUBQUERY", "t3", "index", "PRIMARY", "5", "Using index"],
        ]
        .into_iter()
        .map(|r| r.into_iter().map(String::from).collect())
        .collect();
        let plan = parse_plan(&cols, &rows, "SELECT 1");
        assert_eq!(plan.nodes.len(), 3);
        let [n0, n1, n2] = plan.nodes.as_slice() else {
            unreachable!()
        };
        assert_eq!(n0.parent_id, None);
        assert_eq!(n1.parent_id, Some(0)); // 同 id=1 串链
        assert_eq!(n2.parent_id, None); // id=2 新根
        assert_eq!(n1.title, "PRIMARY · t2");
        assert!(n1.detail.contains("idx_a"));
        assert_eq!(n1.rows, Some(10.0));
    }

    /// 达梦原生 EXPLAIN 文本（cmd=149 帧）：行号前缀 + #操作符列对齐建树 +
    /// 方括号三元组 cost/rows/bytes（真机 hex 实证的原始文本）
    #[test]
    fn parse_dm_text_plan() {
        let cols: Vec<String> = vec!["EXPLAIN_PLAN".to_string()];
        let rows: Vec<Vec<String>> = vec![
            "1   #NSET2: [1, 1, 0] ",
            "2     #PRJT2: [1, 1, 0]; exp_num(1), is_atom(FALSE) ",
            "3       #AAGR2: [1, 1, 0]; grp_num(0), sfun_num(1), distinct_flag[0]; slave_empty(0)",
            "4         #CSCN2: [1, 1, 0]; INDEX33555437(##HISTOGRAMS_TABLE); btr_scan(1)",
        ]
        .into_iter()
        .map(|l| vec![l.to_string()])
        .collect();
        let plan = parse_plan(&cols, &rows, "SELECT COUNT(*) FROM T");
        assert_eq!(plan.nodes.len(), 4);
        let [n0, n1, n2, n3] = plan.nodes.as_slice() else {
            unreachable!()
        };
        assert_eq!(n0.title, "NSET2");
        assert_eq!(n0.parent_id, None);
        assert_eq!(n0.cost, Some(1.0));
        assert_eq!(n1.parent_id, Some(0));
        assert_eq!(n2.parent_id, Some(1));
        assert_eq!(n3.parent_id, Some(2));
        assert_eq!(n3.title, "CSCN2");
        assert!(n3.detail.contains("HISTOGRAMS_TABLE"));
    }

    /// PG 文本行：缩进建树 + cost/rows 解析 + "-> " 前缀剥离
    #[test]
    fn parse_pg_text_plan() {
        let cols: Vec<String> = vec!["QUERY PLAN".to_string()];
        let rows: Vec<Vec<String>> = vec![
            "Sort  (cost=8.17..8.19 rows=1 width=8)".to_string(),
            "  ->  Seq Scan on t1  (cost=0.00..8.15 rows=115 width=8)".to_string(),
            "        Filter: ((a) > (1))".to_string(),
        ]
        .into_iter()
        .map(|l| vec![l])
        .collect();
        let plan = parse_plan(&cols, &rows, "SELECT 1");
        assert_eq!(plan.nodes.len(), 3);
        let [n0, n1, n2] = plan.nodes.as_slice() else {
            unreachable!()
        };
        assert_eq!(n0.parent_id, None);
        assert_eq!(n0.title, "Sort");
        assert_eq!(n0.cost, Some(8.19));
        assert_eq!(n1.parent_id, Some(0));
        assert_eq!(n1.title, "Seq Scan on t1");
        assert_eq!(n1.rows, Some(115.0));
        assert_eq!(n2.parent_id, Some(1));
        assert_eq!(n2.title, "Filter: ((a) > (1))");
    }
}
