use std::io::SeekFrom;

use tauri::State;
use tauri::ipc::Channel;
use tokio::io::AsyncSeekExt;

use super::file_ops::{commit_temp_file, dedup_sftp_status_text, remote_dir_of, sftp_error_message, sibling_temp_path};
use super::manager::{
    close_sftp, connect_sftp, evict_sftp_session, get_or_connect_sftp, get_sftp_ssh_handle,
    register_temp_dir, resolve_host_chain, sftp_exec_capture, sftp_exec_capture_status,
};
use super::transfer::{
    FailureSide, ProgressTracker, PumpError, PumpOptions, RemoteEndpoint, TransferKind,
    evict_decision, pump_stream, resolve_resume_offset,
};
use super::types::{
    FastRemovePreview, RemoteFileItem, RemoveProgress, SftpState, TransferProgress, TreeScanResult,
};
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
    let (client, ssh) = connect_sftp(chain).await?;
    let mut clients = state.clients.lock().await;
    clients.insert(session_id.clone(), client);
    state.ssh_handles.lock().await.insert(session_id, ssh);
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

    // uid/gid → 名称：SFTP attrs 的 user/group 名称字段部分服务端不填（回退数字），
    // 在同连接的独立 exec 通道跑 getent 补齐（缓存后零 exec；失败/无句柄保持数字）
    resolve_owner_group_names(&state, &session_id, &mut items).await;

    Ok(items)
}

/// 把列表里仍是数字的 owner/group 经 getent 解析为名称并回填。
/// 解析结果落 id_name_cache（会话维度），同会话后续列目录零 exec；
/// 回填不依赖本次是否发起 getent——缓存命中的 id 换目录后仍回填名称
/// （全部命中时不再 exec，但必须走回填而不是提前返回，否则显示回数字）。
/// getent 不可用（BSD/macOS 目标机）或无 SSH 句柄时静默保持数字——
/// 列目录主链路不受旁路命令失败影响。
async fn resolve_owner_group_names(
    state: &State<'_, SftpState>,
    session_id: &str,
    items: &mut [RemoteFileItem],
) {
    let sid = session_id.to_string();
    // 数字形态才需要解析（服务端填过名称的直通）；已缓存的 id 不进缺口清单
    let mut uid_misses: Vec<u32> = Vec::new();
    let mut gid_misses: Vec<u32> = Vec::new();
    {
        let cache = state.id_name_cache.lock().await;
        for item in items.iter() {
            if let Ok(uid) = item.owner.parse::<u32>()
                && !cache.contains_key(&(sid.clone(), false, uid))
                && !uid_misses.contains(&uid)
            {
                uid_misses.push(uid);
            }
            if let Ok(gid) = item.group.parse::<u32>()
                && !cache.contains_key(&(sid.clone(), true, gid))
                && !gid_misses.contains(&gid)
            {
                gid_misses.push(gid);
            }
        }
    }

    // getent passwd uid... / getent group gid...：一次 exec 解析一批；
    // 行形如 name:x:ID:...（ID 在第三列）。exec 失败（拒绝/超时/ForceCommand
    // 劫持）标记会话降级——uid/gid 任一分支失败都标记，之后列目录零重试。
    // 缓存锁不跨 exec await（左右分栏并发列目录互不阻塞），结果到齐后一次落锁。
    // 仅当确有缺口、exec 未被禁且句柄在位时才发起；跳过 exec 不影响下面的回填。
    let mut outputs: Vec<(bool, String)> = Vec::new();
    if !(uid_misses.is_empty() && gid_misses.is_empty())
        && !state.exec_disabled.lock().await.contains(&sid)
        && let Some(ssh) = get_sftp_ssh_handle(state, session_id).await
    {
        let lookup = |is_group: bool, ids: &[u32]| {
            let db = if is_group { "group" } else { "passwd" };
            let args = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(" ");
            format!("getent {db} {args}")
        };
        for (is_group, ids) in [(false, &uid_misses), (true, &gid_misses)] {
            if ids.is_empty() {
                continue;
            }
            match sftp_exec_capture(&ssh, &lookup(is_group, ids), 10).await {
                Ok(out) => outputs.push((is_group, out)),
                Err(e) => {
                    log::warn!(
                        "[SFTP] getent 旁路解析失败，本会话降级为数字 uid/gid: {e}"
                    );
                    state.exec_disabled.lock().await.insert(sid.clone());
                }
            }
        }
    }

    // 回填（缓存未命中的保持数字——uid 已被删除等场景）；
    let mut cache = state.id_name_cache.lock().await;
    for (is_group, out) in &outputs {
        // 索引一律 get（-D indexing-slicing 门禁 + 短行/异常行安全跳过）
        for line in out.lines() {
            let fields: Vec<&str> = line.split(':').collect();
            if let (Some(&name), Some(id_str)) = (fields.first(), fields.get(2))
                && let Ok(id) = id_str.parse::<u32>()
            {
                cache.insert((sid.clone(), *is_group, id), name.to_string());
            }
        }
    }
    for item in items.iter_mut() {
        if let Ok(uid) = item.owner.parse::<u32>()
            && let Some(name) = cache.get(&(sid.clone(), false, uid))
        {
            item.owner.clone_from(name);
        }
        if let Ok(gid) = item.group.parse::<u32>()
            && let Some(name) = cache.get(&(sid.clone(), true, gid))
        {
            item.group.clone_from(name);
        }
    }
}

/// 目录大小：在同连接的独立 exec 通道跑 du（SFTP 递归 stat 大目录既慢又费流量）。
/// GNU coreutils 用 `du -sb`（apparent 字节数），不支持 -b 的环境（BusyBox/macOS）
/// 回退 `du -sk`×1024；路径单引号安全包裹。
#[tauri::command]
pub async fn sftp_dir_size(
    session_id: String,
    path: String,
    state: State<'_, SftpState>,
) -> Result<u64, String> {
    // 先确保连接（连带句柄就位）
    let _ = get_or_connect_sftp(&state, &session_id).await?;
    let ssh = get_sftp_ssh_handle(&state, &session_id)
        .await
        .ok_or_else(|| "SSH 会话不可用".to_string())?;

    let quoted = format!("'{}'", path.replace('\'', "'\\\"'"));
    // du 大目录可能长跑：限时 120s；-b 失败（非法选项退出非零且无输出）再试 -sk
    if let Ok(out) = sftp_exec_capture(&ssh, &format!("du -sb -- {quoted}"), 120).await
        && let Some(bytes) = out.split_whitespace().next().and_then(|t| t.parse::<u64>().ok())
    {
        return Ok(bytes);
    }
    let out = sftp_exec_capture(&ssh, &format!("du -sk -- {quoted}"), 120).await?;
    let kb = out
        .split_whitespace()
        .next()
        .and_then(|t| t.parse::<u64>().ok())
        .ok_or_else(|| format!("du 输出无法解析: {}", out.trim()))?;
    Ok(kb.saturating_mul(1024))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
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
#[allow(clippy::too_many_arguments)]
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
#[allow(clippy::too_many_arguments)]
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
pub async fn sftp_pick_upload_files() -> Result<Option<Vec<String>>, String> {
    // 多选：一次拖起/选起多个文件批量上传（取消返回 None，空选也返回 None）
    let files = rfd::AsyncFileDialog::new().pick_files().await;
    Ok(files.map(|fs| {
        fs.iter()
            .map(|f| f.path().to_string_lossy().to_string())
            .collect()
    }))
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
pub async fn sftp_pick_download_dir() -> Result<Option<String>, String> {
    // 多选批量下载：选目标目录（文件名取远程同名）
    let dir = rfd::AsyncFileDialog::new().pick_folder().await;
    Ok(dir.map(|d| d.path().to_string_lossy().to_string()))
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

// ─────────── 目录递归删除（纯 SFTP 逐文件 + Shell 极速两路） ───────────
//
// 交互契约（前端 SftpSessionView 的 useRemoteDelete 状态机）：
//   确认删除目录 → sftp_scan_tree 计数（上限/超时，超限即 truncated）
//     ├─ ≤ 阈值且未截断 → sftp_remove_tree（进度 + 可取消）
//     └─ 超阈值/截断 → sftp_preview_fast_remove 取真实命令串弹红字审批框
//         ├─ 确认 → sftp_remove_fast（退出码==0 判成功；失败自动回退 remove_tree）
//         └─ 仍用 SFTP → sftp_remove_tree

/// 触发「极速删除」询问的节点数阈值：SFTP unlink 实测 80–200 项/秒，
/// 500 ≈ 3–6s+，是用户可感知「慢」的起点（含根目录自身）
const FAST_ASK_THRESHOLD: usize = 500;
/// 扫描计数上限：超过即判「大量文件」（truncated=true，前端显示「N+」）。
/// readdir 批量很快，2000 节点亚秒级——不必数全即可决策
const SCAN_CAP: usize = 2000;
/// 扫描超时（秒）：防御黑洞型 readdir 挂起（堡垒机限流等）
const SCAN_TIMEOUT_SECS: u64 = 8;
/// 进度事件节流：每处理 N 项发一次（Channel 事件开销控制）
const REMOVE_PROGRESS_EVERY: u32 = 20;

/// POSIX 单引号安全包裹（' → '\''）：路径只作为独立 argv（$1）传给内层 sh，
/// 绝不拼接进命令体——引号/空格/分号/反引号/中文全部失效，无注入面
fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// 构造极速删除命令（预览与执行同一函数——单一事实源，展示什么就执行什么；
/// 执行命令绝不信前端回传）。外层登录 shell（sh/bash/zsh 通吃）把它解析为
/// 固定四参，路径仅作为内层 sh 的 $1：`sh -c 'rm -rf -- "$1"' veloterm-rm <q>`
fn build_fast_remove_command(path: &str) -> String {
    format!(
        "sh -c 'rm -rf -- \"$1\"' veloterm-rm {}",
        shell_single_quote(path)
    )
}

/// 极速删除超时：节点数换算（rm -rf 实测数千项/秒，公式留 10× 余量），
/// 封顶 600s、无节点提示时取保守上限
fn fast_remove_timeout_secs(node_count: Option<u64>) -> u64 {
    node_count
        .map(|n| std::cmp::min(600, 60 + n / 100))
        .unwrap_or(600)
}

/// 删除目标护栏：拒绝服务器根（rm -rf 的口令式灾难必须代码层拦死，
/// 前端已拦一道，这里是后端双保险）
fn ensure_safe_remove_target(path: &str) -> Result<(), String> {
    let p = path.trim().trim_end_matches('/');
    if p.is_empty() || p == "/" {
        return Err("拒绝删除服务器根目录".to_string());
    }
    Ok(())
}

/// 归一化删除目标路径（trim + 反斜杠→正斜杠，与 sftp_remove 同规则）
fn normalize_remove_path(path: &str) -> Result<String, String> {
    let p = path.trim().replace('\\', "/");
    ensure_safe_remove_target(&p)?;
    Ok(p)
}

/// 远端路径拼接（与 sftp_list_dir 的 full_path 规则一致：根目录不加双斜杠）
fn join_remote(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{name}")
    } else {
        format!("{base}/{name}")
    }
}

/// 可测试的远端树操作抽象：真实实现包 russh_sftp 客户端；单测用内存假实现
/// （symlink 不下钻 / 深层迭代 / 部分失败继续 / 截断计数四类语义在 macOS 门禁可测）
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TreeEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
}

pub(super) trait RemoteTreeFs: Send + Sync {
    fn list(&self, path: &str) -> impl std::future::Future<Output = Result<Vec<TreeEntry>, String>> + Send;
    fn unlink(&self, path: &str) -> impl std::future::Future<Output = Result<(), String>> + Send;
    fn rmdir(&self, path: &str) -> impl std::future::Future<Output = Result<(), String>> + Send;
}

/// russh_sftp 真实适配层
struct SftpTreeFs<'a> {
    client: &'a russh_sftp::client::SftpSession,
}

impl RemoteTreeFs for SftpTreeFs<'_> {
    async fn list(&self, path: &str) -> Result<Vec<TreeEntry>, String> {
        let entries = self
            .client
            .read_dir(path)
            .await
            .map_err(sftp_error_message)?;
        Ok(entries
            .into_iter()
            .map(|e| TreeEntry {
                name: e.file_name(),
                is_dir: e.file_type().is_dir(),
                is_symlink: e.file_type().is_symlink(),
            })
            .collect())
    }

    async fn unlink(&self, path: &str) -> Result<(), String> {
        self.client.remove_file(path).await.map_err(sftp_error_message)
    }

    async fn rmdir(&self, path: &str) -> Result<(), String> {
        self.client.remove_dir(path).await.map_err(sftp_error_message)
    }
}

/// 计数扫描：DFS 显式栈（防深目录递归溢出），symlink 只计数不下钻；
/// 读不了的子目录跳过（扫描只为决策与进度分母，不追求精确）。
/// 返回 (节点数含根, 是否触上限截断)
async fn count_tree<Fs: RemoteTreeFs>(fs: &Fs, root: &str, cap: usize) -> (u64, bool) {
    let mut count: u64 = 1; // 根自身
    let mut truncated = false;
    let mut stack = vec![root.to_string()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs.list(&dir).await else {
            continue;
        };
        for e in entries {
            count += 1;
            if count as usize >= cap {
                truncated = true;
                return (count, true);
            }
            if e.is_dir && !e.is_symlink {
                stack.push(join_remote(&dir, &e.name));
            }
        }
    }
    (count, truncated)
}

/// 递归删除汇总
pub(super) struct RemoveSummary {
    pub deleted: u64,
    pub failed: u64,
    pub first_error: Option<String>,
}

enum TreeOp {
    /// 进入目录：列子项，压入自身 Rmdir + 各子项操作（后序：子项全清才删自身）
    Enter(String),
    Unlink(String),
    Rmdir(String),
}

/// 纯 SFTP 后序递归删除：显式栈迭代（无递归栈深限制）；symlink 直接 unlink
/// 不下钻（与 rm -rf 同语义）；单项失败（权限等）记录并继续——部分删除是
/// 递归删除的正确语义，连接级错误由上层 await 自然中断。
/// on_progress 节流回调 (deleted, failed)，每 REMOVE_PROGRESS_EVERY 项一次。
async fn remove_tree<Fs: RemoteTreeFs>(
    fs: &Fs,
    root: &str,
    mut on_progress: impl FnMut(u64, u64),
) -> RemoveSummary {
    let mut summary = RemoveSummary { deleted: 0, failed: 0, first_error: None };
    let mut since_event: u32 = 0;
    let mut record = |summary: &mut RemoveSummary, r: Result<(), String>| match r {
        Ok(()) => summary.deleted += 1,
        Err(e) => {
            summary.failed += 1;
            if summary.first_error.is_none() {
                summary.first_error = Some(e);
            }
        }
    };
    let mut stack = vec![TreeOp::Enter(root.to_string())];
    while let Some(op) = stack.pop() {
        match op {
            TreeOp::Enter(dir) => {
                match fs.list(&dir).await {
                    Ok(entries) => {
                        // 先压 Rmdir（栈序保证最后弹出），再压子项
                        stack.push(TreeOp::Rmdir(dir.clone()));
                        for e in entries {
                            let child = join_remote(&dir, &e.name);
                            if e.is_dir && !e.is_symlink {
                                stack.push(TreeOp::Enter(child));
                            } else {
                                stack.push(TreeOp::Unlink(child));
                            }
                        }
                    }
                    Err(e) => {
                        summary.failed += 1;
                        if summary.first_error.is_none() {
                            summary.first_error = Some(e);
                        }
                    }
                }
            }
            TreeOp::Unlink(p) => record(&mut summary, fs.unlink(&p).await),
            TreeOp::Rmdir(p) => record(&mut summary, fs.rmdir(&p).await),
        }
        since_event += 1;
        if since_event >= REMOVE_PROGRESS_EVERY {
            since_event = 0;
            on_progress(summary.deleted, summary.failed);
        }
    }
    summary
}

/// 扫描计数：超过 FAST_ASK_THRESHOLD（且未截断）由前端直接走纯 SFTP 删除；
/// 截断即「大量文件」必然询问。阈值不在此处判断——决策留在前端状态机
#[tauri::command]
pub async fn sftp_scan_tree(
    session_id: String,
    path: String,
    state: State<'_, SftpState>,
) -> Result<TreeScanResult, String> {
    let client = get_or_connect_sftp(&state, &session_id).await?;
    let target = normalize_remove_path(&path)?;
    let fs = SftpTreeFs { client: &client };
    let (node_count, truncated) = tokio::time::timeout(
        tokio::time::Duration::from_secs(SCAN_TIMEOUT_SECS),
        count_tree(&fs, &target, SCAN_CAP),
    )
    .await
    .map_err(|_| format!("扫描超时（>{SCAN_TIMEOUT_SECS}s），目录过大或网络不稳定"))?;
    Ok(TreeScanResult { node_count, truncated })
}

/// 极速删除预览：返回将与执行完全一致（同一构造函数）的命令串与超时，
/// 前端弹红字审批框。EXEC_DISABLED 的会话在预览即报错——前端不弹极速选项
#[tauri::command]
pub async fn sftp_preview_fast_remove(
    session_id: String,
    path: String,
    node_count: Option<u64>,
    state: State<'_, SftpState>,
) -> Result<FastRemovePreview, String> {
    let _ = get_or_connect_sftp(&state, &session_id).await?;
    if state.exec_disabled.lock().await.contains(&session_id) {
        return Err("EXEC_DISABLED".to_string());
    }
    if get_sftp_ssh_handle(&state, &session_id).await.is_none() {
        return Err("SSH 会话不可用".to_string());
    }
    let target = normalize_remove_path(&path)?;
    Ok(FastRemovePreview {
        command: build_fast_remove_command(&target),
        timeout_secs: fast_remove_timeout_secs(node_count),
    })
}

/// Shell 极速删除：sh -c 'rm -rf -- "$1"' 形态（路径仅作 $1，无注入面）；
/// 退出码==0 判成功 + stat 双保险；127（sh 不存在，如 Windows 默认 shell）
/// 标记 exec_disabled 并给出明确文案。无进度、不可中断（语义如审批框提示）
#[tauri::command]
pub async fn sftp_remove_fast(
    session_id: String,
    path: String,
    node_count: Option<u64>,
    state: State<'_, SftpState>,
) -> Result<(), String> {
    let client = get_or_connect_sftp(&state, &session_id).await?;
    if state.exec_disabled.lock().await.contains(&session_id) {
        return Err("EXEC_DISABLED".to_string());
    }
    let ssh = get_sftp_ssh_handle(&state, &session_id)
        .await
        .ok_or_else(|| "SSH 会话不可用".to_string())?;
    let target = normalize_remove_path(&path)?;
    let command = build_fast_remove_command(&target);
    let (output, code) = sftp_exec_capture_status(
        &ssh,
        &command,
        fast_remove_timeout_secs(node_count),
    )
    .await
    .map_err(|e| format!("服务器 Shell 通道不可用：{e}"))?;
    match code {
        Some(0) => {
            // 双保险：命令报成功但目标仍在（如只读挂载上 rm 静默失败的场景）
            if client.metadata(&target).await.is_ok() {
                return Err(format!(
                    "命令报告成功但目标仍存在：{}",
                    output.trim()
                ));
            }
            Ok(())
        }
        Some(127) => {
            // command not found：服务器无 POSIX sh（Windows OpenSSH 默认 cmd 等）
            state.exec_disabled.lock().await.insert(session_id.clone());
            Err("服务器无 POSIX shell（sh 不可用），极速删除不适用".to_string())
        }
        other => Err(format!(
            "服务器 Shell 删除失败（退出码 {other:?}）：{}",
            output.trim().chars().take(200).collect::<String>()
        )),
    }
}

/// 纯 SFTP 递归删除结果
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveTreeOutcome {
    pub deleted: u64,
    pub failed: u64,
}

/// 纯 SFTP 逐文件递归删除（进度 + 可取消）：单项失败继续、结束汇总；
/// 取消语义——已删不恢复，如实上报（前端 toast 已删计数）
#[tauri::command]
pub async fn sftp_remove_tree(
    session_id: String,
    path: String,
    task_id: usize,
    total_hint: Option<u64>,
    on_progress: Channel<RemoveProgress>,
    state: State<'_, SftpState>,
) -> Result<RemoveTreeOutcome, String> {
    let client = get_or_connect_sftp(&state, &session_id).await?;
    let target = normalize_remove_path(&path)?;

    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    state.cancel_tokens.lock().await.insert(task_id, cancel_tx);

    // 取消后仍需读到的最新计数（闭包与外层共享）
    let counters = std::sync::Arc::new(std::sync::Mutex::new((0u64, 0u64)));
    let progress_counters = std::sync::Arc::clone(&counters);
    let final_channel = on_progress.clone();
    let fs = SftpTreeFs { client: &client };

    let outcome = tokio::select! {
        _ = cancel_rx => Err("CANCELLED".to_string()),
        summary = remove_tree(&fs, &target, move |deleted, failed| {
            if let Ok(mut c) = progress_counters.lock() {
                *c = (deleted, failed);
            }
            let _ = on_progress.send(RemoveProgress {
                task_id,
                deleted,
                failed,
                total: total_hint,
                is_finished: false,
            });
        }) => Ok(summary),
    };
    remove_cancel_token(&state, task_id).await;

    let (deleted, failed) = *counters.lock().map_err(|e| e.to_string())?;
    let _ = final_channel.send(RemoveProgress {
        task_id,
        deleted,
        failed,
        total: total_hint,
        is_finished: true,
    });

    match outcome {
        Ok(summary) => {
            if summary.failed > 0 {
                Err(format!(
                    "已删除 {} 项，{} 项失败：{}",
                    summary.deleted,
                    summary.failed,
                    summary.first_error.unwrap_or_default()
                ))
            } else {
                Ok(RemoveTreeOutcome { deleted: summary.deleted, failed: 0 })
            }
        }
        Err(_) => Err(format!("CANCELLED:{deleted}")),
    }
}

#[cfg(test)]
mod tree_tests {
    use super::*;

    // ── 引号安全（注入攻击面）──

    #[test]
    fn shell_single_quote_escapes_attacker_inputs() {
        assert_eq!(shell_single_quote(""), "''");
        assert_eq!(shell_single_quote("plain"), "'plain'");
        assert_eq!(shell_single_quote("with space"), "'with space'");
        // 单引号闭合注入：a' → 'a'\''b' 在 shell 中解析为字面 a'b
        assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
        // 命令注入元字符全部落在引号内、原样传递
        assert_eq!(
            shell_single_quote("x'; rm -rf /; '"),
            "'x'\\''; rm -rf /; '\\'''"
        );
        assert_eq!(shell_single_quote("$(id)`id`"), "'$(id)`id`'");
        assert_eq!(shell_single_quote("中文/路径"), "'中文/路径'");
        assert_eq!(shell_single_quote("line\nbreak"), "'line\nbreak'");
    }

    #[test]
    fn fast_remove_command_shape() {
        let cmd = build_fast_remove_command("/srv/data old");
        assert_eq!(cmd, "sh -c 'rm -rf -- \"$1\"' veloterm-rm '/srv/data old'");
        // 含单引号的路径仍只产生一个 argv 位（'\'' 闭合-转义-重开序列）
        let evil = build_fast_remove_command("a'; rm -rf /");
        assert!(evil.ends_with("a'\\''; rm -rf /'"));
        // 以 sh -n 等价性手验：外层解析后恰为 4 个词，$1 为路径本身
    }

    #[test]
    fn fast_remove_timeout_formula() {
        assert_eq!(fast_remove_timeout_secs(None), 600);
        assert_eq!(fast_remove_timeout_secs(Some(0)), 60);
        assert_eq!(fast_remove_timeout_secs(Some(5000)), 110);
        assert_eq!(fast_remove_timeout_secs(Some(1_000_000)), 600); // 封顶
    }

    #[test]
    fn remove_target_guard() {
        assert!(ensure_safe_remove_target("/").is_err());
        assert!(ensure_safe_remove_target("//").is_err());
        assert!(ensure_safe_remove_target("").is_err());
        assert!(ensure_safe_remove_target("   ").is_err());
        assert!(ensure_safe_remove_target("/a").is_ok());
        assert!(ensure_safe_remove_target("/a/").is_ok());
        assert!(ensure_safe_remove_target("relative/x").is_ok());
    }

    // ── 树核心（内存假实现）──

    /// 假远端：HashMap 目录表 + 文件/符号链接集合；fail_on 模拟权限失败
    struct FakeFs {
        dirs: std::collections::HashMap<String, Vec<TreeEntry>>,
        files: std::collections::HashSet<String>,
        fail_on: Option<String>,
    }

    impl FakeFs {
        fn new() -> Self {
            Self { dirs: std::collections::HashMap::new(), files: std::collections::HashSet::new(), fail_on: None }
        }
        fn dir(mut self, path: &str, children: &[(&str, bool, bool)]) -> Self {
            let entries = children
                .iter()
                .map(|(n, d, s)| TreeEntry { name: n.to_string(), is_dir: *d, is_symlink: *s })
                .collect();
            self.dirs.insert(path.to_string(), entries);
            self
        }
        fn file(mut self, path: &str) -> Self {
            self.files.insert(path.to_string());
            self
        }
        fn fail_on(mut self, path: &str) -> Self {
            self.fail_on = Some(path.to_string());
            self
        }
    }

    impl RemoteTreeFs for FakeFs {
        async fn list(&self, path: &str) -> Result<Vec<TreeEntry>, String> {
            match self.dirs.get(path) {
                Some(v) => Ok(v.clone()),
                None => Err(format!("no such dir: {path}")),
            }
        }
        async fn unlink(&self, path: &str) -> Result<(), String> {
            if Some(path) == self.fail_on.as_deref() {
                return Err(format!("permission denied: {path}"));
            }
            Ok(())
        }
        async fn rmdir(&self, path: &str) -> Result<(), String> {
            if Some(path) == self.fail_on.as_deref() {
                return Err(format!("permission denied: {path}"));
            }
            Ok(())
        }
    }

    fn sample_tree() -> FakeFs {
        FakeFs::new()
            .dir("/r", &[("a", true, false), ("f1", false, false), ("link", false, true)])
            .dir("/r/a", &[("f2", false, false), ("b", true, false)])
            .dir("/r/a/b", &[("f3", false, false)])
            .file("/r/f1")
            .file("/r/a/f2")
            .file("/r/a/b/f3")
    }

    #[tokio::test]
    async fn count_tree_walks_without_following_symlinks() {
        let fs = sample_tree();
        // 根 + a + f1 + link + f2 + b + f3 = 7；link 是 symlink 不下钻
        let (count, truncated) = count_tree(&fs, "/r", 100).await;
        assert_eq!(count, 7);
        assert!(!truncated);
    }

    #[tokio::test]
    async fn count_tree_caps_and_reports_truncation() {
        // 100 个文件 + 根：cap=50 必截断
        let mut fs = FakeFs::new();
        let children: Vec<(&str, bool, bool)> =
            (0..100).map(|i| (Box::leak(format!("f{i}").into_boxed_str()) as &str, false, false)).collect();
        fs = fs.dir("/big", &children);
        let (count, truncated) = count_tree(&fs, "/big", 50).await;
        assert!(truncated);
        assert_eq!(count as usize, 50);
    }

    #[tokio::test]
    async fn remove_tree_is_post_order_and_counts_all() {
        let fs = sample_tree();
        let summary = remove_tree(&fs, "/r", |_, _| {}).await;
        assert_eq!(summary.deleted, 7); // 3 文件 + link + 3 目录
        assert_eq!(summary.failed, 0);
    }

    #[tokio::test]
    async fn remove_tree_continues_on_single_failure() {
        let fs = sample_tree().fail_on("/r/a/f2");
        let summary = remove_tree(&fs, "/r", |_, _| {}).await;
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.deleted, 6);
        assert!(summary.first_error.unwrap().contains("permission denied"));
    }

    #[tokio::test]
    async fn remove_tree_handles_deep_chains_iteratively() {
        // 500 层深链：递归实现会栈溢出，显式栈必须无恙
        let mut fs = FakeFs::new();
        let mut children: Vec<TreeEntry> = Vec::new();
        let mut path = String::from("/deep");
        for i in 0..500 {
            let child = format!("{path}/d{i}");
            let mut entries = vec![TreeEntry { name: format!("d{i}"), is_dir: true, is_symlink: false }];
            if i % 50 == 0 {
                entries.push(TreeEntry { name: format!("f{i}"), is_dir: false, is_symlink: false });
            }
            // 用字符串名重建（避免借用）：直接插入下轮目录
            fs.dirs.insert(path.clone(), entries);
            path = child;
        }
        fs.dirs.insert(path, vec![]); // 最底层空目录
        let summary = remove_tree(&fs, "/deep", |_, _| {}).await;
        assert_eq!(summary.failed, 0);
        // 501 个目录（含根 /deep 与末层空目录）+ 每 50 层一个文件共 10 个
        assert_eq!(summary.deleted, 511);
    }
}
