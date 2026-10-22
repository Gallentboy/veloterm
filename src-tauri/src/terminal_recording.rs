//! 终端会话录制（Asciinema v2 .cast）的文件导出。
//!
//! 与 command_library 的 `command_export_file` 同一惯例：rfd 保存对话框
//! （建议文件名 + 单扩展名 filter）+ 直接写文件，用户取消返回 `Ok(None)`。
//! 建议文件名由前端生成（含主机标签与时间戳），本命令只负责「选位置 + 落盘」，
//! 同时服务 .cast 与剥除 ANSI 的纯文本 .log 两种导出。
//!
//! GIF 导出（`export_recording_gif`）走 vendor agg 的 lib 目标（asciinema
//! 官方渲染管线：avt 虚拟终端回放 + swash 字形栅格化 + gifski 编码），
//! 在阻塞线程池渲染（秒级耗时，不占 async 运行时）；主题串与字体回退链由
//! 前端传入（与终端/回放器观感对齐），行高/跳空闲阈值也对齐 xterm 回放端。

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use tauri::ipc::Channel;

/// GIF 渲染进度（0-100，整数百分比，仅变化时上报）
#[derive(Clone, serde::Serialize)]
pub struct GifProgress {
    pub percent: u8,
}

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

/// 导出 GIF：rfd 保存对话框 + agg 渲染。返回落盘路径；用户取消返回 None。
///
/// - `cast_content`：.cast 的 JSON Lines 全文（与导出 .cast 同源）；
/// - `theme`：18 个十六进制三元组逗号串（bg,fg,c0..c15），None 用 agg 内置
///   Dracula；格式错误由 agg 的解析报错透传；
/// - `font_family`：逗号分隔的字体回退链（可含 CJK 系统字体名），None 用
///   agg 跨平台默认链；
/// - `on_progress`：进度 Channel（选完保存路径即报 0%，随后逐帧推进至 100%）。
#[tauri::command]
pub async fn export_recording_gif(
    suggested_name: String,
    cast_content: String,
    theme: Option<String>,
    font_family: Option<String>,
    on_progress: Channel<GifProgress>,
) -> Result<Option<String>, String> {
    let picked = rfd::AsyncFileDialog::new()
        .set_file_name(&suggested_name)
        .add_filter("GIF image", &["gif"])
        .save_file()
        .await;
    let Some(handle) = picked else {
        return Ok(None);
    };
    let path = handle.path().to_path_buf();
    let result_path = path.to_string_lossy().to_string();
    // 字体扫描/建帧阶段无帧可报，先报 0% 让前端离开「未开始」态
    let _ = on_progress.send(GifProgress { percent: 0 });
    // 逐帧回调 → 整数百分比节流（同值不重发，最多 101 条消息）
    let channel = on_progress.clone();
    let last_percent = Arc::new(AtomicU8::new(u8::MAX));
    let progress = Box::new(move |done: u64, total: u64| {
        let percent = if total == 0 {
            100
        } else {
            ((done * 100) / total).min(100) as u8
        };
        if last_percent.swap(percent, Ordering::Relaxed) != percent {
            let _ = channel.send(GifProgress { percent });
        }
    });
    tokio::task::spawn_blocking(move || {
        render_gif(&path, &cast_content, theme, font_family, Some(progress))
    })
    .await
    .map_err(|e| format!("导出任务失败: {e}"))??;
    Ok(Some(result_path))
}

/// agg 渲染（阻塞线程池内执行）。Config 对齐回放器观感：
/// 行高 1.0（xterm 默认，agg 缺省 1.4 偏松）、空闲超 2s 折叠（回放端同阈值）。
/// v1.9.0 的 swash 文字渲染恒为全档抗锯齿（无档位旋钮），量化质量由树内
/// 补丁（gifski fast: false）保证。
/// pub(crate)：MP4 导出（gif2mp4.rs）复用本管线先渲染临时 GIF 再转码
pub(crate) fn render_gif(
    path: &Path,
    cast_content: &str,
    theme: Option<String>,
    font_family: Option<String>,
    progress: Option<Box<dyn Fn(u64, u64) + Send>>,
) -> Result<(), String> {
    let mut config = agg::Config {
        show_progress_bar: false,
        ..Default::default()
    };
    config.line_height = 1.0;
    config.idle_time_limit = Some(2.0);
    // xterm.js 的 drawBoldTextInBrightColors 默认 true：粗体用亮色系渲染。
    // agg 默认 false，粗体文字（如 1;31m）会比终端里暗一档——观感发昏的真凶
    config.bold_is_bright = true;
    if let Some(families) = font_family {
        config.text_font_family = families;
    }
    if let Some(triplets) = theme {
        config.theme = Some(agg::Theme::Custom(triplets));
    }
    config.progress = progress;
    let reader = io::Cursor::new(cast_content.as_bytes());
    let output = fs::File::create(path).map_err(|e| format!("创建文件失败: {e}"))?;
    agg::run(reader, output, config).map_err(|e| format!("生成 GIF 失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 端到端冒烟：合成最小 .cast（含输出与 resize 事件）→ render_gif →
    /// 产物存在且为 GIF 魔数（GIF89a/GIF87a）。字体走系统扫描 + 树内
    /// NotoEmoji/SymbolsNerdFont，macOS 门禁机即可跑。
    #[test]
    fn render_gif_produces_gif_file() {
        // JSON 行内换行须为转义文本 \r\n（serializeCast 由 JSON.stringify 产出，
        // 天然合规；手写测试串须双反斜杠避免 Rust 层提前转义成控制字符）
        let cast = concat!(
            "{\"version\":2,\"width\":48,\"height\":4,\"timestamp\":0,",
            "\"env\":{\"TERM\":\"xterm-256color\"}}\n",
            "[0.0,\"o\",\"$ ssh deploy@prod01\\r\\n\"]\n",
            "[0.1,\"o\",\"\\u001b[32m✔\\u001b[0m build passed \\u001b[1;32m(12s)\\u001b[0m\\r\\n\"]\n",
            "[0.2,\"o\",\"\\u001b[31m✘\\u001b[0m \\u001b[1;31mERROR\\u001b[0m 部署失败：连接超时\\r\\n\"]\n",
            "[0.4,\"o\",\"\\u001b[33mwarn\\u001b[0m: 3 warnings, 你好世界 hello world\\r\\n\"]\n",
            "[1.0,\"r\",\"64x4\"]\n",
            "[1.2,\"o\",\"$ done\"]\n",
        );
        let dir = std::env::temp_dir().join(format!("veloterm-gif-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.gif");
        render_gif(&path, cast, None, None, None).expect("agg 渲染应成功");
        let bytes = fs::read(&path).expect("GIF 文件应已写出");
        assert!(bytes.starts_with(b"GIF8"), "产物应为 GIF 魔数，实际头: {:?}", bytes.get(..4));
        assert!(bytes.len() > 100, "GIF 应有实际内容");

        // 主题串（18 三元组）与字体回退链两参通路同测（前端随命令传入的形态）
        let path2 = dir.join("out2.gif");
        render_gif(
            &path2,
            cast,
            Some("161616,bdbdbd,363636,ff5c5c,46bf72,ff8a30,4099ff,7b5ce5,42c8c8,adadad,747474,ff9999,87d9a4,ffb26b,80beff,a888f2,8ee5e5,f8f8f8".to_string()),
            Some("Menlo, PingFang SC".to_string()),
            None,
        )
        .expect("带主题与字体的渲染应成功");
        assert!(fs::read(&path2).unwrap().starts_with(b"GIF8"));

        // 进度回调通路：单调不减、末值 100、总帧数非零（渲染前先建帧）。
        // 回调要求 'static，故收集器经 Arc 克隆 move 进闭包
        let seen = Arc::new(std::sync::Mutex::new(Vec::<(u64, u64)>::new()));
        let sink = Arc::clone(&seen);
        let rec = move |done: u64, total: u64| sink.lock().unwrap().push((done, total));
        let path3 = dir.join("out3.gif");
        render_gif(&path3, cast, None, None, Some(Box::new(rec))).expect("带进度的渲染应成功");
        let seen = seen.lock().unwrap();
        let first = seen.first().expect("应至少回调一次");
        assert!(first.0 == 0 && first.1 > 0, "首次回调应为 0/N，实际 {first:?}");
        let total = first.1;
        assert_eq!(seen.last().unwrap(), &(total, total), "末次应到 N/N");
        // windows(2) 长度恒为 2，用切片模式匹配代替索引（仓库 deny indexing_slicing）
        assert!(
            seen.windows(2).all(|w| matches!(w, [a, b] if a.0 < b.0)),
            "进度应严格递增"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
