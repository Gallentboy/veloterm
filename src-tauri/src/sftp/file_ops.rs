//! 远程文件在线编辑的 SFTP 读写命令。
//!
//! 走既有 [`super::manager::get_or_connect_sftp`] 通道：`session_id` 传
//! `host_id` 字符串即可命中 manager 的按 host_id 动态连接逻辑，manager 零改动。
//!
//! - 读取：先 stat 校验大小上限，再读前 512 字节做文本嗅探，通过后全量读取；
//! - 写入：同目录临时文件 + rename 原子覆盖（SFTP v3 rename 在目标已存在时
//!   会被 OpenSSH 拒绝，故失败时回退为「删除目标后再 rename」）。

use serde::Serialize;
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use super::commands::shell_single_quote;
use super::manager::{
    get_or_connect_sftp, get_sftp_ssh_handle, resolve_host_chain, sftp_exec_capture_status,
    sftp_exec_with_stdin,
};
use super::types::{SftpClient, SftpState};
use crate::terminal::ssh::SSHTargetAuth;
use crate::text_detect::{MAX_EDITABLE_FILE_SIZE_BYTES, is_editable_text};

/// 文本嗅探探针长度（与 `is_text_buffer` 约定一致）。
const SNIFF_PROBE_BYTES: usize = 512;

/// `sftp_read_file` 的返回载荷（camelCase 契约）。
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SftpFileContent {
    pub content: String,
    pub size: u64,
}

/// 从远端路径提取纯文件名（最后一个 `/` 之后的部分）。
fn file_name_of(remote_path: &str) -> &str {
    remote_path.rsplit('/').next().unwrap_or(remote_path)
}

/// SFTP 远端错误的展示格式化（client 调用：open/create/metadata/rename/…）。
///
/// russh-sftp 的 Display 是 `"{状态码名}: {服务器消息}"`，而 OpenSSH sftp-server
/// 的消息文本通常与状态码名同文（如 "Permission denied: Permission denied"）。
/// 此处：消息为空或与状态码名忽略大小写同文时只保留一份；自定义 sftp
/// 服务器的不同文详细消息保留 `"{码名}: {消息}"`；非 Status 错误
/// （IO/Timeout 等）原样输出。
pub(crate) fn sftp_error_message(e: russh_sftp::client::error::Error) -> String {
    use russh_sftp::client::error::Error;
    match &e {
        Error::Status(status) => {
            let code = status.status_code.to_string();
            let message = status.error_message.trim();
            if message.is_empty() || message.eq_ignore_ascii_case(&code) {
                code
            } else {
                format!("{code}: {message}")
            }
        }
        _ => e.to_string(),
    }
}

/// 远端流式 IO 错误（read/write/seek）的字符串去重：库把 [`sftp_error_message》
/// 所面对的那份重复 Display 整串塞进了 `io::Error` 的 message（其 file.rs 的
/// `io::Error::new(ErrorKind::Other, e.to_string())`），无法解构回 Status，
/// 按 `"X: X"` 首尾同文折叠。仅用于远端流式错误，本地 fs 错误不适用。
pub(crate) fn dedup_sftp_status_text(text: &str) -> String {
    if let Some((head, tail)) = text.split_once(": ")
        && head.eq_ignore_ascii_case(tail.trim())
    {
        return head.to_string();
    }
    text.to_string()
}

/// 生成与目标文件同目录的临时文件路径：`<dir>/.veloterm.tmp.<uuid>`。
pub(crate) fn sibling_temp_path(remote_path: &str) -> String {
    let temp_name = format!(".veloterm.tmp.{}", Uuid::new_v4());
    match remote_path.rsplit_once('/') {
        // "/a.txt" → "/.veloterm.tmp.<uuid>"（根目录）
        Some(("", _)) => format!("/{temp_name}"),
        Some((dir, _)) => format!("{dir}/{temp_name}"),
        // 相对路径（无 '/'）→ 临时文件落在 SFTP 工作目录
        None => temp_name,
    }
}

/// 提取远端 POSIX 路径的所在目录（纯字符串处理）。
///
/// SFTP 远端路径恒以 `/` 分隔，**不能**用 `std::path::Path::parent()`——
/// 那会按**宿主平台**语义解析（Windows 上是 `\`），语义错位。
/// - `/a.txt` → `Some("/")`
/// - `/a/b.txt` → `Some("/a")`
/// - `a.txt`（相对路径，无目录部分）→ `None`
pub(crate) fn remote_dir_of(remote_path: &str) -> Option<&str> {
    match remote_path.rsplit_once('/') {
        Some(("", _)) => Some("/"),
        Some((dir, _)) => Some(dir),
        None => None,
    }
}

/// 将临时文件 rename 覆盖目标：SFTP v3 rename 在目标已存在时会被部分服务器
/// （如 OpenSSH sftp-server）拒绝，此时回退为「删除目标后再 rename」；
/// 二次 rename 仍失败则删除临时文件并返回错误。
pub(crate) async fn commit_temp_file(
    client: &SftpClient,
    temp: &str,
    target: &str,
) -> Result<(), String> {
    if let Err(first_err) = client.rename(temp, target).await {
        let _ = client.remove_file(target).await;
        if let Err(retry_err) = client.rename(temp, target).await {
            let _ = client.remove_file(temp).await;
            return Err(format!(
                "Failed to rename temp file onto '{target}': {}; retry after remove: {}",
                sftp_error_message(first_err),
                sftp_error_message(retry_err)
            ));
        }
    }
    Ok(())
}

/// 读取远端文本文件内容（大小上限 + 文本嗅探校验）。
#[tauri::command]
pub async fn sftp_read_file(
    host_id: i64,
    remote_path: String,
    state: State<'_, SftpState>,
) -> Result<SftpFileContent, String> {
    let session_key = host_id.to_string();
    let t0 = std::time::Instant::now();
    let client = get_or_connect_sftp(&state, &session_key).await?;
    let t_conn = t0.elapsed();

    // 1. stat 校验大小上限
    let t1 = std::time::Instant::now();
    let meta = client
        .metadata(&remote_path)
        .await
        .map_err(|e| format!("Failed to stat remote file '{remote_path}': {}", sftp_error_message(e)))?;
    let t_stat = t1.elapsed();
    let size = meta.len();
    if size > MAX_EDITABLE_FILE_SIZE_BYTES {
        return Err("文件超过 5MB，不可编辑".to_string());
    }

    // 2. 打开文件，读前 512 字节做文本嗅探
    let t2 = std::time::Instant::now();
    let mut remote_file = client
        .open(&remote_path)
        .await
        .map_err(|e| format!("Failed to open remote file '{remote_path}': {}", sftp_error_message(e)))?;
    let t_open = t2.elapsed();

    let mut probe = vec![0u8; SNIFF_PROBE_BYTES.min(size as usize)];
    let mut probed = 0usize;
    while probed < probe.len() {
        let Some(chunk) = probe.get_mut(probed..) else {
            break;
        };
        let n = remote_file
            .read(chunk)
            .await
            .map_err(|e| {
                format!(
                    "Failed to read remote file probe: {}",
                    dedup_sftp_status_text(&e.to_string())
                )
            })?;
        if n == 0 {
            break;
        }
        probed += n;
    }
    probe.truncate(probed);

    let name = file_name_of(&remote_path);
    if !is_editable_text(Some(name), &probe) {
        return Err("不是可编辑的文本文件".to_string());
    }

    // 3. 嗅探通过后全量读取（探针字节复用，不重复读）
    let mut data = probe;
    data.reserve((size as usize).saturating_sub(data.len()));
    let mut buffer = [0u8; 65536];
    loop {
        let n = remote_file
            .read(&mut buffer)
            .await
            .map_err(|e| {
                format!(
                    "Failed to read remote file '{remote_path}': {}",
                    dedup_sftp_status_text(&e.to_string())
                )
            })?;
        if n == 0 {
            break;
        }
        if let Some(written) = buffer.get(..n) {
            data.extend_from_slice(written);
        }
        if data.len() as u64 > MAX_EDITABLE_FILE_SIZE_BYTES {
            return Err("文件超过 5MB，不可编辑".to_string());
        }
    }

    let t3 = std::time::Instant::now();
    let size = data.len() as u64;
    // 显式关闭句柄：泄漏的 open handle 会在服务器侧累积，导致后续操作变慢/挂起
    let _ = remote_file.close().await;
    log::info!(
        "[SFTP] read_file '{remote_path}' done: conn={t_conn:?} stat={t_stat:?} open={t_open:?} read={:?} total={:?} bytes={size}",
        t3.elapsed(),
        t0.elapsed()
    );
    Ok(SftpFileContent {
        content: String::from_utf8_lossy(&data).into_owned(),
        size,
    })
}

/// 原子写入远端文件：同目录临时文件写入成功后 rename 覆盖目标。
#[tauri::command]
pub async fn sftp_write_file(
    host_id: i64,
    remote_path: String,
    content: String,
    state: State<'_, SftpState>,
) -> Result<(), String> {
    let session_key = host_id.to_string();
    let client = get_or_connect_sftp(&state, &session_key).await?;

    let temp_path = sibling_temp_path(&remote_path);

    // 1. 写临时文件（create = O_WRONLY|O_CREAT|O_TRUNC）
    let mut temp_file = client
        .create(&temp_path)
        .await
        .map_err(|e| format!("Failed to create temp file '{temp_path}': {}", sftp_error_message(e)))?;
    temp_file
        .write_all(content.as_bytes())
        .await
        .map_err(|e| {
            format!(
                "Failed to write temp file '{temp_path}': {}",
                dedup_sftp_status_text(&e.to_string())
            )
        })?;
    temp_file
        .shutdown()
        .await
        .map_err(|e| {
            format!(
                "Failed to flush temp file '{temp_path}': {}",
                dedup_sftp_status_text(&e.to_string())
            )
        })?;

    // 2. 尽力保留原文件权限（原文件不存在或 setstat 不受支持时忽略）
    if let Ok(origin_meta) = client.metadata(&remote_path).await
        && let Some(perms) = origin_meta.permissions
    {
        let attrs = russh_sftp::protocol::FileAttributes {
            permissions: Some(perms),
            ..Default::default()
        };
        let _ = client.set_metadata(&temp_path, attrs).await;
    }

    // 3. rename 覆盖目标（目标已存在时回退为「删除目标后再 rename」）
    commit_temp_file(&client, &temp_path, &remote_path).await?;

    log::info!(
        "[SFTP] Atomically wrote {} bytes to '{remote_path}'",
        content.len()
    );
    Ok(())
}

/* ─────────────────── sudo 提权读写（Permission denied 的编辑/预览兜底） ───────────────────
 *
 * 场景：登录用户对目标文件无读/写权限（root 属主的配置文件等）。读走
 * `sudo cat 家族`（head -c 限长），写走 `sudo tee`（stdin 灌内容 + EOF，
 * 原位覆写且保留属主/权限）。不走 PTY——PTY 的本地回显与 ONLCR 翻译会
 * 污染文件内容；密码经 `sudo -S` 从 stdin 首行读取（sudo 逐字节消费，
 * 不会吞掉后续内容），不进命令行（远端 ps 不可见）、不落日志。
 *
 * 密码三候选（优先级）：本次入参（前端弹窗输入）> 会话缓存（验证通过后
 * 按主机记存、纯内存）> 主机登录密码凭证（SQLCipher 解密）。任何密码候选
 * 都先经 `sudo -S -k -v` 验证再干活——直接把密码和文件内容一起灌给 tee，
 * 密码错误时 sudo 会把后续内容行当密码重试吃掉，文件必写坏。
 * 无任何候选时先探 NOPASSWD（`sudo -n`），需要密码则返回
 * SUDO_PASSWORD_REQUIRED 由前端弹窗收集。
 */

/// sudo 凭据形态
#[derive(Clone, PartialEq)]
enum SudoCred {
    /// NOPASSWD 免密（`sudo -n`，零打扰）
    NoPassword,
    /// 密码提权（`sudo -S`，密码走 stdin 首行）
    WithPassword(String),
}

/// 组装 stdin：密码行（若有）+ 文件内容（若有）
fn sudo_stdin(cred: &SudoCred, content: Option<&str>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let SudoCred::WithPassword(password) = cred {
        bytes.extend_from_slice(password.as_bytes());
        bytes.push(b'\n');
    }
    if let Some(content) = content {
        bytes.extend_from_slice(content.as_bytes());
    }
    bytes
}

/// sudo 读取命令：head -c 读 MAX+1 字节——长度超过 MAX 即判超限
/// （区分恰好等于上限的合法文件），GNU/BSD/busybox head 均支持 -c。
/// 路径为列表构造的绝对路径，仅作 $1 单引号包裹传递（同 chmod/chown 范式）。
fn build_sudo_read_command(path: &str, cred: &SudoCred) -> String {
    let sudo = match cred {
        SudoCred::NoPassword => "sudo -n",
        SudoCred::WithPassword(_) => "sudo -S -p \"\"",
    };
    format!(
        "sh -c '{sudo} head -c {} \"$1\"' veloterm-sudo-read {}",
        MAX_EDITABLE_FILE_SIZE_BYTES + 1,
        shell_single_quote(path),
    )
}

/// sudo 写入命令：tee 原位覆写（POSIX 规定 O_TRUNC，不留旧尾巴）且保留
/// 属主/权限；tee 自身回显的内容重定向 /dev/null
fn build_sudo_write_command(path: &str, cred: &SudoCred) -> String {
    let sudo = match cred {
        SudoCred::NoPassword => "sudo -n",
        SudoCred::WithPassword(_) => "sudo -S -p \"\"",
    };
    format!(
        "sh -c '{sudo} tee \"$1\" > /dev/null' veloterm-sudo-write {}",
        shell_single_quote(path),
    )
}

/// sudo 失败分类（稳定前缀码，前端映射 i18n 文案）
fn classify_sudo_failure(output: &str, code: Option<u32>) -> String {
    if code == Some(127) {
        return "SUDO_MISSING".to_string();
    }
    let lower = output.to_lowercase();
    if lower.contains("a password is required") {
        return "SUDO_PASSWORD_REQUIRED".to_string();
    }
    if lower.contains("incorrect password")
        || lower.contains("sorry, try again")
        || lower.contains("authentication failure")
    {
        return "SUDO_PASSWORD_WRONG".to_string();
    }
    if lower.contains("not in the sudoers") || lower.contains("not allowed to execute") {
        return "SUDO_NOT_ALLOWED".to_string();
    }
    if lower.contains("no tty present") || lower.contains("a terminal is required") {
        return "SUDO_NO_TTY".to_string();
    }
    let tail: String = output.trim().chars().take(200).collect();
    format!("REMOTE:{tail}")
}

/// 主机登录密码凭证（auth=Password 时存在，SQLCipher 已解密形态）
fn stored_host_password(host_id: i64) -> Option<String> {
    let chain = resolve_host_chain(host_id as usize).ok()?;
    match &chain.last()?.auth {
        SSHTargetAuth::Password { password } => Some(password.clone()),
        _ => None,
    }
}

/// 验证密码：`sudo -S -k -v`（stdin 只喂密码行 + EOF；sudo 逐字节读密码、
/// 不碰后续数据）。stdin 不含任何文件内容，密码错误时 sudo 重试读到 EOF
/// 即失败退出——不存在吞内容问题。
async fn validate_sudo_password(
    ssh: &super::types::SftpSshHandle,
    password: &str,
) -> Result<(), String> {
    let stdin = format!("{password}\n");
    let (output, code) = sftp_exec_with_stdin(ssh, "sudo -S -k -v -p \"\"", stdin.as_bytes(), 10)
        .await
        .map_err(|e| format!("EXEC_CHANNEL:{e}"))?;
    match code {
        Some(0) => Ok(()),
        other => Err(classify_sudo_failure(&output, other)),
    }
}

/// 解析 sudo 凭据：入参 > 会话缓存 > 主机登录密码；有候选先验证（通过落
/// 缓存，失败清缓存交前端重输），无候选探 NOPASSWD。
async fn resolve_sudo_credential(
    state: &State<'_, SftpState>,
    ssh: &super::types::SftpSshHandle,
    host_id: i64,
    password_param: Option<String>,
    stored: Option<String>,
) -> Result<SudoCred, String> {
    let cached = state.sudo_passwords.lock().await.get(&host_id).cloned();
    let candidate = password_param.or(cached).or(stored);
    if let Some(password) = candidate {
        return match validate_sudo_password(ssh, &password).await {
            Ok(()) => {
                state
                    .sudo_passwords
                    .lock()
                    .await
                    .insert(host_id, password.clone());
                Ok(SudoCred::WithPassword(password))
            }
            Err(e) => {
                // 密码失效（远端改密/凭证过期）：清缓存，前端弹窗重输
                state.sudo_passwords.lock().await.remove(&host_id);
                Err(e)
            }
        };
    }
    // 无任何密码候选：探测 NOPASSWD（-n 非交互，需要密码时立即失败）
    let (output, code) = sftp_exec_capture_status(ssh, "sudo -n -v", 10)
        .await
        .map_err(|e| format!("EXEC_CHANNEL:{e}"))?;
    match code {
        Some(0) => Ok(SudoCred::NoPassword),
        other => Err(classify_sudo_failure(&output, other)),
    }
}

/// sudo 提权读取（预览/编辑的 Permission denied 兜底）：NOPASSWD 或验证
/// 通过的密码读前 MAX+1 字节，超限判 TOO_LARGE，二进制判 NOT_TEXT。
#[tauri::command]
pub async fn sftp_sudo_read(
    host_id: i64,
    remote_path: String,
    password: Option<String>,
    state: State<'_, SftpState>,
) -> Result<SftpFileContent, String> {
    let session_key = host_id.to_string();
    let _client = get_or_connect_sftp(&state, &session_key).await?;
    let Some(ssh) = get_sftp_ssh_handle(&state, &session_key).await else {
        return Err("EXEC_CHANNEL:SSH 会话不可用".to_string());
    };
    let stored = stored_host_password(host_id);
    let cred = resolve_sudo_credential(&state, &ssh, host_id, password, stored).await?;
    let command = build_sudo_read_command(&remote_path, &cred);
    let stdin = sudo_stdin(&cred, None);
    let (output, code) = sftp_exec_with_stdin(&ssh, &command, &stdin, 20)
        .await
        .map_err(|e| format!("EXEC_CHANNEL:{e}"))?;
    match code {
        Some(0) => {}
        other => return Err(classify_sudo_failure(&output, other)),
    }
    if output.len() as u64 > MAX_EDITABLE_FILE_SIZE_BYTES {
        return Err("TOO_LARGE".to_string());
    }
    let name = file_name_of(&remote_path);
    let probe_len = output.len().min(SNIFF_PROBE_BYTES);
    let probe = output.as_bytes().get(..probe_len).unwrap_or(&[]);
    if !is_editable_text(Some(name), probe) {
        return Err("NOT_TEXT".to_string());
    }
    let size = output.len() as u64;
    log::info!(
        "[SFTP] sudo read '{remote_path}' done: {size} bytes, cred={}",
        matches!(cred, SudoCred::NoPassword)
    );
    Ok(SftpFileContent {
        content: output,
        size,
    })
}

/// sudo 提权写入：tee 原位覆写（与常规路径的临时文件+rename 相比无原子性——
/// 中途失败会留半截文件，提权路径接受此取舍）。stdin = 密码行 + 内容 + EOF。
#[tauri::command]
pub async fn sftp_sudo_write(
    host_id: i64,
    remote_path: String,
    content: String,
    password: Option<String>,
    state: State<'_, SftpState>,
) -> Result<(), String> {
    let session_key = host_id.to_string();
    let _client = get_or_connect_sftp(&state, &session_key).await?;
    let Some(ssh) = get_sftp_ssh_handle(&state, &session_key).await else {
        return Err("EXEC_CHANNEL:SSH 会话不可用".to_string());
    };
    let stored = stored_host_password(host_id);
    let cred = resolve_sudo_credential(&state, &ssh, host_id, password, stored).await?;
    let command = build_sudo_write_command(&remote_path, &cred);
    let stdin = sudo_stdin(&cred, Some(&content));
    // 内容可达 5MiB：慢链路写入留足时间
    let (output, code) = sftp_exec_with_stdin(&ssh, &command, &stdin, 60)
        .await
        .map_err(|e| format!("EXEC_CHANNEL:{e}"))?;
    match code {
        Some(0) => {
            log::info!(
                "[SFTP] sudo wrote {} bytes to '{remote_path}'",
                content.len()
            );
            Ok(())
        }
        other => Err(classify_sudo_failure(&output, other)),
    }
}

#[cfg(test)]
mod tests {    use super::*;

    #[test]
    fn file_name_extraction() {
        assert_eq!(file_name_of("/etc/nginx/nginx.conf"), "nginx.conf");
        assert_eq!(file_name_of("/hosts"), "hosts");
        assert_eq!(file_name_of("relative.txt"), "relative.txt");
    }

    #[test]
    fn temp_paths_stay_in_same_directory() {
        let path = sibling_temp_path("/etc/nginx/nginx.conf");
        assert!(
            path.starts_with("/etc/nginx/.veloterm.tmp."),
            "temp file must be sibling of target: {path}"
        );

        let root = sibling_temp_path("/hosts");
        assert!(
            root.starts_with("/.veloterm.tmp."),
            "root-level temp: {root}"
        );

        let relative = sibling_temp_path("notes.md");
        assert!(
            relative.starts_with(".veloterm.tmp."),
            "relative temp: {relative}"
        );
    }

    #[test]
    fn read_payload_serializes_camel_case() {
        let payload = SftpFileContent {
            content: "hello".to_string(),
            size: 5,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"content\":\"hello\""));
        assert!(json.contains("\"size\":5"));
    }

    /// 构造 SFTP status 错误（protocol::Status 字段全 pub）
    fn status_error(
        code: russh_sftp::protocol::StatusCode,
        message: &str,
    ) -> russh_sftp::client::error::Error {
        russh_sftp::client::error::Error::Status(russh_sftp::protocol::Status {
            id: 0,
            status_code: code,
            error_message: message.to_string(),
            language_tag: String::new(),
        })
    }

    #[test]
    fn sftp_error_message_dedups_same_text_status() {
        // OpenSSH 常态：消息文本与状态码名同文（忽略大小写）→ 只留一份
        assert_eq!(
            sftp_error_message(status_error(
                russh_sftp::protocol::StatusCode::PermissionDenied,
                "Permission denied",
            )),
            "Permission denied"
        );
        assert_eq!(
            sftp_error_message(status_error(
                russh_sftp::protocol::StatusCode::NoSuchFile,
                "no such file",
            )),
            "No such file"
        );
    }

    #[test]
    fn sftp_error_message_keeps_distinct_detail() {
        // 自定义 sftp 服务器的不同文详细消息保留
        assert_eq!(
            sftp_error_message(status_error(
                russh_sftp::protocol::StatusCode::PermissionDenied,
                "quota exceeded on /home",
            )),
            "Permission denied: quota exceeded on /home"
        );
        // 空消息 → 只留码名
        assert_eq!(
            sftp_error_message(status_error(
                russh_sftp::protocol::StatusCode::Failure,
                "  ",
            )),
            "Failure"
        );
        // 非 Status 变体原样
        assert_eq!(
            sftp_error_message(russh_sftp::client::error::Error::Timeout),
            "Timeout"
        );
    }

    #[test]
    fn dedup_status_text_folds_same_text_pair() {
        assert_eq!(
            dedup_sftp_status_text("Permission denied: Permission denied"),
            "Permission denied"
        );
        // 不同文 / 无分隔符 → 原样
        assert_eq!(
            dedup_sftp_status_text("Failure: custom detail"),
            "Failure: custom detail"
        );
        assert_eq!(dedup_sftp_status_text("No such file"), "No such file");
    }

    // ── sudo 提权 ──

    #[test]
    fn sudo_command_shapes() {
        assert_eq!(
            build_sudo_read_command(
                "/etc/shadow",
                &SudoCred::WithPassword("pw".to_string())
            ),
            "sh -c 'sudo -S -p \"\" head -c 5242881 \"$1\"' veloterm-sudo-read '/etc/shadow'"
        );
        assert_eq!(
            build_sudo_write_command("/x", &SudoCred::NoPassword),
            "sh -c 'sudo -n tee \"$1\" > /dev/null' veloterm-sudo-write '/x'"
        );
        // 注入元字符路径仍只作一个 argv 位
        let evil = build_sudo_read_command("a'; rm -rf /", &SudoCred::NoPassword);
        assert!(evil.ends_with("a'\\''; rm -rf /'"));
    }

    #[test]
    fn sudo_stdin_composition() {
        // 密码行 + 内容；NOPASSWD 只有内容
        assert_eq!(
            sudo_stdin(&SudoCred::WithPassword("pw".into()), Some("body")),
            b"pw\nbody".to_vec()
        );
        assert_eq!(sudo_stdin(&SudoCred::NoPassword, Some("body")), b"body".to_vec());
        assert_eq!(sudo_stdin(&SudoCred::WithPassword("pw".into()), None), b"pw\n".to_vec());
    }

    #[test]
    fn sudo_failure_classification() {
        assert_eq!(
            classify_sudo_failure("sudo: a password is required", Some(1)),
            "SUDO_PASSWORD_REQUIRED"
        );
        assert_eq!(
            classify_sudo_failure("sudo: 1 incorrect password attempt", Some(1)),
            "SUDO_PASSWORD_WRONG"
        );
        assert_eq!(
            classify_sudo_failure("Sorry, try again.", Some(1)),
            "SUDO_PASSWORD_WRONG"
        );
        assert_eq!(
            classify_sudo_failure("x is not in the sudoers file.", Some(1)),
            "SUDO_NOT_ALLOWED"
        );
        assert_eq!(
            classify_sudo_failure("sudo: no tty present and no askpass program specified", Some(1)),
            "SUDO_NO_TTY"
        );
        assert_eq!(classify_sudo_failure("sh: sudo: command not found", Some(127)), "SUDO_MISSING");
        assert_eq!(
            classify_sudo_failure("weird failure", Some(2)),
            "REMOTE:weird failure"
        );
    }
}
