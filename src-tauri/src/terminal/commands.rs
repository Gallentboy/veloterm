use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use super::manager::TerminalManager;
use super::resource_monitor::{self, ResourceSnapshot};
use super::session::{TerminalCreateRequest, TerminalCreateResult};
use super::ssh::{KnownHostEntry, KnownHosts};

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
    data: Vec<u8>,
) -> Result<(), String> {
    manager.write(id, &data)
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
