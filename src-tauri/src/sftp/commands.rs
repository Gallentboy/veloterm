use tauri::ipc::Channel;
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::file_ops::{commit_temp_file, sibling_temp_path};
use super::manager::{close_sftp, get_or_connect_sftp, resolve_host_chain, connect_sftp};
use super::types::{RemoteFileItem, SftpState, TransferProgress};
use crate::text_detect::is_editable_sftp_entry;

/// 简单令牌桶限速器：按累计字节与预期耗时对比，超前则睡眠补齐。
/// speed_limit_bps 为 0/None 表示不限速；允许 1 秒级别的突发。
struct SpeedLimiter {
  limit_bps: u64,
  window_start: tokio::time::Instant,
  window_bytes: u64,
}

impl SpeedLimiter {
  fn new(limit_bps: Option<u64>) -> Option<Self> {
    let limit_bps = limit_bps.filter(|v| *v > 0)?;
    Some(Self {
      limit_bps,
      window_start: tokio::time::Instant::now(),
      window_bytes: 0,
    })
  }

  async fn throttle(&mut self, chunk_bytes: u64) {
    self.window_bytes += chunk_bytes;
    let expected = tokio::time::Duration::from_secs_f64(
      self.window_bytes as f64 / self.limit_bps as f64,
    );
    let elapsed = self.window_start.elapsed();
    if expected > elapsed {
      tokio::time::sleep(expected - elapsed).await;
    } else if elapsed > expected + tokio::time::Duration::from_secs(1) {
      // 窗口内已落后于配额（低速读源），重置窗口避免长期欠账后限死
      self.window_start = tokio::time::Instant::now();
      self.window_bytes = 0;
    }
  }
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
  speed_limit_bps: Option<u64>,
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
  let mut limiter = SpeedLimiter::new(speed_limit_bps);

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
        let Some(chunk) = buffer.get(..n) else {
          break;
        };
        local_file
          .write_all(chunk)
          .await
          .map_err(|e| format!("Failed to write to local file: {e}"))?;

        if let Some(ref mut limiter) = limiter {
          limiter.throttle(n as u64).await;
        }
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
  speed_limit_bps: Option<u64>,
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
  let mut limiter = SpeedLimiter::new(speed_limit_bps);

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
        let Some(chunk) = buffer.get(..n) else {
          break;
        };
        remote_file
          .write_all(chunk)
          .await
          .map_err(|e| format!("Failed to write to remote file: {e}"))?;
        if let Some(ref mut limiter) = limiter {
          limiter.throttle(n as u64).await;
        }

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

/// 远端到远端的流式复制：源会话读、目标会话写，数据全程不落本地磁盘。
/// 两个 session_id 相同时命中同一缓存连接（同主机复制），不同则为跨主机搬运。
/// 目标侧写同目录临时文件，成功后 rename 原子覆盖（与在线编辑写入一致）。
#[tauri::command]
pub async fn sftp_copy_file(
  state: State<'_, SftpState>,
  task_id: usize,
  source_session_id: String,
  source_path: String,
  target_session_id: String,
  target_path: String,
  speed_limit_bps: Option<u64>,
  on_progress: Channel<TransferProgress>,
) -> Result<(), String> {
  // 注册取消令牌（与 download/upload 一致，按 task_id 索引）
  let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
  {
    let mut tokens = state.cancel_tokens.lock().await;
    tokens.insert(task_id, cancel_tx);
  }

  // 解析两端客户端；任一失败时回收取消令牌
  let source_client = match get_or_connect_sftp(&state, &source_session_id).await {
    Ok(client) => client,
    Err(e) => {
      state.cancel_tokens.lock().await.remove(&task_id);
      return Err(e);
    }
  };
  let target_client = match get_or_connect_sftp(&state, &target_session_id).await {
    Ok(client) => client,
    Err(e) => {
      state.cancel_tokens.lock().await.remove(&task_id);
      return Err(e);
    }
  };

  let mut source_file = match source_client.open(&source_path).await {
    Ok(file) => file,
    Err(e) => {
      state.cancel_tokens.lock().await.remove(&task_id);
      return Err(format!("Failed to open source file '{source_path}': {e}"));
    }
  };

  // stat 源文件取总大小与权限（失败时容忍：进度无百分比、不保留权限）
  let meta = source_file.metadata().await.unwrap_or_default();
  let total_size = meta.len();
  let source_perms = meta.permissions;

  // 目标侧写同目录临时文件
  let temp_path = sibling_temp_path(&target_path);
  let mut temp_file = match target_client.create(&temp_path).await {
    Ok(file) => file,
    Err(e) => {
      state.cancel_tokens.lock().await.remove(&task_id);
      return Err(format!("Failed to create temp file '{temp_path}': {e}"));
    }
  };

  let mut buffer = [0u8; 65536];
  let mut transferred: u64 = 0;
  let mut last_tick = tokio::time::Instant::now();
  let mut bytes_since_last_tick: u64 = 0;
  let mut limiter = SpeedLimiter::new(speed_limit_bps);

  loop {
    tokio::select! {
      _ = &mut cancel_rx => {
        state.cancel_tokens.lock().await.remove(&task_id);
        // 尽力清理半成品临时文件
        let _ = target_client.remove_file(&temp_path).await;
        log::info!("[SFTP] Copy task {task_id} cancelled by user");
        return Err("Transfer cancelled by user".to_string());
      }
      res = source_file.read(&mut buffer) => {
        let n = match res {
          Ok(n) => n,
          Err(e) => {
            // 中途失败：回收取消令牌并清理半成品临时文件
            state.cancel_tokens.lock().await.remove(&task_id);
            let _ = target_client.remove_file(&temp_path).await;
            return Err(format!("Failed to read from source file: {e}"));
          }
        };
        if n == 0 {
          break; // EOF
        }
        let Some(chunk) = buffer.get(..n) else {
          break;
        };
        if let Err(e) = temp_file.write_all(chunk).await {
          // 中途失败：回收取消令牌并清理半成品临时文件
          state.cancel_tokens.lock().await.remove(&task_id);
          let _ = target_client.remove_file(&temp_path).await;
          return Err(format!("Failed to write to temp file: {e}"));
        }

        if let Some(ref mut limiter) = limiter {
          limiter.throttle(n as u64).await;
        }
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

  if let Err(e) = temp_file.shutdown().await {
    state.cancel_tokens.lock().await.remove(&task_id);
    let _ = target_client.remove_file(&temp_path).await;
    return Err(format!("Failed to flush temp file '{temp_path}': {e}"));
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
    state.cancel_tokens.lock().await.remove(&task_id);
    return Err(e);
  }

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
  log::info!("[SFTP] Copy task {task_id} completed: {transferred} bytes");
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


