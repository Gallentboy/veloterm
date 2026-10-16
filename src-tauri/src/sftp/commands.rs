use std::io::SeekFrom;

use tauri::State;
use tauri::ipc::Channel;
use tokio::io::AsyncSeekExt;

use super::file_ops::{commit_temp_file, dedup_sftp_status_text, remote_dir_of, sftp_error_message, sibling_temp_path};
use super::manager::{
    close_sftp, connect_sftp, evict_sftp_session, get_or_connect_sftp, register_temp_dir,
    resolve_host_chain,
};
use super::transfer::{
    FailureSide, ProgressTracker, PumpError, PumpOptions, RemoteEndpoint, TransferKind,
    evict_decision, pump_stream, resolve_resume_offset,
};
use super::types::{RemoteFileItem, SftpState, TransferProgress};
use crate::text_detect::is_editable_sftp_entry;

/// 构造逐块进度回调：累计绝对字节数（含续传起始偏移），≥100ms 节流发包。
/// 完成包由调用方用 `offset + pump 返回值` 另发。
fn make_chunk_reporter(
    task_id: usize,
    total_size: u64,
    start_offset: u64,
    on_progress: Channel<TransferProgress>,
) -> impl FnMut(u64) {
    let tracker = std::cell::RefCell::new(ProgressTracker::new());
    let transferred = std::cell::Cell::new(start_offset);
    move |delta: u64| {
        let abs = transferred.get() + delta;
        transferred.set(abs);
        let mut tracker = tracker.borrow_mut();
        if let Some((speed_bps, percent)) =
            tracker.on_bytes(tokio::time::Instant::now(), delta, abs, total_size)
        {
            let _ = on_progress.send(TransferProgress {
                task_id,
                transferred: abs.to_string(),
                total: total_size.to_string(),
                speed_bps,
                percent,
                is_finished: false,
            });
        }
    }
}

/// 按剔除决策把失效会话从缓存移除（尽力关闭），使重试自动重建连接。
async fn evict_failed_session(
    state: &SftpState,
    kind: TransferKind,
    side: FailureSide,
    source_session_id: &str,
    target_session_id: &str,
) {
    match evict_decision(kind, side) {
        Some(RemoteEndpoint::Source) => evict_sftp_session(state, source_session_id).await,
        Some(RemoteEndpoint::Target) => evict_sftp_session(state, target_session_id).await,
        None => {}
    }
}

/// 回收任务取消令牌（所有出口路径必经）
async fn remove_cancel_token(state: &SftpState, task_id: usize) {
    state.cancel_tokens.lock().await.remove(&task_id);
}

#[tauri::command]
pub async fn sftp_next_task_id(state: State<'_, SftpState>) -> Result<usize, String> {
    Ok(state.allocate_task_id())
}

#[tauri::command]
pub async fn sftp_cancel_transfer(
    task_id: usize,
    state: State<'_, SftpState>,
) -> Result<(), String> {
    let mut tokens = state.cancel_tokens.lock().await;
    if let Some(tx) = tokens.remove(&task_id) {
        let _ = tx.send(());
        log::info!("[SFTP] Cancelled transfer task_id={task_id}");
    }
    Ok(())
}

#[tauri::command]
pub async fn sftp_connect(
    session_id: String,
    host_id: usize,
    state: State<'_, SftpState>,
) -> Result<(), String> {
    let chain = resolve_host_chain(host_id)?;
    let client = connect_sftp(chain).await?;
    let mut clients = state.clients.lock().await;
    clients.insert(session_id, client);
    Ok(())
}

#[tauri::command]
pub async fn sftp_close(session_id: String, state: State<'_, SftpState>) -> Result<(), String> {
    close_sftp(&state, &session_id).await
}

#[tauri::command]
pub async fn sftp_list_dir(
    session_id: String,
    path: String,
    state: State<'_, SftpState>,
) -> Result<Vec<RemoteFileItem>, String> {
    let client = get_or_connect_sftp(&state, &session_id).await?;

    let raw_path = path.trim();
    let normalized_path = if raw_path.is_empty() {
        ".".to_string()
    } else {
        raw_path.replace('\\', "/")
    };

    let entries = client
        .read_dir(&normalized_path)
        .await
        .map_err(|e| {
            format!(
                "Failed to read remote directory '{normalized_path}': {}",
                sftp_error_message(e)
            )
        })?;

    let mut items = Vec::new();
    for entry in entries {
        let name = entry.file_name();
        let is_dir = entry.file_type().is_dir();
        let is_symlink = entry.file_type().is_symlink();
        let meta = entry.metadata();

        let full_path = if normalized_path == "/" {
            format!("/{}", name)
        } else if normalized_path.ends_with('/') {
            format!("{}{}", normalized_path, name)
        } else if normalized_path == "." {
            name.clone()
        } else {
            format!("{}/{}", normalized_path, name)
        };

        let owner = meta
            .user
            .clone()
            .or_else(|| meta.uid.map(|u| u.to_string()))
            .unwrap_or_else(|| "-".to_string());

        let group = meta
            .group
            .clone()
            .or_else(|| meta.gid.map(|g| g.to_string()))
            .unwrap_or_else(|| "-".to_string());

        let editable = !is_dir && !is_symlink && is_editable_sftp_entry(&name, meta.len());

        items.push(RemoteFileItem {
            name,
            path: full_path,
            is_dir,
            is_symlink,
            size: meta.len().to_string(),
            modified_at: meta.mtime.map(u64::from).unwrap_or(0),
            permissions: meta.permissions.unwrap_or(0),
            owner,
            group,
            editable,
        });
    }

    // 目录优先，同级按文件名忽略大小写排序
    items.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(items)
}

#[tauri::command]
pub async fn sftp_download_file(
    session_id: String,
    task_id: usize,
    remote_path: String,
    local_path: String,
    resume_offset: Option<u64>,
    speed_limit_bps: Option<u64>,
    on_progress: Channel<TransferProgress>,
    state: State<'_, SftpState>,
) -> Result<(), String> {
    let kind = TransferKind::Download;
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut tokens = state.cancel_tokens.lock().await;
        tokens.insert(task_id, cancel_tx);
    }

    let client = match get_or_connect_sftp(&state, &session_id).await {
        Ok(client) => client,
        Err(e) => {
            remove_cancel_token(&state, task_id).await;
            return Err(e);
        }
    };

    let mut remote_file = match client.open(&remote_path).await {
        Ok(file) => file,
        Err(e) => {
            evict_failed_session(&state, kind, FailureSide::Source, &session_id, &session_id).await;
            remove_cancel_token(&state, task_id).await;
            return Err(format!(
                "Failed to open remote file '{remote_path}': {}",
                sftp_error_message(e)
            ));
        }
    };

    let total_size = match remote_file.metadata().await {
        Ok(meta) => meta.len(),
        Err(e) => {
            evict_failed_session(&state, kind, FailureSide::Source, &session_id, &session_id).await;
            remove_cancel_token(&state, task_id).await;
            return Err(format!(
                "Failed to stat remote file '{remote_path}': {}",
                sftp_error_message(e)
            ));
        }
    };

    if let Some(parent) = std::path::Path::new(&local_path).parent()
        && let Err(e) = tokio::fs::create_dir_all(parent).await
    {
        remove_cancel_token(&state, task_id).await;
        return Err(format!("Failed to create local directory: {e}"));
    }

    // 断点续传：本地现存长度与上报值取 min，截断到该偏移后续写（幂等覆盖防空洞）
    let existing_len = tokio::fs::metadata(&local_path).await.map(|m| m.len()).ok();
    let offset = resolve_resume_offset(resume_offset, existing_len);
    let local_file = if offset > 0 {
        let file = match tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&local_path)
            .await
        {
            Ok(file) => file,
            Err(e) => {
                remove_cancel_token(&state, task_id).await;
                return Err(format!("Failed to open local file '{local_path}': {e}"));
            }
        };
        if let Err(e) = file.set_len(offset).await {
            remove_cancel_token(&state, task_id).await;
            return Err(format!(
                "Failed to truncate local file to resume offset: {e}"
            ));
        }
        let mut file = file;
        if let Err(e) = file.seek(SeekFrom::Start(offset)).await {
            remove_cancel_token(&state, task_id).await;
            return Err(format!("Failed to seek local file: {e}"));
        }
        file
    } else {
        match tokio::fs::File::create(&local_path).await {
            Ok(file) => file,
            Err(e) => {
                remove_cancel_token(&state, task_id).await;
                return Err(format!("Failed to create local file '{local_path}': {e}"));
            }
        }
    };
    if offset > 0
        && let Err(e) = remote_file.seek(SeekFrom::Start(offset)).await
    {
        evict_failed_session(&state, kind, FailureSide::Source, &session_id, &session_id).await;
        remove_cancel_token(&state, task_id).await;
        return Err(format!(
            "Failed to seek remote file: {}",
            dedup_sftp_status_text(&e.to_string())
        ));
    }
    if offset > 0 {
        log::info!("[SFTP] Download task {task_id} resuming from offset {offset}");
    }

    let on_chunk = make_chunk_reporter(task_id, total_size, offset, on_progress.clone());
    let pump_result = pump_stream(
        remote_file,
        local_file,
        cancel_rx,
        on_chunk,
        PumpOptions {
            limit_bps: speed_limit_bps,
            ..Default::default()
        },
    )
    .await;

    match pump_result {
        Ok(copied) => {
            let transferred = offset + copied;
            if total_size > 0 && transferred != total_size {
                remove_cancel_token(&state, task_id).await;
                return Err(format!(
                    "Remote file changed during transfer: expected {total_size} bytes, got {transferred}"
                ));
            }
            let _ = on_progress.send(TransferProgress {
                task_id,
                transferred: transferred.to_string(),
                total: total_size.to_string(),
                speed_bps: 0,
                percent: 100.0,
                is_finished: true,
            });
            remove_cancel_token(&state, task_id).await;
            log::info!("[SFTP] Download task {task_id} completed: {transferred} bytes");
            Ok(())
        }
        Err(PumpError::Cancelled) => {
            remove_cancel_token(&state, task_id).await;
            // 保留本地部分文件：重试携带 resume_offset 可续传
            log::info!("[SFTP] Download task {task_id} cancelled by user");
            Err("Transfer cancelled by user".to_string())
        }
        Err(PumpError::Read(e)) => {
            // 远端读失败：剔除死连接，保留本地断点
            evict_failed_session(&state, kind, FailureSide::Source, &session_id, &session_id).await;
            remove_cancel_token(&state, task_id).await;
            Err(format!(
                "Failed to read from remote file: {}",
                dedup_sftp_status_text(&e.to_string())
            ))
        }
        Err(PumpError::Write(e)) => {
            // 本地写失败：无需剔除远端连接，保留远端可重试
            remove_cancel_token(&state, task_id).await;
            Err(format!("Failed to write to local file: {e}"))
        }
    }
}

#[tauri::command]
pub async fn sftp_upload_file(
    session_id: String,
    task_id: usize,
    local_path: String,
    remote_path: String,
    resume_offset: Option<u64>,
    speed_limit_bps: Option<u64>,
    on_progress: Channel<TransferProgress>,
    state: State<'_, SftpState>,
) -> Result<(), String> {
    let kind = TransferKind::Upload;
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut tokens = state.cancel_tokens.lock().await;
        tokens.insert(task_id, cancel_tx);
    }

    let mut local_file = match tokio::fs::File::open(&local_path).await {
        Ok(file) => file,
        Err(e) => {
            remove_cancel_token(&state, task_id).await;
            return Err(format!("Failed to open local file '{local_path}': {e}"));
        }
    };

    let total_size = local_file.metadata().await.map(|m| m.len()).unwrap_or(0);

    let client = match get_or_connect_sftp(&state, &session_id).await {
        Ok(client) => client,
        Err(e) => {
            remove_cancel_token(&state, task_id).await;
            return Err(e);
        }
    };

    // 写同目录临时文件，成功后 rename 原子覆盖（与 copy/在线编辑一致）；
    // 失败/取消时保留临时文件作为断点，重试携带 resume_offset 续传
    let temp_path = sibling_temp_path(&remote_path);
    // 登记断点所在目录：重连时据此扫描清理超龄孤儿断点（>1 天）
    if let Some(dir) = remote_dir_of(&remote_path) {
        register_temp_dir(&state, &session_id, dir).await;
    }
    let temp_len = client.metadata(&temp_path).await.map(|m| m.len()).ok();
    let offset = resolve_resume_offset(resume_offset, temp_len);

    let temp_file = if offset > 0 {
        let file = match client
            .open_with_flags(
                &temp_path,
                russh_sftp::protocol::OpenFlags::CREATE | russh_sftp::protocol::OpenFlags::WRITE,
            )
            .await
        {
            Ok(file) => file,
            Err(e) => {
                evict_failed_session(&state, kind, FailureSide::Target, &session_id, &session_id)
                    .await;
                remove_cancel_token(&state, task_id).await;
                return Err(format!(
                    "Failed to open temp file '{temp_path}' for resume: {e}"
                ));
            }
        };
        let mut file = file;
        if let Err(e) = file.seek(SeekFrom::Start(offset)).await {
            evict_failed_session(&state, kind, FailureSide::Target, &session_id, &session_id).await;
            remove_cancel_token(&state, task_id).await;
            return Err(format!(
                "Failed to seek temp file to resume offset: {}",
                dedup_sftp_status_text(&e.to_string())
            ));
        }
        file
    } else {
        match client.create(&temp_path).await {
            Ok(file) => file,
            Err(e) => {
                evict_failed_session(&state, kind, FailureSide::Target, &session_id, &session_id)
                    .await;
                remove_cancel_token(&state, task_id).await;
                return Err(format!(
                    "Failed to create temp file '{temp_path}': {}",
                    sftp_error_message(e)
                ));
            }
        }
    };
    if offset > 0
        && let Err(e) = local_file.seek(SeekFrom::Start(offset)).await
    {
        remove_cancel_token(&state, task_id).await;
        return Err(format!("Failed to seek local file: {e}"));
    }
    if offset > 0 {
        log::info!("[SFTP] Upload task {task_id} resuming from offset {offset}");
    }

    let on_chunk = make_chunk_reporter(task_id, total_size, offset, on_progress.clone());
    let pump_result = pump_stream(
        local_file,
        temp_file,
        cancel_rx,
        on_chunk,
        PumpOptions {
            limit_bps: speed_limit_bps,
            ..Default::default()
        },
    )
    .await;

    match pump_result {
        Ok(copied) => {
            let transferred = offset + copied;
            if total_size > 0 && transferred != total_size {
                let _ = client.remove_file(&temp_path).await;
                remove_cancel_token(&state, task_id).await;
                return Err(format!(
                    "Local file changed during transfer: expected {total_size} bytes, got {transferred}"
                ));
            }
            // rename 原子覆盖目标（失败时 commit_temp_file 内部已清理临时文件）
            if let Err(e) = commit_temp_file(&client, &temp_path, &remote_path).await {
                evict_failed_session(&state, kind, FailureSide::Target, &session_id, &session_id)
                    .await;
                remove_cancel_token(&state, task_id).await;
                return Err(e);
            }
            let _ = on_progress.send(TransferProgress {
                task_id,
                transferred: transferred.to_string(),
                total: total_size.to_string(),
                speed_bps: 0,
                percent: 100.0,
                is_finished: true,
            });
            remove_cancel_token(&state, task_id).await;
            log::info!("[SFTP] Upload task {task_id} completed: {transferred} bytes");
            Ok(())
        }
        Err(PumpError::Cancelled) => {
            remove_cancel_token(&state, task_id).await;
            // 保留远端临时断点：重试携带 resume_offset 可续传
            log::info!("[SFTP] Upload task {task_id} cancelled by user");
            Err("Transfer cancelled by user".to_string())
        }
        Err(PumpError::Read(e)) => {
            // 本地读失败：无需剔除远端连接，保留远端断点
            remove_cancel_token(&state, task_id).await;
            Err(format!("Failed to read from local file: {e}"))
        }
        Err(PumpError::Write(e)) => {
            // 远端写失败：剔除死连接，保留临时断点
            evict_failed_session(&state, kind, FailureSide::Target, &session_id, &session_id).await;
            remove_cancel_token(&state, task_id).await;
            Err(format!(
                "Failed to write to remote file: {}",
                dedup_sftp_status_text(&e.to_string())
            ))
        }
    }
}

/// 远端到远端的流式复制：源会话读、目标会话写，数据全程不落本地磁盘。
/// 两个 session_id 相同时命中同一缓存连接（同主机复制），不同则为跨主机搬运。
/// 目标侧写同目录临时文件，成功后 rename 原子覆盖；失败/取消保留临时断点，
/// 重试携带 resume_offset 从断点续传（读写两端 seek 到偏移后续写，幂等覆盖防空洞）。
#[tauri::command]
pub async fn sftp_copy_file(
    state: State<'_, SftpState>,
    task_id: usize,
    source_session_id: String,
    source_path: String,
    target_session_id: String,
    target_path: String,
    resume_offset: Option<u64>,
    speed_limit_bps: Option<u64>,
    on_progress: Channel<TransferProgress>,
) -> Result<(), String> {
    let kind = TransferKind::Copy;
    // 注册取消令牌（与 download/upload 一致，按 task_id 索引）
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut tokens = state.cancel_tokens.lock().await;
        tokens.insert(task_id, cancel_tx);
    }

    // 解析两端客户端；任一失败时回收取消令牌
    let source_client = match get_or_connect_sftp(&state, &source_session_id).await {
        Ok(client) => client,
        Err(e) => {
            remove_cancel_token(&state, task_id).await;
            return Err(e);
        }
    };
    let target_client = match get_or_connect_sftp(&state, &target_session_id).await {
        Ok(client) => client,
        Err(e) => {
            remove_cancel_token(&state, task_id).await;
            return Err(e);
        }
    };

    let mut source_file = match source_client.open(&source_path).await {
        Ok(file) => file,
        Err(e) => {
            evict_failed_session(
                &state,
                kind,
                FailureSide::Source,
                &source_session_id,
                &target_session_id,
            )
            .await;
            remove_cancel_token(&state, task_id).await;
            return Err(format!(
                "Failed to open source file '{source_path}': {}",
                sftp_error_message(e)
            ));
        }
    };

    // stat 源文件取总大小与权限（失败视为连接问题：剔除后中断，交给重试重连）
    let meta = match source_file.metadata().await {
        Ok(meta) => meta,
        Err(e) => {
            evict_failed_session(
                &state,
                kind,
                FailureSide::Source,
                &source_session_id,
                &target_session_id,
            )
            .await;
            remove_cancel_token(&state, task_id).await;
            return Err(format!(
                "Failed to stat source file '{source_path}': {}",
                sftp_error_message(e)
            ));
        }
    };
    let total_size = meta.len();
    let source_perms = meta.permissions;

    // 目标侧写同目录临时文件；断点续传取「上报值与现存临时长度」的较小值
    let temp_path = sibling_temp_path(&target_path);
    // 登记断点所在目录：重连时据此扫描清理超龄孤儿断点（>1 天）
    if let Some(dir) = remote_dir_of(&target_path) {
        register_temp_dir(&state, &target_session_id, dir).await;
    }
    let temp_len = target_client
        .metadata(&temp_path)
        .await
        .map(|m| m.len())
        .ok();
    let offset = resolve_resume_offset(resume_offset, temp_len);

    let temp_file = if offset > 0 {
        let file = match target_client
            .open_with_flags(
                &temp_path,
                russh_sftp::protocol::OpenFlags::CREATE | russh_sftp::protocol::OpenFlags::WRITE,
            )
            .await
        {
            Ok(file) => file,
            Err(e) => {
                evict_failed_session(
                    &state,
                    kind,
                    FailureSide::Target,
                    &source_session_id,
                    &target_session_id,
                )
                .await;
                remove_cancel_token(&state, task_id).await;
                return Err(format!(
                    "Failed to open temp file '{temp_path}' for resume: {e}"
                ));
            }
        };
        let mut file = file;
        if let Err(e) = file.seek(SeekFrom::Start(offset)).await {
            evict_failed_session(
                &state,
                kind,
                FailureSide::Target,
                &source_session_id,
                &target_session_id,
            )
            .await;
            remove_cancel_token(&state, task_id).await;
            return Err(format!(
                "Failed to seek temp file to resume offset: {}",
                dedup_sftp_status_text(&e.to_string())
            ));
        }
        file
    } else {
        match target_client.create(&temp_path).await {
            Ok(file) => file,
            Err(e) => {
                evict_failed_session(
                    &state,
                    kind,
                    FailureSide::Target,
                    &source_session_id,
                    &target_session_id,
                )
                .await;
                remove_cancel_token(&state, task_id).await;
                return Err(format!(
                    "Failed to create temp file '{temp_path}': {}",
                    sftp_error_message(e)
                ));
            }
        }
    };
    if offset > 0
        && let Err(e) = source_file.seek(SeekFrom::Start(offset)).await
    {
        evict_failed_session(
            &state,
            kind,
            FailureSide::Source,
            &source_session_id,
            &target_session_id,
        )
        .await;
        remove_cancel_token(&state, task_id).await;
        return Err(format!(
            "Failed to seek source file to resume offset: {}",
            dedup_sftp_status_text(&e.to_string())
        ));
    }
    if offset > 0 {
        log::info!("[SFTP] Copy task {task_id} resuming from offset {offset}");
    }

    let on_chunk = make_chunk_reporter(task_id, total_size, offset, on_progress.clone());
    let pump_result = pump_stream(
        source_file,
        temp_file,
        cancel_rx,
        on_chunk,
        PumpOptions {
            limit_bps: speed_limit_bps,
            ..Default::default()
        },
    )
    .await;

    match pump_result {
        Ok(copied) => {
            let transferred = offset + copied;
            if total_size > 0 && transferred != total_size {
                // 源在传输中发生变化，断点已无意义：清理临时文件
                let _ = target_client.remove_file(&temp_path).await;
                remove_cancel_token(&state, task_id).await;
                return Err(format!(
                    "Source file changed during transfer: expected {total_size} bytes, got {transferred}"
                ));
            }

            // 尽力保留源文件权限（源端无权限信息或 setstat 不受支持时忽略）
            if let Some(perms) = source_perms {
                let attrs = russh_sftp::protocol::FileAttributes {
                    permissions: Some(perms),
                    ..Default::default()
                };
                let _ = target_client.set_metadata(&temp_path, attrs).await;
            }

            // rename 原子覆盖目标（失败时 commit_temp_file 内部已清理临时文件）
            if let Err(e) = commit_temp_file(&target_client, &temp_path, &target_path).await {
                evict_failed_session(
                    &state,
                    kind,
                    FailureSide::Target,
                    &source_session_id,
                    &target_session_id,
                )
                .await;
                remove_cancel_token(&state, task_id).await;
                return Err(e);
            }

            let _ = on_progress.send(TransferProgress {
                task_id,
                transferred: transferred.to_string(),
                total: total_size.to_string(),
                speed_bps: 0,
                percent: 100.0,
                is_finished: true,
            });
            remove_cancel_token(&state, task_id).await;
            log::info!("[SFTP] Copy task {task_id} completed: {transferred} bytes");
            Ok(())
        }
        Err(PumpError::Cancelled) => {
            remove_cancel_token(&state, task_id).await;
            // 保留临时断点：重试携带 resume_offset 可续传（原实现删除断点，已改为保留）
            log::info!("[SFTP] Copy task {task_id} cancelled by user");
            Err("Transfer cancelled by user".to_string())
        }
        Err(PumpError::Read(e)) => {
            // 源端读失败：剔除死连接，保留目标断点
            evict_failed_session(
                &state,
                kind,
                FailureSide::Source,
                &source_session_id,
                &target_session_id,
            )
            .await;
            remove_cancel_token(&state, task_id).await;
            Err(format!("Failed to read from source file: {e}"))
        }
        Err(PumpError::Write(e)) => {
            // 目标端写失败：剔除死连接，保留断点
            evict_failed_session(
                &state,
                kind,
                FailureSide::Target,
                &source_session_id,
                &target_session_id,
            )
            .await;
            remove_cancel_token(&state, task_id).await;
            Err(format!("Failed to write to temp file: {e}"))
        }
    }
}

#[tauri::command]
pub async fn sftp_pick_upload_file() -> Result<Option<String>, String> {
    let file = rfd::AsyncFileDialog::new().pick_file().await;
    Ok(file.map(|f| f.path().to_string_lossy().to_string()))
}

#[tauri::command]
pub async fn sftp_pick_download_path(suggested_name: String) -> Result<Option<String>, String> {
    let file = rfd::AsyncFileDialog::new()
        .set_file_name(&suggested_name)
        .save_file()
        .await;
    Ok(file.map(|f| f.path().to_string_lossy().to_string()))
}

#[tauri::command]
pub async fn sftp_remove(
    session_id: String,
    path: String,
    is_dir: bool,
    state: State<'_, SftpState>,
) -> Result<(), String> {
    let client = get_or_connect_sftp(&state, &session_id).await?;
    let target_path = path.trim().replace('\\', "/");
    log::info!(
        "[SFTP] Deleting {} at {}",
        if is_dir { "directory" } else { "file" },
        target_path
    );

    if is_dir {
        client
            .remove_dir(target_path)
            .await
            .map_err(|e| format!("Failed to remove directory: {e}"))?;
    } else {
        client
            .remove_file(target_path)
            .await
            .map_err(|e| format!("Failed to remove file: {e}"))?;
    }

    Ok(())
}
