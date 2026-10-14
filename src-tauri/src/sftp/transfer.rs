//! SFTP 传输核心：读写流水线泵流、限速、进度节流、断点续传偏移解析与失效会话剔除决策。
//!
//! 三条传输命令（download / upload / copy）共用 [`pump_stream`]：
//! 生产者任务按块读取（512KB/块）推入有界通道（12 块 ≈ 6MB 在途上限），
//! 消费侧写入远端/本地并限速、回报进度；两端速度差由通道容量天然背压吸收，
//! 无论链路多不对称，内存占用恒定在 ~6MB 量级。

use std::io;

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

/// 单块读取缓冲大小：一次 `write_all` 可填满 16×32KB 的 SFTP 写流水线
pub const TRANSFER_CHUNK_SIZE: usize = 512 * 1024;
/// 有界通道容量（块数）：在途数据上限 ≈ 512KB × 12 = 6MB，背压由此产生
pub const TRANSFER_CHANNEL_CAPACITY: usize = 12;
/// 每条 SFTP 连接的并发在途写请求数（russh-sftp 默认 8，高延迟链路提高吞吐）
pub const SFTP_MAX_CONCURRENT_WRITES: usize = 32;

/// 简单令牌桶限速器：按累计字节与预期耗时对比，超前则睡眠补齐。
/// limit_bps 为 0/None 表示不限速；允许 1 秒级别的突发。
pub struct SpeedLimiter {
  limit_bps: u64,
  window_start: tokio::time::Instant,
  window_bytes: u64,
}

impl SpeedLimiter {
  pub fn new(limit_bps: Option<u64>) -> Option<Self> {
    let limit_bps = limit_bps.filter(|v| *v > 0)?;
    Some(Self {
      limit_bps,
      window_start: tokio::time::Instant::now(),
      window_bytes: 0,
    })
  }

  pub async fn throttle(&mut self, chunk_bytes: u64) {
    self.window_bytes += chunk_bytes;
    let expected = tokio::time::Duration::from_secs_f64(
      self.window_bytes as f64 / self.limit_bps as f64,
    );
    let elapsed = self.window_start.elapsed();
    if expected > elapsed {
      tokio::time::sleep(expected - elapsed).await;
    } else if elapsed > expected + tokio::time::Duration::from_secs(1) {
      // 窗口内已落后于配额（低速读源），重置窗口避免长期欠账后限死
      self.window_start = tokio::time::Instant::now();
      self.window_bytes = 0;
    }
  }
}

/// 进度上报节流器：≥100ms 才产生一次上报包，同时计算瞬时速度与百分比。
pub struct ProgressTracker {
  last_tick: tokio::time::Instant,
  bytes_since_tick: u64,
}

impl Default for ProgressTracker {
  fn default() -> Self {
    Self::new()
  }
}

impl ProgressTracker {
  pub fn new() -> Self {
    Self {
      last_tick: tokio::time::Instant::now(),
      bytes_since_tick: 0,
    }
  }

  /// 累计一段新传输字节；距上次上报 ≥100ms 时返回 `(speed_bps, percent)` 供调用方发包。
  pub fn on_bytes(
    &mut self,
    now: tokio::time::Instant,
    delta_bytes: u64,
    transferred_abs: u64,
    total_size: u64,
  ) -> Option<(u64, f32)> {
    self.bytes_since_tick += delta_bytes;
    let elapsed = now.duration_since(self.last_tick);
    if elapsed < tokio::time::Duration::from_millis(100) {
      return None;
    }
    let speed_bps = (self.bytes_since_tick as f64 / elapsed.as_secs_f64()) as u64;
    let percent = if total_size > 0 {
      (transferred_abs as f64 / total_size as f64 * 100.0).clamp(0.0, 100.0) as f32
    } else {
      0.0
    };
    self.last_tick = now;
    self.bytes_since_tick = 0;
    Some((speed_bps, percent))
  }
}

/// 断点续传偏移解析：取「前端上报值」与「目标侧现存长度」的较小值。
///
/// 取 min 的原因：上报值可能落后于服务端实际已写长度（存在未确认的在途写包），
/// 从较小值续写会对 [min, max) 区间做幂等覆盖，杜绝乱序写留下的空洞；
/// 现存长度为 0 / 不存在 / 未请求续传时从头开始。
pub fn resolve_resume_offset(requested: Option<u64>, existing_len: Option<u64>) -> u64 {
  let Some(requested) = requested.filter(|v| *v > 0) else {
    return 0;
  };
  match existing_len {
    Some(len) if len > 0 => requested.min(len),
    _ => 0,
  }
}

/// 传输临时断点文件名统一前缀（与 `file_ops::sibling_temp_path` 生成的名字一致）
pub const TEMP_FILE_PREFIX: &str = ".veloterm.tmp.";
/// 遗留孤儿断点的最大保留时长：超过 1 天视为用户已放弃重试，重连时清理
pub const TEMP_FILE_MAX_AGE_SECS: u64 = 24 * 60 * 60;

/// 孤儿断点判定：名字带统一前缀且修改时间距今 ≥ 1 天。
/// mtime 缺失（服务器未返回）时保守不删，避免误删刚创建的断点。
pub fn is_stale_temp_entry(name: &str, mtime_secs: Option<u64>, now_secs: u64) -> bool {
  if !name.starts_with(TEMP_FILE_PREFIX) {
    return false;
  }
  match mtime_secs {
    Some(mtime) => now_secs.saturating_sub(mtime) >= TEMP_FILE_MAX_AGE_SECS,
    None => false,
  }
}

/// 扫描并清理指定目录集合中超龄的 `.veloterm.tmp.*` 孤儿断点（尽力而为，单目录失败忽略）。
/// 返回成功删除的文件数（用于日志观测）。
pub async fn cleanup_stale_temp_files(client: &super::types::SftpClient, dirs: &[String]) -> usize {
  let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0);
  let mut removed = 0usize;
  for dir in dirs {
    let entries = match client.read_dir(dir).await {
      Ok(entries) => entries,
      Err(_) => continue, // 目录不存在/无权限：正常情况（断点已随 rename 消失）
    };
    for entry in entries {
      let name = entry.file_name();
      let mtime = entry.metadata().mtime.map(u64::from);
      if !is_stale_temp_entry(&name, mtime, now) {
        continue;
      }
      let path = if dir == "/" {
        format!("/{name}")
      } else {
        format!("{dir}/{name}")
      };
      if client.remove_file(&path).await.is_ok() {
        removed += 1;
      }
    }
  }
  removed
}

/// 传输命令种类与失败侧的远端剔除决策：
/// 读侧/写侧哪一端是远端连接，失败时才需要把对应会话从缓存剔除以便重连。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
  Download,
  Upload,
  Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureSide {
  /// 读侧失败（源数据读取中断）
  Source,
  /// 写侧失败（目标写入中断）
  Target,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteEndpoint {
  Source,
  Target,
}

/// 返回该失败下应从缓存剔除的远端会话端；`None` 表示失败发生在本地磁盘，无需剔除。
pub fn evict_decision(kind: TransferKind, side: FailureSide) -> Option<RemoteEndpoint> {
  match (kind, side) {
    // 下载：远端读 + 本地写
    (TransferKind::Download, FailureSide::Source) => Some(RemoteEndpoint::Source),
    (TransferKind::Download, FailureSide::Target) => None,
    // 上传：本地读 + 远端写
    (TransferKind::Upload, FailureSide::Source) => None,
    (TransferKind::Upload, FailureSide::Target) => Some(RemoteEndpoint::Target),
    // 远端复制：两端都是远端
    (TransferKind::Copy, FailureSide::Source) => Some(RemoteEndpoint::Source),
    (TransferKind::Copy, FailureSide::Target) => Some(RemoteEndpoint::Target),
  }
}

/// 泵流错误：区分读侧/写侧与用户取消，供上层做会话剔除与断点保留决策。
#[derive(Debug)]
pub enum PumpError {
  Cancelled,
  Read(io::Error),
  Write(io::Error),
}

pub struct PumpOptions {
  pub chunk_size: usize,
  pub channel_capacity: usize,
  pub limit_bps: Option<u64>,
}

impl Default for PumpOptions {
  fn default() -> Self {
    Self {
      chunk_size: TRANSFER_CHUNK_SIZE,
      channel_capacity: TRANSFER_CHANNEL_CAPACITY,
      limit_bps: None,
    }
  }
}

/// 读写并行泵流：生产者任务持续填块读入有界通道，消费侧写块 + 限速 + 逐块回调。
///
/// * 两端速度差由通道容量背压吸收（写慢 → 通道满 → 生产者停读），内存恒定有界；
/// * `on_chunk(delta)` 在每块**写成功后**回调（新增字节数），保证上报值 ≤ 实际已提交；
/// * 正常结束返回本次新增字节数（不含调用方预 seek 的续传偏移），并 `shutdown` 写侧；
/// * 取消/错误时中止生产者，写侧已写部分即为可续传断点。
pub async fn pump_stream<R, W, F>(
  reader: R,
  mut writer: W,
  cancel_rx: oneshot::Receiver<()>,
  mut on_chunk: F,
  opts: PumpOptions,
) -> Result<u64, PumpError>
where
  R: AsyncRead + Unpin + Send + 'static,
  W: AsyncWrite + Unpin + Send + 'static,
  F: FnMut(u64) + Send + 'static,
{
  let (tx, mut rx) = mpsc::channel::<Bytes>(opts.channel_capacity.max(1));
  let chunk_size = opts.chunk_size.max(1);

  let producer = tokio::spawn(async move {
    let mut reader = reader;
    let mut buf = BytesMut::with_capacity(chunk_size);
    loop {
      buf.reserve(chunk_size);
      loop {
        if buf.len() >= chunk_size {
          break;
        }
        let n = reader.read_buf(&mut buf).await?;
        if n == 0 {
          break;
        }
      }
      if buf.is_empty() {
        return Ok(());
      }
      let chunk = buf.split().freeze();
      // 消费侧提前退出（写失败/取消）时发送失败属正常，按 EOF 收场
      if tx.send(chunk).await.is_err() {
        return Ok(());
      }
    }
  });

  let mut transferred: u64 = 0;
  let mut limiter = SpeedLimiter::new(opts.limit_bps);
  let mut cancel_rx = cancel_rx;

  let result: Result<(), PumpError> = async {
    loop {
      tokio::select! {
        _ = &mut cancel_rx => {
          return Err(PumpError::Cancelled);
        }
        maybe = rx.recv() => {
          let Some(chunk) = maybe else {
            break;
          };
          writer.write_all(&chunk).await.map_err(PumpError::Write)?;
          if let Some(ref mut limiter) = limiter {
            limiter.throttle(chunk.len() as u64).await;
          }
          transferred += chunk.len() as u64;
          on_chunk(chunk.len() as u64);
        }
      }
    }
    Ok(())
  }
  .await;

  match result {
    Ok(()) => {
      let producer_result = producer
        .await
        .map_err(|e| io::Error::other(format!("producer task failed: {e}")))
        .and_then(|r| r);
      match producer_result {
        Ok(()) => {
          writer.shutdown().await.map_err(PumpError::Write)?;
          Ok(transferred)
        }
        Err(e) => Err(PumpError::Read(e)),
      }
    }
    Err(e) => {
      producer.abort();
      Err(e)
    }
  }
}

/// 读侧故障注入 Reader：正常吐出 `prefix` 数据后返回错误。
#[cfg(test)]
struct FailingReader {
  prefix: Bytes,
}

#[cfg(test)]
impl AsyncRead for FailingReader {
  fn poll_read(
    mut self: std::pin::Pin<&mut Self>,
    _cx: &mut std::task::Context<'_>,
    buf: &mut tokio::io::ReadBuf<'_>,
  ) -> std::task::Poll<std::io::Result<()>> {
    if self.prefix.is_empty() {
      return std::task::Poll::Ready(Err(io::Error::other("reader-side boom")));
    }
    let take = buf.remaining().min(self.prefix.len());
    let chunk = self.prefix.split_to(take);
    buf.put_slice(&chunk);
    std::task::Poll::Ready(Ok(()))
  }
}

/// 写侧故障注入 Writer：前 `ok_bytes` 字节正常吞掉，之后返回错误。
#[cfg(test)]
struct FailingWriter {
  ok_bytes: usize,
}

#[cfg(test)]
impl AsyncWrite for FailingWriter {
  fn poll_write(
    mut self: std::pin::Pin<&mut Self>,
    _cx: &mut std::task::Context<'_>,
    buf: &[u8],
  ) -> std::task::Poll<std::io::Result<usize>> {
    let take = buf.len().min(self.ok_bytes);
    if take == 0 {
      return std::task::Poll::Ready(Err(io::Error::other("writer-side boom")));
    }
    self.ok_bytes -= take;
    std::task::Poll::Ready(Ok(take))
  }

  fn poll_flush(
    self: std::pin::Pin<&mut Self>,
    _cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<std::io::Result<()>> {
    std::task::Poll::Ready(Ok(()))
  }

  fn poll_shutdown(
    self: std::pin::Pin<&mut Self>,
    _cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<std::io::Result<()>> {
    std::task::Poll::Ready(Ok(()))
  }
}

/// 拥有所有权的 Vec 写入端（测试用）：满足 `AsyncWrite + 'static` 约束
#[cfg(test)]
struct VecSink {
  inner: Vec<u8>,
}

#[cfg(test)]
impl AsyncWrite for VecSink {
  fn poll_write(
    mut self: std::pin::Pin<&mut Self>,
    _cx: &mut std::task::Context<'_>,
    buf: &[u8],
  ) -> std::task::Poll<std::io::Result<usize>> {
    self.inner.extend_from_slice(buf);
    std::task::Poll::Ready(Ok(buf.len()))
  }

  fn poll_flush(
    self: std::pin::Pin<&mut Self>,
    _cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<std::io::Result<()>> {
    std::task::Poll::Ready(Ok(()))
  }

  fn poll_shutdown(
    self: std::pin::Pin<&mut Self>,
    _cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<std::io::Result<()>> {
    std::task::Poll::Ready(Ok(()))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::time::Duration;
  use tokio::io::duplex;

  fn sample_data(size: usize) -> Vec<u8> {
    // 简单可校验的伪随机序列（LCG），避免全零数据掩盖错位问题
    let mut data = Vec::with_capacity(size);
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    for _ in 0..size {
      state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
      data.push((state >> 33) as u8);
    }
    data
  }

  /// 把 duplex 对端读到的全部字节聚合为单个 Vec（读到 EOF 返回）
  async fn drain_duplex(mut server: tokio::io::DuplexStream) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
      match server.read(&mut buf).await {
        Ok(0) => return Ok(out),
        Ok(n) => {
          if let Some(chunk) = buf.get(..n) {
            out.extend_from_slice(chunk);
          }
        }
        Err(e) => return Err(e),
      }
    }
  }

  #[tokio::test]
  async fn pump_copies_full_content_with_small_channel() {
    // 小通道 + 小块强制多次背压往返，验证分块与顺序完整性
    let data = sample_data(300 * 1024);
    let (client, server) = duplex(8 * 1024);
    let collector = tokio::spawn(drain_duplex(server));

    let (_cancel_tx, cancel_rx) = oneshot::channel::<()>();
    let callback_total = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counter = std::sync::Arc::clone(&callback_total);
    let copied = pump_stream(
      std::io::Cursor::new(data.clone()),
      client,
      cancel_rx,
      move |delta| {
        counter.fetch_add(delta, std::sync::atomic::Ordering::Relaxed);
      },
      PumpOptions {
        chunk_size: 17 * 1024, // 故意非整除，覆盖短块路径
        channel_capacity: 3,
        limit_bps: None,
      },
    )
    .await
    .expect("pump should succeed");

    let collected = collector
      .await
      .expect("collector should finish")
      .expect("drain should be ok");

    assert_eq!(copied, data.len() as u64);
    assert_eq!(callback_total.load(std::sync::atomic::Ordering::Relaxed), copied);
    assert_eq!(collected, data);
  }

  #[tokio::test]
  async fn pump_returns_cancelled_and_keeps_partial_writes() {
    // duplex 较小缓冲模拟慢消费；累计写满一定量后触发取消
    let data = sample_data(4 * 1024 * 1024);
    let (client, mut server) = duplex(64 * 1024);

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    let trigger =
      std::sync::Arc::new(std::sync::Mutex::new(Some(cancel_tx)));
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let trigger_in = std::sync::Arc::clone(&trigger);
    let seen_in = std::sync::Arc::clone(&seen);
    let reader = std::io::Cursor::new(data.clone());

    let pump = tokio::spawn(async move {
      pump_stream(
        reader,
        client,
        cancel_rx,
        move |delta| {
          if seen_in.fetch_add(delta, std::sync::atomic::Ordering::Relaxed) + delta
            >= 512 * 1024
            && let Ok(mut guard) = trigger_in.lock()
            && let Some(tx) = guard.take()
          {
            let _ = tx.send(());
          }
        },
        PumpOptions::default(),
      )
      .await
    });

    // 持续消费 duplex 对端；泵结束后排空余量并读到 EOF
    use tokio::io::AsyncReadExt;
    let mut sink = Vec::<u8>::new();
    let mut rbuf = vec![0u8; 32 * 1024];
    loop {
      if pump.is_finished() {
        while let Ok(n) = server.read(&mut rbuf).await {
          if n == 0 {
            break;
          }
          if let Some(chunk) = rbuf.get(..n) {
            sink.extend_from_slice(chunk);
          }
        }
        break;
      }
      match server.read(&mut rbuf).await {
        Ok(0) => break,
        Ok(n) => {
          if let Some(chunk) = rbuf.get(..n) {
            sink.extend_from_slice(chunk);
          }
        }
        Err(_) => break,
      }
    }

    let result = pump.await.expect("pump task panicked");
    assert!(matches!(result, Err(PumpError::Cancelled)));
    // 已写部分应为正且小于全量（断点保留语义的输入）
    assert!(!sink.is_empty());
    assert!(sink.len() < 4 * 1024 * 1024);
    // 部分数据必须是源数据的前缀（顺序未被破坏）
    let expected = data.get(..sink.len()).unwrap_or(data.as_slice());
    assert_eq!(sink.as_slice(), expected);
  }

  #[tokio::test]
  async fn pump_maps_reader_error_to_read_side() {
    let (_cancel_tx, cancel_rx) = oneshot::channel::<()>();
    let failing = FailingReader {
      prefix: Bytes::from_static(b"some leading bytes"),
    };
    let err = pump_stream(
      failing,
      VecSink { inner: Vec::new() },
      cancel_rx,
      |_| {},
      PumpOptions::default(),
    )
    .await
    .expect_err("should fail");
    assert!(matches!(err, PumpError::Read(_)));
  }

  #[tokio::test]
  async fn pump_maps_writer_error_to_write_side() {
    let (_cancel_tx, cancel_rx) = oneshot::channel::<()>();
    let err = pump_stream(
      std::io::Cursor::new(sample_data(64 * 1024)),
      FailingWriter { ok_bytes: 1024 },
      cancel_rx,
      |_| {},
      PumpOptions::default(),
    )
    .await
    .expect_err("should fail");
    assert!(matches!(err, PumpError::Write(_)));
  }

  #[tokio::test(start_paused = true)]
  async fn progress_tracker_throttles_to_100ms() {
    let mut tracker = ProgressTracker::new();
    let t0 = tokio::time::Instant::now();

    // 首个 100ms 内不报
    assert!(
      tracker
        .on_bytes(t0, 1000, 1000, 10_000)
        .is_none()
    );

    // 推进 150ms 后应报：窗口字节 = 未报的 1000 + 本轮 1000 = 2000B / 0.15s ≈ 13333B/s
    tokio::time::advance(Duration::from_millis(150)).await;
    let t1 = t0 + Duration::from_millis(150);
    let (speed, percent) = tracker
      .on_bytes(t1, 1000, 2000, 10_000)
      .expect("should report after 100ms");
    assert!((speed as i64 - 13333).abs() < 100, "speed {speed}");
    assert!((percent - 20.0).abs() < 0.01);

    // 刚报完立即再来一笔不应再报
    assert!(tracker.on_bytes(t1 + Duration::from_millis(1), 500, 2500, 10_000).is_none());
  }

  #[test]
  fn resolve_resume_offset_uses_min_policy() {
    // 未请求续传 → 从头
    assert_eq!(resolve_resume_offset(None, Some(1024)), 0);
    // 请求 0 → 从头
    assert_eq!(resolve_resume_offset(Some(0), Some(1024)), 0);
    // 目标不存在/长度 0 → 从头
    assert_eq!(resolve_resume_offset(Some(1024), None), 0);
    assert_eq!(resolve_resume_offset(Some(1024), Some(0)), 0);
    // 上报值落后于现存长度（在途写已落盘）→ 取较小值（覆盖重写防空洞）
    assert_eq!(resolve_resume_offset(Some(1000), Some(1200)), 1000);
    // 上报值超前于现存长度（用户删过/写失败回退）→ 取较小值
    assert_eq!(resolve_resume_offset(Some(2000), Some(1200)), 1200);
    // 相等 → 原值
    assert_eq!(resolve_resume_offset(Some(1200), Some(1200)), 1200);
  }

  #[test]
  fn stale_temp_entry_detection() {
    let now = 1_000_000u64;
    // 前缀 + 恰好 1 天 → 清理
    assert!(is_stale_temp_entry(
      ".veloterm.tmp.abc",
      Some(now - TEMP_FILE_MAX_AGE_SECS),
      now
    ));
    // 前缀 + 超过 1 天 → 清理（含时钟回拨场景 saturating_sub 为 0 不误删）
    assert!(is_stale_temp_entry(".veloterm.tmp.abc", Some(now - 10 * 86_400), now));
    // 前缀 + 23 小时 → 保留（活跃断点）
    assert!(!is_stale_temp_entry(".veloterm.tmp.abc", Some(now - 86_400 + 3_600), now));
    // 前缀 + mtime 缺失 → 保守保留
    assert!(!is_stale_temp_entry(".veloterm.tmp.abc", None, now));
    // 非断点文件：无论多旧都不动
    assert!(!is_stale_temp_entry("notes.txt", Some(0), now));
    assert!(!is_stale_temp_entry("veloterm.tmp.abc", Some(0), now));
    assert!(!is_stale_temp_entry(".veloterm.tmp", Some(0), now));
    // 时钟早于 mtime（服务器时钟偏移）→ saturating 为 0，不删
    assert!(!is_stale_temp_entry(
      ".veloterm.tmp.abc",
      Some(now + 60),
      now
    ));
  }

  #[test]
  fn evict_decision_maps_sides_per_kind() {
    use super::{FailureSide, RemoteEndpoint, TransferKind};

    // 下载：远端读 + 本地写
    assert_eq!(
      evict_decision(TransferKind::Download, FailureSide::Source),
      Some(RemoteEndpoint::Source)
    );
    assert_eq!(
      evict_decision(TransferKind::Download, FailureSide::Target),
      None
    );
    // 上传：本地读 + 远端写
    assert_eq!(
      evict_decision(TransferKind::Upload, FailureSide::Source),
      None
    );
    assert_eq!(
      evict_decision(TransferKind::Upload, FailureSide::Target),
      Some(RemoteEndpoint::Target)
    );
    // 远端复制：两端均远端
    assert_eq!(
      evict_decision(TransferKind::Copy, FailureSide::Source),
      Some(RemoteEndpoint::Source)
    );
    assert_eq!(
      evict_decision(TransferKind::Copy, FailureSide::Target),
      Some(RemoteEndpoint::Target)
    );
  }

  #[tokio::test(start_paused = true)]
  async fn speed_limiter_none_when_disabled_and_throttles_when_set() {
    assert!(SpeedLimiter::new(None).is_none());
    assert!(SpeedLimiter::new(Some(0)).is_none());

    let mut limiter = SpeedLimiter::new(Some(1000)).expect("limiter");
    // 1000B @ 1000B/s → 期望 1s：paused 时间下 sleep 会自动推进
    limiter.throttle(1000).await;
    // 推进后窗口时间已达标（不 panic、状态一致即通过）
    limiter.throttle(1).await;
  }

  /// 静态断言：FailingReader prefix 分支确实能吐数据（供 read 侧测试信赖）
  #[tokio::test]
  async fn failing_reader_emits_prefix_before_error() {
    use tokio::io::AsyncReadExt;
    let mut r = FailingReader {
      prefix: Bytes::from_static(b"abc"),
    };
    let mut out = Vec::new();
    let mut tmp = [0u8; 8];
    loop {
      match r.read(&mut tmp).await {
        Ok(0) => break,
        Ok(n) => {
          if let Some(chunk) = tmp.get(..n) {
            out.extend_from_slice(chunk);
          }
        }
        Err(_) => break,
      }
    }
    assert_eq!(out, b"abc");
  }
}
