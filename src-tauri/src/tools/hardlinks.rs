//! coreutils 硬链接安装（BusyBox 模式的短名入口）。
//!
//! Windows 专属流程：应用启动时在「主程序同级目录 `tools/`」下，为
//! `coreutils.exe` 创建每个命令的 NTFS 硬链接（`ls.exe` `grep.exe`…）。
//! 硬链接与源共享同一份文件数据，11 个短名入口只占一份磁盘空间。
//!
//! 幂等策略（启动时检查，每次都跑、开销为每命令一次 metadata 读取）：
//! - 链接不存在 → 创建；
//! - 链接存在且大小与源一致 → 跳过不重建；
//! - 链接存在但大小不一致（应用升级换了新 coreutils.exe，旧 inode 残留）→ 删除重建。
//!
//! 核心逻辑 `ensure_tool_links` 为纯跨平台实现（APFS/NTFS 都支持硬链接），
//! 在 macOS 上以真实文件系统单元测试覆盖；`ensure_installed_tools` 仅
//! Windows 编译，从 `current_exe()` 推导安装位置。

use super::coreutils::all_commands;
use std::fs;
use std::path::Path;

/// 单个命令链接的处理结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkOutcome {
    /// 不存在，已创建硬链接
    Created,
    /// 已存在且与源大小一致，跳过
    KeptExisting,
    /// 已存在但大小不一致（升级残留），已删除重建
    RecreatedStale,
    /// 源二进制不存在（如 dev 模式未编译 coreutils），整体跳过
    SkippedNoSource,
    /// 创建/删除失败（如文件系统不支持硬链接）
    Failed,
}

/// 命令名 → 链接文件名（Windows 下带 `.exe` 后缀）
fn link_file_name(command: &str) -> String {
    format!("{}{}", command, std::env::consts::EXE_SUFFIX)
}

/// 确保每个命令在 `tools_dir` 下都有指向 `source_bin` 的硬链接（幂等）。
/// 永不 panic：任何文件系统错误都记录为 `Failed` 并继续处理其余命令。
pub fn ensure_tool_links(source_bin: &Path, tools_dir: &Path) -> Vec<(&'static str, LinkOutcome)> {
    let commands = all_commands();
    let Some(source_len) = fs::metadata(source_bin).ok().map(|m| m.len()) else {
        return commands
            .iter()
            .map(|cmd| (*cmd, LinkOutcome::SkippedNoSource))
            .collect();
    };

    let _ = fs::create_dir_all(tools_dir);
    commands
        .iter()
        .map(|cmd| {
            let link = tools_dir.join(link_file_name(cmd));

            // 存在且大小与源一致 → 幂等跳过
            if fs::metadata(&link)
                .map(|m| m.len() == source_len)
                .unwrap_or(false)
            {
                return (*cmd, LinkOutcome::KeptExisting);
            }

            // 不存在 → 创建；存在但大小不符 → 升级残留，删除后重建
            let had_stale_entry = link.symlink_metadata().is_ok();
            if had_stale_entry && fs::remove_file(&link).is_err() {
                return (*cmd, LinkOutcome::Failed);
            }
            let outcome = match fs::hard_link(source_bin, &link) {
                Ok(()) if had_stale_entry => LinkOutcome::RecreatedStale,
                Ok(()) => LinkOutcome::Created,
                Err(_) => LinkOutcome::Failed,
            };
            (*cmd, outcome)
        })
        .collect()
}

/// Windows 启动钩子：为主程序同级目录下的 coreutils.exe 建立 `tools/` 硬链接。
/// dev 模式下 coreutils.exe 可能未编译（cargo run 只构建默认 bin），此时整体
/// 跳过且不影响启动；PATH 注入的 tools 目录为空时命令自然回退到系统 PATH。
#[cfg(target_os = "windows")]
pub fn ensure_installed_tools() {
    let Ok(exe_path) = std::env::current_exe() else {
        return;
    };
    let Some(app_dir) = exe_path.parent() else {
        return;
    };
    let source = app_dir.join(format!("coreutils{}", std::env::consts::EXE_SUFFIX));
    let tools_dir = app_dir.join("tools");
    let results = ensure_tool_links(&source, &tools_dir);
    for (cmd, outcome) in &results {
        if *outcome == LinkOutcome::Failed {
            log::warn!("[tools] 硬链接创建失败: {cmd}");
        }
    }
    if results
        .iter()
        .any(|(_, o)| *o == LinkOutcome::SkippedNoSource)
    {
        log::info!(
            "[tools] coreutils.exe 不存在，跳过工具链接（dev 模式需先 cargo build --bin coreutils）"
        );
        return;
    }
    let touched = results
        .iter()
        .filter(|(_, o)| matches!(o, LinkOutcome::Created | LinkOutcome::RecreatedStale))
        .count();
    if touched > 0 {
        log::info!(
            "[tools] 已更新 {touched}/{} 个 coreutils 硬链接",
            all_commands().len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "veloterm-hardlinks-test-{}-{}",
            tag,
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn all(outcomes: &[(&'static str, LinkOutcome)], want: LinkOutcome) -> bool {
        !outcomes.is_empty() && outcomes.iter().all(|(_, o)| *o == want)
    }

    #[test]
    fn creates_links_then_idempotent() {
        let dir = tempdir("basic");
        let source = dir.join("coreutils");
        fs::write(&source, vec![0u8; 1024]).unwrap();
        let tools = dir.join("tools");

        // 首次：全部创建
        let first = ensure_tool_links(&source, &tools);
        assert!(all(&first, LinkOutcome::Created), "first: {first:?}");
        for cmd in all_commands() {
            assert!(tools.join(cmd).is_file(), "{cmd} 链接应存在");
        }

        // 真硬链接验证（unix 下同 inode，非拷贝）
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let src_meta = fs::metadata(&source).unwrap();
            let ls_meta = fs::metadata(tools.join("ls")).unwrap();
            assert_eq!(src_meta.ino(), ls_meta.ino(), "ls 应与 coreutils 同 inode");
        }

        // 二次：全部幂等跳过
        let second = ensure_tool_links(&source, &tools);
        assert!(
            all(&second, LinkOutcome::KeptExisting),
            "second: {second:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn recreates_stale_links_after_source_update() {
        let dir = tempdir("stale");
        let source = dir.join("coreutils");
        fs::write(&source, vec![0u8; 512]).unwrap();
        let tools = dir.join("tools");
        let _ = ensure_tool_links(&source, &tools);

        // 模拟应用升级：安装器先删旧 coreutils 再写新文件 → 源换成新 inode。
        // 注意不能直接 fs::write 源文件——硬链接共享 inode，原地写会连同
        // 链接一起更新，必须经「新文件 rename 替换」制造新 inode。
        let staged = dir.join("coreutils.new");
        fs::write(&staged, vec![1u8; 2048]).unwrap();
        fs::rename(&staged, &source).unwrap();
        let third = ensure_tool_links(&source, &tools);
        assert!(all(&third, LinkOutcome::RecreatedStale), "third: {third:?}");
        // 重建后大小与新源一致 → 再次运行回到幂等跳过
        let fourth = ensure_tool_links(&source, &tools);
        assert!(
            all(&fourth, LinkOutcome::KeptExisting),
            "fourth: {fourth:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_entirely_when_source_missing() {
        let dir = tempdir("missing");
        let tools = dir.join("tools");
        let outcomes = ensure_tool_links(&dir.join("coreutils-not-built"), &tools);
        assert!(all(&outcomes, LinkOutcome::SkippedNoSource));
        // 目录可能已创建，但不应有任何链接文件
        for cmd in all_commands() {
            assert!(!tools.join(cmd).exists(), "{cmd} 不应被创建");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn link_names_carry_exe_suffix_on_windows() {
        let name = link_file_name("ls");
        if cfg!(windows) {
            assert_eq!(name, "ls.exe");
        } else {
            assert_eq!(name, "ls");
        }
    }
}
