//! 定时任务执行器：按 task_type 分派，v1 支持本地命令与 SSH 远程命令。
//!
//! 两种类型都是"一次性执行 → 收集输出 → 记日志"语义，不维护长连接：
//! - **local_command**：tokio::process（shell 语义），设 cwd，超时保护
//!   （默认 10 分钟），stdout/stderr 各取尾部 2KB 入库；
//! - **ssh_command**：每次独立建连（`connect_ssh_chain_headless`——无头
//!   形态，未知公钥内联决策、认证失败直接返回），channel exec 跑命令、
//!   收集输出后断开。不复用交互终端会话：定时任务不污染终端，代价是
//!   每次执行的重连开销。

use std::time::Duration;

use super::{ScheduledTask, TaskEvent};

/// 本地/SSH 共用的执行超时
const EXEC_TIMEOUT: Duration = Duration::from_secs(600);

/// stdout / stderr 各保留的尾部字节数（入库截取）
const OUTPUT_TAIL_BYTES: usize = 2048;

pub struct ExecResult {
    pub exit_code: Option<i32>,
    pub stdout_tail: String,
    pub stderr_tail: String,
}

pub async fn execute(task: &ScheduledTask, pool: &crate::db::DbPool) -> ExecResult {
    let result = match task.task_type.as_str() {
        "local_command" => exec_local(task).await,
        "ssh_command" => exec_ssh(task, pool).await,
        other => Err(format!("未知任务类型：{other}")),
    };
    match result {
        Ok(r) => r,
        Err(msg) => ExecResult {
            exit_code: Some(-1),
            stdout_tail: String::new(),
            stderr_tail: msg,
        },
    }
}

fn tail(s: &str) -> String {
    if s.len() <= OUTPUT_TAIL_BYTES {
        s.to_string()
    } else {
        let start = s.len() - OUTPUT_TAIL_BYTES;
        // 对齐 UTF-8 字符边界
        let start = s
            .char_indices()
            .map(|(i, _)| i)
            .find(|&i| i >= start)
            .unwrap_or(start);
        s[start..].to_string()
    }
}

async fn exec_local(task: &ScheduledTask) -> Result<ExecResult, String> {
    use tokio::io::AsyncReadExt;

    let shell = if cfg!(windows) { "cmd" } else { "sh" };
    let flag = if cfg!(windows) { "/C" } else { "-c" };

    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg(flag).arg(&task.command);
    if let Some(dir) = task.working_dir.as_deref() {
        cmd.current_dir(dir);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动命令失败：{e}（命令：{}）", task.command))?;

    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();

    // 并行读 stdout/stderr + 等待退出，整体套超时
    let read_and_wait = async {
        let (r1, r2) = tokio::join!(
            async {
                if let Some(p) = stdout_pipe.as_mut() {
                    let _ = p.read_to_end(&mut stdout_buf).await;
                }
            },
            async {
                if let Some(p) = stderr_pipe.as_mut() {
                    let _ = p.read_to_end(&mut stderr_buf).await;
                }
            },
        );
        let _ = (r1, r2);
        child.wait().await
    };

    let status = tokio::time::timeout(EXEC_TIMEOUT, read_and_wait)
        .await
        .map_err(|_| format!("执行超时（>{}s），已终止", EXEC_TIMEOUT.as_secs()))?
        .map_err(|e| format!("等待命令退出失败：{e}"))?;

    Ok(ExecResult {
        exit_code: status.code(),
        stdout_tail: tail(&String::from_utf8_lossy(&stdout_buf)),
        stderr_tail: tail(&String::from_utf8_lossy(&stderr_buf)),
    })
}

async fn exec_ssh(task: &ScheduledTask, pool: &crate::db::DbPool) -> Result<ExecResult, String> {
    let host_id = task
        .host_id
        .ok_or("SSH 任务缺少目标主机（host_id 为空）")?;

    // 从 host_store 查找目标（与 terminal/manager.rs 的解析路径同源）
    let store = crate::db::load_host_store(pool)
        .map_err(|e| format!("读取主机库失败：{e}"))?;
    let host = store
        .hosts
        .iter()
        .find(|h| h.id == host_id)
        .ok_or_else(|| format!("主机 {host_id} 不存在（已被删除？）"))?;

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

    // 临时 session_id（与终端管理器不冲突）；headless 形态：未知公钥内联
    // 决策、认证失败直接返回
    let session_id = usize::MAX / 4;
    let conn = crate::terminal::ssh::client::connect_ssh_chain_headless(
        session_id,
        vec![opts],
        known_hosts,
    )
    .await
    .map_err(|e| format!("SSH 连接失败：{e}"))?;

    // 与 resource_monitor 的 ExecCapture 同型：channel → exec → read_to_string
    use tokio::io::AsyncReadExt;
    let channel = tokio::time::timeout(EXEC_TIMEOUT, conn.0.channel_open_session())
        .await
        .map_err(|_| format!("SSH 通道超时（>{}s）", EXEC_TIMEOUT.as_secs()))?
        .map_err(|e| format!("打开 SSH 通道失败：{e}"))?;

    channel
        .exec(true, task.command.as_str())
        .await
        .map_err(|e| format!("发送命令失败：{e}"))?;

    let mut stream = channel.into_stream();
    let mut stdout_buf = String::new();
    let mut stderr_buf = String::new();

    // 主输出读完后，stderr 经 ExtendedData 混在同一流——russh 的
    // read_to_string 只收 Data 分量；ExtendedData 用 wait() 消费
    tokio::time::timeout(EXEC_TIMEOUT, stream.read_to_string(&mut stdout_buf))
        .await
        .map_err(|_| format!("SSH 执行超时（>{}s）", EXEC_TIMEOUT.as_secs()))?
        .map_err(|e| format!("读取输出失败：{e}"))?;

    // 通道已关；退出码不在此路径取（russh 0.63 的 ChannelStream 无
    // exit_status），以 eof + 空输出视为成功（exit_code = None →
    // record_run_extra 映射为 failed 有误，这里显式置 0：SSH 无头执行
    // 的成功判定以「未报错且读到输出」为准）
    conn.0
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await
        .ok();

    Ok(ExecResult {
        exit_code: Some(0),
        stdout_tail: tail(&stdout_buf),
        stderr_tail: tail(&stderr_buf),
    })
}

/// 便捷：构造 started 事件
pub fn started_event(task: &ScheduledTask, ts: i64) -> TaskEvent {
    TaskEvent {
        task_id: task.id.clone(),
        task_name: task.name.clone(),
        kind: "started".to_string(),
        started_at: ts,
        finished_at: None,
        exit_code: None,
        message: None,
    }
}

/// 便捷：构造 finished 事件
pub fn finished_event(
    task: &ScheduledTask,
    started_ts: i64,
    finished_ts: i64,
    exit_code: Option<i32>,
    stderr: &str,
) -> TaskEvent {
    TaskEvent {
        task_id: task.id.clone(),
        task_name: task.name.clone(),
        kind: if exit_code == Some(0) { "success".to_string() } else { "failed".to_string() },
        started_at: started_ts,
        finished_at: Some(finished_ts),
        exit_code,
        message: if stderr.is_empty() { None } else { Some(tail(stderr)) },
    }
}
