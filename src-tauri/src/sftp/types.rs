use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, oneshot};

pub type SftpClient = Arc<russh_sftp::client::SftpSession>;

/// SFTP 连接的底层 SSH 句柄：同连接可开独立 exec 通道跑旁路命令
/// （目录大小 du / uid-gid 名称解析 getent），与 sftp 子系统通道互不干扰
/// （SSH 多路复用；同型先例：终端侧 RCCommand::ExecCapture）
pub type SftpSshHandle =
    russh::client::Handle<crate::terminal::ssh::handler::ClientHandler>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RemoteFileItem {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: String,     // 物理文件大数字节，序列化为字符串防越界
    pub modified_at: u64, // Unix 秒级时间戳
    pub permissions: u32,
    pub owner: String,  // 所有者 (如 zhangyue 或 uid)
    pub group: String,  // 所属用户组 (如 staff 或 gid)
    pub editable: bool, // 大小/后缀判定为可在线编辑的文本文件（目录与符号链接恒为 false）
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

/// 目录递归删除的扫描结果（计数带上限：超限即 truncated=true，前端显示「N+」
/// 并按「大量文件」走极速删除询问流程）
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TreeScanResult {
    pub node_count: u64,
    pub truncated: bool,
}

/// Shell 极速删除的预览载荷：command 为后端用与执行完全相同的构造函数生成
/// （单一事实源——展示什么就执行什么；执行命令绝不信前端回传）
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FastRemovePreview {
    pub command: String,
    pub timeout_secs: u64,
}

/// 递归删除进度事件（纯 SFTP 逐文件路径；Channel 直发，同 TransferProgress 先例）
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RemoveProgress {
    pub task_id: usize,
    pub deleted: u64,
    pub failed: u64,
    /// 总项数（扫描计数所得；扫描超限时为 None，前端显示不定进度）
    pub total: Option<u64>,
    pub is_finished: bool,
}

/// uid/gid → 名称解析缓存键（session_id, is_group, id）
pub type IdNameKey = (String, bool, u32);

pub struct SftpState {
    pub clients: Arc<Mutex<HashMap<String, SftpClient>>>,
    /// SFTP 连接的 SSH 句柄（与 clients 同生共死：旁路 exec 通道用）
    pub ssh_handles: Arc<Mutex<HashMap<String, std::sync::Arc<SftpSshHandle>>>>,
    /// uid/gid → 名称解析缓存（getent 一次性解析后落缓存，后续列目录零 exec；
    /// 同会话内映射不变，天然安全）
    pub id_name_cache: Arc<Mutex<HashMap<IdNameKey, String>>>,
    /// exec 旁路通道不可用的会话（堡垒机禁 shell/ForceCommand internal-sftp 等）：
    /// 一次失败即标记，后续列目录直接跳过名称解析——否则黑洞型拒绝（通道
    /// 打开后挂起）会让每次列目录都等满超时。重连（evict/close）时清除重试
    pub exec_disabled: Arc<Mutex<HashSet<String>>>,
    pub cancel_tokens: Arc<Mutex<HashMap<usize, oneshot::Sender<()>>>>,
    pub next_task_id: AtomicUsize,
    /// 串行化动态建连：并发请求共享一次握手，避免重复建 SSH 连接
    pub connect_lock: Arc<Mutex<()>>,
    /// 会话写过的临时断点文件目录（session_id → 目录集合）：
    /// 重连时用于扫描并清理超过 1 天的 `.veloterm.tmp.*` 遗留孤儿断点
    pub temp_dirs: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    /// AppHandle（setup 后注入）：SFTP 生命周期事件发射用
    pub app: std::sync::Mutex<Option<tauri::AppHandle>>,
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
            ssh_handles: Arc::new(Mutex::new(HashMap::new())),
            id_name_cache: Arc::new(Mutex::new(HashMap::new())),
            exec_disabled: Arc::new(Mutex::new(HashSet::new())),
            cancel_tokens: Arc::new(Mutex::new(HashMap::new())),
            next_task_id: AtomicUsize::new(1),
            connect_lock: Arc::new(Mutex::new(())),
            temp_dirs: Arc::new(Mutex::new(HashMap::new())),
            app: std::sync::Mutex::new(None),
        }
    }

    /// setup 后注入 AppHandle，供 SFTP 生命周期事件发射
    pub fn set_app(&self, app: tauri::AppHandle) {
        if let Ok(mut slot) = self.app.lock() {
            *slot = Some(app);
        }
    }

    /// 便捷发射 SFTP 生命周期事件（AppHandle 未注入时静默跳过）
    pub fn emit_sftp(
        &self,
        kind: crate::lifecycle::LifecycleKind,
        session_id: &str,
        host: &str,
        error: Option<&str>,
    ) {
        let app = self.app.lock().ok().and_then(|slot| slot.clone());
        if let Some(app) = app {
            let host_id = session_id.parse::<usize>().ok();
            crate::lifecycle::emit_sftp_lifecycle(
                &app,
                kind,
                host_id.unwrap_or(0),
                host_id,
                host,
                22,
                error,
            );
        }
    }

    #[inline(always)]
    pub fn allocate_task_id(&self) -> usize {
        crate::common::next_safe_usize(&self.next_task_id)
    }
}
