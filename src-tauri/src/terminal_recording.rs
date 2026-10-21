//! 终端会话录制（Asciinema v2 .cast）的文件导出。
//!
//! 与 command_library 的 `command_export_file` 同一惯例：rfd 保存对话框
//! （建议文件名 + 单扩展名 filter）+ 直接写文件，用户取消返回 `Ok(None)`。
//! 建议文件名由前端生成（含主机标签与时间戳），本命令只负责「选位置 + 落盘」，
//! 同时服务 .cast 与剥除 ANSI 的纯文本 .log 两种导出。

use std::fs;

/// 导出文本文件：rfd 保存对话框 + 写盘。返回落盘路径；用户取消返回 None。
#[tauri::command]
pub async fn export_text_file(
    suggested_name: String,
    ext: String,
    filter_label: String,
    content: String,
) -> Result<Option<String>, String> {
    let picked = rfd::AsyncFileDialog::new()
        .set_file_name(&suggested_name)
        .add_filter(&filter_label, &[&ext])
        .save_file()
        .await;
    let Some(handle) = picked else {
        return Ok(None);
    };
    let path = handle.path().to_path_buf();
    let result_path = path.to_string_lossy().to_string();
    let bytes = content.into_bytes();
    tokio::task::spawn_blocking(move || fs::write(&path, bytes))
        .await
        .map_err(|e| format!("导出任务失败: {e}"))?
        .map_err(|e| format!("写入文件失败: {e}"))?;
    Ok(Some(result_path))
}
