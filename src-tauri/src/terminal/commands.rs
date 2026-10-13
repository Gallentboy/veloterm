use tauri::State;
use tauri::ipc::Channel;

use super::manager::TerminalManager;
use super::session::{TerminalCreateRequest, TerminalCreateResult};
use super::ssh::{KnownHostEntry, KnownHosts};

#[tauri::command]
pub async fn terminal_create(
  manager: State<'_, TerminalManager>,
  request: TerminalCreateRequest,
  on_output: Channel<Vec<u8>>,
  on_exit: Channel<i32>,
) -> Result<TerminalCreateResult, String> {
  manager.create(request, on_output, on_exit)
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
