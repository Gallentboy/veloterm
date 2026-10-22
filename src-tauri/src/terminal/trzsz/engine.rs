//! trzsz 传输引擎：与既有 ZMODEM（`terminal/zmodem/`）并列的终端内传输。
//!
//! 数据全程在 Rust（不经前端）：SSH 输出流喂 `TrzszBridge` → tokio 通道 →
//! 引擎任务（帧状态机）；引擎写回经 outbound 通道 → SSH 通道输入。
//!
//! 流程（协议参照 trzsz-go MIT 客户端，Rust 自实现；仅文本模式 base64+zlib，
//! ACT 声明 `binary:false`、`protocol:1`——V1 语义全版本服务端兼容）：
//! - 检测命中 → 本地对话框（下载选目录 / trz 选文件 / trz -d 选目录）→
//!   `#ACT:`（confirm=false 表示取消，服务端静默退出）→ 收 `#CFG:`（超时/
//!   目录/覆盖写等配置）→ NUM/(NAME/SIZE/DATA*/MD5)×N → `#EXIT:`
//! - 下载（tsz，模式 S）：NAME（目录模式为 sourceFile JSON）→ 本地建文件/
//!   目录树（同名 `.0/.1` 重命名，CFG.overwrite 时直写）→ 逐帧回 SUCC
//! - 上传（trz，模式 R/D）：本地清单（目录树展开为逐条 sourceFile）→
//!   逐块发 DATA、按 SUCC 确认、块大小自适应（1024 → CFG.bufsize）
//! - 取消：用户 Ctrl+C 经终端输入直达服务端（服务端 abort 并回 `#fail:`）；
//!   会话关闭 → 宿主 drop 桥（入向通道关闭）+ 置取消位，引擎读循环即刻退出

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::Serialize;
use tauri::AppHandle;
use tauri::Emitter;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use super::codec::{
    LineError, LineReader, SourceFileJson, decode_payload, encode_payload, extract_frame,
    parse_frame, Md5Stream,
};
use super::detector::TrszszTrigger;

/// 前端事件名（终端页监听后渲染就地状态行）
pub const TRZSZ_EVENT: &str = "trzsz-progress";

/// 块大小自适应上限（协议 bufsize 上限）
const MAX_BUF_SIZE: usize = 10 * 1024 * 1024;
/// 默认读超时（CFG 之前）
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
/// 进度事件节流间隔
const EMIT_INTERVAL: Duration = Duration::from_millis(200);

/// 进度事件（前端只消费这个，不接触协议字节；字段与 ZmodemProgress 同构）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrzszProgress {
    pub session_id: String,
    /// "download"（tsz，服务器→本地）| "upload"（trz，本地→服务器）
    pub direction: &'static str,
    pub file_name: String,
    pub transferred: u64,
    pub total: u64,
    pub speed_bps: u64,
    pub elapsed_ms: u64,
    pub is_finished: bool,
    pub error: Option<String>,
}

/// 进度事件出口（生产走 Tauri emit；测试用捕获实现）
pub(crate) trait EventSink: Send + Sync {
    fn progress(&self, p: TrzszProgress);
}

struct TauriEventSink {
    app: AppHandle,
}

impl EventSink for TauriEventSink {
    fn progress(&self, p: TrzszProgress) {
        let _ = self.app.emit(TRZSZ_EVENT, p);
    }
}

/// 传输桥（宿主侧句柄）：探测命中后由 SSH 事件循环持有，喂数据与取消。
/// 可 Clone（宿主存一份、任务侧取标志后立即 drop，保证宿主 drop 桥 =
/// 入向通道关闭 = 引擎读到 EOF 即刻退出）。
#[derive(Clone)]
pub struct TrzszBridge {
    pub session_id: String,
    pub direction: &'static str,
    inbound_tx: UnboundedSender<Vec<u8>>,
    cancelled: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}

impl TrzszBridge {
    pub fn new(session_id: String, direction: &'static str) -> (Self, UnboundedReceiver<Vec<u8>>) {
        let (inbound_tx, inbound_rx) = tokio::sync::mpsc::unbounded_channel();
        (
            Self {
                session_id,
                direction,
                inbound_tx,
                cancelled: Arc::new(AtomicBool::new(false)),
                finished: Arc::new(AtomicBool::new(false)),
            },
            inbound_rx,
        )
    }

    /// 喂 SSH 收到的字节给引擎。false = 引擎侧接收端已 drop，调用方应把
    /// 本块字节回落到终端透传（否则吞掉 shell 提示符）。
    pub fn feed(&self, data: &[u8]) -> bool {
        self.inbound_tx.send(data.to_vec()).is_ok()
    }

    /// 请求取消：置取消位（引擎下一检查点退出）+ 置 finished（宿主恢复透传）
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.finished.store(true, Ordering::Relaxed);
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    pub fn finished_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.finished)
    }
}

/// 引擎错误
#[derive(Debug)]
pub(crate) enum EngineError {
    /// 用户取消（对话框取消 / Ctrl+C / 会话关闭）
    Cancelled,
    /// 连接（通道）关闭
    ConnectionClosed,
    /// 帧读超时
    Timeout,
    /// 流中出现 Ctrl+C
    Interrupted,
    /// 协议违背（意外帧型/解析失败/校验不合）
    Protocol(String),
    /// 服务端 `#fail:` / `#FAIL:` / `#EXIT:`（消息已解码）
    RemoteFail(String),
    /// 本地文件 IO
    Io(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "cancelled"),
            Self::ConnectionClosed => write!(f, "connection closed"),
            Self::Timeout => write!(f, "receive data timeout"),
            Self::Interrupted => write!(f, "interrupted"),
            Self::Protocol(msg) => write!(f, "protocol error: {msg}"),
            Self::RemoteFail(msg) => write!(f, "{msg}"),
            Self::Io(msg) => write!(f, "{msg}"),
        }
    }
}

fn map_line_err(e: LineError) -> EngineError {
    match e {
        LineError::Interrupted => EngineError::Interrupted,
        LineError::Closed => EngineError::ConnectionClosed,
        LineError::Timeout => EngineError::Timeout,
        LineError::Cancelled => EngineError::Cancelled,
        LineError::TooLong => EngineError::Protocol("line too long".into()),
    }
}

/// 客户端能力声明（ACT 帧 JSON；字段名与上游一致）
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct ClientAction {
    lang: &'static str,
    version: &'static str,
    confirm: bool,
    newline: &'static str,
    protocol: i64,
    binary: bool,
    support_dir: bool,
}

/// 服务端配置（CFG 帧 JSON；缺省字段取协议默认）
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ServerConfig {
    #[allow(dead_code)]
    quiet: bool,
    binary: bool,
    directory: bool,
    overwrite: bool,
    timeout: i64,
    #[serde(default = "default_bufsize")]
    bufsize: i64,
    tmux_output_junk: bool,
    #[allow(dead_code)]
    tmux_pane_width: i64,
}

/// 协议默认 bufsize（10MB）
fn default_bufsize() -> i64 {
    10 * 1024 * 1024
}

/// 上传清单条目（sourceFile 的本地侧形态）
#[derive(Debug, Clone)]
pub(crate) struct UploadEntry {
    pub path_id: i64,
    pub abs: PathBuf,
    pub rel: Vec<String>,
    pub is_dir: bool,
    pub size: u64,
}

impl UploadEntry {
    fn display_name(&self) -> String {
        self.rel.last().cloned().unwrap_or_default()
    }
}

/// 传输结果摘要（完成事件用）
pub(crate) struct TransferSummary {
    pub names: Vec<String>,
    pub total_bytes: u64,
}

/// 传输引擎（帧状态机）。持有双向通道与行读取器，所有 IO 经此收发。
pub(crate) struct Engine {
    sink: Arc<dyn EventSink>,
    session_id: String,
    direction: &'static str,
    rx: UnboundedReceiver<Vec<u8>>,
    tx: UnboundedSender<Vec<u8>>,
    reader: LineReader,
    cancel: Arc<AtomicBool>,
    /// 发送行终结符：Windows 服务端为 "!\n"（conhost 下裸 \n 不可靠）
    newline: &'static str,
    /// tmux 噪声容错（CFG 后确定；ACT/CFG 阶段按 may_junk=true 读）
    junk: bool,
    /// CFG 之后的读超时（0 = 永不）
    timeout: Option<Duration>,
    bufsize: usize,
    directory: bool,
    overwrite: bool,
    /// 已收字节（完成事件总量展示）
    received_bytes: u64,
    // 进度事件状态
    progress_name: String,
    last_emit_at: Instant,
    last_emit_bytes: u64,
}

impl Engine {
    pub(crate) fn new(
        sink: Arc<dyn EventSink>,
        session_id: String,
        direction: &'static str,
        trigger: &TrszszTrigger,
        rx: UnboundedReceiver<Vec<u8>>,
        tx: UnboundedSender<Vec<u8>>,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        let mut reader = LineReader::new();
        reader.set_windows(trigger.win_server);
        reader.set_cancel(Arc::clone(&cancel));
        Self {
            sink,
            session_id,
            direction,
            rx,
            tx,
            reader,
            cancel,
            newline: if trigger.win_server { "!\n" } else { "\n" },
            junk: false,
            timeout: Some(DEFAULT_TIMEOUT),
            bufsize: 1024 * 1024,
            directory: false,
            overwrite: false,
            received_bytes: 0,
            progress_name: String::new(),
            last_emit_at: Instant::now(),
            last_emit_bytes: 0,
        }
    }

    fn check_cancel(&self) -> Result<(), EngineError> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(EngineError::Cancelled)
        } else {
            Ok(())
        }
    }

    // ── 帧收发 ──

    fn send_line(&mut self, typ: &str, content: &str) -> Result<(), EngineError> {
        let mut line = format!("#{typ}:{content}");
        line.push_str(self.newline);
        self.tx
            .send(line.into_bytes())
            .map_err(|_| EngineError::ConnectionClosed)
    }

    fn send_string(&mut self, typ: &str, s: &str) -> Result<(), EngineError> {
        self.send_line(typ, &encode_payload(s.as_bytes()))
    }

    fn send_payload(&mut self, typ: &str, data: &[u8]) -> Result<(), EngineError> {
        self.send_line(typ, &encode_payload(data))
    }

    fn send_integer(&mut self, typ: &str, v: i64) -> Result<(), EngineError> {
        self.send_line(typ, &v.to_string())
    }

    /// 读一帧并校验类型。类型不合时：fail/FAIL/EXIT 解码为远端错误，
    /// 其余按协议错误处理。
    async fn recv_check(&mut self, expect: &str, may_junk: bool) -> Result<String, EngineError> {
        self.check_cancel()?;
        let effective_junk = may_junk || self.junk;
        let line = self
            .reader
            .read_logical(&mut self.rx, self.timeout, effective_junk)
            .await
            .map_err(map_line_err)?;
        let frame = extract_frame(&line, expect, effective_junk);
        let (typ, content) = parse_frame(&frame).map_err(EngineError::Protocol)?;
        if typ != expect {
            if matches!(typ, "fail" | "FAIL" | "EXIT") {
                let msg = decode_payload(content)
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_else(|_| content.to_string());
                return Err(EngineError::RemoteFail(msg));
            }
            return Err(EngineError::Protocol(format!(
                "expect #{expect}: frame, got #{typ}"
            )));
        }
        Ok(content.to_string())
    }

    async fn recv_string(&mut self, typ: &str, may_junk: bool) -> Result<String, EngineError> {
        let content = self.recv_check(typ, may_junk).await?;
        let bytes = decode_payload(&content).map_err(EngineError::Protocol)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn recv_integer(&mut self, typ: &str, may_junk: bool) -> Result<i64, EngineError> {
        let content = self.recv_check(typ, may_junk).await?;
        content
            .trim()
            .parse::<i64>()
            .map_err(|e| EngineError::Protocol(format!("parse integer from {content:?}: {e}")))
    }

    async fn check_integer(&mut self, expect: i64) -> Result<(), EngineError> {
        let got = self.recv_integer("SUCC", false).await?;
        if got != expect {
            return Err(EngineError::Protocol(format!(
                "integer check [{got}] <> [{expect}]"
            )));
        }
        Ok(())
    }

    // ── 进度事件 ──

    fn emit_start_file(&mut self, name: &str, total: u64) {
        self.progress_name = name.to_string();
        self.last_emit_at = Instant::now();
        self.last_emit_bytes = 0;
        self.sink.progress(TrzszProgress {
            session_id: self.session_id.clone(),
            direction: self.direction,
            file_name: name.to_string(),
            transferred: 0,
            total,
            speed_bps: 0,
            elapsed_ms: 0,
            is_finished: false,
            error: None,
        });
    }

    fn emit_step(&mut self, transferred: u64, total: u64) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_emit_at);
        if dt < EMIT_INTERVAL {
            return;
        }
        let speed = (transferred.saturating_sub(self.last_emit_bytes) as f64 / dt.as_secs_f64())
            as u64;
        self.last_emit_at = now;
        self.last_emit_bytes = transferred;
        self.sink.progress(TrzszProgress {
            session_id: self.session_id.clone(),
            direction: self.direction,
            file_name: self.progress_name.clone(),
            transferred,
            total,
            speed_bps: speed,
            elapsed_ms: 0,
            is_finished: false,
            error: None,
        });
    }

    // ── 配置协商 ──

    fn send_action(&mut self, confirm: bool, win_server: bool) -> Result<(), EngineError> {
        let action = ClientAction {
            lang: "rs",
            version: env!("CARGO_PKG_VERSION"),
            confirm,
            newline: if win_server { "!\n" } else { "\n" },
            // 协议声明 v1（zlib+base64 文本模式、逐块 SUCC 应答）：全版本
            // 服务端兼容（Python 客户端同此声明）
            protocol: 1,
            binary: false,
            support_dir: true,
        };
        let json = serde_json::to_string(&action).map_err(|e| EngineError::Protocol(e.to_string()))?;
        self.send_string("ACT", &json)
    }

    async fn recv_config(&mut self) -> Result<ServerConfig, EngineError> {
        let payload = self.recv_string("CFG", true).await?;
        let cfg: ServerConfig =
            serde_json::from_str(&payload).map_err(|e| EngineError::Protocol(e.to_string()))?;
        if cfg.binary {
            // 我们声明 binary:false，正常服务端不会选中二进制模式
            return Err(EngineError::Protocol(
                "server selected binary mode (unsupported)".into(),
            ));
        }
        self.junk = cfg.tmux_output_junk;
        self.timeout = if cfg.timeout > 0 {
            Some(Duration::from_secs(cfg.timeout as u64))
        } else {
            None
        };
        let bufsize = if cfg.bufsize > 0 {
            cfg.bufsize
        } else {
            default_bufsize()
        };
        self.bufsize = (bufsize as usize).clamp(1024, MAX_BUF_SIZE);
        self.directory = cfg.directory;
        self.overwrite = cfg.overwrite;
        Ok(cfg)
    }

    /// 失败上报（对应 trzsz-go clientError）：本地错误回 `#FAIL:` 让服务端
    /// 立即中止；远端错误/连接关闭无需再发。尽力而为，失败不掩盖原错误。
    async fn report_failure(&mut self, err: &EngineError) {
        if matches!(
            err,
            EngineError::RemoteFail(_) | EngineError::ConnectionClosed | EngineError::Cancelled
        ) {
            return;
        }
        let msg = err.to_string();
        let _ = self.send_string("FAIL", &msg);
    }

    // ── 下载（tsz：收文件）──

    async fn recv_files(&mut self, dir: &Path) -> Result<Vec<String>, EngineError> {
        let num = self.recv_integer("NUM", false).await?;
        self.send_integer("SUCC", num)?;
        let mut saved: Vec<String> = Vec::new();
        let mut name_map: HashMap<i64, String> = HashMap::new();
        for _ in 0..num {
            self.check_cancel()?;
            let payload = self.recv_string("NAME", false).await?;
            let (file_name, local_name, file) = if self.directory {
                self.create_dir_or_file(dir, &payload, &mut name_map)?
            } else {
                safe_segment(&payload)?;
                let local = get_new_name(dir, &payload, self.overwrite)?;
                let full = dir.join(&local);
                let file = std::fs::File::create(&full)
                    .map_err(|e| EngineError::Io(format!("create {}: {e}", full.display())))?;
                (payload.clone(), local, Some(file))
            };
            if !saved.contains(&local_name) {
                saved.push(local_name.clone());
            }
            self.send_string("SUCC", &local_name)?;
            let Some(mut file) = file else {
                continue; // 目录条目：无 SIZE/DATA
            };
            let size = self.recv_integer("SIZE", false).await?;
            if size < 0 {
                return Err(EngineError::Protocol(format!("negative size {size}")));
            }
            self.send_integer("SUCC", size)?;
            self.emit_start_file(&file_name, size as u64);
            let digest = self.recv_file_data(&mut file, size as u64).await?;
            self.check_file_md5(&digest).await?;
        }
        Ok(saved)
    }

    /// 目录模式 NAME 载荷 → 本地建目录树/文件（含根名重命名的 path_id 一致性）
    fn create_dir_or_file(
        &mut self,
        dir: &Path,
        payload: &str,
        name_map: &mut HashMap<i64, String>,
    ) -> Result<(String, String, Option<std::fs::File>), EngineError> {
        let sf: SourceFileJson = serde_json::from_str(payload)
            .map_err(|e| EngineError::Protocol(format!("bad NAME json: {e}")))?;
        if sf.path_name.is_empty() {
            return Err(EngineError::Protocol("empty path_name".into()));
        }
        for seg in &sf.path_name {
            safe_segment(seg)?;
        }
        let file_name = sf.file_name();
        // 根名映射：同 path_id（同一棵源树）共用同一个重命名后的根名
        // （path_name 非空已前置校验；first() 取不到按空段路径错误兜底）
        let Some(root_seg) = sf.path_name.first().cloned() else {
            return Err(EngineError::Protocol("empty path_name".into()));
        };
        let root = if self.overwrite {
            root_seg
        } else {
            match name_map.get(&sf.path_id) {
                Some(v) => v.clone(),
                None => {
                    let v = get_new_name(dir, &root_seg, false)?;
                    name_map.insert(sf.path_id, v.clone());
                    v
                }
            }
        };
        let full = if sf.path_name.len() > 1 {
            let mut p = dir.join(&root);
            for seg in sf.path_name.get(1..sf.path_name.len() - 1).unwrap_or_default() {
                p = p.join(seg);
            }
            std::fs::create_dir_all(&p)
                .map_err(|e| EngineError::Io(format!("mkdir {}: {e}", p.display())))?;
            p.join(&file_name)
        } else {
            dir.join(&root)
        };
        if sf.is_dir {
            std::fs::create_dir_all(&full)
                .map_err(|e| EngineError::Io(format!("mkdir {}: {e}", full.display())))?;
            return Ok((file_name, root, None));
        }
        let file = std::fs::File::create(&full)
            .map_err(|e| EngineError::Io(format!("create {}: {e}", full.display())))?;
        Ok((file_name, root, Some(file)))
    }

    async fn recv_file_data(
        &mut self,
        file: &mut std::fs::File,
        size: u64,
    ) -> Result<[u8; 16], EngineError> {
        let mut step: u64 = 0;
        let mut hasher = Md5Stream::new();
        while step < size {
            self.check_cancel()?;
            let content = self.recv_check("DATA", false).await?;
            let data = decode_payload(&content).map_err(EngineError::Protocol)?;
            file.write_all(&data)
                .map_err(|e| EngineError::Io(format!("write file: {e}")))?;
            step += data.len() as u64;
            self.received_bytes += data.len() as u64;
            hasher.update(&data);
            self.send_integer("SUCC", data.len() as i64)?;
            self.emit_step(step.min(size), size);
        }
        Ok(hasher.finalize())
    }

    async fn check_file_md5(&mut self, digest: &[u8; 16]) -> Result<(), EngineError> {
        let content = self.recv_check("MD5", false).await?;
        let expect = decode_payload(&content).map_err(EngineError::Protocol)?;
        if expect.as_slice() != digest {
            return Err(EngineError::Protocol("Check MD5 failed".into()));
        }
        self.send_payload("SUCC", digest)
    }

    // ── 上传（trz：发文件）──

    async fn send_files(&mut self, entries: &[UploadEntry]) -> Result<Vec<String>, EngineError> {
        self.send_integer("NUM", entries.len() as i64)?;
        self.check_integer(entries.len() as i64).await?;
        let mut remote_names: Vec<String> = Vec::new();
        let mut cumulative: u64 = 0;
        let total: u64 = entries.iter().map(|e| e.size).sum();
        for entry in entries {
            self.check_cancel()?;
            let payload = if self.directory {
                let sf = SourceFileJson {
                    path_id: entry.path_id,
                    path_name: entry.rel.clone(),
                    is_dir: entry.is_dir,
                    size: entry.size as i64,
                };
                serde_json::to_string(&sf)
                    .map_err(|e| EngineError::Protocol(e.to_string()))?
            } else {
                entry.display_name()
            };
            self.send_string("NAME", &payload)?;
            let remote = self.recv_string("SUCC", false).await?;
            if !remote_names.contains(&remote) {
                remote_names.push(remote);
            }
            self.emit_start_file(&entry.display_name(), total);
            if entry.is_dir {
                continue;
            }
            self.send_integer("SIZE", entry.size as i64)?;
            self.check_integer(entry.size as i64).await?;
            cumulative = self.send_file_data(entry, cumulative, total).await?;
        }
        Ok(remote_names)
    }

    async fn send_file_data(
        &mut self,
        entry: &UploadEntry,
        mut cumulative: u64,
        total: u64,
    ) -> Result<u64, EngineError> {
        let mut file = std::fs::File::open(&entry.abs)
            .map_err(|e| EngineError::Io(format!("open {}: {e}", entry.abs.display())))?;
        let mut hasher = Md5Stream::new();
        let mut step: u64 = 0;
        let mut chunk_size: usize = 1024;
        let mut buf = vec![0u8; chunk_size];
        while step < entry.size {
            self.check_cancel()?;
            let want = chunk_size.min((entry.size - step) as usize);
            let n = file
                .read(buf.get_mut(..want).unwrap_or(&mut []))
                .map_err(|e| EngineError::Io(format!("read file: {e}")))?;
            if n == 0 {
                return Err(EngineError::Protocol(format!(
                    "EOF but step [{step}] <> size [{}]",
                    entry.size
                )));
            }
            let begin = Instant::now();
            self.send_payload("DATA", buf.get(..n).unwrap_or_default())?;
            hasher.update(buf.get(..n).unwrap_or_default());
            self.check_integer(n as i64).await?;
            step += n as u64;
            cumulative += n as u64;
            self.emit_step(cumulative, total);
            let chunk_time = begin.elapsed();
            if n == chunk_size
                && chunk_time < Duration::from_millis(500)
                && chunk_size < self.bufsize
            {
                chunk_size = (chunk_size * 2).min(self.bufsize);
                buf.resize(chunk_size, 0);
            } else if chunk_time >= Duration::from_secs(2) && chunk_size > 1024 {
                chunk_size = 1024;
                buf.resize(chunk_size, 0);
            }
        }
        let digest = hasher.finalize();
        self.send_payload("MD5", &digest)?;
        // 服务端校验后回 SUCC（载荷必须与摘要一致）
        let ack = self.recv_check("SUCC", false).await?;
        let ack_digest = decode_payload(&ack).map_err(EngineError::Protocol)?;
        if ack_digest.as_slice() != digest {
            return Err(EngineError::Protocol("MD5 ack mismatch".into()));
        }
        Ok(cumulative)
    }
}

// ── 本地文件辅助 ──

/// 路径段安全校验：拒绝穿越/分隔符/控制字符（接收侧加固，上游未做）
fn safe_segment(seg: &str) -> Result<(), EngineError> {
    if seg.is_empty()
        || seg == "."
        || seg == ".."
        || seg.contains('/')
        || seg.contains('\\')
        || seg.contains('\0')
        || seg.len() > 255
    {
        return Err(EngineError::Protocol(format!("unsafe path segment: {seg:?}")));
    }
    Ok(())
}

/// 同名冲突重命名：name、name.0、name.1 ..（overwrite 时原样使用）
fn get_new_name(dir: &Path, name: &str, overwrite: bool) -> Result<String, EngineError> {
    if overwrite || !dir.join(name).exists() {
        return Ok(name.to_string());
    }
    for i in 0..1000 {
        let candidate = format!("{name}.{i}");
        if !dir.join(&candidate).exists() {
            return Ok(candidate);
        }
    }
    Err(EngineError::Io(format!(
        "fail to assign new file name to {name}"
    )))
}

/// EXIT 载荷：Saved N file(s)[ to path] + 清单（服务端会把它显示到终端）
fn format_saved_files(names: &[String], path: Option<&Path>) -> String {
    let mut s = format!(
        "Saved {} {}",
        names.len(),
        if names.len() > 1 {
            "files/directories"
        } else {
            "file/directory"
        }
    );
    if let Some(p) = path
        && !p.as_os_str().is_empty()
    {
        s.push_str(&format!(" to {}", p.display()));
    }
    for name in names {
        s.push_str(&format!("\r\n- {name}"));
    }
    s
}

/// 展开上传清单：目录递归为逐条 sourceFile（先根目录后子项，条目名排序保证
/// 确定性；符号链接环路按 canonicalize 去重剪枝）
pub(crate) fn collect_upload_entries(paths: &[PathBuf]) -> Result<Vec<UploadEntry>, String> {
    let mut out = Vec::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    for (i, p) in paths.iter().enumerate() {
        let meta = std::fs::metadata(p).map_err(|e| format!("stat {}: {e}", p.display()))?;
        let name = p
            .file_name()
            .ok_or_else(|| format!("invalid upload path: {}", p.display()))?
            .to_string_lossy()
            .into_owned();
        collect_walk(p, i as i64, vec![name], &meta, &mut out, &mut visited)?;
    }
    Ok(out)
}

fn collect_walk(
    path: &Path,
    path_id: i64,
    rel: Vec<String>,
    meta: &std::fs::Metadata,
    out: &mut Vec<UploadEntry>,
    visited: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    if meta.is_dir() {
        let canon = path
            .canonicalize()
            .map_err(|e| format!("realpath {}: {e}", path.display()))?;
        if !visited.insert(canon) {
            return Ok(()); // 环路/重复链接：跳过
        }
        out.push(UploadEntry {
            path_id,
            abs: path.to_path_buf(),
            rel: rel.clone(),
            is_dir: true,
            size: 0,
        });
        let mut children: Vec<(PathBuf, String, std::fs::Metadata)> = Vec::new();
        let rd = std::fs::read_dir(path).map_err(|e| format!("readdir {}: {e}", path.display()))?;
        for entry in rd {
            let entry = entry.map_err(|e| format!("readdir {}: {e}", path.display()))?;
            let child_meta =
                std::fs::metadata(entry.path()).map_err(|e| format!("stat: {e}"))?;
            children.push((
                entry.path(),
                entry.file_name().to_string_lossy().into_owned(),
                child_meta,
            ));
        }
        children.sort_by(|a, b| a.1.cmp(&b.1));
        for (child_path, child_name, child_meta) in children {
            let mut child_rel = rel.clone();
            child_rel.push(child_name);
            collect_walk(&child_path, path_id, child_rel, &child_meta, out, visited)?;
        }
    } else if meta.is_file() {
        out.push(UploadEntry {
            path_id,
            abs: path.to_path_buf(),
            rel,
            is_dir: false,
            size: meta.len(),
        });
    }
    // 套接字/FIFO/设备等非常规文件跳过
    Ok(())
}

// ── 传输编排 ──

/// 探测命中后的入口：弹对话框 → 协商 → 收/发文件 → EXIT → 完成事件。
/// 由 SSH 事件循环 spawn；`bridge` 的标志位取出后立即 drop（宿主成为入向
/// 通道唯一持有者，会话关闭即引擎读到 EOF）。
pub fn start_transfer(
    app: AppHandle,
    direction: &'static str,
    trigger: TrszszTrigger,
    inbound_rx: UnboundedReceiver<Vec<u8>>,
    outbound_tx: UnboundedSender<Vec<u8>>,
    bridge: TrzszBridge,
) {
    tokio::spawn(async move {
        let session_id = bridge.session_id.clone();
        let finished = bridge.finished_flag();
        let cancel = bridge.cancel_flag();
        drop(bridge); // 释放 inbound_tx 克隆
        let sink: Arc<dyn EventSink> = Arc::new(TauriEventSink { app });
        let started = Instant::now();
        sink.progress(TrzszProgress {
            session_id: session_id.clone(),
            direction,
            file_name: String::new(),
            transferred: 0,
            total: 0,
            speed_bps: 0,
            elapsed_ms: 0,
            is_finished: false,
            error: None,
        });
        let result = run_transfer(
            direction,
            &trigger,
            inbound_rx,
            outbound_tx,
            cancel,
            Arc::clone(&sink),
            session_id.clone(),
        )
        .await;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let (error, names, total) = match result {
            Ok(summary) => (None, summary.names, summary.total_bytes),
            Err(EngineError::Cancelled) => (Some("cancelled".into()), Vec::new(), 0),
            Err(e) => (Some(e.to_string()), Vec::new(), 0),
        };
        let file_name = names.join(", ");
        let transferred = if error.is_none() { total } else { 0 };
        sink.progress(TrzszProgress {
            session_id,
            direction,
            file_name,
            transferred,
            total,
            speed_bps: 0,
            elapsed_ms,
            is_finished: true,
            error,
        });
        finished.store(true, Ordering::Relaxed);
    });
}

async fn run_transfer(
    direction: &'static str,
    trigger: &TrszszTrigger,
    inbound_rx: UnboundedReceiver<Vec<u8>>,
    outbound_tx: UnboundedSender<Vec<u8>>,
    cancel: Arc<AtomicBool>,
    sink: Arc<dyn EventSink>,
    session_id: String,
) -> Result<TransferSummary, EngineError> {
    let mut engine = Engine::new(
        sink,
        session_id,
        direction,
        trigger,
        inbound_rx,
        outbound_tx,
        Arc::clone(&cancel),
    );

    // 对话框阶段（原生对话框必须挪出异步线程，同 zmodem 的实测结论）
    enum Picked {
        Dir(PathBuf),
        Files(Vec<PathBuf>),
    }
    let dialog_title = if direction == "download" {
        "选择 trzsz 下载保存目录"
    } else if trigger.mode == b'D' {
        "选择要上传的目录"
    } else {
        "选择要上传的文件"
    };
    let pick_dir = direction == "download" || trigger.mode == b'D';
    let picked = tokio::task::spawn_blocking(move || {
        let dialog = rfd::FileDialog::new().set_title(dialog_title);
        if pick_dir {
            dialog.pick_folder().map(Picked::Dir)
        } else {
            dialog.pick_files().map(Picked::Files)
        }
    })
    .await
    .unwrap_or(None);

    engine.check_cancel()?;

    let result = match direction {
        "download" => {
            run_download(&mut engine, picked.map(|p| match p {
                Picked::Dir(d) => d,
                Picked::Files(_) => unreachable!("download always picks a folder"),
            }),
            trigger)
            .await
        }
        _ => {
            run_upload(&mut engine, picked.map(|p| match p {
                Picked::Dir(d) => vec![d],
                Picked::Files(f) => f,
            }),
            trigger)
            .await
        }
    };
    match &result {
        Ok(_) => {}
        Err(e) => {
            engine.report_failure(e).await;
        }
    }
    result
}

async fn run_download(
    engine: &mut Engine,
    picked: Option<PathBuf>,
    trigger: &TrszszTrigger,
) -> Result<TransferSummary, EngineError> {
    let Some(dir) = picked else {
        // 用户取消：confirm=false 让服务端静默退出（serverExit "Cancelled"）
        engine.send_action(false, trigger.win_server)?;
        return Err(EngineError::Cancelled);
    };
    if !dir.is_dir() {
        return Err(EngineError::Io(format!(
            "not a directory: {}",
            dir.display()
        )));
    }
    engine.send_action(true, trigger.win_server)?;
    engine.recv_config().await?;
    let names = engine.recv_files(&dir).await?;
    let msg = format_saved_files(&names, Some(&dir));
    engine.send_string("EXIT", &msg)?;
    Ok(TransferSummary {
        names,
        total_bytes: engine.received_bytes,
    })
}

async fn run_upload(
    engine: &mut Engine,
    picked: Option<Vec<PathBuf>>,
    trigger: &TrszszTrigger,
) -> Result<TransferSummary, EngineError> {
    // 模式 F：服务端指定客户端文件（trz -F client_files）——MVP 不支持，
    // confirm=false 让服务端静默退出
    if trigger.mode == b'F' {
        engine.send_action(false, trigger.win_server)?;
        return Err(EngineError::Protocol(
            "trz -F (server-specified client files) is not supported".into(),
        ));
    }
    let Some(paths) = picked.filter(|p| !p.is_empty()) else {
        engine.send_action(false, trigger.win_server)?;
        return Err(EngineError::Cancelled);
    };
    let entries = collect_upload_entries(&paths).map_err(EngineError::Io)?;
    if entries.is_empty() {
        engine.send_action(false, trigger.win_server)?;
        return Err(EngineError::Cancelled);
    }
    engine.send_action(true, trigger.win_server)?;
    engine.recv_config().await?;
    let names = engine.send_files(&entries).await?;
    let total: u64 = entries.iter().map(|e| e.size).sum();
    let msg = format_saved_files(&names, None);
    engine.send_string("EXIT", &msg)?;
    Ok(TransferSummary {
        names,
        total_bytes: total,
    })
}

#[cfg(test)]
#[allow(clippy::panic)] // 断言失败分支以 panic! 报告（restriction lint 经 [lints] deny，对测试同样生效）
mod tests {
    use super::*;
    use super::super::codec::md5_digest;
    use std::sync::Mutex;

    /// 测试用进度捕获 sink
    struct CaptureSink(Mutex<Vec<TrzszProgress>>);
    impl EventSink for CaptureSink {
        fn progress(&self, p: TrzszProgress) {
            self.0.lock().unwrap().push(p);
        }
    }

    fn sink() -> (Arc<dyn EventSink>, Arc<CaptureSink>) {
        let s = Arc::new(CaptureSink(Mutex::new(Vec::new())));
        (Arc::clone(&s) as Arc<dyn EventSink>, s)
    }

    fn trigger(mode: u8) -> TrszszTrigger {
        TrszszTrigger {
            mode,
            version: "1.2.0".into(),
            unique_id: "1730100000000".into(),
            win_server: false,
        }
    }

    fn new_engine(
        sink: Arc<dyn EventSink>,
    ) -> (
        Engine,
        UnboundedSender<Vec<u8>>,
        UnboundedReceiver<Vec<u8>>,
        Arc<AtomicBool>,
    ) {
        let (in_tx, in_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let (out_tx, out_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let cancel = Arc::new(AtomicBool::new(false));
        let engine = Engine::new(
            sink,
            "terminal-test".into(),
            "download",
            &trigger(b'S'),
            in_rx,
            out_tx,
            Arc::clone(&cancel),
        );
        (engine, in_tx, out_rx, cancel)
    }

    /// 服务端仿真：发一行文本模式帧（b64zlib 载荷）
    fn send_frame(tx: &UnboundedSender<Vec<u8>>, typ: &str, payload: &[u8]) {
        let line = format!("#{typ}:{}\n", encode_payload(payload));
        tx.send(line.into_bytes()).unwrap();
    }

    fn send_int_frame(tx: &UnboundedSender<Vec<u8>>, typ: &str, v: i64) {
        tx.send(format!("#{typ}:{v}\n").into_bytes()).unwrap();
    }

    async fn next_outbound(rx: &mut UnboundedReceiver<Vec<u8>>) -> String {
        tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("outbound within timeout")
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .expect("outbound channel open")
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    #[test]
    fn config_roundtrip_via_act_and_cfg() {
        rt().block_on(async {
            let (sink_arc, _captured) = sink();
            let (mut eng, srv_in, mut srv_out, _cancel) = new_engine(sink_arc);
            // 并行跑 recv_config，先喂 CFG
            let cfg_json = r#"{"lang":"go","quiet":false,"directory":true,"overwrite":false,"timeout":20,"bufsize":10485760,"tmux_output_junk":false}"#;
            srv_in
                .send(format!("#CFG:{}\n", encode_payload(cfg_json.as_bytes())).into_bytes())
                .unwrap();
            // 先发 ACT（同步）再验证 CFG 解析
            eng.send_action(true, false).unwrap();
            let cfg = eng.recv_config().await.unwrap();
            assert!(cfg.directory);
            assert_eq!(cfg.timeout, 20);
            // ACT 帧内容可解码且字段正确
            let act_line = next_outbound(&mut srv_out).await;
            assert!(act_line.starts_with("#ACT:"));
            let content = act_line.trim_end().strip_prefix("#ACT:").unwrap();
            let decoded: serde_json::Value =
                serde_json::from_slice(&decode_payload(content).unwrap()).unwrap();
            assert_eq!(decoded.get("confirm"), Some(&serde_json::Value::Bool(true)));
            assert_eq!(decoded.get("protocol"), Some(&serde_json::Value::from(1)));
            assert_eq!(decoded.get("binary"), Some(&serde_json::Value::Bool(false)));
            assert_eq!(decoded.get("support_dir"), Some(&serde_json::Value::Bool(true)));
            assert_eq!(decoded.get("newline"), Some(&serde_json::Value::from("\n")));
            // 引擎内部配置已生效
            assert!(eng.directory);
            assert_eq!(eng.timeout, Some(Duration::from_secs(20)));
        });
    }

    #[test]
    fn download_flow_end_to_end() {
        rt().block_on(async {
            let dir = std::env::temp_dir().join(format!(
                "trzsz-test-dl-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();

            let (sink_arc, _captured) = sink();
            let (mut eng, srv_in, mut srv_out, _cancel) = new_engine(sink_arc);
            eng.directory = true; // 模拟 CFG.directory=true

            let content = b"hello trzsz download payload".to_vec();
            let digest = md5_digest(&content);
            let saved_path = dir.join("proj").join("main.rs");
            let proj_dir = dir.join("proj");
            let dir_cleanup = dir.clone();

            let handle = tokio::spawn(async move {
                let names = eng.recv_files(&dir).await.unwrap();
                names
            });

            // 服务端序：NUM → (NAME dir)(NAME file)(SIZE)(DATA)(MD5)
            send_int_frame(&srv_in, "NUM", 2);
            let dir_entry = r#"{"path_id":0,"path_name":["proj"],"is_dir":true,"size":0}"#;
            send_frame(&srv_in, "NAME", dir_entry.as_bytes());
            let file_entry =
                r#"{"path_id":0,"path_name":["proj","main.rs"],"is_dir":false,"size":28}"#;
            send_frame(&srv_in, "NAME", file_entry.as_bytes());
            send_int_frame(&srv_in, "SIZE", 28);
            send_frame(&srv_in, "DATA", &content);
            send_frame(&srv_in, "MD5", &digest);
            drop(srv_in); // 关闭输入通道让 recv_files 若继续读会报错（正常应已完成）

            let names = tokio::time::timeout(Duration::from_secs(5), handle)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(names, vec!["proj".to_string()]);

            // 客户端应答序校验（通道关闭后循环自然退出）
            let mut replies = Vec::new();
            while let Ok(Some(line)) =
                tokio::time::timeout(Duration::from_millis(200), srv_out.recv()).await
            {
                replies.push(String::from_utf8_lossy(&line).into_owned());
            }
            assert_eq!(replies.first().unwrap(), "#SUCC:2\n");
            // 两条 NAME 各回一次本地名（目录条目与文件条目共用根名映射）
            let name_reply1 = decode_payload(replies.get(1).unwrap().trim_end().strip_prefix("#SUCC:").unwrap());
            let name_reply2 = decode_payload(replies.get(2).unwrap().trim_end().strip_prefix("#SUCC:").unwrap());
            assert_eq!(name_reply1.unwrap(), b"proj".to_vec());
            assert_eq!(name_reply2.unwrap(), b"proj".to_vec());
            assert_eq!(replies.get(3).unwrap(), "#SUCC:28\n");
            assert_eq!(replies.get(4).unwrap(), "#SUCC:28\n"); // DATA 长度应答
            let md5_ack = decode_payload(replies.get(5).unwrap().trim_end().strip_prefix("#SUCC:").unwrap());
            assert_eq!(md5_ack.unwrap(), digest.to_vec());

            // 本地文件落盘
            let saved = std::fs::read(&saved_path).unwrap();
            assert_eq!(saved, content);
            assert!(proj_dir.is_dir());
            std::fs::remove_dir_all(dir_cleanup).unwrap();
        });
    }

    #[test]
    fn upload_flow_end_to_end() {
        rt().block_on(async {
            let tmp = std::env::temp_dir().join(format!(
                "trzsz-test-up-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&tmp).unwrap();
            let file_path = tmp.join("note.txt");
            let content = b"upload payload 123".to_vec();
            std::fs::write(&file_path, &content).unwrap();

            let (sink_arc, _captured) = sink();
            let (mut eng, srv_in, mut srv_out, _cancel) = new_engine(sink_arc);
            eng.directory = false;

            let entries = collect_upload_entries(&[file_path.clone()]).unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries.first().unwrap().size, content.len() as u64);

            let handle = tokio::spawn(async move { eng.send_files(&entries).await.unwrap() });

            // 服务端序：NUM → NAME → SIZE → DATA → MD5，逐项回 SUCC
            let num_line = next_outbound(&mut srv_out).await;
            assert!(num_line.starts_with("#NUM:1"));
            srv_in.send(b"#SUCC:1\n".to_vec()).unwrap();
            let name_line = next_outbound(&mut srv_out).await;
            let name = decode_payload(name_line.trim_end().strip_prefix("#NAME:").unwrap()).unwrap();
            assert_eq!(name, b"note.txt".to_vec());
            srv_in
                .send(format!("#SUCC:{}\n", encode_payload(b"note.txt")).into_bytes())
                .unwrap();
            let size_line = next_outbound(&mut srv_out).await;
            assert_eq!(size_line, "#SIZE:18\n");
            srv_in.send(b"#SUCC:18\n".to_vec()).unwrap();
            let data_line = next_outbound(&mut srv_out).await;
            assert!(data_line.starts_with("#DATA:"));
            let data = decode_payload(data_line.trim_end().strip_prefix("#DATA:").unwrap()).unwrap();
            assert_eq!(data, content);
            srv_in.send(b"#SUCC:18\n".to_vec()).unwrap();
            let md5_line = next_outbound(&mut srv_out).await;
            let sent_digest =
                decode_payload(md5_line.trim_end().strip_prefix("#MD5:").unwrap()).unwrap();
            assert_eq!(sent_digest, md5_digest(&content).to_vec());
            srv_in
                .send(format!("#SUCC:{}\n", encode_payload(&md5_digest(&content))).into_bytes())
                .unwrap();

            let remote_names =
                tokio::time::timeout(Duration::from_secs(5), handle).await.unwrap().unwrap();
            assert_eq!(remote_names, vec!["note.txt".to_string()]);
            std::fs::remove_dir_all(&tmp).unwrap();
        });
    }

    #[test]
    fn remote_fail_frame_surfaces_as_error() {
        rt().block_on(async {
            let (sink_arc, _c) = sink();
            let (mut eng, srv_in, _out, _cancel) = new_engine(sink_arc);
            let handle = tokio::spawn(async move { eng.recv_check("CFG", true).await });
            // 期望 CFG 却收到 fail（载荷为编码后的错误消息）
            srv_in
                .send(format!("#fail:{}\n", encode_payload(b"server disk full")).into_bytes())
                .unwrap();
            let err = tokio::time::timeout(Duration::from_secs(2), handle)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            match err {
                EngineError::RemoteFail(msg) => assert_eq!(msg, "server disk full"),
                other => panic!("expected RemoteFail, got {other:?}"),
            }
        });
    }

    #[test]
    fn cancel_flag_stops_engine() {
        rt().block_on(async {
            let (sink_arc, _c) = sink();
            let (mut eng, _srv_in, _out, cancel) = new_engine(sink_arc);
            cancel.store(true, Ordering::Relaxed);
            let err = eng.recv_check("CFG", true).await.unwrap_err();
            assert!(matches!(err, EngineError::Cancelled));
        });
    }

    #[test]
    fn junk_mode_cfg_read_survives_tmux_noise() {
        rt().block_on(async {
            let (sink_arc, _c) = sink();
            let (mut eng, srv_in, mut srv_out, _cancel) = new_engine(sink_arc);
            // tmux 噪声：成对 DCS 状态行包住前置垃圾 + 折行（\r\n 折叠标记），
            // 尾部悬空 DCS 被 strip 截断
            let cfg_json = r#"{"lang":"go","timeout":20}"#;
            let encoded = encode_payload(cfg_json.as_bytes());
            let (head, tail) = encoded.split_at(encoded.len() / 2);
            let noisy = format!(
                "\x1bP=1\x1b\\junk\x1bP=2\x1b\\#CFG:{head}\r\n{tail}\x1bP=3\n"
            );
            srv_in.send(noisy.into_bytes()).unwrap();
            eng.send_action(true, false).unwrap();
            eng.recv_config().await.unwrap();
            let _ = next_outbound(&mut srv_out).await; // ACT 行
        });
    }

    #[test]
    fn get_new_name_and_safe_segment() {
        let dir = std::env::temp_dir().join(format!(
            "trzsz-test-name-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        assert_eq!(get_new_name(&dir, "a.txt", false).unwrap(), "a.txt.0");
        assert_eq!(get_new_name(&dir, "a.txt", true).unwrap(), "a.txt");
        assert_eq!(get_new_name(&dir, "b.txt", false).unwrap(), "b.txt");
        std::fs::remove_dir_all(&dir).unwrap();

        assert!(safe_segment("../etc").is_err());
        assert!(safe_segment("a/b").is_err());
        assert!(safe_segment("").is_err());
        assert!(safe_segment("..").is_err());
        assert!(safe_segment("正常文件.txt").is_ok());
    }

    #[test]
    fn saved_files_format() {
        let s = format_saved_files(&["a.zip".into(), "b/".into()], Some(Path::new("/tmp")));
        assert!(s.starts_with("Saved 2 files/directories to /tmp"));
        assert!(s.contains("\r\n- a.zip"));
        assert!(s.contains("\r\n- b/"));
        let single = format_saved_files(&["x".into()], None);
        assert_eq!(single, "Saved 1 file/directory\r\n- x");
    }

    #[test]
    fn collect_entries_walks_tree_sorted() {
        let tmp = std::env::temp_dir().join(format!(
            "trzsz-test-walk-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(tmp.join("root").join("sub")).unwrap();
        std::fs::write(tmp.join("root").join("b.txt"), b"bb").unwrap();
        std::fs::write(tmp.join("root").join("sub").join("a.txt"), b"aaa").unwrap();
        let entries = collect_upload_entries(&[tmp.join("root")]).unwrap();
        let rels: Vec<String> = entries
            .iter()
            .map(|e| e.rel.join("/"))
            .collect();
        assert_eq!(
            rels,
            vec!["root".to_string(), "root/b.txt".to_string(), "root/sub".to_string(), "root/sub/a.txt".to_string()]
        );
        let file_entry = entries.iter().find(|e| !e.is_dir).unwrap();
        assert_eq!(file_entry.size, 2);
        assert!(entries
            .iter()
            .find(|e| e.rel.join("/") == "root/sub")
            .unwrap()
            .is_dir);
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
