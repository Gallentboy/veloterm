use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::types::{SftpClient, SftpSshHandle, SftpState};
use tokio::io::AsyncReadExt as _;
use crate::terminal::ssh::{KnownHosts, TargetSSHOptions, connect_ssh_chain};

fn record_to_ssh_options(record: &crate::db::HostRecord) -> TargetSSHOptions {
    TargetSSHOptions {
        host: record.host.clone(),
        port: record.port,
        username: if record.username.trim().is_empty() {
            "root".to_string()
        } else {
            record.username.clone()
        },
        allow_insecure_algos: record.allow_insecure_algos,
        auth: record.auth.clone(),
        jump_host: None,
        host_key_verification: record.host_key_verification,
        // SFTP 连接同样支持经中继（与终端/端口转发同型：嵌入式优先，relay_id 兜底）
        relay: record
            .relay
            .clone()
            .or_else(|| crate::relays::endpoint_by_id(record.relay_id)),
    }
}

pub fn resolve_host_chain(host_id: usize) -> Result<Vec<TargetSSHOptions>, String> {
    let pool =
        crate::db::global_pool().ok_or_else(|| "Database connection pool not ready".to_string())?;
    let db_store =
        crate::db::load_host_store(&pool).map_err(|e| format!("Failed to load host store: {e}"))?;

    let target_record = db_store
        .hosts
        .iter()
        .find(|h| h.id == host_id)
        .cloned()
        .ok_or_else(|| format!("Host not found in database (id={host_id})"))?;

    let by_id: HashMap<usize, &crate::db::HostRecord> =
        db_store.hosts.iter().map(|h| (h.id, h)).collect();

    let mut hops = Vec::new();
    let mut visited = HashSet::from([target_record.id]);
    let mut cursor = target_record.jump_host_id;

    while let Some(jid) = cursor {
        if !visited.insert(jid) {
            return Err(format!("SSH jump host cycle detected at host id {jid}"));
        }
        let Some(hop) = by_id.get(&jid) else {
            return Err(format!("SSH jump host id {jid} not found"));
        };
        hops.insert(0, record_to_ssh_options(hop));
        cursor = hop.jump_host_id;
    }

    hops.push(record_to_ssh_options(&target_record));
    Ok(hops)
}

pub async fn connect_sftp(chain: Vec<TargetSSHOptions>) -> Result<(SftpClient, std::sync::Arc<SftpSshHandle>), String> {
    let known_hosts = KnownHosts::global();
    let session_id = 0; // 内部连接序号
    let connection = connect_ssh_chain(session_id, chain, known_hosts)
        .await
        .map_err(|e| format!("SSH connection failed: {e}"))?;

    let session = Arc::new(connection.0);
    let mut active_rx = connection.1;

    // 保持连接事件接收协程，防止未消费事件阻塞 SSH 状态机
    tokio::spawn(async move { while let Some(_event) = active_rx.recv().await {} });

    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| format!("Failed to open SSH channel for SFTP: {e}"))?;

    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| format!("Failed to request 'sftp' subsystem: {e}"))?;

    let stream = channel.into_stream();
    // 并发在途写请求数提升至 32（库默认 8）：高延迟链路下填满写流水线，
    // 配合 512KB 单块 write_all 一次可压入 ~16×32KB 写包，显著提升上传/复制吞吐。
    let sftp_config = russh_sftp::client::Config {
        max_concurrent_writes: crate::sftp::transfer::SFTP_MAX_CONCURRENT_WRITES,
        ..Default::default()
    };
    let sftp_session = russh_sftp::client::SftpSession::new_with_config(stream, sftp_config)
        .await
        .map_err(|e| format!("Failed to initialize SFTP session: {e}"))?;

    Ok((Arc::new(sftp_session), session))
}

/// 在 SFTP 连接的 SSH 句柄上开独立 exec 通道执行一次性命令并捕获输出
/// （同型先例：终端侧 RCCommand::ExecCapture；不占 sftp 子系统通道）。
/// 超时与调用方自定：du 大目录允许长跑，getent 秒回。
pub async fn sftp_exec_capture(
    ssh: &SftpSshHandle,
    command: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    let channel = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        ssh.channel_open_session(),
    )
    .await
    .map_err(|_| "旁路命令超时：打开通道失败".to_string())?
    .map_err(|e| format!("打开通道失败: {e}"))?;
    channel
        .exec(true, command)
        .await
        .map_err(|e| format!("执行旁路命令失败: {e}"))?;
    let mut stream = channel.into_stream();
    let mut output = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        stream.read_to_string(&mut output),
    )
    .await
    .map_err(|_| "旁路命令超时：读取输出失败".to_string())?
    .map_err(|e| format!("读取旁路命令输出失败: {e}"))?;
    Ok(output)
}

/// sftp_exec_capture 的退出码变体：用 channel.wait() 循环收
/// Data/ExtendedData/ExitStatus（模式对齐终端侧 channel_session），通道关闭
/// 或收到 Close 后返回 (stdout+stderr 合并输出, 退出码)。退出码缺省 None
/// （服务器没发 ExitStatus 就关通道的受限 shell 形态）。
/// 消费方：rm -rf 极速删除——必须以退出码==0 判成功，不能只看输出。
pub async fn sftp_exec_capture_status(
    ssh: &SftpSshHandle,
    command: &str,
    timeout_secs: u64,
) -> Result<(String, Option<u32>), String> {
    let channel = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        ssh.channel_open_session(),
    )
    .await
    .map_err(|_| "旁路命令超时：打开通道失败".to_string())?
    .map_err(|e| format!("打开通道失败: {e}"))?;
    channel
        .exec(true, command)
        .await
        .map_err(|e| format!("执行旁路命令失败: {e}"))?;
    let mut channel = channel;
    let mut output = String::new();
    let mut exit_code: Option<u32> = None;
    let deadline = tokio::time::Duration::from_secs(timeout_secs);
    let result = tokio::time::timeout(deadline, async {
        loop {
            // russh 0.63：wait() 直接返回 Option<ChannelMsg>（None = 通道关闭）
            match channel.wait().await {
                Some(russh::ChannelMsg::Data { ref data }) => {
                    output.push_str(&String::from_utf8_lossy(data));
                }
                Some(russh::ChannelMsg::ExtendedData { ref data, .. }) => {
                    output.push_str(&String::from_utf8_lossy(data));
                }
                Some(russh::ChannelMsg::ExitStatus { exit_status }) => {
                    exit_code = Some(exit_status);
                }
                Some(russh::ChannelMsg::Close) | None => break,
                Some(_) => {}
            }
        }
        Ok(())
    })
    .await;
    match result {
        Ok(Ok(())) => Ok((output, exit_code)),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("旁路命令超时：读取输出失败".to_string()),
    }
}

/// 取会话的 SSH 句柄（无连接/未缓存返回 None，调用方降级——如保留数字 uid）
pub async fn get_sftp_ssh_handle(
    state: &SftpState,
    session_id: &str,
) -> Option<std::sync::Arc<SftpSshHandle>> {
    state.ssh_handles.lock().await.get(session_id).cloned()
}

/// 传输失败后把指定会话从缓存剔除（尽力关闭）：下一次 `get_or_connect_sftp`
/// 会自动重建连接，避免重试一直命中已死亡的连接。
pub async fn evict_sftp_session(state: &SftpState, session_id: &str) {
    let client = state.clients.lock().await.remove(session_id);
    state.ssh_handles.lock().await.remove(session_id);
    state.id_name_cache.lock().await.retain(|(sid, _, _), _| sid != session_id);
    state.user_group_cache.lock().await.remove(session_id);
    state.exec_disabled.lock().await.remove(session_id);
    if let Some(client) = client {
        let _ = client.close().await;
        log::info!("[SFTP] Evicted session '{session_id}' from cache after transfer failure");
    }
}

/// 登记某会话写入过断点临时文件的目录：重连时据此扫描清理超龄孤儿断点。
pub async fn register_temp_dir(state: &SftpState, session_id: &str, dir: &str) {
    let mut temp_dirs = state.temp_dirs.lock().await;
    temp_dirs
        .entry(session_id.to_string())
        .or_default()
        .insert(dir.to_string());
}

/// 新建连接后清理该会话登记目录中超龄的 `.veloterm.tmp.*` 孤儿断点（>1 天，尽力而为）。
/// 后台执行不阻塞建连；正常断点（rename 已消费 / 未超龄）不受影响。
async fn cleanup_stale_temp_files_for(state: &SftpState, session_id: &str, client: &SftpClient) {
    let dirs: Vec<String> = {
        let temp_dirs = state.temp_dirs.lock().await;
        temp_dirs
            .get(session_id)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    };
    if dirs.is_empty() {
        return;
    }
    let client = Arc::clone(client);
    let session_id = session_id.to_string();
    tokio::spawn(async move {
        let removed = super::transfer::cleanup_stale_temp_files(&client, &dirs).await;
        if removed > 0 {
            log::info!(
                "[SFTP] Cleaned {removed} stale temp file(s) for session '{session_id}' after reconnect"
            );
        }
    });
}

pub async fn get_or_connect_sftp(
    state: &SftpState,
    session_id: &str,
) -> Result<SftpClient, String> {
    {
        let clients = state.clients.lock().await;
        if let Some(client) = clients.get(session_id) {
            return Ok(Arc::clone(client));
        }
    }

    // 尝试将 session_id 解析为 host_id 进行动态连接。
    // connect_lock 串行化建连：并发请求（如 StrictMode 双执行）共享一次握手，
    // 二次检查缓存避免重复建 SSH 连接。
    if let Ok(host_id) = session_id.parse::<usize>() {
        let _connect_guard = state.connect_lock.lock().await;
        {
            let clients = state.clients.lock().await;
            if let Some(client) = clients.get(session_id) {
                return Ok(Arc::clone(client));
            }
        }
        log::info!("[SFTP] Connecting on-demand for host_id={host_id} (session_id={session_id})");
        state.emit_sftp(
            crate::lifecycle::LifecycleKind::Connecting,
            session_id,
            "",
            None,
        );
        let started = std::time::Instant::now();
        let chain = resolve_host_chain(host_id)?;
        let (client, ssh) = match connect_sftp(chain).await {
            Ok(c) => c,
            Err(e) => {
                state.emit_sftp(
                    crate::lifecycle::LifecycleKind::Failed,
                    session_id,
                    "",
                    Some(&e),
                );
                return Err(e);
            }
        };
        log::info!(
            "[SFTP] Connected on-demand for host_id={host_id} in {}ms",
            started.elapsed().as_millis()
        );
        state.emit_sftp(
            crate::lifecycle::LifecycleKind::Connected,
            session_id,
            "",
            None,
        );
        let mut clients = state.clients.lock().await;
        clients.insert(session_id.to_string(), Arc::clone(&client));
        state
            .ssh_handles
            .lock()
            .await
            .insert(session_id.to_string(), ssh);
        // 新建（含断线重连）成功后，顺带清理该会话超龄的孤儿断点临时文件
        cleanup_stale_temp_files_for(state, session_id, &client).await;
        return Ok(client);
    }

    Err(format!(
        "SFTP session '{session_id}' is not connected and is not a valid host id"
    ))
}

pub async fn close_sftp(state: &SftpState, session_id: &str) -> Result<(), String> {
    let client = state.clients.lock().await.remove(session_id);
    state.ssh_handles.lock().await.remove(session_id);
    state.id_name_cache.lock().await.retain(|(sid, _, _), _| sid != session_id);
    state.exec_disabled.lock().await.remove(session_id);
    if let Some(client) = client {
        let _ = client.close().await;
        log::info!("[SFTP] Closed session {session_id}");
        state.emit_sftp(
            crate::lifecycle::LifecycleKind::Disconnected,
            session_id,
            "",
            None,
        );
    }
    Ok(())
}
