//! WebHook 动作执行引擎：本地 Shell 命令、SSH 远端主机命令与 V8/JS 脚本执行（支持多步骤链式流水线）。

use std::collections::HashMap;
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::store::{WebhookActionStep, WebhookRule};
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

/// 链式执行中上一步骤的输出上下文（注入后一步）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrevStepOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub data: Option<Value>,
    pub output: String,
}

/// 单个步骤的执行明细（写入流水线记录与审计流水账）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepExecutionResult {
    pub step_index: usize,
    pub step_id: String,
    pub step_name: String,
    pub action_type: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub data: Option<Value>,
    pub duration_ms: u64,
}

/// 尾部截断（保留最后 4KB，防止海量日志打爆数据库）
fn tail_4kb(s: &str) -> String {
    const MAX_BYTES: usize = 4096;
    if s.len() <= MAX_BYTES {
        s.to_string()
    } else {
        let truncated = &s[s.len() - MAX_BYTES..];
        format!("... [截断，仅保留尾部 4KB]\n{}", clean_utf8_tail(truncated))
    }
}

fn clean_utf8_tail(slice: &str) -> &str {
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

/// 提取上下文构建所有可供插值的变量字典（包含原始请求与链式上一步输出）
pub fn build_variable_map(
    ctx: &WebhookContext,
    prev: Option<&PrevStepOutput>,
) -> HashMap<String, String> {
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

    // 注入上一步链式输出
    if let Some(p) = prev {
        map.insert("prev.stdout".to_string(), p.stdout.clone());
        map.insert("prev.stderr".to_string(), p.stderr.clone());
        map.insert("prev.output".to_string(), p.output.clone());
        map.insert(
            "prev.exit_code".to_string(),
            p.exit_code.map(|c| c.to_string()).unwrap_or_default(),
        );
        map.insert("prev.success".to_string(), p.success.to_string());

        if let Some(ref d) = p.data {
            flatten_json("prev.data", d, &mut map);
        }
    }

    map
}

/// 模板变量插值：替换 `${var}` / `$var` 并输出净化后的环境变量字典
pub fn interpolate_template(
    template: &str,
    ctx: &WebhookContext,
    prev: Option<&PrevStepOutput>,
    is_windows: bool,
) -> (String, HashMap<String, String>) {
    let vars = build_variable_map(ctx, prev);
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

    // 环境变量字典：只保留合法标识符命名，点分属性转下划线（如 body.ref -> VELO_BODY_REF, prev.stdout -> VELO_PREV_STDOUT）
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

/// 执行 WebHook 绑定的动作或链式流水线
pub async fn execute_rule(
    rule: &WebhookRule,
    ctx: &WebhookContext,
    pool: &DbPool,
) -> Result<ExecOutput, String> {
    let steps = rule.get_action_steps();
    if steps.is_empty() {
        return Ok(ExecOutput {
            exit_code: Some(0),
            stdout: "无配置任何动作步骤".to_string(),
            stderr: String::new(),
        });
    }

    let is_pipeline = steps.len() > 1;
    let mut prev: Option<PrevStepOutput> = None;
    let mut history: Vec<StepExecutionResult> = Vec::new();
    let mut combined_stdout = Vec::new();
    let mut combined_stderr = Vec::new();
    let mut final_exit_code = Some(0);

    for (idx, step) in steps.iter().enumerate() {
        let step_res = exec_single_step(step, idx, rule.timeout_secs, ctx, prev.as_ref(), &history, pool).await;

        let is_step_ok = step_res.success;
        let step_exit = step_res.exit_code;
        let step_name = step.name.as_deref().unwrap_or("未命名步骤");

        // 收集输出日志
        if is_pipeline {
            let banner = format!("--- [步骤 {}/{}: {} ({})] ---", idx + 1, steps.len(), step_name, step.action_type);
            combined_stdout.push(banner.clone());
            if !step_res.stdout.is_empty() {
                combined_stdout.push(step_res.stdout.clone());
            }
            if !step_res.stderr.is_empty() {
                combined_stderr.push(format!("{banner}\n{}", step_res.stderr));
            }
        } else {
            if !step_res.stdout.is_empty() {
                combined_stdout.push(step_res.stdout.clone());
            }
            if !step_res.stderr.is_empty() {
                combined_stderr.push(step_res.stderr.clone());
            }
        }

        // 构造下一步 prev
        let combined_output = if step_res.stderr.is_empty() {
            step_res.stdout.clone()
        } else if step_res.stdout.is_empty() {
            step_res.stderr.clone()
        } else {
            format!("{}\n{}", step_res.stdout, step_res.stderr)
        };

        prev = Some(PrevStepOutput {
            stdout: step_res.stdout.clone(),
            stderr: step_res.stderr.clone(),
            exit_code: step_res.exit_code,
            success: step_res.success,
            data: step_res.data.clone(),
            output: combined_output,
        });

        history.push(step_res);

        if !is_step_ok {
            final_exit_code = step_exit.or(Some(1));
            let continue_on_err = step.continue_on_error.unwrap_or(false);
            if !continue_on_err {
                let err_msg = if !combined_stderr.is_empty() {
                    combined_stderr.join("\n")
                } else {
                    format!("步骤 \"{}\" 退出异常 (code={:?})", step_name, step_exit)
                };
                return Err(format!("流水线步骤 #{}: \"{}\" 执行失败并终止: {}", idx + 1, step_name, err_msg));
            }
        }
    }

    Ok(ExecOutput {
        exit_code: final_exit_code,
        stdout: tail_4kb(&combined_stdout.join("\n")),
        stderr: tail_4kb(&combined_stderr.join("\n")),
    })
}

/// 执行单一步骤并计时
async fn exec_single_step(
    step: &WebhookActionStep,
    step_index: usize,
    fallback_timeout_secs: u64,
    ctx: &WebhookContext,
    prev: Option<&PrevStepOutput>,
    history: &[StepExecutionResult],
    pool: &DbPool,
) -> StepExecutionResult {
    let start = Instant::now();
    let timeout_secs = step.timeout_secs.unwrap_or(fallback_timeout_secs).max(1);
    let timeout = Duration::from_secs(timeout_secs);

    let mut result = StepExecutionResult {
        step_index,
        step_id: step.id.clone(),
        step_name: step.name.clone().unwrap_or_else(|| format!("步骤 {}", step_index + 1)),
        action_type: step.action_type.clone(),
        ..Default::default()
    };

    match step.action_type.as_str() {
        "local_command" => {
            match exec_local_step(step, ctx, prev, timeout).await {
                Ok(out) => {
                    result.exit_code = out.exit_code;
                    result.success = out.exit_code == Some(0) || out.exit_code.is_none();
                    result.stdout = out.stdout;
                    result.stderr = out.stderr;
                }
                Err(err) => {
                    result.exit_code = Some(1);
                    result.success = false;
                    result.stderr = err;
                }
            }
        }
        "ssh_command" => {
            match exec_ssh_step(step, ctx, prev, pool, timeout).await {
                Ok(out) => {
                    result.exit_code = out.exit_code;
                    result.success = out.exit_code == Some(0) || out.exit_code.is_none();
                    result.stdout = out.stdout;
                    result.stderr = out.stderr;
                }
                Err(err) => {
                    result.exit_code = Some(1);
                    result.success = false;
                    result.stderr = err;
                }
            }
        }
        "js_script" => {
            match exec_js_step(step, ctx, prev, history, timeout).await {
                Ok((out, data)) => {
                    result.exit_code = out.exit_code;
                    result.success = out.exit_code == Some(0) || out.exit_code.is_none();
                    result.stdout = out.stdout;
                    result.stderr = out.stderr;
                    result.data = data;
                }
                Err(err) => {
                    result.exit_code = Some(1);
                    result.success = false;
                    result.stderr = err;
                }
            }
        }
        other => {
            result.exit_code = Some(1);
            result.success = false;
            result.stderr = format!("不支持的步骤动作类型: {other}");
        }
    }

    result.duration_ms = start.elapsed().as_millis() as u64;
    result
}

/// 本地 Shell 命令执行单步
async fn exec_local_step(
    step: &WebhookActionStep,
    ctx: &WebhookContext,
    prev: Option<&PrevStepOutput>,
    timeout: Duration,
) -> Result<ExecOutput, String> {
    let is_windows = cfg!(target_os = "windows");
    let (cmd_text, env_vars) = interpolate_template(&step.command_text, ctx, prev, is_windows);

    let mut cmd = if is_windows {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(&cmd_text);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(&cmd_text);
        c
    };

    if let Some(ref cwd) = step.cwd {
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

/// 远程 SSH 主机命令执行单步
async fn exec_ssh_step(
    step: &WebhookActionStep,
    ctx: &WebhookContext,
    prev: Option<&PrevStepOutput>,
    pool: &DbPool,
    timeout: Duration,
) -> Result<ExecOutput, String> {
    let host_id = step
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

    let (cmd_text, env_vars) = interpolate_template(&step.command_text, ctx, prev, false);

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

/// JS 脚本执行单步（内嵌 V8 引擎，注入 req/prev/steps 上下文，支持 return 对象注入下一步）
async fn exec_js_step(
    step: &WebhookActionStep,
    ctx: &WebhookContext,
    prev: Option<&PrevStepOutput>,
    history: &[StepExecutionResult],
    timeout: Duration,
) -> Result<(ExecOutput, Option<Value>), String> {
    let ctx_json = serde_json::to_string(ctx).unwrap_or_else(|_| "{}".to_string());
    let prev_json = serde_json::to_string(&prev).unwrap_or_else(|_| "null".to_string());
    let steps_json = serde_json::to_string(&history).unwrap_or_else(|_| "[]".to_string());

    let source = format!(
        "const webhook = Object.freeze({{\n  req: Object.freeze({}),\n  prev: Object.freeze({}),\n  steps: Object.freeze({})\n}});\n{}",
        ctx_json, prev_json, steps_json, step.command_text
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

                let mut data_val: Option<Value> = None;
                if let Some(res) = outcome.result_json {
                    if !res.is_empty() && res != "null" {
                        stdout_lines.push(format!("[return] {res}"));
                        if let Ok(parsed) = serde_json::from_str::<Value>(&res) {
                            data_val = Some(parsed);
                        }
                    }
                }

                Ok((
                    ExecOutput {
                        exit_code: Some(0),
                        stdout: tail_4kb(&stdout_lines.join("\n")),
                        stderr: tail_4kb(&stderr_lines.join("\n")),
                    },
                    data_val,
                ))
            }
            Err(e) => Ok((
                ExecOutput {
                    exit_code: Some(1),
                    stdout: String::new(),
                    stderr: tail_4kb(&e.to_string()),
                },
                None,
            )),
        }
    })
    .await
    .map_err(|e| format!("执行 JS 任务失败: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpolate_prev_step_variables() {
        let ctx = WebhookContext {
            method: "POST".to_string(),
            path: "/deploy/api".to_string(),
            client_ip: "127.0.0.1".to_string(),
            path_params: HashMap::new(),
            query_params: HashMap::new(),
            headers: HashMap::new(),
            body_json: Some(serde_json::json!({
                "service": "order-center"
            })),
            raw_body: r#"{"service":"order-center"}"#.to_string(),
        };

        let prev = PrevStepOutput {
            stdout: "v2.1.0-build.42".to_string(),
            stderr: "".to_string(),
            exit_code: Some(0),
            success: true,
            data: Some(serde_json::json!({
                "version": "2.1.0",
                "tag": "release-v2.1.0"
            })),
            output: "v2.1.0-build.42".to_string(),
        };

        let template = "docker build -t ${body.service}:${prev.stdout} --build-arg TAG=${prev.data.tag} .";
        let (interpolated, env_vars) = interpolate_template(template, &ctx, Some(&prev), false);

        assert_eq!(
            interpolated,
            "docker build -t order-center:v2.1.0-build.42 --build-arg TAG=release-v2.1.0 ."
        );

        assert_eq!(env_vars.get("VELO_PREV_STDOUT").map(|s| s.as_str()), Some("v2.1.0-build.42"));
        assert_eq!(env_vars.get("VELO_PREV_DATA_TAG").map(|s| s.as_str()), Some("release-v2.1.0"));
        assert_eq!(env_vars.get("VELO_BODY_SERVICE").map(|s| s.as_str()), Some("order-center"));
    }
}

