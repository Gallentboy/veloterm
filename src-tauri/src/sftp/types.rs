use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, Mutex};

pub type SftpClient = Arc<russh_sftp::client::SftpSession>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RemoteFileItem {
  pub name: String,
  pub path: String,
  pub is_dir: bool,
  pub is_symlink: bool,
  pub size: String,     // 物理文件大数字节，序列化为字符串防越界
  pub modified_at: u64, // Unix 秒级时间戳
  pub permissions: u32,
  pub owner: String,    // 所有者 (如 zhangyue 或 uid)
  pub group: String,    // 所属用户组 (如 staff 或 gid)
  pub editable: bool,   // 大小/后缀判定为可在线编辑的文本文件（目录与符号链接恒为 false）
}

#[derive(Debug, Serialize, Clone)]
pub struct TransferProgress {
  pub task_id: usize,      // 任务主键 ID 统一使用 usize (前端对应 number)
  pub transferred: String, // 已传输字节数字符串
  pub total: String,       // 总字节数字符串
  pub speed_bps: u64,      // 即时字节速率
  pub percent: f32,        // 进度百分比 (0.0 ~ 100.0)
  pub is_finished: bool,
}

pub struct SftpState {
  pub clients: Arc<Mutex<HashMap<String, SftpClient>>>,
  pub cancel_tokens: Arc<Mutex<HashMap<usize, oneshot::Sender<()>>>>,
  pub next_task_id: AtomicUsize,
  /// 串行化动态建连：并发请求共享一次握手，避免重复建 SSH 连接
  pub connect_lock: Arc<Mutex<()>>,
}

impl Default for SftpState {
  fn default() -> Self {
    Self::new()
  }
}

impl SftpState {
  pub fn new() -> Self {
    Self {
      clients: Arc::new(Mutex::new(HashMap::new())),
      cancel_tokens: Arc::new(Mutex::new(HashMap::new())),
      next_task_id: AtomicUsize::new(1),
      connect_lock: Arc::new(Mutex::new(())),
    }
  }

  #[inline(always)]
  pub fn allocate_task_id(&self) -> usize {
    crate::common::next_safe_usize(&self.next_task_id)
  }
}
