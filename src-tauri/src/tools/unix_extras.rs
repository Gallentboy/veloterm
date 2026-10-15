//! 自研 Unix 工具补充：which。
//!
//! which 不属于 GNU coreutils / findutils / diffutils（是独立小工具），各
//! uutils 仓库均未提供，由本文件实现编入 coreutils 多呼叫二进制（分发见
//! `coreutils.rs`）。grep/sed/rg/fd/sd/awk/diff 家族均来自 vendor 源码，
//! 不在本文件。
//!
//! which：按 `PATH`（Windows 含 PATHEXT 后缀）查找可执行文件。

use std::env;

// ── which ──
pub fn cmd_which(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("Usage: which <command>");
        return 1;
    }
    let path_var = env::var("PATH").unwrap_or_default();
    let mut code = 0;
    for cmd in args {
        let mut found = false;
        for dir in env::split_paths(&path_var) {
            let candidate = dir.join(cmd);
            let candidate_str = candidate.to_string_lossy();
            #[cfg(windows)]
            let exists = ["", ".exe", ".bat", ".cmd", ".com"]
                .iter()
                .any(|ext| std::path::Path::new(&format!("{}{}", candidate_str, ext)).exists());
            #[cfg(not(windows))]
            let exists = candidate.exists();
            if exists {
                println!("{}", candidate_str);
                found = true;
                break;
            }
        }
        if !found {
            eprintln!("which: no {} in ({})", cmd, path_var);
            code = 1;
        }
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "veloterm-extras-test-{}-{}",
            tag,
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn which_finds_executable_on_path() {
        let dir = tempdir("which");
        let bin_dir = dir.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let exe = bin_dir.join("veloterm-test-cmd-4815162342");
        fs::write(&exe, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        }

        // 操纵 PATH 后校验命中/未命中退出码（测试进程内串行执行，结束时恢复）
        let original = env::var("PATH").unwrap_or_default();
        unsafe {
            env::set_var("PATH", &bin_dir);
        }
        let found = cmd_which(&["veloterm-test-cmd-4815162342".into()]);
        let missing = cmd_which(&["veloterm-no-such-cmd-4815162342".into()]);
        unsafe {
            env::set_var("PATH", &original);
        }
        assert_eq!(found, 0);
        assert_eq!(missing, 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
