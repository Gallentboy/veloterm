use std::error::Error;

#[derive(thiserror::Error, Debug)]
pub enum SshClientError {
  #[error("mpsc error")]
  MpscError,
  #[error("russh error: {0}")]
  Russh(#[from] russh::Error),
  #[error(transparent)]
  Other(Box<dyn Error + Send + Sync>),
}

impl SshClientError {
  #[allow(dead_code)]
  pub fn other<E: Error + Send + Sync + 'static>(err: E) -> Self {
    Self::Other(Box::new(err))
  }
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
  #[error("Host key mismatch (expected {known_key_type} {known_key_base64}, received {received_key_type} {received_key_base64})")]
  HostKeyMismatch {
    received_key_type: russh::keys::Algorithm,
    received_key_base64: String,
    known_key_type: String,
    known_key_base64: String,
  },

  #[error(transparent)]
  Io(#[from] std::io::Error),

  #[error(transparent)]
  Key(#[from] russh::keys::Error),

  #[error(transparent)]
  Ssh(#[from] russh::Error),

  #[error("Could not resolve address")]
  Resolve,

  #[error("Internal error")]
  Internal,

  #[error("Connection aborted")]
  Aborted,

  #[error("Authentication failed: {0}")]
  Authentication(String),

  #[error("Jump host target not found")]
  #[allow(dead_code)]
  JumpHostTargetNotFound,
}
