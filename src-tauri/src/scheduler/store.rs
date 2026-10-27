//! 定时任务 SQLCipher 持久化：scheduled_tasks + scheduled_task_logs 两表，
//! 幂等建表（init_db 挂点调用 ensure_tables）。任务 UUID 由调用方生成。

use rusqlite::Connection;

use super::{ScheduledTask, TaskLog};
use crate::db::DbPool;

/// 幂等建表（应用启动时调用）
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
            last_status TEXT
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
          created_at, updated_at, last_run_at, last_status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
             name = ?2, cron_expr = ?3, task_type = ?4, command = ?5, \
             working_dir = ?6, host_id = ?7, enabled = ?8, updated_at = ?9 \
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
}
