//! 内置工具集的「命令名清单」——**单一事实来源**，且**零依赖**。
//!
//! 为什么单独成 crate：命令名同时被两个方向使用，
//! - **主程序 app**：启动时按清单在 `tools/` 下创建硬链接短名入口（`ls.exe`
//!   `grep.exe`…，见 `src/tools/hardlinks.rs`）；
//! - **coreutils 多呼叫二进制**：按清单做 argv[0] 分发与 usage 列表。
//!
//! 若清单留在 coreutils 里，主程序就必须依赖 coreutils → 连带链接 uutils /
//! ripgrep / openssl-app / putty-telnet / jq-app 整张依赖图，正是本次拆分要
//! 消除的问题。
//! 故把「有哪些命令」与「命令怎么实现」彻底分开：本 crate 只有字符串表，
//! 实现全部在 `crates/coreutils`。

/// 内嵌补充命令：OWN_ARGV 家族（rg/fd/awk/parallel）+ 自研 which（coreutils 的
/// `unix_extras.rs`）+ 自研 ps / top（不经 uutils 的 uumain 表，由
/// `run_command` 早退分发）
pub const EXTRA_COMMANDS: &[&str] = &["awk", "fd", "parallel", "rg", "which", "ps", "top"];

/// 跨平台命令（102 个）：uutils feat_common_core 与 feat_Tier1 全集，
/// 加 uutils/grep、uutils/sed、findutils、diffutils、chmln/sd 家族补充，
/// 加 tar-rs、eza、wget-rs 的库封装（tree = eza --tree）
pub const CROSS_PLATFORM_COMMANDS: &[&str] = &[
    "arch",
    "b2sum",
    "base32",
    "base64",
    "basename",
    "basenc",
    "cat",
    "cksum",
    "cmp",
    "comm",
    "cp",
    "csplit",
    "cut",
    "date",
    "dd",
    "df",
    "diff",
    "dir",
    "dircolors",
    "dirname",
    "du",
    "echo",
    "env",
    "eza",
    "expand",
    "expr",
    "factor",
    "false",
    "find",
    "fmt",
    "fold",
    "grep",
    "head",
    "hostid",
    "hostname",
    "join",
    "kill",
    "link",
    "ln",
    "ls",
    "md5sum",
    "mkdir",
    "mktemp",
    "more",
    "mv",
    "nl",
    "nice",
    "nohup",
    "nproc",
    "numfmt",
    "od",
    "paste",
    "pathchk",
    "pr",
    "printenv",
    "printf",
    "ptx",
    "pwd",
    "readlink",
    "realpath",
    "rm",
    "rmdir",
    "sd",
    "sdiff",
    "sed",
    "seq",
    "sha1sum",
    "sha224sum",
    "sha256sum",
    "sha384sum",
    "sha512sum",
    "shred",
    "shuf",
    "sleep",
    "sort",
    "split",
    "sum",
    "sync",
    "tac",
    "tar",
    "tail",
    "tee",
    "test",
    "timeout",
    "touch",
    "tr",
    "tree",
    "true",
    "truncate",
    "tsort",
    "tty",
    "uname",
    "unexpand",
    "uniq",
    "unlink",
    "uptime",
    "vdir",
    "wc",
    "wget",
    "whoami",
    "xargs",
    "yes",
];

/// Unix 专属命令（uutils feat_require_unix_core 12 个 + utmpx 3 个 + stdbuf）
#[cfg(unix)]
pub const PLATFORM_COMMANDS: &[&str] = &[
    "chgrp", "chmod", "chown", "chroot", "groups", "id", "install", "logname", "mkfifo", "mknod",
    "stat", "stty", "pinky", "users", "who", "stdbuf",
];

/// Windows 专属命令（uutils feat_os_windows 额外项 + vcpkg 静态链接的 openssl CLI +
/// 源码内嵌 PuTTY 的 telnet + vendor 源码现场编译的 bc/jq CLI + yqffi/fzfffi/
/// qalcffi.dll 动态导入的 yq、fzf 与 qalc + urlencoding 库封装的 urlencode/urldecode
/// + data-encoding 家族的 base32hex/base64url/hex/hexupper——base32/base64 已由
/// 跨平台清单的 uutils 实现覆盖，不重复收录）
#[cfg(target_os = "windows")]
pub const PLATFORM_COMMANDS: &[&str] = &[
    "base32hex", "base64url", "bc", "fzf", "hex", "hexupper", "jq", "openssl", "qalc",
    "stdbuf", "telnet", "urldecode", "urlencode", "yq",
];

/// 其余平台（本项目不存在，仅保证常量无条件可用）
#[cfg(not(any(unix, target_os = "windows")))]
pub const PLATFORM_COMMANDS: &[&str] = &[];

/// 全部命令（hardlink 短名与 usage 列表的单一事实来源）
pub fn all_commands() -> Vec<&'static str> {
    let mut all = Vec::with_capacity(
        CROSS_PLATFORM_COMMANDS.len() + PLATFORM_COMMANDS.len() + EXTRA_COMMANDS.len(),
    );
    all.extend_from_slice(CROSS_PLATFORM_COMMANDS);
    all.extend_from_slice(PLATFORM_COMMANDS);
    all.extend_from_slice(EXTRA_COMMANDS);
    all
}

/// 是否为已知命令
pub fn is_known_command(name: &str) -> bool {
    CROSS_PLATFORM_COMMANDS.contains(&name)
        || PLATFORM_COMMANDS.contains(&name)
        || EXTRA_COMMANDS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 清单自身的结构性约束：无重复；自研命令不得与 uutils 命令重名。
    #[test]
    fn command_table_consistency() {
        let all = all_commands();
        let mut sorted = all.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len());
        for cmd in EXTRA_COMMANDS {
            assert!(
                !CROSS_PLATFORM_COMMANDS.contains(cmd) && !PLATFORM_COMMANDS.contains(cmd),
                "{cmd} 不应与 uutils 命令重名"
            );
        }
    }
}
