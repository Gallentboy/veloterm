//! 联动组内 ZMODEM 请求聚合：一次交互，N 路扇出。
//!
//! 场景：联动（broadcast）开启时用户在一台终端敲 `sz file`，字节被镜像到
//! 同组全部终端 → 每台服务器都启动 sz，各自回 ZRQINIT；或敲 `rz`，每台
//! 回 `rz\r` 公告。若不聚合，会弹出 N 个对话框。
//!
//! 聚合语义：
//! - 以「联动组」为键收集命中（`register`）；
//! - **凑齐组内全部成员即立即触发**（最佳体验），另有 `WINDOW` 兜底
//!   （成员响应慢/掉线时不至于卡住）；
//! - 触发后只弹一次对话框（sz 选目录 / rz 选文件），结果扇出给每个成员
//!   各自的引擎。
//!
//! 组信息由前端在联动集合变化时经 `terminal_set_broadcast_group` 同步。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

/// 聚合窗口兜底：首命中后最多等这么久（凑齐成员则提前触发）
pub const AGGREGATE_WINDOW: Duration = Duration::from_millis(500);

/// 一次命中登记
pub struct PendingHit {
    /// 该会话的入向字节接收端（引擎用）
    pub inbound_rx: super::bridge::InboundRx,
    /// 该会话的桥（喂数据/取消）
    pub bridge: super::bridge::ZmodemBridge,
    /// 引擎写回通道
    pub outbound_tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

/// 聚合结果：同组的一次请求
pub struct Batch {
    pub direction: &'static str,
    pub hits: Vec<PendingHit>,
}

struct GroupState {
    /// 当前批的登记（session_id → 命中）
    hits: HashMap<usize, PendingHit>,
    /// 首命中时间（窗口起点）
    started: Option<Instant>,
    /// 已登记方向（sz/rz 混在同一批时以先到者为准）
    direction: &'static str,
    /// 触发回调（凑齐或窗口到期时调用一次）
    fire_tx: Option<oneshot::Sender<()>>,
}

/// 全局聚合器（挂 App 状态；按联动组 id 分桶）
#[derive(Default)]
pub struct ZmodemAggregator {
    groups: Arc<Mutex<HashMap<String, GroupState>>>,
    /// 联动组映射：session_id → group_key（None = 未联动，立即触发）
    membership: Arc<Mutex<HashMap<usize, String>>>,
}

impl ZmodemAggregator {
    pub fn new() -> Self {
        Self::default()
    }

    /// 前端同步联动集合（空 = 全部解绑）
    pub fn set_group(&self, group_key: &str, session_ids: &[usize]) {
        let mut m = self.membership.lock().expect("membership poisoned");
        // 先清除旧归属（同一会话可能从 A 组换到 B 组）
        for sid in session_ids {
            m.insert(*sid, group_key.to_string());
        }
        // 移除已不在任何组里的会话由前端显式传空组处理（group_key = ""）
        if group_key.is_empty() {
            m.retain(|_, g| !g.is_empty());
        }
    }

    /// 清除某会话归属（联动关闭）
    pub fn clear_session(&self, session_id: usize) {
        self.membership
            .lock()
            .expect("membership poisoned")
            .remove(&session_id);
    }

    /// 组内成员数（凑齐判定用）
    fn group_size(&self, group_key: &str) -> usize {
        self.membership
            .lock()
            .expect("membership poisoned")
            .values()
            .filter(|g| *g == group_key)
            .count()
    }

    /// 该会话所属组键（未联动返回 None → 调用方立即触发）
    pub fn group_of(&self, session_id: usize) -> Option<String> {
        self.membership
            .lock()
            .expect("membership poisoned")
            .get(&session_id)
            .cloned()
    }

    /// 登记一次命中。返回 `Some(batch)` 表示本批应当立即触发（凑齐/单台）；
    /// `None` 表示已并入等待中的批，由窗口定时器或后续命中触发。
    pub fn register(
        &self,
        session_id: usize,
        direction: &'static str,
        hit: PendingHit,
    ) -> Option<Batch> {
        let Some(group) = self.group_of(session_id) else {
            // 未联动：立即单台触发
            return Some(Batch { direction, hits: vec![hit] });
        };
        let size = self.group_size(&group);
        let mut groups = self.groups.lock().expect("aggregator poisoned");
        let state = groups.entry(group.clone()).or_insert_with(|| GroupState {
            hits: HashMap::new(),
            started: None,
            direction,
            fire_tx: None,
        });
        if state.started.is_none() {
            state.started = Some(Instant::now());
            state.direction = direction;
        }
        state.hits.insert(session_id, hit);
        // 凑齐全部成员（或只剩一个成员）→ 立即触发
        if state.hits.len() >= size.max(1) {
            let hits: Vec<PendingHit> = state.hits.drain().map(|(_, h)| h).collect();
            let dir = state.direction;
            state.started = None;
            groups.remove(&group);
            return Some(Batch { direction: dir, hits });
        }
        None
    }

    /// 取出某组当前已登记但未凑齐的批（窗口到期时调用）
    pub fn take_batch(&self, group_key: &str) -> Option<Batch> {
        let mut groups = self.groups.lock().expect("aggregator poisoned");
        let state = groups.get_mut(group_key)?;
        if state.hits.is_empty() {
            groups.remove(group_key);
            return None;
        }
        let hits: Vec<PendingHit> = state.hits.drain().map(|(_, h)| h).collect();
        let direction = state.direction;
        groups.remove(group_key);
        Some(Batch { direction, hits })
    }

    /// 清空全部（会话关闭/退出时）
    pub fn clear_all(&self) {
        self.groups.lock().expect("aggregator poisoned").clear();
        self.membership
            .lock()
            .expect("membership poisoned")
            .clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_hit() -> PendingHit {
        let (bridge, inbound_rx) = super::super::bridge::ZmodemBridge::new("t".into(), "download");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        PendingHit { inbound_rx, bridge, outbound_tx: tx }
    }

    #[test]
    fn ungrouped_session_fires_immediately() {
        let agg = ZmodemAggregator::new();
        let batch = agg.register(1, "download", dummy_hit());
        assert!(batch.is_some());
        assert_eq!(batch.unwrap().hits.len(), 1);
    }

    #[test]
    fn group_waits_for_all_members_then_fires() {
        let agg = ZmodemAggregator::new();
        agg.set_group("g1", &[1, 2, 3]);
        assert!(agg.register(1, "download", dummy_hit()).is_none());
        assert!(agg.register(2, "download", dummy_hit()).is_none());
        // 第三个凑齐 → 立即触发
        let batch = agg.register(3, "download", dummy_hit()).expect("should fire");
        assert_eq!(batch.hits.len(), 3);
    }

    #[test]
    fn window_fallback_takes_partial_batch() {
        let agg = ZmodemAggregator::new();
        agg.set_group("g1", &[1, 2, 3]);
        assert!(agg.register(1, "upload", dummy_hit()).is_none());
        let batch = agg.take_batch("g1").expect("window should take partial");
        assert_eq!(batch.hits.len(), 1);
        assert_eq!(batch.direction, "upload");
        // 取出后组内清空，可重新聚合
        assert!(agg.take_batch("g1").is_none());
    }

    #[test]
    fn membership_switch_clears_old_group() {
        let agg = ZmodemAggregator::new();
        agg.set_group("g1", &[1, 2]);
        assert_eq!(agg.group_of(1).as_deref(), Some("g1"));
        agg.clear_session(1);
        assert!(agg.group_of(1).is_none());
        assert_eq!(agg.group_size("g1"), 1);
    }
}
