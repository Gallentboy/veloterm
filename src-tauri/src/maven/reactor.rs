//! Maven 多模块 Reactor（工作空间反应堆）：
//! 自动探测本地多模块多层级工程目录树，向上沿着 <parent><relativePath> 级联追溯根 POM，
//! 向下沿着 <modules> 递归发现所有子模块，建立本地模块坐标索引与依赖聚合。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::maven::error::{Error, Result};
use crate::maven::log;
use crate::maven::model::{Coordinate, DepDecl, GaKey};
use crate::maven::pom::effective::{EffectivePom, ResolveContext};
use crate::maven::pom::raw::{RawPom, RawRepo};

/// 本地 Reactor 中的单个子模块信息
#[derive(Clone, Debug)]
pub struct ReactorModule {
    /// 落实后的自身坐标（group/version 继承自父链）
    pub coord: Coordinate,
    /// 模块 pom.xml 所在目录
    pub dir: PathBuf,
    /// pom.xml 绝对路径
    pub pom_path: PathBuf,
    /// 原始未解析 POM
    pub raw: RawPom,
}

/// 本地 Reactor 工作空间
pub struct ReactorWorkspace {
    /// 入口 POM 路径
    pub entry_path: PathBuf,
    /// 探测到的最顶级本地根 POM 路径
    pub root_pom_path: PathBuf,
    /// 收集到的所有本地模块（含根工程本身）
    pub modules: Vec<ReactorModule>,
    /// 本地工程内部模块键集合 (group, artifact)
    pub reactor_keys: HashSet<(String, String)>,
}

impl ReactorWorkspace {
    /// 从用户传入的任意 pom.xml 路径自动探测并构建 Reactor 工作空间
    pub fn discover(entry_pom_path: &Path) -> Result<Self> {
        let entry_canonical = entry_pom_path
            .canonicalize()
            .map_err(|e| Error::Other(format!("pom 不可达 {}: {e}", entry_pom_path.display())))?;
        let entry_raw = RawPom::from_file(&entry_canonical)?;

        // 1. 尝试向上追溯最顶级的本地父 POM
        let (root_pom_path, root_raw) = find_topmost_local_pom(&entry_canonical, &entry_raw)?;

        // 2. 从最顶级 POM 递归向下扫描所有 <modules>
        let mut modules = Vec::new();
        let mut visited_paths = HashSet::new();
        scan_modules_recursive(&root_pom_path, root_raw, &mut modules, &mut visited_paths)?;

        // 若向下扫描没有收集到 entry（例如 entry 不在顶级 modules 列表中），补充 entry 自身
        if !visited_paths.contains(&entry_canonical) {
            let entry_dir = entry_canonical.parent().unwrap_or(&entry_canonical).to_path_buf();
            let entry_coord = resolve_initial_coord(&entry_raw, &modules);
            modules.push(ReactorModule {
                coord: entry_coord,
                dir: entry_dir,
                pom_path: entry_canonical.clone(),
                raw: entry_raw,
            });
        }

        // 3. 构建内部模块坐标集合
        let mut reactor_keys = HashSet::new();
        for m in &modules {
            if !m.coord.group.is_empty() && !m.coord.artifact.is_empty() {
                reactor_keys.insert((m.coord.group.clone(), m.coord.artifact.clone()));
            }
        }

        log::info(format!(
            "Reactor 发现本地工程：顶层根 POM `{}`，共包含 {} 个模块",
            root_pom_path.display(),
            modules.len()
        ));

        Ok(Self {
            entry_path: entry_canonical,
            root_pom_path,
            modules,
            reactor_keys,
        })
    }

    /// 判断某个坐标是否为本地 Reactor 内部模块（不需要也不应该从远程仓库拉取 jar）
    pub fn is_internal_module(&self, group: &str, artifact: &str) -> bool {
        self.reactor_keys.contains(&(group.to_string(), artifact.to_string()))
    }

    /// 构建聚合 Effective POM：
    /// - 若是单模块（没有 modules 声明），走标准从本地构建。
    /// - 若是多模块工程，自动解析所有子模块并将有效外部依赖汇总合并（过滤内部模块互引，保序去重）。
    pub async fn build_effective(&self, ctx: &ResolveContext) -> Result<Arc<EffectivePom>> {
        let root_dir = self.entry_path.parent();
        let entry_raw = RawPom::from_file(&self.entry_path)?;

        // 若入口 POM 没有声明 modules，说明用户选的是一个具体单模块或叶子模块
        if entry_raw.modules.is_empty() {
            let eff = ctx.build_from_local(entry_raw, root_dir, Vec::new()).await?;
            return Ok(eff);
        }

        // 多模块聚合模式：先解析入口 POM 自身（取得根坐标、根 properties、根 depMgmt、根 repositories）
        let root_eff = ctx.build_from_local(entry_raw, root_dir, Vec::new()).await?;

        // 遍历所有子模块，分别构建其 EffectivePom
        let mut aggregated_deps: Vec<DepDecl> = Vec::new();
        let mut seen_dep_keys: HashSet<GaKey> = HashSet::new();
        let mut aggregated_repos: Vec<RawRepo> = root_eff.repositories.clone();
        let mut seen_repo_ids: HashSet<String> = root_eff.repositories.iter().map(|r| r.id.clone()).collect();

        // 聚合根 POM 自身已有的直接依赖（如有）
        for d in &root_eff.deps {
            if !self.is_internal_module(&d.group, &d.artifact) {
                if seen_dep_keys.insert(d.key()) {
                    aggregated_deps.push(d.clone());
                }
            }
        }

        for m in &self.modules {
            // 跳过入口聚合根 POM（自身 packaging=pom 无具体业务依赖，前面已处理）
            if m.pom_path == self.entry_path {
                continue;
            }
            let sub_eff = match ctx
                .build_from_local(m.raw.clone(), Some(&m.dir), Vec::new())
                .await
            {
                Ok(eff) => eff,
                Err(err) => {
                    log::warn(format!(
                        "解析子模块 `{}` ({}) 失败: {err}，已跳过该模块",
                        m.coord.artifact,
                        m.pom_path.display()
                    ));
                    continue;
                }
            };

            // 收集子模块的仓库声明
            for repo in &sub_eff.repositories {
                if seen_repo_ids.insert(repo.id.clone()) {
                    aggregated_repos.push(repo.clone());
                }
            }

            // 聚合子模块的依赖，过滤掉本地工程内部相互依赖
            for d in &sub_eff.deps {
                if self.is_internal_module(&d.group, &d.artifact) {
                    log::verbose(format!(
                        "忽略本地工程内部模块依赖: {}:{}",
                        d.group, d.artifact
                    ));
                    continue;
                }
                if seen_dep_keys.insert(d.key()) {
                    aggregated_deps.push(d.clone());
                }
            }
        }

        log::info(format!(
            "多模块依赖聚合完成：汇总 {} 个子模块，共提取 {} 个有效外部直接依赖",
            self.modules.len(),
            aggregated_deps.len()
        ));

        Ok(Arc::new(EffectivePom {
            coord: root_eff.coord.clone(),
            deps: aggregated_deps,
            dep_mgmt: root_eff.dep_mgmt.clone(),
            repositories: aggregated_repos,
        }))
    }
}

/// 向上查找最顶级的本地父 POM
fn find_topmost_local_pom(start_path: &Path, start_raw: &RawPom) -> Result<(PathBuf, RawPom)> {
    let mut current_path = start_path.to_path_buf();
    let mut current_raw = start_raw.clone();
    let mut seen_paths = HashSet::new();
    seen_paths.insert(current_path.clone());

    while let Some(parent) = &current_raw.parent {
        let parent_expected = (parent.group.clone(), parent.artifact.clone(), parent.version.clone());
        let current_dir = current_path.parent();
        let local_parent = crate::maven::pom::effective::try_local_parent(
            current_dir,
            &current_raw.relative_path,
            &parent_expected,
        );

        match local_parent {
            Some((p_raw, p_path)) => {
                if !seen_paths.insert(p_path.clone()) {
                    // 成环保护
                    break;
                }
                current_path = p_path;
                current_raw = p_raw;
            }
            None => {
                // 向上已无本地父 POM（父 POM 是外部远程库如 spring-boot-starter-parent）
                break;
            }
        }
    }

    Ok((current_path, current_raw))
}

/// 递归向下扫描所有子模块的 pom.xml
fn scan_modules_recursive(
    pom_path: &Path,
    raw: RawPom,
    modules: &mut Vec<ReactorModule>,
    visited_paths: &mut HashSet<PathBuf>,
) -> Result<()> {
    let canonical = pom_path
        .canonicalize()
        .unwrap_or_else(|_| pom_path.to_path_buf());
    if !visited_paths.insert(canonical.clone()) {
        return Ok(());
    }

    let dir = canonical.parent().unwrap_or(&canonical).to_path_buf();
    let coord = resolve_initial_coord(&raw, modules);

    let module_subpaths = raw.modules.clone();
    modules.push(ReactorModule {
        coord,
        dir: dir.clone(),
        pom_path: canonical,
        raw,
    });

    for sub in module_subpaths {
        let mut sub_path = dir.join(&sub);
        if sub_path.is_dir() {
            sub_path = sub_path.join("pom.xml");
        }
        if !sub_path.exists() {
            log::warn(format!("子模块 POM 不存在: {}", sub_path.display()));
            continue;
        }
        let sub_canonical = match sub_path.canonicalize() {
            Ok(p) => p,
            Err(_) => sub_path.clone(),
        };
        if visited_paths.contains(&sub_canonical) {
            continue;
        }
        let sub_raw = match RawPom::from_file(&sub_canonical) {
            Ok(r) => r,
            Err(e) => {
                log::warn(format!("无法解析子模块 {}: {e}", sub_canonical.display()));
                continue;
            }
        };
        scan_modules_recursive(&sub_canonical, sub_raw, modules, visited_paths)?;
    }

    Ok(())
}

/// 初步推算模块坐标（继承 parent 的 groupId 和 version）
fn resolve_initial_coord(raw: &RawPom, _existing_modules: &[ReactorModule]) -> Coordinate {
    let group = raw
        .group
        .clone()
        .or_else(|| raw.parent.as_ref().map(|p| p.group.clone()))
        .unwrap_or_default();
    let version = raw
        .version
        .clone()
        .or_else(|| raw.parent.as_ref().map(|p| p.version.clone()))
        .unwrap_or_default();

    Coordinate {
        group,
        artifact: raw.artifact.clone(),
        version,
        packaging: raw.packaging.clone(),
        classifier: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::maven::pom::raw::BoxFut;

    struct DummyPomSource;
    impl crate::maven::pom::raw::PomSource for DummyPomSource {
        fn fetch_raw_pom<'a>(&'a self, _g: &'a str, _a: &'a str, _v: &'a str) -> BoxFut<'a, Result<RawPom>> {
            Box::pin(async { Err(Error::Other("network not available in test".to_string())) })
        }
    }

    struct TempTestDir(PathBuf);
    impl TempTestDir {
        fn new(name: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("veloterm-maven-{name}-{nanos}"));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempTestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn test_multi_module_aggregation_and_parent_cascade() {
        let tmp = TempTestDir::new("multi-module");
        let root_dir = tmp.path();

        // 1. 根 POM（packaging=pom，声明 modules，定义全局 properties 和 depMgmt）
        let root_pom = r#"
<project>
  <groupId>com.example</groupId>
  <artifactId>root</artifactId>
  <version>1.0.0</version>
  <packaging>pom</packaging>
  <modules>
    <module>core</module>
    <module>web</module>
  </modules>
  <properties>
    <slf4j.version>1.7.36</slf4j.version>
  </properties>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.slf4j</groupId>
        <artifactId>slf4j-api</artifactId>
        <version>${slf4j.version}</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
</project>"#;
        std::fs::write(root_dir.join("pom.xml"), root_pom).unwrap();

        // 2. core 模块（无显式 version，继承 root，依赖 slf4j-api（无 version，由 root depMgmt 管理））
        let core_dir = root_dir.join("core");
        std::fs::create_dir_all(&core_dir).unwrap();
        let core_pom = r#"
<project>
  <parent>
    <groupId>com.example</groupId>
    <artifactId>root</artifactId>
    <version>1.0.0</version>
  </parent>
  <artifactId>core</artifactId>
  <dependencies>
    <dependency>
      <groupId>org.slf4j</groupId>
      <artifactId>slf4j-api</artifactId>
    </dependency>
  </dependencies>
</project>"#;
        std::fs::write(core_dir.join("pom.xml"), core_pom).unwrap();

        // 3. web 模块（依赖 core 内部模块，以及第三方外部库 commons-io）
        let web_dir = root_dir.join("web");
        std::fs::create_dir_all(&web_dir).unwrap();
        let web_pom = r#"
<project>
  <parent>
    <groupId>com.example</groupId>
    <artifactId>root</artifactId>
    <version>1.0.0</version>
    <relativePath>../pom.xml</relativePath>
  </parent>
  <artifactId>web</artifactId>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>core</artifactId>
      <version>1.0.0</version>
    </dependency>
    <dependency>
      <groupId>commons-io</groupId>
      <artifactId>commons-io</artifactId>
      <version>2.11.0</version>
    </dependency>
  </dependencies>
</project>"#;
        std::fs::write(web_dir.join("pom.xml"), web_pom).unwrap();

        // 验证 1：从 root 发现工作空间，包含 3 个模块
        let ws = ReactorWorkspace::discover(&root_dir.join("pom.xml")).unwrap();
        assert_eq!(ws.modules.len(), 3);
        assert!(ws.is_internal_module("com.example", "root"));
        assert!(ws.is_internal_module("com.example", "core"));
        assert!(ws.is_internal_module("com.example", "web"));

        // 验证 2：聚合 EffectivePom 构建
        let ctx = ResolveContext::new(Arc::new(DummyPomSource), HashMap::new());
        let eff = ws.build_effective(&ctx).await.unwrap();

        // 内部模块 com.example:core 应该被过滤掉，保留 slf4j-api 与 commons-io
        assert_eq!(eff.deps.len(), 2);
        let slf4j = eff.deps.iter().find(|d| d.artifact == "slf4j-api").unwrap();
        assert_eq!(slf4j.version.as_deref(), Some("1.7.36")); // 来自 root 属性插值与 depMgmt

        let commons = eff.deps.iter().find(|d| d.artifact == "commons-io").unwrap();
        assert_eq!(commons.version.as_deref(), Some("2.11.0"));

        // 验证 3：从子模块 web/pom.xml 也能自动向上追溯到顶级 root 并发现完整 Reactor
        let ws_from_sub = ReactorWorkspace::discover(&web_dir.join("pom.xml")).unwrap();
        assert_eq!(ws_from_sub.root_pom_path, root_dir.join("pom.xml").canonicalize().unwrap());
        assert_eq!(ws_from_sub.modules.len(), 3);
    }

    #[tokio::test]
    async fn test_three_level_parent_properties_cascade() {
        let tmp = TempTestDir::new("three-level");
        let base = tmp.path();

        // Level 1: grandparent
        let gp_dir = base.join("grandparent");
        std::fs::create_dir_all(&gp_dir).unwrap();
        let gp_pom = r#"
<project>
  <groupId>com.corp</groupId>
  <artifactId>grandparent</artifactId>
  <version>3.0.0</version>
  <packaging>pom</packaging>
  <properties>
    <guava.version>31.1-jre</guava.version>
  </properties>
</project>"#;
        std::fs::write(gp_dir.join("pom.xml"), gp_pom).unwrap();

        // Level 2: parent
        let p_dir = base.join("parent");
        std::fs::create_dir_all(&p_dir).unwrap();
        let p_pom = r#"
<project>
  <parent>
    <groupId>com.corp</groupId>
    <artifactId>grandparent</artifactId>
    <version>3.0.0</version>
    <relativePath>../grandparent/pom.xml</relativePath>
  </parent>
  <artifactId>parent</artifactId>
  <packaging>pom</packaging>
  <properties>
    <extra.prop>hello</extra.prop>
  </properties>
</project>"#;
        std::fs::write(p_dir.join("pom.xml"), p_pom).unwrap();

        // Level 3: leaf
        let leaf_dir = base.join("leaf");
        std::fs::create_dir_all(&leaf_dir).unwrap();
        let leaf_pom = r#"
<project>
  <parent>
    <groupId>com.corp</groupId>
    <artifactId>parent</artifactId>
    <version>3.0.0</version>
    <relativePath>../parent/pom.xml</relativePath>
  </parent>
  <artifactId>leaf</artifactId>
  <dependencies>
    <dependency>
      <groupId>com.google.guava</groupId>
      <artifactId>guava</artifactId>
      <version>${guava.version}</version>
    </dependency>
  </dependencies>
</project>"#;
        std::fs::write(leaf_dir.join("pom.xml"), leaf_pom).unwrap();

        let ctx = ResolveContext::new(Arc::new(DummyPomSource), HashMap::new());
        let leaf_raw = RawPom::from_file(&leaf_dir.join("pom.xml")).unwrap();
        let eff = ctx.build_from_local(leaf_raw, Some(&leaf_dir), Vec::new()).await.unwrap();

        assert_eq!(eff.coord.group, "com.corp");
        assert_eq!(eff.coord.version, "3.0.0");
        assert_eq!(eff.deps.len(), 1);
        assert_eq!(eff.deps[0].artifact, "guava");
        assert_eq!(eff.deps[0].version.as_deref(), Some("31.1-jre"));
    }
}
