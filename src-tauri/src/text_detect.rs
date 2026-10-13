//! 文本文件检测：判断远程/本地文件是否为可直接编辑的纯文本。
//!
//! 两级判定：
//! 1. [`is_text_file_by_name`] —— 扩展名/常见无后缀文件名静态映射，零 I/O 成本；
//!    白名单/黑名单能确定的直接给结论，未知后缀返回 `None` 交给第二级。
//! 2. [`is_text_buffer`] —— 内容首部字节嗅探（调用方通常只读取前 512 字节）：
//!    零字节规则 + 不可打印控制字符占比阈值（与 Git 的二进制判定思路一致）。
//!
//! 组合入口见 [`is_editable_text`]。
//!
//! 说明：不采用严格 UTF-8 校验（`str::from_utf8`）——GBK/Latin-1 等单字节编码的
//! 纯文本会被误杀；控制字符占比判定对任意单字节编码同样有效，且多字节 UTF-8
//! 序列的连续高位字节（>= 0x80）不计入控制字符统计，不会误伤。

use std::path::Path;

/// SFTP 在线编辑允许的最大文件大小（5 MiB）：列表条目与读取入口共用同一上限。
pub const MAX_EDITABLE_FILE_SIZE_BYTES: u64 = 5 * 1024 * 1024;

/// 无后缀的常见文本配置文件名（全小写比较）。
const EXACT_TEXT_FILES: &[&str] = &[
  "dockerfile",
  "makefile",
  "cmakelists.txt",
  "gemfile",
  "procfile",
  "vagrantfile",
  "licence",
  "license",
  "readme",
];

/// 纯文本扩展名白名单（编程语言、脚本、配置、文档）。
const TEXT_EXTENSIONS: &[&str] = &[
  // 编程语言与脚本
  "rs", "ts", "js", "py", "go", "c", "cpp", "h", "java", "sh", "bash", "zsh", "sql",
  // 配置与文档
  "json", "yaml", "yml", "toml", "xml", "ini", "conf", "cfg", "env", "md", "txt", "log", "csv",
];

/// 常见二进制扩展名黑名单。
const BINARY_EXTENSIONS: &[&str] = &[
  "png", "jpg", "jpeg", "gif", "webp", "zip", "tar", "gz", "so", "dylib", "dll", "bin", "exe",
  "pdf",
];

/// 第一级：仅凭文件名判定是否文本文件。
///
/// - `Some(true)`：文件名/后缀命中文本白名单；
/// - `Some(false)`：命中二进制黑名单；
/// - `None`：未知后缀或无后缀（如 `.gitignore`），需读取内容走第二级嗅探。
pub fn is_text_file_by_name(filename: &str) -> Option<bool> {
  let lower_name = filename.to_lowercase();
  if EXACT_TEXT_FILES.contains(&lower_name.as_str()) {
    return Some(true);
  }

  let ext = Path::new(&lower_name)
    .extension()
    .and_then(|s| s.to_str())
    .map(|s| s.to_lowercase());

  match ext.as_deref() {
    Some(ext) if TEXT_EXTENSIONS.contains(&ext) => Some(true),
    Some(ext) if BINARY_EXTENSIONS.contains(&ext) => Some(false),
    _ => None,
  }
}

/// 第二级：内容字节嗅探（通常传文件前 512 字节）。
///
/// - 零字节规则：正文中出现 `b'\0'` 即判定二进制（UTF-8/ASCII 纯文本不会出现）；
/// - 控制字符阈值：剔除合法控制字符（`\t` `\n` `\r` `\x08` `\x0c`）与 DEL（`0x7F`）外，
///   不可打印字符占比 >= 1.5% 判定二进制；
/// - 空文件按文本处理（允许编辑后保存）。
pub fn is_text_buffer(bytes: &[u8]) -> bool {
  if bytes.is_empty() {
    return true;
  }

  if bytes.contains(&0) {
    return false;
  }

  let non_printable_count = bytes
    .iter()
    .filter(|&&b| {
      (b < 32 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x08 | 0x0C)) || b == 0x7F
    })
    .count();

  (non_printable_count as f32 / bytes.len() as f32) < 0.015
}

/// 组合入口：文件名能确定的直接返回结论，否则对已读取的内容探针做嗅探。
///
/// `filename` 传 `None` 时（调用方拿不到文件名）直接走内容嗅探。
pub fn is_editable_text(filename: Option<&str>, bytes: &[u8]) -> bool {
  if let Some(verdict) = filename.and_then(is_text_file_by_name) {
    return verdict;
  }
  is_text_buffer(bytes)
}

/// SFTP 列表条目级判定：大小不超上限且文件名命中已知文本后缀（白名单）才可编辑。
///
/// 未知后缀（`None`）在列表阶段保守判不可编辑，避免为嗅探逐个读文件；
/// 打开文件时由 [`is_editable_text`] 结合内容探针做二次判定。
pub fn is_editable_sftp_entry(name: &str, size: u64) -> bool {
  size <= MAX_EDITABLE_FILE_SIZE_BYTES && is_text_file_by_name(name) == Some(true)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn exact_names_are_text() {
    assert_eq!(is_text_file_by_name("Dockerfile"), Some(true));
    assert_eq!(is_text_file_by_name("Makefile"), Some(true));
    assert_eq!(is_text_file_by_name("CMakeLists.txt"), Some(true));
  }

  #[test]
  fn text_extensions() {
    assert_eq!(is_text_file_by_name("main.RS"), Some(true));
    assert_eq!(is_text_file_by_name("config.YaMl"), Some(true));
    assert_eq!(is_text_file_by_name("notes.md"), Some(true));
  }

  #[test]
  fn binary_extensions() {
    assert_eq!(is_text_file_by_name("logo.PNG"), Some(false));
    assert_eq!(is_text_file_by_name("app.dmg"), None); // 未知后缀 → 嗅探
    assert_eq!(is_text_file_by_name("archive.tar.gz"), Some(false));
  }

  #[test]
  fn unknown_or_no_extension_defers_to_sniff() {
    assert_eq!(is_text_file_by_name("runbook"), None);
    assert_eq!(is_text_file_by_name(".gitignore"), None);
  }

  #[test]
  fn buffer_detects_text() {
    assert!(is_text_buffer(b"hello, world\n"));
    assert!(is_text_buffer(b"")); // 空文件允许编辑
    assert!(is_text_buffer("中文内容测试\nsecond line\r\n\ttab".as_bytes()));
  }

  #[test]
  fn buffer_detects_binary() {
    assert!(!is_text_buffer(&[0x1F, 0x8B, 0x08, 0x00, 0x00])); // gzip 魔数含 \0
    assert!(!is_text_buffer(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]));
    // 高密度控制字符
    let mut noisy = vec![b'A'; 200];
    for byte in noisy.iter_mut().take(10) {
      *byte = 0x01;
    }
    assert!(!is_text_buffer(&noisy)); // 5% 控制字符 > 1.5%
  }

  #[test]
  fn editable_sftp_entry_respects_size_limit() {
    assert!(is_editable_sftp_entry("notes.txt", 1024));
    assert!(is_editable_sftp_entry("notes.txt", MAX_EDITABLE_FILE_SIZE_BYTES));
    // 超限的 .txt → false
    assert!(!is_editable_sftp_entry(
      "huge.txt",
      MAX_EDITABLE_FILE_SIZE_BYTES + 1
    ));
  }

  #[test]
  fn editable_sftp_entry_rejects_unknown_extension() {
    // 限内未知后缀 → false（列表阶段不嗅探，保守不可编辑）
    assert!(!is_editable_sftp_entry("runbook", 100));
    assert!(!is_editable_sftp_entry(".gitignore", 100));
    // 命中二进制黑名单同样不可编辑
    assert!(!is_editable_sftp_entry("logo.png", 100));
  }

  #[test]
  fn combined_entry() {
    assert!(!is_editable_text(Some("a.png"), &[0u8; 8])); // 黑名单优先，不读内容也能判
    assert!(is_editable_text(Some("data.xyz"), b"plain text"));
    assert!(!is_editable_text(Some("data.xyz"), &[0x00, 0x01, 0x02]));
    assert!(is_editable_text(None, b"nameless text"));
  }
}
