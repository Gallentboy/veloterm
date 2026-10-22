//! 秘密与 HMAC 证明。秘密永不上线，线上只出现一次性的 HMAC-SHA256 证明。

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::error::ProtoError;
use crate::frame::TargetAddr;

pub const SECRET_LEN: usize = 32;
pub const NONCE_LEN: usize = 16;
pub const PROOF_LEN: usize = 32;

/// HMAC 域分隔前缀：同一 secret 可复用到多个通道，域字符串防跨通道重放证明。
pub const DOM_STREAM: &[u8] = b"veloterm-relay-v2/stream";
pub const DOM_UDP_DATA: &[u8] = b"veloterm-relay-v2/udp-data";
pub const DOM_UDP_SYNC: &[u8] = b"veloterm-relay-v2/udp-timesync";

type HmacSha256 = Hmac<Sha256>;

/// 长期凭证。32 字节真随机，两端配置持有，从不经过网络。
#[derive(Clone)]
pub struct Secret([u8; SECRET_LEN]);

// 手写 Debug：日志里绝不输出秘密本体
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl Secret {
    pub fn from_bytes(bytes: [u8; SECRET_LEN]) -> Self {
        Self(bytes)
    }

    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8; SECRET_LEN] {
        &self.0
    }

    /// 生成 32 字节真随机秘密（服务端 CLI `token generate` 与测试用）。
    pub fn generate() -> Self {
        let mut buf = [0u8; SECRET_LEN];
        getrandom::fill(&mut buf).expect("系统熵源不可用");
        Self(buf)
    }

    pub fn from_hex(s: &str) -> Result<Self, ProtoError> {
        let bytes = hex_decode(s.as_bytes()).ok_or(ProtoError::BadValue)?;
        let arr: [u8; SECRET_LEN] = bytes.try_into().map_err(|_| ProtoError::BadLength)?;
        Ok(Self(arr))
    }

    pub fn to_hex(&self) -> String {
        hex_encode(&self.0)
    }
}

/// 常数时间比较，防时序侧信道逐字节猜测证明值。
#[inline]
pub fn const_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// 16 字节随机 nonce。
pub fn random_nonce() -> [u8; NONCE_LEN] {
    let mut buf = [0u8; NONCE_LEN];
    getrandom::fill(&mut buf).expect("系统熵源不可用");
    buf
}

fn hmac_parts(key: &[u8], parts: &[&[u8]]) -> [u8; PROOF_LEN] {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC-SHA256 接受任意长度密钥");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

/// target 的 HMAC 输入编码：host 字节 + 端口大端两字节。
/// 端口定长收尾，host/port 边界无歧义。
fn target_hmac_bytes(target: Option<&TargetAddr>) -> Vec<u8> {
    match target {
        Some(t) => {
            let mut v = Vec::with_capacity(t.host.len() + 2);
            v.extend_from_slice(t.host.as_bytes());
            v.extend_from_slice(&t.port.to_be_bytes());
            v
        }
        None => Vec::new(),
    }
}

/// 流式通道证明：绑定 key_id + 双 nonce + 客户端声明的 target（防在途篡改选路）。
pub fn stream_proof(
    secret: &Secret,
    key_id: u8,
    client_nonce: &[u8; NONCE_LEN],
    server_nonce: &[u8; NONCE_LEN],
    target: Option<&TargetAddr>,
) -> [u8; PROOF_LEN] {
    let key_id_bytes = [key_id];
    let target_bytes = target_hmac_bytes(target);
    hmac_parts(
        secret.as_bytes(),
        &[
            DOM_STREAM,
            &key_id_bytes,
            client_nonce,
            server_nonce,
            &target_bytes,
        ],
    )
}

pub fn verify_stream_proof(
    secret: &Secret,
    key_id: u8,
    client_nonce: &[u8; NONCE_LEN],
    server_nonce: &[u8; NONCE_LEN],
    target: Option<&TargetAddr>,
    proof: &[u8; PROOF_LEN],
) -> bool {
    const_eq(&stream_proof(secret, key_id, client_nonce, server_nonce, target), proof)
}

/// UDP 数据报证明：绑定 key_id + 服务器相对时间戳（客户端已用时间同步抵消钟差）。
#[inline]
pub fn udp_data_proof(secret: &Secret, key_id: u8, time_ms: u64) -> [u8; PROOF_LEN] {
    let key_id_bytes = [key_id];
    hmac_parts(
        secret.as_bytes(),
        &[DOM_UDP_DATA, &key_id_bytes, &time_ms.to_be_bytes()],
    )
}

/// UDP 时间同步应答证明：绑定客户端 nonce + 服务器时间（防伪造时间欺骗客户端钟差）。
pub fn udp_sync_proof(secret: &Secret, key_id: u8, client_nonce: &[u8; NONCE_LEN], server_time_ms: u64) -> [u8; PROOF_LEN] {
    let key_id_bytes = [key_id];
    hmac_parts(
        secret.as_bytes(),
        &[
            DOM_UDP_SYNC,
            &key_id_bytes,
            client_nonce,
            &server_time_ms.to_be_bytes(),
        ],
    )
}

const HEX_LOWER: &[u8; 16] = b"0123456789abcdef";

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX_LOWER[(b >> 4) as usize] as char);
        out.push(HEX_LOWER[(b & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(bytes: &[u8]) -> Option<Vec<u8>> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.as_chunks::<2>().0 {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(byte: u8) -> Secret {
        Secret::from_bytes([byte; SECRET_LEN])
    }

    fn target() -> TargetAddr {
        TargetAddr { host: "linux1.internal".into(), port: 22 }
    }

    #[test]
    fn secret_hex_roundtrip() {
        let s = Secret::generate();
        assert_eq!(Secret::from_hex(&s.to_hex()).unwrap().to_hex(), s.to_hex());
        assert!(Secret::from_hex("zz").is_err());
        assert!(Secret::from_hex(&"ab".repeat(31)).is_err());
    }

    #[test]
    fn secret_debug_never_leaks() {
        let s = secret(0xAB);
        assert!(!format!("{s:?}").contains("ab"));
    }

    #[test]
    fn stream_proof_binds_every_input() {
        let s = secret(7);
        let cn = [1u8; NONCE_LEN];
        let sn = [2u8; NONCE_LEN];
        let t = target();
        let good = stream_proof(&s, 7, &cn, &sn, Some(&t));
        assert!(verify_stream_proof(&s, 7, &cn, &sn, Some(&t), &good));

        // 改任何一个输入都应失败：换 secret / key_id / nonce / target / 证明字节
        assert!(!verify_stream_proof(&secret(8), 7, &cn, &sn, Some(&t), &good));
        assert!(!verify_stream_proof(&s, 8, &cn, &sn, Some(&t), &good));
        assert!(!verify_stream_proof(&s, 7, &cn, &[3u8; NONCE_LEN], Some(&t), &good));
        let mut bad = good;
        bad[0] ^= 1;
        assert!(!verify_stream_proof(&s, 7, &cn, &sn, Some(&t), &bad));
        let other_target = TargetAddr { host: "linux1.internal".into(), port: 2222 };
        assert!(!verify_stream_proof(&s, 7, &cn, &sn, Some(&other_target), &good));
        // target 是否携带本身就是证明的一部分（有/无 target 证明不同）
        assert_ne!(stream_proof(&s, 7, &cn, &sn, None), good);
    }

    #[test]
    fn const_eq_sanity() {
        assert!(const_eq(b"abc", b"abc"));
        assert!(!const_eq(b"abc", b"abd"));
        assert!(!const_eq(b"abc", b"ab"));
    }
}
