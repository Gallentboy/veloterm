//! 协议层错误类型。

use thiserror::Error;

/// 帧解码失败的原因。协议头在魔法数里带版本（`VLR2`），
/// 版本不匹配同样落在这里的 [`ProtoError::BadMagic`]。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProtoError {
    #[error("magic 不匹配（协议版本不符或非 veloterm-relay 流量）")]
    BadMagic,
    #[error("帧长度非法（过短、超限或与 flags 声明不符）")]
    BadLength,
    #[error("flags/status/host 取值非法")]
    BadValue,
    #[error("UTF-8 解码失败")]
    BadUtf8,
}
