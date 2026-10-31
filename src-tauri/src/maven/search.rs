//! Maven 仓库搜索（原 mvn-dr search 子命令的库化形态）：
//! 查 Nexus（search API）或中央仓（solrsearch），关键词模糊搜列出坐标+最新版本；
//! g:a + 指定仓库时走 metadata 探测路径（版本级 maven-metadata.xml + 并发 HEAD
//! 三态探测）；g:a:v 指定版本时给出存在性与相邻完整版本建议。
//! 打印输出全部改为结构化返回（供 Tauri 命令层序列化给前端表格）。

use crate::maven::log;
use serde_json::Value;

/// 关键词/g:a 搜索的单条命中
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub group: String,
    pub artifact: String,
    pub latest: String,
    pub extra: String,
}

/// 单个附加探测项的三态：1=存在 ✓ / 0=不存在 ✗ / 2=无法判定 ?
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeCell {
    pub name: String,
    /// 1 = 存在 / 0 = 不存在 / 2 = 无法判定（401/403/405/5xx/传输失败）
    pub state: u8,
}

/// g:a 模式下单个版本的存在性探测结果
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionProbe {
    pub version: String,
    pub pom_ok: bool,
    pub extras: Vec<ProbeCell>,
}

/// g:a:v 模式的相邻完整版本建议
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionSuggestion {
    pub version: String,
    /// "前"（早于指定版本）/ "后"（晚于）
    pub tag: String,
    pub extras: Vec<ProbeCell>,
}

/// 搜索结果三形态（与前端三个展示模式一一对应）
#[derive(Debug, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SearchOutcome {
    /// 关键词 / g:a 搜索命中列表（Nexus 或中央仓）
    #[serde(rename_all = "camelCase")]
    Hits {
        #[serde(rename = "hits")]
        hits: Vec<SearchHit>,
    },
    /// g:a + 仓库：全部版本三态探测（cmp_version 升序，最新在最后）
    #[serde(rename_all = "camelCase")]
    Versions {
        group: String,
        artifact: String,
        versions: Vec<VersionProbe>,
        /// 无法判定的探测项数（结果可信度提示）
        unknown_count: usize,
    },
    /// g:a:v + 仓库：指定版本存在性
    #[serde(rename_all = "camelCase")]
    Existence {
        group: String,
        artifact: String,
        version: String,
        /// pom + 全部附加探测项都存在
        exists: bool,
        pom_ok: bool,
        extras: Vec<ProbeCell>,
        /// 不存在/不完整时的相邻完整版本建议（前 3 / 后 3，升序）
        suggestions: Vec<VersionSuggestion>,
    },
}

/// 搜索请求参数（原 SearchArgs 的库化形态）。
pub struct SearchParams {
    /// 关键词 / g:a / g:a:v
    pub query: String,
    /// Nexus 地址或任意仓库地址（None = 中央仓 solrsearch）
    pub repo: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    /// g:a 模式列全部版本（-V）
    pub versions: bool,
    /// 探测 jar 存在性（--check-jar）
    pub check_jar: bool,
    /// 附加探测的 classifier 列表（--classifier，已展开）
    pub classifiers: Vec<String>,
    /// 关键词搜索的返回行数
    pub rows: usize,
}

/// 搜索入口：按 query 形态与 repo 有无分流三种模式。
pub async fn run_search(p: &SearchParams) -> Result<SearchOutcome, String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("veloterm-maven/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let auth = split_auth(&p.username, &p.password);

    // 指定仓库 + g:a 或 g:a:v：走 metadata 探测路径（版本存在性实测）
    if let Some(repo) = &p.repo {
        let parts: Vec<&str> = p.query.split(':').collect();
        let probe_args = ProbeArgs { check_jar: p.check_jar, classifier: p.classifiers.clone() };
        if parts.len() == 2 && !p.versions {
            return probe_versions(&client, repo, &auth, parts[0], parts[1], None, &probe_args).await;
        }
        if parts.len() == 3 {
            return probe_versions(&client, repo, &auth, parts[0], parts[1], Some(parts[2]), &probe_args).await;
        }
    }

    // Nexus / 中央仓关键词搜索
    let (url, body): (String, Value) = if let Some(nexus) = &p.repo {
        let mut u = format!("{}/service/rest/v1/search", nexus.trim_end_matches('/'));
        let is_gav = p.query.contains(':');
        if is_gav {
            let Some((g, a)) = p.query.split_once(':') else {
                return Err("坐标形态无效（缺 artifactId）".into());
            };
            if p.versions {
                return Err("Nexus 模式不支持列全部版本（按版本聚合返回）；请换中央仓或用仓库探测模式".into());
            }
            u.push_str(&format!("?maven.groupId={g}&maven.artifactId={a}"));
        } else {
            u.push_str(&format!("?q={}", urlencode(&p.query)));
        }
        u.push_str(&format!("&limit={}", p.rows));
        let mut req = client.get(&u);
        if let Some((u_, p_)) = &auth {
            req = req.basic_auth(u_, Some(p_));
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("{u} -> {status}\n{}", truncate(&text, 200)));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| format!("JSON: {e}\n{}", truncate(&text, 200)))?;
        (u, v)
    } else {
        let is_gav = p.query.contains(':');
        let core = if p.versions { "gav" } else { "g" };
        let q = if is_gav {
            let Some((g, a)) = p.query.split_once(':') else {
                return Err("坐标形态无效（缺 artifactId）".into());
            };
            format!("g:{g} AND a:{a}")
        } else {
            p.query.clone()
        };
        let u = format!(
            "https://search.maven.org/solrsearch/select?q={}&rows={}&wt=json&core={core}",
            urlencode(&q),
            p.rows
        );
        let resp = client.get(&u).send().await.map_err(|e| e.to_string())?;
        let text = resp.text().await.unwrap_or_default();
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("JSON: {e}"))?;
        (u, v)
    };

    let hits = extract_hits(&body, p.repo.is_some());
    if hits.is_empty() {
        return Err(format!("无结果（{url}）"));
    }
    Ok(SearchOutcome::Hits { hits })
}

/// 探测配置（从 SearchParams 归一）
struct ProbeArgs {
    check_jar: bool,
    classifier: Vec<String>,
}

/// 拉版本级 maven-metadata.xml 解析全部版本，并发 HEAD 探测存在性。
#[allow(clippy::too_many_arguments)]
async fn probe_versions(
    client: &reqwest::Client,
    repo: &str,
    auth: &Option<(String, String)>,
    g: &str,
    a: &str,
    want: Option<&str>,
    args: &ProbeArgs,
) -> Result<SearchOutcome, String> {
    let base = repo.trim_end_matches('/');
    let meta_url = format!("{}/{}/{a}/maven-metadata.xml", base, g.replace('.', "/"));
    let mut req = client.get(&meta_url);
    if let Some((u, p)) = auth {
        req = req.basic_auth(u, Some(p));
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("{meta_url} -> {}", resp.status()));
    }
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let versions = parse_versions(&text);
    if versions.is_empty() {
        return Err(format!("metadata 无版本列表：{meta_url}"));
    }
    log::info(format!("metadata 列出 {} 个版本，并发探测存在性…", versions.len()));

    // 并发 HEAD 探测（JoinSet + Semaphore 16）：
    // 每版本探测 pom（基准）+ jar（check_jar）+ 各 classifier
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(16));
    let mut set = tokio::task::JoinSet::new();
    #[derive(Clone)]
    struct Probe {
        kind: String,
        suffix: String,
    }
    let mut probes = vec![Probe { kind: "pom".into(), suffix: ".pom".into() }];
    if args.check_jar {
        probes.push(Probe { kind: "jar".into(), suffix: ".jar".into() });
    }
    for cls in &args.classifier {
        probes.push(Probe { kind: cls.clone(), suffix: format!("-{cls}.jar") });
    }
    for v in versions {
        for probe in &probes {
            let permit = sem.clone().acquire_owned().await.map_err(|e| e.to_string())?;
            let client = client.clone();
            let base = base.to_string();
            let (g, a) = (g.to_string(), a.to_string());
            let auth = auth.clone();
            let v = v.clone();
            let probe = probe.clone();
            set.spawn(async move {
                let _permit = permit;
                let url = format!("{}/{}/{a}/{v}/{a}-{v}{}", base, g.replace('.', "/"), probe.suffix);
                let mut req = client.head(&url);
                if let Some((u, p)) = &auth {
                    req = req.basic_auth(u, Some(p));
                }
                // 三态：200=存在 / 404=不存在 / 其它（401/403/405/5xx）=无法判定
                let st = req.send().await.map(|r| r.status().as_u16()).unwrap_or(0);
                let tri = if st == 200 { 1u8 } else if st == 404 { 0u8 } else { 2u8 };
                (v, probe.kind, tri)
            });
        }
    }
    // 汇总：version -> [(kind, tri)]
    let mut results: std::collections::BTreeMap<String, Vec<(String, u8)>> =
        std::collections::BTreeMap::new();
    while let Some(j) = set.join_next().await {
        let (v, kind, tri) = j.map_err(|e| e.to_string())?;
        results.entry(v).or_default().push((kind, tri));
    }
    let probe_names: Vec<String> = probes.iter().map(|p| p.kind.clone()).collect();
    let extra_of = |v: &str| -> Vec<ProbeCell> {
        results
            .get(v)
            .map(|ks| {
                probe_names
                    .iter()
                    .filter(|k| k.as_str() != "pom")
                    .map(|name| ProbeCell {
                        name: name.clone(),
                        state: ks.iter().find(|(k, _)| k == name).map(|(_, t)| *t).unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let pom_ok_of = |v: &str| -> bool {
        results
            .get(v)
            .and_then(|ks| ks.iter().find(|(k, _)| k == "pom"))
            .map(|(_, t)| *t == 1)
            .unwrap_or(false)
    };
    // 升序：cmp_version（旧→新，最新最后）
    let mut sorted_versions: Vec<String> = results.keys().cloned().collect();
    sorted_versions.sort_by(|x, y| {
        crate::maven::version::cmp_version(x, y).then_with(|| x.cmp(y))
    });

    // 存在"无法判定"时提示（认证/限流/HEAD 被拒——结果不可全信）
    let unknown_count: usize = results
        .values()
        .flat_map(|ks| ks.iter().map(|(_, t)| if *t == 2 { 1 } else { 0 }))
        .sum();
    if unknown_count > 0 {
        log::warn(format!(
            "{unknown_count} 项探测返回非 200/404（401/403/405/5xx）——可能是认证失败、限流或仓库不支持 HEAD，标为 ? 的结果不可信"
        ));
    }

    if let Some(want) = want {
        // 指定版本：完整存在 = pom + 全部附加探测项都 ✓
        let extras = extra_of(want);
        let pom_ok = pom_ok_of(want);
        let exists = pom_ok && extras.iter().all(|c| c.state == 1);
        let mut suggestions = Vec::new();
        if !exists {
            // 相邻完整版本（前 3 / 后 3，升序）：pom + 全部附加探测项都存在
            let complete: Vec<&String> = sorted_versions
                .iter()
                .filter(|v| {
                    let cells = extra_of(v);
                    pom_ok_of(v) && cells.iter().all(|c| c.state == 1)
                })
                .collect();
            let (mut before, mut after): (Vec<&String>, Vec<&String>) = (Vec::new(), Vec::new());
            for v in complete {
                if crate::maven::version::cmp_version(v, want) == std::cmp::Ordering::Less {
                    before.push(v);
                } else {
                    after.push(v);
                }
            }
            for v in before.iter().rev().take(3).collect::<Vec<_>>().iter().rev() {
                suggestions.push(VersionSuggestion {
                    version: (*v).to_string(),
                    tag: "前".into(),
                    extras: extra_of(v),
                });
            }
            for v in after.iter().take(3) {
                suggestions.push(VersionSuggestion {
                    version: (*v).clone(),
                    tag: "后".into(),
                    extras: extra_of(v),
                });
            }
        }
        return Ok(SearchOutcome::Existence {
            group: g.to_string(),
            artifact: a.to_string(),
            version: want.to_string(),
            exists,
            pom_ok,
            extras,
            suggestions,
        });
    }

    // g:a 模式：全部版本列出（含三态附加列），升序、最新最后
    let versions: Vec<VersionProbe> = sorted_versions
        .iter()
        .map(|v| VersionProbe { version: v.clone(), pom_ok: pom_ok_of(v), extras: extra_of(v) })
        .collect();
    Ok(SearchOutcome::Versions {
        group: g.to_string(),
        artifact: a.to_string(),
        versions,
        unknown_count,
    })
}

/// 解析版本级 maven-metadata.xml 的 <versioning><versions><version> 列表
fn parse_versions(xml: &str) -> Vec<String> {
    let doc = match roxmltree::Document::parse(xml) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    doc.root_element()
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "versioning")
        .and_then(|v| v.children().find(|n| n.is_element() && n.tag_name().name() == "versions"))
        .map(|vs| {
            vs.children()
                .filter(|n| n.is_element() && n.tag_name().name() == "version")
                .filter_map(|n| n.text())
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// 拆认证：username 支持 user:pass 合写
#[inline]
fn split_auth(user: &Option<String>, pass: &Option<String>) -> Option<(String, String)> {
    match (user, pass) {
        (Some(u), Some(p)) => Some((u.clone(), p.clone())),
        (Some(u), None) => u.split_once(':').map(|(a, b)| (a.to_string(), b.to_string())),
        _ => None,
    }
}

/// 从两种 API 响应中提取（Nexus items[] / solr docs[]）
fn extract_hits(v: &Value, nexus: bool) -> Vec<SearchHit> {
    let arr = if nexus {
        v.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default()
    } else {
        v.pointer("/response/docs").and_then(|d| d.as_array()).cloned().unwrap_or_default()
    };
    arr.iter()
        .filter_map(|d| {
            let g = d.get("group").or_else(|| d.get("g"))?.as_str()?.to_string();
            let a = d
                .get("name")
                .or_else(|| d.get("artifactId"))
                .or_else(|| d.get("a"))?
                .as_str()?
                .to_string();
            let latest = d
                .get("version")
                .or_else(|| d.get("latestVersion"))
                .or_else(|| d.get("v"))?
                .as_str()?
                .to_string();
            let extra = if nexus {
                let group = d.get("group").and_then(|x| x.as_str()).unwrap_or("");
                let name = d.get("name").and_then(|x| x.as_str()).unwrap_or("");
                let repo = d.get("repository").and_then(|x| x.as_str()).unwrap_or("");
                if group.is_empty() && name.is_empty() {
                    format!("repo={repo}")
                } else {
                    String::new()
                }
            } else {
                let p = d.get("p").and_then(|x| x.as_str()).unwrap_or("");
                let vc = d.get("versionCount").and_then(|x| x.as_i64()).unwrap_or(0);
                let ts = d.get("timestamp").and_then(|x| x.as_i64()).unwrap_or(0);
                let date = if ts > 0 {
                    chrono::DateTime::from_timestamp(ts / 1000, 0)
                        .map(|t| t.format("%Y-%m-%d").to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                format!("{p} | {vc} 版本 | {date}")
            };
            Some(SearchHit { group: g, artifact: a, latest, extra })
        })
        .collect()
}

/// 极简 URL 编码（query 里的空格/:等）
#[inline]
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[inline]
fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
