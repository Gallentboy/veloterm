use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusqlite::params;
use russh::keys::{PublicKey, PublicKeyBase64};
use serde::{Deserialize, Serialize};

use crate::common::SafeUsizeCounter;
use crate::db::{DbPool, global_pool, now_timestamp_ms};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnownHostEntry {
  #[serde(default)]
  pub id: usize,
  pub host: String,
  pub port: u16,
  pub key_type: String,
  pub key_base64: String,
}

#[derive(Debug, Clone)]
pub enum KnownHostValidationResult {
  Valid,
  Invalid {
    key_type: String,
    key_base64: String,
  },
  Unknown,
}

/// 远程主机公钥校验与持久化存储（迁移自 `warpgate-protocol-ssh/src/known_hosts.rs`，
/// 底层直接存储于 SQLCipher 加密数据库 `veloterm.db` 的 `known_hosts` 表）。
#[derive(Clone, Default)]
pub struct KnownHosts {
  pool: Option<DbPool>,
  fallback_entries: Arc<Mutex<Vec<KnownHostEntry>>>,
  fallback_counter: Arc<SafeUsizeCounter>,
}

fn legacy_known_hosts_path() -> Option<PathBuf> {
  let home = env::var_os("HOME")
    .or_else(|| env::var_os("USERPROFILE"))
    .map(PathBuf::from)?;
  Some(home.join(".veloterm").join("known_hosts.json"))
}

impl KnownHosts {
  pub fn global() -> Self {
    let pool = global_pool();
    if let Some(ref db_pool) = pool {
      Self::migrate_legacy_file_if_present(db_pool);
    }
    Self {
      pool,
      fallback_entries: Arc::new(Mutex::new(Vec::new())),
      fallback_counter: Arc::new(SafeUsizeCounter::new(1)),
    }
  }

  pub fn with_pool(pool: DbPool) -> Self {
    Self {
      pool: Some(pool),
      fallback_entries: Arc::new(Mutex::new(Vec::new())),
      fallback_counter: Arc::new(SafeUsizeCounter::new(1)),
    }
  }

  fn active_pool(&self) -> Option<DbPool> {
    self.pool.clone().or_else(global_pool)
  }

  /// 若存在早期版本的 `~/.veloterm/known_hosts.json` 明文文件，自动迁移至 `veloterm.db` 并清理明文文件。
  fn migrate_legacy_file_if_present(pool: &DbPool) {
    let Some(path) = legacy_known_hosts_path() else {
      return;
    };
    if !path.is_file() {
      return;
    }
    if let Ok(raw) = fs::read_to_string(&path)
      && let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(&raw)
      && let Ok(conn) = pool.get()
    {
      let now = now_timestamp_ms();
      for entry in entries {
        let host = entry.get("host").and_then(|v| v.as_str()).unwrap_or_default();
        let port = entry.get("port").and_then(|v| v.as_i64()).unwrap_or(22);
        let key_type = entry.get("key_type").and_then(|v| v.as_str()).unwrap_or_default();
        let key_base64 = entry.get("key_base64").and_then(|v| v.as_str()).unwrap_or_default();
        if !host.is_empty() && !key_type.is_empty() && !key_base64.is_empty() {
          let _ = conn.execute(
            "INSERT INTO known_hosts (host, port, key_type, key_base64, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![host, port, key_type, key_base64, now],
          );
        }
      }
      let _ = fs::remove_file(&path);
    }
  }

  pub fn validate(
    &self,
    host: &str,
    port: u16,
    key: &PublicKey,
  ) -> Result<KnownHostValidationResult, String> {
    let algo = key.algorithm();
    let algo_str = algo.as_str();
    let key_base64 = key.public_key_base64();

    let entries = self.list()?;
    let matching: Vec<&KnownHostEntry> = entries
      .iter()
      .filter(|x| x.host == host && x.port == port && x.key_type == algo_str)
      .collect();

    if matching.iter().any(|x| x.key_base64 == key_base64) {
      return Ok(KnownHostValidationResult::Valid);
    }
    if let Some(first) = matching.first() {
      return Ok(KnownHostValidationResult::Invalid {
        key_type: first.key_type.clone(),
        key_base64: first.key_base64.clone(),
      });
    }
    Ok(KnownHostValidationResult::Unknown)
  }

  pub fn trust(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), String> {
    let key_type = key.algorithm().to_string();
    let key_base64 = key.public_key_base64();

    if let Some(pool) = self.active_pool() {
      let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection for known_hosts: {e}"))?;
      let exists: bool = conn
        .query_row(
          "SELECT EXISTS(
             SELECT 1 FROM known_hosts
             WHERE host = ?1 AND port = ?2 AND key_type = ?3 AND key_base64 = ?4
           )",
          params![host, i64::from(port), &key_type, &key_base64],
          |row| row.get(0),
        )
        .map_err(|e| format!("failed to check existing known_host: {e}"))?;

      if !exists {
        conn
          .execute(
            "INSERT INTO known_hosts (host, port, key_type, key_base64, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
              host,
              i64::from(port),
              key_type,
              key_base64,
              now_timestamp_ms(),
            ],
          )
          .map_err(|e| format!("failed to insert trusted known_host: {e}"))?;
      }
      return Ok(());
    }

    let mut guard = self
      .fallback_entries
      .lock()
      .map_err(|_| "known_hosts fallback mutex poisoned".to_string())?;
    if !guard.iter().any(|x| {
      x.host == host && x.port == port && x.key_type == key_type && x.key_base64 == key_base64
    }) {
      guard.push(KnownHostEntry {
        id: self.fallback_counter.next(),
        host: host.to_owned(),
        port,
        key_type,
        key_base64,
      });
    }
    Ok(())
  }

  pub fn list(&self) -> Result<Vec<KnownHostEntry>, String> {
    if let Some(pool) = self.active_pool() {
      let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection for known_hosts: {e}"))?;
      let mut stmt = conn
        .prepare(
          "SELECT id, host, port, key_type, key_base64
           FROM known_hosts
           ORDER BY created_at ASC, id ASC",
        )
        .map_err(|e| format!("failed to prepare known_hosts query: {e}"))?;
      let rows = stmt
        .query_map([], |row| {
          let port_i64: i64 = row.get(2)?;
          Ok(KnownHostEntry {
            id: row.get(0)?,
            host: row.get(1)?,
            port: u16::try_from(port_i64.clamp(1, 65535)).unwrap_or(22),
            key_type: row.get(3)?,
            key_base64: row.get(4)?,
          })
        })
        .map_err(|e| format!("failed to query known_hosts: {e}"))?;

      let mut result = Vec::new();
      for row in rows {
        result.push(row.map_err(|e| format!("failed to decode known_host row: {e}"))?);
      }
      return Ok(result);
    }

    let guard = self
      .fallback_entries
      .lock()
      .map_err(|_| "known_hosts fallback mutex poisoned".to_string())?;
    Ok(guard.clone())
  }

  pub fn remove(&self, id: usize) -> Result<bool, String> {
    if let Some(pool) = self.active_pool() {
      let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection for known_hosts: {e}"))?;
      let affected = conn
        .execute("DELETE FROM known_hosts WHERE id = ?1", params![id])
        .map_err(|e| format!("failed to delete known_host {id}: {e}"))?;
      return Ok(affected > 0);
    }

    let mut guard = self
      .fallback_entries
      .lock()
      .map_err(|_| "known_hosts fallback mutex poisoned".to_string())?;
    let before = guard.len();
    guard.retain(|entry| entry.id != id);
    Ok(guard.len() != before)
  }

  pub fn clear(&self) -> Result<usize, String> {
    if let Some(pool) = self.active_pool() {
      let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection for known_hosts: {e}"))?;
      let affected = conn
        .execute("DELETE FROM known_hosts", [])
        .map_err(|e| format!("failed to clear known_hosts: {e}"))?;
      return Ok(affected);
    }

    let mut guard = self
      .fallback_entries
      .lock()
      .map_err(|_| "known_hosts fallback mutex poisoned".to_string())?;
    let count = guard.len();
    guard.clear();
    Ok(count)
  }
}
