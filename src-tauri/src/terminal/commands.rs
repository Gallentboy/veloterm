use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::Deserialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use super::manager::TerminalManager;
use super::resource_monitor::{self, ResourceSnapshot};
use super::session::{TerminalCreateRequest, TerminalCreateResult};
use super::ssh::{KnownHostEntry, KnownHosts};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum TerminalWritePayload {
    /// 紧凑 Base64 字符串（大幅优化前端 IPC 传输体积与 GC 压力）
    Base64(String),
    /// 兼容旧版及测试环境的原始字节数组
    Bytes(Vec<u8>),
}

impl TerminalWritePayload {
    pub fn into_bytes(self) -> Result<Vec<u8>, String> {
        match self {
            TerminalWritePayload::Base64(s) => {
                BASE64_STANDARD.decode(s.trim()).map_err(|e| format!("Base64 decode failed: {e}"))
            }
            TerminalWritePayload::Bytes(b) => Ok(b),
        }
    }
}

#[tauri::command]
pub async fn terminal_create(
    app: AppHandle,
    manager: State<'_, TerminalManager>,
    request: TerminalCreateRequest,
    on_output: Channel<Vec<u8>>,
    on_exit: Channel<i32>,
) -> Result<TerminalCreateResult, String> {
    manager.create(request, on_output, on_exit, app)
}

#[tauri::command]
pub async fn terminal_write(
    manager: State<'_, TerminalManager>,
    id: usize,
    data: TerminalWritePayload,
) -> Result<(), String> {
    let bytes = data.into_bytes()?;
    // 改密输入捕获旁路（提示词激活时镜像记录，不影响转发）
    super::passwd_input::PasswdInputLog::global().feed_input(id, &bytes);
    manager.write(id, &bytes)
}

#[tauri::command]
pub async fn terminal_resize(
    manager: State<'_, TerminalManager>,
    id: usize,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    manager.resize(id, cols, rows)
}

#[tauri::command]
pub async fn terminal_dispose(
    manager: State<'_, TerminalManager>,
    id: usize,
) -> Result<(), String> {
    manager.dispose(id)
}

/// SSH 远程主机资源快照（延迟 / CPU / 内存 / SWAP / 网络速率 / 磁盘）：
/// 对既有连接做一次旁路 exec 采集（独立通道，不碰交互 shell），rtt 为该
/// 命令往返耗时。CPU% 与网络速率依赖前次采样差分，首拍为 null；
/// 仅 SSH 会话支持（本地 PTY 回 Err）。前端对可见会话轮询调用
#[tauri::command]
pub async fn ssh_resource_snapshot(
    manager: State<'_, TerminalManager>,
    id: usize,
) -> Result<ResourceSnapshot, String> {
    let started = std::time::Instant::now();
    let output = manager
        .exec_capture(id, resource_monitor::RESOURCE_COMMAND)
        .await?;
    let rtt_ms = started.elapsed().as_millis() as u64;
    Ok(resource_monitor::build_snapshot(id, &output, rtt_ms))
}

#[tauri::command]
pub async fn ssh_list_known_hosts() -> Result<Vec<KnownHostEntry>, String> {
    KnownHosts::global().list()
}

#[tauri::command]
pub async fn ssh_remove_known_host(id: usize) -> Result<bool, String> {
    KnownHosts::global().remove(id)
}

#[tauri::command]
pub async fn ssh_clear_known_hosts() -> Result<usize, String> {
    KnownHosts::global().clear()
}

/// 同步终端联动（广播）组成员给后端：ZMODEM 命中聚合用（一次交互 N 路扇出）。
/// `session_ids` 为当前勾选的联动集合；空数组 = 关闭联动。
#[tauri::command]
pub async fn terminal_set_broadcast_group(
    session_ids: Vec<usize>,
    aggregator: tauri::State<'_, std::sync::Arc<crate::terminal::zmodem::ZmodemAggregator>>,
) -> Result<(), String> {
    if session_ids.is_empty() {
        aggregator.clear_all();
        return Ok(());
    }
    // 组键固定为 "broadcast"（前端只有一组联动集合）
    aggregator.set_group("broadcast", &session_ids);
    Ok(())
}

/// 取消进行中的 ZMODEM 传输：向服务器发中止序列（CAN×10+BS×10）并让引擎退出。
/// 前端浮层的「取消」按钮调用；会话关闭时后端也会自动执行同样清理。
#[tauri::command]
pub async fn terminal_cancel_zmodem(session_id: usize) -> Result<(), String> {
    crate::terminal::zmodem::request_cancel(session_id);
    Ok(())
}

