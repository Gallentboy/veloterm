//! Oracle 引擎专属辅助：错误文案（内置 ORA 码表）、用户清单（按真实
//! 服务器版本适配 + 904 降级）、行取值。
//!
//! OceanBase Oracle 模式的辅助函数（`ob_*`）也在此：它与 Oracle 共用
//! all_users 黑名单口径。

use oracle_async::Queryable;
use crate::db_browser::oracle_messages;

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
