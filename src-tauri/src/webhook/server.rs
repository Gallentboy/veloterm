//! WebHook HTTP 守护服务：基于 Axum 的动态路由分发、签名鉴权与响应控制。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};
use axum::routing::any;
use axum::Router;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tower_http::cors::{Any, CorsLayer};

use super::executor::{execute_rule, WebhookContext};
use super::store::{self, WebhookLog, WebhookRule, WebhookSettings};
use crate::db::{now_timestamp_ms, DbPool};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub settings: Arc<tokio::sync::RwLock<WebhookSettings>>,
    pub rules: Arc<tokio::sync::RwLock<Vec<WebhookRule>>>,
}

/// 路径模式匹配：支持 `/api/deploy`、`/hook/:service/:env` 与通配符
pub fn match_path(pattern: &str, actual_path: &str) -> Option<HashMap<String, String>> {
    let pat_trimmed = pattern.trim_matches('/');
    let act_trimmed = actual_path.trim_matches('/');

    if pat_trimmed == act_trimmed {
        return Some(HashMap::new());
    }

    let pat_parts: Vec<&str> = if pat_trimmed.is_empty() {
        Vec::new()
    } else {
        pat_trimmed.split('/').collect()
    };
    let act_parts: Vec<&str> = if act_trimmed.is_empty() {
        Vec::new()
    } else {
        act_trimmed.split('/').collect()
    };

    if pat_parts.len() != act_parts.len() {
        // 通配符匹配 (如 /hook/*)
        if pat_parts.last() == Some(&"*") && act_parts.len() >= pat_parts.len() - 1 {
            let mut params = HashMap::new();
            for (p, a) in pat_parts.iter().zip(act_parts.iter()) {
                if *p == "*" {
                    break;
                }
                if p.starts_with(':') {
                    params.insert(p[1..].to_string(), a.to_string());
                } else if p != a {
                    return None;
                }
            }
            return Some(params);
        }
        return None;
    }

    let mut params = HashMap::new();
    for (p, a) in pat_parts.iter().zip(act_parts.iter()) {
        if p.starts_with(':') {
            params.insert(p[1..].to_string(), a.to_string());
        } else if p != a {
            return None;
        }
    }

    Some(params)
}

/// 简易十六进制字符串解码为字节流
fn decode_hex(hex_str: &str) -> Option<Vec<u8>> {
    if hex_str.len() % 2 != 0 {
        return None;
    }
    (0..hex_str.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).ok())
        .collect()
}

/// 常数时间安全字节比较（防范时序侧信道攻击）
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut res = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        res |= x ^ y;
    }
    res == 0
}

/// 校验 GitHub HMAC SHA-256 签名
fn verify_github_signature(secret: &str, raw_body: &[u8], sig_header: &str) -> bool {
    let expected_prefix = "sha256=";
    if !sig_header.starts_with(expected_prefix) {
        return false;
    }
    let hex_sig = &sig_header[expected_prefix.len()..];
    let Some(sig_bytes) = decode_hex(hex_sig) else {
        return false;
    };

    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(raw_body);
    let result = mac.finalize().into_bytes();

    constant_time_eq(result.as_slice(), &sig_bytes)
}

/// 简单求值过滤表达式（如 `body.ref == "refs/heads/main"`）
fn evaluate_filter_expr(expr: &str, ctx: &WebhookContext) -> bool {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return true;
    }

    let var_map = super::executor::build_variable_map(ctx, None);

    if let Some((left, right)) = trimmed.split_once("==") {
        let l_key = left.trim();
        let r_val = right.trim().trim_matches('"').trim_matches('\'');
        let l_val = var_map.get(l_key).map(|s| s.as_str()).unwrap_or("");
        return l_val == r_val;
    }

    if let Some((left, right)) = trimmed.split_once("!=") {
        let l_key = left.trim();
        let r_val = right.trim().trim_matches('"').trim_matches('\'');
        let l_val = var_map.get(l_key).map(|s| s.as_str()).unwrap_or("");
        return l_val != r_val;
    }

    if let Some((left, right)) = trimmed.split_once(" contains ") {
        let l_key = left.trim();
        let r_val = right.trim().trim_matches('"').trim_matches('\'');
        let l_val = var_map.get(l_key).map(|s| s.as_str()).unwrap_or("");
        return l_val.contains(r_val);
    }

    true
}

/// 全局兜底 Handler：接收所有 HTTP 请求并分发
async fn handle_webhook(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query_params): Query<HashMap<String, String>>,
    req: Request,
) -> impl IntoResponse {
    let start_time = Instant::now();
    let method = req.method().to_string().to_uppercase();
    let path = req.uri().path().to_string();
    let client_ip = addr.ip().to_string();

    let settings = state.settings.read().await.clone();
    if !settings.enabled {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "code": 503,
                "error": "WebHook 服务未开启"
            })),
        );
    }

    // 读取请求体
    let raw_bytes: Bytes = match axum::body::to_bytes(req.into_body(), settings.max_body_bytes).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({
                    "code": 413,
                    "error": format!("请求体过大或读取失败: {e}")
                })),
            );
        }
    };

    let raw_body = String::from_utf8_lossy(&raw_bytes).to_string();
    let body_json: Option<serde_json::Value> = serde_json::from_str(&raw_body).ok();

    // 收集 Headers
    let mut header_map: HashMap<String, String> = HashMap::new();
    for (k, v) in &headers {
        if let Ok(v_str) = v.to_str() {
            header_map.insert(k.as_str().to_lowercase(), v_str.to_string());
        }
    }

    // 全局 Token 校验（若配置）
    if let Some(ref global_sec) = settings.global_secret {
        if !global_sec.trim().is_empty() {
            let req_token = header_map
                .get("x-velo-token")
                .cloned()
                .or_else(|| query_params.get("token").cloned());
            if req_token.as_deref() != Some(global_sec) {
                log_rejection(
                    &state.pool,
                    None,
                    None,
                    &client_ip,
                    &method,
                    &path,
                    &query_params,
                    &header_map,
                    &raw_body,
                    "AUTH_FAILED",
                    "全局密钥校验失败",
                );
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "code": 401,
                        "error": "全局认证密钥校验失败"
                    })),
                );
            }
        }
    }

    // 匹配路由规则
    let rules = state.rules.read().await.clone();
    let mut matched_rule: Option<WebhookRule> = None;
    let mut matched_path_params = HashMap::new();

    for rule in &rules {
        if !rule.enabled {
            continue;
        }

        // 1. Method 匹配
        if rule.method != "ANY" && rule.method != method {
            continue;
        }

        // 2. Path 匹配
        if let Some(params) = match_path(&rule.path_pattern, &path) {
            matched_rule = Some(rule.clone());
            matched_path_params = params;
            break;
        }
    }

    let Some(rule) = matched_rule else {
        log_rejection(
            &state.pool,
            None,
            None,
            &client_ip,
            &method,
            &path,
            &query_params,
            &header_map,
            &raw_body,
            "UNMATCHED",
            "未匹配到任何启用的 WebHook 规则",
        );
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "code": 404,
                "error": "未匹配到任何 WebHook 规则"
            })),
        );
    };

    // 局部规则认证校验
    let auth_ok = match rule.signature_type.as_str() {
        "bearer" => {
            if let Some(ref sec) = rule.secret_token {
                let auth_header = header_map.get("authorization").cloned().unwrap_or_default();
                let expected = format!("Bearer {sec}");
                auth_header == expected
            } else {
                true
            }
        }
        "github_hmac_sha256" => {
            if let Some(ref sec) = rule.secret_token {
                let sig = header_map.get("x-hub-signature-256").cloned().unwrap_or_default();
                verify_github_signature(sec, &raw_bytes, &sig)
            } else {
                true
            }
        }
        "gitee_token" => {
            if let Some(ref sec) = rule.secret_token {
                let token = header_map.get("x-gitee-token").cloned().unwrap_or_default();
                token == *sec
            } else {
                true
            }
        }
        _ => true,
    };

    if !auth_ok {
        log_rejection(
            &state.pool,
            Some(&rule.id),
            Some(&rule.name),
            &client_ip,
            &method,
            &path,
            &query_params,
            &header_map,
            &raw_body,
            "AUTH_FAILED",
            "规则签名/令牌校验失败",
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "code": 401,
                "error": "规则签名或令牌验证不通过"
            })),
        );
    }

    // 组装上下文
    let ctx = WebhookContext {
        method: method.clone(),
        path: path.clone(),
        client_ip: client_ip.clone(),
        path_params: matched_path_params,
        query_params: query_params.clone(),
        headers: header_map.clone(),
        body_json,
        raw_body: raw_body.clone(),
    };

    // 过滤条件判断
    if let Some(ref expr) = rule.filter_expr {
        if !evaluate_filter_expr(expr, &ctx) {
            log_rejection(
                &state.pool,
                Some(&rule.id),
                Some(&rule.name),
                &client_ip,
                &method,
                &path,
                &query_params,
                &header_map,
                &raw_body,
                "FILTER_SKIPPED",
                &format!("不满足过滤表达式: {expr}"),
            );
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "code": 200,
                    "message": "请求已忽略（不满足过滤表达式条件）"
                })),
            );
        }
    }

    // 生成日志 ID 并落库初始状态
    let log_id = uuid::Uuid::new_v4().to_string();
    let query_str = serde_json::to_string(&query_params).unwrap_or_default();
    let headers_json = serde_json::to_string(&header_map).unwrap_or_default();

    let initial_log = WebhookLog {
        id: log_id.clone(),
        rule_id: Some(rule.id.clone()),
        rule_name: Some(rule.name.clone()),
        client_ip,
        method,
        path,
        query_string: Some(query_str),
        request_headers: Some(headers_json),
        request_body: Some(raw_body),
        match_status: "MATCHED".to_string(),
        execution_status: Some("RUNNING".to_string()),
        exit_code: None,
        stdout_tail: None,
        stderr_tail: None,
        duration_ms: None,
        created_at: now_timestamp_ms(),
    };

    if let Ok(conn) = state.pool.get() {
        let _ = store::insert_log(&conn, &initial_log);
    }

    // 同步 / 异步分发
    if rule.response_mode == "sync" {
        // 同步等待执行
        let exec_result = execute_rule(&rule, &ctx, &state.pool).await;
        let elapsed = start_time.elapsed().as_millis() as i64;

        let (status_str, exit_code, stdout, stderr) = match exec_result {
            Ok(out) => {
                let st = if out.exit_code == Some(0) {
                    "SUCCESS"
                } else {
                    "FAILED"
                };
                (st, out.exit_code, out.stdout, out.stderr)
            }
            Err(e) => ("FAILED", Some(1), String::new(), e),
        };

        if let Ok(conn) = state.pool.get() {
            let _ = store::update_log_execution(
                &conn,
                &log_id,
                status_str,
                exit_code,
                Some(&stdout),
                Some(&stderr),
                elapsed,
            );
        }

        let resp_status = if status_str == "SUCCESS" {
            StatusCode::OK
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };

        (
            resp_status,
            Json(serde_json::json!({
                "code": resp_status.as_u16(),
                "logId": log_id,
                "status": status_str,
                "exitCode": exit_code,
                "stdout": stdout,
                "stderr": stderr,
                "durationMs": elapsed
            })),
        )
    } else {
        // 异步响应
        let pool_clone = state.pool.clone();
        let log_id_clone = log_id.clone();
        let rule_clone = rule.clone();
        let ctx_clone = ctx.clone();

        tokio::spawn(async move {
            let start = Instant::now();
            let exec_result = execute_rule(&rule_clone, &ctx_clone, &pool_clone).await;
            let elapsed = start.elapsed().as_millis() as i64;

            let (status_str, exit_code, stdout, stderr) = match exec_result {
                Ok(out) => {
                    let st = if out.exit_code == Some(0) {
                        "SUCCESS"
                    } else {
                        "FAILED"
                    };
                    (st, out.exit_code, out.stdout, out.stderr)
                }
                Err(e) => ("FAILED", Some(1), String::new(), e),
            };

            if let Ok(conn) = pool_clone.get() {
                let _ = store::update_log_execution(
                    &conn,
                    &log_id_clone,
                    status_str,
                    exit_code,
                    Some(&stdout),
                    Some(&stderr),
                    elapsed,
                );
            }
        });

        (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "code": 202,
                "message": "Webhook 已接收并触发后台执行",
                "logId": log_id,
                "rule": rule.name
            })),
        )
    }
}

fn log_rejection(
    pool: &DbPool,
    rule_id: Option<&str>,
    rule_name: Option<&str>,
    client_ip: &str,
    method: &str,
    path: &str,
    query_params: &HashMap<String, String>,
    headers: &HashMap<String, String>,
    raw_body: &str,
    match_status: &str,
    err_msg: &str,
) {
    if let Ok(conn) = pool.get() {
        let query_str = serde_json::to_string(query_params).unwrap_or_default();
        let headers_json = serde_json::to_string(headers).unwrap_or_default();
        let log = WebhookLog {
            id: uuid::Uuid::new_v4().to_string(),
            rule_id: rule_id.map(|s| s.to_string()),
            rule_name: rule_name.map(|s| s.to_string()),
            client_ip: client_ip.to_string(),
            method: method.to_string(),
            path: path.to_string(),
            query_string: Some(query_str),
            request_headers: Some(headers_json),
            request_body: Some(raw_body.to_string()),
            match_status: match_status.to_string(),
            execution_status: Some("FAILED".to_string()),
            exit_code: Some(1),
            stdout_tail: None,
            stderr_tail: Some(err_msg.to_string()),
            duration_ms: Some(0),
            created_at: now_timestamp_ms(),
        };
        let _ = store::insert_log(&conn, &log);
    }
}

/// 构建 Axum 路由
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .fallback(any(handle_webhook))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_path_exact() {
        let res = match_path("/api/deploy", "/api/deploy");
        assert!(res.is_some());
        assert!(res.unwrap().is_empty());

        let res2 = match_path("/api/deploy", "/api/build");
        assert!(res2.is_none());
    }

    #[test]
    fn test_match_path_params() {
        let res = match_path("/api/deploy/:service/:env", "/api/deploy/frontend/prod");
        assert!(res.is_some());
        let params = res.unwrap();
        assert_eq!(params.get("service").unwrap(), "frontend");
        assert_eq!(params.get("env").unwrap(), "prod");
    }

    #[test]
    fn test_match_path_wildcard() {
        let res = match_path("/webhook/*", "/webhook/github/push/main");
        assert!(res.is_some());
    }

    #[test]
    fn test_hex_decode_and_ct_eq() {
        let hex = "0123456789abcdef";
        let bytes = decode_hex(hex).unwrap();
        assert_eq!(bytes, vec![0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]);
        assert!(constant_time_eq(&bytes, &bytes));
    }
}
