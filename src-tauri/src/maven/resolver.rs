//! 传递依赖解析：层同步 BFS 收集（层内并行预取 POM）→ 每 key (g:a:ext:classifier)
//! 按 (depth, 全局 seq) nearest-wins 调解 → 落败子树整体剪枝。

use std::collections::HashMap;
use std::sync::Arc;

use crate::maven::error::{Error, Result};
use crate::maven::log;
use crate::maven::model::{transitive_scope, Coordinate, DepDecl, Exclusion, GaKey, Scope};
use crate::maven::pom::effective::{EffectivePom, ResolveContext};
use crate::maven::repo::RepoSet;
use crate::maven::version::is_range_or_meta;

/// 取消旗标：resolve 每层开始时检查，命中即立即返回已解析部分
pub type CancelFlag = std::sync::Arc<std::sync::atomic::AtomicBool>;

/// 前端交互树节点（arena 序列化产物）。
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNodeDto {
    pub id: usize,
    pub parent: Option<usize>,
    pub depth: usize,
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub packaging: String,
    pub classifier: Option<String>,
    pub scope: String,
    pub loser: bool,
    pub conflict_with: Option<String>,
    pub managed_from: Option<String>,
    pub children: Vec<usize>,
}

/// arena → 结构化节点清单（前端交互树数据源）。
pub fn export_nodes(outcome: &ResolveOutcome) -> Vec<TreeNodeDto> {
    outcome
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| TreeNodeDto {
            id: i,
            parent: n.parent,
            depth: n.depth,
            group: n.coord.group.clone(),
            artifact: n.coord.artifact.clone(),
            version: n.coord.version.clone(),
            packaging: n.coord.packaging.clone(),
            classifier: n.coord.classifier.clone(),
            scope: n.scope.as_str().to_string(),
            loser: n.loser,
            conflict_with: n.conflict_with.clone(),
            managed_from: n.managed_from.clone(),
            children: n.children.clone(),
        })
        .collect()
}

/// 依赖图节点（arena 索引式，nodes[parent] 索引恒小于 children）。
pub struct Node {
    pub coord: Coordinate,
    pub scope: Scope,
    pub depth: usize,
    /// 路径累积排除集（并集）
    pub exclusions: Vec<Exclusion>,
    pub children: Vec<usize>,
    pub parent: Option<usize>,
    /// 调解落败（tree 中显示 omitted for conflict）
    pub loser: bool,
    /// 被 winner 版本取代
    pub conflict_with: Option<String>,
    /// 版本被根 dependencyManagement 覆盖前的原值
    pub managed_from: Option<String>,
}

impl Node {
    pub fn key(&self) -> GaKey {
        self.coord.ga_key()
    }

    pub fn display(&self) -> String {
        let cls = self
            .coord
            .classifier
            .as_deref()
            .map(|c| format!(":{c}"))
            .unwrap_or_default();
        format!(
            "{}:{}:{}{} ({})",
            self.coord.group,
            self.coord.artifact,
            self.coord.version,
            cls,
            self.scope.as_str()
        )
    }
}

/// 解析入口的根请求。
pub enum RootRequest {
    /// `-g`：单 GAV（无根 depMgmt 约束）
    Gav(Coordinate),
    /// `-f` 的 .txt 清单：多个 GAV 全部作为根依赖（level0 展开）
    GavList(Vec<String>),
    /// `-f`：根 effective POM（deps 为直接依赖，dep_mgmt 为全图约束）
    Pom(Arc<EffectivePom>),
}

pub struct ResolveOutcome {
    /// 全图节点（含 loser 标记），索引即 seq 顺序
    pub nodes: Vec<Node>,
    /// 每 key 的幸存节点索引 = 下载清单
    pub winners: Vec<usize>,
    /// 解析期跳过的构件（POM 不存在等；不中断整体，最终汇总非零退出）
    pub failures: Vec<(String, String)>,
}

/// 解析主流程。
/// 每当在线确立一个 winner（每 key 首次出现的坐标）即通过 `on_winner` 投递——
/// 流水线化：解析端持续投递，下载端（DownloadPipeline）边收边下载。
pub async fn resolve(
    root: &RootRequest,
    ctx: Arc<ResolveContext>,
    repos: Arc<RepoSet>,
    no_transitive: bool,
    cancel: Option<CancelFlag>,
    on_winner: &tokio::sync::mpsc::Sender<Coordinate>,
) -> Result<ResolveOutcome> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut winner_map: HashMap<GaKey, usize> = HashMap::new();
    let mut failures: Vec<(String, String)> = Vec::new();

    // ---- level 0：根 direct 依赖 ----
    // root effective 借用：层内并行任务需要 'static，改持有 Arc<EffectivePom>
    let root_eff: Option<Arc<EffectivePom>> = match root {
        RootRequest::Gav(_) | RootRequest::GavList(_) => None,
        RootRequest::Pom(eff) => Some(eff.clone()),
    };
    let mut level: Vec<usize> = Vec::new();
    match root {
        RootRequest::GavList(gavs) => {
            for g in gavs {
                let coord = crate::maven::model::Coordinate::parse_gav(g)
                    .map_err(|e| Error::Other(format!("{e}")))?;
                let ci = push_node(&mut nodes, coord, Scope::Compile, Vec::new(), None);
                // 清单内重复 g:a（不同版本）走同款在线调解：先出现者胜，
                // 后到者标 loser 不投递（与实际下载/清单一致）
                let key = nodes[ci].key();
                match winner_map.get(&key) {
                    Some(&w) => {
                        nodes[ci].loser = true;
                        nodes[ci].conflict_with = Some(nodes[w].coord.version.clone());
                        log::warn(format!(
                            "清单内重复 {}: later version {} skipped (kept {})",
                            format!("{}:{}:{}", key.group, key.artifact, key.ext),
                            nodes[ci].coord.version,
                            nodes[w].coord.version
                        ));
                    }
                    None => {
                        winner_map.insert(key, ci);
                        level.push(ci);
                    }
                }
            }
        }
        RootRequest::Gav(coord) => {
            // optional 不适用于 -g 直接指定的坐标
            let ci = push_node(&mut nodes, coord.clone(), Scope::Compile, Vec::new(), None);
            winner_map.insert(nodes[ci].key(), ci);
            level.push(ci);
        }
        RootRequest::Pom(eff) => {
            // effective deps 已完成版本落实（depMgmt 补缺省 + 父子合并 + 插值）
            let direct: Vec<DepDecl> = bom_fallback_entries(&eff.deps, &eff.dep_mgmt);
            for d in &direct {
                if let Some(c) = decl_to_coord(d) {
                    let ci = push_node(
                        &mut nodes,
                        c,
                        parse_scope(d.scope.as_deref()),
                        d.exclusions.clone(),
                        None,
                    );
                    winner_map.insert(nodes[ci].key(), ci);
                    level.push(ci);
                }
            }
        }
    }

    // ---- 层同步 BFS：层内 effective 构建/展开并行（JoinSet），gather 按层序并入 ----
    // 保序原因：在线调解的 (depth, 层内序) tie-break 语义依赖确定的节点顺序
    // 解析层内并发上限：每任务一个在途 HTTP 请求，无上限会耗尽 fd（macOS 默认 256）
    let parse_gate = Arc::new(tokio::sync::Semaphore::new(32));
    while !level.is_empty() {
        // 取消检查点：每层开始时检查，命中立即返回已解析部分（Err 语义交给上层汇总）
        if let Some(flag) = &cancel {
            if flag.load(std::sync::atomic::Ordering::Relaxed) {
                let winners = {
                    let mut ws: Vec<usize> = winner_map.into_values().collect();
                    ws.sort_unstable();
                    ws
                };
                return Ok(ResolveOutcome { nodes, winners, failures });
            }
        }
        /// 单节点的展开产物（gather 时并入 arena）
        struct NodeWork {
            ni: usize,
            /// (child 坐标, scope, 累积排除, managed_from)
            children: Vec<(Coordinate, Scope, Vec<Exclusion>, Option<String>)>,
            /// 单条子依赖的局部失败（缺版本/范围解析失败——只跳过该条，不砍整节点子树）
            local_failures: Vec<(String, String)>,
        }
        let mut set = tokio::task::JoinSet::new();
        for &ni in &level {
            let (coord, scope, exclusions, depth) = {
                let n = &nodes[ni];
                (n.coord.clone(), n.scope, n.exclusions.clone(), n.depth)
            };
            let ctx = ctx.clone();
            let repos = repos.clone();
            let root_eff = root_eff.clone();
            let on_winner = on_winner.clone();
            let no_transitive = no_transitive;
            let _gate = parse_gate.clone().acquire_owned().await;
            set.spawn(async move {
                let _gate = _gate;
                // 单个构件解析失败（如 BOM 管理了但仓库从未发布的模块）宽容跳过
                let pom = match ctx.build_by_gav(&coord.group, &coord.artifact, &coord.version).await {
                    Ok(p) => p,
                    Err(e) => return (ni, Err((coord.gav(), e.to_string()))),
                };
                // 投递下载任务以 POM 实际 packaging 为准（packaging=pom 只有 POM，已写透）
                dispatch_after_pom(&coord, &pom, &on_winner).await;
                if no_transitive {
                    return (ni, Ok(NodeWork { ni, children: Vec::new(), local_failures: Vec::new() }));
                }
                // 根目标（depth==1）为纯 BOM 时管理条目视为直接依赖
                // 普通路径借用 effective deps（避免每节点整表克隆）；BOM fallback 才需要 owned
                let fallback: Vec<DepDecl>;
                let child_sources: &[DepDecl] = if !pom.deps.is_empty() {
                    &pom.deps
                } else if depth == 1 && !pom.dep_mgmt.is_empty() {
                    log::info(format!(
                        "`{}` declares no <dependencies>; treating {} managed entries as dependencies (BOM)",
                        coord.gav(),
                        pom.dep_mgmt.len()
                    ));
                    fallback = bom_fallback_entries(&[], &pom.dep_mgmt);
                    &fallback
                } else {
                    &[]
                };
                let root_mgmt = root_eff.as_ref().map(|e| &e.dep_mgmt);
                let mut children = Vec::new();
                let mut local_failures: Vec<(String, String)> = Vec::new();
                for d in child_sources {
                    // 根 depMgmt 全局约束：无论路径声明什么版本一律用管理版本
                    let (version, mut managed_from) = match root_mgmt.and_then(|m| m.get(&d.key())) {
                        Some(m) => match &m.as_ref().version {
                            Some(v) => (v.clone(), d.version.clone().filter(|orig| orig != v)),
                            None => (d.version.clone().unwrap_or_default(), None),
                        },
                        None => (d.version.clone().unwrap_or_default(), None),
                    };
                    let mut version = match version {
                        v if v.is_empty() => {
                            local_failures
                                .push((d.display(), "missing version (transitive)".to_string()));
                            continue;
                        }
                        v => v,
                    };
                    // 版本范围 / LATEST / RELEASE：拉 GA 级 metadata 解析（任务内并行）
                    if is_range_or_meta(&version) {
                        match repos.resolve_version(&d.group, &d.artifact, &version).await {
                            Ok(resolved) => {
                                if managed_from.is_none() {
                                    managed_from = Some(version);
                                }
                                version = resolved;
                            }
                            Err(reason) => {
                                local_failures.push((d.display(), reason));
                                continue;
                            }
                        }
                    }
                    // system scope 不下载；provided/test 声明不传递
                    let declared_raw = d.scope.as_deref().unwrap_or("compile");
                    if declared_raw == "system" {
                        continue;
                    }
                    let Some(inherited) = transitive_scope(scope, Scope::parse(declared_raw)) else {
                        continue;
                    };
                    // optional 边不展开（同 GA 从非 optional 路径到达仍会引入）
                    if d.optional {
                        continue;
                    }
                    // 累积排除集命中（含 * 通配）
                    if exclusions.iter().any(|e| e.matches(&d.group, &d.artifact)) {
                        continue;
                    }
                    let child_coord = Coordinate {
                        group: d.group.clone(),
                        artifact: d.artifact.clone(),
                        version,
                        packaging: sanitize_type(d.type_.as_deref()),
                        classifier: sanitize_classifier(&d.classifier),
                    };
                    let mut child_exclusions = exclusions.clone();
                    child_exclusions.extend(d.exclusions.iter().cloned());
                    children.push((child_coord, inherited, child_exclusions, managed_from));
                }
                (ni, Ok(NodeWork { ni, children, local_failures }))
            });
        }
        // gather：按 level 序落位（join_next 完成序不定，先收集再按序处理）
        let mut results: Vec<Option<std::result::Result<NodeWork, (String, String)>>> =
            level.iter().map(|_| None).collect();
        let mut by_ni: HashMap<usize, usize> = HashMap::new();
        for (idx, &ni) in level.iter().enumerate() {
            by_ni.insert(ni, idx);
        }
        while let Some(joined) = set.join_next().await {
            // 任务 panic 经 JoinError 收录为失败项（不 expect——release 的 panic=abort
            // 下二次 panic 会击杀整个应用）
            let (ni, r) = match joined {
                Ok(v) => v,
                Err(e) => {
                    failures.push((format!("resolve task: {e}"), "task panicked".into()));
                    continue;
                }
            };
            if let Some(&idx) = by_ni.get(&ni) {
                results[idx] = Some(r);
            }
        }
        let mut next: Vec<usize> = Vec::new();
        for r in results.into_iter().flatten() {
            let work = match r {
                Ok(w) => w,
                Err((gav, e)) => {
                    log::error(format!("skipping {gav}: {e}"));
                    failures.push((gav, e));
                    continue;
                }
            };
            let ni = work.ni;
            failures.extend(work.local_failures);
            for (child_coord, inherited, child_exclusions, managed_from) in work.children {
                // 依赖环：child 的 GA 出现在祖先链上则收边但不再展开
                let cyclic = has_ancestor(&nodes, ni, &child_coord);
                let ci = push_node(&mut nodes, child_coord, inherited, child_exclusions, Some(ni));
                nodes[ni].children.push(ci);
                nodes[ci].managed_from = managed_from;
                // 在线调解：BFS 层序保证 (depth, 层内序) 单调不减，后到的同 key 必落败
                let key = nodes[ci].key();
                match winner_map.get(&key) {
                    Some(&w) => {
                        nodes[ci].loser = true;
                        nodes[ci].conflict_with = Some(nodes[w].coord.version.clone());
                    }
                    None => {
                        winner_map.insert(key, ci);
                        if !cyclic {
                            next.push(ci);
                        }
                    }
                }
            }
        }
        level = next;
    }

    let winners = {
        let mut ws: Vec<usize> = winner_map.into_values().collect();
        ws.sort_unstable();
        ws
    };
    Ok(ResolveOutcome { nodes, winners, failures })
}

/// 纯 BOM fallback：无 `<dependencies>` 时取 depMgmt 条目（按 key 排序保证确定性；
/// 排除 scope=import 的子 BOM 条目——它们自身无主 artifact，其 pom 已在解析期写透）。
/// 版本范围条目照常返回，由 BFS 统一解析为具体版本。
fn bom_fallback_entries(deps: &[DepDecl], mgmt: &HashMap<GaKey, Arc<DepDecl>>) -> Vec<DepDecl> {
    if !deps.is_empty() || mgmt.is_empty() {
        return deps.to_vec();
    }
    let mut keys: Vec<&GaKey> = mgmt.keys().collect();
    keys.sort();
    keys.iter()
        .map(|k| (*mgmt[k]).clone())
        .filter(|d| d.scope.as_deref() != Some("import"))
        .collect()
}

/// 节点 POM 获取后投递下载任务：以 POM 实际 packaging 为准（声明 type 可能缺省/不准，
/// 如 -g 指向 BOM 时默认 jar 会 404）。packaging=pom 的只有 POM 本身（解析期已写透）。
async fn dispatch_after_pom(
    coord: &Coordinate,
    pom: &EffectivePom,
    on_winner: &tokio::sync::mpsc::Sender<Coordinate>,
) {
    if pom.coord.packaging == "pom" {
        return;
    }
    let mut coord = coord.clone();
    if pom.coord.packaging.contains("${") {
        // POM 自身 packaging 引用了未解析属性（如 javax.ws.rs-api 的
        // ${packaging.type} 定义在 JDK 条件激活的 profile 中）——不拿字面量
        // 覆盖，保留依赖声明 type（进入本函数前已 sanitize，不含 ${）
        if coord.packaging.contains("${") {
            coord.packaging = "jar".to_string();
        }
    } else {
        coord.packaging = pom.coord.packaging.clone();
    }
    // 先计数后 send：recv 侧立即 fetch_sub，若 send 后才 add 存在瞬时回绕窗口
    log::DL_QUEUED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if on_winner.send(coord).await.is_err() {
        log::DL_QUEUED.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// 依赖声明 → 坐标（version 已落实）；system scope 依赖跳过。
fn decl_to_coord(d: &DepDecl) -> Option<Coordinate> {
    if d.scope.as_deref() == Some("system") {
        return None;
    }
    Some(Coordinate {
        group: d.group.clone(),
        artifact: d.artifact.clone(),
        version: d.version.clone().unwrap_or_default(),
        packaging: sanitize_type(d.type_.as_deref()),
        classifier: sanitize_classifier(&d.classifier),
    })
}

/// type/packaging 含未解析属性（`${...}`，如某些 POM 的 `${packaging.type}` 定义在
/// 未启用的 profile 中）时回退 jar——拿字面量拼 URL 必然 404。
#[inline]
fn sanitize_type(t: Option<&str>) -> String {
    match t {
        Some(t) if t.contains("${") => {
            log::warn(format!("unresolved property in type `{t}`, falling back to jar"));
            "jar".to_string()
        }
        Some(t) => t.to_string(),
        None => "jar".to_string(),
    }
}

/// classifier 含未解析属性时置 None（下载主件）——字面量拼文件名必然 404。
/// （os.detected.classifier 等已由环境属性模拟解析，此处仅兜底真正无解的场景）
#[inline]
fn sanitize_classifier(c: &Option<String>) -> Option<String> {
    match c {
        Some(c) if c.contains("${") => {
            log::warn(format!("unresolved property in classifier `{c}`, ignoring classifier"));
            None
        }
        other => other.clone(),
    }
}

fn parse_scope(s: Option<&str>) -> Scope {
    Scope::parse(s.unwrap_or("compile"))
}

fn push_node(
    nodes: &mut Vec<Node>,
    coord: Coordinate,
    scope: Scope,
    exclusions: Vec<Exclusion>,
    parent: Option<usize>,
) -> usize {
    let idx = nodes.len();
    nodes.push(Node {
        coord,
        scope,
        depth: parent.map(|p| nodes[p].depth + 1).unwrap_or(1),
        exclusions,
        children: Vec::new(),
        parent,
        loser: false,
        conflict_with: None,
        managed_from: None,
    });
    idx
}

/// child 的 (g, a) 是否出现在 ni 的祖先链上（依赖环剪断）。
#[inline]
fn has_ancestor(nodes: &[Node], mut ni: usize, child: &Coordinate) -> bool {
    while let Some(p) = nodes[ni].parent {
        if nodes[p].coord.group == child.group && nodes[p].coord.artifact == child.artifact {
            return true;
        }
        ni = p;
    }
    false
}

/// 渲染依赖树（含 conflict/managed 标注）。
pub fn render_tree(outcome: &ResolveOutcome) -> String {
    let mut out = String::new();
    // 根的 children 即顶层（-f 多个根；-g 单根）
    let top: Vec<usize> = (0..outcome.nodes.len()).filter(|i| outcome.nodes[*i].parent.is_none()).collect();
    for (ti, &i) in top.iter().enumerate() {
        if ti > 0 {
            out.push('\n');
        }
        render_node(outcome, i, "", true, &mut out);
    }
    out
}

fn render_node(outcome: &ResolveOutcome, i: usize, prefix: &str, is_last: bool, out: &mut String) {
    let node = &outcome.nodes[i];
    if !prefix.is_empty() {
        out.push_str(if is_last { "└─ " } else { "├─ " });
    }
    out.push_str(&node.display());
    if let Some(m) = &node.managed_from {
        out.push_str(&format!(" (version managed from {m})"));
    }
    if node.loser {
        if let Some(w) = &node.conflict_with {
            out.push_str(&format!(" - omitted for conflict with {w}"));
        } else {
            out.push_str(" - omitted for duplicate");
        }
    }
    out.push('\n');
    let child_prefix = if prefix.is_empty() {
        "  ".to_string()
    } else {
        format!("{prefix}{}", if is_last { "  " } else { "│ " })
    };
    let n = node.children.len();
    for (ci, &ch) in node.children.iter().enumerate() {
        render_node(outcome, ch, &child_prefix, ci + 1 == n, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maven::pom::raw::{PomSource, RawPom};
    use std::collections::BTreeMap;

    #[derive(Clone)]
    struct MockSource {
        poms: BTreeMap<String, String>,
    }

    impl MockSource {
        fn new() -> Self {
            Self { poms: BTreeMap::new() }
        }
        fn add(mut self, g: &str, a: &str, v: &str, deps: &[(&str, &str, &str, &str)]) -> Self {
            let deps_xml = deps
                .iter()
                .map(|(dg, da, dv, ds)| {
                    let scope = if *ds == "compile" {
                        String::new()
                    } else {
                        format!("<scope>{ds}</scope>")
                    };
                    format!(
                        "<dependency><groupId>{dg}</groupId><artifactId>{da}</artifactId><version>{dv}</version>{scope}</dependency>"
                    )
                })
                .collect::<String>();
            let xml = format!(
                r#"<project><groupId>{g}</groupId><artifactId>{a}</artifactId><version>{v}</version>
                <dependencies>{deps_xml}</dependencies></project>"#
            );
            self.poms.insert(format!("{g}:{a}:{v}"), xml);
            self
        }
        fn add_raw(mut self, g: &str, a: &str, v: &str, xml: &str) -> Self {
            self.poms.insert(format!("{g}:{a}:{v}"), xml.to_string());
            self
        }
    }

    impl PomSource for MockSource {
        fn fetch_raw_pom<'b>(&'b self, g: &'b str, a: &'b str, v: &'b str)
            -> crate::maven::pom::raw::BoxFut<'b, Result<RawPom>> {
            Box::pin(async move {
                match self.poms.get(&format!("{g}:{a}:{v}")) {
                    Some(xml) => RawPom::parse(xml),
                    None => Err(Error::PomNotFound {
                        coord: format!("{g}:{a}:{v}"),
                        tried: "mock: not found".to_string(),
                    }),
                }
            })
        }
    }

    fn mock_repos() -> Arc<RepoSet> {
        Arc::new(RepoSet::new(
            Vec::new(),
            Arc::new(crate::maven::layout::Store::new(std::env::temp_dir().join("mvn-dr-test-unused"), crate::maven::layout::Layout::M2)),
            None,
        ).expect("test repos"))
    }

    async fn resolve_mock(root: RootRequest, src: &MockSource) -> Result<ResolveOutcome> {
        let ctx = Arc::new(ResolveContext::new(Arc::new(src.clone()), HashMap::new()));
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        resolve(&root, ctx, mock_repos(), false, None, &tx).await
    }

    fn winners_by_ga(out: &ResolveOutcome) -> Vec<String> {
        out.winners
            .iter()
            .map(|&i| format!("{}:{}:{}", out.nodes[i].coord.group, out.nodes[i].coord.artifact, out.nodes[i].coord.version))
            .collect()
    }

    #[tokio::test]
    async fn nearest_wins_and_loser_subtree_pruned() {
        // root(-f) → b:1.0 → x:1.0（depth 2，胜出）
        // root(-f) → c1:1.0 → y:1.0 → x:2.0（depth 3，落败）→ deep:1.0（随落败子树剪除）
        // mock 不提供 deep 的 POM：若实现未剪枝将直接 PomNotFound 失败
        let src = MockSource::new()
            .add("t", "b", "1.0", &[("t", "x", "1.0", "compile")])
            .add("t", "x", "1.0", &[])
            .add("t", "x", "2.0", &[("t", "deep", "1.0", "compile")])
            .add("t", "c1", "1.0", &[("t", "y", "1.0", "compile")])
            .add("t", "y", "1.0", &[("t", "x", "2.0", "compile")]);
        let root_pom = r#"<project><groupId>t</groupId><artifactId>r</artifactId><version>1.0</version>
          <dependencies>
            <dependency><groupId>t</groupId><artifactId>b</artifactId><version>1.0</version></dependency>
            <dependency><groupId>t</groupId><artifactId>c1</artifactId><version>1.0</version></dependency>
          </dependencies></project>"#;
        let eff = {
            let ctx = Arc::new(ResolveContext::new(Arc::new(src.clone()), HashMap::new()));
            ctx.build_from_local(RawPom::parse(root_pom).unwrap(), None, Vec::new()).await.unwrap()
        };
        let out = resolve_mock(RootRequest::Pom(eff), &src).await.unwrap();
        let winners = winners_by_ga(&out);
        assert!(winners.contains(&"t:x:1.0".to_string()), "winners: {winners:?}");
        // 落败的 x:2.0 在树中标记 conflict 但其子树不进入结果（也未被网络展开）
        let x2 = out.nodes.iter().find(|n| n.coord.artifact == "x" && n.coord.version == "2.0").unwrap();
        assert!(x2.loser);
        assert_eq!(x2.conflict_with.as_deref(), Some("1.0"));
        assert!(!winners.contains(&"t:deep:1.0".to_string()), "落败子树必须剪除: {winners:?}");
    }

    #[tokio::test]
    async fn same_depth_first_declared_wins() {
        // root（-f）同时声明 c1, c2；两者都引 x，先声明的 c1 路径胜出
        let src = MockSource::new()
            .add("t", "c1", "1.0", &[("t", "x", "1.0", "compile")])
            .add("t", "c2", "1.0", &[("t", "x", "2.0", "compile")])
            .add("t", "x", "1.0", &[])
            .add("t", "x", "2.0", &[]);
        let root_pom = r#"<project><groupId>t</groupId><artifactId>r</artifactId><version>1.0</version>
          <dependencies>
            <dependency><groupId>t</groupId><artifactId>c1</artifactId><version>1.0</version></dependency>
            <dependency><groupId>t</groupId><artifactId>c2</artifactId><version>1.0</version></dependency>
          </dependencies></project>"#;
        let eff = {
            let ctx = Arc::new(ResolveContext::new(Arc::new(src.clone()), HashMap::new()));
            ctx.build_from_local(RawPom::parse(root_pom).unwrap(), None, Vec::new()).await.unwrap()
        };
        let out = resolve_mock(RootRequest::Pom(eff), &src).await.unwrap();
        let winners = winners_by_ga(&out);
        assert!(winners.contains(&"t:x:1.0".to_string()), "同深先声明胜: {winners:?}");
        assert!(!winners.contains(&"t:x:2.0".to_string()));
    }

    #[tokio::test]
    async fn root_dep_mgmt_pins_transitive_version() {
        // 传递路径上 c:9.9，根 depMgmt 钉 c:1.0 → 全局 1.0
        let src = MockSource::new().add("t", "a", "1.0", &[("t", "c", "9.9", "compile")]).add("t", "c", "1.0", &[]);
        let root_pom = r#"<project><groupId>t</groupId><artifactId>r</artifactId><version>1.0</version>
          <dependencyManagement><dependencies>
            <dependency><groupId>t</groupId><artifactId>c</artifactId><version>1.0</version></dependency>
          </dependencies></dependencyManagement>
          <dependencies>
            <dependency><groupId>t</groupId><artifactId>a</artifactId><version>1.0</version></dependency>
          </dependencies></project>"#;
        let eff = {
            let ctx = Arc::new(ResolveContext::new(Arc::new(src.clone()), HashMap::new()));
            ctx.build_from_local(RawPom::parse(root_pom).unwrap(), None, Vec::new()).await.unwrap()
        };
        let out = resolve_mock(RootRequest::Pom(eff), &src).await.unwrap();
        let winners = winners_by_ga(&out);
        assert!(winners.contains(&"t:c:1.0".to_string()), "depMgmt 全局约束: {winners:?}");
        // managed_from 记录
        let c_node = out.nodes.iter().find(|n| n.coord.artifact == "c").unwrap();
        assert_eq!(c_node.managed_from.as_deref(), Some("9.9"));
    }

    #[tokio::test]
    async fn optional_and_exclusions_prune() {
        let src = MockSource::new()
            .add("t", "a", "1.0", &[("t", "opt", "1.0", "compile")])
            .add("t", "opt", "1.0", &[("t", "hidden", "1.0", "compile")])
            .add("t", "direct-opt", "1.0", &[]);
        let root_pom = r#"<project><groupId>t</groupId><artifactId>r</artifactId><version>1.0</version>
          <dependencies>
            <dependency>
              <groupId>t</groupId><artifactId>a</artifactId><version>1.0</version>
              <exclusions><exclusion><groupId>t</groupId><artifactId>opt</artifactId></exclusion></exclusions>
            </dependency>
            <dependency><groupId>t</groupId><artifactId>direct-opt</artifactId><version>1.0</version><optional>true</optional></dependency>
          </dependencies></project>"#;
        // opt 被排除；direct-opt 是根的直接 optional 依赖 → 保留
        let eff = {
            let ctx = Arc::new(ResolveContext::new(Arc::new(src.clone()), HashMap::new()));
            ctx.build_from_local(RawPom::parse(root_pom).unwrap(), None, Vec::new()).await.unwrap()
        };
        let out = resolve_mock(RootRequest::Pom(eff), &src).await.unwrap();
        let winners = winners_by_ga(&out);
        assert!(!winners.contains(&"t:opt:1.0".to_string()), "exclusion 生效");
        assert!(!winners.contains(&"t:hidden:1.0".to_string()));
        assert!(winners.contains(&"t:direct-opt:1.0".to_string()), "根 direct optional 保留: {winners:?}");
    }

    #[tokio::test]
    async fn optional_transitive_edge_not_expanded() {
        // a 的 pom 声明 optional 依赖 opt-dep → 不传递
        let src = MockSource::new().add_raw(
            "t",
            "a",
            "1.0",
            r#"<project><groupId>t</groupId><artifactId>a</artifactId><version>1.0</version>
              <dependencies>
                <dependency><groupId>t</groupId><artifactId>opt-dep</artifactId><version>1.0</version><optional>true</optional></dependency>
              </dependencies></project>"#,
        );
        let root = RootRequest::Gav(Coordinate::parse_gav("t:a:1.0").unwrap());
        let out = resolve_mock(root, &src).await.unwrap();
        assert!(!winners_by_ga(&out).contains(&"t:opt-dep:1.0".to_string()));
    }

    #[tokio::test]
    async fn test_scope_not_inherited() {
        // a 声明 test 依赖 t1；t1 声明 c —— test 边不传递
        let src = MockSource::new()
            .add_raw(
                "t",
                "a",
                "1.0",
                r#"<project><groupId>t</groupId><artifactId>a</artifactId><version>1.0</version>
                  <dependencies>
                    <dependency><groupId>t</groupId><artifactId>t1</artifactId><version>1.0</version><scope>test</scope></dependency>
                  </dependencies></project>"#,
            )
            .add("t", "t1", "1.0", &[("t", "c", "1.0", "compile")]);
        let root = RootRequest::Gav(Coordinate::parse_gav("t:a:1.0").unwrap());
        let out = resolve_mock(root, &src).await.unwrap();
        let winners = winners_by_ga(&out);
        assert!(!winners.contains(&"t:t1:1.0".to_string()), "test 传递丢弃");
        assert!(!winners.contains(&"t:c:1.0".to_string()));
    }

    #[tokio::test]
    async fn dependency_cycle_cut() {
        // a -> b -> a：环剪断不死循环
        let src = MockSource::new()
            .add("t", "a", "1.0", &[("t", "b", "1.0", "compile")])
            .add("t", "b", "1.0", &[("t", "a", "1.0", "compile")]);
        let root = RootRequest::Gav(Coordinate::parse_gav("t:a:1.0").unwrap());
        let out = resolve_mock(root, &src).await.unwrap();
        let winners = winners_by_ga(&out);
        assert!(winners.contains(&"t:a:1.0".to_string()));
        assert!(winners.contains(&"t:b:1.0".to_string()));
    }

    #[tokio::test]
    async fn pure_bom_root_expands_managed_entries() {
        // -g 指向纯 BOM（无 <dependencies>、depMgmt 非空）→ 管理条目视为直接依赖全部递归下载
        let src = MockSource::new().add_raw(
            "t",
            "the-bom",
            "1.0",
            r#"<project><groupId>t</groupId><artifactId>the-bom</artifactId><version>1.0</version>
              <packaging>pom</packaging>
              <dependencyManagement><dependencies>
                <dependency><groupId>t</groupId><artifactId>lib-a</artifactId><version>2.0</version></dependency>
                <dependency><groupId>t</groupId><artifactId>lib-b</artifactId><version>3.0</version></dependency>
                <dependency><groupId>t</groupId><artifactId>sub-bom</artifactId><version>1.0</version><type>pom</type><scope>import</scope></dependency>
              </dependencies></dependencyManagement>
            </project>"#,
        )
        .add_raw(
            "t",
            "sub-bom",
            "1.0",
            r#"<project><groupId>t</groupId><artifactId>sub-bom</artifactId><version>1.0</version><packaging>pom</packaging></project>"#,
        )
        .add("t", "lib-a", "2.0", &[("t", "lib-a-dep", "1.0", "compile")])
        .add("t", "lib-b", "3.0", &[])
        .add("t", "lib-a-dep", "1.0", &[]);
        let ctx = Arc::new(ResolveContext::new(Arc::new(src.clone()), HashMap::new()));
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let root = RootRequest::Gav(Coordinate::parse_gav("t:the-bom:1.0").unwrap());
        let out = resolve(&root, ctx, mock_repos(), false, None, &tx).await.unwrap();
        drop(tx);
        let winners = winners_by_ga(&out);
        // 管理条目进入下载清单
        assert!(winners.contains(&"t:lib-a:2.0".to_string()), "{winners:?}");
        // 并递归其传递依赖
        assert!(winners.contains(&"t:lib-a-dep:1.0".to_string()), "{winners:?}");
        // 投递的 jar 任务不含 BOM 自身与 import 子 BOM
        let mut dispatched = Vec::new();
        while let Ok(c) = rx.try_recv() {
            dispatched.push(c.artifact);
        }
        assert!(dispatched.contains(&"lib-a".to_string()));
        assert!(!dispatched.contains(&"the-bom".to_string()));
        assert!(!dispatched.contains(&"sub-bom".to_string()));
    }

    #[tokio::test]
    async fn bom_packaging_dispatches_no_artifact() {
        // -g 指向 packaging=pom 的 BOM：实际 packaging 以 POM 为准，BOM 自身不投递 jar 任务；
        // 管理条目按新语义展开为依赖（加入下载清单）
        let src = MockSource::new()
            .add_raw(
                "t",
                "the-bom",
                "1.0",
                r#"<project><groupId>t</groupId><artifactId>the-bom</artifactId><version>1.0</version>
                  <packaging>pom</packaging>
                  <dependencyManagement><dependencies>
                    <dependency><groupId>t</groupId><artifactId>managed</artifactId><version>2.0</version></dependency>
                  </dependencies></dependencyManagement>
                </project>"#,
            )
            .add("t", "managed", "2.0", &[]);
        let ctx = Arc::new(ResolveContext::new(Arc::new(src.clone()), HashMap::new()));
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let root = RootRequest::Gav(Coordinate::parse_gav("t:the-bom:1.0").unwrap());
        let out = resolve(&root, ctx, mock_repos(), false, None, &tx).await.unwrap();
        drop(tx);
        // BOM 自身不投递（POM 已写透）；管理条目正常投递
        let mut dispatched = Vec::new();
        while let Ok(c) = rx.try_recv() {
            dispatched.push(c.artifact);
        }
        assert!(!dispatched.contains(&"the-bom".to_string()));
        assert!(dispatched.contains(&"managed".to_string()));
        // 管理条目进入清单
        let winners = winners_by_ga(&out);
        assert!(winners.contains(&"t:managed:2.0".to_string()), "{winners:?}");
    }

    #[tokio::test]
    async fn dispatch_uses_actual_packaging_from_pom() {
        // -g 默认 packaging=jar，但 POM 实际声明 aar → 投递坐标用实际 packaging
        let src = MockSource::new().add_raw(
            "t",
            "lib",
            "1.0",
            r#"<project><groupId>t</groupId><artifactId>lib</artifactId><version>1.0</version>
              <packaging>aar</packaging>
            </project>"#,
        );
        let ctx = Arc::new(ResolveContext::new(Arc::new(src.clone()), HashMap::new()));
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let root = RootRequest::Gav(Coordinate::parse_gav("t:lib:1.0").unwrap());
        resolve(&root, ctx, mock_repos(), true, None, &tx).await.unwrap();
        drop(tx);
        let dispatched = rx.recv().await.unwrap();
        assert_eq!(dispatched.packaging, "aar");
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn no_transitive_only_direct() {
        let src = MockSource::new().add("t", "a", "1.0", &[("t", "x", "1.0", "compile")]).add("t", "x", "1.0", &[]);
        let ctx = Arc::new(ResolveContext::new(Arc::new(src.clone()), HashMap::new()));
        let root = RootRequest::Gav(Coordinate::parse_gav("t:a:1.0").unwrap());
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let out = resolve(&root, ctx, mock_repos(), true, None, &tx).await.unwrap();
        assert_eq!(winners_by_ga(&out), vec!["t:a:1.0".to_string()]);
    }
}
