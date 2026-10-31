//! 流水线下载：resolver 每在线确立一个 winner 立即投递（mpsc），
//! 下载端 JoinSet + Semaphore 限流边收边下载，与 POM 解析并行。
//! 每个文件下载开始/完成各输出一行日志（带时间戳，完成绿色、失败红色）。
//! 已存在文件跳过（release 视为不可变）；SNAPSHOT 已存在默认也跳过，`-U` 强刷。

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio::task::JoinSet;

use crate::maven::log;
use crate::maven::model::{packaging_ext, Coordinate};
use crate::maven::repo::RepoSet;
use crate::maven::version::is_snapshot;

use super::resolver::CancelFlag;

#[derive(Clone, Debug)]
pub struct DownloadOpts {
    /// 并行下载任务数（`-j`）
    pub threads: usize,
    /// `-U`：SNAPSHOT 强制重新拉取
    pub update_snapshots: bool,
    /// `--all-classifiers`：主件后尝试各平台 native classifier
    pub all_classifiers: bool,
    /// `--sources`：额外下载源码 jar
    pub sources: bool,
}

#[derive(Default, Debug)]
pub struct Summary {
    pub downloaded: usize,
    pub skipped: usize,
    pub failed: Vec<(String, String)>,
    /// 全部下载任务总耗时（高精度 Instant）
    pub elapsed: Option<Duration>,
    /// 用户主动取消（已落盘文件保留，幂等可续传）
    pub cancelled: bool,
}

/// 单任务输出：Ok(新下载?, 耗时) / Err((显示名, 错误), 耗时)
type TaskOut = std::result::Result<(bool, Duration), ((String, String), Duration)>;

/// 启动下载流水线：返回投递端与汇总接收端。
/// 投递端 drop（解析完成）后，剩余任务排空并经 oneshot 回传 Summary。
pub fn start(
    repos: Arc<RepoSet>,
    opts: DownloadOpts,
    cancel: Option<CancelFlag>,
) -> (mpsc::Sender<Coordinate>, oneshot::Receiver<Summary>) {
    let (tx, mut rx) = mpsc::channel::<Coordinate>(1024);
    let (done_tx, done_rx) = oneshot::channel::<Summary>();
    tokio::spawn(async move {
        let sem = Arc::new(Semaphore::new(opts.threads.max(1)));
        let started_all = Instant::now();
        let mut summary = Summary::default();
        let mut set: JoinSet<TaskOut> = JoinSet::new();

        loop {
            tokio::select! {
                // biased：优先收任务保持流水线满载
                biased;
                maybe = rx.recv() => match maybe {
                    Some(coord) => {
                        log::DL_QUEUED.fetch_sub(1, Ordering::Relaxed);
                        // 达到并发上限时在此等待 permit，天然限制 spawn 数量
                        let permit = match sem.clone().acquire_owned().await {
                            Ok(p) => p,
                            Err(_) => break,
                        };
                        let repos = repos.clone();
                        let update = opts.update_snapshots;
                        let all_cls = opts.all_classifiers;
                        let want_sources = opts.sources;
                        set.spawn(async move {
                            let _permit = permit;
                            let started = Instant::now();
                            let r =
                                download_artifact_opts(&coord, &repos, update, all_cls, want_sources).await;
                            let took = started.elapsed();
                            // 底部状态行计数（含 skipped：文件已在库中视为就绪）
                            match &r {
                                Err(_) => {
                                    log::DL_FAILED.fetch_add(1, Ordering::Relaxed);
                                }
                                _ => {
                                    log::DL_DONE.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            log::refresh_status();
                            match &r {
                                Ok(true) => {}
                                Ok(false) => log::verbose(format!(
                                    "exists, skipped {} ({})",
                                    rel_path(&coord),
                                    log::fmt_duration(took)
                                )),
                                Err((name, e)) => log::error(format!("Failed {name}: {e}")),
                            }
                            r.map(|ok| (ok, took))
                                .map_err(|(name, e)| ((name, e), took))
                        });
                    }
                    None => break, // 解析端完成：进入排空阶段
                },
                // 取消轮询：200ms 粒度检查旗标，命中即停止收新任务（在途任务排空完成）
                _ = tokio::time::sleep(Duration::from_millis(200)), if cancel.is_some() => {
                    if let Some(flag) = cancel.as_ref() {
                        if flag.load(Ordering::Relaxed) {
                            summary.cancelled = true;
                            break;
                        }
                    }
                }
                joined = set.join_next(), if !set.is_empty() => {
                    // 任务 panic 经 JoinError 收录为失败项（不再 expect——release 的
                    // panic=abort 下任何二次 panic 都会击杀整个应用）
                    match joined {
                        Some(Ok(Ok((true, _)))) => summary.downloaded += 1,
                        Some(Ok(Ok((false, _)))) => summary.skipped += 1,
                        Some(Ok(Err(_))) => {} // 失败已在任务内即时输出
                        Some(Err(e)) => {
                            summary.failed.push((format!("{e}"), "task panicked".into()))
                        }
                        None => {} // guard 保证非空，防御分支
                    }
                }
            }
        }
        // 排空剩余任务
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(Ok((true, _))) => summary.downloaded += 1,
                Ok(Ok((false, _))) => summary.skipped += 1,
                Ok(Err(((name, e), took))) => summary.failed.push((
                    name,
                    format!("{e} (after {})", log::fmt_duration(took)),
                )),
                Err(e) => summary.failed.push((format!("{e}"), "task panicked".into())),
            }
        }
        summary.elapsed = Some(started_all.elapsed());
        let _ = done_tx.send(summary);
    });
    (tx, done_rx)
}

/// 仓库内相对路径（日志显示用）。
fn rel_path(coord: &Coordinate) -> String {
    let ext = packaging_ext(&coord.packaging);
    let cls = coord.classifier.as_deref().map(|c| format!("-{c}")).unwrap_or_default();
    format!(
        "{}/{}/{}/{}-{}{cls}.{ext}",
        coord.group.replace('.', "/"),
        coord.artifact,
        coord.version,
        coord.artifact,
        coord.version
    )
}

/// 单个 artifact 下载；Ok(true)=新下载，Ok(false)=已存在跳过。
/// SNAPSHOT 已存在默认跳过；`-U`（update）强制重新拉取覆盖。
/// 开始/完成各打一行日志。Err 携带（显示名, 错误）。
async fn download_artifact_opts(
    coord: &Coordinate,
    repos: &RepoSet,
    update: bool,
    opts_all_classifiers: bool,
    opts_sources: bool,
) -> std::result::Result<bool, (String, String)> {
    let main_result = download_single(coord, repos, update).await?;
    // --all-classifiers：各平台 native；--sources：源码 jar（可叠加）。
    // 固定集合逐个尝试：存在即下、404 静默跳过（预期行为）
    let mut targets: Vec<&str> = Vec::new();
    if opts_all_classifiers {
        targets.extend([
            "linux-x86_64",
            "linux-aarch_64",
            "osx-x86_64",
            "osx-aarch_64",
            "windows-x86_64",
            "windows-aarch_64",
        ]);
    }
    if opts_sources {
        targets.push("sources");
    }
    for cls in targets {
        if coord.classifier.as_deref() == Some(cls) {
            continue;
        }
        let mut c2 = coord.clone();
        c2.classifier = Some(cls.to_string());
        match download_single(&c2, repos, update).await {
            Ok(_) => {}
            Err((_, e)) if e.contains("404") => {}
            Err((name, e)) => log::warn(format!("classifier {name}: {e}")),
        }
    }
    Ok(main_result)
}

/// 下载单个文件（主件或指定 classifier）
async fn download_single(
    coord: &Coordinate,
    repos: &RepoSet,
    update: bool,
) -> std::result::Result<bool, (String, String)> {
    let ext = packaging_ext(&coord.packaging);
    let local = repos.store().path(coord, ext);
    let snapshot = is_snapshot(&coord.version);
    if local.exists() && local.metadata().map(|m| m.len() > 0).unwrap_or(false) {
        if !(snapshot && update) {
            return Ok(false);
        }
    }
    let rel = rel_path(coord);
    let started = Instant::now();
    log::DL_ACTIVE.fetch_add(1, Ordering::Relaxed);
    log::refresh_status();
    log::downloading_line(&rel);
    // 流式下载直落盘：chunk 级进度/速度统计，整文件不驻留内存
    let mut last_bytes: u64 = 0;
    let fetched = repos
        .download_to_store(coord, &mut |cur, _| {
            // written 单轮内单调递增；重试重开时 written 归零 → cur < last_bytes，
            // 对齐基线（被丢弃部分不计流量），后续纯增量入账
            if cur < last_bytes {
                last_bytes = cur;
            } else if cur > last_bytes {
                log::DL_BYTES.fetch_add(cur - last_bytes, Ordering::Relaxed);
                last_bytes = cur;
            }
            log::refresh_status();
        })
        .await;
    // 无论成败先减量——写在 `?` 之后会让失败路径跳过减法，在途计数泄漏虚高
    log::DL_ACTIVE.fetch_sub(1, Ordering::Relaxed);
    let size = fetched.map_err(|e| (rel.clone(), e.to_string()))?;
    let elapsed = started.elapsed().as_secs_f64().max(0.001);
    let speed = size as f64 / elapsed;
    log::downloaded_line(
        &rel,
        &log::fmt_bytes(size),
        &log::fmt_bytes(speed as u64),
        &log::fmt_duration(started.elapsed()),
    );
    Ok(true)
}
