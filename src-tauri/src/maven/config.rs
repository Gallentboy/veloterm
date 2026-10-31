//! 仓库列表合成：CLI `-r` → 根 POM 链 `<repositories>` → 内置 central 兜底，
//! 应用 mirror 改写并按 mirror 自身 id 绑定凭据，最后按最终 URL 去重。
//! 查询语义为严格顺序 fallback：第一个仓库 404 才找第二个，全部失败才算失败。

use std::collections::HashMap;

use crate::maven::log;
use crate::maven::model::RemoteRepo;
use crate::maven::pom::raw::RawRepo;
use crate::maven::settings::{is_local_url, Settings};

/// 内置兜底仓库：阿里云公共镜像（Maven Central 的镜像，国内访问快）。
/// id 保持 `central` 不变，settings.xml 中 `mirrorOf="central"` 的镜像规则仍可命中。
pub const DEFAULT_REPO_URL: &str = "https://maven.aliyun.com/repository/public";

/// CLI `-r` 参数：`id=url` 或裸 `url`（裸 url 的 id 用 url 本身，无法命中 settings 认证）。
pub struct CliRepo {
    pub id: String,
    pub url: String,
    /// repositories.json 提供的内联认证（Basic）
    pub auth: Option<(String, String)>,
}

impl CliRepo {
    pub fn parse(s: &str) -> Self {
        match s.split_once('=') {
            Some((id, url)) => Self { id: id.trim().to_string(), url: url.trim().to_string(), auth: None },
            None => Self { id: s.trim().to_string(), url: s.trim().to_string(), auth: None },
        }
    }
}

/// 合成最终仓库列表（顺序即 fallback 优先级）。
pub fn build_repos(cli_repos: &[CliRepo], pom_repos: &[RawRepo], settings: &Settings) -> Vec<RemoteRepo> {
    let mut ordered: Vec<(String, String)> = Vec::new();
    let mut inline_auth: HashMap<String, (String, String)> = HashMap::new();
    // JSON（内联认证）与 CLI -r 均最优先，按序
    for r in cli_repos {
        if let Some(a) = &r.auth {
            inline_auth.insert(r.id.clone(), a.clone());
        }
        ordered.push((r.id.clone(), r.url.clone()));
    }
    // 根 POM 及其父链声明的仓库（传递 POM 的 repositories 被忽略——Maven 有意为之）
    for r in pom_repos {
        ordered.push((r.id.clone(), r.url.clone()));
    }
    // 内置 central 兜底
    ordered.push(("central".to_string(), DEFAULT_REPO_URL.to_string()));

    // mirror 改写 + 凭据绑定 + 去重（按最终 URL）
    let mut result: Vec<RemoteRepo> = Vec::new();
    let mut seen_urls: Vec<String> = Vec::new();
    for (id, url) in ordered {
        // mirror 接管：凭据查 mirror 自身的 id；未接管：查仓库 id / JSON 内联认证。
        // 凭据规则：mirror 接管时只允许 mirror 自身 id 的凭据（settings servers）——
        // 原 id 的 JSON 内联认证是为原 URL 准备的，不发往 mirror 主机（防串仓泄漏）
        let (mirrored, final_id, final_url) = match settings.mirror_for(&id, &url) {
            Some(m) => (true, m.id.clone(), m.url.clone()),
            None => (false, id.clone(), url),
        };
        let auth = settings
            .server(&final_id)
            .map(|s| (s.username.clone(), s.password.clone()))
            .or_else(|| if mirrored { None } else { inline_auth.get(&id).cloned() });
        if seen_urls.contains(&final_url) {
            continue;
        }
        if final_url.starts_with("http://") && !is_local_url(&final_url) {
            log::warn(format!(
                "repository `{final_id}` uses plain HTTP ({final_url}); credentials would be sent in clear text"
            ));
        }
        seen_urls.push(final_url.clone());
        result.push(RemoteRepo { id: final_id, url: final_url, auth });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maven::settings::{Mirror, Server};

    fn settings_with(mirrors: Vec<Mirror>, servers: Vec<Server>) -> Settings {
        Settings { local_repository: None, servers, mirrors, proxy: None, profiles: Vec::new(), active_profile_ids: Vec::new() }
    }

    #[test]
    fn order_is_cli_then_pom_then_central() {
        let s = settings_with(vec![], vec![]);
        let cli = vec![CliRepo::parse("my=https://my.example.com/repo")];
        let pom = vec![RawRepo { id: "jboss".into(), url: "https://repo.jboss.org/x".into() }];
        let repos = build_repos(&cli, &pom, &s);
        let urls: Vec<&str> = repos.iter().map(|r| r.url.as_str()).collect();
        assert_eq!(
            urls,
            vec!["https://my.example.com/repo", "https://repo.jboss.org/x", DEFAULT_REPO_URL]
        );
    }

    #[test]
    fn mirror_rewrites_and_binds_auth_by_mirror_id() {
        let s = settings_with(
            vec![Mirror {
                id: "aliyun".into(),
                url: "https://maven.aliyun.com/repository/public".into(),
                mirror_of: "central".into(),
            }],
            vec![Server { id: "aliyun".into(), username: "u".into(), password: "p".into() }],
        );
        let repos = build_repos(&[], &[], &s);
        // central 被 aliyun 接管
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].url, "https://maven.aliyun.com/repository/public");
        assert_eq!(repos[0].auth.clone().unwrap(), ("u".to_string(), "p".to_string()));
    }

    #[test]
    fn duplicate_final_urls_deduped() {
        let s = settings_with(vec![], vec![]);
        let cli = vec![
            CliRepo::parse("a=https://x.example.com/repo"),
            CliRepo::parse("b=https://x.example.com/repo"),
        ];
        let repos = build_repos(&cli, &[], &s);
        assert_eq!(repos.len(), 2); // 重复 URL 的 b 被去重，保留先声明的 a
        assert_eq!(repos[0].id, "a");
    }
}
