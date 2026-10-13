use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::ipc::Channel;

use crate::common::SafeUsizeCounter;
use super::local_pty::LocalPtySession;
use super::session::{
  TerminalBackendSession, TerminalCreateRequest, TerminalCreateResult, TerminalTarget,
};
use super::ssh::{SshTerminalSession, TargetSSHOptions};

/// 统一终端会话管理器（同时管理本地 `portable-pty` 会话与远程 `russh` 会话，使用 `usize` 整型会话 ID 消除高频 IPC 堆分配）。
pub struct TerminalManager {
  sessions: Arc<Mutex<HashMap<usize, Arc<dyn TerminalBackendSession>>>>,
  counter: SafeUsizeCounter,
}

impl Default for TerminalManager {
  fn default() -> Self {
    Self::new()
  }
}

impl TerminalManager {
  pub fn new() -> Self {
    Self {
      sessions: Arc::new(Mutex::new(HashMap::new())),
      counter: SafeUsizeCounter::new(1),
    }
  }

  pub fn create(
    &self,
    request: TerminalCreateRequest,
    on_output: Channel<Vec<u8>>,
    on_exit: Channel<i32>,
  ) -> Result<TerminalCreateResult, String> {
    let id = self.counter.next();
    let target = request.resolve_target();

    match target {
      TerminalTarget::Local { cwd, shell } => {
        let sessions_ref = Arc::clone(&self.sessions);

        let (session, result) = LocalPtySession::spawn(
          id,
          request.cols,
          request.rows,
          cwd,
          shell,
          on_output,
          on_exit,
          move |closed_id| {
            if let Ok(mut map) = sessions_ref.lock() {
              map.remove(&closed_id);
            }
          },
        )?;

        let mut map = self
          .sessions
          .lock()
          .map_err(|_| "terminal sessions mutex poisoned".to_string())?;
        map.insert(id, session);
        log::info!(
          "[TerminalManager] created local PTY session id={} shell={} cols={} rows={}",
          id,
          result.shell,
          request.cols,
          request.rows
        );
        Ok(result)
      }

      TerminalTarget::Ssh {
        host_id,
        host,
        port,
        username,
        auth,
        allow_insecure_algos,
        host_key_verification,
        jump_hosts,
      } => {
        let db_store = crate::db::global_pool()
          .and_then(|pool| crate::db::load_host_store(&pool).ok());
        let db_host = host_id.and_then(|hid| {
          db_store
            .as_ref()
            .and_then(|s| s.hosts.iter().find(|h| h.id == hid))
            .cloned()
        });

        let resolved_host = host
          .map(|h| h.trim().to_string())
          .filter(|h| !h.is_empty())
          .or_else(|| db_host.as_ref().map(|h| h.host.clone()))
          .ok_or_else(|| {
            format!(
              "SSH target host address is required (host_id={host_id:?})"
            )
          })?;
        let resolved_port = port
          .or_else(|| db_host.as_ref().map(|h| h.port))
          .unwrap_or(22);
        let resolved_username = username
          .map(|u| u.trim().to_string())
          .filter(|u| !u.is_empty())
          .or_else(|| db_host.as_ref().map(|h| h.username.clone()))
          .filter(|u| !u.is_empty())
          .unwrap_or_else(|| "root".to_string());
        let resolved_auth = auth
          .or_else(|| db_host.as_ref().map(|h| h.auth.clone()))
          .unwrap_or_default();
        let resolved_allow_insecure = allow_insecure_algos
          .or_else(|| db_host.as_ref().map(|h| h.allow_insecure_algos))
          .unwrap_or(false);
        let resolved_verification = host_key_verification
          .or_else(|| db_host.as_ref().map(|h| h.host_key_verification))
          .unwrap_or_default();

        let mut chain = if let Some(explicit_jumps) = jump_hosts {
          explicit_jumps
        } else if let (Some(store), Some(target_record)) = (&db_store, &db_host) {
          let by_id: HashMap<usize, &crate::db::HostRecord> =
            store.hosts.iter().map(|h| (h.id, h)).collect();
          let mut hops = Vec::new();
          let mut visited = std::collections::HashSet::from([target_record.id]);
          let mut cursor = target_record.jump_host_id;
          while let Some(jid) = cursor {
            if !visited.insert(jid) {
              break;
            }
            let Some(hop) = by_id.get(&jid) else {
              break;
            };
            hops.insert(
              0,
              TargetSSHOptions {
                host: hop.host.clone(),
                port: hop.port,
                username: hop.username.clone(),
                allow_insecure_algos: hop.allow_insecure_algos,
                auth: hop.auth.clone(),
                jump_host: None,
                host_key_verification: hop.host_key_verification,
              },
            );
            cursor = hop.jump_host_id;
          }
          hops
        } else {
          Vec::new()
        };

        chain.push(TargetSSHOptions {
          host: resolved_host.clone(),
          port: resolved_port,
          username: resolved_username.clone(),
          allow_insecure_algos: resolved_allow_insecure,
          auth: resolved_auth,
          jump_host: None,
          host_key_verification: resolved_verification,
        });

        let sessions_ref = Arc::clone(&self.sessions);

        let (session, result) = SshTerminalSession::spawn(
          id,
          request.cols,
          request.rows,
          chain,
          on_output,
          on_exit,
          move |closed_id| {
            if let Ok(mut map) = sessions_ref.lock() {
              map.remove(&closed_id);
            }
          },
        )?;

        let mut map = self
          .sessions
          .lock()
          .map_err(|_| "terminal sessions mutex poisoned".to_string())?;
        map.insert(id, session);
        log::info!(
          "[TerminalManager] created SSH session id={} target={}@{}:{} cols={} rows={}",
          id,
          resolved_username,
          resolved_host,
          resolved_port,
          request.cols,
          request.rows
        );
        Ok(result)
      }
    }
  }

  pub fn write(&self, id: usize, data: &[u8]) -> Result<(), String> {
    let session = {
      let map = self
        .sessions
        .lock()
        .map_err(|_| "terminal sessions mutex poisoned".to_string())?;
      map.get(&id).cloned()
    };

    if let Some(session) = session {
      session.write(data)
    } else {
      Ok(())
    }
  }

  pub fn resize(&self, id: usize, cols: u16, rows: u16) -> Result<(), String> {
    let session = {
      let map = self
        .sessions
        .lock()
        .map_err(|_| "terminal sessions mutex poisoned".to_string())?;
      map.get(&id).cloned()
    };

    if let Some(session) = session {
      log::debug!(
        "[TerminalManager] resize session id={} cols={} rows={}",
        id,
        cols,
        rows
      );
      session.resize(cols, rows)
    } else {
      Ok(())
    }
  }

  pub fn dispose(&self, id: usize) -> Result<(), String> {
    let session = {
      let mut map = self
        .sessions
        .lock()
        .map_err(|_| "terminal sessions mutex poisoned".to_string())?;
      map.remove(&id)
    };

    if let Some(session) = session {
      log::info!("[TerminalManager] dispose session id={}", id);
      session.close()
    } else {
      Ok(())
    }
  }
}
