use tauri::ipc::Channel;
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::manager::{close_sftp, get_or_connect_sftp, resolve_host_chain, connect_sftp};
use super::types::{RemoteFileItem, SftpState, TransferProgress};

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
pub async fn sftp_close(
  session_id: String,
  state: State<'_, SftpState>,
) -> Result<(), String> {
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
    .map_err(|e| format!("Failed to read remote directory '{normalized_path}': {e}"))?;

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
  on_progress: Channel<TransferProgress>,
  state: State<'_, SftpState>,
) -> Result<(), String> {
  let client = get_or_connect_sftp(&state, &session_id).await?;

  let mut remote_file = client
    .open(&remote_path)
    .await
    .map_err(|e| format!("Failed to open remote file '{remote_path}': {e}"))?;

  let total_size = remote_file
    .metadata()
    .await
    .map(|m| m.len())
    .unwrap_or(0);

  if let Some(parent) = std::path::Path::new(&local_path).parent() {
    tokio::fs::create_dir_all(parent)
      .await
      .map_err(|e| format!("Failed to create local directory: {e}"))?;
  }

  let mut local_file = tokio::fs::File::create(&local_path)
    .await
    .map_err(|e| format!("Failed to create local file '{local_path}': {e}"))?;

  let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
  {
    let mut tokens = state.cancel_tokens.lock().await;
    tokens.insert(task_id, cancel_tx);
  }

  let mut buffer = [0u8; 65536];
  let mut transferred: u64 = 0;
  let mut last_tick = tokio::time::Instant::now();
  let mut bytes_since_last_tick: u64 = 0;

  loop {
    tokio::select! {
      _ = &mut cancel_rx => {
        state.cancel_tokens.lock().await.remove(&task_id);
        log::info!("[SFTP] Download task {task_id} cancelled by user");
        return Err("Transfer cancelled by user".to_string());
      }
      res = remote_file.read(&mut buffer) => {
        let n = res.map_err(|e| format!("Failed to read from remote file: {e}"))?;
        if n == 0 {
          break; // EOF
        }
        local_file
          .write_all(&buffer[..n])
          .await
          .map_err(|e| format!("Failed to write to local file: {e}"))?;

        transferred += n as u64;
        bytes_since_last_tick += n as u64;

        let now = tokio::time::Instant::now();
        let elapsed = now.duration_since(last_tick);
        if elapsed >= tokio::time::Duration::from_millis(100) {
          let speed_bps = (bytes_since_last_tick as f64 / elapsed.as_secs_f64()) as u64;
          let percent = if total_size > 0 {
            (transferred as f64 / total_size as f64 * 100.0).clamp(0.0, 100.0) as f32
          } else {
            0.0
          };

          let _ = on_progress.send(TransferProgress {
            task_id,
            transferred: transferred.to_string(),
            total: total_size.to_string(),
            speed_bps,
            percent,
            is_finished: false,
          });

          last_tick = now;
          bytes_since_last_tick = 0;
        }
      }
    }
  }

  local_file
    .flush()
    .await
    .map_err(|e| format!("Failed to flush local file: {e}"))?;

  // 发送 100% 完成包
  let _ = on_progress.send(TransferProgress {
    task_id,
    transferred: transferred.to_string(),
    total: total_size.to_string(),
    speed_bps: 0,
    percent: 100.0,
    is_finished: true,
  });

  state.cancel_tokens.lock().await.remove(&task_id);
  log::info!("[SFTP] Download task {task_id} completed: {transferred} bytes");
  Ok(())
}

#[tauri::command]
pub async fn sftp_upload_file(
  session_id: String,
  task_id: usize,
  local_path: String,
  remote_path: String,
  on_progress: Channel<TransferProgress>,
  state: State<'_, SftpState>,
) -> Result<(), String> {
  let client = get_or_connect_sftp(&state, &session_id).await?;

  let mut local_file = tokio::fs::File::open(&local_path)
    .await
    .map_err(|e| format!("Failed to open local file '{local_path}': {e}"))?;

  let total_size = local_file
    .metadata()
    .await
    .map(|m| m.len())
    .unwrap_or(0);

  let mut remote_file = client
    .create(&remote_path)
    .await
    .map_err(|e| format!("Failed to create remote file '{remote_path}': {e}"))?;

  let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
  {
    let mut tokens = state.cancel_tokens.lock().await;
    tokens.insert(task_id, cancel_tx);
  }

  let mut buffer = [0u8; 65536];
  let mut transferred: u64 = 0;
  let mut last_tick = tokio::time::Instant::now();
  let mut bytes_since_last_tick: u64 = 0;

  loop {
    tokio::select! {
      _ = &mut cancel_rx => {
        state.cancel_tokens.lock().await.remove(&task_id);
        log::info!("[SFTP] Upload task {task_id} cancelled by user");
        return Err("Transfer cancelled by user".to_string());
      }
      res = local_file.read(&mut buffer) => {
        let n = res.map_err(|e| format!("Failed to read from local file: {e}"))?;
        if n == 0 {
          break; // EOF
        }
        remote_file
          .write_all(&buffer[..n])
          .await
          .map_err(|e| format!("Failed to write to remote file: {e}"))?;

        transferred += n as u64;
        bytes_since_last_tick += n as u64;

        let now = tokio::time::Instant::now();
        let elapsed = now.duration_since(last_tick);
        if elapsed >= tokio::time::Duration::from_millis(100) {
          let speed_bps = (bytes_since_last_tick as f64 / elapsed.as_secs_f64()) as u64;
          let percent = if total_size > 0 {
            (transferred as f64 / total_size as f64 * 100.0).clamp(0.0, 100.0) as f32
          } else {
            0.0
          };

          let _ = on_progress.send(TransferProgress {
            task_id,
            transferred: transferred.to_string(),
            total: total_size.to_string(),
            speed_bps,
            percent,
            is_finished: false,
          });

          last_tick = now;
          bytes_since_last_tick = 0;
        }
      }
    }
  }

  remote_file
    .shutdown()
    .await
    .map_err(|e| format!("Failed to shutdown remote file write stream: {e}"))?;

  // 发送 100% 完成包
  let _ = on_progress.send(TransferProgress {
    task_id,
    transferred: transferred.to_string(),
    total: total_size.to_string(),
    speed_bps: 0,
    percent: 100.0,
    is_finished: true,
  });

  state.cancel_tokens.lock().await.remove(&task_id);
  log::info!("[SFTP] Upload task {task_id} completed: {transferred} bytes");
  Ok(())
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
  log::info!("[SFTP] Deleting {} at {}", if is_dir { "directory" } else { "file" }, target_path);

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


