use std::fmt::{Display, Formatter};

use bytes::Bytes;
use russh::{ChannelId, Pty, Sig};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 限制终端尺寸在合理区间，防止异常尺寸导致内存分配过大（对齐 `warpgate_core::sane_terminal_size`）。
pub fn sane_terminal_size(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(2, 1000), rows.clamp(1, 500))
}

#[derive(Clone, Debug)]
pub struct PtyRequest {
    pub term: String,
    pub col_width: u32,
    pub row_height: u32,
    pub pix_width: u32,
    pub pix_height: u32,
    pub modes: Vec<(Pty, u32)>,
}

impl PtyRequest {
    /// Terminal dimensions as `(cols, rows)`, bounded by [`sane_terminal_size`].
    pub fn screen_size(&self) -> (u16, u16) {
        sane_terminal_size(
            u16::try_from(self.col_width).unwrap_or(u16::MAX),
            u16::try_from(self.row_height).unwrap_or(u16::MAX),
        )
    }
}

pub fn make_pty_request(cols: u32, rows: u32) -> PtyRequest {
    let (safe_cols, safe_rows) = sane_terminal_size(
        u16::try_from(cols).unwrap_or(80),
        u16::try_from(rows).unwrap_or(24),
    );
    PtyRequest {
        term: "xterm-256color".to_owned(),
        col_width: u32::from(safe_cols),
        row_height: u32::from(safe_rows),
        pix_width: 0,
        pix_height: 0,
        modes: vec![],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Hash, Eq)]
pub struct ServerChannelId(pub ChannelId);

impl Display for ServerChannelId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug)]
pub struct DirectTCPIPParams {
    pub host_to_connect: String,
    pub port_to_connect: u32,
    pub originator_address: String,
    pub originator_port: u32,
}

#[derive(Clone, Debug)]
pub struct ForwardedTcpIpParams {
    pub connected_address: String,
    pub connected_port: u32,
    pub originator_address: String,
    pub originator_port: u32,
}

#[derive(Clone, Debug)]
pub struct ForwardedStreamlocalParams {
    pub socket_path: String,
}

#[derive(Clone, Debug)]
pub struct X11Request {
    pub single_conection: bool,
    pub x11_auth_protocol: String,
    pub x11_auth_cookie: String,
    pub x11_screen_number: u32,
}

#[derive(Clone, Debug)]
pub enum ChannelOperation {
    OpenShell,
    OpenDirectTCPIP(DirectTCPIPParams),
    OpenDirectStreamlocal(String),
    #[allow(dead_code)]
    OpenX11(String, u32),
    RequestPty(PtyRequest),
    ResizePty(PtyRequest),
    RequestShell,
    RequestEnv(String, String),
    #[allow(dead_code)]
    RequestExec(String),
    #[allow(dead_code)]
    RequestX11(X11Request),
    #[allow(dead_code)]
    AgentForward,
    #[allow(dead_code)]
    RequestSubsystem(String),
    Data(Bytes),
    #[allow(dead_code)]
    ExtendedData {
        data: Bytes,
        ext: u32,
    },
    Close,
    #[allow(dead_code)]
    Eof,
    #[allow(dead_code)]
    Signal(Sig),
}

/// 主机公钥校验策略（对齐 Warpgate `SshHostKeyVerificationMode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SshHostKeyVerificationMode {
    /// 首次连接自动信任并记录，后续连接若公钥不匹配则拒绝（TOFU）
    #[default]
    AutoAccept,
    /// 忽略主机公钥校验
    Ignore,
    /// 未知主机交由上层交互确认
    Prompt,
    /// 未知主机直接拒绝
    AutoReject,
}

/// SSH 目标认证配置（对齐前端 `AuthConfig` 与 Warpgate `SSHTargetAuth`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "method", rename_all = "lowercase")]
pub enum SSHTargetAuth {
    Password {
        password: String,
    },
    #[serde(alias = "publickey")]
    Key {
        #[serde(rename = "privateKey", alias = "private_key")]
        private_key: String,
        #[serde(default)]
        passphrase: Option<String>,
    },
    #[default]
    Agent,
}

fn default_ssh_port() -> u16 {
    22
}

fn default_ssh_username() -> String {
    "root".to_string()
}

fn default_allow_insecure_algos() -> bool {
    true
}

/// 中继入站传输类型（veloterm_relay 的监听协议）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelayTransport {
    Tcp,
    Ws,
    Quic,
}

/// 经 veloterm_relay 中继连接 SSH 端点的参数（挂在连接链首跳上）。
/// secret 为 32 字节十六进制（veloterm_relay token generate 生成），存 SQLCipher 库。
/// 目标不需要配置：客户端自动携带所连主机（或链首跳板）的 host:port 作为中继目标。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshRelayEndpoint {
    pub host: String,
    pub port: u16,
    pub transport: RelayTransport,
    pub key_id: u8,
    pub secret: String,
}

/// 单个 SSH 目标或跳板机连接参数（迁移自 `warpgate_common::TargetSSHOptions`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetSSHOptions {
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    #[serde(default = "default_ssh_username")]
    pub username: String,
    #[serde(
        default = "default_allow_insecure_algos",
        alias = "allow_insecure_algos"
    )]
    pub allow_insecure_algos: bool,
    #[serde(default)]
    pub auth: SSHTargetAuth,
    #[serde(default, alias = "jump_host")]
    pub jump_host: Option<Uuid>,
    #[serde(default)]
    pub host_key_verification: SshHostKeyVerificationMode,
    /// 中继（仅链首有效）：有值时首跳经 veloterm_relay 建立 VLR2 流再 connect_stream。
    #[serde(default)]
    pub relay: Option<SshRelayEndpoint>,
}
