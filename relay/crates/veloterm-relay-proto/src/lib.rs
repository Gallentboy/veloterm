//! veloterm 中继线缆协议（VLR2）与鉴权的单一实现。
//!
//! 三个消费方共用本 crate，杜绝协议逻辑重复：
//! - `veloterm-relay` 服务端：[`handshake::server_handshake`] 解析入站、校验证明；
//! - veloterm 客户端（src-tauri，P3 接入）：[`handshake::client_handshake`] 发起握手；
//! - 双端的 UDP 模式：[`udp`] 模块（时间同步 + 数据报编解码）。
//!
//! 鉴权模型：双 nonce 挑战响应（流式通道）与时间窗 HMAC（UDP），
//! 秘密（[`auth::Secret`]，32 字节）永不上线。

pub mod auth;
pub mod crypto;
pub mod error;
pub mod frame;
pub mod handshake;
pub mod udp;
