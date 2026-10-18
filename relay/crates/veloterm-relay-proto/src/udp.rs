//! UDP 模式：时间同步 + 数据报编解码。
//!
//! UDP 无连接做不了 nonce 问答，采用「先协商钟差 + 时间窗证明」：
//! 1. 客户端首次使用（或收到 TimeSkew 错误）时发 [`encode_sync_request`]，
//!    服务端回 [`encode_sync_response`]（带 HMAC，防伪造时间欺骗钟差）；
//! 2. 客户端据 [`ClockOffset`] 把本地时间换算成服务器相对时间再发数据报；
//! 3. 服务端按自己的时钟 ± [`DEFAULT_WINDOW_MS`] 验窗（容忍残余钟差与 RTT）。
//!
//! 数据报证明在窗口内仍可被链路截获者重放 ≤ 一个窗口——UDP 模式定位是
//! 固定目标的低敏转发（DNS 等），SSH 不走它，属已接受的权衡。

use crate::auth::{self, NONCE_LEN, PROOF_LEN, Secret};
use crate::error::ProtoError;

pub const DATA_MAGIC: [u8; 4] = *b"VLRD";
pub const SYNC_MAGIC: [u8; 4] = *b"VLTS";
pub const ERR_MAGIC: [u8; 4] = *b"VLTE";
/// 时间窗：30 秒。
pub const DEFAULT_WINDOW_MS: u64 = 30_000;

// ---------- 数据报 ----------

/// 编码数据报：`magic | key_id | time_ms(8 BE) | proof(32) | payload`。
pub fn encode_data(secret: &Secret, key_id: u8, server_time_ms: u64, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + 1 + 8 + PROOF_LEN + payload.len());
    buf.extend_from_slice(&DATA_MAGIC);
    buf.push(key_id);
    buf.extend_from_slice(&server_time_ms.to_be_bytes());
    buf.extend_from_slice(&auth::udp_data_proof(secret, key_id, server_time_ms));
    buf.extend_from_slice(payload);
    buf
}

/// 解码后的数据报（不含 magic）：证明输入与载荷切片。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDatagram<'a> {
    pub key_id: u8,
    pub time_ms: u64,
    pub proof: [u8; PROOF_LEN],
    pub payload: &'a [u8],
}

/// 解码 + 验证。任一步失败返回 `None`（UDP 无法回执，调用方只能丢弃）。
pub fn decode_and_verify_data(buf: &[u8], secret_for: impl Fn(u8) -> Option<Secret>, now_ms: u64) -> Option<DataDatagram<'_>> {
    let header_len = 4 + 1 + 8 + PROOF_LEN;
    if buf.len() < header_len + 1 || buf[..4] != DATA_MAGIC {
        return None;
    }
    let key_id = buf[4];
    let time_ms = u64::from_be_bytes(buf[5..13].try_into().unwrap());
    let proof: [u8; PROOF_LEN] = buf[13..45].try_into().unwrap();
    let secret = secret_for(key_id)?;
    if !auth::const_eq(&auth::udp_data_proof(&secret, key_id, time_ms), &proof) {
        return None;
    }
    if !time_within_window(time_ms, now_ms) {
        return None;
    }
    Some(DataDatagram { key_id, time_ms, proof, payload: &buf[header_len..] })
}

/// 验窗：`|time_ms - now_ms| <= window`。
pub fn time_within_window(time_ms: u64, now_ms: u64) -> bool {
    time_ms.abs_diff(now_ms) <= DEFAULT_WINDOW_MS
}

// ---------- 时间同步 ----------

/// 同步请求：`magic | key_id | client_nonce(16)`。
pub fn encode_sync_request(key_id: u8, client_nonce: &[u8; NONCE_LEN]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + 1 + NONCE_LEN);
    buf.extend_from_slice(&SYNC_MAGIC);
    buf.push(key_id);
    buf.extend_from_slice(client_nonce);
    buf
}

pub fn decode_sync_request(buf: &[u8]) -> Option<(u8, [u8; NONCE_LEN])> {
    if buf.len() != 4 + 1 + NONCE_LEN || buf[..4] != SYNC_MAGIC {
        return None;
    }
    Some((buf[4], buf[5..].try_into().unwrap()))
}

/// 同步应答：`magic | key_id | client_nonce(16) | server_time_ms(8 BE) | proof(32)`。
pub fn encode_sync_response(secret: &Secret, key_id: u8, client_nonce: &[u8; NONCE_LEN], server_time_ms: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + 1 + NONCE_LEN + 8 + PROOF_LEN);
    buf.extend_from_slice(&SYNC_MAGIC);
    buf.push(key_id);
    buf.extend_from_slice(client_nonce);
    buf.extend_from_slice(&server_time_ms.to_be_bytes());
    buf.extend_from_slice(&auth::udp_sync_proof(secret, key_id, client_nonce, server_time_ms));
    buf
}

/// 客户端校验同步应答：nonce 匹配 + HMAC 有效 → 返回服务器时间。
pub fn verify_sync_response(secret: &Secret, key_id: u8, client_nonce: &[u8; NONCE_LEN], buf: &[u8]) -> Result<u64, ProtoError> {
    let expected = 4 + 1 + NONCE_LEN + 8 + PROOF_LEN;
    if buf.len() != expected || buf[..4] != SYNC_MAGIC {
        return Err(ProtoError::BadLength);
    }
    if buf[4] != key_id || &buf[5..5 + NONCE_LEN] != client_nonce {
        return Err(ProtoError::BadValue);
    }
    let server_time_ms = u64::from_be_bytes(buf[5 + NONCE_LEN..5 + NONCE_LEN + 8].try_into().unwrap());
    let proof: [u8; PROOF_LEN] = buf[5 + NONCE_LEN + 8..].try_into().unwrap();
    if !auth::const_eq(&auth::udp_sync_proof(secret, key_id, client_nonce, server_time_ms), &proof) {
        return Err(ProtoError::BadValue);
    }
    Ok(server_time_ms)
}

/// 客户端持有的钟差（server_time ≈ local_time + offset）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockOffset {
    pub offset_ms: i64,
}

impl ClockOffset {
    /// 由一次同步采样计算：server_time - 收到应答时的本地时间。
    /// 单采样含半个 RTT 的误差，30 秒窗口下可忽略。
    pub fn from_sample(server_time_ms: u64, local_recv_ms: u64) -> Self {
        Self { offset_ms: server_time_ms as i64 - local_recv_ms as i64 }
    }

    pub fn to_server_time(&self, local_ms: u64) -> u64 {
        (local_ms as i64 + self.offset_ms).max(0) as u64
    }
}

// ---------- 服务端错误回执（无鉴权，仅提示客户端重新对时） ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UdpErrorCode {
    TimeSkew = 1,
    AuthFailed = 2,
}

impl UdpErrorCode {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => UdpErrorCode::TimeSkew,
            2 => UdpErrorCode::AuthFailed,
            _ => return None,
        })
    }
}

/// 错误回执：`magic | code | server_time_ms(8 BE)`。
pub fn encode_error(code: UdpErrorCode, server_time_ms: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + 1 + 8);
    buf.extend_from_slice(&ERR_MAGIC);
    buf.push(code as u8);
    buf.extend_from_slice(&server_time_ms.to_be_bytes());
    buf
}

pub fn decode_error(buf: &[u8]) -> Option<(UdpErrorCode, u64)> {
    if buf.len() != 4 + 1 + 8 || buf[..4] != ERR_MAGIC {
        return None;
    }
    let code = UdpErrorCode::from_u8(buf[4])?;
    let time = u64::from_be_bytes(buf[5..].try_into().unwrap());
    Some((code, time))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret() -> Secret {
        Secret::from_bytes([5u8; 32])
    }

    fn now() -> u64 {
        1_700_000_000_000
    }

    #[test]
    fn data_roundtrip_and_window() {
        let s = secret();
        let dg = encode_data(&s, 3, now(), b"query");
        let parsed = decode_and_verify_data(&dg, |k| (k == 3).then(|| s.clone()), now()).unwrap();
        assert_eq!(parsed.payload, b"query");

        // 时间戳出窗 → 拒
        assert!(decode_and_verify_data(&dg, |k| (k == 3).then(|| s.clone()), now() + DEFAULT_WINDOW_MS + 1).is_none());
        // 错误 key_id / 秘密 → 拒
        assert!(decode_and_verify_data(&dg, |k| (k == 4).then(|| s.clone()), now()).is_none());
        let wrong = Secret::from_bytes([6u8; 32]);
        assert!(decode_and_verify_data(&dg, |k| (k == 3).then(|| wrong.clone()), now()).is_none());
        // 载荷被篡改不破坏证明（证明不覆盖载荷——UDP 模式目标固定且低敏，
        // 载荷完整性属上层协议职责，此处行为锁定为「仍然放行」）
        let mut tampered = dg.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0xFF;
        assert!(decode_and_verify_data(&tampered, |k| (k == 3).then(|| s.clone()), now()).is_some());
    }

    #[test]
    fn time_sync_roundtrip() {
        let s = secret();
        let nonce = [9u8; NONCE_LEN];
        let req = encode_sync_request(3, &nonce);
        let (key_id, parsed_nonce) = decode_sync_request(&req).unwrap();
        assert_eq!((key_id, parsed_nonce), (3, nonce));

        let resp = encode_sync_response(&s, 3, &nonce, now());
        assert_eq!(verify_sync_response(&s, 3, &nonce, &resp).unwrap(), now());
        // nonce 不匹配 / 换秘密 / 换 key_id → 拒
        assert!(verify_sync_response(&s, 3, &[8u8; NONCE_LEN], &resp).is_err());
        let wrong = Secret::from_bytes([6u8; 32]);
        assert!(verify_sync_response(&wrong, 3, &nonce, &resp).is_err());
        assert!(verify_sync_response(&s, 4, &nonce, &resp).is_err());
    }

    #[test]
    fn clock_offset_math() {
        // 服务器比本地快 2.5 秒
        let off = ClockOffset::from_sample(now(), now() - 2_500);
        assert_eq!(off.offset_ms, 2_500);
        assert_eq!(off.to_server_time(now()), now() + 2_500);
        // 本地比服务器快的情况（负 offset）
        let off2 = ClockOffset::from_sample(now(), now() + 4_000);
        assert_eq!(off2.to_server_time(now()), now() - 4_000);
    }

    #[test]
    fn error_receipt_roundtrip() {
        let e = encode_error(UdpErrorCode::TimeSkew, now());
        assert_eq!(decode_error(&e).unwrap(), (UdpErrorCode::TimeSkew, now()));
        assert!(decode_error(&e[..e.len() - 1]).is_none());
    }
}
