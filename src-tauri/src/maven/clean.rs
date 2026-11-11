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
}
