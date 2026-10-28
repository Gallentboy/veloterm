//! 定时任务 SQLCipher 持久化：scheduled_tasks + scheduled_task_logs 两表，
//! 幂等建表（init_db 挂点调用 ensure_tables）。任务 UUID 由调用方生成。

use rusqlite::Connection;

use super::{ScheduledTask, TaskHistoryItem, TaskHistoryQuery, TaskHistoryResponse, TaskLog};
use crate::db::DbPool;

/// 幂等建表（应用启动时调用）+ 老表迁移（v1.0.12 之前建的表缺新列）
pub fn ensure_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS scheduled_tasks (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            cron_expr   TEXT NOT NULL,
            task_type   TEXT NOT NULL,
            command     TEXT NOT NULL,
            working_dir TEXT,
            host_id     INTEGER,
            enabled     INTEGER NOT NULL DEFAULT 1,
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL,
            last_run_at INTEGER,
            last_status TEXT,
            notify_on   TEXT,
            webhook_url TEXT,
            timeout_secs INTEGER DEFAULT 600,
            max_retries INTEGER DEFAULT 0,
            retry_backoff_secs INTEGER DEFAULT 30,
            run_at      INTEGER,
            retention_days INTEGER DEFAULT 30
        );
        CREATE TABLE IF NOT EXISTS scheduled_task_logs (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id     TEXT NOT NULL,
            started_at  INTEGER NOT NULL,
            finished_at INTEGER,
            exit_code   INTEGER,
            stdout_tail TEXT,
            stderr_tail TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_task_logs_task_id
            ON scheduled_task_logs(task_id, started_at DESC);",
    )
    .map_err(|e| format!("建定时任务表失败：{e}"))?;

    // 老表迁移：v1.0.12 之前的表缺新列（幂等——列已存在时报错忽略）
    let migrations = [
        "ALTER TABLE scheduled_tasks ADD COLUMN notify_on TEXT",
        "ALTER TABLE scheduled_tasks ADD COLUMN webhook_url TEXT",
        "ALTER TABLE scheduled_tasks ADD COLUMN timeout_secs INTEGER DEFAULT 600",
        "ALTER TABLE scheduled_tasks ADD COLUMN max_retries INTEGER DEFAULT 0",
        "ALTER TABLE scheduled_tasks ADD COLUMN retry_backoff_secs INTEGER DEFAULT 30",
        "ALTER TABLE scheduled_tasks ADD COLUMN run_at INTEGER",
        "ALTER TABLE scheduled_tasks ADD COLUMN retention_days INTEGER DEFAULT 30",
    ];
    for sql in migrations {
        let _ = conn.execute(sql, []);
    }
    Ok(())
}

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduledTask> {
    Ok(ScheduledTask {
        id: row.get("id")?,
        name: row.get("name")?,
        cron_expr: row.get("cron_expr")?,
        task_type: row.get("task_type")?,
        command: row.get("command")?,
        working_dir: row.get("working_dir")?,
        host_id: row.get("host_id")?,
        enabled: row.get::<_, i32>("enabled")? != 0,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        last_run_at: row.get("last_run_at")?,
        last_status: row.get("last_status")?,
        notify_on: row.get("notify_on")?,
        webhook_url: row.get("webhook_url")?,
        timeout_secs: row.get("timeout_secs")?,
        max_retries: row.get::<_, Option<i32>>("max_retries")?.map(|v| v as u32),
        retry_backoff_secs: row.get("retry_backoff_secs")?,
        run_at: row.get("run_at")?,
        retention_days: row.get::<_, Option<i32>>("retention_days")?.map(|v| v as u32),
    })
}

fn row_to_log(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskLog> {
    Ok(TaskLog {
        id: row.get("id")?,
        task_id: row.get("task_id")?,
        started_at: row.get("started_at")?,
        finished_at: row.get("finished_at")?,
        exit_code: row.get("exit_code")?,
        stdout_tail: row.get("stdout_tail")?,
        stderr_tail: row.get("stderr_tail")?,
    })
}

pub fn create_task(pool: &DbPool, task: &ScheduledTask) -> Result<ScheduledTask, String> {
    let mut created = task.clone();
    created.created_at = now_ts();
    created.updated_at = created.created_at;
    let conn = pool.get().map_err(|e| format!("取数据库连接失败：{e}"))?;
    conn.execute(
        "INSERT INTO scheduled_tasks \
         (id, name, cron_expr, task_type, command, working_dir, host_id, enabled, \
          created_at, updated_at, last_run_at, last_status, \
          notify_on, webhook_url, timeout_secs, max_retries, retry_backoff_secs, \
          run_at, retention_days) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
        rusqlite::params![
            created.id,
            created.name,
            created.cron_expr,
            created.task_type,
            created.command,
            created.working_dir,
            created.host_id,
            i32::from(created.enabled),
            created.created_at,
            created.updated_at,
            created.last_run_at,
            created.last_status,
            created.notify_on,
            created.webhook_url,
            created.timeout_secs.unwrap_or(600),
            created.max_retries.map(|v| v as i32).unwrap_or(0),
            created.retry_backoff_secs.unwrap_or(30),
            created.run_at,
            created.retention_days.map(|v| v as i32).unwrap_or(30),
        ],
    )
    .map_err(|e| format!("插入定时任务失败：{e}"))?;
    Ok(created)
}

pub fn update_task(pool: &DbPool, task: &ScheduledTask) -> Result<ScheduledTask, String> {
    let mut updated = task.clone();
    updated.updated_at = now_ts();
    let conn = pool.get().map_err(|e| format!("取数据库连接失败：{e}"))?;
    let n = conn
        .execute(
            "UPDATE scheduled_tasks SET \
             name=?2, cron_expr=?3, task_type=?4, command=?5, \
             working_dir=?6, host_id=?7, enabled=?8, updated_at=?9, \
             notify_on=?10, webhook_url=?11, timeout_secs=?12, max_retries=?13, \
             retry_backoff_secs=?14, run_at=?15, retention_days=?16 \
             WHERE id = ?1",
            rusqlite::params![
                updated.id,
                updated.name,
                updated.cron_expr,
                updated.task_type,
                updated.command,
                updated.working_dir,
                updated.host_id,
                i32::from(updated.enabled),
                updated.updated_at,
                updated.notify_on,
                updated.webhook_url,
                updated.timeout_secs.unwrap_or(600),
                updated.max_retries.map(|v| v as i32).unwrap_or(0),
                updated.retry_backoff_secs.unwrap_or(30),
                updated.run_at,
                updated.retention_days.map(|v| v as i32).unwrap_or(30),
            ],
        )
        .map_err(|e| format!("更新定时任务失败：{e}"))?;
    if n == 0 {
        return Err("任务不存在（已被删除？）".to_string());
    }
    Ok(updated)
}

pub fn delete_task(pool: &DbPool, id: &str) -> Result<(), String> {
    let conn = pool.get().map_err(|e| format!("取数据库连接失败：{e}"))?;
    conn.execute("DELETE FROM scheduled_task_logs WHERE task_id = ?1", [id])
        .map_err(|e| format!("删除任务日志失败：{e}"))?;
    conn.execute("DELETE FROM scheduled_tasks WHERE id = ?1", [id])
        .map_err(|e| format!("删除定时任务失败：{e}"))?;
    Ok(())
}

pub fn set_enabled(pool: &DbPool, id: &str, enabled: bool) -> Result<ScheduledTask, String> {
    let conn = pool.get().map_err(|e| format!("取数据库连接失败：{e}"))?;
    conn.execute(
        "UPDATE scheduled_tasks SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![id, i32::from(enabled), now_ts()],
    )
    .map_err(|e| format!("切换任务状态失败：{e}"))?;
    get_task(pool, id)
}

pub fn get_task(pool: &DbPool, id: &str) -> Result<ScheduledTask, String> {
    let conn = pool.get().map_err(|e| format!("取数据库连接失败：{e}"))?;
    conn.query_row(
        "SELECT * FROM scheduled_tasks WHERE id = ?1",
        [id],
        row_to_task,
    )
    .map_err(|e| format!("读取定时任务失败：{e}").into())
}

pub fn list_tasks(pool: &DbPool) -> Result<Vec<ScheduledTask>, String> {
    let conn = pool.get().map_err(|e| format!("取数据库连接失败：{e}"))?;
    let mut stmt = conn
        .prepare("SELECT * FROM scheduled_tasks ORDER BY created_at DESC")
        .map_err(|e| format!("查询定时任务失败：{e}"))?;
    let rows = stmt
        .query_map([], row_to_task)
        .map_err(|e| format!("查询定时任务失败：{e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取定时任务行失败：{e}").into())
}

pub fn list_logs(pool: &DbPool, task_id: &str, limit: usize) -> Result<Vec<TaskLog>, String> {
    let conn = pool.get().map_err(|e| format!("取数据库连接失败：{e}"))?;
    let mut stmt = conn
        .prepare(
            "SELECT * FROM scheduled_task_logs WHERE task_id = ?1 \
             ORDER BY started_at DESC LIMIT ?2",
        )
        .map_err(|e| format!("查询任务日志失败：{e}"))?;
    let rows = stmt
        .query_map(rusqlite::params![task_id, limit as i64], row_to_log)
        .map_err(|e| format!("查询任务日志失败：{e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取任务日志行失败：{e}").into())
}

/// 记录一次执行（插入日志 + 更新任务的 last_run_at / last_status）
pub fn record_run(
    pool: &DbPool,
    task_id: &str,
    started_at: i64,
    finished_at: Option<i64>,
    exit_code: Option<i32>,
    stdout_tail: Option<&str>,
    stderr_tail: Option<&str>,
) -> Result<(), String> {
    let status = match exit_code {
        Some(0) => "success",
        Some(_) => "failed",
        None => "running",
    };
    let conn = pool.get().map_err(|e| format!("取数据库连接失败：{e}"))?;
    conn.execute(
        "INSERT INTO scheduled_task_logs \
         (task_id, started_at, finished_at, exit_code, stdout_tail, stderr_tail) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![task_id, started_at, finished_at, exit_code, stdout_tail, stderr_tail],
    )
    .map_err(|e| format!("插入任务日志失败：{e}"))?;
    conn.execute(
        "UPDATE scheduled_tasks SET last_run_at = ?2, last_status = ?3 WHERE id = ?1",
        rusqlite::params![task_id, started_at, status],
    )
    .map_err(|e| format!("更新任务状态失败：{e}"))?;
    Ok(())
}

/// 补全一条 running 记录的执行结果（executor 完成后调用）
pub fn record_run_extra(
    pool: &DbPool,
    task_id: &str,
    started_at: i64,
    finished_at: i64,
    exit_code: Option<i32>,
    stdout_tail: Option<&str>,
    stderr_tail: Option<&str>,
) -> Result<(), String> {
    let status = match exit_code {
        Some(0) => "success",
        Some(_) => "failed",
        None => "failed", // 无退出码（超时/SSH 断开等）
    };
    let conn = pool.get().map_err(|e| format!("取数据库连接失败：{e}"))?;
    conn.execute(
        "UPDATE scheduled_task_logs SET \
         finished_at = ?3, exit_code = ?4, stdout_tail = ?5, stderr_tail = ?6 \
         WHERE task_id = ?1 AND started_at = ?2 AND finished_at IS NULL",
        rusqlite::params![task_id, started_at, finished_at, exit_code, stdout_tail, stderr_tail],
    )
    .map_err(|e| format!("补全任务日志失败：{e}"))?;
    conn.execute(
        "UPDATE scheduled_tasks SET last_run_at = ?2, last_status = ?3 WHERE id = ?1",
        rusqlite::params![task_id, started_at, status],
    )
    .map_err(|e| format!("更新任务状态失败：{e}"))?;
    Ok(())
}

/// 执行历史查询（支持多维度过滤、分页、任务元信息关联）
pub fn query_history(
    pool: &DbPool,
    query: &TaskHistoryQuery,
) -> Result<TaskHistoryResponse, String> {
    let conn = pool.get().map_err(|e| format!("取数据库连接失败：{e}"))?;

    let mut where_clauses = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(ref tid) = query.task_id {
        let trimmed = tid.trim();
        if !trimmed.is_empty() && trimmed != "all" {
            where_clauses.push(format!("l.task_id = ?{}", params.len() + 1));
            params.push(Box::new(trimmed.to_string()));
        }
    }

    if let Some(ref status) = query.status {
        match status.as_str() {
            "success" => {
                where_clauses.push("l.finished_at IS NOT NULL AND l.exit_code = 0".to_string());
            }
            "failed" => {
                where_clauses.push(
                    "l.finished_at IS NOT NULL AND (l.exit_code != 0 OR l.exit_code IS NULL)"
                        .to_string(),
                );
            }
            "running" => {
                where_clauses.push("l.finished_at IS NULL".to_string());
            }
            _ => {}
        }
    }

    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    // 查总数
    let count_sql = format!("SELECT COUNT(*) FROM scheduled_task_logs l {where_sql}");
    let mut count_stmt = conn
        .prepare(&count_sql)
        .map_err(|e| format!("准备统计查询失败：{e}"))?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let total: usize = count_stmt
        .query_row(param_refs.as_slice(), |r| r.get(0))
        .map_err(|e| format!("统计历史记录数失败：{e}"))?;

    // 查记录
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0);
    let limit_param_idx = params.len() + 1;
    let offset_param_idx = params.len() + 2;

    let data_sql = format!(
        "SELECT l.id, l.task_id, COALESCE(t.name, '已删除任务') AS task_name, \
         t.task_type, t.command, l.started_at, l.finished_at, l.exit_code, \
         l.stdout_tail, l.stderr_tail \
         FROM scheduled_task_logs l \
         LEFT JOIN scheduled_tasks t ON l.task_id = t.id \
         {where_sql} \
         ORDER BY l.started_at DESC, l.id DESC \
         LIMIT ?{limit_param_idx} OFFSET ?{offset_param_idx}"
    );

    params.push(Box::new(limit as i64));
    params.push(Box::new(offset as i64));
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn
        .prepare(&data_sql)
        .map_err(|e| format!("准备历史查询失败：{e}"))?;
    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            let started_at: i64 = row.get("started_at")?;
            let finished_at: Option<i64> = row.get("finished_at")?;
            let exit_code: Option<i32> = row.get("exit_code")?;
            let duration_ms = finished_at.map(|fin| (fin - started_at).max(0) * 1000);
            let status = match (finished_at, exit_code) {
                (None, _) => "running".to_string(),
                (Some(_), Some(0)) => "success".to_string(),
                (Some(_), _) => "failed".to_string(),
            };
            Ok(TaskHistoryItem {
                id: row.get("id")?,
                task_id: row.get("task_id")?,
                task_name: row.get("task_name")?,
                task_type: row.get("task_type")?,
                command: row.get("command")?,
                started_at,
                finished_at,
                duration_ms,
                exit_code,
                status,
                stdout_tail: row.get("stdout_tail")?,
                stderr_tail: row.get("stderr_tail")?,
            })
        })
        .map_err(|e| format!("查询历史记录失败：{e}"))?;

    let items = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取历史记录行失败：{e}"))?;
    Ok(TaskHistoryResponse { items, total })
}

/// 清空历史记录（可按 task_id 或清空全部）
pub fn clear_history(pool: &DbPool, task_id: Option<&str>) -> Result<(), String> {
    let conn = pool.get().map_err(|e| format!("取数据库连接失败：{e}"))?;
    if let Some(tid) = task_id {
        let trimmed = tid.trim();
        if !trimmed.is_empty() && trimmed != "all" {
            conn.execute("DELETE FROM scheduled_task_logs WHERE task_id = ?1", [trimmed])
                .map_err(|e| format!("清空任务日志失败：{e}"))?;
            return Ok(());
        }
    }
    conn.execute("DELETE FROM scheduled_task_logs", [])
        .map_err(|e| format!("清空全部日志失败：{e}"))?;
    Ok(())
}

/// D5 历史保留策略清理：按各任务的 retention_days 删除超期日志。
/// retention_days = 0 表示永久保留（跳过）；null 用默认 30 天。
/// 引擎 load_all 后调用
pub fn cleanup_expired_logs(pool: &DbPool) -> Result<usize, String> {
    let conn = pool.get().map_err(|e| format!("取数据库连接失败：{e}"))?;
    let now = now_ts();
    // 按任务的 retention_days 逐个清理（每个任务可以有不同保留期）
    let sql = "
        DELETE FROM scheduled_task_logs
        WHERE task_id IN (
            SELECT id FROM scheduled_tasks WHERE COALESCE(retention_days, 30) > 0
        )
        AND started_at < (
            SELECT ?1 - COALESCE(retention_days, 30) * 86400
            FROM scheduled_tasks WHERE id = task_id
        )";
    let n = conn
        .execute(sql, [now])
        .map_err(|e| format!("清理超期日志失败：{e}"))?;
    if n > 0 {
        log::info!("[scheduler] 清理了 {n} 条超期执行日志");
    }
    Ok(n)
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_pool() -> DbPool {
        let manager = r2d2_sqlite::SqliteConnectionManager::memory();
        r2d2::Pool::builder()
            .build(manager)
            .expect("建内存库失败")
    }

    fn sample(id: &str, task_type: &str, enabled: bool) -> ScheduledTask {
        ScheduledTask {
            id: id.to_string(),
            name: format!("测试任务 {id}"),
            cron_expr: "*/5 * * * *".to_string(),
            task_type: task_type.to_string(),
            command: "echo hello".to_string(),
            working_dir: None,
            host_id: if task_type == "ssh_command" { Some(1) } else { None },
            enabled,
            created_at: 0,
            updated_at: 0,
            last_run_at: None,
            last_status: None,
            notify_on: None,
            webhook_url: None,
            timeout_secs: None,
            max_retries: None,
            retry_backoff_secs: None,
            run_at: None,
            retention_days: None,
        }
    }

    #[test]
    fn task_crud_roundtrip() {
        let pool = test_pool();
        {
            let conn = pool.get().unwrap();
            ensure_tables(&conn).unwrap();
        }

        // 创建 → 读回 → 字段一致
        let t = sample("uuid-1", "local_command", true);
        let created = create_task(&pool, &t).unwrap();
        assert!(created.created_at > 0);
        let got = get_task(&pool, "uuid-1").unwrap();
        assert_eq!(got.name, "测试任务 uuid-1");
        assert!(got.enabled);
        assert_eq!(got.task_type, "local_command");

        // 更新
        let mut updated = got.clone();
        updated.name = "改名".to_string();
        updated.cron_expr = "0 3 * * *".to_string();
        let u = update_task(&pool, &updated).unwrap();
        assert_eq!(u.name, "改名");
        assert_eq!(u.cron_expr, "0 3 * * *");

        // 启停
        let paused = set_enabled(&pool, "uuid-1", false).unwrap();
        assert!(!paused.enabled);
        let resumed = set_enabled(&pool, "uuid-1", true).unwrap();
        assert!(resumed.enabled);

        // 列表
        let _t2 = create_task(&pool, &sample("uuid-2", "ssh_command", false)).unwrap();
        let all = list_tasks(&pool).unwrap();
        assert_eq!(all.len(), 2);

        // 日志记录 + 查询
        record_run(&pool, "uuid-1", 100, Some(160), Some(0), Some("hello"), None).unwrap();
        record_run(&pool, "uuid-1", 200, Some(260), Some(1), None, Some("err")).unwrap();
        let logs = list_logs(&pool, "uuid-1", 10).unwrap();
        assert_eq!(logs.len(), 2);
        // DESC 排序：最新在前
        let latest = logs.first().expect("至少 2 条");
        assert_eq!(latest.started_at, 200);
        assert_eq!(latest.exit_code, Some(1));
        assert_eq!(latest.stderr_tail.as_deref(), Some("err"));

        // 状态联动
        let after = get_task(&pool, "uuid-1").unwrap();
        assert_eq!(after.last_status.as_deref(), Some("failed"));
        assert_eq!(after.last_run_at, Some(200));

        // 删除（连带日志）
        delete_task(&pool, "uuid-1").unwrap();
        assert!(get_task(&pool, "uuid-1").is_err());
        assert!(list_logs(&pool, "uuid-1", 10).unwrap().is_empty());
    }

    #[test]
    fn test_history_query_and_clear() {
        let pool = test_pool();
        {
            let conn = pool.get().unwrap();
            ensure_tables(&conn).unwrap();
        }

        let t1 = sample("task-a", "local_command", true);
        let t2 = sample("task-b", "ssh_command", true);
        create_task(&pool, &t1).unwrap();
        create_task(&pool, &t2).unwrap();

        // 插入若干条日志
        record_run(&pool, "task-a", 1000, Some(1010), Some(0), Some("ok a"), None).unwrap();
        record_run(&pool, "task-a", 1020, Some(1025), Some(1), None, Some("err a")).unwrap();
        record_run(&pool, "task-b", 1030, None, None, None, None).unwrap(); // running

        // 1. 全局查
        let q_all = TaskHistoryQuery::default();
        let resp = query_history(&pool, &q_all).unwrap();
        assert_eq!(resp.total, 3);
        assert_eq!(resp.items.len(), 3);
        assert_eq!(resp.items.first().unwrap().started_at, 1030);
        assert_eq!(resp.items.first().unwrap().status, "running");
        assert_eq!(resp.items.first().unwrap().task_name, "测试任务 task-b");

        // 2. 按任务查
        let q_task_a = TaskHistoryQuery {
            task_id: Some("task-a".to_string()),
            ..Default::default()
        };
        let resp_a = query_history(&pool, &q_task_a).unwrap();
        assert_eq!(resp_a.total, 2);
        assert_eq!(resp_a.items.first().unwrap().started_at, 1020);
        assert_eq!(resp_a.items.first().unwrap().status, "failed");

        // 3. 按状态查
        let q_success = TaskHistoryQuery {
            status: Some("success".to_string()),
            ..Default::default()
        };
        let resp_s = query_history(&pool, &q_success).unwrap();
        assert_eq!(resp_s.total, 1);
        assert_eq!(resp_s.items.first().unwrap().task_id, "task-a");
        assert_eq!(resp_s.items.first().unwrap().exit_code, Some(0));

        // 4. 清理特定任务历史
        clear_history(&pool, Some("task-a")).unwrap();
        let resp_after_clear_a = query_history(&pool, &q_all).unwrap();
        assert_eq!(resp_after_clear_a.total, 1);
        assert_eq!(resp_after_clear_a.items.first().unwrap().task_id, "task-b");

        // 5. 清理全部历史
        clear_history(&pool, None).unwrap();
        let resp_empty = query_history(&pool, &q_all).unwrap();
        assert_eq!(resp_empty.total, 0);
    }
}
