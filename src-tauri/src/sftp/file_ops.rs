//! 远程文件在线编辑的 SFTP 读写命令。
//!
//! 走既有 [`super::manager::get_or_connect_sftp`] 通道：`session_id` 传
//! `host_id` 字符串即可命中 manager 的按 host_id 动态连接逻辑，manager 零改动。
//!
//! - 读取：先 stat 校验大小上限，再读前 512 字节做文本嗅探，通过后全量读取；
//! - 写入：同目录临时文件 + rename 原子覆盖（SFTP v3 rename 在目标已存在时
//!   会被 OpenSSH 拒绝，故失败时回退为「删除目标后再 rename」）。

use serde::Serialize;
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use super::manager::get_or_connect_sftp;
use super::types::SftpState;
use crate::text_detect::{MAX_EDITABLE_FILE_SIZE_BYTES, is_editable_text};

/// 文本嗅探探针长度（与 `is_text_buffer` 约定一致）。
const SNIFF_PROBE_BYTES: usize = 512;

/// `sftp_read_file` 的返回载荷（camelCase 契约）。
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SftpFileContent {
  pub content: String,
  pub size: u64,
}

/// 从远端路径提取纯文件名（最后一个 `/` 之后的部分）。
fn file_name_of(remote_path: &str) -> &str {
  remote_path.rsplit('/').next().unwrap_or(remote_path)
}

/// 生成与目标文件同目录的临时文件路径：`<dir>/.veloterm.tmp.<uuid>`。
fn sibling_temp_path(remote_path: &str) -> String {
  let temp_name = format!(".veloterm.tmp.{}", Uuid::new_v4());
  match remote_path.rsplit_once('/') {
    // "/a.txt" → "/.veloterm.tmp.<uuid>"（根目录）
    Some(("", _)) => format!("/{temp_name}"),
    Some((dir, _)) => format!("{dir}/{temp_name}"),
    // 相对路径（无 '/'）→ 临时文件落在 SFTP 工作目录
    None => temp_name,
  }
}

/// 读取远端文本文件内容（大小上限 + 文本嗅探校验）。
#[tauri::command]
pub async fn sftp_read_file(
  host_id: i64,
  remote_path: String,
  state: State<'_, SftpState>,
) -> Result<SftpFileContent, String> {
  let session_key = host_id.to_string();
  let client = get_or_connect_sftp(&state, &session_key).await?;

  // 1. stat 校验大小上限
  let meta = client
    .metadata(&remote_path)
    .await
    .map_err(|e| format!("Failed to stat remote file '{remote_path}': {e}"))?;
  let size = meta.len();
  if size > MAX_EDITABLE_FILE_SIZE_BYTES {
    return Err("文件超过 5MB，不可编辑".to_string());
  }

  // 2. 打开文件，读前 512 字节做文本嗅探
  let mut remote_file = client
    .open(&remote_path)
    .await
    .map_err(|e| format!("Failed to open remote file '{remote_path}': {e}"))?;

  let mut probe = vec![0u8; SNIFF_PROBE_BYTES.min(size as usize)];
  let mut probed = 0usize;
  while probed < probe.len() {
    let n = remote_file
      .read(&mut probe[probed..])
      .await
      .map_err(|e| format!("Failed to read remote file probe: {e}"))?;
    if n == 0 {
      break;
    }
    probed += n;
  }
  probe.truncate(probed);

  let name = file_name_of(&remote_path);
  if !is_editable_text(Some(name), &probe) {
    return Err("不是可编辑的文本文件".to_string());
  }

  // 3. 嗅探通过后全量读取（探针字节复用，不重复读）
  let mut data = probe;
  data.reserve((size as usize).saturating_sub(data.len()));
  let mut buffer = [0u8; 65536];
  loop {
    let n = remote_file
      .read(&mut buffer)
      .await
      .map_err(|e| format!("Failed to read remote file '{remote_path}': {e}"))?;
    if n == 0 {
      break;
    }
    data.extend_from_slice(&buffer[..n]);
    if data.len() as u64 > MAX_EDITABLE_FILE_SIZE_BYTES {
      return Err("文件超过 5MB，不可编辑".to_string());
    }
  }

  let size = data.len() as u64;
  Ok(SftpFileContent {
    content: String::from_utf8_lossy(&data).into_owned(),
    size,
  })
}

/// 原子写入远端文件：同目录临时文件写入成功后 rename 覆盖目标。
#[tauri::command]
pub async fn sftp_write_file(
  host_id: i64,
  remote_path: String,
  content: String,
  state: State<'_, SftpState>,
) -> Result<(), String> {
  let session_key = host_id.to_string();
  let client = get_or_connect_sftp(&state, &session_key).await?;

  let temp_path = sibling_temp_path(&remote_path);

  // 1. 写临时文件（create = O_WRONLY|O_CREAT|O_TRUNC）
  let mut temp_file = client
    .create(&temp_path)
    .await
    .map_err(|e| format!("Failed to create temp file '{temp_path}': {e}"))?;
  temp_file
    .write_all(content.as_bytes())
    .await
    .map_err(|e| format!("Failed to write temp file '{temp_path}': {e}"))?;
  temp_file
    .shutdown()
    .await
    .map_err(|e| format!("Failed to flush temp file '{temp_path}': {e}"))?;

  // 2. 尽力保留原文件权限（原文件不存在或 setstat 不受支持时忽略）
  if let Ok(origin_meta) = client.metadata(&remote_path).await
    && let Some(perms) = origin_meta.permissions
  {
    let attrs = russh_sftp::protocol::FileAttributes {
      permissions: Some(perms),
      ..Default::default()
    };
    let _ = client.set_metadata(&temp_path, attrs).await;
  }

  // 3. rename 覆盖目标；SFTP v3 rename 在目标已存在时会被部分服务器
  //    （如 OpenSSH sftp-server）拒绝，此时回退为「删除目标后再 rename」。
  if let Err(first_err) = client.rename(&temp_path, &remote_path).await {
    let _ = client.remove_file(&remote_path).await;
    if let Err(retry_err) = client.rename(&temp_path, &remote_path).await {
      let _ = client.remove_file(&temp_path).await;
      return Err(format!(
        "Failed to rename temp file onto '{remote_path}': {first_err}; retry after remove: {retry_err}"
      ));
    }
  }

  log::info!(
    "[SFTP] Atomically wrote {} bytes to '{remote_path}'",
    content.len()
  );
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn file_name_extraction() {
    assert_eq!(file_name_of("/etc/nginx/nginx.conf"), "nginx.conf");
    assert_eq!(file_name_of("/hosts"), "hosts");
    assert_eq!(file_name_of("relative.txt"), "relative.txt");
  }

  #[test]
  fn temp_paths_stay_in_same_directory() {
    let path = sibling_temp_path("/etc/nginx/nginx.conf");
    assert!(
      path.starts_with("/etc/nginx/.veloterm.tmp."),
      "temp file must be sibling of target: {path}"
    );

    let root = sibling_temp_path("/hosts");
    assert!(root.starts_with("/.veloterm.tmp."), "root-level temp: {root}");

    let relative = sibling_temp_path("notes.md");
    assert!(
      relative.starts_with(".veloterm.tmp."),
      "relative temp: {relative}"
    );
  }

  #[test]
  fn read_payload_serializes_camel_case() {
    let payload = SftpFileContent {
      content: "hello".to_string(),
      size: 5,
    };
    let json = serde_json::to_string(&payload).unwrap();
    assert!(json.contains("\"content\":\"hello\""));
    assert!(json.contains("\"size\":5"));
  }
}
