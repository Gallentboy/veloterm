//! Jar / War / Zip 压缩包流式结构透视与元数据解析
//!
//! 基于 `zip::ZipArchive` 读取 Central Directory，完全无需将大 Jar 解压落盘，
//! 毫秒级提取 Package/Class 树形结构与 Maven/Manifest 元数据。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

/// Jar 内部条目节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JarEntryNode {
    /// 完整内部相对路径 (例如 "com/example/UserService.class" 或 "BOOT-INF/classes/")
    pub path: String,
    /// 显示名称 (例如 "UserService.class")
    pub name: String,
    /// 是否为目录
    pub is_dir: bool,
    /// 是否为 .class 文件
    pub is_class: bool,
    /// 是否为内部类/匿名类 (例如 "UserService$1.class")
    pub is_inner_class: bool,
    /// 原始文件字节大小
    pub size: u64,
    /// 压缩后大小
    pub compressed_size: u64,
    /// 子节点 (仅目录有)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub children: Vec<JarEntryNode>,
}

/// Jar 构件元信息（从 pom.properties 与 MANIFEST.MF 提取）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JarMetadata {
    pub group_id: Option<String>,
    pub artifact_id: Option<String>,
    pub version: Option<String>,
    pub main_class: Option<String>,
    pub built_by: Option<String>,
    pub build_jdk: Option<String>,
    pub created_by: Option<String>,
    pub total_classes: usize,
    pub total_files: usize,
    pub manifest_entries: HashMap<String, String>,
}

/// Jar 概览结构体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JarStructureOverview {
    pub file_path: String,
    pub file_size: u64,
    pub metadata: JarMetadata,
    pub tree: Vec<JarEntryNode>,
}

// 暂存所有非目录文件的相对路径和属性
struct FlatItem {
    path: String,
    name: String,
    is_dir: bool,
    is_class: bool,
    is_inner_class: bool,
    size: u64,
    compressed_size: u64,
}

/// 解析 Jar / War / Zip 文件结构与元数据
pub fn parse_jar_structure<P: AsRef<Path>>(file_path: P) -> Result<JarStructureOverview, String> {
    let path_ref = file_path.as_ref();
    let file = File::open(path_ref).map_err(|e| format!("无法打开文件 {:?}: {}", path_ref, e))?;
    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);

    let mut archive = ZipArchive::new(file).map_err(|e| format!("解析 Zip/Jar 归档失败: {}", e))?;

    let mut metadata = JarMetadata::default();
    let mut total_classes = 0usize;
    let mut total_files = 0usize;

    let mut items = Vec::with_capacity(archive.len());
    let mut pom_properties_content: Option<String> = None;
    let mut manifest_content: Option<String> = None;

    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let raw_name = match entry.name() {
            Ok(n) => n.to_string(),
            Err(_) => continue,
        };
        let is_dir = entry.is_dir() || raw_name.ends_with('/');
        let norm_path = raw_name.trim_end_matches('/').to_string();
        if norm_path.is_empty() {
            continue;
        }

        total_files += 1;
        let is_class = !is_dir && norm_path.ends_with(".class");
        let name = norm_path.split('/').last().unwrap_or(&norm_path).to_string();
        let is_inner_class = is_class && name.contains('$');

        if is_class {
            total_classes += 1;
        }

        // 识别 MANIFEST.MF
        if norm_path == "META-INF/MANIFEST.MF" && manifest_content.is_none() {
            let mut s = String::new();
            if entry.read_to_string(&mut s).is_ok() {
                manifest_content = Some(s);
            }
        }

        // 识别 pom.properties (取找到的第一个)
        if norm_path.starts_with("META-INF/maven/") && norm_path.ends_with("pom.properties") && pom_properties_content.is_none() {
            let mut s = String::new();
            if entry.read_to_string(&mut s).is_ok() {
                pom_properties_content = Some(s);
            }
        }

        items.push(FlatItem {
            path: norm_path,
            name,
            is_dir,
            is_class,
            is_inner_class,
            size: entry.size(),
            compressed_size: entry.compressed_size(),
        });
    }

    metadata.total_classes = total_classes;
    metadata.total_files = total_files;

    // 解析 MANIFEST.MF
    if let Some(content) = manifest_content {
        parse_manifest(&content, &mut metadata);
    }

    // 解析 pom.properties
    if let Some(content) = pom_properties_content {
        parse_pom_properties(&content, &mut metadata);
    }

    // 构建目录树
    let tree = build_entry_tree(items);

    Ok(JarStructureOverview {
        file_path: path_ref.to_string_lossy().to_string(),
        file_size,
        metadata,
        tree,
    })
}

/// 读取 Jar 内指定条目的原始字节流
pub fn read_jar_entry_bytes<P: AsRef<Path>>(file_path: P, entry_path: &str) -> Result<Vec<u8>, String> {
    let path_ref = file_path.as_ref();
    let file = File::open(path_ref).map_err(|e| format!("无法打开文件 {:?}: {}", path_ref, e))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("解析 Zip/Jar 归档失败: {}", e))?;

    let mut entry = archive
        .by_name(entry_path)
        .map_err(|_| format!("Jar 包中未找到条目: {}", entry_path))?;

    let mut buffer = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buffer).map_err(|e| format!("读取条目失败: {}", e))?;
    Ok(buffer)
}

/// 解析 MANIFEST.MF 属性
fn parse_manifest(content: &str, meta: &mut JarMetadata) {
    let mut current_key: Option<String> = None;
    let mut current_val = String::new();

    fn flush_entry(k: Option<&String>, v: &str, meta: &mut JarMetadata) {
        if let Some(key) = k {
            let val = v.trim();
            meta.manifest_entries.insert(key.clone(), val.to_string());
            match key.to_ascii_lowercase().as_str() {
                "main-class" => meta.main_class = Some(val.to_string()),
                "built-by" => meta.built_by = Some(val.to_string()),
                "build-jdk" | "build-jdk-spec" => meta.build_jdk = Some(val.to_string()),
                "created-by" => meta.created_by = Some(val.to_string()),
                _ => {}
            }
        }
    }

    for line in content.lines() {
        if line.starts_with(' ') {
            // 折行续写
            current_val.push_str(line.trim_start());
        } else if let Some((k, v)) = line.split_once(':') {
            flush_entry(current_key.as_ref(), &current_val, meta);
            current_key = Some(k.trim().to_string());
            current_val = v.trim().to_string();
        }
    }
    flush_entry(current_key.as_ref(), &current_val, meta);
}

/// 解析 pom.properties
fn parse_pom_properties(content: &str, meta: &mut JarMetadata) {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            let val = v.trim();
            match key {
                "groupId" => meta.group_id = Some(val.to_string()),
                "artifactId" => meta.artifact_id = Some(val.to_string()),
                "version" => meta.version = Some(val.to_string()),
                _ => {}
            }
        }
    }
}

/// 将扁平的 items 组装为嵌套目录树
fn build_entry_tree(items: Vec<FlatItem>) -> Vec<JarEntryNode> {
    use std::collections::BTreeMap;

    struct DirNode {
        name: String,
        path: String,
        sub_dirs: BTreeMap<String, DirNode>,
        files: Vec<JarEntryNode>,
    }

    impl DirNode {
        fn new(name: String, path: String) -> Self {
            Self {
                name,
                path,
                sub_dirs: BTreeMap::new(),
                files: Vec::new(),
            }
        }

        fn insert_file(&mut self, parts: &[String], item: FlatItem) {
            if parts.len() <= 1 {
                self.files.push(JarEntryNode {
                    path: item.path,
                    name: item.name,
                    is_dir: false,
                    is_class: item.is_class,
                    is_inner_class: item.is_inner_class,
                    size: item.size,
                    compressed_size: item.compressed_size,
                    children: Vec::new(),
                });
            } else {
                let dir_name = &parts[0];
                let sub_path = if self.path.is_empty() {
                    dir_name.clone()
                } else {
                    format!("{}/{}", self.path, dir_name)
                };
                let sub_dir = self
                    .sub_dirs
                    .entry(dir_name.clone())
                    .or_insert_with(|| DirNode::new(dir_name.clone(), sub_path));
                sub_dir.insert_file(&parts[1..], item);
            }
        }

        fn to_nodes(mut self) -> Vec<JarEntryNode> {
            let mut list = Vec::new();
            for (_, sub) in self.sub_dirs {
                let path = sub.path.clone();
                let name = sub.name.clone();
                let children = sub.to_nodes();
                list.push(JarEntryNode {
                    path,
                    name,
                    is_dir: true,
                    is_class: false,
                    is_inner_class: false,
                    size: 0,
                    compressed_size: 0,
                    children,
                });
            }
            // 目录在前，文件在后
            self.files.sort_by(|a, b| a.name.cmp(&b.name));
            list.append(&mut self.files);
            list
        }
    }

    let mut root = DirNode::new(String::new(), String::new());
    for item in items {
        let path_str = item.path.clone();
        if item.is_dir {
            let mut cur = &mut root;
            let mut p = String::new();
            for part in path_str.split('/') {
                if !p.is_empty() {
                    p.push('/');
                }
                p.push_str(part);
                cur = cur
                    .sub_dirs
                    .entry(part.to_string())
                    .or_insert_with(|| DirNode::new(part.to_string(), p.clone()));
            }
        } else {
            let parts: Vec<String> = path_str.split('/').map(|s| s.to_string()).collect();
            root.insert_file(&parts, item);
        }
    }

    root.to_nodes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_parse_jar_structure() {
        let tmp_dir = std::env::temp_dir();
        let jar_path = tmp_dir.join("test_demo.jar");

        // 创建一个测试 zip 内存包
        {
            let file = File::create(&jar_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();

            // 写入 MANIFEST.MF
            zip.start_file("META-INF/MANIFEST.MF", options).unwrap();
            zip.write_all(b"Manifest-Version: 1.0\nMain-Class: com.example.App\nBuilt-By: test\n").unwrap();

            // 写入 pom.properties
            zip.start_file("META-INF/maven/com.example/demo/pom.properties", options).unwrap();
            zip.write_all(b"groupId=com.example\nartifactId=demo\nversion=1.0.0\n").unwrap();

            // 写入普通类与内部类
            zip.start_file("com/example/App.class", options).unwrap();
            zip.write_all(b"\xca\xfe\xba\xbe dummy bytecode").unwrap();

            zip.start_file("com/example/App$Config.class", options).unwrap();
            zip.write_all(b"\xca\xfe\xba\xbe dummy inner").unwrap();

            zip.start_file("application.yml", options).unwrap();
            zip.write_all(b"server:\n  port: 8080\n").unwrap();

            zip.finish().unwrap();
        }

        let res = parse_jar_structure(&jar_path).expect("parse jar should succeed");
        assert_eq!(res.metadata.group_id.as_deref(), Some("com.example"));
        assert_eq!(res.metadata.artifact_id.as_deref(), Some("demo"));
        assert_eq!(res.metadata.version.as_deref(), Some("1.0.0"));
        assert_eq!(res.metadata.main_class.as_deref(), Some("com.example.App"));
        assert_eq!(res.metadata.total_classes, 2);

        // 测试读取单个条目字节
        let bytes = read_jar_entry_bytes(&jar_path, "com/example/App.class").unwrap();
        assert!(bytes.starts_with(b"\xca\xfe\xba\xbe"));

        let _ = std::fs::remove_file(&jar_path);
    }
}
