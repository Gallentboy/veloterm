//! 生产环境命令安全分析引擎（Safety Guard）。
//!
//! 基于 `tree-sitter-bash` 进行 AST 语法分析，能够精准区分真实命令节点与
//! 字符串字面量 / 注释（彻底杜绝 `echo "rm -rf /"` 等字符串误报）。
//! 支持穿透 `sudo`, `nohup`, `env`, `time`, `doas`, `builtin`, `exec` 等命令包装，
//! 并支持递归分析 `bash -c "..."` / `sh -c "..."` 内嵌脚本。

use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};

/// 危险等级
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DangerLevel {
    Safe,
    Warning,
    High,
    Critical,
}

impl DangerLevel {
    pub fn severity_score(&self) -> u8 {
        match self {
            DangerLevel::Safe => 0,
            DangerLevel::Warning => 1,
            DangerLevel::High => 2,
            DangerLevel::Critical => 3,
        }
    }
}

/// 安全分析结果
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetyInspectResult {
    pub level: DangerLevel,
    pub matched_rule: Option<String>,
    pub rule_title: Option<String>,
    pub explanation: Option<String>,
    pub offending_command: Option<String>,
    pub require_verification_code: bool,
}

impl SafetyInspectResult {
    pub fn safe() -> Self {
        Self {
            level: DangerLevel::Safe,
            matched_rule: None,
            rule_title: None,
            explanation: None,
            offending_command: None,
            require_verification_code: false,
        }
    }
}

/// 解析后的扁平命令结构
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    /// 真正的可执行命令名（已穿透 sudo/nohup）
    pub name: String,
    /// 参数列表（已去除包装选项）
    pub args: Vec<String>,
    /// 原始命令文本切片
    pub raw_text: String,
}

/// 检查整段脚本/命令行的安全性。
/// 仅当 `host_env` 为 `"prod"` 或 `"staging"` 时做高危阻断；非生产环境直接返回 Safe。
#[tauri::command]
pub fn inspect_command_safety(command: String, host_env: String) -> SafetyInspectResult {
    // 仅在预发或生产环境启用强拦截；dev/test 直接放行
    let is_production_like = matches!(host_env.trim().to_lowercase().as_str(), "prod" | "production" | "staging");
    if !is_production_like {
        return SafetyInspectResult::safe();
    }

    analyze_command_safety(&command)
}

/// 纯文本安全分析核心函数
pub fn analyze_command_safety(script: &str) -> SafetyInspectResult {
    let trimmed = script.trim();
    if trimmed.is_empty() {
        return SafetyInspectResult::safe();
    }

    // 0. 特殊快速检测：
    // - 注释行直接放行（在 Bash 语法中整行注释不执行任何指令）
    if trimmed.starts_with('#') {
        return SafetyInspectResult::safe();
    }

    // - Fork 炸弹（AST 解析可能会将其拆解为函数，直接文本模式最稳妥）
    if trimmed.contains(":(){ :|:& };:") || trimmed.contains(":(){:|:&};:") {
        return SafetyInspectResult {
            level: DangerLevel::Critical,
            matched_rule: Some("fork-bomb".to_string()),
            rule_title: Some("Fork 炸弹".to_string()),
            explanation: Some("试图耗尽系统进程表导致操作系统死机崩溃".to_string()),
            offending_command: Some(":(){ :|:& };:".to_string()),
            require_verification_code: true,
        };
    }

    // 1. Rust 剥离 Shell Prompt 提示符（消除 # 注释误判及 $ / % 语法解析异常）
    let cleaned = strip_shell_prompt(trimmed);

    // 2. 将清洗后的命令构建 AST 并进行规则评估
    let ast_res = inspect_ast(cleaned);
    if ast_res.level != DangerLevel::Safe {
        return ast_res;
    }

    // 3. 方案 3：全量 AST 候选段滑动探测（Sliding Window Scanner）
    // 应对未识别的奇异自定义主题（如零标点 "my-custom-prompt rm -rf /"）
    if let Some(candidate_res) = scan_ast_candidates(trimmed) {
        return candidate_res;
    }

    SafetyInspectResult::safe()
}

/// 剥离终端 Shell Prompt 提示符前缀。
/// 包含传统 Unix 符号（#、$、%、>）与现代主题终止符（❯、➜、»、λ、▶、→、›、⚡ 等），
/// 结合引号扫描保护，确保不会误伤字符串内部的字符（如 echo "hello # world"）。
pub fn strip_shell_prompt(line: &str) -> &str {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return "";
    }

    let bytes = trimmed.as_bytes();
    let len = bytes.len();

    // 引号状态扫描：单引号、双引号、反引号内部不作为提示符切分点
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let mut in_quote = vec![false; len];

    let mut prev_byte = 0u8;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'"' && !in_single && prev_byte != b'\\' {
            in_double = !in_double;
        } else if b == b'\'' && !in_double {
            in_single = !in_single;
        } else if b == b'`' && !in_single && prev_byte != b'\\' {
            in_backtick = !in_backtick;
        }
        if let Some(slot) = in_quote.get_mut(i) {
            *slot = in_single || in_double || in_backtick;
        }
        prev_byte = b;
    }

    // 从后往前逆向寻找最近的有效 Prompt 终止符
    for (byte_idx, ch) in trimmed.char_indices().rev() {
        if in_quote.get(byte_idx).copied().unwrap_or(false) {
            continue;
        }
        // 如果是 '#' 且位于行首，这是以 # 开头的纯注释而非提示符终止符
        if ch == '#' && byte_idx == 0 {
            continue;
        }
        if is_prompt_terminator(ch) {
            let next_idx = byte_idx + ch.len_utf8();
            // 终止符后面通常必须跟着空白字符
            if bytes.get(next_idx).is_some_and(|b| b.is_ascii_whitespace())
                && let Some(candidate) = trimmed.get(next_idx..)
            {
                let cleaned = candidate.trim_start();
                if !cleaned.is_empty() {
                    return cleaned;
                }
            }
        }
    }

    // 辅助规则：识别 "): "、"]: "
    if let Some(pos) = trimmed.rfind("]: ")
        && !in_quote.get(pos).copied().unwrap_or(false)
        && let Some(candidate) = trimmed.get(pos + 3..)
    {
        return candidate.trim_start();
    }
    if let Some(pos) = trimmed.rfind("): ")
        && !in_quote.get(pos).copied().unwrap_or(false)
        && let Some(candidate) = trimmed.get(pos + 3..)
    {
        return candidate.trim_start();
    }

    trimmed
}

fn is_prompt_terminator(ch: char) -> bool {
    matches!(
        ch,
        '#' | '$' | '%' | '>' | '❯' | '➜' | '»' | 'λ' | '▶' | '→' | '›' | '﹥' | '⚡' | '✗' | '✔' | '✓' | '✘'
    )
}

/// 解析指定脚本并在 AST 语法树上评估危险规则
fn inspect_ast(script: &str) -> SafetyInspectResult {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_bash::LANGUAGE.into()).is_err() {
        return SafetyInspectResult::safe();
    }

    let Some(tree) = parser.parse(script, None) else {
        return SafetyInspectResult::safe();
    };

    let source = script.as_bytes();
    let root = tree.root_node();

    // 收集所有处于调用位置的可执行命令
    let mut commands = Vec::new();
    collect_commands(root, source, &mut commands);

    // 逐个命令评估规则，保留最高危险级别
    let mut highest_result = SafetyInspectResult::safe();

    for cmd in commands {
        if let Some(res) = evaluate_command(&cmd)
            && res.level.severity_score() > highest_result.level.severity_score()
        {
            highest_result = res;
            if highest_result.level == DangerLevel::Critical {
                break;
            }
        }
    }

    highest_result
}

const SENSITIVE_COMMAND_TRIGGERS: &[&str] = &[
    "rm", "sudo", "mkfs", "fdisk", "parted", "sfdisk", "dd",
    "reboot", "shutdown", "poweroff", "halt", "init", "kill",
    "redis-cli", "iptables", "ufw", "systemctl", "chmod", "chown",
];

/// 方案 3：全量 AST 候选段滑动探测
fn scan_ast_candidates(script: &str) -> Option<SafetyInspectResult> {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_bash::LANGUAGE.into()).is_err() {
        return None;
    }
    let tree = parser.parse(script, None)?;
    let root = tree.root_node();

    for trigger in SENSITIVE_COMMAND_TRIGGERS {
        let mut search_from = 0;
        while let Some(rel_idx) = script.get(search_from..).and_then(|s| s.find(trigger)) {
            let offset = search_from + rel_idx;
            search_from = offset + trigger.len();

            // 检查该敏感词是否是独立词法边界
            let is_start_boundary = offset == 0
                || script
                    .as_bytes()
                    .get(offset.saturating_sub(1))
                    .is_some_and(|&b| b.is_ascii_whitespace() || b == b';' || b == b'&' || b == b'|' || b == b'>' || b == b'#' || b == b'$');
            let next_offset = offset + trigger.len();
            let is_end_boundary = next_offset >= script.len()
                || script
                    .as_bytes()
                    .get(next_offset)
                    .is_some_and(|&b| b.is_ascii_whitespace() || b == b';' || b == b'&' || b == b'|');

            if !is_start_boundary || !is_end_boundary {
                continue;
            }

            // 检查该 offset 是否处于字符串字面量 (string / raw_string) 或注释 (comment) 内
            if is_offset_in_string_or_comment(root, offset) {
                continue;
            }

            // 提取从 offset 开始的候选切片
            if let Some(candidate) = script.get(offset..) {
                let trimmed_candidate = candidate.trim();
                if trimmed_candidate != script.trim() {
                    let mut candidate_parser = Parser::new();
                    if candidate_parser.set_language(&tree_sitter_bash::LANGUAGE.into()).is_ok()
                        && let Some(c_tree) = candidate_parser.parse(trimmed_candidate, None)
                    {
                        let mut c_commands = Vec::new();
                        collect_commands(c_tree.root_node(), trimmed_candidate.as_bytes(), &mut c_commands);
                        for cmd in c_commands {
                            if let Some(res) = evaluate_command(&cmd)
                                && res.level != DangerLevel::Safe
                            {
                                return Some(res);
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// 检查给定字节偏移量在语法树中是否位于字符串字面量或注释内部
fn is_offset_in_string_or_comment(node: Node, offset: usize) -> bool {
    let start = node.start_byte();
    let end = node.end_byte();

    if offset < start || offset >= end {
        return false;
    }

    let kind = node.kind();
    if kind == "string" || kind == "raw_string" || kind == "comment" {
        return true;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if is_offset_in_string_or_comment(child, offset) {
            return true;
        }
    }

    false
}

/// 递归收集 AST 中的所有真实 command 节点
fn collect_commands(node: Node, source: &[u8], out: &mut Vec<ParsedCommand>) {
    if node.kind() == "command" {
        if let Some(parsed) = parse_command_node(node, source) {
            // 如果是 bash -c "..." 或 sh -c "..."，递归分析内嵌脚本
            if (parsed.name == "bash" || parsed.name == "sh" || parsed.name == "zsh")
                && parsed.args.iter().any(|a| a == "-c")
                && let Some(c_pos) = parsed.args.iter().position(|a| a == "-c")
                && let Some(nested_script) = parsed.args.get(c_pos + 1)
            {
                let mut sub_commands = Vec::new();
                let mut sub_parser = Parser::new();
                if sub_parser.set_language(&tree_sitter_bash::LANGUAGE.into()).is_ok()
                    && let Some(sub_tree) = sub_parser.parse(nested_script, None)
                {
                    collect_commands(sub_tree.root_node(), nested_script.as_bytes(), &mut sub_commands);
                }
                out.extend(sub_commands);
            }

            out.push(parsed);
        }
        return;
    }

    // 递归子节点
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_commands(child, source, out);
    }
}

/// 解析单个 command 节点，穿透 sudo / nohup / env 等包装
fn parse_command_node(node: Node, source: &[u8]) -> Option<ParsedCommand> {
    let raw_text = node.utf8_text(source).ok()?.to_string();

    let mut words = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        let kind = child.kind();
        // 排除重定向、注释等非参数节点
        if kind.contains("redirect") || kind == "comment" {
            continue;
        }
        if let Ok(text) = child.utf8_text(source) {
            let clean = strip_quotes(text);
            if !clean.is_empty() {
                words.push(clean);
            }
        }
    }

    if words.is_empty() {
        return None;
    }

    // 穿透包装命令（如 sudo -u root nohup ...）
    let mut idx = 0;
    while idx < words.len() {
        let Some(w) = words.get(idx) else { break };
        let base_name = w.rsplit('/').next().unwrap_or(w);

        if is_wrapper_command(base_name) {
            idx += 1;
            // 跳过该包装命令的参数（如 sudo 的 -u, -E, -i 等）
            while let Some(opt) = words.get(idx) {
                if !opt.starts_with('-') {
                    break;
                }
                let is_opt_with_val = opt == "-u" || opt == "-g";
                idx += 1;
                if is_opt_with_val && idx < words.len() {
                    idx += 1;
                }
            }
        } else {
            break;
        }
    }

    let real_command = words.get(idx)?.clone();
    let args = words.get(idx + 1..).map(|s| s.to_vec()).unwrap_or_default();

    Some(ParsedCommand {
        name: real_command,
        args,
        raw_text,
    })
}

fn strip_quotes(s: &str) -> String {
    let trimmed = s.trim();
    if ((trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
        && trimmed.len() >= 2
        && let Some(inner) = trimmed.get(1..trimmed.len().saturating_sub(1))
    {
        return inner.to_string();
    }
    trimmed.to_string()
}

fn is_wrapper_command(cmd: &str) -> bool {
    matches!(
        cmd,
        "sudo" | "nohup" | "env" | "time" | "doas" | "builtin" | "exec" | "xargs"
    )
}

/// 对单个命令和参数列表做规则判定
fn evaluate_command(cmd: &ParsedCommand) -> Option<SafetyInspectResult> {
    let name = cmd.name.rsplit('/').next().unwrap_or(&cmd.name).to_lowercase();
    let args = &cmd.args;

    // 1. rm 命令判定
    if name == "rm" {
        let has_recursive = args.iter().any(|a| {
            a.starts_with('-') && !a.starts_with("--") && (a.contains('r') || a.contains('R'))
        }) || args.iter().any(|a| a == "--recursive");

        let targets: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).map(|s| s.as_str()).collect();

        if has_recursive {
            for target in &targets {
                let t = target.trim();
                // 危险根目标
                if t == "/" || t == "/*" || t == "/." || t == "~" || t == "~/" || t == "$HOME" || t == "./*" {
                    return Some(SafetyInspectResult {
                        level: DangerLevel::Critical,
                        matched_rule: Some("rm-root".to_string()),
                        rule_title: Some("根目录或全盘递归删除".to_string()),
                        explanation: Some(format!("尝试递归删除危险路径 '{}'，将导致操作系统或主目录彻底损毁", t)),
                        offending_command: Some(cmd.raw_text.clone()),
                        require_verification_code: true,
                    });
                }
            }

            // 普通递归删除（不是根目录，但也带 -r）-> Warning
            if !targets.is_empty() {
                return Some(SafetyInspectResult {
                    level: DangerLevel::Warning,
                    matched_rule: Some("rm-recursive".to_string()),
                    rule_title: Some("递归删除目录".to_string()),
                    explanation: Some(format!("递归删除目标 '{}'，操作不可逆，请核对目标路径", targets.join(", "))),
                    offending_command: Some(cmd.raw_text.clone()),
                    require_verification_code: false,
                });
            }
        }
    }

    // 2. 磁盘底层格式化与写入：mkfs.*, fdisk, parted, dd of=/dev/sd*
    if name.starts_with("mkfs") || name == "fdisk" || name == "parted" || name == "sfdisk" {
        return Some(SafetyInspectResult {
            level: DangerLevel::Critical,
            matched_rule: Some("disk-format".to_string()),
            rule_title: Some("底层磁盘操作与格式化".to_string()),
            explanation: Some(format!("执行磁盘格式化/分区工具 '{}'，可能破坏分区表并造成全盘数据丢失", name)),
            offending_command: Some(cmd.raw_text.clone()),
            require_verification_code: true,
        });
    }

    if name == "dd" {
        for arg in args {
            if let Some(dev) = arg.strip_prefix("of=/dev/")
                && (dev.starts_with("sd") || dev.starts_with("vd") || dev.starts_with("nvme") || dev.starts_with("mapper"))
            {
                return Some(SafetyInspectResult {
                    level: DangerLevel::Critical,
                    matched_rule: Some("dd-block-device".to_string()),
                    rule_title: Some("底层块设备覆写".to_string()),
                    explanation: Some(format!("dd 直接向块设备 '{}' 写入数据，极易破坏文件系统", arg)),
                    offending_command: Some(cmd.raw_text.clone()),
                    require_verification_code: true,
                });
            }
        }
    }

    // 3. 根目录权限全量篡改：chmod -R 777 /
    if (name == "chmod" || name == "chown") && args.iter().any(|a| a.contains('R')) {
        let targets: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).map(|s| s.as_str()).collect();
        for target in targets {
            let t = target.trim();
            if t == "/" || t == "/*" {
                return Some(SafetyInspectResult {
                    level: DangerLevel::Critical,
                    matched_rule: Some("chmod-root".to_string()),
                    rule_title: Some("根目录权限全局篡改".to_string()),
                    explanation: Some("递归修改根目录权限或所属者会导致操作系统关键组件（如 sudo, sshd）拒绝运行".to_string()),
                    offending_command: Some(cmd.raw_text.clone()),
                    require_verification_code: true,
                });
            }
        }
    }

    // 4. 关机与重启：shutdown, reboot, halt, poweroff, init 0, init 6
    if matches!(name.as_str(), "shutdown" | "reboot" | "halt" | "poweroff") {
        return Some(SafetyInspectResult {
            level: DangerLevel::High,
            matched_rule: Some("power-control".to_string()),
            rule_title: Some("系统关机或重启".to_string()),
            explanation: Some(format!("执行 '{}' 将导致生产服务器下线或服务中断", name)),
            offending_command: Some(cmd.raw_text.clone()),
            require_verification_code: false,
        });
    }

    if name == "init" && args.iter().any(|a| a == "0" || a == "6") {
        return Some(SafetyInspectResult {
            level: DangerLevel::High,
            matched_rule: Some("init-runlevel".to_string()),
            rule_title: Some("切换系统运行级别至关机/重启".to_string()),
            explanation: Some("init 0 或 init 6 将关闭或重启操作系统".to_string()),
            offending_command: Some(cmd.raw_text.clone()),
            require_verification_code: false,
        });
    }

    // 5. 杀掉 1 号进程：kill -9 1
    if name == "kill" && args.iter().any(|a| a == "1") && args.iter().any(|a| a == "-9" || a == "-KILL") {
        return Some(SafetyInspectResult {
            level: DangerLevel::High,
            matched_rule: Some("kill-init".to_string()),
            rule_title: Some("强制终止 1 号进程".to_string()),
            explanation: Some("强制杀死 init/systemd 进程将引发系统崩溃 (Kernel Panic)".to_string()),
            offending_command: Some(cmd.raw_text.clone()),
            require_verification_code: false,
        });
    }

    // 6. Redis 危险操作：redis-cli flushall / flushdb
    if name == "redis-cli" {
        for arg in args {
            let lower = arg.to_lowercase();
            if lower == "flushall" || lower == "flushdb" {
                return Some(SafetyInspectResult {
                    level: DangerLevel::High,
                    matched_rule: Some("redis-flush".to_string()),
                    rule_title: Some("Redis 清空所有键值".to_string()),
                    explanation: Some(format!("执行 redis-cli {} 将抹除缓存或持久化数据，可能引发雪崩", lower)),
                    offending_command: Some(cmd.raw_text.clone()),
                    require_verification_code: false,
                });
            }
        }
    }

    // 7. 防火墙停用：iptables -F, ufw disable
    if (name == "iptables" && args.iter().any(|a| a == "-F" || a == "--flush"))
        || (name == "ufw" && args.iter().any(|a| a == "disable"))
    {
        return Some(SafetyInspectResult {
            level: DangerLevel::High,
            matched_rule: Some("firewall-disable".to_string()),
            rule_title: Some("清空或停用防火墙".to_string()),
            explanation: Some("关闭防火墙规则可能切断已有安全防护或导致 SSH 意外断开".to_string()),
            offending_command: Some(cmd.raw_text.clone()),
            require_verification_code: false,
        });
    }

    // 8. systemctl 关键服务停止
    if name == "systemctl"
        && let (Some(action), Some(service)) = (args.first(), args.get(1))
        && (action == "stop" || action == "restart")
        && matches!(
            service.trim_end_matches(".service"),
            "sshd" | "ssh" | "docker" | "k8s" | "kubelet" | "mysqld" | "mysql" | "nginx" | "redis"
        )
    {
        return Some(SafetyInspectResult {
            level: DangerLevel::Warning,
            matched_rule: Some("systemctl-service-control".to_string()),
            rule_title: Some("停止或重启核心服务".to_string()),
            explanation: Some(format!("将对核心服务 '{}' 执行 {} 操作，可能引起瞬时抖动", service, action)),
            offending_command: Some(cmd.raw_text.clone()),
            require_verification_code: false,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_commands() {
        assert_eq!(analyze_command_safety("ls -lh /var/log").level, DangerLevel::Safe);
        assert_eq!(analyze_command_safety("git status").level, DangerLevel::Safe);
        assert_eq!(analyze_command_safety("cargo test --workspace").level, DangerLevel::Safe);
    }

    #[test]
    fn test_echo_string_not_false_positive() {
        // 关键用例：echo 包含高危命令的字符串，绝不能误报！
        let cmd = r#"echo "warning: never run rm -rf / or reboot in prod""#;
        assert_eq!(analyze_command_safety(cmd).level, DangerLevel::Safe);

        let comment = "# rm -rf / is dangerous";
        assert_eq!(analyze_command_safety(comment).level, DangerLevel::Safe);
    }

    #[test]
    fn test_rm_root_critical() {
        assert_eq!(analyze_command_safety("rm -rf /").level, DangerLevel::Critical);
        assert_eq!(analyze_command_safety("rm -fr /*").level, DangerLevel::Critical);
        assert_eq!(analyze_command_safety("rm -r -f ~/").level, DangerLevel::Critical);
        assert_eq!(analyze_command_safety("rm -rf ./*").level, DangerLevel::Critical);

        // 穿透 sudo
        assert_eq!(analyze_command_safety("sudo rm -rf /").level, DangerLevel::Critical);
        assert_eq!(analyze_command_safety("sudo -u root nohup rm -rf /").level, DangerLevel::Critical);

        // 普通目录递归删除是 Warning，不是 Critical
        assert_eq!(analyze_command_safety("rm -rf /tmp/my_build_dir").level, DangerLevel::Warning);
    }

    #[test]
    fn test_disk_and_dd_critical() {
        assert_eq!(analyze_command_safety("mkfs.ext4 /dev/sdb1").level, DangerLevel::Critical);
        assert_eq!(analyze_command_safety("fdisk /dev/nvme0n1").level, DangerLevel::Critical);
        assert_eq!(analyze_command_safety("dd if=/dev/zero of=/dev/sda bs=1M").level, DangerLevel::Critical);
    }

    #[test]
    fn test_power_control_high() {
        assert_eq!(analyze_command_safety("reboot").level, DangerLevel::High);
        assert_eq!(analyze_command_safety("sudo shutdown -h now").level, DangerLevel::High);
        assert_eq!(analyze_command_safety("init 0").level, DangerLevel::High);
        assert_eq!(analyze_command_safety("kill -9 1").level, DangerLevel::High);
    }

    #[test]
    fn test_redis_and_firewall() {
        assert_eq!(analyze_command_safety("redis-cli flushall").level, DangerLevel::High);
        assert_eq!(analyze_command_safety("sudo iptables -F").level, DangerLevel::High);
    }

    #[test]
    fn test_fork_bomb() {
        assert_eq!(analyze_command_safety(":(){ :|:& };:").level, DangerLevel::Critical);
    }

    #[test]
    fn test_nested_sh_c() {
        assert_eq!(
            analyze_command_safety(r#"bash -c "echo hello && rm -rf /""#).level,
            DangerLevel::Critical
        );
    }

    #[test]
    fn test_custom_prompt_with_sliding_window() {
        // 方案 3 核心验证：即使提示符完全未剥离且没有终止符，滑动 AST 探测依然精准触发
        let cmd = "my-weird-custom-prompt-without-symbol rm -rf /";
        assert_eq!(analyze_command_safety(cmd).level, DangerLevel::Critical);

        let cmd2 = "fancy-theme-git-branch-main reboot";
        assert_eq!(analyze_command_safety(cmd2).level, DangerLevel::High);

        // 字符串字面量保护：即便包含高危词，只要在引号内就绝不误报
        let cmd3 = "my-weird-prompt echo 'rm -rf /'";
        assert_eq!(analyze_command_safety(cmd3).level, DangerLevel::Safe);
    }

    #[test]
    fn test_strip_shell_prompt_and_real_terminals() {
        // 1. strip_shell_prompt 单元功能验证
        assert_eq!(strip_shell_prompt("[root@fa3cec97e2e2 ~]# rm -rf /"), "rm -rf /");
        assert_eq!(strip_shell_prompt("ubuntu@ip-172-31-0-1:~$ sudo rm -rf /"), "sudo rm -rf /");
        assert_eq!(strip_shell_prompt("macbook% reboot"), "reboot");
        assert_eq!(strip_shell_prompt("starship ❯ rm -rf /"), "rm -rf /");
        assert_eq!(strip_shell_prompt("agnoster ➜ init 0"), "init 0");
        assert_eq!(strip_shell_prompt("pure λ kill -9 1"), "kill -9 1");
        assert_eq!(strip_shell_prompt("(my-venv) [user@host project]$ reboot"), "reboot");

        // 2. 引号保护：字符串内部的 # / $ 决不能被剥离
        assert_eq!(
            strip_shell_prompt(r#"echo "hello # world""#),
            r#"echo "hello # world""#
        );

        // 3. 全链路安全分析验证（前端完全不剥离，直接扔整行原始终端文本）
        assert_eq!(
            analyze_command_safety("[root@fa3cec97e2e2 ~]# rm -rf /").level,
            DangerLevel::Critical
        );
        assert_eq!(
            analyze_command_safety("ubuntu@ip-172-31-0-1:~$ sudo rm -rf /*").level,
            DangerLevel::Critical
        );
        assert_eq!(
            analyze_command_safety("macbook% reboot").level,
            DangerLevel::High
        );
        assert_eq!(
            analyze_command_safety("starship-theme ❯ mkfs.ext4 /dev/sdb1").level,
            DangerLevel::Critical
        );
        assert_eq!(
            analyze_command_safety(r#"echo "[root@localhost ~]# rm -rf /""#).level,
            DangerLevel::Safe
        );
    }
}
