//! 本地 Maven 仓库体检与垃圾清理：
//! 递归扫描并清理 *.lastUpdated、*.part、_remote.repositories、*.tmp 等残存损坏文件，
//! 解决 Maven 离线报错、源切换缓存污染与磁盘占用问题，并自动级联清理空目录。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use crate::maven::error::{Error, Result};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokenCacheItem {
    pub path: String,
    pub rel_path: String,
    pub kind: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MavenBrokenCacheScanResult {
    pub repo_path: String,
    pub files: Vec<BrokenCacheItem>,
    pub total_files: usize,
    pub total_bytes: u64,
    pub last_updated_count: usize,
    pub part_count: usize,
    pub remote_repo_count: usize,
    pub tmp_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MavenCleanResult {
    pub deleted_files: usize,
    pub freed_bytes: u64,
    pub cleaned_dirs: usize,
    pub failed_items: Vec<(String, String)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupSpaceItem {
    pub group: String,
    pub total_bytes: u64,
    pub artifact_count: usize,
    pub version_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactVersionItem {
    pub version: String,
    pub dir_path: String,
    pub total_bytes: u64,
    pub file_count: usize,
    pub is_latest: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedundantArtifactGroup {
    pub group: String,
    pub artifact: String,
    pub versions: Vec<ArtifactVersionItem>,
    pub total_bytes: u64,
    pub redundant_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoSpaceAnalysisResult {
    pub repo_path: String,
    pub total_repo_bytes: u64,
    pub total_artifacts: usize,
    pub top_groups: Vec<GroupSpaceItem>,
    pub redundant_artifacts: Vec<RedundantArtifactGroup>,
}

/// 判定单个文件名是否属于垃圾/临时损坏缓存文件，返回 Some(kind)
fn detect_broken_cache_kind(file_name: &str) -> Option<&'static str> {
    if file_name.ends_with(".lastUpdated") {
        Some("lastUpdated")
    } else if file_name.ends_with(".part") {
        Some("part")
    } else if file_name == "_remote.repositories" {
        Some("remoteRepo")
    } else if file_name.ends_with(".tmp") || file_name.ends_with(".sha1.tmp") || file_name.ends_with(".md5.tmp") {
        Some("tmp")
    } else {
        None
    }
}

/// 扫描指定仓库目录下的所有垃圾损坏标记文件
pub fn scan_broken_cache(repo_root: &Path) -> Result<MavenBrokenCacheScanResult> {
    if !repo_root.exists() {
        return Err(Error::Other(format!("仓库目录不存在: {}", repo_root.display())));
    }

    let repo_canonical = repo_root.canonicalize().unwrap_or_else(|_| repo_root.to_path_buf());
    let mut files = Vec::new();
    let mut total_bytes = 0u64;
    let mut last_updated_count = 0usize;
    let mut part_count = 0usize;
    let mut remote_repo_count = 0usize;
    let mut tmp_count = 0usize;

    let mut stack = vec![repo_canonical.clone()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    if let Some(kind) = detect_broken_cache_kind(file_name) {
                        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                        total_bytes += size;
                        match kind {
                            "lastUpdated" => last_updated_count += 1,
                            "part" => part_count += 1,
                            "remoteRepo" => remote_repo_count += 1,
                            "tmp" => tmp_count += 1,
                            _ => {}
                        }
                        let rel_path = path
                            .strip_prefix(&repo_canonical)
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|_| file_name.to_string());

                        files.push(BrokenCacheItem {
                            path: path.to_string_lossy().to_string(),
                            rel_path,
                            kind: kind.to_string(),
                            size,
                        });
                    }
                }
            }
        }
    }

    let total_files = files.len();
    Ok(MavenBrokenCacheScanResult {
        repo_path: repo_canonical.to_string_lossy().to_string(),
        files,
        total_files,
        total_bytes,
        last_updated_count,
        part_count,
        remote_repo_count,
        tmp_count,
    })
}

/// 执行批量删除垃圾文件，并自底向上清理空目录
pub fn clean_broken_cache(file_paths: &[String], repo_root: Option<&Path>) -> MavenCleanResult {
    let mut deleted_files = 0usize;
    let mut freed_bytes = 0u64;
    let mut failed_items = Vec::new();
    let mut candidate_dirs: HashSet<PathBuf> = HashSet::new();

    for path_str in file_paths {
        let p = Path::new(path_str);
        if let Ok(meta) = fs::metadata(p) {
            let size = meta.len();
            if let Some(parent) = p.parent() {
                candidate_dirs.insert(parent.to_path_buf());
            }
            match fs::remove_file(p) {
                Ok(_) => {
                    deleted_files += 1;
                    freed_bytes += size;
                }
                Err(e) => {
                    failed_items.push((path_str.clone(), e.to_string()));
                }
            }
        }
    }

    // 清理空父目录
    let mut cleaned_dirs = 0usize;
    let repo_root_canonical = repo_root.and_then(|r| r.canonicalize().ok());

    // 按路径深度降序排序，保证先删子目录后删父目录
    let mut sorted_dirs: Vec<PathBuf> = candidate_dirs.into_iter().collect();
    sorted_dirs.sort_by(|a, b| b.components().count().cmp(&a.components().count()));

    for mut dir in sorted_dirs {
        while dir.exists() {
            if let Some(root) = &repo_root_canonical {
                if &dir == root || !dir.starts_with(root) {
                    break;
                }
            }
            // 尝试删除空目录（若非空则 remove_dir 报错，静默忽略）
            if fs::remove_dir(&dir).is_ok() {
                cleaned_dirs += 1;
                if let Some(parent) = dir.parent() {
                    dir = parent.to_path_buf();
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }

    MavenCleanResult {
        deleted_files,
        freed_bytes,
        cleaned_dirs,
        failed_items,
    }
}

/// 分析本地仓库的体积分布（Top Groups）并找出冗余多版本（>= min_versions）的构件
pub fn analyze_repo_space(
    repo_root: &Path,
    min_versions: usize,
) -> Result<RepoSpaceAnalysisResult> {
    if !repo_root.exists() {
        return Err(Error::Other(format!("仓库目录不存在: {}", repo_root.display())));
    }
    let repo_canonical = repo_root.canonicalize().unwrap_or_else(|_| repo_root.to_path_buf());

    let mut ga_versions: std::collections::HashMap<(String, String), Vec<ArtifactVersionItem>> = std::collections::HashMap::new();
    let mut total_repo_bytes = 0u64;

    let mut stack = vec![repo_canonical.clone()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let mut subdirs = Vec::new();
        let mut file_count = 0usize;
        let mut dir_bytes = 0u64;

        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                subdirs.push(p);
            } else if p.is_file() {
                file_count += 1;
                dir_bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }

        // 如果包含文件，且具有版本目录特征（父目录为 artifact，祖父目录为 group 路径）
        if file_count > 0 {
            if let Some(version_name) = dir.file_name().and_then(|n| n.to_str()) {
                if let Some(artifact_dir) = dir.parent() {
                    if let Some(artifact_name) = artifact_dir.file_name().and_then(|n| n.to_str()) {
                        if let Some(group_dir) = artifact_dir.parent() {
                            if let Ok(rel_group) = group_dir.strip_prefix(&repo_canonical) {
                                let group_parts: Vec<&str> = rel_group
                                    .iter()
                                    .filter_map(|s| s.to_str())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                                if !group_parts.is_empty() {
                                    let group = group_parts.join(".");
                                    total_repo_bytes += dir_bytes;
                                    ga_versions
                                        .entry((group, artifact_name.to_string()))
                                        .or_default()
                                        .push(ArtifactVersionItem {
                                            version: version_name.to_string(),
                                            dir_path: dir.to_string_lossy().to_string(),
                                            total_bytes: dir_bytes,
                                            file_count,
                                            is_latest: false,
                                        });
                                }
                            }
                        }
                    }
                }
            }
        }

        // 将子目录继续加入栈中
        stack.extend(subdirs);
    }

    // 按 Group 汇总体积
    let mut group_totals: std::collections::HashMap<String, (u64, usize, usize)> = std::collections::HashMap::new();
    let mut redundant_artifacts: Vec<RedundantArtifactGroup> = Vec::new();

    for ((group, artifact), mut versions) in ga_versions {
        let artifact_bytes: u64 = versions.iter().map(|v| v.total_bytes).sum();
        let version_count = versions.len();

        let entry = group_totals.entry(group.clone()).or_insert((0, 0, 0));
        entry.0 += artifact_bytes;
        entry.1 += 1; // artifact_count
        entry.2 += version_count; // version_count

        if version_count >= min_versions {
            // 按语义版本从高到低排序，第 0 位为最新版保留
            versions.sort_by(|a, b| crate::maven::version::cmp_version(&b.version, &a.version));
            if let Some(latest) = versions.first_mut() {
                latest.is_latest = true;
            }
            let redundant_bytes: u64 = versions.iter().filter(|v| !v.is_latest).map(|v| v.total_bytes).sum();
            redundant_artifacts.push(RedundantArtifactGroup {
                group,
                artifact,
                versions,
                total_bytes: artifact_bytes,
                redundant_bytes,
            });
        }
    }

    // Top Groups 按体积从大到小排序
    let mut top_groups: Vec<GroupSpaceItem> = group_totals
        .into_iter()
        .map(|(group, (total_bytes, artifact_count, version_count))| GroupSpaceItem {
            group,
            total_bytes,
            artifact_count,
            version_count,
        })
        .collect();
    top_groups.sort_by(|a, b| b.total_bytes.cmp(&a.total_bytes));

    // 冗余构件按冗余体积从大到小排序
    redundant_artifacts.sort_by(|a, b| b.redundant_bytes.cmp(&a.redundant_bytes));

    let total_artifacts = top_groups.iter().map(|g| g.artifact_count).sum();

    Ok(RepoSpaceAnalysisResult {
        repo_path: repo_canonical.to_string_lossy().to_string(),
        total_repo_bytes,
        total_artifacts,
        top_groups,
        redundant_artifacts,
    })
}

/// 批量清理勾选的历史版本目录
pub fn clean_historical_versions(dirs: &[String], repo_root: Option<&Path>) -> MavenCleanResult {
    let mut deleted_files = 0usize;
    let mut freed_bytes = 0u64;
    let mut failed_items = Vec::new();
    let mut candidate_parents: HashSet<PathBuf> = HashSet::new();

    let repo_root_canonical = repo_root.and_then(|r| r.canonicalize().ok());

    for dir_str in dirs {
        let dir = Path::new(dir_str);
        // 安全前置：若指定了仓库根路径，必须位于仓库根路径内，杜绝越界误删！
        if let Some(root) = &repo_root_canonical {
            if let Ok(dir_canon) = dir.canonicalize() {
                if !dir_canon.starts_with(root) || dir_canon == *root {
                    failed_items.push((dir_str.clone(), "禁止删除仓库目录外部或仓库根目录".to_string()));
                    continue;
                }
            }
        }

        if !dir.exists() {
            continue;
        }

        // 统计将要删除的文件数与大小
        let mut dir_file_count = 0usize;
        let mut dir_size = 0u64;
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            if let Ok(entries) = fs::read_dir(&d) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.is_file() {
                        dir_file_count += 1;
                        dir_size += e.metadata().map(|m| m.len()).unwrap_or(0);
                    }
                }
            }
        }

        if let Some(parent) = dir.parent() {
            candidate_parents.insert(parent.to_path_buf());
        }

        match fs::remove_dir_all(dir) {
            Ok(_) => {
                deleted_files += dir_file_count;
                freed_bytes += dir_size;
            }
            Err(e) => {
                failed_items.push((dir_str.clone(), e.to_string()));
            }
        }
    }

    // 向上清理空目录
    let mut cleaned_dirs = 0usize;
    let mut sorted_parents: Vec<PathBuf> = candidate_parents.into_iter().collect();
    sorted_parents.sort_by(|a, b| b.components().count().cmp(&a.components().count()));

    for mut p in sorted_parents {
        while p.exists() {
            if let Some(root) = &repo_root_canonical {
                if &p == root || !p.starts_with(root) {
                    break;
                }
            }
            if fs::remove_dir(&p).is_ok() {
                cleaned_dirs += 1;
                if let Some(parent) = p.parent() {
                    p = parent.to_path_buf();
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }

    MavenCleanResult {
        deleted_files,
        freed_bytes,
        cleaned_dirs,
        failed_items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_and_clean_broken_cache() {
        let temp_dir = std::env::temp_dir().join(format!("veloterm_clean_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        let pkg_dir = temp_dir.join("org/example/demo/1.0");
        fs::create_dir_all(&pkg_dir).unwrap();

        let good_jar = pkg_dir.join("demo-1.0.jar");
        let last_updated = pkg_dir.join("demo-1.0.jar.lastUpdated");
        let part_file = pkg_dir.join("demo-1.0.pom.part");
        let remote_repo = pkg_dir.join("_remote.repositories");
        let tmp_file = pkg_dir.join("temp_checksum.sha1.tmp");

        fs::write(&good_jar, b"good content").unwrap();
        fs::write(&last_updated, b"error").unwrap();
        fs::write(&part_file, b"incomplete").unwrap();
        fs::write(&remote_repo, b"repo=central").unwrap();
        fs::write(&tmp_file, b"tmp").unwrap();

        // 扫描
        let scan = scan_broken_cache(&temp_dir).unwrap();
        assert_eq!(scan.total_files, 4);
        assert_eq!(scan.last_updated_count, 1);
        assert_eq!(scan.part_count, 1);
        assert_eq!(scan.remote_repo_count, 1);
        assert_eq!(scan.tmp_count, 1);

        // 清理这 4 个垃圾文件
        let to_clean: Vec<String> = scan.files.into_iter().map(|f| f.path).collect();
        let res = clean_broken_cache(&to_clean, Some(&temp_dir));
        assert_eq!(res.deleted_files, 4);
        assert_eq!(res.failed_items.len(), 0);

        // 正常 jar 保留
        assert!(good_jar.exists());
        assert!(!last_updated.exists());
        assert!(!part_file.exists());
        assert!(!remote_repo.exists());
        assert!(!tmp_file.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_analyze_space_and_clean_historical_versions() {
        let temp_dir = std::env::temp_dir().join(format!("veloterm_space_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);

        // 为 org.example.lib 构建 5 个版本：1.0, 1.1, 1.2, 1.3, 2.0
        let versions = ["1.0", "1.1", "1.2", "1.3", "2.0"];
        for v in versions {
            let v_dir = temp_dir.join("org/example/lib").join(v);
            fs::create_dir_all(&v_dir).unwrap();
            fs::write(v_dir.join(format!("lib-{v}.jar")), vec![0u8; 100]).unwrap();
        }

        // 分析空间，阈值为 5
        let analysis = analyze_repo_space(&temp_dir, 5).unwrap();
        assert_eq!(analysis.total_repo_bytes, 500);
        assert_eq!(analysis.top_groups.len(), 1);
        assert_eq!(analysis.top_groups[0].group, "org.example");
        assert_eq!(analysis.redundant_artifacts.len(), 1);

        let group = &analysis.redundant_artifacts[0];
        assert_eq!(group.artifact, "lib");
        assert_eq!(group.versions.len(), 5);
        // 最新版本应当是 2.0
        assert_eq!(group.versions[0].version, "2.0");
        assert!(group.versions[0].is_latest);

        // 清理除最新版本以外的 4 个历史版本
        let to_delete: Vec<String> = group
            .versions
            .iter()
            .filter(|v| !v.is_latest)
            .map(|v| v.dir_path.clone())
            .collect();
        assert_eq!(to_delete.len(), 4);

        let clean_res = clean_historical_versions(&to_delete, Some(&temp_dir));
        assert_eq!(clean_res.deleted_files, 4);
        assert_eq!(clean_res.freed_bytes, 400);

        // 最新版 2.0 保留
        assert!(temp_dir.join("org/example/lib/2.0/lib-2.0.jar").exists());
        // 旧版本 1.0 已删除
        assert!(!temp_dir.join("org/example/lib/1.0").exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
