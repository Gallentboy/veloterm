//! GIF → MP4 转码的 tauri 命令层（libgif2mp4 静态链入，见 vendor/gif2mp4-sys）。
//!
//! 设计要点：
//! - C 侧 `gif2mp4_run` 是阻塞单飞调用——经 `tauri::async_runtime::spawn_blocking`
//!   放工作线程，命令本身 await 不堵异步运行时；并发第二路会收到 C 侧 -7
//!   （Busy），映射为友好错误串；
//! - 进度经 `tauri::ipc::Channel` 推送（sftp 传输同惯例），事件为 serde
//!   camelCase 纯数据；
//! - 取消：`gif2mp4_cancel` 置当前任务的 AtomicBool，进度回调里查到即对
//!   C 侧返回「取消」，得到 Cancelled 结果（现场已清理、无半截产物——
//!   原子性由 C 侧临时文件 + rename 保证）；
//! - 日志：首次调用即安装 C→log crate 桥（gif2mp4_sys::install_log_bridge，
//!   幂等），ffmpeg/x264 内部日志进 tauri-plugin-log 管道。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

use serde::Serialize;
use tauri::ipc::Channel;

use gif2mp4_sys::{self as sys, Code, ConvertParams};

/// 进度事件（前端 camelCase）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gif2Mp4ProgressEvent {
    pub decoded_frames: i64,
    pub total_frames: i64,
    /// 0-100；total 未知时为 -1（前端显示不确定态进度条）
    pub percent: f64,
    pub out_time_sec: f64,
    pub duration_sec: f64,
}

/// 转码结局
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gif2Mp4Outcome {
    /// "done" | "cancelled"
    pub status: &'static str,
}

/// 当前任务的取消标志（C 侧进程级单飞 ⇒ 同时至多一个活动任务）
static CANCEL_FLAG: OnceLock<AtomicBool> = OnceLock::new();
/// 任务占用位（区分「无任务」与「任务未请求取消」，供 cancel 返回是否生效）
static JOB_ACTIVE: OnceLock<AtomicBool> = OnceLock::new();

fn cancel_flag() -> &'static AtomicBool {
    CANCEL_FLAG.get_or_init(|| AtomicBool::new(false))
}

fn job_active() -> &'static AtomicBool {
    JOB_ACTIVE.get_or_init(|| AtomicBool::new(false))
}

/// GIF → MP4 转码（阻塞至完成/取消；进度经 on_progress 通道推送）。
/// 参数语义同 C 侧：fps ≤0/None 跟随源时基；crf None 取 23（0-51）；
/// preset None 取 medium（白名单十档）。
#[tauri::command]
pub async fn gif2mp4_convert(
    input_path: String,
    output_path: String,
    fps: Option<i32>,
    crf: Option<i32>,
    preset: Option<String>,
    on_progress: Channel<Gif2Mp4ProgressEvent>,
) -> Result<Gif2Mp4Outcome, String> {
    sys::install_log_bridge();

    if job_active().swap(true, Ordering::SeqCst) {
        return Err("已有转码任务在进行中".to_string());
    }
    cancel_flag().store(false, Ordering::SeqCst);

    let params = ConvertParams { fps, crf, preset };
    // &'static AtomicBool 直接进闭包（无需 Arc）
    let cancel = cancel_flag();

    let result = tauri::async_runtime::spawn_blocking(move || {
        sys::convert(&input_path, &output_path, &params, |p| {
            let _ = on_progress.send(Gif2Mp4ProgressEvent {
                decoded_frames: p.decoded_frames,
                total_frames: p.total_frames,
                percent: p.percent,
                out_time_sec: p.out_time_sec,
                duration_sec: p.duration_sec,
            });
            // 已请求取消 → 回 false 让 C 侧走清理路径
            !cancel.load(Ordering::Relaxed)
        })
    })
    .await
    .map_err(|e| format!("转码工作线程异常: {e}"))?;

    job_active().store(false, Ordering::SeqCst);
    match result {
        Ok(()) => Ok(Gif2Mp4Outcome { status: "done" }),
        Err(err) => match err.code {
            Code::Cancelled => Ok(Gif2Mp4Outcome { status: "cancelled" }),
            other => Err(format!("{other:?}: {}", err.message)),
        },
    }
}

/// 请求取消当前转码任务；返回是否有活动任务收到取消请求。
#[tauri::command]
pub fn gif2mp4_cancel() -> bool {
    if job_active().load(Ordering::SeqCst) {
        cancel_flag().store(true, Ordering::SeqCst);
        true
    } else {
        false
    }
}

// ─── 录制导出 MP4（cast → 临时 GIF → MP4 两阶段链） ─────────────────

/// MP4 导出进度（双阶段）：phase=gif 为 agg 渲染临时 GIF（0-100 整数步进），
/// phase=mp4 为 GIF→MP4 转码（percent 可为 -1 = 总帧数未知的不确定态）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mp4ExportProgress {
    pub phase: &'static str,
    pub percent: f64,
}

/// 导出录制的 MP4：rfd 选 .mp4 路径 → 复用 GIF 导出管线（agg）渲染到
/// `<out>.tmp.gif` → libgif2mp4 转码 → 删除临时件。取消/失败均清理。
/// 返回落盘路径；用户取消对话框返回 None。
#[tauri::command]
pub async fn export_recording_mp4(
    suggested_name: String,
    cast_content: String,
    theme: Option<String>,
    font_family: Option<String>,
    on_progress: Channel<Mp4ExportProgress>,
) -> Result<Option<String>, String> {
    sys::install_log_bridge();

    let picked = rfd::AsyncFileDialog::new()
        .set_file_name(&suggested_name)
        .add_filter("MP4 video", &["mp4"])
        .save_file()
        .await;
    let Some(handle) = picked else {
        return Ok(None);
    };
    let out_path = PathBuf::from(handle.path());
    let result_path = out_path.to_string_lossy().to_string();
    // 选完保存路径即报 0%，让前端离开「未开始」态（GIF 导出同款）
    let _ = on_progress.send(Mp4ExportProgress { phase: "gif", percent: 0.0 });

    // 阶段一进度：agg 逐帧回调 → 整数百分比节流（同值不重发）
    let gif_channel = on_progress.clone();
    let last_percent = Arc::new(AtomicU8::new(u8::MAX));
    let gif_progress = Box::new(move |done: u64, total: u64| {
        let percent = ((done * 100).checked_div(total).unwrap_or(100)).min(100) as f64;
        if last_percent.swap(percent as u8, Ordering::Relaxed) != percent as u8 {
            let _ = gif_channel.send(Mp4ExportProgress { phase: "gif", percent });
        }
    });

    // 单飞占用与取消标志：与 gif2mp4_convert 命令共用（同时至多一路转码）
    if job_active().swap(true, Ordering::SeqCst) {
        return Err("已有转码任务在进行中".to_string());
    }
    cancel_flag().store(false, Ordering::SeqCst);
    let cancel = cancel_flag();

    // 临时 GIF 落在输出同目录（同盘 rename，避免跨卷拷贝）：
    // out.mp4 → out.tmp.gif；无论成败最后都删
    let tmp_gif = out_path.with_extension("tmp.gif");
    let mp4_channel = on_progress.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        // 阶段一：cast → 临时 GIF（与 GIF 导出完全同源的 agg 管线）
        crate::terminal_recording::render_gif(
            &tmp_gif,
            &cast_content,
            theme,
            font_family,
            Some(gif_progress),
        )?;
        let _ = mp4_channel.send(Mp4ExportProgress { phase: "mp4", percent: 0.0 });

        // 阶段二：GIF → MP4；进度回调里响应取消标志
        let convert = sys::convert(
            tmp_gif.to_string_lossy().as_ref(),
            out_path.to_string_lossy().as_ref(),
            &ConvertParams::default(),
            |p| {
                let _ = mp4_channel.send(Mp4ExportProgress {
                    phase: "mp4",
                    percent: p.percent,
                });
                !cancel.load(Ordering::Relaxed)
            },
        );
        // 转码完成（或失败/取消）：清理临时 GIF
        let _ = std::fs::remove_file(&tmp_gif);
        convert.map_err(|e| format!("MP4 转码失败: {e}"))
    })
    .await
    .map_err(|e| format!("导出任务失败: {e}"))?;

    job_active().store(false, Ordering::SeqCst);
    match result {
        Ok(()) => Ok(Some(result_path)),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_missing_input_maps_to_input_error() {
        // 端到端 FFI 冒烟：路径不存在应得到 C 侧 -2（Input）而非崩溃；
        // 同时验证静态链接与进度回调通道可正常进入 C 侧
        let err = sys::convert("/nonexistent/definitely_missing.gif", "/tmp/x.mp4", &ConvertParams::default(), |_| true)
            .expect_err("应失败");
        assert_eq!(err.code, Code::Input);
    }

    #[test]
    fn log_bridge_is_idempotent() {
        sys::install_log_bridge();
        sys::install_log_bridge();
    }
}
