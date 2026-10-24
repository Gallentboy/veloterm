//! OB20 流编解码器：握手完成后替换 `PacketCodec`，把 mysql_async 的读写
//! 管道整体切到 OceanBase 2.0 协议形态（对照 `vendor/obconnector-c` 的
//! `ma_net.c` ob20 收发路径）。
//!
//! 写：上层的 MySQL 包体（无 4B 头，与 `PacketCodec` 同契约）按标准规则
//! 分块（块上限扣掉 ob20 头/尾开销，对齐 C 的 `MAX_PACKET_LENGTH_WITH_OB20`）；
//! 每块 = `[len:3][seq:1][payload]` 的 MySQL 包 → ob20 包（`ob20::wrap_packet`）
//! → zlib 压缩 → 外层压缩协议帧 `[compressed_len:3][seq:1][uncompressed_len:3]`
//! （`uncompressed_len == 0` 表示不压缩，小包直接透传——libobclient 同此）。
//!
//! 读：解外层帧 →（按需）解压 → `ob20::unwrap_packet`（头 CRC16 的
//! complen 口径恒 0，与 C 读写两侧一致）→ 剥 MySQL 4B 头 → 多块重组，
//! `< 0xFFFFFF` 的块即逻辑包结尾。
//!
//! 序号语义：MySQL 内层包序号（`inner_seq_id`）与外层帧序号
//! （`outer_seq_id`，即 C 的 `compress_pkt_nr`，同时写入 ob20 头的 pkt_seq）
//! 各自独立递增；`reset_seq_id`（每条命令的边界）同时归零两者并推进
//! request_id（对齐 C 的 `update_request_id` 在读写入口的调用时机）。

use bytes::{Buf, BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use std::io::{self, Read};

use crate::{buffer_pool::PooledBuf, error::IoError, ob20};

/// ob20 形态下单块 payload 上限（C 的 MAX_PACKET_LENGTH_WITH_OB20）：
/// 0xFFFFFF 扣掉 ob20 头(24)/尾(4)/extra 长度域(4) 与 MySQL 4B 头的余量
const OB20_MAX_CHUNK: usize = 0xFF_FF_FF
    - (ob20::OB20_HEADER_SIZE + ob20::OB20_TAILER_SIZE + ob20::OB20_EXTRAINFO_LENGTH_SIZE);

pub struct Ob20Codec {
    ctx: ob20::Ob20Ctx,
    compression: flate2::Compression,
    max_allowed_packet: usize,
    inner_seq_id: u8,
    outer_seq_id: u8,
    /// 跨帧的多块重组缓冲（完成时与池换新）
    decode_buf: PooledBuf,
}

/// 外层压缩开关：Java JDBC 的 OB20 实现恒不压缩（uncompressLength 恒 0，
/// checkHeader 里收到非零直接报错），libobclient C 版则做压缩——服务端两种
/// 都应支持，但对齐 Java 参考行为可排除真机验证时的一个变量。
/// 压缩代码保留，后续真机验证后可开开关尝试。
const OUTER_COMPRESS: bool = false;

impl Ob20Codec {
    pub fn new(connection_id: u32, max_allowed_packet: usize) -> Self {
        Self {
            ctx: ob20::Ob20Ctx::new(connection_id),
            compression: flate2::Compression::new(3),
            max_allowed_packet,
            inner_seq_id: 0,
            outer_seq_id: 0,
            decode_buf: crate::buffer_pool().get(),
        }
    }

    /// 命令边界：内/外序号归零 + request_id 前进
    pub fn reset_seq_id(&mut self) {
        self.inner_seq_id = 0;
        self.outer_seq_id = 0;
        self.ctx.next_request();
    }

    /// 对齐 `PacketCodec::sync_seq_id` 的接口（ob20 下内层序号本就自持，
    /// 无需从外层覆盖）
    pub fn sync_seq_id(&mut self) {}

    pub fn set_max_allowed_packet(&mut self, max: usize) {
        self.max_allowed_packet = max;
    }

    /// zlib 压缩级别（仅影响发送侧）
    pub fn set_compression_level(&mut self, level: u32) {
        self.compression = flate2::Compression::new(level);
    }

    fn zlib_compress(&self, data: &[u8]) -> io::Result<Vec<u8>> {
        let mut encoder =
            flate2::read::ZlibEncoder::new(data, self.compression);
        let mut out = Vec::with_capacity(data.len() / 2 + 16);
        encoder.read_to_end(&mut out)?;
        Ok(out)
    }

    fn zlib_decompress(data: &[u8], expected: usize) -> io::Result<Vec<u8>> {
        let mut decoder = flate2::read::ZlibDecoder::new(data);
        let mut out = Vec::with_capacity(expected);
        decoder.read_to_end(&mut out)?;
        Ok(out)
    }
}

impl Encoder<PooledBuf> for Ob20Codec {
    type Error = IoError;

    fn encode(&mut self, item: PooledBuf, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let payload = item.as_ref();
        if payload.len() > self.max_allowed_packet {
            return Err(IoError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "packet too large",
            )));
        }

        // 命令边界：每帧逻辑包推进一次 request_id（encode 即一条完整命令的发出）
        self.ctx.next_request();

        // 标准分块：块大小 ≤ OB20_MAX_CHUNK；整除时补一个空块收尾
        let mut offset = 0;
        loop {
            let chunk_len = (payload.len() - offset).min(OB20_MAX_CHUNK);
            let chunk = &payload[offset..offset + chunk_len];
            offset += chunk_len;

            // MySQL 包（含 4B 头）
            let mut mysql_packet = Vec::with_capacity(4 + chunk.len());
            mysql_packet.extend_from_slice(&(chunk.len() as u32).to_le_bytes()[0..3]);
            mysql_packet.push(self.inner_seq_id);
            self.inner_seq_id = self.inner_seq_id.wrapping_add(1);
            mysql_packet.extend_from_slice(chunk);

            // ob20 包（头 CRC16 的 complen 口径恒 0）；头内 pkt_seq 必须
            // 与外层帧序号一致（libobclient 用同一计数器 compress_pkt_nr）。
            // 最后一块设 IS_LAST_PACKET 标志（Java JDBC 的 commandEnd 语义：
            // 服务端据此知道当前 request 的回复可以结束）
            self.ctx.pkt_seq = self.outer_seq_id;
            let is_last_chunk = chunk_len < OB20_MAX_CHUNK;
            let mut ob20_packet = ob20::wrap_packet(&self.ctx, &mysql_packet, 0);
            if is_last_chunk {
                ob20::set_last_packet_flag(&mut ob20_packet);
            }

            // 外层压缩协议帧：压缩不划算（≥原始长度）则原样发、uncompressed 置 0。
            // 默认不压缩（OUTER_COMPRESS=false，对齐 Java JDBC 行为——它的
            // uncompressLength 恒 0，从不做外层 zlib）
            let seq = self.outer_seq_id;
            self.outer_seq_id = self.outer_seq_id.wrapping_add(1);
            if OUTER_COMPRESS {
                let compressed = self.zlib_compress(&ob20_packet)?;
                if compressed.len() < ob20_packet.len() {
                    dst.extend_from_slice(&(compressed.len() as u32).to_le_bytes()[0..3]);
                    dst.put_u8(seq);
                    dst.extend_from_slice(&(ob20_packet.len() as u32).to_le_bytes()[0..3]);
                    dst.extend_from_slice(&compressed);
                } else {
                    dst.extend_from_slice(&(ob20_packet.len() as u32).to_le_bytes()[0..3]);
                    dst.put_u8(seq);
                    dst.extend_from_slice(&[0, 0, 0]);
                    dst.extend_from_slice(&ob20_packet);
                }
            } else {
                dst.extend_from_slice(&(ob20_packet.len() as u32).to_le_bytes()[0..3]);
                dst.put_u8(seq);
                dst.extend_from_slice(&[0, 0, 0]);
                dst.extend_from_slice(&ob20_packet);
            }

            // 标准终止条件：本块 < 上限（含补尾空块的情形）
            if is_last_chunk {
                break;
            }
        }
        Ok(())
    }
}

impl Decoder for Ob20Codec {
    type Item = PooledBuf;
    type Error = IoError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        loop {
            // 外层帧头：[len:3][seq:1][uncompressed_len:3]
            if src.len() < 7 {
                src.reserve(7 - src.len());
                return Ok(None);
            }
            let frame_len = (u32::from(src[0]) | (u32::from(src[1]) << 8) | (u32::from(src[2]) << 16))
                as usize;
            let frame_seq = src[3];
            let uncompressed_len =
                u32::from(src[4]) | (u32::from(src[5]) << 8) | (u32::from(src[6]) << 16);
            if frame_len > self.max_allowed_packet.max(OB20_MAX_CHUNK) {
                return Err(IoError::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("ob20 外层帧超长：{frame_len}"),
                )));
            }
            if src.len() < 7 + frame_len {
                src.reserve(7 + frame_len - src.len());
                return Ok(None);
            }

            // 服务端帧序号对齐（宽进：跟随服务端，重连/重传边界不误杀）
            let _ = frame_seq;
            self.outer_seq_id = frame_seq.wrapping_add(1);

            let mut frame = src.split_to(7 + frame_len);
            frame.advance(7);
            let ob20_packet: Vec<u8> = if uncompressed_len == 0 {
                frame.to_vec()
            } else {
                Self::zlib_decompress(&frame, uncompressed_len as usize)
                    .map_err(|e| IoError::Io(io::Error::other(format!("ob20 解压失败：{e}"))))?
            };

            // 头 CRC16 的 prefix7 口径：complen 恒 0（libobclient 读侧同口径）
            let mut prefix7 = [0u8; 7];
            prefix7[0..3].copy_from_slice(&(ob20_packet.len() as u32).to_le_bytes()[0..3]);
            prefix7[3] = frame_seq;
            let (_header, mysql_packet) = ob20::unwrap_packet(&prefix7, &ob20_packet)
                .map_err(|e| IoError::Io(io::Error::other(e.to_string())))?;

            // 剥 MySQL 4B 头，多块重组
            if mysql_packet.len() < 4 {
                return Err(IoError::Io(io::Error::other("ob20 内 MySQL 包不足 4B 头")));
            }
            let chunk_len =
                (u32::from(mysql_packet[0]) | (u32::from(mysql_packet[1]) << 8) | (u32::from(mysql_packet[2]) << 16))
                    as usize;
            if mysql_packet.len() != 4 + chunk_len {
                return Err(IoError::Io(io::Error::other(
                    "ob20 内 MySQL 包长度与头不符",
                )));
            }
            self.decode_buf.as_mut().extend_from_slice(&mysql_packet[4..]);

            if chunk_len < OB20_MAX_CHUNK {
                // 逻辑包结束：换新缓冲返回
                let done = std::mem::replace(&mut self.decode_buf, crate::buffer_pool().get());
                return Ok(Some(done));
            }
            // 继续读后续帧（同 request 的续块）
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pooled(data: &[u8]) -> PooledBuf {
        let mut buf = crate::buffer_pool().get();
        buf.as_mut().extend_from_slice(data);
        buf
    }

    /// 全链路往返：encode（分块 + ob20 + zlib + 外层帧）→ decode 逐帧还原，
    /// payload 逐字节一致；含小包（不压缩路径）与较大包（压缩路径）
    #[test]
    fn encode_decode_roundtrip() {
        for payload in [
            &b"SELECT 1 FROM DUAL"[..],
            &vec![b'x'; 700][..],
            b"",
        ] {
            let mut codec = Ob20Codec::new(0x1122_3344, 1024 * 1024);
            let mut wire = BytesMut::new();
            codec.encode(pooled(payload), &mut wire).expect("encode");
            let decoded = codec
                .decode(&mut wire)
                .expect("decode")
                .expect("complete packet");
            assert_eq!(decoded.as_ref(), payload, "payload mismatch");
            assert!(codec.decode(&mut wire).expect("decode").is_none());
        }
    }

    /// 命令边界：两次 encode-decode 顺序还原、request_id 前进
    #[test]
    fn command_boundaries() {
        let mut codec = Ob20Codec::new(1, 65536);
        let first_request = codec.ctx.request_id;
        let mut wire = BytesMut::new();
        codec.encode(pooled(b"ping1"), &mut wire).unwrap();
        codec.encode(pooled(b"ping2"), &mut wire).unwrap();
        let one = codec.decode(&mut wire).unwrap().unwrap();
        assert_eq!(one.as_ref(), b"ping1");
        let two = codec.decode(&mut wire).unwrap().unwrap();
        assert_eq!(two.as_ref(), b"ping2");
        assert_ne!(codec.ctx.request_id, first_request);
    }
}
