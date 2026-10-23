//! Oracle 引擎专属辅助：客户端库目录、错误文案（含缺 .msb 消息文件的兜底）、
//! 用户清单（按服务器版本适配）、行取值与流式发射。
//!
//! 从 engines.rs 拆出（纯移动，零行为变化）。OceanBase Oracle 模式的
//! 辅助函数（`ob_*`）也在此：它与 Oracle 共用 all_users 黑名单口径。

use crate::db_browser::obclient;

use super::{ExecSummary, ROWS_PER_BATCH};

/// 读 Oracle 客户端库目录设置（app_kv；读失败按未设置处理，不阻塞连接）
pub(crate) fn oracle_client_dir() -> Result<Option<String>, String> {
    let Some(pool) = crate::db::global_pool() else {
        return Ok(None);
    };
    let conn = pool.get().map_err(|e| e.to_string())?;
    let dir: Option<String> = conn
        .query_row(
            "SELECT value FROM app_kv WHERE key = 'oracle_client_lib_dir'",
            [],
            |row| row.get(0),
        )
        .ok();
    Ok(dir.filter(|s| !s.is_empty()))
}

/// oracle::Error → 展示文案。DPI-1047 = Oracle 客户端库缺失（运行期依赖
/// Instant Client），追加安装提示——这是 rust-oracle 形态下最常见的环境错。
/// 缺消息文件形态（macOS 包现实，见 oracle_missing_message_hint）先兜底。
#[inline]
pub(crate) fn oracle_err(e: oracle::Error) -> String {
    if let Some(mapped) = oracle_missing_message_hint(&e) {
        return mapped;
    }
    let msg = e.to_string();
    if msg.contains("DPI-1047") {
        format!(
            "{msg}（未找到 Oracle 客户端库：请安装 Oracle Instant Client 并在\
             「数据库」页设置客户端目录；也可依赖系统搜索路径）"
        )
    } else {
        msg
    }
}

/// macOS Instant Client 不随包提供 ORA 消息文件（.msb，包内只有 dylib；
/// Linux basic 包才有 oraus.msb），OCI 本地查文案失败时给出
/// "Message NNN not found; No message file ..."。错误码本体在 DbError
/// 里始终可得，用内置常见码表 + 官方文档链接兜底成可读信息。
fn oracle_missing_message_hint(e: &oracle::Error) -> Option<String> {
    let db = e.db_error()?;
    if !db.message().contains("No message file") {
        return None;
    }
    let code = db.code();
    let hint = oracle_ora_hint(code)
        .map(|h| format!("：{h}"))
        .unwrap_or_default();
    // "arguments: [...]" 段有真实定位价值（如 904 的出错标识符），保留
    let args = db
        .message()
        .find("arguments: [")
        .and_then(|i| db.message().get(i + "arguments: [".len()..))
        .and_then(|s| s.strip_suffix(']'))
        .filter(|s| !s.is_empty())
        .map(|s| format!("，参数 {s}"))
        .unwrap_or_default();
    Some(format!(
        "ORA-{code:05}{hint}{args}（客户端未随包提供错误文案文件，详见 \
         https://docs.oracle.com/error-help/db/ora-{code:05}/）"
    ))
}

/// 常见 ORA 错误码内置文案（仅覆盖本应用高频路径：连接/认证/SQL 执行）。
/// 未收录的码在 oracle_err 里回落为纯码 + 官方文档链接，不会误导。
#[inline(always)]
pub(crate) fn oracle_ora_hint(code: i32) -> Option<&'static str> {
    Some(match code {
        1 => "违反唯一约束",
        1013 => "用户取消操作",
        1017 => "用户名或密码无效",
        1033 => "Oracle 正在启动或关闭中",
        1034 => "Oracle 不可用（实例未启动）",
        1045 => "用户缺少 CREATE SESSION 权限",
        3113 => "通信通道文件结束（连接被服务端断开）",
        3114 => "未连接到 Oracle",
        904 => "标识符无效",
        911 => "无效字符（常见：多余分号或中文标点）",
        918 => "列名有歧义（多表同名列未限定）",
        920 => "无效的关系运算符",
        923 => "未找到 FROM 关键字（常见：SELECT 列表缺逗号）",
        933 => "SQL 命令未正确结束（常见：末尾多余分号）",
        936 => "缺少表达式",
        942 => "表或视图不存在",
        1722 => "无效数字",
        1400 => "无法插入 NULL（非空列缺值）",
        1403 => "未找到数据",
        2292 => "违反完整性约束（存在引用该行的子记录）",
        12154 => "无法解析连接标识符（检查服务名）",
        12170 => "连接超时",
        12505 => "监听器不认识该 SID/服务（检查服务名）",
        12514 => "监听器当前不认识请求的服务（检查服务名是否正确）",
        12541 => "无监听器（目标端口未监听）",
        12543 => "目标主机不可达",
        12545 => "连接失败（目标主机不存在）",
        12899 => "列值过大（超出字段长度）",
        28000 => "账户已被锁定",
        28001 => "密码已过期",
        28002 => "密码即将过期",
        _ => return None,
    })
}

/// 用户（schema）清单，按服务器版本适配 SQL：ORACLE_MAINTAINED 列是
/// 12.1 才加的（11.2 及更早查询直接 ORA-00904），故 <12 走内置账号黑名单
/// 加 APEX_/FLOWS_ 前缀排除。server_version 走 ODPI-C 连接内缓存
/// （connect 时已取过，此处零额外往返）。
pub(crate) fn oracle_usernames(conn: &oracle::Connection) -> Result<Vec<String>, String> {
    let (version, _) = conn
        .server_version()
        .map_err(|e| format!("Oracle 获取服务器版本失败：{}", oracle_err(e)))?;
    let sql = if version.major() >= 12 {
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
    let rs = conn
        .query(sql, &[])
        .map_err(|e| format!("Oracle 查询用户清单失败：{}", oracle_err(e)))?;
    Ok(rs
        .filter_map(|r| r.ok())
        .map(|row| oracle_row_string(&row, 0))
        .collect())
}

/// OB Oracle 模式的用户（schema）清单：all_users + 系统账号黑名单。黑名单
/// 与 oracle_usernames 的 <12 分支同源照抄（OB 的 all_users 无 12c 的
/// oracle_maintained 列，统一走黑名单；OB Oracle 租户自带的 SYS 等系统
/// 账号同被滤除）。
pub(crate) fn ob_oracle_usernames(conn: &mut obclient::ObConn) -> Result<Vec<String>, String> {
    const SQL: &str = "SELECT username FROM all_users WHERE username NOT IN \
         ('SYS','SYSTEM','OUTLN','DIP','ORACLE_OCM','DBSNMP','APPQOSSYS','WMSYS',\
          'EXFSYS','CTXSYS','XDB','ANONYMOUS','XS$NULL','MDDATA','MDSYS','OLAPSYS',\
          'ORDDATA','ORDPLUGINS','ORDSYS','SI_INFORMTN_SCHEMA','LBACSYS','OWBSYS',\
          'OWBSYS_AUDIT','SPATIAL_CSW_ADMIN','SPATIAL_WFS_ADMIN','SYSMAN',\
          'MGMT_VIEW','FLOWS_FILES') \
         AND username NOT LIKE 'APEX\\_%' ESCAPE '\\' \
         AND username NOT LIKE 'FLOWS\\_%' ESCAPE '\\' \
         ORDER BY username";
    Ok(conn
        .query_rows(SQL)
        .map_err(|e| format!("OceanBase 查询用户清单失败：{e}"))?
        .into_iter()
        // 单列查询：缺列行（网络截断半态）跳过不致命
        .filter_map(|row| row.first().map(ob_cell_string))
        .collect())
}

/// SQL 字符串字面量单引号翻倍（obclient 臂无绑定参数，目录 SQL 的过滤值
/// 内联拼接——Oracle 单引号转义规则）。
#[inline(always)]
pub(crate) fn ob_quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// OB 单元格 → 展示字符串（None = NULL 列；字节流 lossy 转串，与
/// emit 流式臂的展示语义一致）。
#[inline(always)]
pub(crate) fn ob_cell_string(cell: &Option<Vec<u8>>) -> String {
    match cell {
        None => "NULL".to_string(),
        Some(b) => String::from_utf8_lossy(b).into_owned(),
    }
}

/// Row 按列取字符串（SqlValue 的 Display 内置 NULL 处理与各类型形态；越界列回 NULL）
#[inline(always)]
pub(crate) fn oracle_row_string(row: &oracle::Row, i: usize) -> String {
    row.sql_values()
        .get(i)
        .map(|v| v.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

/// 逐行流式发射一个 Oracle 结果集：批量回调（ROWS_PER_BATCH 粒度）、
/// max_rows 截断（truncated 探测一行）。columns 未初始化时从结果集补齐
/// 并回调（PL/SQL 隐式结果集路径复用）
pub(crate) fn emit_oracle_rows<F1, F2>(
    rs: &mut oracle::ResultSet<oracle::Row>,
    max_rows: usize,
    on_columns: &mut F1,
    on_rows: &mut F2,
    summary: &mut ExecSummary,
) where
    F1: FnMut(&[String]),
    F2: FnMut(Vec<Vec<String>>),
{
    if summary.columns.is_empty() {
        summary.columns = rs
            .column_info()
            .iter()
            .map(|c| c.name().to_string())
            .collect();
        on_columns(&summary.columns);
    }
    let mut batch: Vec<Vec<String>> = Vec::with_capacity(ROWS_PER_BATCH);
    let mut maybe_more = false;
    for row_r in &mut *rs {
        if summary.rows_read >= max_rows {
            maybe_more = true;
            break;
        }
        let row = match row_r {
            Ok(r) => r,
            Err(e) => {
                // 行级错误不中断：已收行照常发射，错误以截断形式呈现——
                // 单元格取值失败置 NULL（SqlValue Display 已兜底）
                let _ = e;
                continue;
            }
        };
        let cells: Vec<String> = row.sql_values().iter().map(|v| v.to_string()).collect();
        batch.push(cells);
        summary.rows_read += 1;
        if batch.len() >= ROWS_PER_BATCH {
            on_rows(std::mem::take(&mut batch));
        }
    }
    if !batch.is_empty() {
        on_rows(batch);
    }
    if maybe_more {
        summary.truncated = true;
    }
}
