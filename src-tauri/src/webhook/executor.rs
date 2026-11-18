//! WebHook 动作执行引擎：本地 Shell 命令、SSH 远端主机命令与 V8/JS 脚本执行。

use std::collections::HashMap;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::store::WebhookRule;
use crate::db::DbPool;

/// 单次请求上下文（用于模板变量插值与 JS 脚本注入）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookContext {
    pub method: String,
    pub path: String,
    pub client_ip: String,
    pub path_params: HashMap<String, String>,
    pub query_params: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body_json: Option<Value>,
    pub raw_body: String,
}

#[derive(Debug, Clone, Default)]
pub struct ExecOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// 尾部截断（保留最后 4KB，防止海量日志打爆数据库）
fn tail_4kb(s: &str) -> String {
    const MAX_BYTES: usize = 4096;
    if s.len() <= MAX_BYTES {
        s.to_string()
    } else {
        let truncated = &s[s.len() - MAX_BYTES..];
        // 确保 UTF-8 边界安全
        format!("... [截断，仅保留尾部 4KB]\n{}", clean_utf8_tail(truncated))
    }
}

fn clean_utf8_tail(slice: &str) -> &str {
    // Rust 的 &str 本身保证合法 UTF-8，但如果是从字节切片截断需保证字符边界
    slice
}

/// 递归将 JSON 展平为点分属性（如 body.repository.name, body.ref）
fn flatten_json(prefix: &str, val: &Value, map: &mut HashMap<String, String>) {
    match val {
        Value::Object(obj) => {
            for (k, v) in obj {
                let next_prefix = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_json(&next_prefix, v, map);
            }
        }
        Value::Array(arr) => {
            for (idx, v) in arr.iter().enumerate() {
                let next_prefix = format!("{prefix}[{idx}]");
                flatten_json(&next_prefix, v, map);
            }
        }
        Value::String(s) => {
            map.insert(prefix.to_string(), s.clone());
        }
        Value::Number(n) => {
            map.insert(prefix.to_string(), n.to_string());
        }
        Value::Bool(b) => {
            map.insert(prefix.to_string(), b.to_string());
        }
        Value::Null => {
            map.insert(prefix.to_string(), String::new());
        }
    }
}

/// 提取上下文构建所有可供插值的变量字典
pub fn build_variable_map(ctx: &WebhookContext) -> HashMap<String, String> {
    let mut map = HashMap::new();

    map.insert("method".to_string(), ctx.method.clone());
    map.insert("path".to_string(), ctx.path.clone());
    map.insert("client_ip".to_string(), ctx.client_ip.clone());
    map.insert("raw_body".to_string(), ctx.raw_body.clone());

    for (k, v) in &ctx.path_params {
        map.insert(format!("path.{k}"), v.clone());
    }

    for (k, v) in &ctx.query_params {
        map.insert(format!("query.{k}"), v.clone());
    }

    for (k, v) in &ctx.headers {
        map.insert(format!("header.{}", k.to_lowercase()), v.clone());
    }

    if let Some(ref json) = ctx.body_json {
        flatten_json("body", json, &mut map);
    }

    map
}

/// 模板变量插值：替换 `${var}` / `$var` 并输出净化后的环境变量字典
pub fn interpolate_template(
    template: &str,
    ctx: &WebhookContext,
    is_windows: bool,
) -> (String, HashMap<String, String>) {
    let vars = build_variable_map(ctx);
    let mut final_cmd = template.to_string();

    // 按 key 长度降序排序替换，避免前缀被覆盖（如 body.ref_name 优先于 body.ref）
    let mut sorted_keys: Vec<&String> = vars.keys().collect();
    sorted_keys.sort_by_key(|k| std::cmp::Reverse(k.len()));

    for k in sorted_keys {
        if let Some(v) = vars.get(k) {
            // ${var}
            final_cmd = final_cmd.replace(&format!("${{{k}}}"), v);
            // $var
            final_cmd = final_cmd.replace(&format!("${k}"), v);
            // Windows %VAR% 兼容
            if is_windows {
                let win_k = k.replace('.', "_").replace('[', "_").replace(']', "");
                final_cmd = final_cmd.replace(&format!("%{win_k}%"), v);
                final_cmd = final_cmd.replace(&format!("%{k}%"), v);
            }
        }
    }

    // 环境变量字典：只保留合法标识符命名，点分属性转下划线（如 body.ref -> VELO_BODY_REF）
    let mut env_map = HashMap::new();
    for (k, v) in vars {
        let safe_key = format!(
            "VELO_{}",
            k.replace('.', "_")
                .replace('[', "_")
                .replace(']', "")
                .to_uppercase()
        );
        env_map.insert(safe_key, v);
    }

    (final_cmd, env_map)
}

/// 执行 WebHook 绑定的动作
pub async fn execute_rule(
    rule: &WebhookRule,
    ctx: &WebhookContext,
    pool: &DbPool,
) -> Result<ExecOutput, String> {
    let timeout = Duration::from_secs(rule.timeout_secs.max(1));

    match rule.action_type.as_str() {
        "local_command" => exec_local(rule, ctx, timeout).await,
        "ssh_command" => exec_ssh(rule, ctx, pool, timeout).await,
        "js_script" => exec_js(rule, ctx, timeout).await,
        other => Err(format!("不支持的动作类型: {other}")),
    }
}

/// 本地 Shell 命令执行
async fn exec_local(
    rule: &WebhookRule,
    ctx: &WebhookContext,
    timeout: Duration,
) -> Result<ExecOutput, String> {
    let is_windows = cfg!(target_os = "windows");
    let (cmd_text, env_vars) = interpolate_template(&rule.command_text, ctx, is_windows);

    let mut cmd = if is_windows {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(&cmd_text);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(&cmd_text);
        c
    };

    if let Some(ref cwd) = rule.cwd {
        if !cwd.trim().is_empty() {
            cmd.current_dir(cwd);
        }
    }

    for (k, v) in env_vars {
        cmd.env(k, v);
    }

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("启动本地命令失败: {e}"))?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let read_and_wait = async {
        use tokio::io::AsyncReadExt;
        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();

        let (r1, r2) = tokio::join!(
            async {
                if let Some(mut p) = stdout_pipe {
                    let _ = p.read_to_end(&mut stdout_buf).await;
                }
            },
            async {
                if let Some(mut p) = stderr_pipe {
                    let _ = p.read_to_end(&mut stderr_buf).await;
                }
            }
        );
        let _ = (r1, r2);
        child.wait().await.map(|status| (status, stdout_buf, stderr_buf))
    };

    let (status, stdout_bytes, stderr_bytes) = tokio::time::timeout(timeout, read_and_wait)
        .await
        .map_err(|_| format!("本地命令执行超时（>{}s）", timeout.as_secs()))?
        .map_err(|e| format!("等待命令退出失败: {e}"))?;

    let stdout_str = String::from_utf8_lossy(&stdout_bytes);
    let stderr_str = String::from_utf8_lossy(&stderr_bytes);

    Ok(ExecOutput {
        exit_code: status.code(),
        stdout: tail_4kb(&stdout_str),
        stderr: tail_4kb(&stderr_str),
    })
}

/// 远程 SSH 主机命令执行
async fn exec_ssh(
    rule: &WebhookRule,
    ctx: &WebhookContext,
    pool: &DbPool,
    timeout: Duration,
) -> Result<ExecOutput, String> {
    let host_id = rule
        .target_host_id
        .ok_or_else(|| "SSH 动作缺少目标主机 (target_host_id 为空)".to_string())?;

    let store = crate::db::load_host_store(pool).map_err(|e| format!("读取主机库失败: {e}"))?;
    let host = store
        .hosts
        .iter()
        .find(|h| h.id == host_id as usize)
        .ok_or_else(|| format!("目标主机 #{host_id} 不存在（已被删除）"))?;

    let opts = crate::terminal::ssh::TargetSSHOptions {
        host: host.host.clone(),
        port: host.port,
        username: host.username.clone(),
        allow_insecure_algos: host.allow_insecure_algos,
        auth: host.auth.clone(),
        jump_host: None,
        host_key_verification: host.host_key_verification,
        relay: None,
    };

    let known_hosts = crate::terminal::ssh::known_hosts::KnownHosts::global();
    let session_id = usize::MAX / 5;

    let conn = crate::terminal::ssh::client::connect_ssh_chain_headless(
        session_id,
        vec![opts],
        known_hosts,
    )
    .await
    .map_err(|e| format!("SSH 连接失败: {e}"))?;

    use tokio::io::AsyncReadExt;
    let channel = tokio::time::timeout(timeout, conn.0.channel_open_session())
        .await
        .map_err(|_| format!("SSH 打开通道超时（>{}s）", timeout.as_secs()))?
        .map_err(|e| format!("打开 SSH 通道失败: {e}"))?;

    let (cmd_text, env_vars) = interpolate_template(&rule.command_text, ctx, false);

    // 在 Unix 环境下注入 export VAR='VAL'
    let mut export_prefix = String::new();
    for (k, v) in env_vars {
        let escaped_v = v.replace('\'', "'\\''");
        export_prefix.push_str(&format!("export {k}='{escaped_v}'; "));
    }
    let command_to_run = format!("{export_prefix}{cmd_text}");

    channel
        .exec(true, command_to_run.as_str())
        .await
        .map_err(|e| format!("发送 SSH 命令失败: {e}"))?;

    let mut stream = channel.into_stream();
    let mut stdout_buf = String::new();

    tokio::time::timeout(timeout, stream.read_to_string(&mut stdout_buf))
        .await
        .map_err(|_| format!("SSH 命令执行超时（>{}s）", timeout.as_secs()))?
        .map_err(|e| format!("读取 SSH 输出失败: {e}"))?;

    conn.0
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await
        .ok();

    Ok(ExecOutput {
        exit_code: Some(0),
        stdout: tail_4kb(&stdout_buf),
        stderr: String::new(),
    })
}

/// JS 脚本执行（内嵌 V8 引擎，注入 webhook 上下文）
async fn exec_js(
    rule: &WebhookRule,
    ctx: &WebhookContext,
    timeout: Duration,
) -> Result<ExecOutput, String> {
    let ctx_json = serde_json::to_string(ctx).unwrap_or_else(|_| "{}".to_string());
    let source = format!(
        "const webhook = Object.freeze({{ req: Object.freeze({}) }});\n{}",
        ctx_json, rule.command_text
    );

    let is_ts = source.contains("interface ")
        || source.contains("type ")
        || source.contains("enum ")
        || source.contains(": number")
        || source.contains(": string")
        || source.contains(": boolean");

    let language = if is_ts {
        crate::js_runtime::JsLanguage::Typescript
    } else {
        crate::js_runtime::JsLanguage::Javascript
    };

    tokio::task::spawn_blocking(move || {
        let options = crate::js_runtime::JsRunOptions {
            timeout,
            heap_limit_mb: crate::js_runtime::DEFAULT_HEAP_LIMIT_MB,
            language,
        };

        match crate::js_runtime::run_script(&source, &options) {
            Ok(outcome) => {
                let mut stdout_lines = Vec::new();
                let mut stderr_lines = Vec::new();
                for line in outcome.console {
                    match line.level {
                        crate::js_runtime::ConsoleLevel::Error => stderr_lines.push(line.text),
                        _ => stdout_lines.push(line.text),
                    }
                }
                if let Some(res) = outcome.result_json {
                    if !res.is_empty() && res != "null" {
                        stdout_lines.push(format!("[return] {res}"));
                    }
                }
                Ok(ExecOutput {
                    exit_code: Some(0),
                    stdout: tail_4kb(&stdout_lines.join("\n")),
                    stderr: tail_4kb(&stderr_lines.join("\n")),
                })
            }
            Err(e) => Ok(ExecOutput {
                exit_code: Some(1),
                stdout: String::new(),
                stderr: tail_4kb(&e.to_string()),
            }),
        }
    })
    .await
    .map_err(|e| format!("执行 JS 任务失败: {e}"))?
}
