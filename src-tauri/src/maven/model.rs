//! 核心数据模型：坐标、依赖声明、scope 传递表。

/// 依赖/插件的完整坐标。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Coordinate {
    pub group: String,
    pub artifact: String,
    pub version: String,
    /// 打包类型，默认 `jar`
    pub packaging: String,
    pub classifier: Option<String>,
}

impl Coordinate {
    /// `g:a:v[:packaging[:classifier]]` 形式解析（-g 参数格式，与 dependency:get 一致）
    pub fn parse_gav(s: &str) -> anyhow::Result<Self> {
        let parts: Vec<&str> = s.split(':').collect();
        let (g, a, v, packaging, classifier) = match parts.as_slice() {
            [g, a, v] => (*g, *a, *v, "jar", None),
            [g, a, v, pk] => (*g, *a, *v, *pk, None),
            [g, a, v, pk, cls] => (*g, *a, *v, *pk, Some((*cls).to_string())),
            _ => anyhow::bail!(
                "invalid GAV `{s}`: expected groupId:artifactId:version[:packaging[:classifier]]"
            ),
        };
        if g.is_empty() || a.is_empty() || v.is_empty() {
            anyhow::bail!("invalid GAV `{s}`: groupId/artifactId/version must not be empty");
        }
        Ok(Self {
            group: g.to_string(),
            artifact: a.to_string(),
            version: v.to_string(),
            packaging: packaging.to_string(),
            classifier: classifier,
        })
    }

    /// g:a:v 显示形式（日志/错误信息用）
    pub fn gav(&self) -> String {
        format!("{}:{}:{}", self.group, self.artifact, self.version)
    }

    /// 冲突调解粒度的 key：g:a:ext:classifier（tests classifier、aar 与主 jar 互不冲突）
    pub fn ga_key(&self) -> GaKey {
        GaKey {
            group: self.group.clone(),
            artifact: self.artifact.clone(),
            ext: packaging_ext(&self.packaging).to_string(),
            classifier: self.classifier.clone().unwrap_or_default(),
        }
    }
}

/// dependencyManagement / 调解共用的冲突粒度 key。
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GaKey {
    pub group: String,
    pub artifact: String,
    /// packaging 经映射后的文件扩展名（bundle→jar 等）
    pub ext: String,
    /// 无 classifier 为 ""
    pub classifier: String,
}

impl GaKey {
    pub fn new(group: &str, artifact: &str, type_: Option<&str>, classifier: Option<&str>) -> Self {
        Self {
            group: group.to_string(),
            artifact: artifact.to_string(),
            ext: packaging_ext(type_.unwrap_or("jar")).to_string(),
            classifier: classifier.unwrap_or("").to_string(),
        }
    }
}

/// packaging → 文件扩展名映射（bundle/maven-plugin/ejb 实际发布为 jar）。
/// 未知类型按字面量处理（aar/war/ear/rar/hpi/…）。
#[inline]
pub fn packaging_ext(packaging: &str) -> &str {
    match packaging {
        "" | "jar" | "bundle" | "maven-plugin" | "ejb" => "jar",
        other => other,
    }
}

/// 依赖 scope。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Compile,
    Provided,
    Runtime,
    Test,
}

impl Scope {
#[inline]
    pub fn parse(s: &str) -> Scope {
        match s {
            "provided" => Scope::Provided,
            "runtime" => Scope::Runtime,
            "test" => Scope::Test,
            // 未知 scope（如 system）按 compile 处理，system 依赖本工具不特殊支持
            _ => Scope::Compile,
        }
    }

#[inline]
    pub fn as_str(&self) -> &'static str {
        match self {
            Scope::Compile => "compile",
            Scope::Provided => "provided",
            Scope::Runtime => "runtime",
            Scope::Test => "test",
        }
    }
}

/// scope 传递表：行=上游（已继承到的）scope，列=子 POM 声明的 scope。
/// 声明的 provided/test 永不传递；返回 None 表示该分支丢弃。
#[inline]
pub fn transitive_scope(parent: Scope, declared: Scope) -> Option<Scope> {
    use Scope::*;
    match (parent, declared) {
        (Compile, Compile) => Some(Compile),
        (Compile, Runtime) => Some(Runtime),
        (Provided, Compile) => Some(Provided),
        (Provided, Runtime) => Some(Provided),
        (Runtime, Compile) => Some(Runtime),
        (Runtime, Runtime) => Some(Runtime),
        (Test, Compile) => Some(Test),
        (Test, Runtime) => Some(Test),
        // (任意, Provided) / (任意, Test) → 丢弃
        (_, Provided) | (_, Test) => None,
    }
}

/// `<dependency>` / `<dependencyManagement>` 条目（raw 与 effective 共用结构）。
#[derive(Clone, Debug)]
pub struct DepDecl {
    pub group: String,
    pub artifact: String,
    /// 声明的版本，可能缺（待 depMgmt 补或报错）；BOM import 时为被 import 的 pom 坐标版本
    pub version: Option<String>,
    /// None = 默认 compile
    pub scope: Option<String>,
    pub optional: bool,
    /// None = 默认 jar
    pub type_: Option<String>,
    pub classifier: Option<String>,
    pub exclusions: Vec<Exclusion>,
}

impl DepDecl {
    pub fn key(&self) -> GaKey {
        GaKey::new(&self.group, &self.artifact, self.type_.as_deref(), self.classifier.as_deref())
    }

    pub fn display(&self) -> String {
        format!(
            "{}:{}{}",
            self.group,
            self.artifact,
            self.version.as_deref().map(|v| format!(":{v}")).unwrap_or_default()
        )
    }
}

/// `<exclusion>`。group/artifact 均可为 `*`（Maven 3.2.1+），匹配大小写敏感。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exclusion {
    pub group: String,
    pub artifact: String,
}

impl Exclusion {
#[inline]
    pub fn matches(&self, group: &str, artifact: &str) -> bool {
        let g_ok = self.group == "*" || self.group == group;
        let a_ok = self.artifact == "*" || self.artifact == artifact;
        g_ok && a_ok
    }
}

/// 远程仓库（config 合成后的最终形态）。
#[derive(Clone, Debug)]
pub struct RemoteRepo {
    pub id: String,
    pub url: String,
    pub auth: Option<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gav_forms() {
        let c = Coordinate::parse_gav("org.slf4j:slf4j-api:2.0.13").unwrap();
        assert_eq!(c.packaging, "jar");
        assert!(c.classifier.is_none());

        let c = Coordinate::parse_gav("g:a:1.0:war").unwrap();
        assert_eq!(c.packaging, "war");

        let c = Coordinate::parse_gav("g:a:1.0:jar:tests").unwrap();
        assert_eq!(c.classifier.as_deref(), Some("tests"));

        assert!(Coordinate::parse_gav("g:a").is_err());
        assert!(Coordinate::parse_gav("g:a:").is_err());
    }

    #[test]
    fn ga_key_separates_classifier_and_ext() {
        let jar = Coordinate::parse_gav("g:a:1.0").unwrap();
        let tests = Coordinate::parse_gav("g:a:1.0:jar:tests").unwrap();
        let aar = Coordinate::parse_gav("g:a:1.0:aar").unwrap();
        assert_ne!(jar.ga_key(), tests.ga_key());
        assert_ne!(jar.ga_key(), aar.ga_key());
        // bundle 与 jar 视为同一 ext
        let bundle = Coordinate::parse_gav("g:a:1.0:bundle").unwrap();
        assert_eq!(jar.ga_key(), bundle.ga_key());
    }

    #[test]
    fn scope_table_matches_maven_spec() {
        use Scope::*;
        // 声明的 provided/test 永不传递
        for p in [Compile, Provided, Runtime, Test] {
            assert_eq!(transitive_scope(p, Provided), None);
            assert_eq!(transitive_scope(p, Test), None);
        }
        assert_eq!(transitive_scope(Compile, Compile), Some(Compile));
        assert_eq!(transitive_scope(Compile, Runtime), Some(Runtime));
        assert_eq!(transitive_scope(Provided, Compile), Some(Provided));
        assert_eq!(transitive_scope(Provided, Runtime), Some(Provided));
        assert_eq!(transitive_scope(Runtime, Compile), Some(Runtime));
        assert_eq!(transitive_scope(Runtime, Runtime), Some(Runtime));
        assert_eq!(transitive_scope(Test, Compile), Some(Test));
        assert_eq!(transitive_scope(Test, Runtime), Some(Test));
    }

    #[test]
    fn exclusion_wildcards() {
        let e = Exclusion { group: "g".into(), artifact: "*".into() };
        assert!(e.matches("g", "anything"));
        assert!(!e.matches("x", "anything"));
        let all = Exclusion { group: "*".into(), artifact: "*".into() };
        assert!(all.matches("g", "a"));
        // 大小写敏感
        let e = Exclusion { group: "G".into(), artifact: "A".into() };
        assert!(!e.matches("g", "a"));
    }
}
