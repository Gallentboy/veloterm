//! Maven 上传（原 mvn-dr put 子命令的库化形态）：递归扫描本地 m2/flat/Gradle
//! 布局目录，pom 锚定 GAV 后并发 PUT 全部 jar/pom 到远程仓库。
//! 上传地址 = 仓库 URL + GAV 重建的相对路径（与本地布局无关）。
//! GUI 适配：打印输出改为事件（log 层 sink），汇总结构化返回；取消旗标内建。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::maven::log;
use crate::maven::resolver::CancelFlag;

/// 单次 PUT 的结果摘要（serde 给前端）
#[derive(Default, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PutSummary {
    pub uploaded: usize,
    pub skipped: usize,
    pub uploaded_list: Vec<String>,
    pub skipped_list: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub bytes: u64,
    /// 序列化毫秒数
    pub elapsed_ms: u64,
    /// 用户主动取消（未开始的文件不再上传）
    pub cancelled: bool,
    /// 失败报告文件路径（写入系统临时目录）
    pub report_path: Option<String>,
}

/// 上传计划单条（扫描预览用）
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PutPlanItem {
    /// 本地文件路径
    pub local: String,
    /// 远程相对路径（GAV 重建）
    pub rel: String,
}

/// 上传请求参数（原 PutArgs 的库化形态；目录模式，压缩包自动解压 GUI 暂不提供）
pub struct PutParams {
    pub dir: PathBuf,
    pub repo: String,
    pub username: String,
    pub password: String,
    pub threads: Option<usize>,
    /// 存在性校验仓库（HEAD 探测已存在则跳过）
    pub check_repo: Option<String>,
    pub check_username: Option<String>,
    pub check_password: Option<String>,
}

/// 扫描并规划上传（前端「扫描计划」按钮）：返回 (计划列表, 被过滤的 pom 数)。
/// 失败原因以 String 返回（路径不可达 / 无可确定 GAV 的文件）。
pub fn plan_uploads_preview(dir: &Path) -> Result<(Vec<PutPlanItem>, usize), String> {
    let dir = dir
        .canonicalize()
        .map_err(|e| format!("路径不可达 {}: {e}", dir.display()))?;
    if dir.is_file() {
        return Err("请选择目录（压缩包自动解压请在终端使用 mvn-dr）".into());
    }
    let (items, skipped) = plan_uploads(&dir);
    if items.is_empty() {
        return Err(format!(
            "{} 下没有可确定 GAV 的 jar/pom 文件（{} 个 pom 被过滤）",
            dir.display(),
            skipped
        ));
    }
    Ok((
        items
            .into_iter()
            .map(|i| PutPlanItem { local: i.local.display().to_string(), rel: i.rel })
            .collect(),
        skipped,
    ))
}

/// 执行上传全流程：并发 PUT + 事件回调（log 层）+ 可取消；返回结构化汇总。
/// 失败报告写入系统临时目录（get-report 同款语义），路径随 PutSummary 返回。
pub async fn run_upload(args: &PutParams, cancel: Option<CancelFlag>) -> Result<PutSummary, String> {
    let dir = args
        .dir
        .canonicalize()
        .map_err(|e| format!("路径不可达 {}: {e}", args.dir.display()))?;
    let (items, skipped) = plan_uploads(&dir);
    if items.is_empty() {
        return Err(format!(
            "{} 下没有可确定 GAV 的 jar/pom 文件（{} 个 pom 被过滤）",
            dir.display(),
            skipped
        ));
    }
    log::info(format!(
        "扫描完成：{} 个文件待上传（{} 个 pom 无法确定 GAV，已过滤）",
        items.len(),
        skipped
    ));
    let files: Vec<PathBuf> = items.iter().map(|i| i.local.clone()).collect();
    let rels: Vec<String> = items.iter().map(|i| i.rel.clone()).collect();

    log::DL_TOTAL.store(files.len(), std::sync::atomic::Ordering::Relaxed);
    // put 一次性规划全部任务：队列 = 未取出的任务数（随 spawn 消费递减）
    log::DL_QUEUED.store(files.len(), std::sync::atomic::Ordering::Relaxed);
    let client = reqwest::Client::builder()
        .user_agent(concat!("veloterm-maven/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("client: {e}"))?;
    let client = Arc::new(client);
    let repo_url = args.repo.trim_end_matches('/').to_string();
    // username 支持 user:pass 合写（password 提供时优先取整段，密码可含 :）
    let (user, pass) = if args.password.is_empty() {
        match args.username.split_once(':') {
            Some((u, p)) => (u.to_string(), p.to_string()),
            None => (args.username.clone(), String::new()),
        }
    } else {
        (args.username.clone(), args.password.clone())
    };

    // 上传并发上限 8：仓库服务端（Nexus 等）对高频 PUT 限流较紧，过高并发适得其反
    let threads = args.threads.unwrap_or(8).min(8);
    log::DL_CONCURRENCY.store(threads, std::sync::atomic::Ordering::Relaxed);
    let sem = Arc::new(tokio::sync::Semaphore::new(threads.max(1)));
    let started = Instant::now();
    let mut set = tokio::task::JoinSet::new();
    let mut summary = PutSummary::default();

    'launch: for (path, rel) in files.into_iter().zip(rels) {
        // 取消检查：未开始的文件不再起跑（在途任务自然完成）
        if let Some(flag) = &cancel {
            if flag.load(std::sync::atomic::Ordering::Relaxed) {
                summary.cancelled = true;
                break 'launch;
            }
        }
        // acquire 即"取出"（等待许可的任务已离开队列、进入待执行）
        let permit = match sem.clone().try_acquire_owned() {
            Ok(p) => {
                log::DL_QUEUED.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                p
            }
            Err(_) => {
                // 达到并发上限：该任务仍在队列中，等许可（拿到时一并减）
                let p = sem
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|e| format!("{e}"))?;
                log::DL_QUEUED.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                p
            }
        };
        let client = client.clone();
        let repo_url = repo_url.clone();
        let check_repo = args.check_repo.clone();
        // 凭据规则：校验仓 = 上传仓 → 复用主凭据；不同仓库 → 用校验专有凭据
        // （未提供则匿名探测——公开仓场景）
        let same_repo = args
            .check_repo
            .as_deref()
            .map(|c| c.trim_end_matches('/') == args.repo.trim_end_matches('/'))
            .unwrap_or(false);
        let check_auth = if same_repo {
            Some((user.clone(), pass.clone()))
        } else {
            // 不同仓库：绝不把上传仓凭据发给第三方校验仓（凭据泄漏）
            match (&args.check_username, &args.check_password) {
                (Some(u), Some(p)) => Some((u.clone(), p.clone())),
                (Some(u), None) => match u.split_once(':') {
                    Some((cu, cp)) => Some((cu.to_string(), cp.to_string())),
                    None => Some((u.clone(), String::new())),
                },
                _ => None,
            }
        };
        let (user, pass) = (user.clone(), pass.clone());
        set.spawn(async move {
            let _permit = permit;
            upload_one(
                &client,
                &repo_url,
                check_repo.as_deref(),
                check_auth.as_ref().map(|(u, p)| (u.as_str(), p.as_str())),
                &user,
                &pass,
                &path,
                &rel,
            )
            .await
        });
    }

    while let Some(joined) = set.join_next().await {
        // 任务 panic 经 JoinError 收录为失败项（release 的 panic=abort 下绝不二次 panic）
        match joined {
            Ok(Ok((rel, 0))) => {
                summary.skipped += 1;
                summary.skipped_list.push(rel);
            }
            Ok(Ok((rel, size))) => {
                summary.uploaded += 1;
                summary.bytes += size;
                summary.uploaded_list.push(rel);
            }
            Ok(Err((rel, e))) => {
                summary.failed.push((rel, e));
            }
            Err(join_err) => {
                summary.failed.push((format!("{join_err}"), "task panicked".into()));
            }
        }
    }
    summary.elapsed_ms = started.elapsed().as_millis() as u64;
    let took = log::fmt_duration(started.elapsed());
    log::info(format!(
        "put done: {} uploaded, {} skipped(已存在), {} failed, {} in {took} → {repo_url}",
        summary.uploaded,
        summary.skipped,
        summary.failed.len(),
        log::fmt_bytes(summary.bytes),
    ));
    // 报告文档：put-report-<时间戳>.txt（系统临时目录，GUI 下不污染工作目录）
    if !summary.failed.is_empty() {
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let report_path = std::env::temp_dir().join(format!("veloterm-put-report-{ts}.txt"));
        let mut rpt = String::new();
        rpt.push_str(&format!(
            "VeloTerm Maven 上传报告\n========================================\n时间  : {}\n仓库  : {repo_url}\n目录  : {}\n\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            args.dir.display()
        ));
        rpt.push_str(&format!(
            "汇总\n----------------------------------------\n上传成功    : {}\n跳过(已存在): {}\n失败        : {}\n总流量      : {}\n耗时        : {took}\n\n",
            summary.uploaded,
            summary.skipped,
            summary.failed.len(),
            log::fmt_bytes(summary.bytes)
        ));
        rpt.push_str(&format!("\n失败（{}）\n----------------------------------------\n", summary.failed.len()));
        for (rel, e) in &summary.failed {
            rpt.push_str(&format!("  {rel}\n    {e}\n"));
        }
        if std::fs::write(&report_path, rpt).is_ok() {
            summary.report_path = Some(report_path.display().to_string());
            log::info(format!("失败报告已写入 {}", report_path.display()));
        }
    }
    Ok(summary)
}

/// 单轮 `${key}` 替换（未解析保留字面量）
fn interp_once(s: &str, props: &std::collections::HashMap<String, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find("${") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 2..];
        match after.find('}').map(|e| (e, &after[..e])) {
            Some((end, key)) => match props.get(key) {
                Some(v) => out.push_str(v),
                None => {
                    out.push_str("${");
                    out.push_str(&after[..end]);
                    out.push('}');
                }
            },
            None => {
                out.push_str(&rest[pos..]);
                rest = "";
                continue;
            }
        }
        let close = after.find('}').map(|e| e + 1).unwrap_or(after.len());
        rest = &after[close..];
    }
    out.push_str(rest);
    out
}

struct UploadItem {
    local: PathBuf,
    /// 远程相对路径（由 GAV 重建，与本地布局无关）
    rel: String,
}
/// 扫描并规划：pom 锚定 GAV + 同目录/父目录配对 jar；过滤无法确定 GAV 的 pom。
fn plan_uploads(root: &Path) -> (Vec<UploadItem>, usize) {
    let (poms, jars) = scan(root);
    // jar 按目录索引（文件名小写匹配不敏感？Maven 文件名区分大小写，用精确）
    let mut jars_by_dir: HashMap<PathBuf, Vec<(String, PathBuf)>> = HashMap::new();
    for (dir, name, path) in jars {
        jars_by_dir.entry(dir).or_default().push((name, path));
    }
    let mut used: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut items = Vec::new();
    let mut skipped = 0usize;
    for (pom_file, raw) in &poms {
        let dir: &Path = pom_file; // pom 文件路径；其 parent() 即所在目录
        // 轻量插值：仅当 GAV 字段含 ${ 时构建属性表（本 POM properties + parent 声明回退 + project.*），
        // 解不了的保留字面量；最终仍含 ${ 的条目过滤（不把 ${revision} 上传成远端目录）
        let needs_interp = raw.group.as_deref().map(|g| g.contains("${")).unwrap_or(false)
            || raw.version.as_deref().map(|v| v.contains("${")).unwrap_or(false);
        // GAV：groupId/version 缺省取 parent 声明（静态可判定，无需拉远程 parent）。
        // unwrap_or_default：守卫已保证非空 Some，防御性取默认值而非 panic
        let (g, a, v) = match (&raw.group, &raw.parent) {
            (Some(g), Some(_)) if !raw.version.as_deref().unwrap_or("").is_empty() => {
                (g.clone(), raw.artifact.clone(), raw.version.clone().unwrap_or_default())
            }
            (Some(g), Some(p)) if p.version.len() > 0 && raw.version.as_deref().unwrap_or("").is_empty() => {
                (g.clone(), raw.artifact.clone(), p.version.clone())
            }
            (None, Some(p)) if !raw.version.as_deref().unwrap_or("").is_empty() => {
                (p.group.clone(), raw.artifact.clone(), raw.version.clone().unwrap_or_default())
            }
            (None, Some(p)) if p.group.len() > 0 && p.version.len() > 0 => {
                (p.group.clone(), raw.artifact.clone(), p.version.clone())
            }
            (Some(g), None) if !raw.version.as_deref().unwrap_or("").is_empty() => {
                (g.clone(), raw.artifact.clone(), raw.version.clone().unwrap_or_default())
            }
            _ => {
                skipped += 1;
                log::verbose(format!("无法确定 GAV，过滤: {}", dir.display()));
                continue;
            }
        };
        if g.is_empty() || a.is_empty() || v.is_empty() {
            skipped += 1;
            continue;
        }
        // 属性插值（CI-friendly ${revision} 等）：本 POM properties + parent 声明回退 + project.* 内置
        let (g, v) = if needs_interp {
            let mut props: std::collections::HashMap<String, String> = std::collections::HashMap::new();
            props.insert("project.groupId".into(), raw.group.clone().unwrap_or_default());
            props.insert("project.artifactId".into(), raw.artifact.clone());
            props.insert("project.version".into(), v.clone());
            if let Some(par) = &raw.parent {
                props.insert("project.parent.groupId".into(), par.group.clone());
                props.insert("project.parent.version".into(), par.version.clone());
            }
            for (k, val) in &raw.properties {
                props.insert(k.clone(), val.clone());
            }
            let ip = |x: &str| interp_once(x, &props);
            (ip(&g), ip(&v))
        } else {
            (g, v)
        };
        if g.is_empty() || v.is_empty() || g.contains("${") || v.contains("${") {
            log::warn(format!(
                "无法解析属性，跳过 {}（pom: {}）",
                raw.artifact,
                pom_file.display()
            ));
            skipped += 1;
            continue;
        }
        let stem = format!("{a}-{v}");
        let rel_dir = format!("{}/{a}/{v}", g.replace('.', "/"));
        items.push(UploadItem {
            local: dir.to_path_buf(),
            rel: format!("{rel_dir}/{stem}.pom"),
        });
        // 配对 jar：pom 目录、其父目录（m2 的 version 目录/flat），以及父目录下全部兄弟子目录
        // （Gradle cache：g/a/v/<hash1>/a-v.pom 与 g/a/v/<hash2>/a-v.jar）
        // 搜索目录：pom 所在目录（m2 version 目录/flat）、其父目录、及其父目录下全部兄弟子目录
        // （Gradle cache：g/a/v/<hash1>/a-v.pom 与 g/a/v/<hash2>/a-v.jar）
        let pom_dir = dir.parent(); // hash 目录（Gradle）或 version 目录（m2）或 flat 目录
        let mut search_dirs = vec![pom_dir.map(|p| p.to_path_buf())];
        // 父目录（m2: version 的上级也能兜 flat-嵌套场景）
        search_dirs.push(Some(pom_dir.and_then(|p| p.parent()).map(|p| p.to_path_buf()).unwrap_or_default()));
        // 兄弟子目录：pom_dir 的父目录（= Gradle 的 version 目录）下全部子目录（含其它 hash）
        if let Some(vdir) = pom_dir.and_then(|p| p.parent()) {
            // dir=pom 文件路径：parent()=hash 目录，parent().parent()=version 目录（兄弟 hash 所在）
            if let Ok(sibs) = std::fs::read_dir(vdir) {
                for sd in sibs.flatten() {
                    if sd.path().is_dir() {
                        search_dirs.push(Some(sd.path()));
                    }
                }
            }
        }
        let mut found: Vec<PathBuf> = Vec::new();
        // F2：stem 前缀提到循环外（省重复分配）
        let main_jar = format!("{stem}.jar");
        let cls_prefix = format!("{stem}-");
        for sd in search_dirs.into_iter().flatten() {
            if let Some(list) = jars_by_dir.get(&sd) {
                for (name, path) in list {
                    let matches =
                        name == &main_jar || (name.starts_with(&cls_prefix) && name.ends_with(".jar"));
                    // F1：命中即标记 used（search_dirs 可能含重复目录——m2 下 pom_dir
                    // 与兄弟扫描重复进表，原写法 found 收集完才 insert 导致主 jar 双上传）
                    if matches && used.insert(path.clone()) {
                        found.push(path.clone());
                    }
                }
            }
        }
        found.sort();
        for path in found {
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            items.push(UploadItem {
                local: path,
                rel: format!("{rel_dir}/{name}"),
            });
        }
    }
    items.sort_by(|x, y| x.rel.cmp(&y.rel));
    (items, skipped)
}

/// 递归收集 .pom（解析）与 .jar（目录+文件名）
fn scan(root: &Path) -> (Vec<(PathBuf, crate::maven::pom::raw::RawPom)>, Vec<(PathBuf, String, PathBuf)>) {
    let mut poms = Vec::new();
    let mut jars = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&d) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
                    let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                    match ext {
                        "pom" => {
                            if let Ok(xml) = std::fs::read_to_string(&p) {
                                if let Ok(raw) = crate::maven::pom::raw::RawPom::parse(&xml) {
                                    poms.push((p, raw));
                                }
                            }
                        }
                        "jar" => jars.push((d.clone(), name, p)),
                        _ => {}
                    }
                }
            }
        }
    }
    poms.sort_by(|a, b| a.0.cmp(&b.0));
    (poms, jars)
}

/// 上传单个文件（抽象方法）：先 HEAD 探测——已存在（200/401/403 探测受限除外）则跳过，
/// 404 才 PUT。返回 (相对路径, 字节数；0=跳过) 或 (相对路径, 错误)。
async fn upload_one(
    client: &reqwest::Client,
    repo_url: &str,
    check_repo: Option<&str>,
    check_auth: Option<(&str, &str)>,
    user: &str,
    pass: &str,
    path: &Path,
    rel: &str,
) -> Result<(String, u64), (String, String)> {
    // A1：先 metadata 判大小——>5MB 直接流式（省一次全文件读+全量分配）；≤5MB 才整读
    let size = std::fs::metadata(path)
        .map_err(|e| (rel.to_string(), format!("stat: {e}")))?
        .len();
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let url = format!("{repo_url}/{rel}");
    log::DL_ACTIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    log::refresh_status();
    let started = Instant::now();
    // 存在性校验（--check-repo 提供时）：200 = 已存在跳过；404/其它 = PUT。
    // 未提供校验仓库时直接 PUT（覆盖语义）。
    if let Some(check) = check_repo {
        let check_url = format!("{}/{}", check.trim_end_matches('/'), rel);
        // None（异仓且未提供 --cu/--cp）= 真匿名——绝不把上传仓凭据发给第三方校验仓
        let head = match check_auth {
            Some((cu, cp)) => client.head(&check_url).basic_auth(cu, Some(cp)).send().await,
            None => client.head(&check_url).send().await,
        };
        if let Ok(h) = &head {
            if h.status().as_u16() == 200 {
                log::DL_ACTIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                log::DL_DONE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                log::refresh_status();
                log::verbose(format!("exists, skipped {rel}"));
                return Ok((rel.to_string(), 0));
            }
        }
    }
    let ct = if name.ends_with(".pom") { "application/xml" } else { "application/octet-stream" };
    log::uploading_line(rel);
    // PUT 带重试：429（限流）与 5xx 退避重试 3 次（1s/2s/4s——Nexus 默认限流较紧）
    // Bytes 引用计数克隆（原为整文件 memcpy）
    // F6+A1：>5MB 流式上传（未整读；ReaderStream 256KB chunk + 显式 Content-Length），
    // 小文件整读 + Bytes（重试零拷贝）
    let streaming = size > 5 * 1024 * 1024;
    let body_small = if streaming {
        None
    } else {
        let bytes = std::fs::read(path).map_err(|e| (rel.to_string(), format!("read: {e}")))?;
        Some(bytes::Bytes::from(bytes))
    };
    let resp;
    let mut retry = 0u32;
    loop {
        let body_req: reqwest::Body = if let Some(b) = &body_small {
            reqwest::Body::from(b.clone())
        } else {
            use tokio_util::io::ReaderStream;
            match tokio::fs::File::open(path).await {
                Ok(f) => reqwest::Body::wrap_stream(ReaderStream::with_capacity(f, 256 * 1024)),
                Err(e) => {
                    // 与传输失败路径对称（防计数缺口）
                    log::DL_ACTIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    log::DL_DONE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    log::DL_FAILED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return Err((rel.to_string(), format!("reopen: {e}")));
                }
            }
        };
        let r = client
            .put(&url)
            .basic_auth(user, Some(pass))
            .header("Content-Type", ct)
            .header("Content-Length", size)
            .body(body_req)
            .send()
            .await;
        match r {
            Ok(resp_ok) => {
                let code = resp_ok.status().as_u16();
                if (code == 429 || code >= 500) && retry < 3 {
                    retry += 1;
                    tokio::time::sleep(std::time::Duration::from_secs(1 << retry)).await;
                    continue;
                }
                resp = resp_ok;
                break;
            }
            Err(_) if retry < 3 => {
                retry += 1;
                tokio::time::sleep(std::time::Duration::from_secs(1 << retry)).await;
            }
            Err(e) => {
                // 无论成败先减量（与 download_single 一致——防在途计数泄漏）；
                // 传输失败也计入 DONE（与 HTTP 状态失败口径一致，进度可达分母）
                log::DL_ACTIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                log::DL_DONE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                log::DL_FAILED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                log::refresh_status();
                return Err((rel.to_string(), format!("{url}: {e}")));
            }
        }
    }

    log::DL_ACTIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    log::DL_BYTES.fetch_add(size, std::sync::atomic::Ordering::Relaxed);
    log::DL_DONE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if !resp.status().is_success() {
        log::DL_FAILED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    log::refresh_status();
    match resp.status().as_u16() {
            200 | 201 | 202 => {
                log::uploaded_line(rel, &log::fmt_bytes(size), &log::fmt_duration(started.elapsed()));
                Ok((rel.to_string(), size))
            }
            409 => Err((rel.to_string(), format!("{url} -> 409（已存在；release 仓库禁止覆盖）"))),
            401 => Err((rel.to_string(), format!("{url} -> 401（认证失败：检查 -u/-p）"))),
            403 => Err((rel.to_string(), format!(
                "{url} -> 403（无权限或被限流：确认账号有该仓库的部署权限；若因限流，降低 -j 并发）"
            ))),
            429 => Err((rel.to_string(), format!("{url} -> 429（限流：降低 -j 并发后重试）"))),
            code => Err((rel.to_string(), format!("{url} -> {code}"))),
    }
}
