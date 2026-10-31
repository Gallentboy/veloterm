//! Maven 仓库助手（自 mvn-dr 迁移适配）：依赖解析 / 并发下载 / 仓库搜索 / 上传部署。
//!
//! 模块分层（与原 mvn-dr 仓库同名文件一一对应，便于人工对照同步）：
//! - `model/version/error/layout`：领域模型（GAV/scope 传递表/版本比较/存储布局）
//! - `pom/raw|effective`：POM 宽容解析与 Maven 语义构建（parent 链/插值/BOM/depMgmt）
//! - `resolver`：层同步 BFS + nearest-wins 调解（含 arena → 前端树节点导出）
//! - `repo`：多仓 fallback / SNAPSHOT 决策 / POM 三级缓存 / 流式下载
//! - `download`：解析-下载流水线（mpsc + JoinSet + Semaphore，支持取消）
//! - `settings/config`：settings.xml 解析与仓库列表合成（mirror/凭据）
//! - `search/put`：仓库搜索（Nexus/solr/metadata 探测）与并发上传
//! - `log`：GUI 事件化的日志与进度层（DL_* 计数器 + 事件 sink，替代原 TUI）
//!
//! 本文件（mod.rs）：Tauri 命令层——请求/响应 DTO、事件枚举（Channel 流式）、
//! 运行注册表（取消）、get 全流程编排（原 main.rs run_get 的库化形态）。

// 迁移代码 lint 豁免：本模块主体为 mvn-dr（用户自有工程）的既有实现原样迁移——
// arena 索引（resolver 的 nodes[i]）是模块内部不变量（索引由 push_node 顺序生成、
// 长度单调递增）。对久经真机回归的代码做样式类重写只有回归风险没有收益，按
// vendor 树 cap-lints 的同等口径整模块豁免机械样式 lint。
// ⚠️ clippy::panic **不在豁免之列**（工程 [lints] deny + release panic=abort 下
// 任何 panic 都会击杀整个应用）：expect/unwrap/unreachable 已全部改为优雅降级
// （JoinError 收录失败项、锁中毒恢复、防御分支），新增代码同样禁止 panic。
#![allow(
    clippy::indexing_slicing,
    clippy::collapsible_if,
    clippy::too_many_arguments,
    clippy::len_zero,
    clippy::type_complexity,
    clippy::unnecessary_filter_map,
    clippy::redundant_locals,
    clippy::redundant_field_names,
    clippy::match_single_binding,
    clippy::manual_range_patterns,
    clippy::manual_find,
    clippy::format_in_format_args,
    clippy::filter_next,
    clippy::doc_lazy_continuation
)]

pub mod config;
pub mod download;
pub mod error;
pub mod layout;
pub mod log;
pub mod model;
pub mod pom;
pub mod put;
pub mod repo;
pub mod resolver;
pub mod search;
pub mod settings;
pub mod version;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use serde::Serialize;
use tauri::ipc::Channel;

use crate::maven::config::CliRepo;
use crate::maven::download::{DownloadOpts, Summary as DlSummary};
use crate::maven::layout::{Layout, Store};
use crate::maven::model::Coordinate;
use crate::maven::pom::effective::ResolveContext;
use crate::maven::pom::raw::RawPom;
use crate::maven::repo::{ChecksumMode, ProxySpec, RepoSet};
use crate::maven::resolver::{
    export_nodes, render_tree, resolve, CancelFlag, ResolveOutcome, RootRequest, TreeNodeDto,
};
use crate::maven::search::{run_search, SearchOutcome, SearchParams};
use crate::maven::settings::Settings;

// ---------------- 事件（tag=type 判别联合；逐变体 rename_all 是工程踩坑铁律） ----------------

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MavenEvent {
    #[serde(rename_all = "camelCase")]
    Log { level: u8, message: String },
    #[serde(rename_all = "camelCase")]
    DownloadStart { rel: String },
    #[serde(rename_all = "camelCase")]
    DownloadDone { rel: String, size: String, speed: String, took: String },
    #[serde(rename_all = "camelCase")]
    UploadStart { rel: String },
    #[serde(rename_all = "camelCase")]
    UploadDone { rel: String, size: String, took: String },
    /// 250ms 轮询的实时统计（DL_* 计数器快照）
    #[serde(rename_all = "camelCase")]
    Stats { done: usize, total: usize, active: usize, queued: usize, failed: usize, bytes: u64 },
}

// ---------------- 运行注册表（取消；js_runtime ACTIVE_RUNS 同型） ----------------

fn runs() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static RUNS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    RUNS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_run(run_id: &str) -> CancelFlag {
    let flag = Arc::new(AtomicBool::new(false));
    runs().lock().unwrap_or_else(|e| e.into_inner()).insert(run_id.to_string(), flag.clone());
    flag
}

fn unregister_run(run_id: &str) {
    runs().lock().unwrap_or_else(|e| e.into_inner()).remove(run_id);
}

/// 取消某次运行（get / put 通用）；不存在的 runId 返回 false。
#[tauri::command]
pub fn maven_cancel_run(run_id: String) -> bool {
    match runs().lock().unwrap_or_else(|e| e.into_inner()).get(&run_id) {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

// ---------------- get / resolve：请求与响应 DTO ----------------

/// 下载 / 仅解析 共用请求。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MavenGetRequest {
    /// 单坐标 `g:a:v[:packaging[:classifier]]`（支持 LATEST/RELEASE/范围）
    pub gav: Option<String>,
    /// 本地 pom.xml 路径（parent 链继承 + 根 depMgmt 全局钉版本）
    pub pom_path: Option<String>,
    /// GAV 清单多行文本（每行一个坐标，# 与 // 行内注释）
    pub gav_list: Option<String>,
    /// 输出目录；None = settings localRepository 或 ~/.m2/repository
    pub output: Option<String>,
    /// "m2"（默认）| "flat"
    pub layout: Option<String>,
    /// settings.xml 显式路径；None = ~/.m2/settings.xml（不存在则空配置）
    pub settings_path: Option<String>,
    /// 额外仓库（`id=url` 或裸 `url`，按序优先于内置兜底）
    pub repos: Option<Vec<String>>,
    /// 属性覆盖（`k=v`，插值最高优先）
    pub define: Option<Vec<String>>,
    /// -U：SNAPSHOT 强刷
    pub update_snapshots: Option<bool>,
    /// --all-classifiers：各平台 native classifier
    pub all_classifiers: Option<bool>,
    /// --sources：附加源码 jar
    pub sources: Option<bool>,
    /// "off" | "warn"（默认）| "strict"
    pub checksum: Option<String>,
    /// 并发下载数；None = CPU×2 下限 8
    pub threads: Option<usize>,
    /// 不展开传递依赖（仅直接依赖）
    pub no_transitive: Option<bool>,
    /// 显式代理 `http://host:port`
    pub proxy: Option<String>,
}

/// 仅解析（dry-run）结果：交互树 + 平面清单 + 文本树。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MavenResolveResult {
    /// 全部节点（含 loser），前端按 parent/children 组树
    pub nodes: Vec<TreeNodeDto>,
    /// winner 节点的 `g:a:v (scope)` 展示串
    pub winners: Vec<String>,
    /// render_tree 文本（└─/├─ 形态，含冲突/管理标注）
    pub tree_text: String,
    /// 解析期跳过的构件（POM 不存在等）
    pub failures: Vec<MavenFailedItem>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MavenFailedItem {
    pub item: String,
    pub error: String,
}

/// 下载全流程汇总。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MavenGetSummary {
    pub downloaded: usize,
    pub skipped: usize,
    /// 解析期 POM 网络下载 / 盘缓存命中数
    pub pom_downloaded: usize,
    pub pom_cache_hits: usize,
    pub failed: Vec<MavenFailedItem>,
    pub bytes: u64,
    pub elapsed_ms: u64,
    pub cancelled: bool,
    pub resolved_total: usize,
    pub output_dir: String,
    pub report_path: Option<String>,
}

/// get 流程内部产物（dry-run 与完整模式共用）。
struct GetFlowOutput {
    outcome: ResolveOutcome,
    output_dir: PathBuf,
    dl_summary: Option<DlSummary>,
    poms: usize,
    pom_hits: usize,
    bytes: u64,
    elapsed: std::time::Duration,
    cancelled: bool,
    report_path: Option<String>,
}

/// 仅解析依赖（dry-run）：不出文件，产出交互树节点与文本树。
#[tauri::command]
pub async fn maven_resolve(request: MavenGetRequest) -> Result<MavenResolveResult, String> {
    let out = run_get_flow(&request, true, None).await?;
    let winners: Vec<String> = out
        .outcome
        .winners
        .iter()
        .map(|&wi| out.outcome.nodes[wi].display())
        .collect();
    Ok(MavenResolveResult {
        nodes: export_nodes(&out.outcome),
        tree_text: render_tree(&out.outcome),
        winners,
        failures: out
            .outcome
            .failures
            .iter()
            .map(|(i, e)| MavenFailedItem { item: i.clone(), error: e.clone() })
            .collect(),
    })
}

/// 下载全流程（解析 + 下载流水线），事件经 Channel 流式回传。
#[tauri::command]
pub async fn maven_get_run(
    run_id: String,
    request: MavenGetRequest,
    on_event: Channel<MavenEvent>,
) -> Result<MavenGetSummary, String> {
    let flag = register_run(&run_id);
    let ch = Arc::new(Mutex::new(on_event));
    // log 层事件 sink：UiEvent → MavenEvent（运行期间安装，结束卸载）
    let sink_ch = ch.clone();
    log::set_sink(Some(Box::new(move |e: log::UiEvent| {
        let ev = match e {
            log::UiEvent::Log { level, message } => MavenEvent::Log { level, message },
            log::UiEvent::DownloadStart { rel } => MavenEvent::DownloadStart { rel },
            log::UiEvent::DownloadDone { rel, size, speed, took } => {
                MavenEvent::DownloadDone { rel, size, speed, took }
            }
            log::UiEvent::UploadStart { rel } => MavenEvent::UploadStart { rel },
            log::UiEvent::UploadDone { rel, size, took } => MavenEvent::UploadDone { rel, size, took },
        };
        if let Ok(guard) = sink_ch.lock() {
            let _ = guard.send(ev);
        }
    })));
    log::reset_counters();

    // 统计 ticker：250ms 快照 DL_* 计数器发 Stats（流程结束 abort）
    let ticker_ch = ch.clone();
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let s = log::snapshot();
            if let Ok(guard) = ticker_ch.lock() {
                let _ = guard.send(MavenEvent::Stats {
                    done: s.done,
                    total: s.total,
                    active: s.active,
                    queued: s.queued,
                    failed: s.failed,
                    bytes: s.bytes,
                });
            }
        }
    });

    let result = run_get_flow(&request, false, Some(flag.clone())).await;
    ticker.abort();
    log::set_sink(None);
    unregister_run(&run_id);
    let out = result?;

    Ok(MavenGetSummary {
        downloaded: out.dl_summary.as_ref().map(|s| s.downloaded).unwrap_or(0) + out.poms,
        skipped: out.dl_summary.as_ref().map(|s| s.skipped).unwrap_or(0) + out.pom_hits,
        pom_downloaded: out.poms,
        pom_cache_hits: out.pom_hits,
        failed: out
            .all_failed()
            .into_iter()
            .map(|(item, error)| MavenFailedItem { item, error })
            .collect(),
        bytes: out.bytes,
        elapsed_ms: out.elapsed.as_millis() as u64,
        cancelled: out.cancelled,
        resolved_total: out.outcome.winners.len(),
        output_dir: out.output_dir.display().to_string(),
        report_path: out.report_path,
    })
}

impl GetFlowOutput {
    /// 解析失败 + 下载失败合并清单（报告与前端失败列表共用口径）。
    fn all_failed(&self) -> Vec<(String, String)> {
        let mut all = self.outcome.failures.clone();
        if let Some(s) = &self.dl_summary {
            all.extend(s.failed.iter().cloned());
        }
        all
    }
}

/// get 全流程编排（原 mvn-dr main.rs run_get 的库化形态）。
/// dry_run=true：dummy 下载通道，解析完即返（不下载文件）。
async fn run_get_flow(
    cli: &MavenGetRequest,
    dry_run: bool,
    cancel: Option<CancelFlag>,
) -> Result<GetFlowOutput, String> {
    let total_started = Instant::now();

    // ---- settings 与输出目录 ----
    let settings = Settings::load(cli.settings_path.as_deref().map(std::path::Path::new))
        .map_err(|e| format!("settings.xml: {e}"))?;
    // 输出目录为必填：不做任何默认回退（settings localRepository / ~/.m2/repository
    // 均不启用）——用户必须显式选择，杜绝误写默认仓库
    let output = cli
        .output
        .clone()
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "必须选择输出目录（output 必填，不使用默认位置）".to_string())?;
    let store = Arc::new(Store::new(
        output.clone(),
        Layout::parse(cli.layout.as_deref().unwrap_or("m2")),
    ));
    let disk_root = std::fs::canonicalize(&output).unwrap_or_else(|_| output.clone());
    repo::set_disk_root(disk_root);

    // 代理优先级：显式 > settings.xml <proxies> > 环境变量（reqwest 内建）
    let proxy: Option<ProxySpec> = match &cli.proxy {
        Some(url) => Some(ProxySpec { url: url.clone(), auth: None }),
        None => settings.proxy.clone(),
    };

    // settings.xml activeProfiles 注入：属性作为 define 之下的默认层；仓库并入根仓库列表
    let mut cli_props = HashMap::new();
    for prof in settings.active_profiles() {
        for (k, v) in &prof.properties {
            cli_props.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    let settings_profile_repos: Vec<pom::raw::RawRepo> = settings
        .active_profiles()
        .iter()
        .flat_map(|p| p.repositories.iter().cloned())
        .collect();
    let cli_repos: Vec<CliRepo> = cli
        .repos
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|s| CliRepo::parse(s))
        .collect();
    for d in cli.define.as_deref().unwrap_or(&[]) {
        let (k, v) = d
            .split_once('=')
            .ok_or_else(|| format!("define 需要 k=v 形态，收到 `{d}`"))?;
        cli_props.insert(k.trim().to_string(), v.trim().to_string());
    }

    // ---- 根请求（两段式仓库合成：pom 模式需先拉 parent 链才能拿到其 repositories）----
    let repos = Arc::new(RepoSet::new(
        config::build_repos(&cli_repos, &[], &settings),
        store.clone(),
        proxy,
    )?);
    if cli.update_snapshots.unwrap_or(false) {
        repos.set_force_snapshots(true);
    }
    repos.set_checksum_mode(match cli.checksum.as_deref().unwrap_or("warn") {
        "off" => ChecksumMode::Off,
        "strict" => ChecksumMode::Strict,
        _ => ChecksumMode::Warn,
    });

    let root_request;
    let pom_repos;
    if let Some(gav) = &cli.gav {
        let mut coord = Coordinate::parse_gav(gav).map_err(|e| e.to_string())?;
        // 根坐标支持 LATEST/RELEASE/版本范围：GA metadata 解析为具体版本
        if version::is_range_or_meta(&coord.version) {
            let resolved = repos
                .resolve_version(&coord.group, &coord.artifact, &coord.version)
                .await
                .map_err(|e| format!("解析根版本 `{}`: {e}", coord.version))?;
            log::info(format!("根版本 `{}` -> {resolved}", coord.version));
            coord.version = resolved;
        }
        root_request = RootRequest::Gav(coord);
        pom_repos = Vec::new();
    } else if let Some(list) = &cli.gav_list {
        // GAV 清单模式：每行一个坐标（# 与 // 行内注释、空行忽略），全部作为根依赖
        let gavs: Vec<String> = list
            .lines()
            .map(|l| {
                let l = l.trim();
                let cut = [l.find('#'), l.find("//")]
                    .into_iter()
                    .flatten()
                    .min()
                    .unwrap_or(l.len());
                l[..cut].trim().to_string()
            })
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect();
        if gavs.is_empty() {
            return Err("GAV 清单为空".into());
        }
        // 版本表达式并行解析（JoinSet + 16），按原行序回收（保「清单内先出现胜」语义）
        let mut coords: Vec<Option<Coordinate>> = Vec::with_capacity(gavs.len());
        let mut errs: Vec<String> = Vec::new();
        let mut set = tokio::task::JoinSet::new();
        let sem = Arc::new(tokio::sync::Semaphore::new(16));
        for g in &gavs {
            let coord = Coordinate::parse_gav(g).map_err(|e| format!("清单行 `{g}`: {e}"))?;
            if !version::is_range_or_meta(&coord.version) {
                coords.push(Some(coord));
                continue;
            }
            let permit = sem
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| e.to_string())?;
            let repos = repos.clone();
            let idx = coords.len();
            coords.push(None); // 占位，join 后回填
            set.spawn(async move {
                let _permit = permit;
                let rv = repos
                    .resolve_version(&coord.group, &coord.artifact, &coord.version)
                    .await;
                (idx, coord, rv)
            });
        }
        while let Some(j) = set.join_next().await {
            let (idx, mut coord, rv) = j.map_err(|e| e.to_string())?;
            match rv {
                Ok(v) => {
                    log::info(format!("清单 `{}` -> 版本 {v}", gavs[idx]));
                    coord.version = v;
                    coords[idx] = Some(coord);
                }
                Err(e) => errs.push(format!("清单行 `{}` 版本解析: {e}", gavs[idx])),
            }
        }
        if !errs.is_empty() {
            return Err(errs.join("\n"));
        }
        // 占位 None（版本解析失败已被 errs 收录并提前 Err）防御性跳过而非 panic
        let gav_strings: Vec<String> = coords
            .into_iter()
            .filter_map(|c| {
                let c = c?;
                Some(match c.classifier.as_deref() {
                    Some(cls) if !cls.is_empty() => {
                        format!("{}:{}:{}:{}:{}", c.group, c.artifact, c.version, c.packaging, cls)
                    }
                    _ => format!("{}:{}:{}:{}", c.group, c.artifact, c.version, c.packaging),
                })
            })
            .collect();
        root_request = RootRequest::GavList(gav_strings);
        pom_repos = Vec::new();
    } else if let Some(pom_path) = &cli.pom_path {
        let pom_path = std::path::Path::new(pom_path)
            .canonicalize()
            .map_err(|e| format!("pom 不可达 {}: {e}", pom_path))?;
        let xml = std::fs::read_to_string(&pom_path).map_err(|e| e.to_string())?;
        let raw = RawPom::parse(&xml).map_err(|e| format!("parse {}: {e}", pom_path.display()))?;
        let root_dir = pom_path.parent().map(std::path::Path::to_path_buf);
        // 先用显式仓库 + 兜底拉根 effective（parent 链需要网络）
        let eff = {
            let ctx = ResolveContext::new(repos.clone(), cli_props.clone());
            ctx.build_from_local(raw, root_dir.as_deref(), Vec::new())
                .await
                .map_err(|e| format!("build effective pom: {e}"))?
        };
        log::info(format!(
            "resolved root pom {}:{}:{} ({} direct dependencies)",
            eff.coord.group,
            eff.coord.artifact,
            eff.coord.version,
            eff.deps.len()
        ));
        pom_repos = eff.repositories.clone();
        root_request = RootRequest::Pom(eff);
    } else {
        return Err("三种输入（GAV / pom 路径 / 清单文本）必须提供其一".into());
    }

    // ---- 最终仓库列表（顺序 fallback：显式 → settings activeProfiles → 根 POM 链 → 兜底）----
    let mut all_pom_repos = settings_profile_repos;
    all_pom_repos.extend(pom_repos.iter().cloned());
    let final_repos = config::build_repos(&cli_repos, &all_pom_repos, &settings);
    log::info(format!(
        "repositories: {}",
        final_repos.iter().map(|r| format!("{}={}", r.id, r.url)).collect::<Vec<_>>().join(", ")
    ));
    repos.update_repos(final_repos);

    // ---- 流水线：下载端先启动，解析端每确立一个 winner 立即投递 ----
    let threads = cli.threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get() * 2)
            .unwrap_or(8)
            .max(8)
    });
    log::DL_CONCURRENCY.store(threads, Ordering::Relaxed);
    let opts = DownloadOpts {
        threads,
        update_snapshots: cli.update_snapshots.unwrap_or(false),
        all_classifiers: cli.all_classifiers.unwrap_or(false),
        sources: cli.sources.unwrap_or(false),
    };
    let (summary_rx, sink_tx): (
        Option<tokio::sync::oneshot::Receiver<DlSummary>>,
        tokio::sync::mpsc::Sender<Coordinate>,
    ) = if dry_run {
        // dry-run：dummy channel（接收端已 drop，send 失败被 dispatch 忽略）；
        // 解析仍需拉 POM，但静默其下载日志
        repos.set_log_pom_downloads(false);
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(rx);
        (None, tx)
    } else {
        let (tx, rx) = download::start(repos.clone(), opts, cancel.clone());
        (Some(rx), tx)
    };

    let ctx = Arc::new(ResolveContext::new(repos.clone(), cli_props));
    let outcome = resolve(
        &root_request,
        ctx,
        repos.clone(),
        cli.no_transitive.unwrap_or(false),
        cancel.clone(),
        &sink_tx,
    )
    .await
    .map_err(|e| format!("resolve: {e}"))?;

    // 解析完成：drop 投递端，下载端进入排空阶段
    drop(sink_tx);

    // 分母口径：done + queued + active 闭合 = 管线将完成的总文件数（原 main.rs 同款）
    if !dry_run {
        let done = log::DL_DONE.load(Ordering::Relaxed);
        let queued = log::DL_QUEUED.load(Ordering::Relaxed);
        let active = log::DL_ACTIVE.load(Ordering::Relaxed);
        log::DL_TOTAL.store(done + queued + active, Ordering::Relaxed);
    }
    log::info(format!("resolved {} artifacts", outcome.winners.len()));

    let poms = repos.pom_downloaded.load(Ordering::Relaxed);
    let pom_hits = repos.pom_cache_hits.load(Ordering::Relaxed);

    if dry_run {
        return Ok(GetFlowOutput {
            outcome,
            output_dir: output,
            dl_summary: None,
            poms,
            pom_hits,
            bytes: log::DL_BYTES.load(Ordering::Relaxed),
            elapsed: total_started.elapsed(),
            cancelled: false,
            report_path: None,
        });
    }

    // ---- 下载汇总 ----
    // 非 dry-run 恒为 Some；None 走空汇总的防御分支（不 panic——release 的
    // panic=abort 会击杀整个应用）
    let summary = match summary_rx {
        Some(rx) => rx.await.map_err(|_| "download pipeline dropped".to_string())?,
        None => DlSummary::default(),
    };
    let bytes = log::DL_BYTES.load(Ordering::Relaxed);
    let cancelled =
        summary.cancelled || cancel.map(|f| f.load(Ordering::Relaxed)).unwrap_or(false);
    log::info(format!(
        "get done: {} downloaded, {} skipped, {} failed, {} in {}",
        summary.downloaded + poms,
        summary.skipped + pom_hits,
        summary.failed.len(),
        log::fmt_bytes(bytes),
        log::fmt_duration(total_started.elapsed()),
    ));

    // 失败报告（系统临时目录；解析/下载失败合并）
    let mut all_failed: Vec<(String, String)> = outcome.failures.clone();
    all_failed.extend(summary.failed.iter().cloned());
    let report_path = if !all_failed.is_empty() {
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let report = std::env::temp_dir().join(format!("veloterm-get-report-{ts}.txt"));
        let mut rpt = String::new();
        rpt.push_str(&format!(
            "VeloTerm Maven 下载报告\n========================================\n时间  : {}\n输出  : {}\n\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            output.display()
        ));
        rpt.push_str(&format!(
            "汇总\n----------------------------------------\n下载成功 : {}\n跳过     : {}\n失败     : {}\n累计流量 : {}\n\n",
            summary.downloaded + poms,
            summary.skipped + pom_hits,
            all_failed.len(),
            log::fmt_bytes(bytes),
        ));
        rpt.push_str(&format!("失败（{}）\n----------------------------------------\n", all_failed.len()));
        for (item, err) in &all_failed {
            rpt.push_str(&format!("  {item}\n    {err}\n"));
        }
        if std::fs::write(&report, rpt).is_ok() {
            log::info(format!("失败明细已写入 {}", report.display()));
            Some(report.display().to_string())
        } else {
            None
        }
    } else {
        None
    };

    Ok(GetFlowOutput {
        outcome,
        output_dir: output,
        dl_summary: Some(summary),
        poms,
        pom_hits,
        bytes,
        elapsed: total_started.elapsed(),
        cancelled,
        report_path,
    })
}

// ---------------- search ----------------

/// 仓库搜索：关键词（Nexus/中央仓）/ g:a 版本探测 / g:a:v 存在性三形态。
#[tauri::command]
pub async fn maven_search(
    query: String,
    repo: Option<String>,
    username: Option<String>,
    password: Option<String>,
    versions: Option<bool>,
    check_jar: Option<bool>,
    classifiers: Option<Vec<String>>,
    rows: Option<usize>,
) -> Result<SearchOutcome, String> {
    let p = SearchParams {
        query,
        repo,
        username,
        password,
        versions: versions.unwrap_or(false),
        check_jar: check_jar.unwrap_or(false),
        classifiers: classifiers.unwrap_or_default(),
        rows: rows.unwrap_or(20).clamp(1, 200),
    };
    run_search(&p).await
}

// ---------------- put（上传） ----------------

use crate::maven::put::{PutParams, PutPlanItem, PutSummary};

/// 扫描上传计划（前端预览表格）：pom 锚定 GAV + 配对 jar 的远程相对路径清单。
#[tauri::command]
pub fn maven_put_plan(dir: String) -> Result<MavenPutPlan, String> {
    let (items, skipped) = put::plan_uploads_preview(std::path::Path::new(&dir))?;
    Ok(MavenPutPlan { items, skipped })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MavenPutPlan {
    pub items: Vec<PutPlanItem>,
    pub skipped: usize,
}

/// 上传全流程（并发 PUT），事件经 Channel 流式回传。
#[tauri::command]
pub async fn maven_put_run(
    run_id: String,
    dir: String,
    repo: String,
    username: Option<String>,
    password: Option<String>,
    threads: Option<usize>,
    check_repo: Option<String>,
    check_username: Option<String>,
    check_password: Option<String>,
    on_event: Channel<MavenEvent>,
) -> Result<PutSummary, String> {
    let flag = register_run(&run_id);
    let ch = Arc::new(Mutex::new(on_event));
    let sink_ch = ch.clone();
    log::set_sink(Some(Box::new(move |e: log::UiEvent| {
        let ev = match e {
            log::UiEvent::Log { level, message } => MavenEvent::Log { level, message },
            log::UiEvent::DownloadStart { rel } => MavenEvent::UploadStart { rel },
            log::UiEvent::DownloadDone { rel, size, speed, took } => {
                MavenEvent::UploadDone { rel, size: format!("{size} ({speed})"), took }
            }
            log::UiEvent::UploadStart { rel } => MavenEvent::UploadStart { rel },
            log::UiEvent::UploadDone { rel, size, took } => MavenEvent::UploadDone { rel, size, took },
        };
        if let Ok(guard) = sink_ch.lock() {
            let _ = guard.send(ev);
        }
    })));
    log::reset_counters();

    let ticker_ch = ch.clone();
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let s = log::snapshot();
            if let Ok(guard) = ticker_ch.lock() {
                let _ = guard.send(MavenEvent::Stats {
                    done: s.done,
                    total: s.total,
                    active: s.active,
                    queued: s.queued,
                    failed: s.failed,
                    bytes: s.bytes,
                });
            }
        }
    });

    let params = PutParams {
        dir: PathBuf::from(dir),
        repo,
        username: username.unwrap_or_default(),
        password: password.unwrap_or_default(),
        threads,
        check_repo,
        check_username,
        check_password,
    };
    let result = put::run_upload(&params, Some(flag.clone())).await;
    ticker.abort();
    log::set_sink(None);
    unregister_run(&run_id);
    result
}

// ---------------- 环境信息 ----------------

/// 环境默认值（前端表单 placeholder 用）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MavenEnvInfo {
    pub settings_path: Option<String>,
    pub settings_exists: bool,
    /// settings localRepository 或 ~/.m2/repository
    pub local_repository: String,
    /// 内置兜底仓库（阿里云公共镜像）
    pub default_repo_url: String,
}

#[tauri::command]
pub fn maven_env_info() -> MavenEnvInfo {
    let settings_path = home::home_dir().map(|h| h.join(".m2").join("settings.xml"));
    let settings_exists = settings_path.as_ref().map(|p| p.exists()).unwrap_or(false);
    let local_repository = settings_path
        .as_deref()
        .and_then(|_| Settings::load(None).ok())
        .and_then(|s| s.local_repository)
        .map(|p| p.display().to_string())
        .or_else(|| {
            home::home_dir()
                .map(|h| h.join(".m2").join("repository").display().to_string())
        })
        .unwrap_or_else(|| "~/.m2/repository".to_string());
    MavenEnvInfo {
        settings_path: settings_path.map(|p| p.display().to_string()),
        settings_exists,
        local_repository,
        default_repo_url: config::DEFAULT_REPO_URL.to_string(),
    }
}
