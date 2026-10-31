//! effective POM 构建：parent 链合并、properties 迭代插值、
//! dependencyManagement 合并（含 BOM import）、`<dependencies>` 父子合并、版本落实。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::maven::error::{Error, Result};
use crate::maven::log;
use crate::maven::model::{Coordinate, DepDecl, GaKey};
use crate::maven::version::is_range_or_meta;

use super::raw::{PomSource, RawPom, RawRepo, RelativePath};

/// 构建完成的 effective POM。
#[derive(Debug)]
pub struct EffectivePom {
    /// 落实后的自身坐标（g/v 可能来自继承；packaging 含于 coord）
    pub coord: Coordinate,
    /// 有效依赖（插值 + 父子合并 + depMgmt 补缺省 + 版本已落实，保序）
    pub deps: Vec<DepDecl>,
    /// 值为 Arc：BOM import 展开时共享条目而非整表克隆（内存优化）
    pub dep_mgmt: HashMap<GaKey, Arc<DepDecl>>,
    /// 本 POM 链声明的仓库（仅供根模式并入仓库列表；传递 POM 的被忽略）
    pub repositories: Vec<RawRepo>,
}

type Gav = (String, String, String);

/// POM profile 激活判定（静态可判定部分）：
/// activeByDefault，或 jdk（按模拟 java.version 1.8 匹配单值前缀/区间），
/// 或 os（name/arch/family 按运行平台）；无法判定的条件视为不激活。
fn profile_active(p: &super::raw::RawProfile) -> bool {
    use crate::maven::version::cmp_version;
    const SIM_JAVA: &str = "1.8.0_482";
    if let Some(jdk) = &p.jdk {
        use std::cmp::Ordering::*;
        let m = if let Some((lo, hi_opt)) = jdk.split_once(',') {
            // 区间 [a,b) / [a,]
            let lo = lo.trim_start_matches(['[', '(']).trim();
            let hi = hi_opt.trim_end_matches([']', ')']).trim();
            let ge = cmp_version(SIM_JAVA, lo) != Less;
            let lt = if hi.is_empty() { true } else { cmp_version(SIM_JAVA, hi) == Less };
            ge && lt
        } else {
            // 单值前缀匹配（Maven 语义 "1.8" 匹配所有 1.8.x）
            SIM_JAVA.starts_with(jdk.as_str())
        };
        if m {
            return true;
        }
    }
    if p.os_family.is_some() || p.os_name.is_some() || p.os_arch.is_some() {
        let (fam, _name, arch) = (
            if cfg!(target_os = "macos") { "mac" } else if cfg!(target_os = "windows") { "windows" } else { "linux" },
            "",
            if cfg!(target_arch = "aarch64") { "aarch64" } else { "amd64" },
        );
        let f_ok = p.os_family.as_deref().map(|v| v.eq_ignore_ascii_case(fam)).unwrap_or(true);
        let n_ok = p
            .os_name
            .as_deref()
            .map(|v| v.to_ascii_lowercase().contains(fam))
            .unwrap_or(true);
        let a_ok = p.os_arch.as_deref().map(|v| v.eq_ignore_ascii_case(arch)).unwrap_or(true);
        if f_ok && n_ok && a_ok {
            return true;
        }
    }
    p.active_by_default
}

/// 解析上下文：POM 来源 + CLI 属性 + effective 记忆化。
/// 并发化设计：缓存/在途表锁保护，`&self` 方法可被多任务并发调用；
/// 同 key 并发构建通过 per-key 异步锁去重（后到者等待并复用结果），
/// BOM 环检测用调用栈（import_stack）而非锁——避免嵌套等待死锁。
pub struct ResolveContext {
    src: std::sync::Arc<dyn PomSource>,
    cli_props: Arc<HashMap<String, String>>,
    eff_cache: std::sync::Mutex<HashMap<Gav, Arc<EffectivePom>>>,
    /// per-key 去重锁（同 key 并发构建者排队）
    inflight: tokio::sync::Mutex<HashMap<Gav, Arc<tokio::sync::Mutex<()>>>>,
}

impl ResolveContext {
    pub fn new(src: std::sync::Arc<dyn PomSource>, cli_props: HashMap<String, String>) -> Self {
        Self {
            src,
            cli_props: Arc::new(cli_props),
            eff_cache: std::sync::Mutex::new(HashMap::new()),
            inflight: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// 按 GAV 构建（远程来源，无本地 relativePath），带记忆化与并发去重。
    pub fn build_by_gav<'b>(
        &'b self,
        g: &'b str,
        a: &'b str,
        v: &'b str,
    ) -> super::raw::BoxFut<'b, Result<Arc<EffectivePom>>> {
        self.build_inner(g, a, v, Vec::new())
    }

    fn build_inner<'b>(
        &'b self,
        g: &'b str,
        a: &'b str,
        v: &'b str,
        import_stack: Vec<Gav>,
    ) -> super::raw::BoxFut<'b, Result<Arc<EffectivePom>>> {
        Box::pin(async move {
            let key = (g.to_string(), a.to_string(), v.to_string());
            // 环检测前移（拿锁之前）：栈上已含本 key 说明经 BOM import 绕回自身——
            // 此时拿锁会等待自己持有的 per-key 锁（顶层入口不在 import 栈上），必死锁
            if import_stack.contains(&key) {
                return Err(Error::BomCycle { coord: format!("{g}:{a}:{v}") });
            }
            // 压入"自己"再下传：后续 import 绕回本 key 时由入口检测拦截
            let mut import_stack = import_stack;
            import_stack.push(key.clone());
            if let Some(cached) = self.eff_cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
                return Ok(cached.clone());
            }
            // per-key 去重：后到的并发同伴等待；环已在 import 展开处按栈拦截，
            // 能走到这里的等待者必是并行同伴而非递归重入
            // 防死锁设计：本 key 的锁只保护「查缓存 + fetch raw」这段无递归的临界区；
            // 任何会递归 acquire 其它 key 锁的阶段（relocation / BOM import 构建全程）
            // 都在释放本 key 锁之后进行。递归可能产生的重复构建由 eff_cache 收敛
            // （并发同 key 双方都会查缓存，最多多构建一次，正确性不受影响）。
            let key_lock = {
                let mut inflight = self.inflight.lock().await;
                inflight.entry(key.clone()).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
            };
            let raw = {
                let _guard = key_lock.lock().await;
                // 注意：std MutexGuard 不能跨 await——先取出结果释放锁
                let cached = self.eff_cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key).cloned();
                if let Some(cached) = cached {
                    self.inflight.lock().await.remove(&key);
                    return Ok(cached);
                }
                match self.src.fetch_raw_pom(g, a, v).await {
                    Ok(r) => r,
                    Err(e) => {
                        self.inflight.lock().await.remove(&key);
                        return Err(e);
                    }
                }
                // guard 在块尾释放——下方递归不再持有本 key 锁（防交叉互引死锁）
            };
            self.inflight.lock().await.remove(&key);
            // 释放锁后二次查缓存（锁持有期间的其它任务可能已构建）
            if let Some(cached) = self.eff_cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key).cloned() {
                return Ok(cached);
            }
            // relocation 跟随：坐标已迁移 → 改拉新坐标（version 缺省沿用旧值）
            if let Some(rel) = raw.relocation.clone() {
                let (rg, ra, rv) = (
                    rel.group,
                    rel.artifact,
                    if rel.version.is_empty() { v.to_string() } else { rel.version },
                );
                log::verbose(format!("relocation: {g}:{a}:{v} -> {rg}:{ra}:{rv}"));
                // 以新坐标构建并缓存到旧 key（复用结果）
                let eff = self.build_inner(&rg, &ra, &rv, import_stack.clone()).await?;
                self.eff_cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, eff.clone());
                return Ok(eff);
            }
            let eff = self.build_from_local(raw, None, import_stack).await?;
            self.eff_cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, eff.clone());
            Ok(eff)
        })
    }

    /// 模拟 Maven 插值可见的 Java system properties：
    /// - 运行时可取的真实值：os.* / user.* / file.separator / path.separator /
    ///   line.separator / java.io.tmpdir / os.detected.*（os-maven-plugin 约定）
    /// - JVM 模拟值（无 JVM 运行时；采用 JDK 8 形态，与常见构建环境一致）
    /// 用 entry().or_insert()：POM/profile 显式定义的同名属性优先。
    fn environment_properties() -> Vec<(&'static str, String)> {
        let (os_name, detected_name) = if cfg!(target_os = "macos") {
            ("Mac OS X", "osx")
        } else if cfg!(target_os = "windows") {
            ("Windows 10", "windows")
        } else if cfg!(target_os = "linux") {
            ("Linux", "linux")
        } else {
            ("Unknown", "unknown")
        };
        let arch = if cfg!(target_arch = "aarch64") {
            "aarch_64"
        } else if cfg!(target_arch = "x86_64") {
            "x86_64"
        } else {
            "unknown"
        };
        let user_home = home::home_dir().map(|h| h.to_string_lossy().to_string());
        let user_name = std::env::var(if cfg!(windows) { "USERNAME" } else { "USER" }).ok();
        let user_dir = std::env::current_dir().ok().map(|d| d.to_string_lossy().to_string());
        vec![
            ("os.name", os_name.to_string()),
            ("os.arch", if arch == "aarch_64" { "aarch64" } else { "amd64" }.to_string()),
            ("sun.arch.data.model", "64".to_string()),
            ("sun.cpu.endian", "little".to_string()),
            ("os.detected.name", detected_name.to_string()),
            ("os.detected.arch", arch.to_string()),
            // netty-native / rocksdb 等 native 依赖的 classifier 惯用形式
            ("os.detected.classifier", format!("{detected_name}-{arch}")),
            ("file.separator", std::path::MAIN_SEPARATOR.to_string()),
            ("path.separator", if cfg!(windows) { ";" } else { ":" }.to_string()),
            ("line.separator", "\n".to_string()),
            ("java.io.tmpdir", std::env::temp_dir().to_string_lossy().to_string()),
            // JVM 模拟（JDK 8 形态）
            ("java.version", "1.8.0_482".to_string()),
            ("java.specification.version", "1.8".to_string()),
            ("java.runtime.version", "1.8.0_482-b08".to_string()),
            ("java.vm.version", "25.482-b08".to_string()),
            ("java.vendor", "Azul Systems, Inc.".to_string()),
            ("java.vm.name", "OpenJDK 64-Bit Server VM".to_string()),
            ("java.class.version", "52.0".to_string()),
            ("file.encoding", "UTF-8".to_string()),
            ("user.timezone", "".to_string()),
        ]
        .into_iter()
        .chain(user_home.map(|v| ("user.home", v)))
        .chain(user_name.map(|v| ("user.name", v)))
        .chain(user_dir.map(|v| ("user.dir", v)))
        .collect()
    }

    /// 从已取得的 raw POM 构建 effective。
    /// `root_dir`：-f 模式根 POM 所在目录（relativePath 基准）；None = 全远程。
    /// `import_stack`：BOM import 调用链（环检测）。
    pub async fn build_from_local(
        &self,
        root: RawPom,
        root_dir: Option<&Path>,
        import_stack: Vec<Gav>,
    ) -> Result<Arc<EffectivePom>> {
        let src = self.src.clone();
        let cli_props = self.cli_props.clone();

        // ---- 1. parent 链（自身 → 最远祖先）----
        let mut chain: Vec<RawPom> = vec![root];
        let mut seen: HashSet<Gav> = HashSet::new();
        while let Some(parent) = chain.last().and_then(|p| p.parent.clone()) {
            let key = (parent.group.clone(), parent.artifact.clone(), parent.version.clone());
            if !seen.insert(key.clone()) {
                return Err(Error::ParentCycle {
                    coord: format!("{}:{}:{}", key.0, key.1, key.2),
                });
            }
            if chain.len() >= 50 {
                return Err(Error::Other(format!("parent chain too deep (>50) at {}", chain.len() + 1)));
            }
            // relativePath 仅对 -f 根 POM 生效（链上其余 POM 无本地文件系统上下文）
            let local = if chain.len() == 1 {
                try_local_parent(root_dir, &chain[0].relative_path, &key)
            } else {
                None
            };
            let raw = match local {
                Some(r) => r,
                None => src.fetch_raw_pom(&key.0, &key.1, &key.2).await?,
            };
            chain.push(raw);
        }
        chain.reverse(); // 最远祖先在前，根在后

        // ---- 2. resolved 坐标（g/v 沿链继承）----
        let mut resolved: Vec<(String, String, String)> = Vec::with_capacity(chain.len());
        for (i, p) in chain.iter().enumerate() {
            let (pg, pv) = if i == 0 {
                (None, None)
            } else {
                (Some(resolved[i - 1].0.clone()), Some(resolved[i - 1].2.clone()))
            };
            resolved.push((
                p.group.clone().or(pg).unwrap_or_default(),
                p.artifact.clone(),
                p.version.clone().or(pv).unwrap_or_default(),
            ));
        }
        let (root_g, root_a, root_v) = resolved.last().ok_or_else(|| Error::Other("empty pom chain".into()))?;
        if root_g.is_empty() || root_a.is_empty() || root_v.is_empty() {
            return Err(Error::Other(format!(
                "cannot resolve root coordinate of pom `{root_a}` (missing groupId/version inheritance)"
            )));
        }

        // ---- 3. properties 合并 + 迭代插值，resolved 坐标随后插值（${revision} 支持）----
        let mut props: HashMap<String, String> = HashMap::new();
        // 内置属性（根视角）；project.parent.* 指根的直接 parent = chain[len-2]
        let mut builtin = |k: &str, v: &str| {
            props.insert(k.to_string(), v.to_string());
        };
        builtin("project.groupId", root_g);
        builtin("project.artifactId", root_a);
        builtin("project.version", root_v);
        if let Some(last) = chain.last() {
            builtin("project.packaging", &last.packaging);
        }
        builtin("pom.groupId", root_g);
        builtin("pom.artifactId", root_a);
        builtin("pom.version", root_v);
        if chain.len() >= 2 {
            let (pg, pa, pv) = &resolved[chain.len() - 2];
            builtin("project.parent.groupId", pg);
            builtin("project.parent.artifactId", pa);
            builtin("project.parent.version", pv);
        }
        if let Some(dir) = root_dir {
            builtin("project.basedir", &dir.to_string_lossy());
        }
        // 链属性（顶→尾，近者覆盖）；activeByDefault profile 属性（如 ${packaging.type}）；CLI -D 最高
        for p in &chain {
            for (k, v) in &p.properties {
                props.insert(k.clone(), v.clone());
            }
        }
        for p in &chain {
            for prof in &p.profiles {
                if profile_active(prof) {
                    for (k, v) in &prof.properties {
                        props.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        // 环境属性模拟（Maven 的 Java system properties / os-maven-plugin 注入）：
        // os.name 等供插值；os.detected.classifier 供 netty 等 native 依赖的 classifier
        for (k, v) in Self::environment_properties() {
            props.entry(k.to_string()).or_insert_with(|| v.clone());
        }
        for (k, v) in cli_props.iter() {
            props.insert(k.clone(), v.clone());
        }
        // 属性值自身的 ${...} 引用迭代展开到不动点（≤5 轮；无任何 ${ 时跳过整轮重建）
        if !props.values().any(|v| v.contains("${")) {
            // 无插值需求
        } else {
        for _ in 0..5 {
            let next: Vec<(String, String)> = props
                .iter()
                .map(|(k, v)| (k.clone(), interpolate(v, &props)))
                .collect();
            let changed = next.iter().any(|(k, v)| props.get(k) != Some(v));
            for (k, v) in next {
                props.insert(k, v);
            }
            if !changed {
                break;
            }
        }
        }
        // resolved 坐标插值（CI-friendly ${revision}）
        let root_g = interpolate(root_g, &props);
        let root_a = interpolate(root_a, &props);
        let root_v = interpolate(root_v, &props);

        // ---- 4. depMgmt 合并（顶→尾；本 POM 直接 > 父链 > import 只填补；
        //      激活 profile 的 depMgmt 视作该 POM 的一部分，优先级同本 POM 直接条目）----
        let mut mgmt: HashMap<GaKey, Arc<DepDecl>> = HashMap::new();
        for p in &chain {
            let mut imports: Vec<DepDecl> = Vec::new();
            // 该 POM 的全部 mgmt 条目 = 声明条目 + 激活 profile 条目（按 profile 声明序）
            let mut entries: Vec<&crate::maven::model::DepDecl> = p.dep_mgmt.iter().collect();
            for prof in &p.profiles {
                if profile_active(prof) {
                    entries.extend(prof.dep_mgmt.iter());
                }
            }
            for e in entries {
                let e = interpolate_decl(e, &props);
                if e.scope.as_deref() == Some("import") {
                    imports.push(e);
                } else {
                    mgmt.insert(e.key(), Arc::new(e));
                }
            }
            for imp in imports {
                let Some(v) = imp.version.clone() else {
                    return Err(Error::MissingVersion { dep: imp.display() });
                };
                log::verbose(format!("importing BOM {}:{}:{v}", imp.group, imp.artifact));
                // 环检测在 build_inner 入口（拿锁前，检测后压自身）；此处原样传递调用链
                let bom = self
                    .build_inner(&imp.group, &imp.artifact, &v, import_stack.clone())
                    .await?;
                for (k, be) in &bom.dep_mgmt {
                    // Arc clone：共享 BOM 条目（原为整条 DepDecl 深拷贝）
                    mgmt.entry(k.clone()).or_insert_with(|| Arc::clone(be));
                }
            }
        }

        // ---- 5. `<dependencies>` 父子合并（根在前，父追加未冲突项；
        //      激活 profile 的 deps 并入该 POM 的依赖列表——profile 条目按声明序参与同规则合并）----
        let mut deps: Vec<DepDecl> = Vec::new();
        let mut dep_keys: HashSet<GaKey> = HashSet::new();
        for p in chain.iter().rev() {
            let mut decls: Vec<&crate::maven::model::DepDecl> = p.dependencies.iter().collect();
            for prof in &p.profiles {
                if profile_active(prof) {
                    decls.extend(prof.dependencies.iter());
                }
            }
            for d in decls {
                let d = interpolate_decl(d, &props);
                if dep_keys.insert(d.key()) {
                    deps.push(d);
                }
            }
        }

        // ---- 6. 版本落实 + depMgmt 补缺省（单条失败剔除并警告，不砍整 POM）----
        let mut dropped: Vec<GaKey> = Vec::new();
        for d in &mut deps {
            if let Some(m) = mgmt.get(&d.key()) {
                // 借用按需取字段（原为整条 DepDecl 深拷贝）
                if d.version.is_none() {
                    d.version = m.version.clone();
                }
                if d.scope.is_none() {
                    d.scope = m.scope.clone();
                }
                if d.type_.is_none() {
                    d.type_ = m.type_.clone();
                }
                if d.classifier.is_none() {
                    d.classifier = m.classifier.clone();
                }
                if d.exclusions.is_empty() {
                    d.exclusions = m.exclusions.clone();
                }
            }
            let Some(v) = d.version.clone() else {
                log::warn(format!("dropping dependency without version: {}", d.display()));
                dropped.push(d.key());
                continue;
            };
            // 版本范围 / LATEST / RELEASE：经 PomSource 解析为具体版本（取满足范围的最高）
            let v = if is_range_or_meta(&v) {
                let src = self.src.clone();
                match src.resolve_version(&d.group, &d.artifact, &v).await {
                    Ok(resolved) => {
                        log::verbose(format!("resolved {} `{v}` -> {resolved}", d.display()));
                        resolved
                    }
                    Err(reason) => {
                        log::warn(format!("dropping {}: {reason}", d.display()));
                        dropped.push(d.key());
                        continue;
                    }
                }
            } else {
                v
            };
            if v.contains("${") {
                log::warn(format!("unresolved property in version of {}: {v}", d.display()));
            }
            d.version = Some(v);
        }
        if !dropped.is_empty() {
            deps.retain(|d| !dropped.contains(&d.key()));
        }

        // ---- 7. 链上仓库（去重 by id，仅供根使用）----
        let mut repositories: Vec<RawRepo> = Vec::new();
        let mut repo_ids: HashSet<String> = HashSet::new();
        for p in chain.iter().rev() {
            for r in &p.repositories {
                if repo_ids.insert(r.id.clone()) {
                    repositories.push(r.clone());
                }
            }
            for prof in &p.profiles {
                if profile_active(prof) {
                    for r in &prof.repositories {
                        if repo_ids.insert(r.id.clone()) {
                            repositories.push(r.clone());
                        }
                    }
                }
            }
        }

        let packaging = {
            let p = chain.last().map(|c| c.packaging.as_str()).unwrap_or("jar");
            if p.contains("${") {
                // 直接指出本地落盘的 pom 文件路径（可打开核查）
                let pom_file = crate::maven::repo::disk_pom_path(&root_g, &root_a, &root_v);
                log::warn(format!(
                    "unresolved property in packaging `{p}` of {}:{root_a}（pom: {}）, falling back to jar",
                    root_g,
                    pom_file.display()
                ));
                "jar".to_string()
            } else {
                p.to_string()
            }
        };
        Ok(Arc::new(EffectivePom {
            coord: Coordinate {
                group: root_g.clone(),
                artifact: root_a.clone(),
                version: root_v.clone(),
                packaging,
                classifier: None,
            },
            deps,
            dep_mgmt: mgmt,
            repositories,
        }))
    }
}

/// relativePath 本地父 POM 查找：命中返回 Some，未命中/禁用/GAV 不匹配返回 None（回退远程）。
fn try_local_parent(root_dir: Option<&Path>, rel: &RelativePath, expected: &Gav) -> Option<RawPom> {
    let dir = root_dir?;
    let rel_path: PathBuf = match rel {
        RelativePath::Disabled => return None,
        RelativePath::Default => "../pom.xml".into(),
        RelativePath::Explicit(p) => p.into(),
    };
    let path = dir.join(rel_path);
    let xml = std::fs::read_to_string(&path).ok()?;
    let pom = RawPom::parse(&xml).ok()?;
    // GAV 一致性校验：artifact 必须匹配；group/version 未声明（继承自更上层）时不校验
    let group_ok = pom.group.as_ref().map(|g| g == &expected.0).unwrap_or(true);
    let version_ok = pom.version.as_ref().map(|v| v == &expected.2).unwrap_or(true);
    if pom.artifact == expected.1 && group_ok && version_ok {
        Some(pom)
    } else {
        log::warn(format!(
            "local parent {} GAV mismatch (expected {}:{}:{}), falling back to remote",
            path.display(),
            expected.0,
            expected.1,
            expected.2
        ));
        None
    }
}

/// `${key}` 插值；未解析的引用保留字面量（Maven 行为）。
fn interpolate(s: &str, props: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let key = &after[..end];
                match props.get(key) {
                    Some(v) => out.push_str(v),
                    None => {
                        // 保留字面量 ${key}
                        out.push_str("${");
                        out.push_str(key);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn interpolate_decl(d: &DepDecl, props: &HashMap<String, String>) -> DepDecl {
    let ip = |s: &Option<String>| s.as_ref().map(|v| interpolate(v, props));
    DepDecl {
        group: interpolate(&d.group, props),
        artifact: interpolate(&d.artifact, props),
        version: ip(&d.version),
        scope: ip(&d.scope),
        optional: d.optional,
        type_: ip(&d.type_),
        classifier: ip(&d.classifier),
        exclusions: d
            .exclusions
            .iter()
            .map(|e| crate::maven::model::Exclusion {
                group: interpolate(&e.group, props),
                artifact: interpolate(&e.artifact, props),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maven::model::Exclusion;
    use std::collections::BTreeMap;

    /// 测试用 PomSource：预置 (g:a:v) → POM XML。
    struct MockSource {
        poms: BTreeMap<String, String>,
    }

    impl MockSource {
        fn new() -> Self {
            Self { poms: BTreeMap::new() }
        }
        fn add(mut self, g: &str, a: &str, v: &str, xml: &str) -> Self {
            self.poms.insert(format!("{g}:{a}:{v}"), xml.to_string());
            self
        }
    }

    impl PomSource for MockSource {
        fn fetch_raw_pom<'b>(&'b self, g: &'b str, a: &'b str, v: &'b str)
            -> super::super::raw::BoxFut<'b, Result<RawPom>> {
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

    fn pom(xml: &str) -> RawPom {
        RawPom::parse(xml).unwrap()
    }

    async fn build(root_xml: &str) -> Result<Arc<EffectivePom>> {
        let src = std::sync::Arc::new(MockSource::new());
        let ctx = ResolveContext::new(src, HashMap::new());
        ctx.build_from_local(pom(root_xml), None, Vec::new()).await
    }

    fn find_dep<'e>(eff: &'e EffectivePom, a: &str) -> &'e DepDecl {
        eff.deps.iter().find(|d| d.artifact == a).unwrap_or_else(|| panic!("dep {a} not found"))
    }

    #[tokio::test]
    async fn property_interpolation_chain() {
        let eff = build(
            r#"<project>
              <groupId>g</groupId><artifactId>root</artifactId><version>1.0</version>
              <properties>
                <a>${b}</a><b>value-b</b><v>1.2.3</v>
              </properties>
              <dependencies>
                <dependency><groupId>g</groupId><artifactId>x</artifactId><version>${v}</version></dependency>
                <dependency><groupId>${a}</groupId><artifactId>y</artifactId><version>${missing}</version></dependency>
              </dependencies>
            </project>"#,
        )
        .await
        .unwrap();
        assert_eq!(find_dep(&eff, "x").version.as_deref(), Some("1.2.3"));
        // 迭代展开 a -> b
        assert_eq!(find_dep(&eff, "y").group, "value-b");
        // 未解析属性保留字面量
        assert_eq!(find_dep(&eff, "y").version.as_deref(), Some("${missing}"));
    }

    #[tokio::test]
    async fn project_builtin_properties() {
        let eff = build(
            r#"<project>
              <groupId>g</groupId><artifactId>root</artifactId><version>1.0</version>
              <dependencies>
                <dependency><groupId>${project.groupId}</groupId><artifactId>x</artifactId><version>${project.version}</version></dependency>
              </dependencies>
            </project>"#,
        )
        .await
        .unwrap();
        assert_eq!(find_dep(&eff, "x").group, "g");
        assert_eq!(find_dep(&eff, "x").version.as_deref(), Some("1.0"));
    }

    #[tokio::test]
    async fn parent_deps_merge_and_inheritance() {
        let src = MockSource::new().add(
            "gp",
            "parent",
            "1.0",
            r#"<project>
              <groupId>gp</groupId><artifactId>parent</artifactId><version>1.0</version>
              <properties><shared.v>2.0</shared.v></properties>
              <dependencies>
                <dependency><groupId>g</groupId><artifactId>from-parent</artifactId><version>${shared.v}</version></dependency>
                <dependency><groupId>g</groupId><artifactId>overridden</artifactId><version>1.0</version></dependency>
              </dependencies>
            </project>"#,
        );
        let ctx = ResolveContext::new(std::sync::Arc::new(src), HashMap::new());
        let eff = ctx
            .build_from_local(
                pom(
                    r#"<project>
                      <parent><groupId>gp</groupId><artifactId>parent</artifactId><version>1.0</version></parent>
                      <artifactId>child</artifactId>
                      <dependencies>
                        <dependency><groupId>g</groupId><artifactId>own</artifactId><version>3.0</version></dependency>
                        <dependency><groupId>g</groupId><artifactId>overridden</artifactId><version>9.9</version></dependency>
                      </dependencies>
                    </project>"#,
                ),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        // g/v 继承自 parent
        assert_eq!(eff.coord.group, "gp");
        assert_eq!(eff.coord.version, "1.0");
        assert_eq!(eff.coord.artifact, "child");
        // 子声明覆盖父
        assert_eq!(find_dep(&eff, "overridden").version.as_deref(), Some("9.9"));
        // 父属性在子 POM 插值生效
        assert_eq!(find_dep(&eff, "from-parent").version.as_deref(), Some("2.0"));
        assert_eq!(eff.deps.len(), 3);
    }

    #[tokio::test]
    async fn dep_mgmt_priority_direct_over_bom() {
        let src = MockSource::new().add(
            "bom",
            "the-bom",
            "1.0",
            r#"<project>
              <groupId>bom</groupId><artifactId>the-bom</artifactId><version>1.0</version>
              <dependencyManagement><dependencies>
                <dependency><groupId>g</groupId><artifactId>managed</artifactId><version>1.0-bom</version></dependency>
                <dependency><groupId>g</groupId><artifactId>bom-only</artifactId><version>2.0</version></dependency>
              </dependencies></dependencyManagement>
            </project>"#,
        );
        let ctx = ResolveContext::new(std::sync::Arc::new(src), HashMap::new());
        let eff = ctx
            .build_from_local(
                pom(
                    r#"<project>
                      <groupId>g</groupId><artifactId>root</artifactId><version>1.0</version>
                      <dependencyManagement><dependencies>
                        <dependency><groupId>g</groupId><artifactId>managed</artifactId><version>1.0-direct</version></dependency>
                        <dependency><groupId>bom</groupId><artifactId>the-bom</artifactId><version>1.0</version><type>pom</type><scope>import</scope></dependency>
                      </dependencies></dependencyManagement>
                      <dependencies>
                        <dependency><groupId>g</groupId><artifactId>managed</artifactId></dependency>
                        <dependency><groupId>g</groupId><artifactId>bom-only</artifactId></dependency>
                      </dependencies>
                    </project>"#,
                ),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        // 直接条目 > BOM（BOM 只填补）
        assert_eq!(find_dep(&eff, "managed").version.as_deref(), Some("1.0-direct"));
        assert_eq!(find_dep(&eff, "bom-only").version.as_deref(), Some("2.0"));
    }

    #[tokio::test]
    async fn parent_mgmt_beats_child_bom_import() {
        // 文档化行为：父链旧条目赢过子新 import 的 BOM
        let src = MockSource::new()
            .add(
                "gp",
                "parent",
                "1.0",
                r#"<project><groupId>gp</groupId><artifactId>parent</artifactId><version>1.0</version>
                  <dependencyManagement><dependencies>
                    <dependency><groupId>g</groupId><artifactId>x</artifactId><version>from-parent</version></dependency>
                  </dependencies></dependencyManagement>
                </project>"#,
            )
            .add(
                "bom",
                "b",
                "1.0",
                r#"<project><groupId>bom</groupId><artifactId>b</artifactId><version>1.0</version>
                  <dependencyManagement><dependencies>
                    <dependency><groupId>g</groupId><artifactId>x</artifactId><version>from-bom</version></dependency>
                  </dependencies></dependencyManagement>
                </project>"#,
            );
        let ctx = ResolveContext::new(std::sync::Arc::new(src), HashMap::new());
        let eff = ctx
            .build_from_local(
                pom(
                    r#"<project>
                      <parent><groupId>gp</groupId><artifactId>parent</artifactId><version>1.0</version></parent>
                      <artifactId>child</artifactId>
                      <dependencyManagement><dependencies>
                        <dependency><groupId>bom</groupId><artifactId>b</artifactId><version>1.0</version><type>pom</type><scope>import</scope></dependency>
                      </dependencies></dependencyManagement>
                      <dependencies>
                        <dependency><groupId>g</groupId><artifactId>x</artifactId></dependency>
                      </dependencies>
                    </project>"#,
                ),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(find_dep(&eff, "x").version.as_deref(), Some("from-parent"));
    }

    #[tokio::test]
    async fn missing_version_dropped_not_fatal() {
        // 语义：单条缺版本剔除该条并警告（不砍整 POM 的其它依赖）
        let eff = build(
            r#"<project><groupId>g</groupId><artifactId>root</artifactId><version>1.0</version>
              <dependencies>
                <dependency><groupId>g</groupId><artifactId>noversion</artifactId></dependency>
                <dependency><groupId>g</groupId><artifactId>ok</artifactId><version>2.0</version></dependency>
              </dependencies>
            </project>"#,
        )
        .await
        .unwrap();
        assert_eq!(eff.deps.len(), 1);
        assert_eq!(eff.deps[0].artifact, "ok");
    }

    #[tokio::test]
    async fn version_range_unresolvable_dropped() {
        // Mock 的 resolve_version 不支持 → 剔除该条而非整体失败
        let eff = build(
            r#"<project><groupId>g</groupId><artifactId>root</artifactId><version>1.0</version>
              <dependencies>
                <dependency><groupId>g</groupId><artifactId>r</artifactId><version>[1.0,2.0)</version></dependency>
                <dependency><groupId>g</groupId><artifactId>ok</artifactId><version>2.0</version></dependency>
              </dependencies>
            </project>"#,
        )
        .await
        .unwrap();
        assert_eq!(eff.deps.len(), 1);
        assert_eq!(eff.deps[0].artifact, "ok");
    }

    #[tokio::test]
    async fn mgmt_fills_scope_and_exclusions() {
        let eff = build(
            r#"<project><groupId>g</groupId><artifactId>root</artifactId><version>1.0</version>
              <dependencyManagement><dependencies>
                <dependency>
                  <groupId>g</groupId><artifactId>x</artifactId><version>1.0</version>
                  <scope>runtime</scope>
                  <exclusions><exclusion><groupId>bad</groupId><artifactId>y</artifactId></exclusion></exclusions>
                </dependency>
              </dependencies></dependencyManagement>
              <dependencies>
                <dependency><groupId>g</groupId><artifactId>x</artifactId></dependency>
              </dependencies>
            </project>"#,
        )
        .await
        .unwrap();
        let d = find_dep(&eff, "x");
        assert_eq!(d.scope.as_deref(), Some("runtime"));
        assert_eq!(d.exclusions, vec![Exclusion { group: "bad".into(), artifact: "y".into() }]);
    }

    #[tokio::test]
    async fn bom_import_cycle_detected() {
        let src = MockSource::new().add(
            "a",
            "loop",
            "1.0",
            r#"<project><groupId>a</groupId><artifactId>loop</artifactId><version>1.0</version>
              <dependencyManagement><dependencies>
                <dependency><groupId>a</groupId><artifactId>loop</artifactId><version>1.0</version><type>pom</type><scope>import</scope></dependency>
              </dependencies></dependencyManagement>
            </project>"#,
        );
        let ctx = ResolveContext::new(std::sync::Arc::new(src), HashMap::new());
        let err = ctx.build_by_gav("a", "loop", "1.0").await.unwrap_err();
        assert!(matches!(err, Error::BomCycle { .. }), "got: {err:?}");
    }

    #[tokio::test]
    async fn cli_props_override() {
        let src = MockSource::new();
        let mut props = HashMap::new();
        props.insert("revision".to_string(), "9.9".to_string());
        let ctx = ResolveContext::new(std::sync::Arc::new(src), props);
        let eff = ctx
            .build_from_local(
                pom(
                    r#"<project><groupId>g</groupId><artifactId>root</artifactId><version>${revision}</version>
                      <dependencies>
                        <dependency><groupId>g</groupId><artifactId>x</artifactId><version>${revision}</version></dependency>
                      </dependencies>
                    </project>"#,
                ),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(eff.coord.version, "9.9");
        assert_eq!(find_dep(&eff, "x").version.as_deref(), Some("9.9"));
    }
}
