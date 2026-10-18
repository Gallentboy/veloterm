//! SSH 端口转发（本地 -L / 远程 -R / 动态 SOCKS5 -D，参考 rssh 的实现形态）。
//!
//! # 架构
//!
//! - **规则按主机维度**持久化在 `port_forwards` 表（与终端会话解耦：终端
//!   关闭不影响转发），每台主机至多一条**转发专用 SSH 连接**（独立于终端
//!   会话的连接，经 `connect_ssh_chain` 直连 russh `Handle`，含跳板链）。
//! - 数据面三种规则（rssh 同型）：
//!   - **Local（-L）**：本地 `TcpListener` accept → `channel_open_direct_tcpip`
//!     到目标 → 双向桥（`copy_bidirectional`）；
//!   - **Remote（-R）**：`tcpip_forward` 请求服务器监听 → 服务器入站
//!     `ForwardedTcpIp` 通道（按 connected_port 匹配规则）→ 反连本地目标 → 桥；
//!   - **Dynamic（-D）**：本地 `TcpListener` accept → **fast-socks5 库**完成
//!     SOCKS5 服务端握手（无认证、仅 CONNECT，IPv4/域名/IPv6 目标）→ 取目标
//!     地址开 direct-tcpip → 桥。
//! - 连接断开自动重连（5s 间隔，规则全部停用时连接退出）；状态经
//!   `port-forward-state` 事件推给前端。
//!
//! # 为什么用 fast-socks5 而不是手写握手
//!
//! SOCKS5 服务端握手是协议样板（方法协商/CONNECT/三种 ATYP），库（1.0，
//! MIT，tokio 1）的 typestate API 恰好是「握手 → 交出目标地址 → 归还原始
//! socket 由调用方接管」，与 direct-tcpip 桥接零耦合，省掉 ~100 行手写
//! 解析与回包（rssh 是手写的，这里换成库）。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use russh::client::{Handle, Msg};
use russh::Channel;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::db::{self, DbPool};
use crate::terminal::ssh::handler::{ClientHandler, ClientHandlerEvent};
use crate::terminal::ssh::{ForwardedTcpIpParams, KnownHosts, TargetSSHOptions, connect_ssh_chain};

/// 共享连接句柄（russh 0.63 的 Handle 非 Clone，Arc 共享给每规则任务）
type SharedHandle = Arc<Handle<ClientHandler>>;

/// 状态事件名（payload = [`ForwardStateMsg`]，规则状态变化时广播）
pub const FORWARD_STATE_EVENT: &str = "port-forward-state";
/// 断线重连间隔
const RECONNECT_DELAY_SECS: u64 = 5;
/// 连接建立超时
const CONNECT_TIMEOUT_SECS: u64 = 20;

// ───────────────────────────── 模型 ─────────────────────────────

/// 转发类型：本地（-L）/ 远程（-R）/ 动态 SOCKS5（-D）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortForwardKind {
    Local,
    Remote,
    Dynamic,
}

/// 一条转发规则（db 记录形态，serde camelCase 直达前端）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortForwardRule {
    pub id: i64,
    pub host_id: i64,
    pub kind: PortForwardKind,
    pub bind_host: String,
    pub bind_port: u16,
    /// Dynamic 规则不使用目标（目标由 SOCKS5 客户端逐连接指定），可留空
    pub target_host: String,
    pub target_port: u16,
    pub enabled: bool,
}

/// 规则运行态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleStatus {
    Stopped,
    Starting,
    Active,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleRuntime {
    pub rule_id: i64,
    pub status: RuleStatus,
    pub last_error: Option<String>,
    /// Remote 规则服务器实际分配的端口（bind_port=0 时由服务器分配）
    pub effective_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardStateMsg {
    pub host_id: i64,
    pub rules: Vec<RuleRuntime>,
}

// ───────────────────────────── db CRUD ─────────────────────────────

fn row_to_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<PortForwardRule> {
    let kind_str: String = row.get("kind")?;
    let kind = match kind_str.as_str() {
        "local" => PortForwardKind::Local,
        "remote" => PortForwardKind::Remote,
        _ => PortForwardKind::Dynamic,
    };
    Ok(PortForwardRule {
        id: row.get("id")?,
        host_id: row.get("host_id")?,
        kind,
        bind_host: row.get("bind_host")?,
        bind_port: row.get::<_, i64>("bind_port")? as u16,
        target_host: row.get("target_host")?,
        target_port: row.get::<_, i64>("target_port")? as u16,
        enabled: row.get::<_, i64>("enabled")? != 0,
    })
}

pub fn db_list_by_host(pool: &DbPool, host_id: i64) -> Result<Vec<PortForwardRule>, String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    let mut stmt = conn
        .prepare("SELECT * FROM port_forwards WHERE host_id = ?1 ORDER BY id")
        .map_err(|e| format!("failed to list port forwards: {e}"))?;
    let rules = stmt
        .query_map([host_id], row_to_rule)
        .map_err(|e| format!("failed to query port forwards: {e}"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| format!("failed to read port forward rows: {e}"))?;
    Ok(rules)
}

/// 新建（id<=0）或更新规则，返回落库后的完整规则
pub fn db_upsert(
    pool: &DbPool,
    host_id: i64,
    rule: &PortForwardRule,
) -> Result<PortForwardRule, String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    let now = db::now_timestamp_ms();
    if rule.id > 0 {
        conn.execute(
            "UPDATE port_forwards SET kind = ?1, bind_host = ?2, bind_port = ?3,
             target_host = ?4, target_port = ?5, enabled = ?6, updated_at = ?7
             WHERE id = ?8 AND host_id = ?9",
            rusqlite::params![
                match rule.kind {
                    PortForwardKind::Local => "local",
                    PortForwardKind::Remote => "remote",
                    PortForwardKind::Dynamic => "dynamic",
                },
                rule.bind_host,
                rule.bind_port,
                rule.target_host,
                rule.target_port,
                rule.enabled as i64,
                now,
                rule.id,
                host_id,
            ],
        )
        .map_err(|e| format!("failed to update port forward: {e}"))?;
        return Ok(rule.clone());
    }
    conn.execute(
        "INSERT INTO port_forwards
           (host_id, kind, bind_host, bind_port, target_host, target_port, enabled, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        rusqlite::params![
            host_id,
            match rule.kind {
                PortForwardKind::Local => "local",
                PortForwardKind::Remote => "remote",
                PortForwardKind::Dynamic => "dynamic",
            },
            rule.bind_host,
            rule.bind_port,
            rule.target_host,
            rule.target_port,
            rule.enabled as i64,
            now,
        ],
    )
    .map_err(|e| format!("failed to insert port forward: {e}"))?;
    let id = conn.last_insert_rowid();
    Ok(PortForwardRule { id, ..rule.clone() })
}

pub fn db_delete(pool: &DbPool, host_id: i64, rule_id: i64) -> Result<(), String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    conn.execute(
        "DELETE FROM port_forwards WHERE id = ?1 AND host_id = ?2",
        rusqlite::params![rule_id, host_id],
    )
    .map_err(|e| format!("failed to delete port forward: {e}"))?;
    Ok(())
}

pub fn db_set_enabled(
    pool: &DbPool,
    host_id: i64,
    rule_id: i64,
    enabled: bool,
) -> Result<(), String> {
    let conn = pool
        .get()
        .map_err(|e| format!("failed to get db connection: {e}"))?;
    conn.execute(
        "UPDATE port_forwards SET enabled = ?1, updated_at = ?2 WHERE id = ?3 AND host_id = ?4",
        rusqlite::params![
            enabled as i64,
            db::now_timestamp_ms(),
            rule_id,
            host_id
        ],
    )
    .map_err(|e| format!("failed to toggle port forward: {e}"))?;
    Ok(())
}

// ─────────────────────── 跳板链解析（与 TerminalManager 同型） ───────────────────────

/// 把主机记录解析为 SSH 连接链（目标在末位，跳板按 jump_host_id 链式前插）
fn resolve_host_chain(pool: &DbPool, host_id: i64) -> Result<Vec<TargetSSHOptions>, String> {
    let store = db::load_host_store(pool)?;
    let target = store
        .hosts
        .iter()
        .find(|h| h.id as i64 == host_id)
        .ok_or_else(|| format!("host {host_id} not found"))?;
    let by_id: HashMap<usize, &crate::db::HostRecord> =
        store.hosts.iter().map(|h| (h.id, h)).collect();

    let mut chain = Vec::new();
    let mut visited = std::collections::HashSet::from([target.id]);
    let mut cursor = target.jump_host_id;
    while let Some(jid) = cursor {
        if !visited.insert(jid) {
            break;
        }
        let Some(hop) = by_id.get(&jid) else {
            break;
        };
        chain.insert(
            0,
            TargetSSHOptions {
                host: hop.host.clone(),
                port: hop.port,
                username: hop.username.clone(),
                allow_insecure_algos: hop.allow_insecure_algos,
                auth: hop.auth.clone(),
                jump_host: None,
                host_key_verification: hop.host_key_verification,
                relay: None,
            },
        );
        cursor = hop.jump_host_id;
    }
    chain.push(TargetSSHOptions {
        host: target.host.clone(),
        port: target.port,
        username: target.username.clone(),
        allow_insecure_algos: target.allow_insecure_algos,
        auth: target.auth.clone(),
        jump_host: None,
        host_key_verification: target.host_key_verification,
        relay: None,
    });
    // 中继（仅链首生效）：与 TerminalManager 同型（嵌入式优先，relay_id 兜底），
    // 端口转发连接同样可走中继
    let relay = target
        .relay
        .clone()
        .or_else(|| crate::relays::endpoint_by_id(target.relay_id));
    if let Some(relay) = relay
        && let Some(first) = chain.first_mut()
    {
        first.relay = Some(relay);
    }
    Ok(chain)
}

// ─────────────────────── 管理器 ───────────────────────

enum Control {
    /// 全量规则集（reconcile 语义：与当前运行集做差集启停）
    Apply(Vec<PortForwardRule>),
    Shutdown,
}

struct HostRuntime {
    control: UnboundedSender<Control>,
    /// supervisor 任务句柄（reconcile 移除时 abort）
    task: JoinHandle<()>,
    /// 运行态（命令查询 + 事件广播共用）
    states: Arc<Mutex<HashMap<i64, RuleRuntime>>>,
}

/// 端口转发管理器（tauri State）：每主机一条转发专用连接的 supervisor
#[derive(Default)]
pub struct PortForwardManager {
    hosts: AsyncMutex<HashMap<i64, HostRuntime>>,
}

impl PortForwardManager {
    /// 规则集变化后收敛运行态：无启用规则 → 拆连接；否则确保 supervisor 在跑
    /// 并下发最新规则集（Apply 全量差集）
    pub async fn reconcile(&self, app: &AppHandle, host_id: i64) {
        let pool = match db::global_pool() {
            Some(pool) => pool,
            None => return,
        };
        let rules = db_list_by_host(&pool, host_id).unwrap_or_default();
        let has_enabled = rules.iter().any(|r| r.enabled);

        let mut hosts = self.hosts.lock().await;
        if !has_enabled {
            if let Some(runtime) = hosts.remove(&host_id) {
                let _ = runtime.control.send(Control::Shutdown);
                runtime.task.abort();
            }
            return;
        }
        if let Some(runtime) = hosts.get(&host_id) {
            let _ = runtime.control.send(Control::Apply(rules));
            return;
        }

        // 起 supervisor：初始状态全 Starting（连接建立后逐规则转 Active/Error）
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let states = Arc::new(Mutex::new(
            rules
                .iter()
                .map(|r| {
                    (
                        r.id,
                        RuleRuntime {
                            rule_id: r.id,
                            status: if r.enabled {
                                RuleStatus::Starting
                            } else {
                                RuleStatus::Stopped
                            },
                            last_error: None,
                            effective_port: None,
                        },
                    )
                })
                .collect(),
        ));
        let task = tokio::spawn(run_host_supervisor(
            app.clone(),
            host_id,
            control_rx,
            Arc::clone(&states),
            rules,
        ));
        hosts.insert(
            host_id,
            HostRuntime {
                control: control_tx,
                task,
                states,
            },
        );
    }

    pub fn states_of(&self, host_id: i64) -> Vec<RuleRuntime> {
        // 只读快照：.try_lock 失败（正在收敛）时返回空，由前端经事件再刷
        match self.hosts.try_lock() {
            Ok(hosts) => hosts
                .get(&host_id)
                .map(|rt| rt.states.lock().unwrap().values().cloned().collect())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }
}

/// 广播某主机全部规则状态
fn emit_state(app: &AppHandle, host_id: i64, states: &Mutex<HashMap<i64, RuleRuntime>>) {
    let rules: Vec<RuleRuntime> = states.lock().unwrap().values().cloned().collect();
    let _ = app.emit(
        FORWARD_STATE_EVENT,
        ForwardStateMsg {
            host_id,
            rules,
        },
    );
}

fn set_rule_state(
    states: &Mutex<HashMap<i64, RuleRuntime>>,
    rule_id: i64,
    status: RuleStatus,
    error: Option<String>,
) {
    states.lock().unwrap().entry(rule_id).and_modify(|rt| {
        rt.status = status;
        rt.last_error = error;
        if status != RuleStatus::Active {
            rt.effective_port = None;
        }
    });
}

/// 一条规则的运行任务句柄（停用/删除时终止）
struct RuleTask {
    stop: Option<oneshot::Sender<()>>,
    abort: tokio::task::AbortHandle,
}

impl RuleTask {
    fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.abort.abort();
    }
}

/// 主机转发 supervisor：连接 → 按规则启停 → 断线重连，直到规则全部停用
async fn run_host_supervisor(
    app: AppHandle,
    host_id: i64,
    mut control_rx: UnboundedReceiver<Control>,
    states: Arc<Mutex<HashMap<i64, RuleRuntime>>>,
    initial_rules: Vec<PortForwardRule>,
) {
    let pool = db::global_pool();
    let mut rules = initial_rules;
    'outer: loop {
        if !rules.iter().any(|r| r.enabled) {
            break;
        }
        for rule in rules.iter().filter(|r| r.enabled) {
            set_rule_state(&states, rule.id, RuleStatus::Starting, None);
        }
        emit_state(&app, host_id, &states);

        let chain = pool
            .as_ref()
            .and_then(|p| resolve_host_chain(p, host_id).ok())
            .unwrap_or_default();
        if chain.is_empty() {
            let msg = "无法解析主机连接信息（主机可能已删除）".to_string();
            for rule in rules.iter().filter(|r| r.enabled) {
                set_rule_state(&states, rule.id, RuleStatus::Error, Some(msg.clone()));
            }
            emit_state(&app, host_id, &states);
            break;
        }

        let connected = tokio::time::timeout(
            std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS),
            connect_ssh_chain(host_id as usize, chain, KnownHosts::global()),
        )
        .await;

        match connected {
            Ok(Ok(connection)) => {
                let (handle, mut event_rx): (SharedHandle, _) = (Arc::new(connection.0), connection.1);
                let mut rule_tasks: HashMap<i64, RuleTask> = HashMap::new();
                start_enabled_rules(&app, host_id, &handle, &states, &rules, &mut rule_tasks).await;

                loop {
                    tokio::select! {
                        control = control_rx.recv() => match control {
                            Some(Control::Shutdown) | None => {
                                stop_all_rules(&handle, &states, &mut rule_tasks, &rules).await;
                                let _ = handle
                                    .disconnect(russh::Disconnect::ByApplication, "port forward stopped", "en")
                                    .await;
                                break 'outer;
                            }
                            Some(Control::Apply(next)) => {
                                stop_all_rules(&handle, &states, &mut rule_tasks, &rules).await;
                                rules = next;
                                if !rules.iter().any(|r| r.enabled) {
                                    let _ = handle
                                        .disconnect(russh::Disconnect::ByApplication, "port forward stopped", "en")
                                        .await;
                                    break 'outer;
                                }
                                for rule in rules.iter().filter(|r| !r.enabled) {
                                    set_rule_state(&states, rule.id, RuleStatus::Stopped, None);
                                }
                                start_enabled_rules(&app, host_id, &handle, &states, &rules, &mut rule_tasks).await;
                            }
                        },
                        event = event_rx.recv() => match event {
                            Some(ClientHandlerEvent::ForwardedTcpIp(channel, params)) => {
                                handle_forwarded_inbound(&states, &rules, channel, params).await;
                            }
                            Some(ClientHandlerEvent::Disconnect) | None => {
                                let msg = "SSH 连接已断开，5 秒后重连".to_string();
                                for (rule_id, task) in rule_tasks.drain() {
                                    task.stop();
                                    set_rule_state(&states, rule_id, RuleStatus::Starting, Some(msg.clone()));
                                }
                                emit_state(&app, host_id, &states);
                                break; // 内层 break → 走重连等待
                            }
                            _ => {}
                        },
                    }
                }
            }
            Ok(Err(error)) => {
                let msg = format!("SSH 连接失败: {error}");
                for rule in rules.iter().filter(|r| r.enabled) {
                    set_rule_state(&states, rule.id, RuleStatus::Error, Some(msg.clone()));
                }
                emit_state(&app, host_id, &states);
            }
            Err(_) => {
                let msg = format!("SSH 连接超时（>{CONNECT_TIMEOUT_SECS}s），重试中");
                for rule in rules.iter().filter(|r| r.enabled) {
                    set_rule_state(&states, rule.id, RuleStatus::Error, Some(msg.clone()));
                }
                emit_state(&app, host_id, &states);
            }
        }

        // 重连等待（期间仍响应控制消息：Shutdown / 新规则集）
        loop {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(RECONNECT_DELAY_SECS)) => break,
                control = control_rx.recv() => match control {
                    Some(Control::Shutdown) | None => break 'outer,
                    Some(Control::Apply(next)) => {
                        rules = next;
                        if !rules.iter().any(|r| r.enabled) {
                            break 'outer;
                        }
                    }
                },
            }
        }
    }

    // 收尾：全部置 Stopped
    let mut snapshot = states.lock().unwrap();
    for rt in snapshot.values_mut() {
        rt.status = RuleStatus::Stopped;
        rt.last_error = None;
        rt.effective_port = None;
    }
    drop(snapshot);
    emit_state(&app, host_id, &states);
}

/// 连接建立后启动全部启用规则（local/dynamic = 监听任务；remote = tcpip_forward）
async fn start_enabled_rules(
    app: &AppHandle,
    host_id: i64,
    handle: &SharedHandle,
    states: &Arc<Mutex<HashMap<i64, RuleRuntime>>>,
    rules: &[PortForwardRule],
    rule_tasks: &mut HashMap<i64, RuleTask>,
) {
    for rule in rules.iter().filter(|r| r.enabled) {
        match rule.kind {
            PortForwardKind::Remote => {
                match handle
                    .tcpip_forward(rule.bind_host.clone(), u32::from(rule.bind_port))
                    .await
                {
                    Ok(assigned) => {
                        states.lock().unwrap().entry(rule.id).and_modify(|rt| {
                            rt.status = RuleStatus::Active;
                            rt.last_error = None;
                            // bind_port=0 时服务器分配端口
                            rt.effective_port = Some(assigned as u16);
                        });
                    }
                    Err(error) => {
                        set_rule_state(
                            states,
                            rule.id,
                            RuleStatus::Error,
                            Some(format!("远端监听建立失败: {error}")),
                        );
                    }
                }
            }
            PortForwardKind::Local | PortForwardKind::Dynamic => {
                let (stop_tx, stop_rx) = oneshot::channel();
                let task = tokio::spawn(run_listener_rule(
                    rule.clone(),
                    Arc::clone(handle),
                    Arc::clone(states),
                    stop_rx,
                ));
                rule_tasks.insert(
                    rule.id,
                    RuleTask {
                        stop: Some(stop_tx),
                        abort: task.abort_handle(),
                    },
                );
            }
        }
    }
    emit_state(app, host_id, states);
}

/// 停止全部规则任务（连接级断开前的清理）
async fn stop_all_rules(
    handle: &SharedHandle,
    states: &Arc<Mutex<HashMap<i64, RuleRuntime>>>,
    rule_tasks: &mut HashMap<i64, RuleTask>,
    rules: &[PortForwardRule],
) {
    for (_, task) in rule_tasks.drain() {
        task.stop();
    }
    for rule in rules.iter().filter(|r| r.kind == PortForwardKind::Remote && r.enabled) {
        let _ = handle
            .cancel_tcpip_forward(rule.bind_host.clone(), u32::from(rule.bind_port))
            .await;
        set_rule_state(states, rule.id, RuleStatus::Stopped, None);
    }
    for task_id in states.lock().unwrap().keys().copied().collect::<Vec<_>>() {
        set_rule_state(states, task_id, RuleStatus::Stopped, None);
    }
}

/// Local / Dynamic 规则的监听任务：accept 循环 + 每连接桥接
async fn run_listener_rule(
    rule: PortForwardRule,
    handle: SharedHandle,
    states: Arc<Mutex<HashMap<i64, RuleRuntime>>>,
    mut stop: oneshot::Receiver<()>,
) {
    // 双栈监听：bind_host 逐字绑定（IPv4/IPv6/0.0.0.0），失败置 Error
    let bind_addr: SocketAddr = match format!("{}:{}", rule.bind_host, rule.bind_port).parse() {
        Ok(addr) => addr,
        Err(_) => {
            set_rule_state(
                &states,
                rule.id,
                RuleStatus::Error,
                Some(format!("监听地址无效: {}", rule.bind_host)),
            );
            return;
        }
    };
    let listener = match TcpListener::bind(bind_addr).await {
        Ok(listener) => listener,
        Err(error) => {
            set_rule_state(
                &states,
                rule.id,
                RuleStatus::Error,
                Some(format!("本地监听建立失败: {error}")),
            );
            return;
        }
    };
    states.lock().unwrap().entry(rule.id).and_modify(|rt| {
        rt.status = RuleStatus::Active;
        rt.last_error = None;
        rt.effective_port = Some(rule.bind_port);
    });

    let mut conns = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _ = &mut stop => break,
            accepted = listener.accept() => {
                let Ok((socket, _peer)) = accepted else { break };
                let handle = Arc::clone(&handle);
                let rule = rule.clone();
                conns.spawn(async move {
                    handle_listener_connection(rule, handle, socket).await;
                });
            }
            _ = conns.join_next(), if !conns.is_empty() => {}
        }
    }
    conns.abort_all();
    while conns.join_next().await.is_some() {}
}

/// 单个入站连接：Local 直连目标；Dynamic 先做 SOCKS5 握手解析目标
async fn handle_listener_connection(
    rule: PortForwardRule,
    handle: SharedHandle,
    mut socket: TcpStream,
) {
    let (target_host, target_port) = match rule.kind {
        PortForwardKind::Local => (rule.target_host.clone(), rule.target_port),
        PortForwardKind::Dynamic => {
            // fast-socks5 服务端握手：无认证协商 + 读 CONNECT 命令 + 回成功后
            // 归还原始 socket（库只做协议，转发由本函数接管）
            match socks5_accept(&mut socket).await {
                Ok(Some(target)) => target,
                _ => return,
            }
        }
        PortForwardKind::Remote => return,
    };

    let channel = match handle
        .channel_open_direct_tcpip(target_host, u32::from(target_port), "127.0.0.1", 0)
        .await
    {
        Ok(channel) => channel,
        Err(_) => return,
    };
    let _ = tokio::io::copy_bidirectional(&mut socket, &mut channel.into_stream()).await;
}

/// SOCKS5 服务端握手（fast-socks5）：完成无认证协商、读取 CONNECT 命令，
/// 校验目标后回 SUCCESS 并返回 (host, port)；调用方继续用原始 socket 桥接。
/// 非 CONNECT 命令回 CommandNotSupported 后失败
/// SOCKS5 服务端握手（fast-socks5）：完成无认证协商、读取 CONNECT 命令，
/// 校验目标后回成功并返回 (host, port)；调用方继续用原始 socket 桥接。
/// 非 CONNECT 命令回 CommandNotSupported 后返回 None（连接由调用方丢弃）
async fn socks5_accept(socket: &mut TcpStream) -> Result<Option<(String, u16)>, fast_socks5::SocksError> {
    use fast_socks5::server::Socks5ServerProtocol;
    use fast_socks5::Socks5Command;

    let protocol = Socks5ServerProtocol::accept_no_auth(socket).await?;
    let (protocol, command, target_addr) = protocol.read_command().await?;
    if command != Socks5Command::TCPConnect {
        // BIND/UDP ASSOCIATE 不支持：按协议回 CommandNotSupported 后失败
        protocol
            .reply_error(&fast_socks5::ReplyError::CommandNotSupported)
            .await?;
        return Ok(None);
    }
    // 目标拿到即回成功（对客户端表现为已连上）；direct-tcpip 打不开时由 TCP
    // 层断开，与常见代理的失败表现一致
    let _socket = protocol
        .reply_success(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await?;
    Ok(Some(target_addr.into_string_and_port()))
}

/// Remote 规则的服务器入站通道：匹配规则 → 反连本地目标 → 桥接
async fn handle_forwarded_inbound(
    states: &Arc<Mutex<HashMap<i64, RuleRuntime>>>,
    rules: &[PortForwardRule],
    channel: Channel<Msg>,
    params: ForwardedTcpIpParams,
) {
    // 按 connected_port（服务器侧实际监听端口，含 0 申请时的分配值）匹配规则
    let Some(rule) = rules.iter().find(|r| {
        r.enabled
            && r.kind == PortForwardKind::Remote
            && params.connected_port == effective_remote_bind_port(r, states)
    }) else {
        let _ = channel.close().await;
        return;
    };
    let Ok(mut local) = TcpStream::connect((rule.target_host.as_str(), rule.target_port)).await
    else {
        let _ = channel.close().await;
        return;
    };
    let _ = tokio::io::copy_bidirectional(&mut local, &mut channel.into_stream()).await;
}

/// remote 规则的服务器侧实际监听端口：优先 tcpip_forward 返回的分配值，
/// 取不到（如刚重启未登记）回退规则里的 bind_port
fn effective_remote_bind_port(
    rule: &PortForwardRule,
    states: &Arc<Mutex<HashMap<i64, RuleRuntime>>>,
) -> u32 {
    states
        .lock()
        .unwrap()
        .get(&rule.id)
        .and_then(|rt| rt.effective_port)
        .map(u32::from)
        .unwrap_or_else(|| u32::from(rule.bind_port))
}

// ─────────────────────── tauri 命令 ───────────────────────

/// 列出某主机的全部转发规则（运行态另经 port_forward_states / 事件）
#[tauri::command]
pub async fn port_forward_list(host_id: i64) -> Result<Vec<PortForwardRule>, String> {
    let pool = db::global_pool().ok_or("数据库未初始化")?;
    db_list_by_host(&pool, host_id)
}

/// 保存（新建 id<=0 / 更新）规则并按需收敛连接
#[tauri::command]
pub async fn port_forward_save(
    app: AppHandle,
    manager: State<'_, PortForwardManager>,
    rule: PortForwardRule,
) -> Result<PortForwardRule, String> {
    if rule.bind_port == 0 && rule.kind != PortForwardKind::Remote {
        return Err("监听端口不能为 0".to_string());
    }
    if rule.kind == PortForwardKind::Local && (rule.target_host.is_empty() || rule.target_port == 0)
    {
        return Err("本地转发必须指定目标主机与端口".to_string());
    }
    let pool = db::global_pool().ok_or("数据库未初始化")?;
    let saved = db_upsert(&pool, rule.host_id, &rule)?;
    manager.reconcile(&app, rule.host_id).await;
    Ok(saved)
}

#[tauri::command]
pub async fn port_forward_delete(
    app: AppHandle,
    manager: State<'_, PortForwardManager>,
    host_id: i64,
    rule_id: i64,
) -> Result<(), String> {
    let pool = db::global_pool().ok_or("数据库未初始化")?;
    db_delete(&pool, host_id, rule_id)?;
    manager.reconcile(&app, host_id).await;
    Ok(())
}

#[tauri::command]
pub async fn port_forward_set_enabled(
    app: AppHandle,
    manager: State<'_, PortForwardManager>,
    host_id: i64,
    rule_id: i64,
    enabled: bool,
) -> Result<(), String> {
    let pool = db::global_pool().ok_or("数据库未初始化")?;
    db_set_enabled(&pool, host_id, rule_id, enabled)?;
    manager.reconcile(&app, host_id).await;
    Ok(())
}

/// 前端查询某主机全部规则的运行态（事件之外的主动拉取口径）
#[tauri::command]
pub fn port_forward_states(
    manager: State<'_, PortForwardManager>,
    host_id: i64,
) -> Vec<RuleRuntime> {
    manager.states_of(host_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// SOCKS5 握手端到端（回环 TCP + 手写客户端字节流）：域名目标解析 +
    /// 无认证协商 + 成功回包；这是 fast-socks5 接入面的核心契约
    #[tokio::test]
    async fn socks5_accept_resolves_domain_target() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socks5_accept(&mut socket).await
        });
        let mut client = TcpStream::connect(addr).await.unwrap();
        // greeting：ver5 / 1 个方法 / no-auth
        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut method = [0u8; 2];
        client.read_exact(&mut method).await.unwrap();
        assert_eq!(method, [0x05, 0x00]);
        // CONNECT：域名 example.com:80（ATYP=0x03）
        client.write_all(&[0x05, 0x01, 0x00, 0x03, 0x0b]).await.unwrap();
        client.write_all(b"example.com").await.unwrap();
        client.write_all(&80u16.to_be_bytes()).await.unwrap();
        // 成功回包：ver + rep(0x00) + rsv + ATYP(0x01) + bnd.addr(4) + bnd.port(2)
        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[0], 0x05);
        assert_eq!(reply[1], 0x00, "应答成功");
        let target = server.await.unwrap().unwrap().unwrap();
        assert_eq!(target, ("example.com".to_string(), 80));
    }

    /// IPv4 目标形态（ATYP=0x01）
    #[tokio::test]
    async fn socks5_accept_resolves_ipv4_target() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socks5_accept(&mut socket).await
        });
        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut method = [0u8; 2];
        client.read_exact(&mut method).await.unwrap();
        client.write_all(&[0x05, 0x01, 0x00, 0x01, 10, 0, 0, 1]).await.unwrap();
        client.write_all(&443u16.to_be_bytes()).await.unwrap();
        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[1], 0x00);
        let target = server.await.unwrap().unwrap().unwrap();
        assert_eq!(target, ("10.0.0.1".to_string(), 443));
    }

    /// 规则模型 serde 形态（前端契约锁定：camelCase + lowercase kind）
    #[test]
    fn rule_serializes_camel_case() {
        let rule = PortForwardRule {
            id: 1,
            host_id: 2,
            kind: PortForwardKind::Dynamic,
            bind_host: "127.0.0.1".into(),
            bind_port: 1080,
            target_host: String::new(),
            target_port: 0,
            enabled: true,
        };
        let json = serde_json::to_string(&rule).unwrap();
        assert!(json.contains("\"hostId\":2"));
        assert!(json.contains("\"bindPort\":1080"));
        assert!(json.contains("\"kind\":\"dynamic\""));
    }
}
