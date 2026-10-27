//! tokio-cron-scheduler 包装：应用启动时从 DB 加载 enabled 任务注册 Job，
//! CRUD 时动态 add/remove。每个任务的执行在独立 tokio::spawn 里跑
//! （不阻塞调度循环），执行结果写库 + 事件推送前端。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use super::{executor, store, ScheduledTask};
use crate::db::DbPool;

pub struct SchedulerEngine {
    scheduler: JobScheduler,
    /// 运行中的 Job UUID 集合（防止重复注册；remove 后清理）
    active_jobs: Arc<Mutex<HashMap<String, Uuid>>>,
    pool: DbPool,
    app: AppHandle,
}

impl SchedulerEngine {
    /// 创建并启动引擎（不加载任务——start_from_db 由调用方在 DB 就绪后触发）
    pub async fn new(pool: DbPool, app: AppHandle) -> Result<Self, String> {
        let scheduler = JobScheduler::new()
            .await
            .map_err(|e| format!("创建调度器失败：{e}"))?;
        scheduler
            .start()
            .await
            .map_err(|e| format!("启动调度器失败：{e}"))?;
        Ok(Self {
            scheduler,
            active_jobs: Arc::new(Mutex::new(HashMap::new())),
            pool,
            app,
        })
    }

    /// 应用启动 / 任务变更后：从 DB 加载全部 enabled 任务注册 Job。
    /// 重复调用安全：先清理 active_jobs 里已有的
    pub async fn load_all(&self) -> Result<(), String> {
        let tasks = store::list_tasks(&self.pool)?;
        let mut active = self.active_jobs.lock().await;
        active.clear();
        for task in tasks.iter().filter(|t| t.enabled) {
            if let Err(e) = self.add_job_inner(&mut active, task).await {
                log::warn!(
                    "[scheduler] 任务 {}（{}）注册失败，跳过：{e}",
                    task.name,
                    task.id
                );
            }
        }
        log::info!("[scheduler] 已加载 {}/{} 个定时任务", active.len(), tasks.len());
        Ok(())
    }

    /// 单条任务注册 Job（CRUD create/update/toggle 后调用）
    pub async fn add_job(&self, pool: &DbPool, task: &ScheduledTask) -> Result<(), String> {
        let mut active = self.active_jobs.lock().await;
        self.add_job_inner(&mut active, task).await
    }

    async fn add_job_inner(
        &self,
        active: &mut HashMap<String, Uuid>,
        task: &ScheduledTask,
    ) -> Result<(), String> {
        // 已注册的同 id 任务先删（幂等重挂）
        if let Some(old) = active.remove(&task.id) {
            let _ = self.scheduler.remove(&old).await;
        }

        let task_id = task.id.clone();
        let cron = task.cron_expr.clone();
        let pool = self.pool.clone();
        let app = self.app.clone();
        let task = task.clone();
        let task_for_active = task.id.clone();

        let job = Job::new_async_tz(cron.as_str(), chrono::Local, move |_uuid, _lock| {
            let pool = pool.clone();
            let app = app.clone();
            let task = task.clone();
            Box::pin(async move {
                run_task(&task, &pool, &app).await;
            })
        })
        .map_err(|e| format!("cron 表达式「{cron}」无效：{e}"))?;

        let job_uuid = job.guid();
        self.scheduler
            .add(job)
            .await
            .map_err(|e| format!("注册调度任务失败：{e}"))?;
        active.insert(task_for_active, job_uuid);
        Ok(())
    }

    /// 删除任务的 Job（CRUD delete / toggle off 后调用；不存在时静默）
    pub async fn remove_job(&self, id: &str) {
        let mut active = self.active_jobs.lock().await;
        if let Some(uuid) = active.remove(id) {
            let _ = self.scheduler.remove(&uuid).await;
        }
    }

    /// 立即执行一次（手动触发；不影响 cron 调度）
    pub async fn run_now(&self, id: &str) -> Result<(), String> {
        let task = store::get_task(&self.pool, id)?;
        let pool = self.pool.clone();
        let app = self.app.clone();
        tokio::spawn(async move {
            run_task(&task, &pool, &app).await;
        });
        Ok(())
    }
}

/// 执行一个任务并记录结果 + 推送事件
async fn run_task(task: &ScheduledTask, pool: &DbPool, app: &AppHandle) {
    let started_ts = chrono::Utc::now().timestamp();

    // 推送 started 事件
    let _ = app.emit("scheduler-task-event", executor::started_event(task, started_ts));

    // 记录 running 状态
    let _ = store::record_run(pool, &task.id, started_ts, None, None, None, None);

    let result = executor::execute(task, pool).await;
    let finished_ts = chrono::Utc::now().timestamp();

    // 更新日志 + 任务状态
    let _ = store::record_run_extra(
        pool,
        &task.id,
        started_ts,
        finished_ts,
        result.exit_code,
        Some(&result.stdout_tail),
        Some(&result.stderr_tail),
    );

    // 推送 finished 事件
    let _ = app.emit(
        "scheduler-task-event",
        executor::finished_event(task, started_ts, finished_ts, result.exit_code, &result.stderr_tail),
    );
}
