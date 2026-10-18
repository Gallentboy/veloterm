//! TOML 配置装载与校验。
//!
//! ```toml
//! [relay]
//! max_conns = 4096
//! handshake_timeout_secs = 10
//!
//! [[token]]
//! key_id = 1
//! name  = "macbook"
//! secret = "<veloterm_relay token generate 生成>"
//!
//! [[listener]]
//! proto = "tcp"                # P1: tcp；udp/ws/quic 在后续阶段放开
//! bind = "0.0.0.0:7300"
//! target = "linux1.internal:22"     # 默认目标（客户端不带 target 时用）
//! allow_targets = ["10.0.0.5:22"]   # 客户端可显式指定的精确白名单（可选）
//! ```

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, bail};
use serde::Deserialize;
use veloterm_relay_proto::frame::TargetAddr;
use veloterm_relay_proto::handshake::{AuthEntry, ServerAuth};
use veloterm_relay_proto::auth::Secret;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    #[serde(default)]
    pub relay: RelaySection,
    #[serde(default)]
    pub token: Vec<TokenEntry>,
    #[serde(default)]
    pub listener: Vec<ListenerEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RelaySection {
    pub max_conns: usize,
    pub handshake_timeout_secs: u64,
}

impl Default for RelaySection {
    fn default() -> Self {
        Self { max_conns: 4096, handshake_timeout_secs: 10 }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenEntry {
    pub key_id: u8,
    pub name: String,
    pub secret: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListenerEntry {
    pub proto: String,
    pub bind: String,
    pub target: Option<String>,
    #[serde(default)]
    pub allow_targets: Vec<String>,
    /// UDP 会话空闲回收秒数
    #[serde(default)]
    pub udp_idle_timeout_secs: Option<u64>,
    /// QUIC 证书/私钥 PEM 路径（不配则启动时自签并日志指纹）
    #[serde(default)]
    pub cert: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
}

/// 校验后的运行时配置。
#[derive(Debug)]
pub struct ResolvedConfig {
    pub max_conns: usize,
    pub handshake_timeout: Duration,
    pub auth: ServerAuth,
    pub listeners: Vec<ResolvedListener>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenerProto {
    Tcp,
    Ws,
    Udp,
    Quic,
}

#[derive(Debug, Clone)]
pub struct ResolvedListener {
    pub proto: ListenerProto,
    pub bind: SocketAddr,
    pub default_target: Option<TargetAddr>,
    pub allowed: Vec<TargetAddr>,
    pub udp_idle_timeout: Duration,
    pub tls_cert: Option<std::path::PathBuf>,
    pub tls_key: Option<std::path::PathBuf>,
}

pub fn parse_toml(text: &str) -> anyhow::Result<FileConfig> {
    toml::from_str(text).context("配置文件解析失败")
}

/// init 生成的配置模板（tcp 默认启用，ws/quic 注释待启）。
/// 收敛到 config 模块：main 的 `init` 子命令与 install 的「现场生成配置」共用同一份。
pub fn init_template(secret_hex: &str) -> String {
    format!(
        r####"# veloterm_relay 配置（init 生成）｜校验：veloterm_relay --check｜文档：relay/README.md
# 目标无需配置：客户端自动携带所连主机地址；allow_targets 为可选白名单

[relay]
# max_conns = 4096              # 全局并发上限
# handshake_timeout_secs = 10   # VLR2 握手超时（秒）

# 凭证：每台设备一条，泄露单独删段即吊销（veloterm_relay token generate 可再加）
[[token]]
key_id = 1
name  = "device-1"
secret = "{secret_hex}"

# ── 监听器：默认启用 tcp；ws/quic 去掉每行行首 "# " 即启用 ──────────────

[[listener]]
proto = "tcp"
bind = "0.0.0.0:7300"
# allow_targets = ["10.0.0.5:22", "linux1.internal:22"]   # 可选：限制凭证可达目标

# ── WebSocket：出口只放行 HTTP(S)、经 Nginx 等反代转 wss 时启用 ────────
# [[listener]]
# proto = "ws"
# bind = "0.0.0.0:7301"

# ── QUIC：公网推荐（内建 TLS 1.3，未配证书自动自签并把指纹打进日志）────
# [[listener]]
# proto = "quic"
# bind = "0.0.0.0:7302"
# cert = "/path/to/cert.pem"   # 可选；不配则启动自签
# key  = "/path/to/key.pem"
"####
    )
}

impl FileConfig {
    pub fn resolve(self) -> anyhow::Result<ResolvedConfig> {
        if self.token.is_empty() {
            bail!("至少需要一条 [[token]]（用 `veloterm_relay token generate` 生成）");
        }
        if self.listener.is_empty() {
            bail!("至少需要一个 [[listener]]");
        }

        let mut auth = ServerAuth::new();
        for t in &self.token {
            if auth.get(t.key_id).is_some() {
                bail!("token key_id={} 重复", t.key_id);
            }
            let secret = Secret::from_hex(t.secret.trim())
                .map_err(|e| anyhow::anyhow!("token key_id={} 的 secret 非法（{}，需 64 位十六进制/32 字节）", t.key_id, e))?;
            auth.add(AuthEntry { key_id: t.key_id, name: t.name.clone(), secret });
        }

        let mut listeners = Vec::with_capacity(self.listener.len());
        for (i, l) in self.listener.into_iter().enumerate() {
            let proto = match l.proto.as_str() {
                "tcp" => ListenerProto::Tcp,
                "ws" => ListenerProto::Ws,
                "udp" => ListenerProto::Udp,
                "quic" => ListenerProto::Quic,
                other => bail!("listener[{}] proto=\"{}\" 未知（tcp/udp/ws/quic）", i, other),
            };
            let bind: SocketAddr = l
                .bind
                .parse()
                .with_context(|| format!("listener[{}] bind=\"{}\" 不是合法的 host:port", i, l.bind))?;
            let default_target = match &l.target {
                Some(t) => Some(parse_target(i, t)?),
                None => None,
            };
            // UDP 无连接且数据报不携带目标，必须固定默认目标；
            // 流式监听器（tcp/ws/quic）无需任何目标配置——客户端默认携带
            // 目标（其配置的主机地址），不配 allow_targets 即为
            // 「令牌鉴权的开放中继」，配了则按白名单限制
            if proto == ListenerProto::Udp && default_target.is_none() {
                bail!("listener[{}] udp 监听器必须配置 target（固定目标）", i);
            }
            if !matches!(proto, ListenerProto::Udp) && l.udp_idle_timeout_secs.is_some() {
                bail!("listener[{}] udp_idle_timeout_secs 只适用于 udp 监听器", i);
            }
            if !matches!(proto, ListenerProto::Quic) && (l.cert.is_some() || l.key.is_some()) {
                bail!("listener[{}] cert/key 只适用于 quic 监听器", i);
            }
            if proto == ListenerProto::Quic && l.cert.is_some() != l.key.is_some() {
                bail!("listener[{}] cert 与 key 必须同时配置", i);
            }
            let allowed = l
                .allow_targets
                .iter()
                .map(|t| parse_target(i, t))
                .collect::<Result<Vec<_>, _>>()?;
            listeners.push(ResolvedListener {
                proto,
                bind,
                default_target,
                allowed,
                udp_idle_timeout: Duration::from_secs(l.udp_idle_timeout_secs.unwrap_or(60)),
                tls_cert: l.cert.map(std::path::PathBuf::from),
                tls_key: l.key.map(std::path::PathBuf::from),
            });
        }

        Ok(ResolvedConfig {
            max_conns: self.relay.max_conns,
            handshake_timeout: Duration::from_secs(self.relay.handshake_timeout_secs),
            auth,
            listeners,
        })
    }
}

fn parse_target(idx: usize, s: &str) -> anyhow::Result<TargetAddr> {
    TargetAddr::parse(s).with_context(|| format!("listener[{idx}] 目标 \"{s}\" 非法（应为 host:port）"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_rejects_bad_configs() {
        let secret64 = "ab".repeat(32);

        // 无 token
        let no_token = parse_toml(
            r#"
            [relay]
            [[listener]]
            proto = "tcp"
            bind = "0.0.0.0:7300"
            target = "a:22"
            "#,
        )
        .unwrap();
        assert!(no_token.resolve().is_err());

        // 未知 proto（tcp/udp/ws/quic 之外）
        let future = parse_toml(&format!(
            r#"
            [relay]
            [[token]]
            key_id = 1
            name = "x"
            secret = "{secret64}"
            [[listener]]
            proto = "grpc"
            bind = "0.0.0.0:7300"
            target = "a:22"
            "#
        ))
        .unwrap();
        let err = future.resolve().unwrap_err().to_string();
        assert!(err.contains("未知"), "{err}");

        // tcp 流式监听器无 target/allow_targets = 开放中继（合法）；
        // 相应语义锁定：udp 无 target 必须报错
        let open_tcp = parse_toml(&format!(
            r#"
            [relay]
            [[token]]
            key_id = 1
            name = "x"
            secret = "{secret64}"
            [[listener]]
            proto = "tcp"
            bind = "0.0.0.0:7300"
            "#
        ))
        .unwrap();
        assert!(open_tcp.resolve().is_ok());

        let udp_no_target = parse_toml(&format!(
            r#"
            [[token]]
            key_id = 1
            name = "x"
            secret = "{secret64}"
            [[listener]]
            proto = "udp"
            bind = "0.0.0.0:7303"
            "#
        ))
        .unwrap();
        let err = udp_no_target.resolve().unwrap_err().to_string();
        assert!(err.contains("固定目标"), "{err}");

        // secret 长度非法
        let bad_secret = parse_toml(
            r#"
            [[token]]
            key_id = 1
            name = "x"
            secret = "abcd"
            [[listener]]
            proto = "tcp"
            bind = "127.0.0.1:0"
            target = "a:22"
            "#,
        )
        .unwrap();
        assert!(bad_secret.resolve().is_err());
    }

    #[test]
    fn resolve_happy_path() {
        let secret64 = "ab".repeat(32);
        let cfg = parse_toml(&format!(
            r#"
            [[token]]
            key_id = 7
            name = "mbp"
            secret = "{secret64}"
            [[listener]]
            proto = "tcp"
            bind = "127.0.0.1:17300"
            target = "linux1.internal:22"
            allow_targets = ["10.0.0.5:22", "[::1]:2222"]
            "#
        ))
        .unwrap()
        .resolve()
        .unwrap();
        assert_eq!(cfg.max_conns, 4096); // 默认值
        assert_eq!(cfg.auth.len(), 1);
        assert_eq!(cfg.listeners.len(), 1);
        let l = &cfg.listeners[0];
        assert_eq!(l.bind.port(), 17300);
        assert_eq!(l.default_target.as_ref().unwrap().host, "linux1.internal");
        assert_eq!(l.allowed.len(), 2);
        assert_eq!(l.allowed[1].host, "::1"); // [::1] 去括号
    }
}
