//! 命令历史被动采集：OSC 7777 私有协议的扫描 / 剥离 / 入库（纯后端，不经过前端）。
//!
//! 协议（单发自包含，见设计讨论）：`ESC ] 7777 ; cmd ; <exit_code> ; <percent-encoded-command> ST`
//! - 载荷 7-bit ASCII：`;` / ESC / BEL / 非 ASCII 字节全部 `%XX` 编码（发射侧负责编码）
//! - 终止符认 ST（`ESC \`）与 BEL 两种，均剥离不显示
//! - 未知子命令：同样剥离但不入库（向前兼容——新版片段发的新子命令旧客户端静默吞掉）
//! - 用户 shell 自己发的其他 OSC（0/2 改标题、7 cwd、52 剪贴板……）**原样透传**给前端
//!   xterm，只在确认是 `7777;` 前缀后才截获
//!
//! 注入完全由用户自主（服务器 `~/.bashrc` / `~/.zshrc` / fish config，或本应用的就绪钩子），
//! 没有注入的环境不产生序列，本模块零动作、零报错。
//!
//! 入库路径：扫描线程 `try_send` 有界队列（满即丢弃计数，绝不反压 PTY/SSH 读循环）→
//! 专职写线程批量事务 INSERT（摊薄 fsync）。队列与线程均为应用级单例，随进程生命周期存活。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::db::{self, DbPool};

/// 完整前缀 `ESC ] 7 7 7 7 ;`——匹配到它才进入「我们的载荷」收集态
const OSC_PREFIX: &[u8] = b"\x1b]7777;";
/// 单条序列收集上限：超过仍未见终止符视为脏数据，已收集字节原样放行（宁漏记不污染流）
const MAX_SEQUENCE_BYTES: usize = 64 * 1024;
/// 有界队列容量：异常场景（片段写错死循环等）下发件方丢弃，不反压读循环
const QUEUE_CAPACITY: usize = 1024;
/// 写线程单批上限（一次事务的最多条数）
const WRITE_BATCH_MAX: usize = 256;
/// 采集开关的 kv 键（"0" = 关）。剥离永远执行，开关只控制是否入库
const ENABLED_KV_KEY: &str = "command_history_enabled";

/// 解析层产物：协议载荷字段（归因与时间由 tap 层补齐）
struct ParsedCommand {
    exit_code: i32,
    command: String,
}

/// 一条命令历史（`host_id` 为 None 表示本地终端）
#[derive(Debug, Clone)]
pub struct CommandRecord {
    pub host_id: Option<i64>,
    pub session_id: i64,
    pub command: String,
    pub exit_code: i32,
    /// 客户端接收时刻（unix 毫秒）——协议不带时间戳，接收时刻即近似执行完成时刻
    pub created_at: i64,
}

/// 查询结果行（带主键 id，供前端作 React key）
#[derive(Debug, Clone)]
pub struct CommandHistoryEntry {
    pub id: i64,
    pub record: CommandRecord,
}

/// 采集总开关：剥离不受它控制（已注入的序列永远不能显示到终端），只控制是否入库
static ENABLED: AtomicBool = AtomicBool::new(true);
/// 队列满/写线程缺失时的累计丢弃数（偶发告警用）
static DROPPED: AtomicU64 = AtomicU64::new(0);

// ===== 扫描状态机 =====

#[derive(Debug)]
enum ScanState {
    /// 常态：逐字节透传，找 ESC
    Ground,
    /// 已见 ESC，待判定（下一个字节是 `]` 才可能是 OSC）
    Escaped,
    /// 逐字节匹配 [`OSC_PREFIX`]，idx 为已匹配长度
    MatchPrefix { idx: usize },
    /// 前缀命中：收集载荷直到 BEL / ST；buf 不含前缀
    Payload { buf: Vec<u8> },
    /// 他方 OSC（确认以 `ESC ]` 开头但非 7777）：整段透传，收集到终止符后原样放行
    Foreign { buf: Vec<u8> },
    /// Payload/Foreign 中遇到 ESC：下一个字节为 `\` 即 ST 终止，否则按杂字节回放
    StProbe { ours: bool, buf: Vec<u8> },
}

/// 单会话的增量扫描器：状态跨 chunk 保持（序列可能被任意切分）
#[derive(Debug)]
struct OscScanner {
    state: ScanState,
}

impl OscScanner {
    fn new() -> Self {
        Self {
            state: ScanState::Ground,
        }
    }

    /// 扫描一个 chunk：透传字节推入 `out`，截获的完整序列解析后经 `sink` 上报
    fn feed<F: FnMut(ParsedCommand)>(&mut self, input: &[u8], out: &mut Vec<u8>, mut sink: F) {
        for &byte in input {
            self.step(byte, out, &mut sink);
        }
    }

    fn step<F: FnMut(ParsedCommand)>(&mut self, byte: u8, out: &mut Vec<u8>, sink: &mut F) {
        match std::mem::replace(&mut self.state, ScanState::Ground) {
            ScanState::Ground => {
                self.state = if byte == 0x1b {
                    ScanState::Escaped
                } else {
                    out.push(byte);
                    ScanState::Ground
                };
            }
            ScanState::Escaped => {
                if byte == b']' {
                    // 前缀前两个字节（ESC ]）已匹配
                    self.state = ScanState::MatchPrefix { idx: 2 };
                } else {
                    // 普通转义序列（CSI/SS2…）：回放 ESC，当前字节按常态重新判定
                    out.push(0x1b);
                    self.step(byte, out, sink);
                }
            }
            ScanState::MatchPrefix { idx } => {
                debug_assert!(idx < OSC_PREFIX.len());
                let matched = OSC_PREFIX.get(idx).is_some_and(|&expected| byte == expected);
                if matched && idx + 1 == OSC_PREFIX.len() {
                    self.state = ScanState::Payload { buf: Vec::new() };
                } else if matched {
                    self.state = ScanState::MatchPrefix { idx: idx + 1 };
                } else {
                    // 不是 7777，但确是 OSC：转透传收集，避免把他方载荷当普通文本误扫
                    let mut buf = OSC_PREFIX.get(..idx).map(<[u8]>::to_vec).unwrap_or_default();
                    buf.push(byte);
                    self.state = ScanState::Foreign { buf };
                }
            }
            ScanState::Payload { mut buf } => match byte {
                0x07 => {
                    // BEL 终止：截获，尝试解析
                    if let Some(parsed) = parse_payload(&buf) {
                        sink(parsed);
                    }
                }
                0x1b => {
                    self.state = ScanState::StProbe { ours: true, buf };
                }
                _ => {
                    buf.push(byte);
                    if OSC_PREFIX.len() + buf.len() > MAX_SEQUENCE_BYTES {
                        // 脏数据：整段原样放行（含前缀），绝不吞用户字节
                        out.extend_from_slice(OSC_PREFIX);
                        out.extend_from_slice(&buf);
                    } else {
                        self.state = ScanState::Payload { buf };
                    }
                }
            },
            ScanState::Foreign { mut buf } => match byte {
                0x07 => {
                    out.extend_from_slice(&buf);
                    out.push(0x07);
                }
                0x1b => {
                    self.state = ScanState::StProbe { ours: false, buf };
                }
                _ => {
                    buf.push(byte);
                    if buf.len() > MAX_SEQUENCE_BYTES {
                        // 无终止符的脏 OSC：放行，不再无限积累
                        out.extend_from_slice(&buf);
                    } else {
                        self.state = ScanState::Foreign { buf };
                    }
                }
            },
            ScanState::StProbe { ours, buf } => {
                if byte == b'\\' {
                    // ST 终止
                    if ours {
                        if let Some(parsed) = parse_payload(&buf) {
                            sink(parsed);
                        }
                    } else {
                        out.extend_from_slice(&buf);
                        out.extend_from_slice(&[0x1b, b'\\']);
                    }
                } else {
                    // 载荷内杂 ESC（我们的编码不会产生；他方 OSC 的病态形态）：
                    // 整段按普通字节放行，当前字节回常态重新判定
                    if ours {
                        out.extend_from_slice(OSC_PREFIX);
                    }
                    out.extend_from_slice(&buf);
                    out.push(0x1b);
                    self.step(byte, out, sink);
                }
            }
        }
    }
}

/// 解析载荷 `cmd;<exit>;<percent-encoded>`。非 `cmd` 子命令返回 None（已剥离，静默忽略）
fn parse_payload(buf: &[u8]) -> Option<ParsedCommand> {
    let (subcmd, rest) = split_first(buf, b';')?;
    if subcmd != b"cmd" {
        return None;
    }
    let (exit_bytes, encoded) = split_first(rest, b';')?;
    let exit_code = std::str::from_utf8(exit_bytes).ok()?.parse::<i32>().ok()?;
    Some(ParsedCommand {
        exit_code,
        command: percent_decode(encoded),
    })
}

/// 按第一个分隔字节切成 (前段, 后段)；无分隔字节返回 None
fn split_first(input: &[u8], sep: u8) -> Option<(&[u8], &[u8])> {
    let pos = input.iter().position(|&b| b == sep)?;
    Some((input.get(..pos)?, input.get(pos + 1..)?))
}

/// percent 解码：`%XX`（十六进制）还原为字节，非法序列按原样放行；解码后按 UTF-8 宽容解码
fn percent_decode(input: &[u8]) -> String {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while let Some(&current) = input.get(i) {
        if current == b'%'
            && let (Some(hi), Some(lo)) = (
                input.get(i + 1).copied().and_then(hex),
                input.get(i + 2).copied().and_then(hex),
            )
        {
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(current);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ===== 会话注册表：terminal id → 扫描器（含 host 归因） =====

struct SessionTap {
    scanner: OscScanner,
    host_id: Option<i64>,
}

static TAPS: LazyLock<Mutex<HashMap<i64, SessionTap>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 输出流挂点：原位剥离 `7777` 序列（data 就地改写），解析成功的记录经队列转写线程。
/// 任何内部错误都静默——采集是旁路功能，绝不影响终端字节流。
pub fn tap_output(session_id: i64, host_id: Option<i64>, data: &mut Vec<u8>) {
    if data.is_empty() {
        return;
    }
    let mut records: Vec<CommandRecord> = Vec::new();
    {
        let mut taps = match TAPS.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let tap = taps.entry(session_id).or_insert_with(|| SessionTap {
            scanner: OscScanner::new(),
            host_id,
        });
        tap.host_id = host_id;
        let sink_host = tap.host_id;
        let mut out = Vec::with_capacity(data.len());
        tap.scanner.feed(data, &mut out, |parsed: ParsedCommand| {
            records.push(CommandRecord {
                host_id: sink_host,
                session_id,
                command: parsed.command,
                exit_code: parsed.exit_code,
                created_at: now_millis(),
            });
        });
        *data = out;
    }
    if !enabled() {
        return;
    }
    for record in records {
        enqueue(record);
    }
}

/// 会话结束：丢弃扫描器状态（幂等；SSH 与本地 PTY 的收尾路径都会调）
pub fn end_session(session_id: i64) {
    let mut taps = match TAPS.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    taps.remove(&session_id);
}

// ===== 有界队列 + 专职写线程 =====

static QUEUE_TX: LazyLock<SyncSender<CommandRecord>> = LazyLock::new(|| {
    let (tx, rx) = sync_channel(QUEUE_CAPACITY);
    // 数据库在 setup 阶段初始化，早于任何终端会话；万一竞态未就绪，本批记录丢弃
    if let Some(pool) = db::global_pool() {
        let _ = std::thread::Builder::new()
            .name("veloterm-cmd-history-writer".to_string())
            .spawn(move || writer_loop(rx, pool));
    }
    tx
});

fn enqueue(record: CommandRecord) {
    if QUEUE_TX.try_send(record).is_err() {
        let total = DROPPED.fetch_add(1, Ordering::Relaxed);
        // 每 500 条告警一次：只在异常场景出现，正常节奏不应见到
        if total.is_multiple_of(500) {
            log::warn!(
                "[command_history] 写入队列堆积，已累计丢弃 {} 条记录",
                total + 1
            );
        }
    }
}

fn writer_loop(rx: Receiver<CommandRecord>, pool: DbPool) {
    while let Ok(first) = rx.recv() {
        let mut batch = Vec::with_capacity(WRITE_BATCH_MAX);
        batch.push(first);
        while batch.len() < WRITE_BATCH_MAX {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(record) => batch.push(record),
                Err(_) => break,
            }
        }
        if let Err(e) = insert_batch(&pool, &batch) {
            log::warn!(
                "[command_history] 批量入库失败（{} 条）: {e}",
                batch.len()
            );
        }
    }
}

fn insert_batch(pool: &DbPool, batch: &[CommandRecord]) -> Result<(), String> {
    let mut conn = pool.get().map_err(|e| format!("pool get: {e}"))?;
    let tx = conn.transaction().map_err(|e| format!("begin tx: {e}"))?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO command_history (host_id, session_id, command, exit_code, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        for record in batch {
            stmt.execute(rusqlite::params![
                record.host_id,
                record.session_id,
                record.command,
                record.exit_code,
                record.created_at,
            ])
            .map_err(|e| format!("insert: {e}"))?;
        }
    }
    tx.commit().map_err(|e| format!("commit: {e}"))
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ===== 采集开关（kv 持久化，db 初始化时回灌） =====

#[inline(always)]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// 应用启动时由 init_db 调用：从 kv 回灌开关（缺省开）
pub fn init_enabled_from_db(pool: &DbPool) {
    let Ok(conn) = pool.get() else {
        return;
    };
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM app_kv WHERE key = ?1",
            rusqlite::params![ENABLED_KV_KEY],
            |row| row.get(0),
        )
        .ok();
    if value.as_deref() == Some("0") {
        ENABLED.store(false, Ordering::Relaxed);
    }
}

/// 持久化开关到 kv（由命令层调用；写库失败不影响内存态）。
/// 走 db::set_kv（app_kv.updated_at NOT NULL，裸 INSERT 漏该列会约束失败）
pub fn persist_enabled(pool: &DbPool, enabled: bool) {
    let value = if enabled { "1" } else { "0" };
    let _ = crate::db::set_kv(pool, ENABLED_KV_KEY, value);
}

// ===== 查询 / 清理（命令层调用） =====

/// 按主机过滤 / 只看失败 / 仅本地，按时间倒序分页（local_only 时忽略 host_id）
pub fn list(
    pool: &DbPool,
    host_id: Option<i64>,
    only_failed: bool,
    local_only: bool,
    limit: i64,
    offset: i64,
) -> Result<Vec<CommandHistoryEntry>, String> {
    let conn = pool.get().map_err(|e| format!("pool get: {e}"))?;
    let mut stmt = conn
        .prepare(
            "SELECT id, host_id, session_id, command, exit_code, created_at FROM command_history
             WHERE (?1 IS NULL OR host_id = ?1)
               AND (?2 = 0 OR exit_code != 0)
               AND (?5 = 0 OR host_id IS NULL)
             ORDER BY created_at DESC, id DESC
             LIMIT ?3 OFFSET ?4",
        )
        .map_err(|e| format!("prepare: {e}"))?;
    let failed_flag = if only_failed { 1 } else { 0 };
    let local_flag = if local_only { 1 } else { 0 };
    let rows = stmt
        .query_map(
            rusqlite::params![host_id, failed_flag, limit, offset, local_flag],
            |row| {
                Ok(CommandHistoryEntry {
                    id: row.get(0)?,
                    record: CommandRecord {
                        host_id: row.get(1)?,
                        session_id: row.get(2)?,
                        command: row.get(3)?,
                        exit_code: row.get(4)?,
                        created_at: row.get(5)?,
                    },
                })
            },
        )
        .map_err(|e| format!("query: {e}"))?;
    let mut entries = Vec::new();
    for row in rows {
        entries.push(row.map_err(|e| format!("row: {e}"))?);
    }
    Ok(entries)
}

/// 清空（host_id = Some 清该主机；local_only = true 只清本地记录；两者均缺省清全部）
pub fn clear(pool: &DbPool, host_id: Option<i64>, local_only: bool) -> Result<usize, String> {
    let conn = pool.get().map_err(|e| format!("pool get: {e}"))?;
    conn.execute(
        "DELETE FROM command_history WHERE (?1 IS NULL OR host_id = ?1) AND (?2 = 0 OR host_id IS NULL)",
        rusqlite::params![host_id, if local_only { 1 } else { 0 }],
    )
    .map_err(|e| format!("delete: {e}"))
}

// ===== 单元测试（macOS 门禁全量跑；覆盖跨 chunk 切分/透传/边界） =====

#[cfg(test)]
mod tests {
    use super::*;

    fn run(chunks: &[&[u8]]) -> (Vec<u8>, Vec<(String, i32)>) {
        let mut scanner = OscScanner::new();
        let mut out = Vec::new();
        let mut records = Vec::new();
        for chunk in chunks {
            scanner.feed(chunk, &mut out, |parsed: ParsedCommand| {
                records.push((parsed.command.clone(), parsed.exit_code));
            });
        }
        (out, records)
    }

    fn seq(exit: &str, cmd: &str) -> Vec<u8> {
        format!("\x1b]7777;cmd;{exit};{cmd}\x1b\\").into_bytes()
    }

    #[test]
    fn plain_text_passthrough_unchanged() {
        let input = b"hello world\r\nls -la\n".to_vec();
        let (out, records) = run(&[&input]);
        assert_eq!(out, input);
        assert!(records.is_empty());
    }

    #[test]
    fn our_sequence_stripped_and_parsed() {
        let mut input = b"before ".to_vec();
        input.extend_from_slice(&seq("0", "echo%20hi"));
        input.extend_from_slice(b" after\r\n");
        let (out, records) = run(&[&input]);
        assert_eq!(out, b"before  after\r\n".to_vec());
        assert_eq!(records, vec![("echo hi".to_string(), 0)]);
    }

    #[test]
    fn bel_terminator_also_accepted() {
        let input = b"\x1b]7777;cmd;130;^C%0DDropped\x07rest".to_vec();
        let (out, records) = run(&[&input]);
        assert_eq!(out, b"rest".to_vec());
        assert_eq!(records, vec![("^C\rDropped".to_string(), 130)]);
    }

    #[test]
    fn negative_exit_code_accepted() {
        // 不做 0..=255 硬校验：宽容解析，语义交给展示层
        let input = seq("-1", "x");
        let (out, records) = run(&[&input]);
        assert_eq!(out, b"".to_vec());
        assert_eq!(records, vec![("x".to_string(), -1)]);
    }

    #[test]
    fn sequence_split_across_chunks() {
        // 前缀、载荷、终止符分别跨 chunk
        let a = b"ok \x1b]77".to_vec();
        let b = b"77;cmd;0;l".to_vec();
        let c = b"s%20-la\x1b\\".to_vec();
        let d = b"!\r\n".to_vec();
        let (out, records) = run(&[&a, &b, &c, &d]);
        assert_eq!(out, b"ok !\r\n".to_vec());
        assert_eq!(records, vec![("ls -la".to_string(), 0)]);
    }

    #[test]
    fn prefix_split_between_every_byte() {
        // 前缀逐字节跨 chunk 的极端切分
        let full = seq("7", "uptime");
        let chunks: Vec<Vec<u8>> = full.iter().map(|b| vec![*b]).collect();
        let refs: Vec<&[u8]> = chunks.iter().map(|c| c.as_slice()).collect();
        let (out, records) = run(&refs);
        assert_eq!(out, b"".to_vec());
        assert_eq!(records, vec![("uptime".to_string(), 7)]);
    }

    #[test]
    fn foreign_osc_passthrough_intact() {
        // 他方 OSC 0 改标题：必须原样到达前端（xterm 要消费它）
        let input = b"\x1b]0;my title\x07visible".to_vec();
        let (out, records) = run(&[&input]);
        assert_eq!(out, input);
        assert!(records.is_empty());
    }

    #[test]
    fn foreign_osc_with_st_terminator_passthrough() {
        // OSC 8 超链接（ST 终止 + ESC 反复出现）
        let input = b"\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\".to_vec();
        let (out, records) = run(&[&input]);
        assert_eq!(out, input);
        assert!(records.is_empty());
    }

    #[test]
    fn non_osc_escape_passthrough() {
        // CSI / 方向键等普通转义序列不受影响
        let input = b"\x1b[1;32mgreen\x1b[0m\x1b[A".to_vec();
        let (out, records) = run(&[&input]);
        assert_eq!(out, input);
        assert!(records.is_empty());
    }

    #[test]
    fn lone_esc_passthrough() {
        // 孤立 ESC（流被切断的病态形态）：原样放行
        let input = b"a\x1bb".to_vec();
        let (out, records) = run(&[&input]);
        assert_eq!(out, input);
        assert!(records.is_empty());
    }

    #[test]
    fn almost_our_prefix_is_foreign() {
        // 7778 不是我们的：透传
        let input = b"\x1b]7778;cmd;0;xx\x07tail".to_vec();
        let (out, records) = run(&[&input]);
        assert_eq!(out, input);
        assert!(records.is_empty());
    }

    #[test]
    fn unknown_subcommand_stripped_but_not_recorded() {
        // 前瞻：未来新增子命令，旧客户端剥离且不入库
        let input = b"head\x1b]7777;cwd;/tmp\x1b\\tail".to_vec();
        let (out, records) = run(&[&input]);
        assert_eq!(out, b"headtail".to_vec());
        assert!(records.is_empty());
    }

    #[test]
    fn invalid_exit_code_dropped_whole_sequence_stripped() {
        let input = b"a\x1b]7777;cmd;abc;xx\x1b\\b".to_vec();
        let (out, records) = run(&[&input]);
        assert_eq!(out, b"ab".to_vec());
        assert!(records.is_empty());
    }

    #[test]
    fn malformed_esc_in_payload_passes_through() {
        // 载荷中出现 ESC 且后随非 '\'：整段（含前缀）按普通字节放行
        let input = b"x\x1b]7777;cmd;0;a\x1b[2Ky".to_vec();
        let (out, records) = run(&[&input]);
        assert_eq!(out, input);
        assert!(records.is_empty());
    }

    #[test]
    fn overflow_flushes_unchanged() {
        // 超长无终止：整段原样放行
        let mut input = b"\x1b]7777;cmd;0;".to_vec();
        input.extend(std::iter::repeat_n(b'A', MAX_SEQUENCE_BYTES + 16));
        input.push(0x07);
        let (out, records) = run(&[&input]);
        assert_eq!(out, input);
        assert!(records.is_empty());
    }

    #[test]
    fn percent_decode_utf8_and_stray_percent() {
        // CJK 命令（UTF-8 逐字节编码）+ 尾部孤立 %
        let input = seq("0", "echo%20%E4%BD%A0%E5%A5%BD%25");
        let (out, records) = run(&[&input]);
        assert_eq!(out, b"".to_vec());
        assert_eq!(records, vec![("echo 你好%".to_string(), 0)]);
    }

    #[test]
    fn consecutive_sequences_and_interleaved_text() {
        let mut input = Vec::new();
        input.extend_from_slice(&seq("0", "a"));
        input.extend_from_slice(b"\r\n");
        input.extend_from_slice(&seq("1", "b"));
        input.extend_from_slice(b"\x1b]0;t\x07");
        input.extend_from_slice(&seq("2", "c%3Bd"));
        let (out, records) = run(&[&input]);
        assert_eq!(out, b"\r\n\x1b]0;t\x07".to_vec());
        assert_eq!(
            records,
            vec![
                ("a".to_string(), 0),
                ("b".to_string(), 1),
                ("c;d".to_string(), 2),
            ]
        );
    }

    #[test]
    fn percent_decode_basics() {
        assert_eq!(percent_decode(b"a%20b"), "a b");
        assert_eq!(percent_decode(b"100%"), "100%");
        assert_eq!(percent_decode(b"%zz"), "%zz");
        assert_eq!(percent_decode(b"%C3%A9"), "é");
        assert_eq!(percent_decode(b"%c3%a9"), "é"); // 小写十六进制
        assert_eq!(percent_decode(b"%C3"), "\u{FFFD}"); // 截断的 UTF-8 序列 → lossy
    }

    #[test]
    fn split_first_basics() {
        assert_eq!(split_first(b"a;b;c", b';'), Some((&b"a"[..], &b"b;c"[..])));
        assert_eq!(split_first(b";x", b';'), Some((&b""[..], &b"x"[..])));
        assert_eq!(split_first(b"abc", b';'), None);
    }
}
