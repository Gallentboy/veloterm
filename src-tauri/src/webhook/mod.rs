//! WebHook 守护服务主模块：生命周期管理与 Tauri 命令层。

pub mod executor;
pub mod server;
pub mod store;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use axum::serve;
use tokio::net::TcpListener;
use tokio::sync::{watch, RwLock};

pub use executor::{interpolate_template, WebhookContext};
pub use store::{
    WebhookLog, WebhookRule, WebhookSettings, WebhookStatus,
};
use crate::db::{now_timestamp_ms, DbPool};

/// WebHook 运行管理器
pub struct WebhookManager {
    is_running: AtomicBool,
    last_error: RwLock<Option<String>>,
    stop_tx: RwLock<Option<watch::Sender<bool>>>,
    settings: Arc<RwLock<WebhookSettings>>,
    rules: Arc<RwLock<Vec<WebhookRule>>>,
}

impl WebhookManager {
    pub fn new() -> Self {
        Self {
            is_running: AtomicBool::new(false),
            last_error: RwLock::new(None),
            stop_tx: RwLock::new(None),
            settings: Arc::new(RwLock::new(WebhookSettings::default())),
            rules: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// 重新加载规则缓存
    pub async fn reload_rules(&self, pool: &DbPool) {
        if let Ok(conn) = pool.get() {
            if let Ok(list) = store::list_rules(&conn) {
                *self.rules.write().await = list;
            }
        }
    }

    /// 获取当前运行状态
    pub async fn get_status(&self) -> WebhookStatus {
        let is_running = self.is_running.load(Ordering::SeqCst);
        let s = self.settings.read().await.clone();
        let r_count = self.rules.read().await.iter().filter(|r| r.enabled).count();
        let err = self.last_error.read().await.clone();

        WebhookStatus {
            is_running,
            bind_ip: s.bind_ip,
            bind_port: s.bind_port,
            active_rules_count: r_count,
            error: err,
        }
    }

    /// 平滑重启或停止 HTTP 服务
    pub async fn restart(&self, pool: DbPool) -> Result<(), String> {
        // 1. 发出停止信号并等待先前的服务器退出
        if let Some(tx) = self.stop_tx.write().await.take() {
            let _ = tx.send(true);
        }
        self.is_running.store(false, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(150)).await;

        // 2. 从数据库读取最新配置与规则
        let settings = if let Ok(conn) = pool.get() {
            let s = store::get_settings(&conn)?;
            let r = store::list_rules(&conn)?;
            *self.settings.write().await = s.clone();
            *self.rules.write().await = r;
            s
        } else {
            return Err("无法获取数据库连接".to_string());
        };

        if !settings.enabled {
            *self.last_error.write().await = None;
            return Ok(());
        }

        // 3. 解析绑定地址
        let bind_str = format!("{}:{}", settings.bind_ip, settings.bind_port);
        let addr: SocketAddr = bind_str
            .parse()
            .map_err(|e| format!("无效的监听地址或端口 {bind_str}: {e}"))?;

        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                let err_msg = format!("绑定端口 {bind_str} 失败: {e}");
                *self.last_error.write().await = Some(err_msg.clone());
                return Err(err_msg);
            }
        };

        *self.last_error.write().await = None;
        self.is_running.store(true, Ordering::SeqCst);

        let (stop_tx, mut stop_rx) = watch::channel(false);
        *self.stop_tx.write().await = Some(stop_tx);

        let app_state = server::AppState {
            pool: pool.clone(),
            settings: self.settings.clone(),
            rules: self.rules.clone(),
        };

        let router = server::build_router(app_state);

        tokio::spawn(async move {
            let server_future = serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            );

            tokio::select! {
                _ = server_future => {},
                _ = stop_rx.changed() => {},
            }
            log::info!("[webhook] HTTP 服务已平滑停止");
        });

        log::info!("[webhook] HTTP 服务已在 {bind_str} 成功拉起");
        Ok(())
    }
}

pub static GLOBAL_WEBHOOK_MGR: LazyLock<WebhookManager> = LazyLock::new(WebhookManager::new);

// ---------------------------------------------------------------------------
// Tauri Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn webhook_get_settings(
    pool: tauri::State<'_, DbPool>,
) -> Result<WebhookSettings, String> {
    let pool_clone = pool.inner().clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.get().map_err(|e| e.to_string())?;
        store::get_settings(&conn)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn webhook_update_settings(
    settings: WebhookSettings,
    pool: tauri::State<'_, DbPool>,
) -> Result<(), String> {
    let pool_clone = pool.inner().clone();
    let settings_clone = settings.clone();

    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.get().map_err(|e| e.to_string())?;
        store::update_settings(&conn, &settings_clone)
    })
    .await
    .map_err(|e| e.to_string())??;

    // 平滑重启或启停服务
    GLOBAL_WEBHOOK_MGR.restart(pool.inner().clone()).await
}

#[tauri::command]
pub async fn webhook_get_status() -> Result<WebhookStatus, String> {
    Ok(GLOBAL_WEBHOOK_MGR.get_status().await)
}

#[tauri::command]
pub async fn webhook_rule_list(
    pool: tauri::State<'_, DbPool>,
) -> Result<Vec<WebhookRule>, String> {
    let pool_clone = pool.inner().clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.get().map_err(|e| e.to_string())?;
        store::list_rules(&conn)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn webhook_rule_save(
    rule: WebhookRule,
    pool: tauri::State<'_, DbPool>,
) -> Result<(), String> {
    let pool_clone = pool.inner().clone();
    let r = rule.clone();

    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.get().map_err(|e| e.to_string())?;
        store::save_rule(&conn, &r)
    })
    .await
    .map_err(|e| e.to_string())??;

    GLOBAL_WEBHOOK_MGR.reload_rules(pool.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn webhook_rule_delete(
    id: String,
    pool: tauri::State<'_, DbPool>,
) -> Result<(), String> {
    let pool_clone = pool.inner().clone();
    let id_clone = id.clone();

    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.get().map_err(|e| e.to_string())?;
        store::delete_rule(&conn, &id_clone)
    })
    .await
    .map_err(|e| e.to_string())??;

    GLOBAL_WEBHOOK_MGR.reload_rules(pool.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn webhook_rule_toggle(
    id: String,
    enabled: bool,
    pool: tauri::State<'_, DbPool>,
) -> Result<(), String> {
    let pool_clone = pool.inner().clone();
    let id_clone = id.clone();

    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.get().map_err(|e| e.to_string())?;
        store::toggle_rule(&conn, &id_clone, enabled)
    })
    .await
    .map_err(|e| e.to_string())??;

    GLOBAL_WEBHOOK_MGR.reload_rules(pool.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn webhook_logs_list(
    limit: usize,
    offset: usize,
    rule_id: Option<String>,
    pool: tauri::State<'_, DbPool>,
) -> Result<serde_json::Value, String> {
    let pool_clone = pool.inner().clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.get().map_err(|e| e.to_string())?;
        let (list, total) = store::list_logs(&conn, limit, offset, rule_id.as_deref())?;
        Ok(serde_json::json!({
            "items": list,
            "total": total,
            "limit": limit,
            "offset": offset,
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn webhook_logs_clear(
    pool: tauri::State<'_, DbPool>,
) -> Result<(), String> {
    let pool_clone = pool.inner().clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.get().map_err(|e| e.to_string())?;
        store::clear_logs(&conn)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 模拟测试某条规则的执行
#[tauri::command]
pub async fn webhook_rule_test(
    rule: WebhookRule,
    test_context: WebhookContext,
    pool: tauri::State<'_, DbPool>,
) -> Result<serde_json::Value, String> {
    let start = std::time::Instant::now();
    let output = executor::execute_rule(&rule, &test_context, pool.inner()).await;
    let elapsed = start.elapsed().as_millis() as i64;

    match output {
        Ok(out) => Ok(serde_json::json!({
            "success": out.exit_code == Some(0),
            "exitCode": out.exit_code,
            "stdout": out.stdout,
            "stderr": out.stderr,
            "durationMs": elapsed
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "exitCode": 1,
            "stdout": "",
            "stderr": e,
            "durationMs": elapsed
        })),
    }
}

/// 重新执行（重放）历史请求
#[tauri::command]
pub async fn webhook_log_replay(
    log_id: String,
    pool: tauri::State<'_, DbPool>,
) -> Result<serde_json::Value, String> {
    let pool_clone = pool.inner().clone();
    let (rule, ctx) = tokio::task::spawn_blocking(move || {
        let conn = pool_clone.get().map_err(|e| e.to_string())?;
        let (logs, _) = store::list_logs(&conn, 1, 0, None)?;
        let log = logs.into_iter().find(|l| l.id == log_id)
            .ok_or_else(|| format!("未找到指定的审计日志 #{log_id}"))?;

        let rule_id = log.rule_id.ok_or("该日志未匹配到有效规则，无法直接重放")?;
        let rule = store::get_rule(&conn, &rule_id)?
            .ok_or("原始规则已被删除，无法重放")?;

        let raw_body = log.request_body.unwrap_or_default();
        let body_json = serde_json::from_str(&raw_body).ok();
        let headers: HashMap<String, String> = log.request_headers
            .and_then(|h| serde_json::from_str(&h).ok())
            .unwrap_or_default();
        let query_params: HashMap<String, String> = log.query_string
            .and_then(|q| serde_json::from_str(&q).ok())
            .unwrap_or_default();

        let path_params = server::match_path(&rule.path_pattern, &log.path)
            .unwrap_or_default();

        let ctx = WebhookContext {
            method: log.method,
            path: log.path,
            client_ip: "127.0.0.1".to_string(),
            path_params,
            query_params,
            headers,
            body_json,
            raw_body,
        };

        Ok::<(WebhookRule, WebhookContext), String>((rule, ctx))
    })
    .await
    .map_err(|e| e.to_string())??;

    let start = std::time::Instant::now();
    let output = executor::execute_rule(&rule, &ctx, pool.inner()).await;
    let elapsed = start.elapsed().as_millis() as i64;

    // 记录新的审计日志
    let new_log_id = uuid::Uuid::new_v4().to_string();
    let (status_str, exit_code, stdout, stderr) = match output {
        Ok(ref out) => {
            let st = if out.exit_code == Some(0) { "SUCCESS" } else { "FAILED" };
            (st, out.exit_code, out.stdout.clone(), out.stderr.clone())
        }
        Err(ref e) => ("FAILED", Some(1), String::new(), e.clone()),
    };

    if let Ok(conn) = pool.get() {
        let headers_json = serde_json::to_string(&ctx.headers).unwrap_or_default();
        let query_str = serde_json::to_string(&ctx.query_params).unwrap_or_default();
        let log_entry = WebhookLog {
            id: new_log_id.clone(),
            rule_id: Some(rule.id.clone()),
            rule_name: Some(format!("{} (Replay)", rule.name)),
            client_ip: "127.0.0.1".to_string(),
            method: ctx.method.clone(),
            path: ctx.path.clone(),
            query_string: Some(query_str),
            request_headers: Some(headers_json),
            request_body: Some(ctx.raw_body.clone()),
            match_status: "MATCHED".to_string(),
            execution_status: Some(status_str.to_string()),
            exit_code,
            stdout_tail: Some(stdout.clone()),
            stderr_tail: Some(stderr.clone()),
            duration_ms: Some(elapsed),
            created_at: now_timestamp_ms(),
        };
        let _ = store::insert_log(&conn, &log_entry);
    }

    Ok(serde_json::json!({
        "logId": new_log_id,
        "status": status_str,
        "exitCode": exit_code,
        "stdout": stdout,
        "stderr": stderr,
        "durationMs": elapsed
    }))
}
