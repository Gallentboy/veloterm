//! ~/.m2/settings.xml 解析：`<localRepository>`、`<servers>`（认证）、`<mirrors>`（镜像替换）。
//! `<profiles>`/`<proxies>` 等暂不支持（README 记录为已知差异）。

use std::path::{Path, PathBuf};

use crate::maven::error::{Error, Result};

#[derive(Clone, Debug)]
pub struct Server {
    pub id: String,
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug)]
pub struct Mirror {
    pub id: String,
    pub url: String,
    /// `*` / `external:*` / `external:http:*` / 逗号 id 列表 / `!` 排除
    pub mirror_of: String,
}

/// settings.xml `<profiles><profile>`（属性与仓库定义）
#[derive(Clone, Debug)]
pub struct SettingsProfile {
    pub id: String,
    pub properties: Vec<(String, String)>,
    pub repositories: Vec<crate::maven::pom::raw::RawRepo>,
}

#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub local_repository: Option<PathBuf>,
    pub servers: Vec<Server>,
    pub mirrors: Vec<Mirror>,
    /// 解析出的 active http 代理（settings.xml `<proxies>`；首个 active 且 protocol=http）
    pub proxy: Option<crate::maven::repo::ProxySpec>,
    /// `<profiles>` 定义 + `<activeProfiles>` 激活 id
    pub profiles: Vec<SettingsProfile>,
    pub active_profile_ids: Vec<String>,
}

impl Settings {
    /// 激活（activeProfiles 命中）的 profile 列表
    pub fn active_profiles(&self) -> Vec<&SettingsProfile> {
        self.profiles
            .iter()
            .filter(|p| self.active_profile_ids.iter().any(|id| id == &p.id))
            .collect()
    }
}

impl Settings {
    /// 读取 settings.xml；默认路径不存在时返回默认空配置，显式指定的路径不存在则报错。
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let default_path = || -> PathBuf {
            home::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".m2")
                .join("settings.xml")
        };
        let explicit = path.is_some();
        let path: PathBuf = match path {
            Some(p) => p.to_path_buf(),
            None => default_path(),
        };
        if !path.exists() {
            if !explicit {
                return Ok(Self::default());
            }
            return Err(Error::Other(format!("settings file not found: {}", path.display())));
        }
        let xml = std::fs::read_to_string(&path)
            .map_err(|e| Error::Other(format!("read settings {}: {e}", path.display())))?;
        Self::parse(&xml)
    }

    fn parse(xml: &str) -> Result<Self> {
        let doc = roxmltree::Document::parse(xml)
            .map_err(|e| Error::BadXml { context: "settings.xml".into(), source: e })?;
        let root = doc.root_element();

        let text_of = |node: roxmltree::Node<'_, '_>, name: &str| -> Option<String> {
            node.children()
                .find(|n| n.is_element() && n.tag_name().name() == name)
                .and_then(|n| n.text())
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        };

        let local_repository = text_of(root, "localRepository").map(PathBuf::from);

        let servers: Vec<Server> = root
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "servers")
            .map(|servers| {
                servers
                    .children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "server")
                    .filter_map(|s| {
                        let id = text_of(s, "id")?;
                        let username = text_of(s, "username").unwrap_or_default();
                        let password = text_of(s, "password").unwrap_or_default();
                        Some(Server { id, username, password })
                    })
                    .collect()
            })
            .unwrap_or_default();

        for s in &servers {
            if s.password.starts_with('{') {
                return Err(Error::EncryptedPassword { id: s.id.clone() });
            }
        }

        let mirrors = root
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "mirrors")
            .map(|mirrors| {
                mirrors
                    .children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "mirror")
                    .filter_map(|m| {
                        Some(Mirror {
                            id: text_of(m, "id")?,
                            url: text_of(m, "url")?,
                            mirror_of: text_of(m, "mirrorOf").unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        // profiles 与 activeProfiles
        let profiles = root
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "profiles")
            .map(|ps| {
                ps.children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "profile")
                    .filter_map(|p| {
                        let id = text_of(p, "id")?;
                        let properties = p
                            .children()
                            .find(|n| n.is_element() && n.tag_name().name() == "properties")
                            .map(|props| {
                                props
                                    .children()
                                    .filter(|n| n.is_element())
                                    .map(|n| {
                                        (n.tag_name().name().to_string(), n.text().unwrap_or("").trim().to_string())
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let repositories = p
                            .children()
                            .find(|n| n.is_element() && n.tag_name().name() == "repositories")
                            .map(|rs| {
                                rs.children()
                                    .filter(|n| n.is_element() && n.tag_name().name() == "repository")
                                    .filter_map(|r| {
                                            Some(crate::maven::pom::raw::RawRepo {
                                            id: text_of(r, "id")?,
                                            url: text_of(r, "url")?,
                                        })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        Some(SettingsProfile { id, properties, repositories })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let active_profile_ids = root
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "activeProfiles")
            .map(|aps| {
                aps.children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "activeProfile")
                    .filter_map(|n| n.text())
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // 首个 active 且 protocol=http 的代理；nonProxyHosts 暂不支持（README 记录为已知差异）
        let proxy = root
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "proxies")
            .and_then(|proxies| {
                proxies
                    .children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "proxy")
                    .find(|p| {
                        let active = text_of(*p, "active").map(|a| a == "true").unwrap_or(false);
                        let http = text_of(*p, "protocol").map(|x| x == "http").unwrap_or(true);
                        active && http
                    })
            })
            .and_then(|p| {
                let host = text_of(p, "host")?;
                let port = text_of(p, "port").unwrap_or_else(|| "80".to_string());
                let auth = match (text_of(p, "username"), text_of(p, "password")) {
                    (Some(u), Some(pw)) => Some((u, pw)),
                    _ => None,
                };
                Some(crate::maven::repo::ProxySpec { url: format!("http://{host}:{port}"), auth })
            });

        Ok(Self { local_repository, servers, mirrors, proxy, profiles, active_profile_ids })
    }

    pub fn server(&self, id: &str) -> Option<&Server> {
        self.servers.iter().find(|s| s.id == id)
    }

    /// 为仓库选择 mirror：先 mirrorOf 精确 id 匹配，后声明顺序通配匹配；一个仓库至多命中一个。
    pub fn mirror_for(&self, repo_id: &str, repo_url: &str) -> Option<&Mirror> {
        // 第一轮：精确 id
        for m in &self.mirrors {
            let p = MirrorOf::parse(&m.mirror_of);
            if p.exact.contains(&repo_id.to_string()) && !p.excludes(repo_id) {
                return Some(m);
            }
        }
        // 第二轮：通配（声明顺序）
        for m in &self.mirrors {
            if MirrorOf::parse(&m.mirror_of).matches(repo_id, repo_url) {
                return Some(m);
            }
        }
        None
    }
}

/// mirrorOf 表达式：逗号分隔的 `id` / `*` / `external:*` / `external:http:*` / `!id`。
/// 语义上通配部分不受位置影响；显式排除优先于通配。
struct MirrorOf {
    exact: Vec<String>,
    excludes: Vec<String>,
    all: bool,
    /// 排除 localhost 与 file:// 的一切
    external: bool,
    /// external 且仅 http 明文
    external_http: bool,
}

impl MirrorOf {
    fn parse(s: &str) -> Self {
        let mut p = Self {
            exact: Vec::new(),
            excludes: Vec::new(),
            all: false,
            external: false,
            external_http: false,
        };
        for part in s.split(',') {
            let part = part.trim();
            match part {
                // 逗号后带空格在 Maven 中是著名坑，这里 trim 掉
                "" => {}
                "*" => p.all = true,
                "external:*" => p.external = true,
                "external:http:*" => p.external_http = true,
                _ if part.starts_with('!') => {
                    p.excludes.push(part[1..].trim().to_string());
                }
                _ => p.exact.push(part.to_string()),
            }
        }
        p
    }

    fn excludes(&self, repo_id: &str) -> bool {
        self.excludes.iter().any(|e| e == repo_id)
    }

    fn matches(&self, repo_id: &str, repo_url: &str) -> bool {
        if self.excludes(repo_id) {
            return false;
        }
        let local = is_local_url(repo_url);
        if self.exact.iter().any(|e| e == repo_id) {
            return true;
        }
        if self.all {
            return true;
        }
        if self.external && !local {
            return true;
        }
        if self.external_http && !local && repo_url.starts_with("http://") {
            return true;
        }
        false
    }
}

/// localhost / file:// 仓库
#[inline]
pub fn is_local_url(url: &str) -> bool {
    if url.starts_with("file://") {
        return true;
    }
    let host = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split(['/', ':']).next())
        .unwrap_or("");
    // 带方括号形式（http://[::1]:8081）split(':') 得 "["——先剥括号
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host == "localhost" || host == "127.0.0.1" || host == "::1"
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"
<settings>
  <localRepository>/data/m2-repo</localRepository>
  <servers>
    <server>
      <id>corp-repo</id>
      <username>alice</username>
      <password>secret</password>
    </server>
  </servers>
  <mirrors>
    <mirror>
      <id>aliyun</id>
      <url>https://maven.aliyun.com/repository/public</url>
      <mirrorOf>central</mirrorOf>
    </mirror>
    <mirror>
      <id>corp-all</id>
      <url>https://corp.example.com/repo</url>
      <mirrorOf>*, !corp-repo</mirrorOf>
    </mirror>
  </mirrors>
</settings>"#;

    #[test]
    fn parse_settings() {
        let s = Settings::parse(XML).unwrap();
        assert_eq!(s.local_repository, Some(PathBuf::from("/data/m2-repo")));
        assert_eq!(s.servers.len(), 1);
        assert_eq!(s.servers[0].id, "corp-repo");
        assert_eq!(s.mirrors.len(), 2);
    }

    #[test]
    fn encrypted_password_rejected() {
        let xml = r#"<settings><servers><server><id>x</id><password>{abc=}</password></server></servers></settings>"#;
        let err = Settings::parse(xml).unwrap_err();
        assert!(matches!(err, Error::EncryptedPassword { .. }));
    }

    #[test]
    fn mirror_exact_match_wins_over_wildcard() {
        let s = Settings::parse(XML).unwrap();
        // central 精确命中 aliyun（虽然 corp-all 的 * 也匹配，精确优先）
        let m = s.mirror_for("central", "https://repo.maven.apache.org/maven2").unwrap();
        assert_eq!(m.id, "aliyun");
    }

    #[test]
    fn mirror_wildcard_with_exclude() {
        let s = Settings::parse(XML).unwrap();
        // 其他仓库命中 corp-all
        let m = s.mirror_for("jboss", "https://repository.jboss.org/x").unwrap();
        assert_eq!(m.id, "corp-all");
        // 但 corp-repo 被排除
        assert!(s.mirror_for("corp-repo", "https://corp.example.com/x").is_none());
    }

    #[test]
    fn mirror_star_matches_everything() {
        let s = Settings::parse(XML).unwrap();
        // Maven 语义：`*` 匹配一切仓库（含 localhost）；external:* 才排除 localhost
        let m = s.mirror_for("local1", "http://localhost:8081/repo").unwrap();
        assert_eq!(m.id, "corp-all");
    }

    #[test]
    fn external_excludes_localhost() {
        let s = Settings::parse(XML.replace("*, !corp-repo", "external:*").as_str()).unwrap();
        assert!(s.mirror_for("local1", "http://localhost:8081/repo").is_none());
        assert!(s.mirror_for("remote1", "https://repo.example.com/x").is_some());
    }

    #[test]
    fn mirror_of_parsing_spaces() {
        let p = MirrorOf::parse(" *, !repo1 ");
        assert!(p.all);
        assert!(p.excludes("repo1"));
        assert!(p.matches("repo2", "https://x.com/r"));
        assert!(!p.matches("repo1", "https://x.com/r"));
    }

    #[test]
    fn external_http_only_plain_http() {
        let p = MirrorOf::parse("external:http:*");
        assert!(p.matches("a", "http://repo.example.com/x"));
        assert!(!p.matches("a", "https://repo.example.com/x"));
        assert!(!p.matches("a", "http://localhost:31339/x"));
    }
}
