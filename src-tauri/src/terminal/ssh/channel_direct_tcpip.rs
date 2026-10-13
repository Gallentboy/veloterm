use anyhow::Result;
use bytes::Bytes;
use russh::Channel;
use russh::client::Msg;
use tokio::sync::mpsc::{Sender, UnboundedReceiver};

use super::client::RCEvent;
use super::common::ChannelOperation;
use super::error::SshClientError;

/// SSH Direct TCP/IP 转发通道（迁移自 `warpgate-protocol-ssh/src/client/channel_direct_tcpip.rs`）。
pub struct DirectTCPIPChannel {
  client_channel: Channel<Msg>,
  channel_id: usize,
  ops_rx: UnboundedReceiver<ChannelOperation>,
  events_tx: Sender<RCEvent>,
  session_id: usize,
}

impl DirectTCPIPChannel {
  pub const fn new(
    client_channel: Channel<Msg>,
    channel_id: usize,
    ops_rx: UnboundedReceiver<ChannelOperation>,
    events_tx: Sender<RCEvent>,
    session_id: usize,
  ) -> Self {
    Self {
      client_channel,
      channel_id,
      ops_rx,
      events_tx,
      session_id,
    }
  }

  pub async fn run(mut self) -> Result<(), SshClientError> {
    loop {
      tokio::select! {
        incoming_data = self.ops_rx.recv() => {
          match incoming_data {
            Some(ChannelOperation::Data(data)) => {
              self.client_channel.data(&*data).await?;
            }
            Some(ChannelOperation::Eof) => {
              self.client_channel.eof().await?;
            }
            Some(ChannelOperation::Close) | None => break,
            Some(operation) => {
              log::warn!(
                "[SSH {}] unexpected direct-tcpip channel {} operation: {:?}",
                self.session_id,
                self.channel_id,
                operation
              );
            }
          }
        }
        channel_event = self.client_channel.wait() => {
          match channel_event {
            Some(russh::ChannelMsg::Data { data }) => {
              let bytes: &[u8] = &data;
              self.events_tx.send(RCEvent::Output(
                self.channel_id,
                Bytes::from(bytes.to_vec()),
              )).await.map_err(|_| SshClientError::MpscError)?;
            }
            Some(russh::ChannelMsg::Close) => {
              self.events_tx.send(RCEvent::Close(self.channel_id)).await.map_err(|_| SshClientError::MpscError)?;
            }
            Some(russh::ChannelMsg::Success) => {
              self.events_tx.send(RCEvent::Success(self.channel_id)).await.map_err(|_| SshClientError::MpscError)?;
            }
            Some(russh::ChannelMsg::Eof) => {
              self.events_tx.send(RCEvent::Eof(self.channel_id)).await.map_err(|_| SshClientError::MpscError)?;
            }
            None => {
              self.events_tx.send(RCEvent::Close(self.channel_id)).await.map_err(|_| SshClientError::MpscError)?;
              break;
            }
            Some(operation) => {
              log::warn!(
                "[SSH {}] unexpected direct-tcpip channel {} message: {:?}",
                self.session_id,
                self.channel_id,
                operation
              );
            }
          }
        }
      }
    }
    Ok(())
  }
}

impl Drop for DirectTCPIPChannel {
  fn drop(&mut self) {
    log::info!(
      "[SSH {}] DirectTCPIPChannel {} closed",
      self.session_id,
      self.channel_id
    );
  }
}
