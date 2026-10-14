use std::env;
use std::path::{Path, PathBuf};

use russh::keys::{PrivateKey, decode_secret_key, load_secret_key};

fn expand_tilde_path(raw: &str) -> PathBuf {
  let trimmed = raw.trim();
  if let Some(stripped) = trimmed.strip_prefix("~/")
    && let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"))
  {
    return PathBuf::from(home).join(stripped);
  }
  PathBuf::from(trimmed)
}

/// 解析客户端私钥（参考 `warpgate-protocol-ssh/src/keys.rs` 的 `decode_secret_key` 与 `load_secret_key`）：
/// - 若输入包含 `-----BEGIN` 内联 PEM / OpenSSH 私钥文本，直接调用 `decode_secret_key`；
/// - 否则将其视为本地文件路径（支持 `~/...` 展开）并调用 `load_secret_key`。
pub fn load_client_private_key(
  private_key_or_path: &str,
  passphrase: Option<&str>,
) -> Result<PrivateKey, russh::keys::Error> {
  let trimmed = private_key_or_path.trim();
  let pass = passphrase.map(str::trim).filter(|s| !s.is_empty());

  if trimmed.contains("-----BEGIN") {
    return decode_secret_key(trimmed, pass);
  }

  let candidate_path = expand_tilde_path(trimmed);
  if candidate_path.is_file() {
    return load_secret_key(candidate_path, pass);
  }

  decode_secret_key(trimmed, pass)
}

/// 扫描本机 `~/.ssh` 下的默认私钥文件（`id_ed25519` / `id_ecdsa` / `id_rsa`），
/// 供 Agent 模式回退或未指定内联私钥时使用。
pub fn discover_default_local_keys() -> Vec<PrivateKey> {
  let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")) else {
    return Vec::new();
  };
  let ssh_dir = Path::new(&home).join(".ssh");
  let mut keys = Vec::new();
  for name in ["id_ed25519", "id_ecdsa", "id_rsa"] {
    let path = ssh_dir.join(name);
    if path.is_file() && let Ok(key) = load_secret_key(&path, None) {
      keys.push(key);
    }
  }
  keys
}

/// `<algo> <base64>` 规范化公钥字符串（迁移自 `warpgate-protocol-ssh/src/keys.rs::public_key_line`）。
pub fn public_key_line(key: &PrivateKey) -> Result<String, russh::keys::Error> {
  Ok(
    key
      .public_key()
      .to_openssh()?
      .split_whitespace()
      .take(2)
      .collect::<Vec<_>>()
      .join(" "),
  )
}
