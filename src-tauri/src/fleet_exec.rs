//! 批量执行（fleet exec）：多台 SSH 主机并行跑同一命令，per-host 结果
//! 事件经 Tauri Channel 流式回传（对齐 dbQuery 的 tag=type 模式）。
//!
//! 执行通道全复用既有设施：`resolve_host_chain`（凭据/跳板链/中继解析）
//! → `connect_ssh_chain_headless`（无头建连变体：未知主机公钥按策略
//! 内联代答、认证失败快速失败，见 terminal/ssh/client.rs）→
//! `sftp_exec_capture_status`（带退出码的旁路 exec，SFTP 同款）。每主机
//! 一次独立连接（连上→exec→收完→断开），不占用也不依赖终端会话；
//! 跳板/中继主机自动生效（链解析层透传）。
//!
//! 取消语义：整批一个取消信号（Notify + AtomicBool，db_query_cancel
//! 同型双信号）——排队中不再起跑、执行中的主机 select! 打断（远端命令
//! 随连接断开而终止，与交互终端 Ctrl+C 断连的效果一致）。
//!
//! 已知边界：stdout/stderr 合并展示（不分离）；不sudo 提权（需要提权的
//! 命令请在命令里自带 `sudo -S` 管道）；输出超 256KB/主机截断。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

use crate::sftp::manager::{resolve_host_chain, sftp_exec_capture_status};
use crate::terminal::ssh::client::{connect_ssh_chain_headless, Connection};
use crate::terminal::ssh::known_hosts::KnownHosts;

/// 每主机输出上限（字节）：超限截断并加标记（结果只做展示定位，超大
/// 输出没有增量价值，反而拖垮 Channel 传输与前端渲染）
const OUTPUT_LIMIT_BYTES: usize = 256 * 1024;

/// 批量执行事件（tag=type 判别）。**逐变体补 `#[serde(rename_all)]`**：
/// 枚举级 rename_all 只重命名变体名不吃字段——db_browser 的 QueryEvent
/// 踩过同一坑（elapsed_ms 上线成 elapsedMs 前，前端读到 undefined）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FleetExecEvent {
    #[serde(rename_all = "camelCase")]
    Started { host_count: usize },
    #[serde(rename_all = "camelCase")]
    HostStart { host_id: usize },
    #[serde(rename_all = "camelCase")]
    HostDone {
        host_id: usize,
        /// None = 服务器没发 ExitStatus 就关通道（受限 shell 形态）
        exit_code: Option<i32>,
        elapsed_ms: u64,
        output: String,
        truncated: bool,
    },
    #[serde(rename_all = "camelCase")]
    HostError {
        host_id: usize,
        message: String,
        elapsed_ms: u64,
    },
    #[serde(rename_all = "camelCase")]
    Done { cancelled: bool, elapsed_ms: u64 },
}

/// 一批的汇总（命令返回值）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetExecSummary {
    pub total: usize,
    /// 退出码 == 0 的主机数
    pub succeeded: usize,
    /// 非零退出 / 连接失败 / 被取消
    pub failed: usize,
    pub cancelled: bool,
}

/// 一次运行的控制句柄
struct FleetRun {
    cancel: Arc<tokio::sync::Notify>,
    cancelled: Arc<AtomicBool>,
}

/// 运行表（tauri manage）：runId 前端生成（时间戳 + 序号），运行结束
/// 自动移除——`fleet_exec_cancel` 对不存在的 runId 返回 false（已结束）。
#[derive(Default)]
pub struct FleetExecState {
    runs: Mutex<HashMap<String, Arc<FleetRun>>>,
}

/// 运行表守卫：函数任何出口（含 `?` 早退）都清掉登记，防泄漏
struct RunGuard<'a>(&'a FleetExecState, String);

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        self.0.runs.lock().unwrap().remove(&self.1);
    }
}

/// 输出截断（UTF-8 字符边界安全；工程 clippy 禁裸切片故走 get）
fn truncate_output(raw: &str) -> (String, bool) {
    if raw.len() <= OUTPUT_LIMIT_BYTES {
        return (raw.to_string(), false);
    }
    let mut end = OUTPUT_LIMIT_BYTES;
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    let head = raw.get(..end).unwrap_or("");
    (
        format!(
            "[输出超过 {} KB 已截断，完整内容请重定向到文件]\n{}",
            OUTPUT_LIMIT_BYTES / 1024,
            head
        ),
        true,
    )
}

/// 单主机执行：解析链 → 无头建连 → 排空事件通道 → exec 收输出与退出码
/// → 断开。连接与所有句柄随函数返回 Drop（drainer 显式 abort）。
async fn exec_on_host(
    host_id: usize,
    command: &str,
    timeout_secs: u64,
) -> Result<(String, Option<u32>), String> {
    let chain = resolve_host_chain(host_id).map_err(|e| format!("主机链解析失败：{e}"))?;
    let connection = connect_ssh_chain_headless(host_id, chain, KnownHosts::global())
        .await
        .map_err(|e| format!("连接失败：{e}"))?;
    let Connection(handle, mut handler_rx) = connection;
    // 排空 ClientHandlerEvent（russh 状态机要求有人消费；SFTP 同型）
    let drainer = tokio::spawn(async move {
        while let Some(_event) = handler_rx.recv().await {}
    });
    let result = sftp_exec_capture_status(&handle, command, timeout_secs).await;
    drainer.abort();
    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "batch done", "en")
        .await;
    result
}

/// 批量执行入口。runId 由前端生成传入（取消命令按它寻址）。
#[tauri::command]
pub async fn fleet_exec_run(
    state: State<'_, FleetExecState>,
    run_id: String,
    host_ids: Vec<usize>,
    command: String,
    timeout_secs: Option<u64>,
    concurrency: Option<usize>,
    on_event: Channel<FleetExecEvent>,
) -> Result<FleetExecSummary, String> {
    let command = command.trim().to_string();
    if command.is_empty() {
        return Err("命令为空".to_string());
    }
    if host_ids.is_empty() {
        return Err("未选择主机".to_string());
    }
    // 去重保序
    let mut seen = std::collections::HashSet::new();
    let host_ids: Vec<usize> = host_ids
        .into_iter()
        .filter(|id| seen.insert(*id))
        .collect();
    let timeout_secs = timeout_secs.unwrap_or(30).clamp(1, 600);
    let concurrency = concurrency.unwrap_or(8).clamp(1, 32);

    let run = Arc::new(FleetRun {
        cancel: Arc::new(tokio::sync::Notify::new()),
        cancelled: Arc::new(AtomicBool::new(false)),
    });
    state
        .runs
        .lock()
        .unwrap()
        .insert(run_id.clone(), run.clone());
    let _guard = RunGuard(&state, run_id);

    let _ = on_event.send(FleetExecEvent::Started {
        host_count: host_ids.len(),
    });
    let started = std::time::Instant::now();

    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut tasks = Vec::with_capacity(host_ids.len());
    for host_id in host_ids {
        let semaphore = Arc::clone(&semaphore);
        let run = Arc::clone(&run);
        let on_event = on_event.clone();
        let command = command.clone();
        tasks.push(tokio::spawn(async move {
            // 并发闸门：排队中也响应取消（select 竞争 permit 与 cancel）
            let permit = tokio::select! {
                _ = run.cancel.notified() => {
                    return (host_id, false, true);
                }
                permit = semaphore.acquire() => permit,
            };
            let _permit = permit;
            if run.cancelled.load(Ordering::Relaxed) {
                return (host_id, false, true);
            }
            let _ = on_event.send(FleetExecEvent::HostStart { host_id });
            let t0 = std::time::Instant::now();
            // 执行期取消：select 打断 in-flight 的连接/exec（future 被 drop，
            // 连接句柄随之断开，远端命令终止）
            let outcome = tokio::select! {
                _ = run.cancel.notified() => Err("已取消".to_string()),
                result = exec_on_host(host_id, &command, timeout_secs) => result,
            };
            let elapsed_ms = t0.elapsed().as_millis() as u64;
            match outcome {
                Ok((raw_output, exit_code)) => {
                    let (output, truncated) = truncate_output(&raw_output);
                    let _ = on_event.send(FleetExecEvent::HostDone {
                        host_id,
                        exit_code: exit_code.map(|c| c as i32),
                        elapsed_ms,
                        output,
                        truncated,
                    });
                    (host_id, exit_code == Some(0), false)
                }
                Err(message) => {
                    let _ = on_event.send(FleetExecEvent::HostError {
                        host_id,
                        message,
                        elapsed_ms,
                    });
                    (host_id, false, run.cancelled.load(Ordering::Relaxed))
                }
            }
        }));
    }

    let mut succeeded = 0usize;
    let mut failed = 0usize;
    let mut any_cancelled = false;
    for task in tasks {
        if let Ok((_host_id, ok, was_cancelled)) = task.await {
            if ok {
                succeeded += 1;
            } else {
                failed += 1;
            }
            if was_cancelled {
                any_cancelled = true;
            }
        }
    }
    let cancelled = any_cancelled || run.cancelled.load(Ordering::Relaxed);
    let summary = FleetExecSummary {
        total: succeeded + failed,
        succeeded,
        failed,
        cancelled,
    };
    let _ = on_event.send(FleetExecEvent::Done {
        cancelled,
        elapsed_ms: started.elapsed().as_millis() as u64,
    });
    Ok(summary)
}

/// 取消一批运行：置标志 + 唤醒所有等待者。runId 不存在（已结束）返回
/// false 而非报错——前端取消按钮天然晚于完成事件的竞态不算错。
#[tauri::command]
pub async fn fleet_exec_cancel(
    state: State<'_, FleetExecState>,
    run_id: String,
) -> Result<bool, String> {
    let run = state.runs.lock().unwrap().get(&run_id).cloned();
    match run {
        Some(run) => {
            run.cancelled.store(true, Ordering::Relaxed);
            run.cancel.notify_waiters();
            Ok(true)
        }
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 事件 serde 判别形态：tag=type 小驼峰变体名 + 字段 camelCase
    /// （hostId/exitCode/elapsedMs/hostCount——枚举级 rename 不吃字段，
    /// 逐变体属性是唯一可靠写法，此测试锁住该契约）
    #[test]
    fn fleet_event_serde_camel_case() {
        let v = serde_json::to_value(FleetExecEvent::Started { host_count: 3 }).unwrap();
        assert_eq!(v.get("type").and_then(serde_json::Value::as_str), Some("started"));
        assert_eq!(v.get("hostCount").and_then(serde_json::Value::as_i64), Some(3));

        let v = serde_json::to_value(FleetExecEvent::HostStart { host_id: 7 }).unwrap();
        assert_eq!(v.get("type").and_then(serde_json::Value::as_str), Some("hostStart"));
        assert_eq!(v.get("hostId").and_then(serde_json::Value::as_i64), Some(7));

        let v = serde_json::to_value(FleetExecEvent::HostDone {
            host_id: 9,
            exit_code: Some(2),
            elapsed_ms: 1234,
            output: "out".to_string(),
            truncated: false,
        })
        .unwrap();
        assert_eq!(v.get("type").and_then(serde_json::Value::as_str), Some("hostDone"));
        assert_eq!(v.get("hostId").and_then(serde_json::Value::as_i64), Some(9));
        assert_eq!(v.get("exitCode").and_then(serde_json::Value::as_i64), Some(2));
        assert_eq!(v.get("elapsedMs").and_then(serde_json::Value::as_i64), Some(1234));

        let v = serde_json::to_value(FleetExecEvent::HostError {
            host_id: 5,
            message: "连接失败".to_string(),
            elapsed_ms: 88,
        })
        .unwrap();
        assert_eq!(v.get("type").and_then(serde_json::Value::as_str), Some("hostError"));
        assert_eq!(
            v.get("message").and_then(serde_json::Value::as_str),
            Some("连接失败")
        );

        let v = serde_json::to_value(FleetExecEvent::Done {
            cancelled: true,
            elapsed_ms: 5000,
        })
        .unwrap();
        assert_eq!(v.get("type").and_then(serde_json::Value::as_str), Some("done"));
        assert_eq!(v.get("cancelled").and_then(serde_json::Value::as_bool), Some(true));
    }

    /// 输出截断：短输出原样不标记；超限输出带标记且截断点在 UTF-8
    /// 字符边界上（中文多字节序列中间不撕开）
    #[test]
    fn fleet_output_truncation() {
        let (out, truncated) = truncate_output("hello");
        assert_eq!(out, "hello");
        assert!(!truncated);

        let big = "中".repeat(OUTPUT_LIMIT_BYTES); // 每字 3 字节，必然超限
        let (out, truncated) = truncate_output(&big);
        assert!(truncated);
        assert!(out.contains("已截断"));
        // 截断头必须是合法 UTF-8（from_utf8 不报错即边界正确）
        assert!(out.contains(&"中".to_string()));

        // 恰好等于上限：不截断
        let exact = "a".repeat(OUTPUT_LIMIT_BYTES);
        let (_, truncated) = truncate_output(&exact);
        assert!(!truncated);
    }

    /// 汇总结构 camelCase
    #[test]
    fn fleet_summary_serde() {
        let v = serde_json::to_value(FleetExecSummary {
            total: 5,
            succeeded: 4,
            failed: 1,
            cancelled: false,
        })
        .unwrap();
        assert_eq!(v.get("total").and_then(serde_json::Value::as_i64), Some(5));
        assert_eq!(v.get("succeeded").and_then(serde_json::Value::as_i64), Some(4));
        assert_eq!(v.get("failed").and_then(serde_json::Value::as_i64), Some(1));
    }
}
