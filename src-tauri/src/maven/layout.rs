//! 本地存储布局：m2 标准（默认）与扁平化。
//! SNAPSHOT 落盘一律使用 baseVersion 文件名（与 Maven 本地仓库一致，避免时间戳文件无限堆积）。

use std::path::PathBuf;

use crate::maven::model::Coordinate;

/// 存储布局：m2 标准（默认）与扁平化。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
pub enum Layout {
    /// groupId 点换斜线的标准 Maven 仓库结构
    M2,
    /// 单目录平铺：{groupId}-{artifactId}-{version}[-{classifier}].{ext}
    Flat,
}

impl Layout {
    /// 字符串解析（"m2" | "flat"，大小写不敏感；缺省 m2）
    pub fn parse(s: &str) -> Self {
        if s.eq_ignore_ascii_case("flat") {
            Layout::Flat
        } else {
            Layout::M2
        }
    }
}

impl std::fmt::Display for Layout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Layout::M2 => write!(f, "m2"),
            Layout::Flat => write!(f, "flat"),
        }
    }
}

impl Layout {
    /// 某坐标下某文件的相对路径。
    /// `file_ext`：jar/pom/aar…；SNAPSHOT 时文件名用 baseVersion。
    pub fn rel_path(&self, coord: &Coordinate, file_ext: &str) -> PathBuf {
        let classifier = coord.classifier.as_deref().unwrap_or("");
        let base = &coord.version;
        match self {
            Layout::M2 => {
                let group_dir: PathBuf = coord.group.split('.').collect();
                group_dir
                    .join(&coord.artifact)
                    .join(base)
                    .join(file_name(&coord.artifact, base, classifier, file_ext))
            }
            Layout::Flat => {
                // 与 Maven 仓库一致的原始文件名：{artifactId}-{version}[-{classifier}].{ext}
                PathBuf::from(file_name(&coord.artifact, base, classifier, file_ext))
            }
        }
    }
}

/// `{prefix}-{version}[-{classifier}].{ext}`；version 传入 baseVersion（SNAPSHOT 用字面量名）。
fn file_name(prefix: &str, base_version: &str, classifier: &str, ext: &str) -> String {
    if classifier.is_empty() {
        format!("{prefix}-{base_version}.{ext}")
    } else {
        format!("{prefix}-{base_version}-{classifier}.{ext}")
    }
}

/// 构建输出目录。
#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
    pub layout: Layout,
}

impl Store {
    pub fn new(root: PathBuf, layout: Layout) -> Self {
        Self { root, layout }
    }

    pub fn path(&self, coord: &Coordinate, file_ext: &str) -> PathBuf {
        self.root.join(self.layout.rel_path(coord, file_ext))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coord(packaging: &str, classifier: Option<&str>) -> Coordinate {
        Coordinate {
            group: "org.apache.commons".into(),
            artifact: "commons-lang3".into(),
            version: "3.14.0".into(),
            packaging: packaging.into(),
            classifier: classifier.map(String::from),
        }
    }

    #[test]
    fn m2_layout_paths() {
        let s = Store::new("/repo".into(), Layout::M2);
        let p = s.path(&coord("jar", None), "jar");
        assert_eq!(
            p,
            PathBuf::from("/repo/org/apache/commons/commons-lang3/3.14.0/commons-lang3-3.14.0.jar")
        );
        let p = s.path(&coord("jar", None), "pom");
        assert_eq!(
            p,
            PathBuf::from("/repo/org/apache/commons/commons-lang3/3.14.0/commons-lang3-3.14.0.pom")
        );
        let p = s.path(&coord("jar", Some("sources")), "jar");
        assert_eq!(
            p,
            PathBuf::from(
                "/repo/org/apache/commons/commons-lang3/3.14.0/commons-lang3-3.14.0-sources.jar"
            )
        );
    }

    #[test]
    fn flat_layout_paths() {
        let s = Store::new("/out".into(), Layout::Flat);
        // 原始文件名（与 Maven 仓库一致），不带 groupId 前缀
        let p = s.path(&coord("jar", None), "jar");
        assert_eq!(p, PathBuf::from("/out/commons-lang3-3.14.0.jar"));
        let p = s.path(&coord("jar", Some("tests")), "jar");
        assert_eq!(p, PathBuf::from("/out/commons-lang3-3.14.0-tests.jar"));
    }

    #[test]
    fn snapshot_uses_base_version() {
        let mut c = coord("jar", None);
        c.version = "1.0-SNAPSHOT".into();
        let s = Store::new("/repo".into(), Layout::M2);
        // 落盘文件名固定 baseVersion，不带时间戳
        assert_eq!(
            s.path(&c, "jar"),
            PathBuf::from("/repo/org/apache/commons/commons-lang3/1.0-SNAPSHOT/commons-lang3-1.0-SNAPSHOT.jar")
        );
    }
}
