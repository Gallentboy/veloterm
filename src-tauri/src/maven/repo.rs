//! 远程仓库交互：严格顺序 fallback（第一仓 404 才试第二仓，全部失败才报错）、
//! Basic 认证、SNAPSHOT metadata 文件名决策、raw POM 缓存与写透落盘。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::maven::error::{AttemptLog, Error, Result};
use crate::maven::log;
use crate::maven::model::{Coordinate, RemoteRepo};
use crate::maven::pom::raw::{PomSource, RawPom};
use crate::maven::version::{cmp_version, is_snapshot, parse_range};

/// sha1 校验严格度（与 cli::ChecksumMode 对应）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChecksumMode {
    Off,
    Warn,
    Strict,
}
use crate::maven::layout::Store;

type Gav = (String, String, String);

/// maven-metadata.xml 中与 SNAPSHOT 文件名决策相关的字段。
#[derive(Debug, Default, Clone)]
pub struct SnapshotMeta {
    /// (extension, classifier, value)
    pub snapshot_versions: Vec<(String, String, String)>,
    pub timestamp: Option<String>,
    pub build_number: Option<String>,
}

/// 解析 maven-metadata.xml（仅 SNAPSHOT 相关字段；`<snapshot>`/`<snapshotVersions>`
/// 位于 `<versioning>` 内部，不在 metadata 根下）。
pub fn parse_snapshot_meta(xml: &str) -> Option<SnapshotMeta> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let root = doc.root_element();
    let text_of = |node: roxmltree::Node<'_, '_>, name: &str| -> Option<String> {
        node.children()
            .find(|n| n.is_element() && n.tag_name().name() == name)
            .and_then(|n| n.text())
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
    };
    let versioning = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "versioning")?;
    let mut meta = SnapshotMeta {
        timestamp: None,
        build_number: None,
        snapshot_versions: Vec::new(),
    };
    if let Some(snap) = versioning
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "snapshot")
    {
        meta.timestamp = text_of(snap, "timestamp");
        meta.build_number = text_of(snap, "buildNumber");
    }
    if let Some(vs) = versioning
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "snapshotVersions")
    {
        for v in vs.children().filter(|n| n.is_element() && n.tag_name().name() == "snapshotVersion") {
            // 空元素 <classifier/> 与缺失都归一为 ""
            let cls = text_of(v, "classifier").unwrap_or_default();
            if let (Some(ext), Some(value)) = (text_of(v, "extension"), text_of(v, "value")) {
                meta.snapshot_versions.push((ext, cls, value));
            }
        }
    }
    Some(meta)
}

/// SNAPSHOT 远程文件名决策（回退链）：
/// 1. `<snapshotVersion>` 按 ext+classifier 匹配的 value（可能本身就是 `-SNAPSHOT` 字面量）
/// 2. `<timestamp>` + `-<buildNumber>` 拼 `base-timestamp-buildNumber`
/// 3. 字面量 baseVersion
pub fn snapshot_remote_filename(meta: &SnapshotMeta, base_v: &str, ext: &str, cls: &str) -> String {
    if let Some((_, _, value)) = meta
        .snapshot_versions
        .iter()
        .find(|(e, c, _)| e == ext && c == cls)
    {
        return value.clone();
    }
    if let (Some(ts), Some(bn)) = (&meta.timestamp, &meta.build_number) {
        let stem = base_v.strip_suffix("-SNAPSHOT").unwrap_or(base_v);
        return format!("{stem}-{ts}-{bn}");
    }
    base_v.to_string()
}

/// GA 级 maven-metadata.xml 中与选版相关的字段。
#[derive(Debug, Default, Clone)]
pub struct GaVersions {
    pub versions: Vec<String>,
    pub latest: Option<String>,
    pub release: Option<String>,
}

/// 解析 GA 级 maven-metadata.xml（`<versioning>` 下的 version 列表 / latest / release）。
pub fn parse_ga_versions(xml: &str) -> Option<GaVersions> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let root = doc.root_element();
    let text_children = |node: roxmltree::Node<'_, '_>, name: &str| -> Vec<String> {
        node.children()
            .filter(|n| n.is_element() && n.tag_name().name() == name)
            .filter_map(|n| n.text())
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    };
    let text_of = |node: roxmltree::Node<'_, '_>, name: &str| -> Option<String> {
        node.children()
            .find(|n| n.is_element() && n.tag_name().name() == name)
            .and_then(|n| n.text())
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
    };
    let versioning = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "versioning")?;
    // 版本列表位于 <versions> 容器内
    let versions = versioning
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "versions")
        .map(|vs| text_children(vs, "version"))
        .unwrap_or_default();
    Some(GaVersions {
        versions,
        latest: text_of(versioning, "latest"),
        release: text_of(versioning, "release"),
    })
}

/// 实际输出根目录（main 启动时注入；警告信息按真实下载位置给出文件路径）
static DISK_ROOT: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

pub fn set_disk_root(root: std::path::PathBuf) {
    // 中毒恢复：锁被 panic 污染时取出内部数据继续用（本模块已无 panic 点，纯兑底）
    *DISK_ROOT.lock().unwrap_or_else(|e| e.into_inner()) = Some(root);
}

/// 输出目录布局下的 pom 文件路径（警告信息用：直接指出问题文件位置）
pub fn disk_pom_path(g: &str, a: &str, v: &str) -> std::path::PathBuf {
    let root = DISK_ROOT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| home::home_dir().unwrap_or_default().join(".m2/repository"));
    let mut p = root.join(g.replace('.', "/"));
    p.push(a);
    p.push(v);
    p.push(format!("{a}-{v}.pom"));
    p
}

/// 计算字节的 sha1 hex（小写 40 位；查表写入，无逐字节 format!）
pub fn sha1_hex(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(bytes);
    sha1_hex_of(&h)
}

/// 从 Sha1 状态取 hex（流式路径共用）
#[inline]
fn sha1_hex_of<D: sha1::Digest + Clone>(h: &D) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let out = h.clone().finalize();
    let mut buf = [0u8; 40];
    for (i, b) in out.iter().enumerate() {
        buf[i * 2] = HEX[(b >> 4) as usize];
        buf[i * 2 + 1] = HEX[(b & 0xf) as usize];
    }
    String::from_utf8(buf.to_vec()).unwrap_or_default()
}

/// 解析 .sha1 文件内容：兼容 "hex" 与 "hex  filename.jar" 两种格式；
/// 非 40 位 hex 返回 None。
pub fn parse_sha1_content(text: &str) -> Option<String> {
    let first = text.split_whitespace().next()?;
    let lower = first.to_ascii_lowercase();
    if lower.len() == 40 && lower.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(lower)
    } else {
        None
    }
}

/// 打印错误的完整 cause 链（reqwest 网络错误的 source 常含真正原因，如 DNS/握手失败）。
#[inline]
fn err_chain(e: &reqwest::Error) -> String {
    let mut s = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(inner) = src {
        s.push_str(": ");
        s.push_str(&inner.to_string());
        src = inner.source();
    }
    s
}

/// 代理配置（`--proxy` 或 settings.xml `<proxies>`）。
#[derive(Clone, Debug)]
pub struct ProxySpec {
    pub url: String,
    pub auth: Option<(String, String)>,
}

/// 仓库集合：查询按 `repos` 顺序严格 fallback。
/// 全局唯一 reqwest::Client（连接池复用 HTTP/2 + TCP Keep-Alive，避免重复握手）；
/// 仓库列表可后期替换（-f 模式两段式合成），Client 保持不变。
pub struct RepoSet {
    repos: RwLock<Arc<Vec<RemoteRepo>>>,
    client: reqwest::Client,
    store: Arc<Store>,
    /// (g,a,baseV,ext,cls) → 远程文件名（SNAPSHOT）
    snap_cache: RwLock<HashMap<(String, String, String, String, String), String>>,
    /// `-U`：SNAPSHOT 绕过本地盘与 metadata 缓存强制刷新
    force_snapshots: std::sync::atomic::AtomicBool,
    /// 解析期 POM 的下载计数/盘缓存命中计数与日志开关（汇总用；dry-run 关日志）
    pub pom_downloaded: std::sync::atomic::AtomicUsize,
    pub pom_cache_hits: std::sync::atomic::AtomicUsize,
    log_pom_downloads: std::sync::atomic::AtomicBool,
    /// (g,a,表达式) → 解析出的具体版本（范围/LATEST/RELEASE）
    version_cache: RwLock<HashMap<(String, String, String), String>>,
    /// raw POM 的 per-key 并发去重（并发解析下多个任务同时要同一 parent POM 时只下载一次）
    raw_inflight: tokio::sync::Mutex<HashMap<Gav, Arc<tokio::sync::Mutex<()>>>>,
    /// raw POM 有界缓存（LRU，128 条 ≈ 5-10MB）：消除共享 parent 的重复读盘+净化+解析
    raw_lru: RwLock<std::collections::VecDeque<(Gav, Arc<RawPom>)>>,
    /// sha1 校验模式（--checksum）
    checksum_mode: std::sync::atomic::AtomicU8,
}

impl RepoSet {
    /// 构建失败（非法代理 URL / TLS 后端初始化失败）返回 Err——GUI 进程绝不因
    /// 配置问题 panic（release 下 panic=abort 会击杀整个应用）。
    pub fn new(
        repos: Vec<RemoteRepo>,
        store: Arc<Store>,
        proxy: Option<ProxySpec>,
    ) -> std::result::Result<Self, String> {
        let mut builder = reqwest::Client::builder()
            // 部分 CDN（如 AWS ELB 后的 Central Portal）拒绝无 User-Agent 的请求
            .user_agent(concat!("mvn-dr/", env!("CARGO_PKG_VERSION")))
            // 连接超时与读空闲超时：慢速下载不受影响（只要持续有数据），
            // 但连接半死（长时间无数据）会在超时后转为错误 → 走重试/换仓，避免永久挂起
            .connect_timeout(std::time::Duration::from_secs(10))
            .read_timeout(std::time::Duration::from_secs(30));
        if let Some(p) = proxy {
            match reqwest::Proxy::all(&p.url) {
                Ok(mut pr) => {
                    if let Some((user, pass)) = &p.auth {
                        pr = pr.basic_auth(user, pass);
                    }
                    builder = builder.proxy(pr);
                }
                Err(e) => {
                    // 非法代理地址：降级为不挂代理并警告（而非击杀进程）
                    log::warn(format!("invalid proxy url `{}`: {e}; continuing without proxy", p.url));
                }
            }
        }
        let client = builder.build().map_err(|e| format!("reqwest client: {e}"))?;
        Ok(Self {
            repos: RwLock::new(Arc::new(repos)),
            client,
            store,
            snap_cache: RwLock::new(HashMap::new()),
            force_snapshots: std::sync::atomic::AtomicBool::new(false),
            pom_downloaded: std::sync::atomic::AtomicUsize::new(0),
            pom_cache_hits: std::sync::atomic::AtomicUsize::new(0),
            log_pom_downloads: std::sync::atomic::AtomicBool::new(true),
            version_cache: RwLock::new(HashMap::new()),
            raw_inflight: tokio::sync::Mutex::new(HashMap::new()),
            raw_lru: RwLock::new(std::collections::VecDeque::with_capacity(128)),
            checksum_mode: std::sync::atomic::AtomicU8::new(1), // 默认 Warn
        })
    }

    pub fn set_checksum_mode(&self, m: ChecksumMode) {
        let v = match m {
            ChecksumMode::Off => 0,
            ChecksumMode::Warn => 1,
            ChecksumMode::Strict => 2,
        };
        self.checksum_mode.store(v, std::sync::atomic::Ordering::Relaxed);
    }

    #[inline]
pub fn checksum_mode(&self) -> ChecksumMode {
        match self.checksum_mode.load(std::sync::atomic::Ordering::Relaxed) {
            0 => ChecksumMode::Off,
            2 => ChecksumMode::Strict,
            _ => ChecksumMode::Warn,
        }
    }

    /// `--dry-run`：解析仍需拉 POM，但不打下载日志（避免刷屏清单输出）。
    pub fn set_log_pom_downloads(&self, on: bool) {
        self.log_pom_downloads.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// 替换仓库列表（保持同一 Client 连接池）；Arc 换指针，读侧零拷贝。
    pub fn update_repos(&self, repos: Vec<RemoteRepo>) {
        *self.repos.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(repos);
    }

    /// `-U`：强制刷新 SNAPSHOT。
    pub fn set_force_snapshots(&self, on: bool) {
        self.force_snapshots.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    #[inline]
fn force_refresh(&self) -> bool {
        self.force_snapshots.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[inline]
fn dir_of(g: &str, a: &str, v: &str) -> String {
        format!("{}/{}/{}", g.replace('.', "/"), a, v)
    }

    /// 网络错误 / 5xx / 响应体中途断开的重试次数（不含首次请求，共最多 4 次请求）。
    const RETRIES: usize = 3;


    /// 解析版本表达式（范围 `[4.0,5.0)` / `LATEST` / `RELEASE`）为具体版本：
    /// 拉 GA 级 maven-metadata.xml，范围取满足条件的最高版本（Maven 语义）。
    pub async fn resolve_version(
        &self,
        g: &str,
        a: &str,
        expr: &str,
    ) -> std::result::Result<String, String> {
        let cache_key = (g.to_string(), a.to_string(), expr.to_string());
        if let Some(v) = self.version_cache.read().unwrap_or_else(|e| e.into_inner()).get(&cache_key) {
            return Ok(v.clone());
        }
        let rel = format!("{}/{a}/maven-metadata.xml", g.replace('.', "/"));
        let xml = self
            .fetch_bytes(&rel)
            .await
            .map_err(|e| format!("no version metadata for {g}:{a}: {e}"))?;
        let meta = parse_ga_versions(&String::from_utf8_lossy(&xml))
            .ok_or_else(|| format!("bad maven-metadata.xml for {g}:{a}"))?;
        let resolved = if expr.eq_ignore_ascii_case("LATEST") {
            meta.latest
                .or_else(|| meta.versions.iter().max_by(|x, y| cmp_version(x, y)).cloned())
                .ok_or_else(|| format!("no versions listed for {g}:{a}"))?
        } else if expr.eq_ignore_ascii_case("RELEASE") {
            meta.release
                .or_else(|| {
                    meta.versions
                        .iter()
                        .filter(|v| !is_snapshot(v))
                        .max_by(|x, y| cmp_version(x, y))
                        .cloned()
                })
                .ok_or_else(|| format!("no release version for {g}:{a}"))?
        } else {
            let range = parse_range(expr)
                .ok_or_else(|| format!("invalid version range `{expr}`"))?;
            meta.versions
                .iter()
                .filter(|v| range.contains(v))
                .max_by(|x, y| cmp_version(x, y))
                .cloned()
                .ok_or_else(|| format!("no version of {g}:{a} satisfies `{expr}` (candidates: {:?})", meta.versions))?
        };
        log::verbose(format!("resolved {g}:{a} `{expr}` -> {resolved}"));
        self.version_cache.write().unwrap_or_else(|e| e.into_inner()).insert(cache_key, resolved.clone());
        Ok(resolved)
    }

    /// 按仓库顺序取文件（无进度回调）。
    pub async fn fetch_bytes(&self, rel: &str) -> Result<Vec<u8>> {
        self.fetch_bytes_progress(rel, &mut |_, _| {}).await
    }

    /// 按仓库顺序取文件；网络问题重试 [`RETRIES`] 次（递增退避），404/401/403 直接换下一仓。
    /// `on_prog(已读字节, 总字节)` 随 chunk 流式回调（总字节来自 Content-Length，可能 None）。
    pub async fn fetch_bytes_progress(
        &self,
        rel: &str,
        on_prog: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<Vec<u8>> {
        let repos = Arc::clone(&self.repos.read().unwrap_or_else(|e| e.into_inner()));
        let mut attempts = AttemptLog::default();
        for repo in repos.iter() {
            let url = format!("{}/{}", repo.url.trim_end_matches('/'), rel);
            let mut attempt = 0usize;
            loop {
                let mut req = self.client.get(&url);
                if let Some((user, pass)) = &repo.auth {
                    req = req.basic_auth(user, Some(pass));
                }
                // 网络类失败（发送失败 / 5xx / 响应体中断）→ 重试；404/401/403 → 立即换仓
                let retryable = match req.send().await {
                    Ok(resp) => {
                        let status = resp.status();
                        if status.is_success() {
                            let total = resp.content_length();
                            let mut resp = resp;
                            let mut buf: Vec<u8> = Vec::with_capacity(total.unwrap_or(0) as usize);
                            // 流式读取：每个 chunk 回调进度；中途断开视为网络错误可重试
                            loop {
                                match resp.chunk().await {
                                    Ok(Some(chunk)) => {
                                        buf.extend_from_slice(&chunk);
                                        on_prog(buf.len() as u64, total);
                                    }
                                    Ok(None) => return Ok(buf),
                                    Err(e) => {
                                        attempts.push(&url, &err_chain(&e));
                                        break; // 走重试
                                    }
                                }
                            }
                            true
                        } else {
                            attempts.push(&url, &status);
                            if [404, 401, 403].contains(&status.as_u16()) {
                                break; // 明确不可重试：换仓
                            }
                            true // 5xx 等：重试
                        }
                    }
                    Err(e) => {
                        attempts.push(&url, &err_chain(&e));
                        true // 连接失败 / 超时 / TLS 等：重试
                    }
                };
                if !retryable || attempt >= Self::RETRIES {
                    break; // 重试用尽：换下一仓
                }
                attempt += 1;
                // 递增退避：100ms / 200ms / 300ms
                tokio::time::sleep(std::time::Duration::from_millis(100 * attempt as u64)).await;
            }
        }
        Err(Error::Other(format!("failed to fetch `{rel}` from all repositories\n{attempts}")))
    }

    /// SNAPSHOT 远程文件名（含 metadata 解析与缓存）；release 直接返回字面量文件名。
    async fn remote_file_name(&self, g: &str, a: &str, v: &str, ext: &str, cls: &str) -> String {
        let base_name = if cls.is_empty() {
            format!("{a}-{v}.{ext}")
        } else {
            format!("{a}-{v}-{cls}.{ext}")
        };
        if !is_snapshot(v) {
            return base_name;
        }
        let cache_key = (g.to_string(), a.to_string(), v.to_string(), ext.to_string(), cls.to_string());
        // -U 时绕过 metadata 文件名缓存，强制重新解析
        if !self.force_refresh() {
            if let Some(name) = self.snap_cache.read().unwrap_or_else(|e| e.into_inner()).get(&cache_key) {
                return name.clone();
            }
        }
        let rel = format!("{}/maven-metadata.xml", Self::dir_of(g, a, v));
        let resolved = match self.fetch_bytes(&rel).await {
            Ok(xml) => {
                let text = String::from_utf8_lossy(&xml).to_string();
                parse_snapshot_meta(&text)
                    .map(|m| snapshot_remote_filename(&m, v, ext, cls))
                    .map(|version_part| format!("{a}-{version_part}.{ext}"))
                    .unwrap_or_else(|| base_name)
            }
            Err(_) => {
                log::warn(format!("no maven-metadata.xml for {g}:{a}:{v}, using literal -SNAPSHOT name"));
                base_name
            }
        };
        self.snap_cache.write().unwrap_or_else(|e| e.into_inner()).insert(cache_key, resolved.clone());
        resolved
    }

    /// 拉取 raw POM（带 (g:a:v) 缓存、本地盘命中与写透落盘，SNAPSHOT 感知）。
    #[inline]
fn raw_lru_put(&self, key: Gav, pom: Arc<RawPom>) {
        let mut lru = self.raw_lru.write().unwrap_or_else(|e| e.into_inner());
        if let Some(idx) = lru.iter().position(|(k, _)| *k == key) {
            lru.remove(idx);
        }
        lru.push_back((key, pom));
        while lru.len() > 128 {
            lru.pop_front();
        }
    }

    pub async fn fetch_raw_pom_cached(&self, g: &str, a: &str, v: &str) -> Result<Arc<RawPom>> {
        let key = (g.to_string(), a.to_string(), v.to_string());
        // 并发去重：多个 BFS 任务同时要同一 POM（共享 parent/BOM 高频）只下载一次；
        // 无内存缓存（省驻留）——后到者拿锁后走盘路径命中先者写透的文件。
        // 所有退出路径（LRU/盘命中、网络失败、解析失败）统一清理 inflight 表项（防缓慢泄漏）
        let key_lock = {
            let mut inflight = self.raw_inflight.lock().await;
            inflight
                .entry(key.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _guard = key_lock.lock().await;
        let _inflight_cleanup = InflightCleanup(self, key.clone(), key_lock.clone());
        struct InflightCleanup<'a>(&'a RepoSet, Gav, Arc<tokio::sync::Mutex<()>>);
        impl Drop for InflightCleanup<'_> {
            fn drop(&mut self) {
                // 按身份（Arc 指针相等）删除：防止误删后来者插入的新表项导致并发重复拉取
                if let Ok(mut m) = self.0.raw_inflight.try_lock() {
                    if m.get(&self.1).map(|e| Arc::ptr_eq(e, &self.2)).unwrap_or(false) {
                        m.remove(&self.1);
                    }
                }
            }
        }
        // 本地已落盘（此前写透）直接读，避免重复网络请求；-U 的 SNAPSHOT 例外
        let disk_coord = Coordinate {
            group: g.to_string(),
            artifact: a.to_string(),
            version: v.to_string(),
            packaging: "jar".to_string(),
            classifier: None,
        };
        // LRU 命中（内存，共享 parent 高频场景消除重复读盘+净化+解析）
        if !is_snapshot(v) || !self.force_refresh() {
            let mut lru = self.raw_lru.write().unwrap_or_else(|e| e.into_inner());
            if let Some(idx) = lru.iter().position(|(k, _)| k == &key) {
                let pom = match lru.remove(idx) {
                    Some((_, pom)) => pom,
                    None => return Err(Error::Other("raw LRU 命中后移除失败（内部状态不一致）".into())),
                };
                lru.push_back((key.clone(), pom.clone()));
                if lru.len() > 128 {
                    lru.pop_front();
                }
                drop(lru);
                self.pom_cache_hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                log::DL_DONE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(pom);
            }
        }
        let disk_ok = !(is_snapshot(v) && self.force_refresh());
        let disk_pom_path = self.store.path(&disk_coord, "pom");
        if disk_ok {
            if let Ok(bytes) = std::fs::read(&disk_pom_path) {
                if !bytes.is_empty() {
                    // 自愈：盘缓存损坏（历史残留/写坏）时删除并重新下载，不再反复报 bad XML
                    match RawPom::parse(&String::from_utf8_lossy(&bytes)) {
                        Ok(parsed) => {
                            self.pom_cache_hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            log::DL_DONE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            log::verbose(format!("pom cached {g}/{a}/{v}/{a}-{v}.pom"));
                            let pom = Arc::new(parsed);
                            self.raw_lru_put(key.clone(), pom.clone());
                            return Ok(pom);
                        }
                        Err(e) => {
                            log::warn(format!(
                                "cached pom {g}/{a}/{v} corrupted ({e}); re-downloading"
                            ));
                            let _ = std::fs::remove_file(&disk_pom_path);
                        }
                    }
                }
            }
        }
        let fetch_started = std::time::Instant::now();
        let remote_name = self.remote_file_name(g, a, v, "pom", "").await;
        let rel = format!("{}/{remote_name}", Self::dir_of(g, a, v));
        // sha1 真并发：join! 同时 poll 两个 future（Box::pin 只构造不 poll 是惰性的——
        // 必须与 body 一起 poll 才真正在途）
        let (body_res, remote_sha) = tokio::join!(
            self.fetch_bytes(&rel),
            self.prefetch_remote_sha1(&rel),
        );
        let bytes = body_res.map_err(|e| Error::PomNotFound {
            coord: format!("{g}:{a}:{v}"),
            tried: e.to_string(),
        })?;
        let xml = String::from_utf8_lossy(&bytes).to_string();
        let pom = RawPom::parse(&xml)?;
        // 校验先于落盘（Strict 失败不写盘）
        if let Some(expected) = remote_sha {
            let local = sha1_hex(&bytes);
            if expected != local {
                let msg = format!("sha1 mismatch for {rel}: expected {expected}, got {local}");
                match self.checksum_mode() {
                    ChecksumMode::Strict => return Err(Error::Other(msg)),
                    _ => log::warn(msg),
                }
            }
        }
        // 写透：本地落盘用 baseVersion 文件名（POM 内容与远程名无关；复用 disk_coord）
        let coord = disk_coord;
        if let Err(e) = self.write_store(&coord, "pom", &bytes) {
            log::warn(format!("failed to cache pom locally: {e}"));
        }
        // 解析期的真实网络下载：与 jar 一致的完成日志与计数（packaging=pom 的 BOM 只有这个文件）
        self.pom_downloaded.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        log::DL_BYTES.fetch_add(bytes.len() as u64, std::sync::atomic::Ordering::Relaxed);
        if self.log_pom_downloads.load(std::sync::atomic::Ordering::Relaxed) {
            log::DL_DONE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            log::refresh_status();
            let elapsed = fetch_started.elapsed().as_secs_f64().max(0.001);
            let speed = bytes.len() as f64 / elapsed;
            log::downloaded_line(
                &format!("{}/{a}/{v}/{a}-{v}.pom", g.replace('.', "/")),
                &log::fmt_bytes(bytes.len() as u64),
                &log::fmt_bytes(speed as u64),
                &log::fmt_duration(fetch_started.elapsed()),
            );
        }
        self.raw_inflight.lock().await.remove(&key);
        let pom = Arc::new(pom);
        self.raw_lru_put(key, pom.clone());
        Ok(pom)
    }

    /// 流式下载主 artifact 直落盘（不整文件驻留内存）：
    /// chunk 级回调进度 + sha1 流式计算 + 唯一 .part 临时文件原子 rename。
    /// 返回成功下载的字节数（用于日志）。多仓 fallback/重试与 fetch_bytes 相同策略。
    pub async fn download_to_store(
        &self,
        coord: &Coordinate,
        on_prog: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<u64> {
        let ext = crate::maven::model::packaging_ext(&coord.packaging).to_string();
        let cls = coord.classifier.clone().unwrap_or_default();
        let remote_name = self
            .remote_file_name(&coord.group, &coord.artifact, &coord.version, &ext, &cls)
            .await;
        let rel = format!(
            "{}/{remote_name}",
            Self::dir_of(&coord.group, &coord.artifact, &coord.version)
        );
        // 临时文件（唯一序号）与目标
        let path = self.store.path(coord, &ext);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::Other(format!("mkdir: {e}")))?;
        }
        static PART_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = PART_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let orig_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let part = path.with_extension(format!("{orig_ext}.{seq}.part"));

        let repos = Arc::clone(&self.repos.read().unwrap_or_else(|e| e.into_inner()));
        let mut attempts = AttemptLog::default();
        'outer: for repo in repos.iter() {
            let url = format!("{}/{}", repo.url.trim_end_matches('/'), rel);
            let mut attempt = 0usize;
            loop {
                let mut req = self.client.get(&url);
                if let Some((user, pass)) = &repo.auth {
                    req = req.basic_auth(user, Some(pass));
                }
                let retryable = match req.send().await {
                    Ok(resp) => {
                        let status = resp.status();
                        if status.is_success() {
                            let total = resp.content_length();
                            let mut resp = resp;
                            // P1-5：BufWriter 消掉每 chunk 一次 write(2)
                            let mut file = std::io::BufWriter::with_capacity(
                                256 * 1024,
                                std::fs::File::create(&part)
                                    .map_err(|e| Error::Other(format!("create {}: {e}", part.display())))?,
                            );
                            let mut sha = <sha1::Sha1 as sha1::Digest>::new();
                            let mut written: u64 = 0;
                            // P0-1：body 在途时并发拉 .sha1——spawn 独立任务（真在途；
                            // Box::pin 惰性不 poll 无效）。仅需 client 克隆 + 仓库快照。
                            let sha_task = {
                                let client = self.client.clone();
                                let repos = Arc::clone(&repos);
                                let sha_rel = format!("{rel}.sha1");
                                let mode = self.checksum_mode();
                                tokio::spawn(async move {
                                    if mode == ChecksumMode::Off {
                                        return None;
                                    }
                                    let mut attempts = 0u32;
                                    // 单仓优先（产物来自当前仓），失败即 None（=通过）
                                    if let Some(repo) = repos.first() {
                                        let url = format!("{}/{}", repo.url.trim_end_matches('/'), sha_rel);
                                        loop {
                                            let mut req = client.get(&url);
                                            if let Some((u, p)) = &repo.auth {
                                                req = req.basic_auth(u, Some(p));
                                            }
                                            match req.send().await {
                                                Ok(r) if r.status().is_success() => {
                                                    let t = r.text().await.unwrap_or_default();
                                                    return parse_sha1_content(&t);
                                                }
                                                Ok(_) => return None, // 404/非 2xx：无校验文件
                                                Err(_) if attempts < 2 => {
                                                    attempts += 1;
                                                    tokio::time::sleep(std::time::Duration::from_millis(150 * attempts as u64)).await;
                                                }
                                                Err(_) => return None,
                                            }
                                        }
                                    }
                                    None
                                })
                            };
                            loop {
                                match resp.chunk().await {
                                    Ok(Some(chunk)) => {
                                        use std::io::Write;
                                        if let Err(e) = file.write_all(&chunk) {
                                            let _ = std::fs::remove_file(&part);
                                            return Err(Error::Other(format!("write part: {e}")));
                                        }
                                        sha1::Digest::update(&mut sha, &chunk);
                                        written += chunk.len() as u64;
                                        on_prog(written, total);
                                    }
                                    Ok(None) => {
                                        use std::io::Write;
                                        if let Err(e) = file.flush() {
                                            let _ = std::fs::remove_file(&part);
                                            return Err(Error::Other(format!("flush part: {e}")));
                                        }
                                        let local = sha1_hex_of(&sha);
                                        let remote = sha_task.await.unwrap_or(None);
                                        if let Some(mismatch) =
                                            Self::check_prefetched_sha1(&rel, remote.as_deref(), &local)
                                        {
                                            if self.checksum_mode() == ChecksumMode::Strict {
                                                let _ = std::fs::remove_file(&part);
                                                return Err(Error::Other(mismatch));
                                            }
                                        }
                                        drop(file);
                                        if std::fs::rename(&part, &path).is_err() {
                                            let _ = std::fs::remove_file(&path);
                                            std::fs::rename(&part, &path)
                                                .map_err(|e| Error::Other(format!("rename: {e}")))?;
                                        }
                                        return Ok(written);
                                    }
                                    Err(e) => {
                                        attempts.push(&url, &err_chain(&e));
                                        break; // 走重试
                                    }
                                }
                            }
                            true
                        } else {
                            attempts.push(&url, &status);
                            if [404, 401, 403].contains(&status.as_u16()) {
                                continue 'outer; // 换仓
                            }
                            true
                        }
                    }
                    Err(e) => {
                        attempts.push(&url, &err_chain(&e));
                        true
                    }
                };
                if !retryable || attempt >= Self::RETRIES {
                    continue 'outer;
                }
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_millis(100 * attempt as u64)).await;
            }
        }
        let _ = std::fs::remove_file(&part);
        Err(Error::Other(format!(
            "failed to fetch `{rel}` from all repositories\n{attempts}"
        )))
    }

    /// 预取远程 .sha1（下载 body 的同时并发进行，消除任务尾部的串行 RTT）。
    /// 返回 Option<remote_sha1>；Off 模式/拉取失败/不可解析返回 None（= 通过）。
    async fn prefetch_remote_sha1(&self, rel: &str) -> Option<String> {
        if self.checksum_mode() == ChecksumMode::Off {
            return None;
        }
        let c = self.fetch_bytes(&format!("{rel}.sha1")).await.ok()?;
        parse_sha1_content(&String::from_utf8_lossy(&c))
    }

    /// 比对预取的 sha1（body 完成后调用）：返回 Some(mismatch)（Warn 提示 / Strict 报错并删除）
    fn check_prefetched_sha1(rel: &str, remote: Option<&str>, local_sha: &str) -> Option<String> {
        if let Some(expected) = remote {
            if expected != local_sha {
                let msg = format!("sha1 mismatch for {rel}: expected {expected}, got {local_sha}");
                log::warn(&msg);
                return Some(msg);
            }
        }
        None
    }

    pub fn store(&self) -> &crate::maven::layout::Store {
        &self.store
    }

    /// 写入本地存储（原子写）：临时文件带全局唯一序号——并发任务同时落盘同一坐标时
    /// 各写各的 .part，避免固定名 .part 被交错写入产生损坏文件；rename 原子替换。
    pub fn write_store(&self, coord: &Coordinate, file_ext: &str, bytes: &[u8]) -> std::io::Result<()> {
        let path = self.store.path(coord, file_ext);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        static PART_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = PART_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let orig_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let part = path.with_extension(format!("{orig_ext}.{seq}.part"));
        std::fs::write(&part, bytes)?;
        if std::fs::rename(&part, &path).is_err() {
            let _ = std::fs::remove_file(&path);
            std::fs::rename(&part, &path)?;
        }
        Ok(())
    }
}

impl PomSource for RepoSet {
    fn fetch_raw_pom<'a>(&'a self, g: &'a str, a: &'a str, v: &'a str)
        -> crate::maven::pom::raw::BoxFut<'a, Result<RawPom>> {
        Box::pin(async move {
            let pom = self.fetch_raw_pom_cached(g, a, v).await?;
            Ok((*pom).clone())
        })
    }

    fn resolve_version<'a>(&'a self, g: &'a str, a: &'a str, expr: &'a str)
        -> crate::maven::pom::raw::BoxFut<'a, std::result::Result<String, String>> {
        Box::pin(async move { RepoSet::resolve_version(self, g, a, expr).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(vs: &[(&str, &str, &str)], ts: Option<&str>, bn: Option<&str>) -> SnapshotMeta {
        SnapshotMeta {
            snapshot_versions: vs
                .iter()
                .map(|(e, c, v)| (e.to_string(), c.to_string(), v.to_string()))
                .collect(),
            timestamp: ts.map(String::from),
            build_number: bn.map(String::from),
        }
    }

    #[test]
    fn snapshot_filename_from_snapshot_version() {
        let m = meta(
            &[("jar", "", "1.0-20240101.123456-3"), ("pom", "", "1.0-20240101.123456-3")],
            Some("20240101.123456"),
            Some("3"),
        );
        assert_eq!(
            snapshot_remote_filename(&m, "1.0-SNAPSHOT", "jar", ""),
            "1.0-20240101.123456-3"
        );
        assert_eq!(
            snapshot_remote_filename(&m, "1.0-SNAPSHOT", "pom", ""),
            "1.0-20240101.123456-3"
        );
        // classifier 匹配
        let m = meta(&[("jar", "sources", "1.0-20240101.123456-3-sources")], None, None);
        assert_eq!(
            snapshot_remote_filename(&m, "1.0-SNAPSHOT", "jar", "sources"),
            "1.0-20240101.123456-3-sources"
        );
    }

    #[test]
    fn snapshot_filename_fallback_to_timestamp() {
        let m = meta(&[], Some("20240601.101010"), Some("7"));
        assert_eq!(
            snapshot_remote_filename(&m, "1.0-SNAPSHOT", "jar", ""),
            "1.0-20240601.101010-7"
        );
    }

    #[test]
    fn snapshot_filename_literal_and_nonunique() {
        // 无任何信息 → 字面量
        let m = meta(&[], None, None);
        assert_eq!(snapshot_remote_filename(&m, "1.0-SNAPSHOT", "jar", ""), "1.0-SNAPSHOT");
        // non-unique 仓库 value 本身就是 -SNAPSHOT
        let m = meta(&[("jar", "", "1.0-SNAPSHOT")], None, None);
        assert_eq!(snapshot_remote_filename(&m, "1.0-SNAPSHOT", "jar", ""), "1.0-SNAPSHOT");
    }

    #[test]
    fn sha1_helpers() {
        // "hello" 的 sha1
        assert_eq!(sha1_hex(b"hello"), "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d");
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        // 两种 .sha1 文件格式
        assert_eq!(
            parse_sha1_content("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"),
            Some("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d".into())
        );
        assert_eq!(
            parse_sha1_content("AAF4C61DDCC5E8A2DABEDE0F3B482CD9AEA9434D  some-file.jar\n"),
            Some("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d".into())
        );
        assert_eq!(parse_sha1_content("not-a-hash"), None);
        assert_eq!(parse_sha1_content(""), None);
    }

    #[test]
    fn parse_metadata_xml() {
        let xml = r#"
<metadata>
  <groupId>g</groupId><artifactId>a</artifactId><version>1.0-SNAPSHOT</version>
  <versioning>
    <snapshot><timestamp>20240101.123456</timestamp><buildNumber>3</buildNumber></snapshot>
    <snapshotVersions>
    <snapshotVersion>
      <extension>pom</extension><value>1.0-20240101.123456-3</value><updated>20240101123456</updated>
    </snapshotVersion>
    <snapshotVersion>
      <extension>jar</extension><classifier></classifier><value>1.0-20240101.123456-3</value>
    </snapshotVersion>
    <snapshotVersion>
      <extension>jar</extension><classifier>sources</classifier><value>1.0-20240101.123456-3-sources</value>
    </snapshotVersion>
    </snapshotVersions>
  </versioning>
</metadata>"#;
        let m = parse_snapshot_meta(xml).unwrap();
        assert_eq!(m.timestamp.as_deref(), Some("20240101.123456"));
        assert_eq!(m.build_number.as_deref(), Some("3"));
        assert_eq!(m.snapshot_versions.len(), 3);
        // 空元素 <classifier/> 归一为 ""
        assert!(m.snapshot_versions.iter().any(|(e, c, v)| e == "jar" && c.is_empty() && v == "1.0-20240101.123456-3"));
    }
}
