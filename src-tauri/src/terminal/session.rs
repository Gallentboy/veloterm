use serde::{Deserialize, Serialize};

use super::ssh::{SSHTargetAuth, SshHostKeyVerificationMode, TargetSSHOptions};

/// 终端会话底层驱动类型：本地 PTY (`portable-pty`) 或远程 SSH (`russh`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalBackendKind {
  Local,
  Ssh,
}

/// 终端连接目标描述：
/// - `Local`：启动本机默认登录 Shell（`portable-pty`）；
/// - `Ssh`：连接远程 SSH 主机（支持单跳或多跳 Jump Host 链式隧道、密码/私钥/Agent 认证）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TerminalTarget {
  Local {
    cwd: Option<String>,
    shell: Option<String>,
  },
  Ssh {
    #[serde(rename = "hostId")]
    host_id: Option<usize>,
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    #[serde(default)]
    auth: Option<SSHTargetAuth>,
    #[serde(default, rename = "allowInsecureAlgos")]
    allow_insecure_algos: Option<bool>,
    #[serde(default, rename = "hostKeyVerification")]
    host_key_verification: Option<SshHostKeyVerificationMode>,
    #[serde(default, rename = "jumpHosts")]
    jump_hosts: Option<Vec<TargetSSHOptions>>,
  },
}

/// 前端调用 `terminal_create` 时的请求参数（兼容扁平字段与显式 `target` 结构）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCreateRequest {
  pub session_id: usize,
  pub cols: u16,
  pub rows: u16,
  pub cwd: Option<String>,
  pub host_id: Option<usize>,
  pub target: Option<TerminalTarget>,
}

impl TerminalCreateRequest {
  pub fn resolve_target(&self) -> TerminalTarget {
    if let Some(target) = &self.target {
      return target.clone();
    }
    if let Some(host_id) = self.host_id {
      return TerminalTarget::Ssh {
        host_id: Some(host_id),
        host: None,
        port: None,
        username: None,
        auth: None,
        allow_insecure_algos: None,
        host_key_verification: None,
        jump_hosts: None,
      };
    }
    TerminalTarget::Local {
      cwd: self.cwd.clone(),
      shell: None,
    }
  }
}

/// `terminal_create` 返回给前端的会话元信息。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCreateResult {
  pub id: usize,
  pub kind: TerminalBackendKind,
  pub shell: String,
  pub cwd: Option<String>,
}

/// 通用终端后端会话 Trait（同时适配 `portable-pty` 与 `russh`）：
///
/// - `LocalPtySession`：封装同步 `MasterPty`、`Write` 与 `Child` 句柄；
/// - `SshTerminalSession`：封装异步 `russh::Channel` 发送端（`tokio::sync::mpsc::UnboundedSender`），
///   对外暴露完全相同的 `write` / `resize` / `close` 接口，使上层 `TerminalManager` 与 Tauri 命令完全解耦。
pub trait TerminalBackendSession: Send + Sync {
  #[allow(dead_code)]
  fn id(&self) -> usize;
  #[allow(dead_code)]
  fn kind(&self) -> TerminalBackendKind;
  fn write(&self, data: &[u8]) -> Result<(), String>;
  fn resize(&self, cols: u16, rows: u16) -> Result<(), String>;
  fn close(&self) -> Result<(), String>;
}
