//! VLR2 帧编解码。
//!
//! 流式通道（tcp/ws/quic 携带层）握手布局：
//!
//! ```text
//! C→R  ClientHello : "VLR2" | flags u8 | key_id u8 | client_nonce(16)
//!                        | [host_len u8 | host | port u16 BE]   （flags bit0 时携带）
//! R→C  Continue    : "VLR2" | 0xFF | server_nonce(16)   （21 字节）
//! C→R  Proof       : 32 字节 HMAC-SHA256（见 auth::stream_proof）
//! R→C  Status      : "VLR2" | code u8                     （5 字节，之后按 code 决定管道或关闭）
//! ```
//!
//! Continue 用 `0xFF` 哨兵与 Status 的 0..=4 区分，客户端读前 5 字节即可分流，
//! 不必猜帧长。

use crate::auth::NONCE_LEN;
use crate::error::ProtoError;

pub const MAGIC: [u8; 4] = *b"VLR2";
pub const FLAG_HAS_TARGET: u8 = 0x01;
/// Continue 帧的哨兵字节，避开 Status 的取值域。
pub const CONTINUE_MARKER: u8 = 0xFF;
pub const MAX_HOST_LEN: usize = 255;
/// 头部定长段（magic + flags + key_id + client_nonce）。
pub const HELLO_FIXED_LEN: usize = 4 + 1 + 1 + NONCE_LEN;
pub const MAX_HELLO_LEN: usize = HELLO_FIXED_LEN + 1 + MAX_HOST_LEN + 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetAddr {
    pub host: String,
    pub port: u16,
}

impl TargetAddr {
    /// 解析 `host:port`。端口取最后一个冒号之后的段，裸 IPv6 字面量同样成立。
    pub fn parse(s: &str) -> Result<Self, ProtoError> {
        let (host, port) = s
            .rsplit_once(':')
            .ok_or(ProtoError::BadValue)?;
        let port: u16 = port.parse().map_err(|_| ProtoError::BadValue)?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if host.is_empty() || host.len() > MAX_HOST_LEN {
            return Err(ProtoError::BadLength);
        }
        Ok(Self { host: host.to_string(), port })
    }
}

impl std::fmt::Display for TargetAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientHello {
    pub key_id: u8,
    pub client_nonce: [u8; NONCE_LEN],
    pub target: Option<TargetAddr>,
}

impl ClientHello {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(HELLO_FIXED_LEN + 4 + self.target.as_ref().map_or(0, |t| t.host.len()));
        buf.extend_from_slice(&MAGIC);
        let flags = if self.target.is_some() { FLAG_HAS_TARGET } else { 0 };
        buf.push(flags);
        buf.push(self.key_id);
        buf.extend_from_slice(&self.client_nonce);
        if let Some(t) = &self.target {
            buf.push(t.host.len() as u8);
            buf.extend_from_slice(t.host.as_bytes());
            buf.extend_from_slice(&t.port.to_be_bytes());
        }
        buf
    }

    pub fn decode(buf: &[u8]) -> Result<Self, ProtoError> {
        if buf.len() < HELLO_FIXED_LEN {
            return Err(ProtoError::BadLength);
        }
        if buf[..4] != MAGIC {
            return Err(ProtoError::BadMagic);
        }
        let flags = buf[4];
        if flags & !FLAG_HAS_TARGET != 0 {
            return Err(ProtoError::BadValue);
        }
        let key_id = buf[5];
        let client_nonce: [u8; NONCE_LEN] = buf[6..HELLO_FIXED_LEN]
            .try_into()
            .map_err(|_| ProtoError::BadLength)?;
        let target = if flags & FLAG_HAS_TARGET != 0 {
            Some(decode_target(&buf[HELLO_FIXED_LEN..])?)
        } else {
            if buf.len() != HELLO_FIXED_LEN {
                return Err(ProtoError::BadLength);
            }
            None
        };
        Ok(Self { key_id, client_nonce, target })
    }
}

fn decode_target(rest: &[u8]) -> Result<TargetAddr, ProtoError> {
    if rest.len() < 3 {
        return Err(ProtoError::BadLength);
    }
    let host_len = rest[0] as usize;
    if host_len == 0 || rest.len() != 1 + host_len + 2 {
        return Err(ProtoError::BadLength);
    }
    let host = std::str::from_utf8(&rest[1..1 + host_len]).map_err(|_| ProtoError::BadUtf8)?;
    let port = u16::from_be_bytes([rest[1 + host_len], rest[2 + host_len]]);
    Ok(TargetAddr { host: host.to_string(), port })
}

/// 服务端终局状态码。`Ok` 之后进入纯字节搬运。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Status {
    Ok = 0,
    Malformed = 1,
    AuthFailed = 2,
    TargetNotAllowed = 3,
    ConnectFailed = 4,
}

impl Status {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Status::Ok,
            1 => Status::Malformed,
            2 => Status::AuthFailed,
            3 => Status::TargetNotAllowed,
            4 => Status::ConnectFailed,
            _ => return None,
        })
    }

    /// 终局帧：5 字节 magic + code。
    pub fn encode_frame(self) -> [u8; 5] {
        let mut buf = [0u8; 5];
        buf[..4].copy_from_slice(&MAGIC);
        buf[4] = self as u8;
        buf
    }
}

/// 服务端第一帧的两种形态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerFirst {
    Continue { server_nonce: [u8; NONCE_LEN] },
    Status(Status),
}

impl ServerFirst {
    pub fn encode_continue(server_nonce: [u8; NONCE_LEN]) -> [u8; 21] {
        let mut buf = [0u8; 21];
        buf[..4].copy_from_slice(&MAGIC);
        buf[4] = CONTINUE_MARKER;
        buf[5..].copy_from_slice(&server_nonce);
        buf
    }

    /// 由「前 5 字节 +（若是 Continue）其余 16 字节」解码。
    pub fn decode(head5: &[u8; 5], rest16: &[u8]) -> Result<Self, ProtoError> {
        if head5[..4] != MAGIC {
            return Err(ProtoError::BadMagic);
        }
        if head5[4] == CONTINUE_MARKER {
            if rest16.len() != NONCE_LEN {
                return Err(ProtoError::BadLength);
            }
            Ok(ServerFirst::Continue { server_nonce: rest16.try_into().unwrap() })
        } else {
            let status = Status::from_u8(head5[4]).ok_or(ProtoError::BadValue)?;
            Ok(ServerFirst::Status(status))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_roundtrip_with_and_without_target() {
        let hello = ClientHello {
            key_id: 9,
            client_nonce: [0xA5; NONCE_LEN],
            target: Some(TargetAddr { host: "linux1.internal".into(), port: 22 }),
        };
        assert_eq!(ClientHello::decode(&hello.encode()).unwrap(), hello);

        let bare = ClientHello { key_id: 1, client_nonce: [0; NONCE_LEN], target: None };
        assert_eq!(bare.encode().len(), HELLO_FIXED_LEN);
        assert_eq!(ClientHello::decode(&bare.encode()).unwrap(), bare);
    }

    #[test]
    fn hello_rejects_bad_frames() {
        let hello = ClientHello { key_id: 1, client_nonce: [0; NONCE_LEN], target: None };
        let good = hello.encode();

        let mut bad = good.clone();
        bad[0] = b'X';
        assert_eq!(ClientHello::decode(&bad).unwrap_err(), ProtoError::BadMagic);

        assert_eq!(ClientHello::decode(&good[..good.len() - 1]).unwrap_err(), ProtoError::BadLength);

        let mut unknown_flag = good.clone();
        unknown_flag[4] = 0x08;
        assert_eq!(ClientHello::decode(&unknown_flag).unwrap_err(), ProtoError::BadValue);

        // 带 target 声明但尾段长度与 host_len 不符
        let mut truncated = ClientHello {
            key_id: 1,
            client_nonce: [0; NONCE_LEN],
            target: Some(TargetAddr { host: "ab".into(), port: 22 }),
        }
        .encode();
        truncated.pop();
        assert_eq!(ClientHello::decode(&truncated).unwrap_err(), ProtoError::BadLength);
    }

    #[test]
    fn server_first_frame_roundtrip() {
        let nonce = [7u8; NONCE_LEN];
        let frame = ServerFirst::encode_continue(nonce);
        let head5: &[u8; 5] = frame[..5].try_into().unwrap();
        assert_eq!(ServerFirst::decode(head5, &frame[5..]).unwrap(), ServerFirst::Continue { server_nonce: nonce });

        let status = Status::TargetNotAllowed.encode_frame();
        assert_eq!(ServerFirst::decode(&status, &[]).unwrap(), ServerFirst::Status(Status::TargetNotAllowed));
    }

    #[test]
    fn target_addr_parse() {
        let t = TargetAddr::parse("linux1.internal:22").unwrap();
        assert_eq!((t.host.as_str(), t.port), ("linux1.internal", 22));
        // IPv6 字面量（方括号可选）
        assert_eq!(TargetAddr::parse("[::1]:2222").unwrap().port, 2222);
        assert_eq!(TargetAddr::parse("::1:2222").unwrap().host, "::1");
        assert!(TargetAddr::parse("noport").is_err());
        assert!(TargetAddr::parse("host:notaport").is_err());
    }
}
