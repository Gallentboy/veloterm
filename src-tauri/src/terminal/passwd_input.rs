//! 改密交互输入捕获 + 自动回写——shell 内 passwd 流程的密码变更检测。
//!
//! 原理：终端输入/输出流必经客户端。输出流出现改密提示词 → 进入捕获态；
//! 输入流的下一行即该轮密码。三轮收集完（current/new/retype）且 new ==
//! retype 时回写数据库。
//!
//! ## 提示词匹配：两级策略
//!
//! 1. **用户配置**（优先）：主机编辑页可配三条精确提示词，程序严格按配置
//!    匹配。准确度高——用户从实际终端复制粘贴提示词原文，无误命中。
//! 2. **内置默认**（兜底）：未配置时用内置的常见模式（contains 匹配），
//!    覆盖英文/中文主流 PAM 发行版。准确度较低（可能被含密码字样的普通
//!    输出误触发），仅在未配置时生效。
//!
//! 配置了任一条就整组切换到用户配置（三条都配齐才生效，缺一 fallback 内置），
//! 避免混合匹配（用户配了 current + 程序猜 new → 用户配了就没意义了）。
//!
//! ```text
//! 输出: <current_prompt> → 捕获下一输入行 → current
//! 输出: <new_prompt>     → → new
//! 输出: <retype_prompt>  → → retype；new==retype 时回写 DB
//! ```

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::Mutex;

/// 用户配置的三条提示词（主机编辑页 → hosts.passwd_prompts JSON 列）
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswdPrompts {
    #[serde(default)]
    pub current: String,
    #[serde(default)]
    pub new: String,
    #[serde(default)]
    pub retype: String,
}

impl PasswdPrompts {
    /// 三条都非空才视为完整配置（任一为空 → 用内置默认）
    pub fn is_complete(&self) -> bool {
        !self.current.is_empty() && !self.new.is_empty() && !self.retype.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Await {
    Current,
    New,
    Retype,
}

#[derive(Default)]
struct SessionState {
    await_kind: Option<Await>,
    buf: Vec<u8>,
    scan: Vec<u8>,
    /// 已捕获的新密码（new/retype 各一次，用于一致性确认）
    new_entries: Vec<String>,
    current_entry: Option<String>,
    /// 该会话的主机 id（回写数据库用）
    host_id: Option<usize>,
    /// 用户配置的提示词（None = 用内置默认）
    custom_prompts: Option<PasswdPrompts>,
    /// 本轮已回写过（防止弱密码重试时重复写库——重试成功会覆盖，但避免
    /// 中间态写入），在 cleanup 或下一个改密流程开始时重置
    written: bool,
    /// AppHandle（写库成功后发 host-password-updated 事件刷新前端）
    app_handle: Option<tauri::AppHandle>,
    /// 终端输出通道（回写成功后向终端写「已保存」提示；用户输入 retype
    /// 回车的瞬间就能发——先于服务端响应，不会跟服务器输出混在一起）
    output_channel: Option<tauri::ipc::Channel<Vec<u8>>>,
    /// 已匹配到的提示词原文（语言检测：含 CJK 即中文提示）
    detected_prompt_text: String,
    /// 曾进入过改密流程（跨「两轮提示之间 await_kind 短暂为 None」的间隙）。
    /// shell-ready 判断用它兜底：进过流程就必须等 `await_kind` 归 None 且
    /// 出现明确结束标志（written 置位或收到 shell 提示符）才放行。
    saw_passwd_flow: bool,
    /// 改密链已被服务端接受（retype 轮 new==retype 且非空——PAM 只在两次
    /// 一致时才返回 SUCCESS 的协议不变量）。是「认证通过/流程走完」的判据，
    /// 与 `written`（已存库）**共同构成** shell-ready 的放行条件（缺一不可）：
    /// 后续若再出现改密提示（弱密码被 PAM 拒绝重来）则回置 false——第一轮
    /// retype 匹配虽已写库，流程没走完就绝不能放行。
    flow_verified: bool,
}

/// 全局注册表：terminal_id → 状态
static GLOBAL: LazyLock<PasswdInputLog> = LazyLock::new(PasswdInputLog::new);

pub struct PasswdInputLog {
    sessions: Mutex<HashMap<usize, SessionState>>,
}

/// 扫描窗口上限
const SCAN_WINDOW: usize = 160;

impl PasswdInputLog {
    fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// 全局访问点
    pub fn global() -> &'static Self {
        &GLOBAL
    }

    /// 会话初始化：SSH 会话建立时调用，注入 host_id 归因与用户配置的提示词。
    /// 未调用（本地终端等）→ 状态保持默认（无 host_id，不回写）
    pub fn configure_session(
        &self,
        terminal_id: usize,
        host_id: Option<usize>,
        prompts: PasswdPrompts,
        app_handle: Option<tauri::AppHandle>,
        output_channel: Option<tauri::ipc::Channel<Vec<u8>>>,
    ) {
        let mut map = self.sessions.lock().unwrap();
        let sc = map.entry(terminal_id).or_default();
        sc.host_id = host_id;
        sc.app_handle = app_handle;
        sc.output_channel = output_channel;
        // 只有完整配置才启用自定义匹配（缺条数时 fallback 内置）
        sc.custom_prompts = if prompts.is_complete() {
            Some(prompts)
        } else {
            None
        };
    }

    /// 输出流：扫描提示词，驱动捕获状态。
    /// 在 SSH 的 Output 分支调用（透传前）。
    pub fn feed_output(&self, terminal_id: usize, chunk: &[u8]) {
        let mut map = self.sessions.lock().unwrap();
        let sc = map
            .entry(terminal_id)
            .or_default();
        sc.scan.extend_from_slice(chunk);
        if sc.scan.len() > SCAN_WINDOW {
            let overflow = sc.scan.len() - SCAN_WINDOW;
            sc.scan.drain(..overflow);
        }
        let text = String::from_utf8_lossy(&sc.scan).to_lowercase();

        // 提示词判定（从具体到一般）：用户配置优先，内置默认兜底。
        // 用户配置支持多行（每行一条，命中任意即匹配——textarea 输入）
        let lines_match = |field: &str, text: &str| {
            field
                .lines()
                .filter(|l| !l.trim().is_empty())
                .any(|l| text.contains(&l.trim().to_lowercase()))
        };
        let matched_retype = match sc.custom_prompts.as_ref() {
            Some(p) => lines_match(&p.retype, &text),
            None => {
                text.contains("retype new password")
                    || text.contains("re-enter new password")
                    || text.contains("重新输入")
                    || text.contains("再次输入")
            }
        };
        let matched_new = match sc.custom_prompts.as_ref() {
            Some(p) => lines_match(&p.new, &text),
            None => {
                text.contains("new password:") || text.contains("新密码") || text.contains("新的密码")
            }
        };
        let matched_current = match sc.custom_prompts.as_ref() {
            Some(p) => lines_match(&p.current, &text),
            None => {
                text.contains("current password")
                    || text.contains("current) unix password")
                    || text.contains("当前密码")
                    || text.contains("当前的密码")
            }
        };
        // 密码过期强制改密的**前置警告**：这段文本先于 `(current) UNIX
        // password:` 到达（sshd 的 do_pwchange 先打 WARNING，PAM 随后才出
        // 提示词；两者常分属不同输出块）。若只认提示词，警告块到达时
        // is_awaiting_password 仍为 false → ShellReady 立刻发射 → 就绪钩子
        // 在用户刚开始输密码时把钩子命令敲进去（实测：改密必失败）。
        // 故警告一出现就置 saw_passwd_flow，提前压住 shell-ready。
        let matched_expiry_warning = {
            text.contains("password has expired")
                || text.contains("must change your password")
                || text.contains("you are required to change")
                || text.contains("password expired")
                || text.contains("密码已过期")
                || text.contains("密码已经过期")
                || text.contains("必须更改密码")
                || text.contains("必须修改密码")
                || text.contains("需要更改密码")
        };

        if matched_retype {
            sc.await_kind = Some(Await::Retype);
            sc.saw_passwd_flow = true;
            // 新一轮提示开始：上一轮的「已验证」作废（弱密码被拒重来的循环）
            sc.flow_verified = false;
            sc.buf.clear();
            sc.scan.clear();
        } else if matched_new {
            sc.await_kind = Some(Await::New);
            sc.saw_passwd_flow = true;
            sc.flow_verified = false;
            sc.buf.clear();
            sc.scan.clear();
        } else if matched_current {
            sc.await_kind = Some(Await::Current);
            sc.saw_passwd_flow = true;
            sc.flow_verified = false;
            sc.buf.clear();
            sc.scan.clear();
        } else if matched_expiry_warning {
            // 过期警告：此时还没有提示词，不能设 await_kind（否则下一行用户
            // 输入会被当成密码记错轮次）。只置流程标志，让 shell-ready 继续
            // 等待——真正的提示词随后到达会正常进入捕获态。
            sc.saw_passwd_flow = true;
        }
        // 保留已匹配的提示词原文供语言检测（在 clear 之前拷贝）
        if sc.await_kind.is_some() && !text.is_empty() {
            sc.detected_prompt_text = text.clone();
        }
    }

    /// 输入流：捕获态时记录按键（处理退格），回车提交。
    /// 在 terminal_write 调用（旁路镜像，不影响正常转发）。
    pub fn feed_input(&self, terminal_id: usize, chunk: &[u8]) {
        let mut map = self.sessions.lock().unwrap();
        let Some(sc) = map.get_mut(&terminal_id) else {
            return;
        };
        if sc.await_kind.is_none() {
            return;
        }
        for &b in chunk {
            match b {
                b'\r' | b'\n' => {
                    let line = String::from_utf8_lossy(&sc.buf).into_owned();
                    sc.buf.clear();
                    let kind = sc.await_kind.take();
                    match kind {
                        Some(Await::Current) => {
                            sc.current_entry = Some(line);
                        }
                        Some(Await::New) => {
                            sc.new_entries.push(line);
                        }
                        Some(Await::Retype) => {
                            sc.new_entries.push(line.clone());
                            // Retype 提交即判定：new==retype 且非空 → 认证通过
                            // （协议判据，与 DB 回写无关——见 flow_verified）。
                            // （get 而非索引：仓库 deny indexing_slicing）
                            let n = sc.new_entries.len();
                            if n >= 2
                                && let Some(prev) = sc.new_entries.get(n - 2)
                                && *prev == line
                                && !line.is_empty()
                            {
                                sc.flow_verified = true;
                                self.try_write_password(terminal_id, sc, &line);
                            }
                        }
                        None => {}
                    }
                }
                0x7f | 0x08 => {
                    sc.buf.pop();
                }
                _ => sc.buf.push(b),
            }
        }
    }

    /// 回写数据库（锁定已释放后调用——本方法需要 &self 但不锁 map）
    fn try_write_password(&self, terminal_id: usize, sc: &mut SessionState, new_pw: &str) {
        if sc.written {
            return;
        }
        let Some(host_id) = sc.host_id else {
            log::debug!(
                "[SSH {}] shell-passwd: password confirmed but no host_id, skip DB write",
                terminal_id
            );
            return;
        };
        match crate::db::change_host_password(
            host_id,
            new_pw,
            "shell-passwd",
            sc.app_handle.as_ref(),
        ) {
            Ok(true) => {
                sc.written = true;
                log::info!(
                    "[SSH {}] shell-passwd: password changed via shell passwd, host {} updated",
                    terminal_id,
                    host_id
                );
                // 终端提示（SSH 服务端风格纯文本，按会话语言适配）：
                // 用户输完 retype 回车的瞬间就发——本地操作先于服务端响应
                // 到达，不会被服务器输出冲掉
                if let Some(ch) = sc.output_channel.as_ref() {
                    // 用已捕获的提示词检测语言（含 CJK 即中文）
                    let has_cjk = sc
                        .detected_prompt_text
                        .chars()
                        .any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c));
                    let notice = if has_cjk {
                        "\r\n密码已自动保存到主机配置。"
                    } else {
                        "\r\nPassword saved to host settings."
                    };
                    let _ = ch.send(notice.as_bytes().to_vec());
                }
            }
            Ok(false) => {
                log::debug!(
                    "[SSH {}] shell-passwd: host {} is not password-auth, skip",
                    terminal_id, host_id
                );
            }
            Err(e) => {
                log::warn!("[SSH {}] shell-passwd: DB write failed: {}", terminal_id, e);
            }
        }
    }

    /// 清理（会话关闭）
    pub fn cleanup(&self, terminal_id: usize) {
        self.sessions.lock().unwrap().remove(&terminal_id);
    }

    /// 该会话是否处于「认证尚未通过」状态（用于推迟 shell-ready）。
    ///
    /// 密码过期强制改密时，服务器的首个输出是过期警告/`(current) UNIX
    /// password:` 提示，此时若发 ShellReady，就绪钩子会把钩子命令当成密码
    /// 敲进去（实测：改密必失败）。
    ///
    /// 判定不是简单的「此刻在等输入」——改密流程中两轮提示之间 `await_kind`
    /// 会短暂为 None（提示词已清、下一提示未到），只看瞬时态会误放行。
    ///
    /// **放行条件 = 流程走完 且 已存库**（`flow_verified && written`，
    /// 两者缺一不可）：
    /// - 只看 `written` 不够：弱密码被拒重来时第一轮 retype 匹配就会写库，
    ///   重试轮的提示间隙会误放行（`flow_verified` 随新提示回置）；
    /// - 只看 `flow_verified` 不够：改密被接受但尚未存库就放行，钩子命令
    ///   可能赶在存库完成前落地。
    ///
    /// 无 host_id（临时主机无钩子配置）、密钥认证主机（无密码可存）、
    /// DB 写失败等场景不放行——宁可不放行也不在存库前放行。
    ///
    /// **不做超时兜底**：改密失败（用户连续输错）时服务器不会进 shell，
    /// 永远不发 ShellReady 正是正确行为——用户本来就到不了 shell。
    pub fn is_awaiting_password(&self, terminal_id: usize) -> bool {
        let map = self.sessions.lock().unwrap();
        map.get(&terminal_id).is_some_and(|sc| {
            sc.await_kind.is_some()
                || (sc.saw_passwd_flow && !(sc.flow_verified && sc.written))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_all_three_inputs() {
        let cap = PasswdInputLog::new();
        cap.feed_output(1, b"(current) UNIX password: ");
        cap.feed_input(1, b"Old99\r");
        cap.feed_output(1, b"New password: ");
        cap.feed_input(1, b"New88");
        cap.feed_input(1, b"\x7f8\r"); // 退格改尾字符
        cap.feed_output(1, b"Retype new password: ");
        cap.feed_input(1, b"New88\r");
        // 无 host_id → 不写库（try_write_password 内部 skip）
        // 再输入不捕获（无提示词）
        cap.feed_input(1, b"normal\r");
    }

    #[test]
    fn no_capture_without_prompt() {
        let cap = PasswdInputLog::new();
        cap.feed_input(2, b"ls -la\r"); // 无提示词 → 不捕获（无 panic 即过）
    }

    #[test]
    fn custom_prompts_strict_match() {
        let cap = PasswdInputLog::new();
        cap.configure_session(
            3,
            None,
            PasswdPrompts {
                current: "(current) UNIX password:".into(),
                new: "New password:".into(),
                retype: "Retype new password:".into(),
            },
            None, None,
        );
        // 自定义提示词命中
        cap.feed_output(3, b"(current) UNIX password: ");
        cap.feed_input(3, b"old\r");
        cap.feed_output(3, b"New password: ");
        cap.feed_input(3, b"new\r");
        cap.feed_output(3, b"Retype new password: ");
        cap.feed_input(3, b"new\r");
        // 内置默认不应命中（自定义模式激活时）
        // → "新密码" 不会触发（自定义模式只认英文配置）
    }

    #[test]
    fn custom_prompts_incomplete_falls_back() {
        let cap = PasswdInputLog::new();
        cap.configure_session(
            4,
            None,
            PasswdPrompts {
                current: "current password:".into(),
                new: String::new(), // 缺 new
                retype: "retype:".into(),
            },
            None, None,
        );
        // 配置不完整 → fallback 内置默认
        // 中文提示应命中（内置默认含中文）
        cap.feed_output(4, "当前的密码: ".as_bytes());
        cap.feed_input(4, b"old\r");
    }

    #[test]
    fn prompts_config_complete_check() {
        let p = PasswdPrompts::default();
        assert!(!p.is_complete());
        let p = PasswdPrompts {
            current: "a".into(),
            new: "b".into(),
            retype: "c".into(),
        };
        assert!(p.is_complete());
    }

    /// 改密流程未完成时必须压住 shell-ready（否则就绪钩子把命令当密码敲进去）
    #[test]
    fn shell_ready_blocked_during_passwd_flow() {
        let cap = PasswdInputLog::new();
        // 未进流程：不阻塞
        assert!(!cap.is_awaiting_password(10));
        // 命中 current 提示 → 进入流程，阻塞
        cap.feed_output(10, b"Current password: ");
        assert!(cap.is_awaiting_password(10));
        // 两轮提示之间的间隙（await_kind 已消费为 None，但流程未完成）→ 仍阻塞
        cap.feed_input(10, b"old\r");
        assert!(
            cap.is_awaiting_password(10),
            "flow in progress must keep blocking even between prompts"
        );
    }

    /// 密码过期强制改密：sshd 的过期警告先于 passwd 提示词到达（常分属
    /// 不同输出块）。警告块本身不含任何密码提示词，若只认提示词，
    /// is_awaiting_password 在警告块时为 false → ShellReady 提前发射 →
    /// 就绪钩子在用户刚开始输密码时把钩子命令敲进去（实测改密必失败）。
    /// 警告一出现就必须置 saw_passwd_flow 压住 shell-ready。
    #[test]
    fn expiry_warning_blocks_shell_ready_before_prompt_arrives() {
        let cap = PasswdInputLog::new();
        // 第一块：过期警告（sshd do_pwchange 先打 WARNING，提示词还在后块）
        cap.feed_output(
            50,
            b"WARNING: Your password has expired.\r\nYou must change your password now and login again!\r\n",
        );
        assert!(
            cap.is_awaiting_password(50),
            "expiry warning alone must suppress shell-ready before the prompt arrives"
        );
        // 第二块：passwd 真正的提示词到达，进入捕获态，继续阻塞
        cap.feed_output(50, b"Changing password for user test.\r\n(current) UNIX password: ");
        assert!(cap.is_awaiting_password(50));
        // 用户输完 current 轮，间隙仍阻塞（同 shell_ready_blocked_during_passwd_flow）
        cap.feed_input(50, b"oldpw\r");
        assert!(cap.is_awaiting_password(50));
    }

    /// 流程走完（retype 匹配）但未存库 → 仍阻塞：shell-password 改密流程的
    /// 放行条件是「流程走完 且 已存库」两者同时满足（本测试无 host_id，
    /// 无法回写 → 不放行；真实存库后的放行需真实 DB，见集成验证）
    #[test]
    fn shell_ready_blocked_until_flow_done_and_saved() {
        let cap = PasswdInputLog::new();
        cap.feed_output(20, b"Current password: ");
        cap.feed_input(20, b"old\r");
        cap.feed_output(20, b"New password: ");
        cap.feed_input(20, b"newpass\r");
        cap.feed_output(20, b"Retype new password: ");
        assert!(cap.is_awaiting_password(20));
        // retype 与 new 一致 → 流程走完（flow_verified），但无 host_id 未存库
        // → 仍阻塞
        cap.feed_input(20, b"newpass\r");
        assert!(
            cap.is_awaiting_password(20),
            "flow completion without DB save must keep blocking shell-ready"
        );
    }

    /// 弱密码被 PAM 拒绝重来：retype 通过后又出现 New password: 提示 →
    /// 已验证态作废、重新阻塞（第一轮已写库也不行——流程没走完），
    /// 直到最后一轮走完才进入「等存库」态
    #[test]
    fn weak_password_retry_keeps_blocking_until_final_round() {
        let cap = PasswdInputLog::new();
        cap.feed_output(21, b"Current password: ");
        cap.feed_input(21, b"old\r");
        cap.feed_output(21, b"New password: ");
        cap.feed_input(21, b"weak\r");
        cap.feed_output(21, b"Retype new password: ");
        cap.feed_input(21, b"weak\r");
        // PAM 拒绝并重新提示（BAD PASSWORD → New password:）
        cap.feed_output(21, b"BAD PASSWORD: it is too short\r\nNew password: ");
        assert!(
            cap.is_awaiting_password(21),
            "PAM-rejected round must re-block shell-ready"
        );
        cap.feed_input(21, b"str0ngpw\r");
        // 重试轮的提示间隙（await 已消费、新 retype 提示未到）也必须阻塞
        assert!(
            cap.is_awaiting_password(21),
            "gap inside retry round must keep blocking"
        );
        cap.feed_output(21, b"Retype new password: ");
        cap.feed_input(21, b"str0ngpw\r");
        // 最后一轮走完但仍未存库（无 host_id）→ 继续阻塞（等存库）
        assert!(cap.is_awaiting_password(21));
    }
}
