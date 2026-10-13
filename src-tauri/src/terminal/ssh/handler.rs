use russh::Channel;
use russh::client::{ChannelOpenHandle, Msg, Session};
use russh::keys::{PublicKey, PublicKeyBase64, PublicKeyOrCertificate};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use super::common::{
  ForwardedStreamlocalParams, ForwardedTcpIpParams, SshHostKeyVerificationMode, TargetSSHOptions,
};
use super::error::ConnectionError;
use super::known_hosts::{KnownHostValidationResult, KnownHosts};

#[derive(Debug)]
pub enum ClientHandlerEvent {
  HostKeyReceived(PublicKey),
  HostKeyUnknown(PublicKey, oneshot::Sender<bool>),
  ForwardedTcpIp(Channel<Msg>, ForwardedTcpIpParams),
  ForwardedStreamlocal(Channel<Msg>, ForwardedStreamlocalParams),
  ForwardedAgent(Channel<Msg>),
  X11(Channel<Msg>, String, u32),
  Disconnect,
}

/// `russh::client::Handler` 实现（迁移自 `warpgate-protocol-ssh/src/client/handler.rs`）。
pub struct ClientHandler {
  pub ssh_options: TargetSSHOptions,
  pub event_tx: UnboundedSender<ClientHandlerEvent>,
  pub known_hosts: KnownHosts,
  pub session_id: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientHandlerError {
  #[error("Connection error: {0}")]
  ConnectionError(ConnectionError),

  #[error("SSH: {0}")]
  Ssh(#[from] russh::Error),

  #[error("Internal error")]
  Internal,
}

impl russh::client::Handler for ClientHandler {
  type Error = ClientHandlerError;

  async fn check_server_key(
    &mut self,
    server_public_key: &PublicKeyOrCertificate,
  ) -> Result<bool, Self::Error> {
    let server_public_key = server_public_key.public_key();
    self
      .event_tx
      .send(ClientHandlerEvent::HostKeyReceived(
        server_public_key.clone(),
      ))
      .map_err(|_| ClientHandlerError::ConnectionError(ConnectionError::Internal))?;

    let mode = self.ssh_options.host_key_verification;
    if mode == SshHostKeyVerificationMode::Ignore {
      log::debug!(
        "[SSH {}] Not checking the host key (verification is disabled)",
        self.session_id
      );
      return Ok(true);
    }

    match self.known_hosts.validate(
      &self.ssh_options.host,
      self.ssh_options.port,
      &server_public_key,
    ) {
      Ok(KnownHostValidationResult::Valid) => Ok(true),
      Ok(KnownHostValidationResult::Invalid {
        key_type,
        key_base64,
      }) => {
        log::warn!(
          "[SSH {}] Host key mismatch for {}:{}!",
          self.session_id,
          self.ssh_options.host,
          self.ssh_options.port
        );
        Err(ClientHandlerError::ConnectionError(
          ConnectionError::HostKeyMismatch {
            received_key_type: server_public_key.algorithm(),
            received_key_base64: server_public_key.public_key_base64(),
            known_key_type: key_type,
            known_key_base64: key_base64,
          },
        ))
      }
      Ok(KnownHostValidationResult::Unknown) => {
        log::info!(
          "[SSH {}] Host key for {}:{} is unknown ({})",
          self.session_id,
          self.ssh_options.host,
          self.ssh_options.port,
          server_public_key.algorithm()
        );

        let (tx, rx) = oneshot::channel();
        self
          .event_tx
          .send(ClientHandlerEvent::HostKeyUnknown(
            server_public_key.clone(),
            tx,
          ))
          .map_err(|_| ClientHandlerError::Internal)?;
        let accepted = rx.await.map_err(|_| ClientHandlerError::Internal)?;
        if accepted {
          if let Err(error) = self.known_hosts.trust(
            &self.ssh_options.host,
            self.ssh_options.port,
            &server_public_key,
          ) {
            log::error!(
              "[SSH {}] Failed to save trusted host key: {}",
              self.session_id,
              error
            );
          }
          Ok(true)
        } else {
          Ok(false)
        }
      }
      Err(error) => {
        log::error!(
          "[SSH {}] Failed to verify the host key: {}",
          self.session_id,
          error
        );
        Err(ClientHandlerError::Internal)
      }
    }
  }

  async fn server_channel_open_forwarded_tcpip(
    &mut self,
    channel: Channel<Msg>,
    connected_address: &str,
    connected_port: u32,
    originator_address: &str,
    originator_port: u32,
    reply: ChannelOpenHandle,
    _session: &mut Session,
  ) -> Result<(), Self::Error> {
    let connected_address = connected_address.to_string();
    let originator_address = originator_address.to_string();
    reply.accept().await;
    let _ = self.event_tx.send(ClientHandlerEvent::ForwardedTcpIp(
      channel,
      ForwardedTcpIpParams {
        connected_address,
        connected_port,
        originator_address,
        originator_port,
      },
    ));
    Ok(())
  }

  async fn server_channel_open_x11(
    &mut self,
    channel: Channel<Msg>,
    originator_address: &str,
    originator_port: u32,
    reply: ChannelOpenHandle,
    _session: &mut Session,
  ) -> Result<(), Self::Error> {
    let originator_address = originator_address.to_string();
    reply.accept().await;
    let _ = self.event_tx.send(ClientHandlerEvent::X11(
      channel,
      originator_address,
      originator_port,
    ));
    Ok(())
  }

  async fn server_channel_open_forwarded_streamlocal(
    &mut self,
    channel: Channel<Msg>,
    socket_path: &str,
    reply: ChannelOpenHandle,
    _session: &mut Session,
  ) -> Result<(), Self::Error> {
    let socket_path = socket_path.to_string();
    reply.accept().await;
    let _ = self
      .event_tx
      .send(ClientHandlerEvent::ForwardedStreamlocal(
        channel,
        ForwardedStreamlocalParams { socket_path },
      ));
    Ok(())
  }

  async fn server_channel_open_agent_forward(
    &mut self,
    channel: Channel<Msg>,
    reply: ChannelOpenHandle,
    _session: &mut Session,
  ) -> Result<(), Self::Error> {
    reply.accept().await;
    let _ = self
      .event_tx
      .send(ClientHandlerEvent::ForwardedAgent(channel));
    Ok(())
  }
}

impl Drop for ClientHandler {
  fn drop(&mut self) {
    let _ = self.event_tx.send(ClientHandlerEvent::Disconnect);
    log::debug!("[SSH {}] ClientHandler dropped", self.session_id);
  }
}
