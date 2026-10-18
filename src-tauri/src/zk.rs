//! ZooKeeper 客户端（zookeeper-client crate，纯 Rust/tokio）：
//! 连接管理 + znode 树浏览/数据读写/ACL 读写命令，供前端 ZK 管理页使用。
//!
//! 中继支持（与终端同一条 VLR3 路径）：crate 会话层与 TcpStream 硬耦合
//! （非阻塞读 + 断线重拨），不引入 vendor 补丁，改用**本地桥接**——
//! 回环监听 → 每入连经 dial_relay（VLR3 加密管道）转到 ZK 目标；
//! ZK 客户端连桥接口即可，重连自动再走中继。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha1::Digest as _;
use tauri::{Emitter as _, State};
use tokio::net::TcpListener;
use tokio::sync::watch;
use zookeeper_client::{Acl, AuthId, Client, CreateMode, CreateOptions, Permission};

use crate::terminal::ssh::common::SshRelayEndpoint;
use crate::terminal::ssh::relay_transport::dial_relay;

/// watch 事件名（前端 listen 用）。
pub const ZK_WATCH_EVENT: &str = "zk-watch";

/// ZK 会话表（SftpState 同型：tauri manage 注入）。
pub struct ZkState {
    next_id: AtomicUsize,
    sessions: Mutex<HashMap<String, ZkSession>>,
    /// AppHandle（setup 后注入）：watch 事件发射用
    pub app: std::sync::Mutex<Option<tauri::AppHandle>>,
}

/// 一个 ZK 会话 = 客户端 + （可选）中继桥接任务 + watch 注册表。
pub struct ZkSession {
    client: Arc<Client>,
    bridge: Option<tokio::task::JoinHandle<()>>,
    bridge_shutdown: Option<watch::Sender<()>>,
    /// 已注册的 watch（path + kind），防重复注册；任务退出时自移除
    watches: Arc<Mutex<HashSet<(String, &'static str)>>>,
}

impl Default for ZkState {
    fn default() -> Self {
        Self::new()
    }
}

impl ZkState {
    pub fn new() -> Self {
        Self {
            next_id: AtomicUsize::new(1),
            sessions: Mutex::new(HashMap::new()),
            app: std::sync::Mutex::new(None),
        }
    }

    /// setup 后注入，供 watch 事件发射（SftpState::set_app 同型）。
    pub fn set_app(&self, app: tauri::AppHandle) {
        if let Ok(mut slot) = self.app.lock() {
            *slot = Some(app);
        }
    }

    /// 取会话客户端的克隆句柄（锁内浅拷贝、锁外 await，避免持锁跨 await）。
    fn client(&self, session_id: &str) -> Result<Arc<Client>, String> {
        Ok(self
            .sessions
            .lock()
            .map_err(|_| "ZK 会话表锁中毒".to_string())?
            .get(session_id)
            .ok_or_else(|| format!("ZK 会话 {session_id} 不存在（已断开？）"))?
            .client
            .clone())
    }

    /// 取会话整体（拿 watch 注册表等）。
    fn session(&self, session_id: &str) -> Option<ZkSessionView> {
        self.sessions
            .lock()
            .ok()?
            .get(session_id)
            .map(|s| ZkSessionView { watches: s.watches.clone() })
    }
}

/// 会话只读视图（避免锁外持有借用）。
struct ZkSessionView {
    watches: Arc<Mutex<HashSet<(String, &'static str)>>>,
}

/// znode stat（camelCase 给前端）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeStat {
    pub czxid: i64,
    pub mzxid: i64,
    pub ctime: i64,
    pub mtime: i64,
    pub version: i32,
    pub cversion: i32,
    pub aversion: i32,
    pub ephemeral_owner: i64,
    pub data_length: i32,
    pub num_children: i32,
}

impl NodeStat {
    fn from_stat(stat: &zookeeper_client::Stat) -> Self {
        Self {
            czxid: stat.czxid,
            mzxid: stat.mzxid,
            ctime: stat.ctime,
            mtime: stat.mtime,
            version: stat.version,
            cversion: stat.cversion,
            aversion: stat.aversion,
            ephemeral_owner: stat.ephemeral_owner,
            data_length: stat.data_length,
            num_children: stat.num_children,
        }
    }
}

/// ACL 规则（scheme,id,perms)；perms 为位集：R1 W2 C4 D8 A16（ALL=31）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclRule {
    pub scheme: String,
    pub id: String,
    pub perms: u32,
}

fn bits_permission(bits: u32) -> Permission {
    let mut perm = Permission::NONE;
    for (flag, p) in [
        (1u32, Permission::READ),
        (2, Permission::WRITE),
        (4, Permission::CREATE),
        (8, Permission::DELETE),
        (16, Permission::ADMIN),
    ] {
        if bits & flag != 0 {
            perm = perm | p;
        }
    }
    perm
}

fn rule_to_acl(rule: &AclRule) -> Result<Acl, String> {
    let scheme = rule.scheme.trim();
    let id = rule.id.trim();
    if scheme.is_empty() {
        return Err("ACL scheme 不能为空（world/auth/digest/ip/sasl）".into());
    }
    match scheme {
        "world" if id != "anyone" => Err("world scheme 的 id 只能是 anyone".into()),
        "auth" if !id.is_empty() => Err("auth scheme 的 id 必须为空".into()),
        "digest" if id.is_empty() => {
            Err("digest scheme 需要 id（base64(user:pass)，可用 digest_id 计算）".into())
        }
        "ip" if id.is_empty() => Err("ip scheme 需要 id（IP 或 IP 段）".into()),
        _ => Ok(Acl::new(bits_permission(rule.perms), AuthId::new(scheme, id))),
    }
}

/// crate 0.2 未暴露 Acl getter，经 Debug 格式解析
/// （形如 `Acl { permission: Permission(31), auth_id: AuthId { scheme: "world", id: "anyone" } }`；
/// Permission 的元组值即权限位集：R1 W2 C4 D8 A16）。
fn acl_to_rule(acl: &Acl) -> AclRule {
    let text = format!("{acl:?}");
    let perms = text
        .split("permission: Permission(")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(0);
    let scheme = text
        .split("scheme: \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or("")
        .to_string();
    let id = text
        .split("id: \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or("")
        .to_string();
    AclRule { scheme, id, perms }
}

fn zk_err(context: &str, e: zookeeper_client::Error) -> String {
    format!("ZK {context}失败: {e:?}")
}

/// watch 种类：children（子节点增删）或 data（数据/ACL/删除）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatchKind {
    Children,
    Data,
}

impl WatchKind {
    fn key(self) -> &'static str {
        match self {
            WatchKind::Children => "children",
            WatchKind::Data => "data",
        }
    }
}

/// 确保（path, kind）的 watch 存在：一次性 watcher 触发后重注册，直至会话断开或
/// 节点消失；每次触发向前端发射 `zk-watch` 事件 {sessionId, path, kind}。
/// 任务退出时从注册表自移除，节点重建后下次 list/get 能重新注册。
fn ensure_watch(
    state: &ZkState,
    session_id: &str,
    client: Arc<Client>,
    watches: Arc<Mutex<HashSet<(String, &'static str)>>>,
    path: String,
    kind: WatchKind,
) {
    let key = (path.clone(), kind.key());
    let already_watched = {
        let Ok(mut set) = watches.lock() else { return };
        !set.insert(key)
    };
    if already_watched {
        return; // 已在监听
    }
    let app = state.app.lock().ok().and_then(|slot| slot.clone());
    let session_id = session_id.to_string();
    tokio::spawn(async move {
        loop {
            // 注册一次性 watcher（成功即挂上）
            let watcher = match kind {
                WatchKind::Children => match client.get_and_watch_children(&path).await {
                    Ok((_, _, w)) => w,
                    Err(_) => break, // 会话断开或节点已删
                },
                WatchKind::Data => match client.get_and_watch_data(&path).await {
                    Ok((_, _, w)) => w,
                    Err(_) => break,
                },
            };
            let event = watcher.changed().await;
            if event.session_state.is_terminated() {
                break;
            }
            if let Some(app) = &app {
                let _ = app.emit(
                    ZK_WATCH_EVENT,
                    serde_json::json!({
                        "sessionId": session_id,
                        "path": event.path,
                        "kind": kind.key(),
                    }),
                );
            }
        }
        if let Ok(mut set) = watches.lock() {
            set.remove(&(path, kind.key()));
        }
    });
}

#[tauri::command]
pub async fn zk_connect(
    state: State<'_, ZkState>,
    endpoints: String,
    relay: Option<SshRelayEndpoint>,
) -> Result<serde_json::Value, String> {
    let endpoints = endpoints.trim().to_string();
    if endpoints.is_empty() {
        return Err("ZK 地址不能为空（host:2181，逗号分隔多个）".into());
    }

    // 中继模式：起本地桥接（回环监听 → VLR3 管道 → ZK 目标）。
    // 目标取地址串里的第一个 host:port（中继模式下多地址无意义）。
    let (connect_str, bridge, bridge_shutdown) = if let Some(relay_cfg) = relay {
        let target = endpoints.split(',').next().unwrap_or_default().trim();
        let (host, port) = target
            .rsplit_once(':')
            .ok_or_else(|| format!("中继模式需单一目标 host:port（当前 \"{endpoints}\"）"))?;
        let port: u16 = port.parse().map_err(|_| format!("端口非法：{port}"))?;
        let ssh_endpoint =
            veloterm_relay_proto::frame::TargetAddr { host: host.to_string(), port };
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("本地桥接监听绑定失败: {e}"))?;
        let local_port = listener
            .local_addr()
            .map_err(|e| format!("本地桥接监听地址获取失败: {e}"))?
            .port();
        let (shutdown_tx, mut shutdown_rx) = watch::channel(());
        let relay_for_task = relay_cfg;
        let target_for_task = ssh_endpoint;
        let task = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => break,
                    accepted = listener.accept() => match accepted {
                        Ok((inbound, _)) => {
                            let relay = relay_for_task.clone();
                            let target = target_for_task.clone();
                            tasks.spawn(async move {
                                if let Ok(pipe) = dial_relay(&relay, &target).await {
                                    let (mut a, mut b) = (inbound, pipe);
                                    let _ = tokio::io::copy_bidirectional(&mut a, &mut b).await;
                                }
                            });
                        }
                        Err(_) => break,
                    },
                }
            }
            tasks.abort_all();
        });
        (format!("127.0.0.1:{local_port}"), Some(task), Some(shutdown_tx))
    } else {
        (endpoints.clone(), None, None)
    };

    let client = Client::connect(&connect_str, Duration::from_secs(8))
        .await
        .map_err(|e| {
            if let Some(tx) = &bridge_shutdown {
                let _ = tx.send(());
            }
            format!("连接 ZK {endpoints} 失败: {e:?}")
        })?;
    let id = format!(
        "zk-{}-{}",
        std::process::id(),
        state.next_id.fetch_add(1, Ordering::Relaxed)
    );
    state
        .sessions
        .lock()
        .map_err(|_| "ZK 会话表锁中毒".to_string())?
        .insert(id.clone(), ZkSession {
            client: Arc::new(client),
            bridge,
            bridge_shutdown,
            watches: Arc::new(Mutex::new(HashSet::new())),
        });
    Ok(serde_json::json!({ "sessionId": id }))
}

#[tauri::command]
pub async fn zk_disconnect(state: State<'_, ZkState>, session_id: String) -> Result<(), String> {
    let removed = state
        .sessions
        .lock()
        .map_err(|_| "ZK 会话表锁中毒".to_string())?
        .remove(&session_id);
    if let Some(session) = removed {
        // 先停桥接再 drop 客户端；ZK 会话超时后服务端清理临时节点
        if let Some(tx) = &session.bridge_shutdown {
            let _ = tx.send(());
        }
        if let Some(task) = &session.bridge {
            task.abort();
        }
        drop(session.client);
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZkChildren {
    pub children: Vec<String>,
    pub stat: NodeStat,
}

#[tauri::command]
pub async fn zk_list(
    state: State<'_, ZkState>,
    session_id: String,
    path: String,
) -> Result<ZkChildren, String> {
    let client = state.client(&session_id)?;
    let (children, stat) = client
        .get_children(&path)
        .await
        .map_err(|e| zk_err(&format!("列出子节点 {path} "), e))?;
    // 挂子节点 watch（幂等）：别的客户端增删子节点时前端自动收到 zk-watch
    if let Some(session) = state.session(&session_id) {
        ensure_watch(
            &state,
            &session_id,
            client,
            session.watches.clone(),
            path.clone(),
            WatchKind::Children,
        );
    }
    Ok(ZkChildren { children, stat: NodeStat::from_stat(&stat) })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZkNode {
    /// 节点数据：utf8 有效时为原文；否则为 Base64（encoding 标识）。
    pub data: String,
    /// 数据编码："utf8" | "base64"（二进制节点以 Base64 呈现与编辑，写回时解码）
    pub encoding: String,
    pub stat: NodeStat,
    pub acls: Vec<AclRule>,
}

#[tauri::command]
pub async fn zk_get(
    state: State<'_, ZkState>,
    session_id: String,
    path: String,
) -> Result<ZkNode, String> {
    let client = state.client(&session_id)?;
    let (data, stat) = client
        .get_data(&path)
        .await
        .map_err(|e| zk_err(&format!("读取 {path} "), e))?;
    let (acls, _) = client
        .get_acl(&path)
        .await
        .map_err(|e| zk_err(&format!("读取 {path} ACL "), e))?;
    // 二进制安全：非法 UTF-8 的数据以 Base64 呈现，写回时按 encoding 解码，
    // 杜绝 lossy 转换弄坏序列化二进制（Kafka/dubbo 配置等）
    let (text, encoding) = match std::str::from_utf8(&data) {
        Ok(s) => (s.to_string(), "utf8".to_string()),
        Err(_) => (
            base64::engine::general_purpose::STANDARD.encode(&data),
            "base64".to_string(),
        ),
    };
    // 挂数据 watch（幂等）：数据/ACL/删除变更时前端自动收到 zk-watch
    if let Some(session) = state.session(&session_id) {
        ensure_watch(
            &state,
            &session_id,
            client,
            session.watches.clone(),
            path.clone(),
            WatchKind::Data,
        );
    }
    Ok(ZkNode {
        data: text,
        encoding,
        stat: NodeStat::from_stat(&stat),
        acls: acls.iter().map(acl_to_rule).collect(),
    })
}

#[tauri::command]
pub async fn zk_set_data(
    state: State<'_, ZkState>,
    session_id: String,
    path: String,
    data: String,
    // "utf8"（原文写回）| "base64"（解码后写回，二进制节点）
    encoding: Option<String>,
    version: i32,
) -> Result<NodeStat, String> {
    let client = state.client(&session_id)?;
    let bytes: Vec<u8> = match encoding.as_deref().unwrap_or("utf8") {
        "base64" => base64::engine::general_purpose::STANDARD
            .decode(data.trim())
            .map_err(|e| format!("Base64 解码失败（二进制节点需合法 Base64）: {e}"))?,
        _ => data.into_bytes(),
    };
    let expected = if version < 0 { None } else { Some(version) };
    let stat = client
        .set_data(&path, &bytes, expected)
        .await
        .map_err(|e| zk_err(&format!("写入 {path} "), e))?;
    Ok(NodeStat::from_stat(&stat))
}

#[tauri::command]
pub async fn zk_create(
    state: State<'_, ZkState>,
    session_id: String,
    parent: String,
    name: String,
    data: String,
    ephemeral: bool,
    sequential: bool,
) -> Result<serde_json::Value, String> {
    let name = name.trim().trim_start_matches('/').to_string();
    if name.is_empty() {
        return Err("节点名不能为空".into());
    }
    let path = format!("{}/{}", parent.trim_end_matches('/'), name);
    let client = state.client(&session_id)?;
    let mode = match (ephemeral, sequential) {
        (false, false) => CreateMode::Persistent,
        (false, true) => CreateMode::PersistentSequential,
        (true, false) => CreateMode::Ephemeral,
        (true, true) => CreateMode::EphemeralSequential,
    };
    let options = CreateOptions::new(mode, Acl::anyone_all());
    let (stat, seq) = client
        .create(&path, data.as_bytes(), &options)
        .await
        .map_err(|e| zk_err(&format!("创建 {path} "), e))?;
    // 顺序节点返回实际序号（CreateSequence 的 Display 已按 {:010} 格式化）
    let created = if sequential { format!("{path}{seq}") } else { path };
    Ok(serde_json::json!({ "createdPath": created, "stat": NodeStat::from_stat(&stat) }))
}

#[tauri::command]
pub async fn zk_delete(
    state: State<'_, ZkState>,
    session_id: String,
    path: String,
) -> Result<(), String> {
    if path == "/" {
        return Err("不能删除根节点".into());
    }
    let client = state.client(&session_id)?;
    client
        .delete(&path, None)
        .await
        .map_err(|e| zk_err(&format!("删除 {path} "), e))?;
    Ok(())
}

#[tauri::command]
pub async fn zk_set_acl(
    state: State<'_, ZkState>,
    session_id: String,
    path: String,
    acls: Vec<AclRule>,
    aversion: i32,
) -> Result<NodeStat, String> {
    if acls.is_empty() {
        return Err("ACL 列表不能为空（ZK 要求至少一条规则）".into());
    }
    let parsed: Vec<Acl> = acls.iter().map(rule_to_acl).collect::<Result<_, _>>()?;
    let client = state.client(&session_id)?;
    let expected = if aversion < 0 { None } else { Some(aversion) };
    let stat = client
        .set_acl(&path, &parsed, expected)
        .await
        .map_err(|e| zk_err(&format!("设置 {path} ACL "), e))?;
    Ok(NodeStat::from_stat(&stat))
}

/// 计算 digest scheme 的 id：base64(sha1("user:password"))。
#[tauri::command]
pub fn zk_digest_id(user: String, password: String) -> Result<String, String> {
    if user.is_empty() || password.is_empty() {
        return Err("digest 的用户名与密码不能为空".into());
    }
    let auth = format!("{user}:{password}");
    let digest = sha1::Sha1::digest(auth.as_bytes());
    Ok(base64::engine::general_purpose::STANDARD.encode(digest))
}

/// 会话认证（digest）：认证后可操作 auth:/digest:自身 的 ACL 节点。
#[tauri::command]
pub async fn zk_auth(
    state: State<'_, ZkState>,
    session_id: String,
    user: String,
    password: String,
) -> Result<(), String> {
    if user.is_empty() {
        return Err("认证用户名不能为空".into());
    }
    let client = state.client(&session_id)?;
    let auth = format!("{user}:{password}").into_bytes();
    client
        .auth("digest".into(), auth)
        .await
        .map_err(|e| zk_err("认证 ", e))?;
    Ok(())
}
