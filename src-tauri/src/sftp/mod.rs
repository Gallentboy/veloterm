pub mod commands;
pub mod file_ops;
pub mod manager;
pub mod types;

pub use commands::*;
pub use file_ops::*;
pub use manager::*;
pub use types::*;

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_sftp_state_task_id_allocation() {
    let state = SftpState::new();
    let id1 = state.allocate_task_id();
    let id2 = state.allocate_task_id();
    let id3 = state.allocate_task_id();

    assert_eq!(id1, 1);
    assert_eq!(id2, 2);
    assert_eq!(id3, 3);
  }

  #[tokio::test]
  async fn test_sftp_cancel_token_signaling() {
    let state = SftpState::new();
    let task_id = state.allocate_task_id();
    let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();

    {
      let mut tokens = state.cancel_tokens.lock().await;
      tokens.insert(task_id, tx);
    }

    // 取消任务
    {
      let mut tokens = state.cancel_tokens.lock().await;
      if let Some(sender) = tokens.remove(&task_id) {
        let _ = sender.send(());
      }
    }

    // 验证已收到取消信号
    assert!(rx.try_recv().is_ok());
  }

  #[test]
  fn test_remote_file_item_serialization() {
    let item = RemoteFileItem {
      name: "large_file.iso".to_string(),
      path: "/var/data/large_file.iso".to_string(),
      is_dir: false,
      is_symlink: false,
      size: "10995116277760".to_string(), // 10TB 大数字节字符串防溢出
      modified_at: 1770000000,
      permissions: 0o644,
      owner: "zhangyue".to_string(),
      group: "staff".to_string(),
      editable: false,
    };

    let serialized = serde_json::to_string(&item).unwrap();
    assert!(serialized.contains("\"size\":\"10995116277760\""));
    assert!(serialized.contains("\"is_dir\":false"));
    assert!(serialized.contains("\"owner\":\"zhangyue\""));
    assert!(serialized.contains("\"group\":\"staff\""));
    assert!(serialized.contains("\"editable\":false"));

    let deserialized: RemoteFileItem = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized.name, "large_file.iso");
    assert_eq!(deserialized.size, "10995116277760");
    assert_eq!(deserialized.owner, "zhangyue");
    assert_eq!(deserialized.group, "staff");
  }

  #[test]
  fn test_transfer_progress_serialization() {
    let progress = TransferProgress {
      task_id: 42,
      transferred: "5242880".to_string(),
      total: "10485760".to_string(),
      speed_bps: 1048576,
      percent: 50.0,
      is_finished: false,
    };

    let json = serde_json::to_string(&progress).unwrap();
    assert!(json.contains("\"task_id\":42"));
    assert!(json.contains("\"percent\":50.0"));
    assert!(json.contains("\"is_finished\":false"));
  }
}

