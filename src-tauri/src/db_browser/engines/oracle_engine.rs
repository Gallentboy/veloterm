//! Oracle 引擎专属辅助：错误文案（内置 ORA 码表）、用户清单（按真实
//! 服务器版本适配 + 904 降级）、行取值。
//!
//! OceanBase Oracle 模式的辅助函数（`ob_*`）也在此：它与 Oracle 共用
//! all_users 黑名单口径。

use oracle_async::Queryable;
use crate::db_browser::oracle_messages;

use super::{ObjectDefinition, TableDetail};

/// 错误码的完整中文解释：优先查 19c 全量译文表（20362 条，含 message/
/// cause/action），未收录时回落 80 条精简摘要表（一句话说明）。
///
/// 两个来源的差别：全量表给的是 Oracle 官方手册的完整三段（原因 + 处理
/// 建议），精简表是人工提炼的一句话高频提示。精简表覆盖面窄但措辞更短，
/// 适合直接拼进单行错误文案。
#[derive(Debug, Clone, Copy)]
pub(crate) struct OracleErrorText {
    /// 错误消息本身（如「表或视图不存在」）
    pub message: &'static str,
    /// 原因说明（全量表才有；精简表恒为空）
    pub cause: &'static str,
    /// 处理建议（全量表才有；精简表恒为空）
    pub action: &'static str,
}

/// 查错误码的中文解释：全量表 → 精简表 → None。
///
/// `code` 为负数或超出 u16 范围时只走精简表（ORA 码本身是正数，负值属于
/// 非 ORA 错误，不该进全量表）。
#[inline]
pub(crate) fn oracle_error_text(code: i32) -> Option<OracleErrorText> {
    if let Ok(small) = u16::try_from(code)
        && let Some(m) = oracle_messages::lookup(small)
    {
        return Some(OracleErrorText {
            message: m.message,
            cause: m.cause,
            action: m.action,
        });
    }
    oracle_ora_hint(code).map(|hint| OracleErrorText {
        message: hint,
        cause: "",
        action: "",
    })
}

/// 把中文解释组装成结构化三行：
///
/// ```text
/// ORA-00942：表或视图不存在
/// 原因：输入的表或视图不存在，使用了此处不允许的同义词……
/// 处理建议：请检查以下各项: - 表名或视图名的拼写。……
/// ```
///
/// 三段齐全时用三行（前端用 `whitespace-pre-wrap` 展示）；精简表只有
/// message 时退化为单行 `ORA-xxxxx：摘要`——没有原因/建议就不留空行。
///
/// 不做长度截断：原因与处理建议是给用户看的诊断信息，截断会丢关键步骤
///（实测中位数 70 字符、最长 1459 字符，完整输出更有用）。
fn format_error_text(code: i32, text: &OracleErrorText, args: &str) -> String {
    let mut out = format!("ORA-{code:05}：{}", text.message);
    if !text.cause.is_empty() {
        out.push_str("\n原因：");
        out.push_str(text.cause);
    }
    if !text.action.is_empty() {
        out.push_str("\n处理建议：");
        out.push_str(text.action);
    }
    if !args.is_empty() {
        out.push_str(args);
    }
    out
}

/// oracle_async::Error → 展示文案。
///
/// OCI / go-ora 的原生行为是把服务器返回的文案原样透传——服务器没装
/// 消息文件（.msb，macOS 与多数精简客户端常见）时就是
/// `ORA-00904: Message 904 not found; No message file for product=RDBMS,
/// facility=ORA`，go-ora 只是 16 个码的兜底翻译、其余仅回 `ORA-xxxxx`。
/// 本函数在透传前做一层清理：识别「缺消息文件」形态，剥掉那段英文尾巴，
/// 用内置中文码表 + arguments 段（如 904 的出错标识符）重组成可读文案。
#[inline]
pub(crate) fn oracle_err(e: oracle_async::Error) -> String {
    match e {
        oracle_async::Error::Server(ref db) => {
            let code = db.code;
            let raw = db.message.as_str();
            let text = oracle_error_text(code);
            // 「缺消息文件」形态：服务器文案本身没有信息量，换成内置中文文案
            if raw.contains("No message file") || raw.contains("not found;") {
                let args = extract_error_arguments(raw);
                return match text {
                    Some(t) => format_error_text(code, &t, &args),
                    None => format!("ORA-{code:05}：详见 Oracle 官方错误文档{args}"),
                };
            }
            match text {
                // 服务器原文可读：原文一行 + 中文解释（避免原文已含该中文时重复）
                Some(t) if !raw.contains(t.message) => {
                    let mut out = format!("ORA-{code:05}: {raw}");
                    out.push_str("\n中文：");
                    out.push_str(t.message);
                    if !t.cause.is_empty() {
                        out.push_str("\n原因：");
                        out.push_str(t.cause);
                    }
                    if !t.action.is_empty() {
                        out.push_str("\n处理建议：");
                        out.push_str(t.action);
                    }
                    out
                }
                _ => {
                    if raw.is_empty() {
                        format!("ORA-{code:05}")
                    } else if raw.starts_with("ORA-") {
                        raw.to_string()
                    } else {
                        format!("ORA-{code:05}: {raw}")
                    }
                }
            }
        }
        other => {
            let msg = other.to_string();
            // 尝试从错误文本中识别 ORA-xxxxx
            if let Some(pos) = msg.find("ORA-") {
                let code_str = &msg[pos + 4..];
                let digits: String = code_str.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Some(text) = digits.parse::<i32>().ok().and_then(oracle_error_text) {
                    let code = digits.parse::<i32>().unwrap_or(0);
                    if msg.contains("No message file") {
                        return format_error_text(code, &text, "");
                    }
                    // 非 Server 变体的文本通常已含服务器原文：原文一行 + 中文解释
                    let mut out = msg;
                    out.push_str("\n中文：");
                    out.push_str(text.message);
                    if !text.cause.is_empty() {
                        out.push_str("\n原因：");
                        out.push_str(text.cause);
                    }
                    if !text.action.is_empty() {
                        out.push_str("\n处理建议：");
                        out.push_str(text.action);
                    }
                    return out;
                }
            }
            msg
        }
    }
}

/// 从 OCI 错误文案里提取 `arguments: ["X"]` 段——缺消息文件时这段仍带
/// 真实定位价值（如 904 的出错标识符、1400 的列名）。
fn extract_error_arguments(raw: &str) -> String {
    let Some(start) = raw.find("arguments: [") else {
        return String::new();
    };
    let rest = &raw[start + "arguments: [".len()..];
    let Some(end) = rest.find(']') else {
        return String::new();
    };
    let args = rest[..end].trim();
    if args.is_empty() {
        String::new()
    } else {
        format!("（相关标识符/参数：{args}）")
    }
}

/// 常见 ORA 错误码内置文案（覆盖连接/认证/SQL 执行/DDL/PLSQL 高频路径）。
/// go-ora 的 translate() 只兜 16 个码、其余仅回 `ORA-xxxxx`；本表更宽，
/// 缺消息文件的服务器（OCI 形态最常见）也能给出可读中文。
/// 未收录的码在 oracle_err 里回落为纯码，不会误导。
#[inline(always)]
pub(crate) fn oracle_ora_hint(code: i32) -> Option<&'static str> {
    Some(match code {
        // 约束 / 数据完整性
        1 => "违反唯一约束",
        2290 => "违反检查约束（值不满足 CHECK 条件）",
        2291 => "违反完整性约束（引用的父记录不存在）",
        2292 => "违反完整性约束（存在引用该行的子记录）",
        1400 => "无法插入 NULL（非空列缺值）",
        1438 => "列值超出精度/小数位上限",
        1722 => "无效数字（字符串转数字失败或溢出）",
        1843 => "日期格式与 NLS_DATE_FORMAT 不匹配",
        1858 => "日期时间字段无效（如 2 月 30 日）",
        12899 => "列值过大（超出字段长度）",
        // SQL 语法 / 语义
        900 => "无效的 SQL 语句",
        904 => "标识符无效（列名/别名拼写错误，或该列在当前版本不存在）",
        905 => "关键字拼写错误",
        906 => "缺少左括号",
        907 => "缺少右括号",
        909 => "缺少 FROM 关键字",
        911 => "无效字符（常见：多余分号或中文标点）",
        917 => "无效的 SQL 语句（常见：列数与值数不匹配）",
        918 => "列名有歧义（多表同名列未限定）",
        920 => "无效的关系运算符",
        923 => "未找到 FROM 关键字（常见：SELECT 列表缺逗号）",
        926 => "缺少 INTO 关键字",
        933 => "SQL 命令未正确结束（常见：末尾多余分号）",
        934 => "绑定变量未声明",
        936 => "缺少表达式",
        937 => "表达式类型不匹配",
        942 => "表或视图不存在（检查 schema 前缀与权限）",
        947 => "SELECT 列表表达式个数不匹配",
        949 => "VALUES 子句列数不匹配",
        956 => "缺少 GROUP BY 表达式",
        979 => "违反唯一约束（另一用户已在同一行上修改）",
        1008 => "未找到该表（可能已被删除或无权访问）",
        1017 => "用户名或密码无效",
        1031 => "权限不足（缺少对应对象的访问权限）",
        1033 => "Oracle 正在启动或关闭中",
        1034 => "Oracle 不可用（实例未启动）",
        1045 => "用户缺少 CREATE SESSION 权限",
        1064 => "会话被终止（KILL SESSION）",
        1092 => "等待资源超时（表被锁）",
        1093 => "会话被其它会话等待（死锁）",
        1403 => "未找到数据（NO_DATA_FOUND）",
        1422 => "精度过高（数值超出声明精度）",
        1476 => "除数等于零",
        1652 => "表空间不足（无法扩展）",
        1654 => "无法扩展表空间",
        1756 => "表空间或数据文件不存在",
        24381 => "数组批量操作中部分行出错",
        28000 => "账户已被锁定",
        28001 => "密码已过期",
        28002 => "密码即将过期",
        28040 => "登录失败（协议或凭据协商失败）",
        28041 => "认证协议内部错误",
        28045 => "要求的认证服务不可用",
        // 会话 / 游标 / 通信
        1001 => "无效的游标（游标已关闭或已取尽）",
        1002 => "取数越界（fetch out of sequence，游标状态已失效）",
        1013 => "用户取消操作",
        3113 => "通信通道文件结束（连接被服务端断开）",
        3114 => "未连接到 Oracle",
        3115 => "通信通道未打开",
        3135 => "连接丢失（connection lost contact）",
        12152 => "Oracle 服务器挂断连接（可能被防火墙/超时切断）",
        12154 => "无法解析连接标识符（检查服务名）",
        12170 => "连接超时",
        12203 => "TNS 无法连接目标",
        12206 => "TNS 未找到可用的监听器",
        12506 => "监听器基于服务 ACL 拒绝了连接",
        12528 => "TNS 监听器拒绝了连接（服务受限）",
        12505 => "监听器不认识该 SID/服务（检查服务名）",
        12514 => "监听器当前不认识请求的服务（检查服务名是否正确）",
        12516 => "监听器无可用处理器（实例进程数已满）",
        12541 => "无监听器（目标端口未监听）",
        12543 => "目标主机不可达",
        12545 => "连接失败（目标主机不存在）",
        12560 => "TNS 协议适配器错误（服务器端问题）",
        12571 => "TNS 包写入器失败（连接中断）",
        12631 => "用户名检索失败（外部认证）",
        // PL/SQL / 过程
        20000 => "自定义应用错误（RAISE_APPLICATION_ERROR）",
        6502 => "PL/SQL 数字或值错误（溢出/精度）",
        6508 => "PL/SQL 遇到未处理的异常",
        6510 => "PL/SQL 编译错误（检查过程和函数体）",
        6511 => "PL/SQL 编译单元分析错误",
        6531 => "PL/SQL 参数个数或类型不匹配",
        6533 => "PL/SQL 编译错误（符号未声明）",
        6550 => "PL/SQL 内部错误",
        6553 => "PL/SQL 无法在只读事务中执行 DML",
        20001 => "自定义应用错误（RAISE_APPLICATION_ERROR 自定义码）",
        _ => return None,
    })
}

/// 用户（schema）清单，按服务器版本适配 SQL：ORACLE_MAINTAINED 列是
/// 12.1 才加的（11.2 及更早查询直接 ORA-00904），故 <12 走内置账号黑名单
/// 加 APEX_/FLOWS_ 前缀排除。主版本取自 0x3B 版本查询（db_version_major，
/// banner 尾巴不是数字不可用）；版本未知或 12c+ 查询仍报 904 时降级黑名单。
pub(crate) async fn oracle_usernames(conn: &mut oracle_async::Conn) -> Result<Vec<String>, String> {
    let mut result = query_usernames(conn, conn.db_version_major()).await;
    if result.is_err() {
        result = query_usernames(conn, 0).await;
    }
    result
}

async fn query_usernames(
    conn: &mut oracle_async::Conn,
    major: u32,
) -> Result<Vec<String>, String> {
    let sql = if major >= 12 {
        "SELECT username FROM all_users WHERE oracle_maintained = 'N' ORDER BY username"
    } else {
        "SELECT username FROM all_users WHERE username NOT IN \
         ('SYS','SYSTEM','OUTLN','DIP','ORACLE_OCM','DBSNMP','APPQOSSYS','WMSYS',\
          'EXFSYS','CTXSYS','XDB','ANONYMOUS','XS$NULL','MDDATA','MDSYS','OLAPSYS',\
          'ORDDATA','ORDPLUGINS','ORDSYS','SI_INFORMTN_SCHEMA','LBACSYS','OWBSYS',\
          'OWBSYS_AUDIT','SPATIAL_CSW_ADMIN','SPATIAL_WFS_ADMIN','SYSMAN',\
          'MGMT_VIEW','FLOWS_FILES') \
         AND username NOT LIKE 'APEX\\_%' ESCAPE '\\' \
         AND username NOT LIKE 'FLOWS\\_%' ESCAPE '\\' \
         ORDER BY username"
    };

    let mut rs = conn
        .query(sql)
        .await
        .map_err(|e| format!("Oracle 查询用户清单失败：{}", oracle_err(e)))?;

    let mut usernames = Vec::new();
    while let Some(row) = rs.next_row().await.map_err(|e| format!("Oracle 读取用户清单失败：{}", oracle_err(e)))? {
        usernames.push(oracle_row_string(&row, 0));
    }
    Ok(usernames)
}

/// SQL 字符串字面量单引号翻倍（目录 SQL 的过滤值内联拼接——Oracle 单引号转义规则）。
#[inline(always)]
pub(crate) fn ob_quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Row 按列取字符串（OracleValue Display 内置 NULL 处理与各类型形态；越界列回 NULL）
#[inline(always)]
pub(crate) fn oracle_row_string(row: &oracle_async::Row, i: usize) -> String {
    row.values()
        .get(i)
        .map(|v| v.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

// ── OceanBase Oracle 租户：表详情与对象定义 ────────────────────────────────
//
// 口径对齐 ODC `OBOracleSchemaAccessor` / `OracleSchemaAccessor`：
// - 全部走 `ALL_*` 数据字典（OB 没有独立的 DBA_ 权限体系，恒 all_*）
// - 表 DDL 优先 OB 扩展的 `SHOW CREATE TABLE`（输出含 OB 专有的
//   partition/tablegroup 等子句），失败降级 `DBMS_METADATA.GET_DDL`
// - 列过滤 `USER_GENERATED='YES'`（排除系统生成列与隐藏列），列上老版本
//   没有该列时降级为不过滤
//
// 与原生 Oracle 臂（`DbEngine::Oracle`）的 SQL 同源，差别只有两处：字典
// 前缀固定 `all`，以及执行走 mysql_async 的文本协议（OB Oracle 租户经
// MySQL 线缆访问）。

/// OB Oracle 标识符加双引号（Oracle 引用标识符规则，内部 `"` 翻倍）。
#[inline]
fn ob_ident(v: &str) -> String {
    format!("\"{}\"", v.replace('"', "\"\""))
}

/// OB Oracle 表/视图详情（列/索引/约束/外键/注释/行数）。
///
/// `kind` 为 `"view"` 时 DDL 取 VIEW 定义（`SHOW CREATE TABLE` 对视图返回
/// 建视图语句），其余按表处理。
pub(crate) async fn ob_oracle_table_detail(
    conn: &mut mysql_async::Conn,
    schema: &str,
    table: &str,
    kind: &str,
) -> Result<TableDetail, String> {
    use mysql_async::prelude::Queryable as _;

    let lit_s = ob_quote_literal(schema);
    let lit_t = ob_quote_literal(table);
    let is_view = kind == "view";

    // 1) DDL：SHOW CREATE TABLE（OB 扩展）→ DBMS_METADATA → 空串
    let mut ddl = String::new();
    let show_sql = format!("SHOW CREATE TABLE {}.{}", ob_ident(schema), ob_ident(table));
    if let Ok(rows) = conn.query::<(String, String), _>(show_sql).await
        && let Some((_, d)) = rows.into_iter().next()
    {
        ddl = d;
    }
    if ddl.trim().is_empty() {
        let object_type = if is_view { "VIEW" } else { "TABLE" };
        let meta_sql = format!(
            "SELECT DBMS_METADATA.GET_DDL({}, {}, {}) FROM dual",
            ob_quote_literal(object_type),
            lit_t,
            lit_s
        );
        if let Ok(rows) = conn.query::<Option<String>, _>(meta_sql).await
            && let Some(Some(d)) = rows.into_iter().next()
        {
            ddl = d.trim().to_string();
        }
    }

    // 2) 列：all_tab_columns + all_col_comments（表与视图同表），
    //    USER_GENERATED='YES' 排除系统生成列；老版本没有该列时降级
    let cols_base = format!(
        "SELECT c.column_name, c.data_type, c.nullable, cc.comments \
         FROM all_tab_columns c \
         LEFT JOIN all_col_comments cc ON cc.owner = c.owner \
           AND cc.table_name = c.table_name AND cc.column_name = c.column_name \
         WHERE c.owner = {lit_s} AND c.table_name = {lit_t}"
    );
    type ColRow = (String, String, Option<String>, Option<String>);
    let col_rows: Vec<ColRow> =
        match conn.query(format!("{cols_base} AND c.USER_GENERATED = 'YES' ORDER BY c.column_id")).await {
            Ok(rows) => rows,
            Err(_) => conn
                .query(format!("{cols_base} ORDER BY c.column_id"))
                .await
                .map_err(|e| format!("OceanBase 查询列结构失败：{e}"))?,
        };
    let columns: Vec<Vec<String>> = col_rows
        .iter()
        .map(|(name, ty, nullable, comment)| {
            vec![
                name.clone(),
                ty.clone(),
                match nullable.as_deref() {
                    Some("Y") | Some("y") => "YES".to_string(),
                    Some("N") | Some("n") => "NO".to_string(),
                    _ => String::new(),
                },
                comment.clone().unwrap_or_default(),
            ]
        })
        .collect();

    // 3) 索引：all_indexes + all_ind_columns，LISTAGG 按列位置聚列
    let idx_sql = format!(
        "SELECT i.index_name, i.uniqueness, \
                LISTAGG(c.column_name, ', ') WITHIN GROUP (ORDER BY c.column_position) \
         FROM all_indexes i \
         JOIN all_ind_columns c ON c.index_name = i.index_name AND c.index_owner = i.owner \
         WHERE i.table_owner = {lit_s} AND i.table_name = {lit_t} \
         GROUP BY i.index_name, i.uniqueness ORDER BY i.index_name"
    );
    let index_rows: Vec<(String, Option<String>, Option<String>)> =
        conn.query(idx_sql).await.unwrap_or_default();
    let indexes: Vec<Vec<String>> = index_rows
        .into_iter()
        .map(|(name, uniq, cols)| {
            vec![
                name,
                if uniq.as_deref() == Some("UNIQUE") {
                    "UNIQUE".to_string()
                } else {
                    String::new()
                },
                cols.unwrap_or_default(),
            ]
        })
        .collect();

    // 4) 约束：all_constraints + all_cons_columns，按约束名聚列
    let cons_sql = format!(
        "SELECT c.constraint_name, c.constraint_type, c.r_owner, c.r_constraint_name, \
                c.delete_rule, \
                LISTAGG(cc.column_name, ',') WITHIN GROUP (ORDER BY cc.position) \
         FROM all_constraints c \
         LEFT JOIN all_cons_columns cc ON cc.owner = c.owner \
           AND cc.constraint_name = c.constraint_name \
         WHERE c.owner = {lit_s} AND c.table_name = {lit_t} \
         GROUP BY c.constraint_name, c.constraint_type, c.r_owner, c.r_constraint_name, \
                  c.delete_rule \
         ORDER BY c.constraint_name"
    );
    type ConsRow = (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let cons_rows: Vec<ConsRow> = conn.query(cons_sql).await.unwrap_or_default();
    let mut constraints: Vec<Vec<String>> = Vec::new();
    // F 约束的（约束名, r_owner.r_constraint_name, delete_rule）待反查目标表
    let mut fk_refs: Vec<(String, String, String)> = Vec::new();
    for (name, ctype, r_owner, r_cons, del_rule, cols) in cons_rows {
        let short = match ctype.as_deref() {
            Some("P") => "PK",
            Some("U") => "U",
            Some("R") => "F",
            _ => "C",
        };
        if short == "F" {
            fk_refs.push((
                name.clone(),
                format!(
                    "{}.{}",
                    r_owner.as_deref().unwrap_or_default(),
                    r_cons.as_deref().unwrap_or_default()
                ),
                del_rule.unwrap_or_default(),
            ));
        }
        constraints.push(vec![
            name,
            short.to_string(),
            cols.unwrap_or_default(),
            String::new(),
        ]);
    }
    // 外键目标表名 + 目标列（r_constraint_name → 目标约束），DELETE_RULE 非
    // NO ACTION 时拼 "table(cols) ON DELETE CASCADE"
    for (fk_name, r_owner_name, del_rule) in &fk_refs {
        let parts: Vec<&str> = r_owner_name.splitn(2, '.').collect();
        let [owner_part, cons_part] = parts[..] else {
            continue;
        };
        let ref_sql = format!(
            "SELECT c.table_name, \
                    LISTAGG(cc.column_name, ',') WITHIN GROUP (ORDER BY cc.position) \
             FROM all_constraints c \
             JOIN all_cons_columns cc ON cc.owner = c.owner \
               AND cc.constraint_name = c.constraint_name \
             WHERE c.owner = {} AND c.constraint_name = {} \
             GROUP BY c.table_name",
            ob_quote_literal(owner_part),
            ob_quote_literal(cons_part)
        );
        let Ok(rows) = conn
            .query::<(String, Option<String>), _>(ref_sql)
            .await
        else {
            continue;
        };
        let Some((target, target_cols)) = rows.into_iter().next() else {
            continue;
        };
        if let Some(cell) = constraints
            .iter_mut()
            .find(|c| c.first().map(String::as_str) == Some(fk_name.as_str()))
            && let Some(r) = cell.get_mut(3)
        {
            *r = match target_cols {
                Some(cols) if !cols.is_empty() => format!("{target}({cols})"),
                _ => target,
            };
            let rule = del_rule.trim();
            if !rule.is_empty() && rule != "NO ACTION" {
                r.push_str(" ON DELETE ");
                r.push_str(rule);
            }
        }
    }
    // 5) 反向外键：谁引用了本表（本表 PK/UK 被 r_constraint_name 指向）
    let inc_sql = format!(
        "SELECT c.constraint_name, c.owner || '.' || c.table_name, \
                (SELECT LISTAGG(cc.column_name, ',') WITHIN GROUP (ORDER BY cc.position) \
                   FROM all_cons_columns cc \
                  WHERE cc.owner = c.owner AND cc.constraint_name = c.constraint_name), \
                (SELECT LISTAGG(cc.column_name, ',') WITHIN GROUP (ORDER BY cc.position) \
                   FROM all_cons_columns cc \
                  WHERE cc.owner = c.r_owner AND cc.constraint_name = c.r_constraint_name) \
         FROM all_constraints c \
         WHERE c.constraint_type = 'R' AND c.r_owner = {lit_s} \
           AND c.r_constraint_name IN (SELECT rc.constraint_name FROM all_constraints rc \
                 WHERE rc.owner = {lit_s} AND rc.table_name = {lit_t} \
                   AND rc.constraint_type IN ('P','U')) \
         ORDER BY c.owner, c.table_name, c.constraint_name"
    );
    let incoming: Vec<Vec<String>> = conn
        .query::<(String, String, Option<String>, Option<String>), _>(inc_sql)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(n, t, c, rc)| {
            vec![
                n,
                t,
                c.unwrap_or_default(),
                rc.unwrap_or_default(),
            ]
        })
        .collect();

    // 6) 注释 + 行数（NUM_ROWS 是统计近似，与 ODC 同口径）
    let stat_sql = format!(
        "SELECT (SELECT comments FROM all_tab_comments \
                  WHERE owner = {lit_s} AND table_name = {lit_t}), \
                (SELECT num_rows FROM all_tables \
                  WHERE owner = {lit_s} AND table_name = {lit_t}) \
         FROM dual"
    );
    let (comment, row_count) = match conn
        .query::<(Option<String>, Option<String>), _>(stat_sql)
        .await
    {
        Ok(rows) => rows
            .into_iter()
            .next()
            .map(|(c, n)| (c.unwrap_or_default().trim().to_string(), n.unwrap_or_default()))
            .unwrap_or_default(),
        Err(_) => (String::new(), String::new()),
    };

    Ok(TableDetail {
        ddl,
        comment,
        // 表大小需 USER_SEGMENTS/DBA_SEGMENTS 统计，OB 下不保证可读——
        // 原生 Oracle 臂同样留空（两处口径一致）
        size: String::new(),
        row_count,
        column_headers: vec![
            "Column".into(),
            "Type".into(),
            "Nullable".into(),
            "Comment".into(),
        ],
        columns,
        index_headers: vec!["Index".into(), "Unique".into(), "Columns".into()],
        indexes,
        constraint_headers: vec![
            "Constraint".into(),
            "Type".into(),
            "Columns".into(),
            "References".into(),
        ],
        constraints,
        reference_headers: vec![
            "Constraint".into(),
            "Table".into(),
            "Columns".into(),
            "References".into(),
        ],
        references: incoming,
    })
}

/// OB Oracle 对象定义（DDL/源码）。
///
/// 与原生 Oracle 臂同源：过程/函数/触发器/包走 `all_source` 按行拼回
/// （比 `DBMS_METADATA` 少一层权限依赖），表/视图/序列/同义词/类型走
/// `SHOW CREATE TABLE` 或 `DBMS_METADATA.GET_DDL`。
pub(crate) async fn ob_oracle_object_definition(
    conn: &mut mysql_async::Conn,
    schema: &str,
    name: &str,
    kind: &str,
) -> Result<ObjectDefinition, String> {
    use mysql_async::prelude::Queryable as _;

    let lit_s = ob_quote_literal(schema);
    let lit_n = ob_quote_literal(name);

    // 表：OB 扩展的 SHOW CREATE TABLE（含 OB 专有子句），失败降级 GET_DDL
    if kind == "table" {
        let show_sql = format!(
            "SHOW CREATE TABLE {}.{}",
            ob_ident(schema),
            ob_ident(name)
        );
        if let Ok(rows) = conn.query::<(String, String), _>(show_sql).await
            && let Some((_, ddl)) = rows.into_iter().next()
            && !ddl.trim().is_empty()
        {
            return Ok(ObjectDefinition {
                definition: ddl,
                call_template: format!("SELECT * FROM {};", ob_ident(name)),
                arg_count: 0,
            });
        }
        return ob_oracle_meta_ddl(conn, "TABLE", schema, name, String::new()).await;
    }

    // 视图 / 物化视图 / 序列 / 同义词 / 包 / 类型：DBMS_METADATA.GET_DDL
    let meta_type = match kind {
        "view" => Some("VIEW"),
        "mview" => Some("MATERIALIZED_VIEW"),
        "sequence" => Some("SEQUENCE"),
        "synonym" => Some("SYNONYM"),
        "package" => Some("PACKAGE"),
        "datatype" => Some("TYPE"),
        _ => None,
    };
    if let Some(meta_type) = meta_type {
        let call_template = if kind == "sequence" {
            format!("SELECT {schema}.{name}.NEXTVAL FROM dual;")
        } else {
            String::new()
        };
        let meta = ob_oracle_meta_ddl(conn, meta_type, schema, name, call_template.clone()).await;
        if meta.is_ok() {
            return meta;
        }
        // 视图降级 all_views 文本；包降级 all_source（GET_DDL 对包可能不含
        // 包体，老版本也可能没有 MATERIALIZED_VIEW 的 GET_DDL 支持）
        if kind == "view" {
            let vs = format!(
                "SELECT text FROM all_views WHERE owner = {lit_s} AND view_name = {lit_n}"
            );
            if let Ok(rows) = conn.query::<Option<String>, _>(vs).await
                && let Some(Some(text)) = rows.into_iter().next()
                && !text.trim().is_empty()
            {
                return Ok(ObjectDefinition {
                    definition: text,
                    call_template: String::new(),
                    arg_count: 0,
                });
            }
        }
        if kind == "package" {
            let definition = ob_oracle_source_text(conn, schema, name, "PACKAGE").await?;
            return Ok(ObjectDefinition {
                definition,
                call_template: String::new(),
                arg_count: 0,
            });
        }
        return meta;
    }

    // 触发器：all_source TYPE='TRIGGER'
    if kind == "trigger" {
        let definition = ob_oracle_source_text(conn, schema, name, "TRIGGER").await?;
        return Ok(ObjectDefinition {
            definition,
            call_template: String::new(),
            arg_count: 0,
        });
    }

    // 过程 / 函数：all_source + all_arguments 数入参个数
    if kind != "function" && kind != "procedure" {
        return Err(format!("OceanBase 不支持的取定义对象类型：{kind}"));
    }
    let obj_type = if kind == "function" { "FUNCTION" } else { "PROCEDURE" };
    let definition = ob_oracle_source_text(conn, schema, name, obj_type).await?;
    let cnt_sql = format!(
        "SELECT COUNT(*) FROM all_arguments \
         WHERE owner = {lit_s} AND object_name = {lit_n} AND argument_name IS NOT NULL"
    );
    let arg_count: usize = conn
        .query::<Option<i64>, _>(cnt_sql)
        .await
        .ok()
        .and_then(|rows| rows.into_iter().next().flatten())
        .unwrap_or(0)
        .max(0) as usize;
    let call_template = if kind == "function" {
        format!("SELECT {schema}.{name}() FROM dual;")
    } else {
        format!("CALL {schema}.{name}();")
    };
    Ok(ObjectDefinition {
        definition,
        call_template,
        arg_count,
    })
}

/// `DBMS_METADATA.GET_DDL('<type>', name, schema)` 取定义（统一错误文案）。
async fn ob_oracle_meta_ddl(
    conn: &mut mysql_async::Conn,
    object_type: &str,
    schema: &str,
    name: &str,
    call_template: String,
) -> Result<ObjectDefinition, String> {
    use mysql_async::prelude::Queryable as _;
    let sql = format!(
        "SELECT DBMS_METADATA.GET_DDL({}, {}, {}) FROM dual",
        ob_quote_literal(object_type),
        ob_quote_literal(name),
        ob_quote_literal(schema)
    );
    let ddl = conn
        .query::<Option<String>, _>(sql)
        .await
        .map_err(|e| format!("OceanBase 拉取定义失败：{e}"))?
        .into_iter()
        .next()
        .flatten()
        .unwrap_or_default();
    if ddl.trim().is_empty() {
        return Err("未找到该对象（可能已被删除）".to_string());
    }
    Ok(ObjectDefinition {
        definition: ddl,
        call_template,
        arg_count: 0,
    })
}

/// `all_source` 按行拼回完整源码（过程/函数/触发器/包的包体与声明都在其中）。
async fn ob_oracle_source_text(
    conn: &mut mysql_async::Conn,
    schema: &str,
    name: &str,
    object_type: &str,
) -> Result<String, String> {
    use mysql_async::prelude::Queryable as _;
    let sql = format!(
        "SELECT text FROM all_source WHERE owner = {} AND name = {} AND type = {} \
         ORDER BY line",
        ob_quote_literal(schema),
        ob_quote_literal(name),
        ob_quote_literal(object_type)
    );
    let lines: Vec<Option<String>> = conn
        .query(sql)
        .await
        .map_err(|e| format!("OceanBase 拉取源码失败：{e}"))?;
    let definition: String = lines.into_iter().flatten().collect();
    if definition.trim().is_empty() {
        return Err("未找到该对象（可能已被删除）".to_string());
    }
    Ok(definition)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// OB Oracle 标识符引用：双引号包裹、内部双引号翻倍（Oracle 引用标识符
    /// 规则，与 ODC `StringUtils.quoteOracleIdentifier` 同源）
    #[test]
    fn ob_ident_double_quotes_and_escapes() {
        assert_eq!(ob_ident("T1"), "\"T1\"");
        assert_eq!(ob_ident("lower_case"), "\"lower_case\"");
        assert_eq!(ob_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(ob_ident(""), "\"\"");
    }
}
