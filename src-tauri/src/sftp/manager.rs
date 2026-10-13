use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::terminal::ssh::{
  connect_ssh_chain, KnownHosts, TargetSSHOptions,
};
use super::types::{SftpClient, SftpState};

fn record_to_ssh_options(record: &crate::db::HostRecord) -> TargetSSHOptions {
  TargetSSHOptions {
    host: record.host.clone(),
    port: record.port,
    username: if record.username.trim().is_empty() {
      "root".to_string()
    } else {
      record.username.clone()
    },
    allow_insecure_algos: record.allow_insecure_algos,
    auth: record.auth.clone(),
    jump_host: None,
    host_key_verification: record.host_key_verification,
  }
}

pub fn resolve_host_chain(host_id: usize) -> Result<Vec<TargetSSHOptions>, String> {
  let pool = crate::db::global_pool().ok_or_else(|| "Database connection pool not ready".to_string())?;
  let db_store = crate::db::load_host_store(&pool).map_err(|e| format!("Failed to load host store: {e}"))?;

  let target_record = db_store
    .hosts
    .iter()
    .find(|h| h.id == host_id)
    .cloned()
    .ok_or_else(|| format!("Host not found in database (id={host_id})"))?;

  let by_id: HashMap<usize, &crate::db::HostRecord> =
    db_store.hosts.iter().map(|h| (h.id, h)).collect();

  let mut hops = Vec::new();
  let mut visited = HashSet::from([target_record.id]);
  let mut cursor = target_record.jump_host_id;

  while let Some(jid) = cursor {
    if !visited.insert(jid) {
      return Err(format!("SSH jump host cycle detected at host id {jid}"));
    }
    let Some(hop) = by_id.get(&jid) else {
      return Err(format!("SSH jump host id {jid} not found"));
    };
    hops.insert(0, record_to_ssh_options(hop));
    cursor = hop.jump_host_id;
  }

  hops.push(record_to_ssh_options(&target_record));
  Ok(hops)
}

pub async fn connect_sftp(chain: Vec<TargetSSHOptions>) -> Result<SftpClient, String> {
  let known_hosts = KnownHosts::global();
  let session_id = 0; // 内部连接序号
  let connection = connect_ssh_chain(session_id, chain, known_hosts)
    .await
    .map_err(|e| format!("SSH connection failed: {e}"))?;

  let session = connection.0;
  let mut active_rx = connection.1;

  // 保持连接事件接收协程，防止未消费事件阻塞 SSH 状态机
  tokio::spawn(async move {
    while let Some(_event) = active_rx.recv().await {}
  });

  let channel = session
    .channel_open_session()
    .await
    .map_err(|e| format!("Failed to open SSH channel for SFTP: {e}"))?;

  channel
    .request_subsystem(true, "sftp")
    .await
    .map_err(|e| format!("Failed to request 'sftp' subsystem: {e}"))?;

  let stream = channel.into_stream();
  let sftp_session = russh_sftp::client::SftpSession::new(stream)
    .await
    .map_err(|e| format!("Failed to initialize SFTP session: {e}"))?;

  Ok(Arc::new(sftp_session))
}

pub async fn get_or_connect_sftp(
  state: &SftpState,
  session_id: &str,
) -> Result<SftpClient, String> {
  {
    let clients = state.clients.lock().await;
    if let Some(client) = clients.get(session_id) {
      return Ok(Arc::clone(client));
    }
  }

  // 尝试将 session_id 解析为 host_id 进行动态连接
  if let Ok(host_id) = session_id.parse::<usize>() {
    log::info!("[SFTP] Connecting on-demand for host_id={host_id} (session_id={session_id})");
    let chain = resolve_host_chain(host_id)?;
    let client = connect_sftp(chain).await?;
    let mut clients = state.clients.lock().await;
    clients.insert(session_id.to_string(), Arc::clone(&client));
    return Ok(client);
  }

  Err(format!(
    "SFTP session '{session_id}' is not connected and is not a valid host id"
  ))
}

pub async fn close_sftp(state: &SftpState, session_id: &str) -> Result<(), String> {
  let mut clients = state.clients.lock().await;
  if let Some(client) = clients.remove(session_id) {
    let _ = client.close().await;
    log::info!("[SFTP] Closed session {session_id}");
  }
  Ok(())
}
