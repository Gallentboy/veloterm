//! 离线 Maven 仓库打包器：将下载完成的仓库目录打包为 .zip 或 .tar.gz，
//! 并在根目录自动附带开箱即用的离线 `settings.xml` 与 `README.md`。

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::maven::error::{Error, Result};
use crate::maven::log;

/// 压缩包格式
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackFormat {
    Zip,
    TarGz,
}

impl PackFormat {
    pub fn from_path(p: &Path) -> Self {
        let name = p.to_string_lossy().to_lowercase();
        if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Self::TarGz
        } else {
            Self::Zip
        }
    }
}

/// 打包结果统计
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackSummary {
    pub output_file: String,
    pub total_files: usize,
    pub total_bytes: u64,
    pub elapsed_ms: u64,
}

pub const DEFAULT_OFFLINE_SETTINGS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!--
  Veloterm 离线 Maven 仓库配套 settings.xml
  使用方式：
    1. 在解压后的离线包目录下执行构建：
       mvn clean package -s settings.xml -DskipTests
    2. 或将此 settings.xml 中的 <localRepository> 配置复制至主机的 ~/.m2/settings.xml
-->
<settings xmlns="http://maven.apache.org/SETTINGS/1.2.0"
          xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
          xsi:schemaLocation="http://maven.apache.org/SETTINGS/1.2.0 https://maven.apache.org/xsd/settings-1.2.0.xsd">
  <localRepository>${basedir}/repository</localRepository>
  <offline>true</offline>
  <mirrors>
    <mirror>
      <id>offline-local-mirror</id>
      <name>Veloterm Offline Local Mirror</name>
      <mirrorOf>*</mirrorOf>
      <url>file://${basedir}/repository</url>
    </mirror>
  </mirrors>
  <profiles>
    <profile>
      <id>offline-repo</id>
      <repositories>
        <repository>
          <id>offline-repo</id>
          <url>file://${basedir}/repository</url>
          <releases><enabled>true</enabled></releases>
          <snapshots><enabled>true</enabled></snapshots>
        </repository>
      </repositories>
      <pluginRepositories>
        <pluginRepository>
          <id>offline-plugin-repo</id>
          <url>file://${basedir}/repository</url>
          <releases><enabled>true</enabled></releases>
          <snapshots><enabled>true</enabled></snapshots>
        </pluginRepository>
      </pluginRepositories>
    </profile>
  </profiles>
  <activeProfiles>
    <activeProfile>offline-repo</activeProfile>
  </activeProfiles>
</settings>
"#;

pub const DEFAULT_OFFLINE_README_MD: &str = r#"# Veloterm 离线 Maven 仓库交付包

本离线包由 **Veloterm Maven 仓库助手** 一键自动打包生成，已完整收录项目所需的依赖构件与 POM 元数据。

---

## 快速使用指南

### 方式 1：使用配套 settings.xml 离线构建（推荐，零侵入）
在解压出的目录内包含 `settings.xml` 和 `repository/`。在项目目录执行构建命令时，通过 `-s` 参数指定该配置文件：
```bash
mvn clean package -s /path/to/settings.xml -DskipTests
```

### 方式 2：合并至内网开发机或 CI 节点默认本地仓库
直接将解压出的 `repository/` 目录下的全部内容拷贝或合并至系统的默认本地仓库（通常为 `~/.m2/repository`）：
- **macOS / Linux**:
  ```bash
  cp -R repository/* ~/.m2/repository/
  ```
- **Windows PowerShell**:
  ```powershell
  Copy-Item -Recurse repository\* "$HOME\.m2\repository\"
  ```

---
由 Veloterm 终端驱动器强力构建。
"#;

/// 执行离线仓库打包
pub fn pack_offline_repository(
    source_dir: &Path,
    output_file: &Path,
    format: Option<PackFormat>,
) -> Result<PackSummary> {
    if !source_dir.exists() {
        return Err(Error::Other(format!(
            "源仓库目录不存在: {}",
            source_dir.display()
        )));
    }
    let format = format.unwrap_or_else(|| PackFormat::from_path(output_file));
    if let Some(parent) = output_file.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let started = Instant::now();
    log::info(format!(
        "开始生成离线仓库压缩包 ({:?}): {} -> {}",
        format,
        source_dir.display(),
        output_file.display()
    ));

    // 递归收集待打包的全部文件
    let mut files = Vec::new();
    collect_files_recursive(source_dir, source_dir, &mut files)?;

    let mut total_bytes: u64 = 0;
    for (_, full) in &files {
        if let Ok(meta) = full.metadata() {
            total_bytes += meta.len();
        }
    }

    match format {
        PackFormat::Zip => {
            pack_zip(source_dir, output_file, &files)?;
        }
        PackFormat::TarGz => {
            pack_tar_gz(source_dir, output_file, &files)?;
        }
    }

    let elapsed = started.elapsed();
    let total_files = files.len() + 2; // + settings.xml + README.md
    log::info(format!(
        "离线仓库打包完成: 共写入 {} 个文件 ({})，耗时 {}",
        total_files,
        log::fmt_bytes(total_bytes),
        log::fmt_duration(elapsed)
    ));

    Ok(PackSummary {
        output_file: output_file.to_string_lossy().to_string(),
        total_files,
        total_bytes,
        elapsed_ms: elapsed.as_millis() as u64,
    })
}

fn collect_files_recursive(
    base: &Path,
    current: &Path,
    acc: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    if !current.is_dir() {
        return Ok(());
    }
    let entries = match std::fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_files_recursive(base, &p, acc)?;
        } else if p.is_file() {
            if let Ok(rel) = p.strip_prefix(base) {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                acc.push((rel_str, p));
            }
        }
    }
    Ok(())
}

fn pack_zip(
    _source_dir: &Path,
    output_file: &Path,
    files: &[(String, PathBuf)],
) -> Result<()> {
    let file = File::create(output_file)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // 1. 写入 settings.xml
    zip.start_file("settings.xml", options)
        .map_err(|e| Error::Other(format!("zip start_file settings.xml: {e}")))?;
    zip.write_all(DEFAULT_OFFLINE_SETTINGS_XML.as_bytes())?;

    // 2. 写入 README.md
    zip.start_file("README.md", options)
        .map_err(|e| Error::Other(format!("zip start_file README.md: {e}")))?;
    zip.write_all(DEFAULT_OFFLINE_README_MD.as_bytes())?;

    // 3. 写入 repository 构件文件
    let mut buf = vec![0u8; 64 * 1024];
    for (rel, disk_path) in files {
        let archive_name = format!("repository/{rel}");
        zip.start_file(&archive_name, options)
            .map_err(|e| Error::Other(format!("zip start_file {archive_name}: {e}")))?;
        if let Ok(mut src) = File::open(disk_path) {
            loop {
                let n = src.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                zip.write_all(&buf[..n])?;
            }
        }
    }

    zip.finish()
        .map_err(|e| Error::Other(format!("zip finish: {e}")))?;
    Ok(())
}

fn pack_tar_gz(
    _source_dir: &Path,
    output_file: &Path,
    files: &[(String, PathBuf)],
) -> Result<()> {
    let file = File::create(output_file)?;
    let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut tar = tar::Builder::new(enc);

    // 1. 写入 settings.xml
    let mut header = tar::Header::new_gnu();
    let bytes = DEFAULT_OFFLINE_SETTINGS_XML.as_bytes();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, "settings.xml", bytes)?;

    // 2. 写入 README.md
    let mut header = tar::Header::new_gnu();
    let bytes = DEFAULT_OFFLINE_README_MD.as_bytes();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, "README.md", bytes)?;

    // 3. 写入 repository 构件文件
    for (rel, disk_path) in files {
        let archive_name = format!("repository/{rel}");
        if let Ok(mut src) = File::open(disk_path) {
            let meta = src.metadata()?;
            let mut header = tar::Header::new_gnu();
            header.set_size(meta.len());
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, &archive_name, &mut src)?;
        }
    }

    tar.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_offline_zip_and_tar_gz() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!("veloterm-pack-test-{nanos}"));
        let src_repo = base_dir.join("src_repo");
        let dist_dir = base_dir.join("dist");

        let dummy_jar_dir = src_repo.join("com/example/demo/1.0.0");
        std::fs::create_dir_all(&dummy_jar_dir).unwrap();
        std::fs::write(dummy_jar_dir.join("demo-1.0.0.jar"), b"mock-jar-bytes").unwrap();
        std::fs::write(dummy_jar_dir.join("demo-1.0.0.pom"), b"<project></project>").unwrap();

        // 1. 打包 Zip
        let zip_file = dist_dir.join("offline-repo.zip");
        let summary_zip = pack_offline_repository(&src_repo, &zip_file, Some(PackFormat::Zip)).unwrap();
        assert_eq!(summary_zip.total_files, 4); // jar + pom + settings.xml + README.md
        assert!(zip_file.exists());

        // 验证 Zip 内部结构
        let zip_reader = File::open(&zip_file).unwrap();
        let mut archive = zip::ZipArchive::new(zip_reader).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().unwrap().to_string())
            .collect();
        assert!(names.contains(&"settings.xml".to_string()));
        assert!(names.contains(&"README.md".to_string()));
        assert!(names.contains(&"repository/com/example/demo/1.0.0/demo-1.0.0.jar".to_string()));

        // 2. 打包 tar.gz
        let tar_file = dist_dir.join("offline-repo.tar.gz");
        let summary_tar = pack_offline_repository(&src_repo, &tar_file, Some(PackFormat::TarGz)).unwrap();
        assert_eq!(summary_tar.total_files, 4);
        assert!(tar_file.exists());

        let _ = std::fs::remove_dir_all(&base_dir);
    }
}
