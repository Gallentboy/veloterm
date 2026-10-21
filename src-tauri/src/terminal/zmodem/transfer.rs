//! ZMODEM 传输编排：探测命中后启动下载（sz）或上传（rz）任务。
//!
//! 支持**联动批量**：同组多台终端同时命中时，只弹一次对话框，结果扇出到
//! 每台各自的引擎（聚合见 `aggregator.rs`）。
//!
//! - 下载（sz）：弹一次保存目录 → 各会话 `receive_files` 落到该目录
//!   （同名文件由 `ReceiverConfig.rename` 自动加 `.1/.2` 后缀）
//! - 上传（rz）：多选一次文件 → 各会话 `send_file` ×N + `finish_session`
//! - 全程 spawn_blocking 跑引擎，异步侧只做通道搬运与事件发射
//! - 取消：`ZmodemBridge::cancel()`（引擎在检查点返回 `ZError::Cancelled`）

use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use super::aggregator::PendingHit;
use super::bridge::{BridgeReader, BridgeWriter, ZmodemProgress};

/// 前端事件名（终端页监听后渲染 toast）
pub const ZMODEM_EVENT: &str = "zmodem-progress";

fn emit(app: &AppHandle, progress: ZmodemProgress) {
    let _ = app.emit(ZMODEM_EVENT, progress);
}

fn emit_start(
    app: &AppHandle,
    session_id: &str,
    direction: &'static str,
    file_name: String,
    total: u64,
) {
    emit(
        app,
        ZmodemProgress {
            session_id: session_id.to_string(),
            direction,
            file_name,
            transferred: 0,
            total,
            speed_bps: 0,
            elapsed_ms: 0,
            is_finished: false,
            error: None,
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_finish(
    app: &AppHandle,
    session_id: &str,
    direction: &'static str,
    file_name: String,
    transferred: u64,
    total: u64,
    speed_bps: u64,
    elapsed_ms: u64,
    error: Option<String>,
) {
    emit(
        app,
        ZmodemProgress {
            session_id: session_id.to_string(),
            direction,
            file_name,
            transferred,
            total,
            speed_bps,
            elapsed_ms,
            is_finished: true,
            error,
        },
    );
}

/// 下载（sz）批量入口：一次选目录，N 台各自落盘。
pub async fn start_download_batch(app: AppHandle, hits: Vec<PendingHit>) {
    if hits.is_empty() {
        return;
    }
    // 对话框阶段即锁输入（每台各发一个开始事件；取消时全部发结束事件）
    for hit in &hits {
        emit_start(&app, &hit.bridge.session_id, "download", String::new(), 0);
    }
    // 对话框必须在阻塞线程跑：本函数处于 tokio::spawn 的异步任务中，
    // AsyncFileDialog 在非主线程事件循环下会挂起（实测卡死）。
    let dir = tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("选择下载保存目录")
            .pick_folder()
    })
    .await
    .unwrap_or(None);
    let Some(dir) = dir else {
        for hit in &hits {
            // 先给服务器发中止序列（CAN×10+BS×10），否则远端 lrz 会继续等、
            // 终端卡在接收态；再取消并立即结束（解锁输入、恢复透传）
            hit.bridge.send_abort(&hit.outbound_tx);
            hit.bridge.cancel();
            emit_finish(&app, &hit.bridge.session_id, "download", String::new(), 0, 0, 0, 0, Some("cancelled".into()));
        }
        // 给中止帧一点时间写回 SSH 通道再丢弃桥
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        return;
    };
    let output_dir = dir;

    let mut joins = Vec::with_capacity(hits.len());
    for hit in hits {
        let sid = hit.bridge.session_id.clone();
        let sid_for_task = sid.clone();
        let cancel = hit.bridge.cancel_flag();
        let finished = hit.bridge.finished_flag();
        let app_task = app.clone();
        let dir = output_dir.clone();
        // 已完成字节数（引擎线程写、主流程读，用于完成事件展示整体大小）
        let done_bytes = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let done_bytes_task = std::sync::Arc::clone(&done_bytes);
        let done_bytes_out = std::sync::Arc::clone(&done_bytes);
        // 引擎耗时（毫秒）：完成行算平均速度用
        let elapsed_ms = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let elapsed_ms_task = std::sync::Arc::clone(&elapsed_ms);
        let elapsed_ms_out = std::sync::Arc::clone(&elapsed_ms);
        // 引擎存活标志：线程一退出立即置 false，宿主随即回退透传
        let engine_alive = hit.bridge.engine_alive_flag();
        // 注册到活跃表：前端「取消」按钮经 session id 找到本传输
        let cancel_flag_for_btn = Arc::clone(&cancel);
        let out_tx_for_abort = hit.outbound_tx.clone();
        let numeric_id = sid
            .strip_prefix("terminal-")
            .and_then(|n| n.parse::<usize>().ok());
        if let Some(nid) = numeric_id {
            let mut enc = rzsz::zmodem::frame::FrameEncoder::new();
            let mut abort_bytes = Vec::new();
            let _ = enc.send_cancel(&mut abort_bytes);
            super::register_active(
                nid,
                Box::new(move || {
                    cancel_flag_for_btn.store(true, std::sync::atomic::Ordering::Relaxed);
                    let _ = out_tx_for_abort.send(abort_bytes.clone());
                }),
            );
        }
        let join = tokio::task::spawn_blocking(move || {
            let started = std::time::Instant::now();
            let reader = BridgeReader::new(hit.inbound_rx);
            let mut reader = rzsz::serial::reader::ModemReader::new(reader, 8192);
            let mut writer = BridgeWriter::new(hit.outbound_tx);
            let mut session = rzsz::zmodem::session::Session::new();
            let progress_sid = sid_for_task.clone();
            let progress_app = app_task.clone();
            // 当前文件名：接收端解析到 ZFILE 后写入，进度回调读取
            let name_slot = std::sync::Arc::clone(&session.current_file);
            let size_slot = std::sync::Arc::clone(&session.current_file_size);
            let size_slot_final = std::sync::Arc::clone(&size_slot);
            // 速度差分：上次字节数与时刻（节流 200ms，避免事件过密）
            let mut last_bytes = 0u64;
            let mut last_at = std::time::Instant::now();
            session.on_progress = Some(Box::new(move |bytes: u64| {
                let now = std::time::Instant::now();
                let elapsed = now.duration_since(last_at).as_secs_f64();
                if elapsed < 0.2 {
                    return;
                }
                let speed = ((bytes.saturating_sub(last_bytes)) as f64 / elapsed) as u64;
                last_bytes = bytes;
                last_at = now;
                let current_name = name_slot
                    .lock()
                    .ok()
                    .and_then(|g| g.clone())
                    .unwrap_or_default();
                let _ = progress_app.emit(
                    ZMODEM_EVENT,
                    ZmodemProgress {
                        session_id: progress_sid.clone(),
                        direction: "download",
                        file_name: current_name,
                        transferred: bytes,
                        total: size_slot.load(std::sync::atomic::Ordering::Relaxed),
                        speed_bps: speed,
                        elapsed_ms: 0,
                        is_finished: false,
                        error: None,
                    },
                );
            }));
            let cancel_flag = Arc::clone(&cancel);
            session.is_cancelled = Some(Box::new(move || {
                cancel_flag.load(std::sync::atomic::Ordering::Relaxed)
            }));
            // 联动批量落同一目录：同名自动加 .1/.2 后缀
            let config = rzsz::receiver::ReceiverConfig {
                output_dir: dir,
                rename: true,
                ..Default::default()
            };
            let result =
                rzsz::receiver::receive_files(&mut session, &mut reader, &mut writer, &config);
            // 记录最终已收字节（完成事件用）
            done_bytes_task.store(
                size_slot_final.load(std::sync::atomic::Ordering::Relaxed),
                std::sync::atomic::Ordering::Relaxed,
            );
            elapsed_ms_task.store(
                started.elapsed().as_millis() as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
            // 引擎不再读数据：此后到达的字节必须透传（含 shell 提示符）
            engine_alive.store(false, std::sync::atomic::Ordering::Relaxed);
            result
        });
        joins.push((sid, finished, join, done_bytes_out, elapsed_ms_out));
    }

    for (sid, finished, join, done_bytes, elapsed_ms) in joins {
        let result = join.await;
        finished.store(true, std::sync::atomic::Ordering::Relaxed);
        let (error, files) = match result {
            Ok(Ok(files)) => (None, files),
            Ok(Err(e)) => (Some(format!("{e:?}")), Vec::new()),
            Err(e) => (Some(format!("join error: {e}")), Vec::new()),
        };
        if let Some(nid) = sid.strip_prefix("terminal-").and_then(|n| n.parse::<usize>().ok()) {
            super::unregister_active(nid);
        }
        let bytes = done_bytes.load(std::sync::atomic::Ordering::Relaxed);
        // 平均速度：总字节 / 引擎实际耗时
        let elapsed_raw = elapsed_ms.load(std::sync::atomic::Ordering::Relaxed);
        let elapsed = elapsed_raw as f64 / 1000.0;
        let avg_speed = if elapsed > 0.0 { (bytes as f64 / elapsed) as u64 } else { 0 };
        emit_finish(&app, &sid, "download", files.join(", "), bytes, bytes, avg_speed, elapsed_raw, error);
    }
}

/// 上传（rz）批量入口：一次多选文件，N 台各自发送。
pub async fn start_upload_batch(app: AppHandle, hits: Vec<PendingHit>) {
    if hits.is_empty() {
        return;
    }
    for hit in &hits {
        emit_start(&app, &hit.bridge.session_id, "upload", String::new(), 0);
    }
    // 单选（pick_file 单数版）：原生对话框禁用多选，且只能选文件不能选目录
    let picked = tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("选择要上传的文件")
            .pick_file()
    })
    .await
    .unwrap_or(None);
    let Some(picked) = picked else {
        for hit in &hits {
            // 同下载：先发中止序列让远端 rz 退出，再取消
            hit.bridge.send_abort(&hit.outbound_tx);
            hit.bridge.cancel();
            emit_finish(&app, &hit.bridge.session_id, "upload", String::new(), 0, 0, 0, 0, Some("cancelled".into()));
        }
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        return;
    };
    let paths: Vec<std::path::PathBuf> = vec![picked];
    // 选完文件后补发带名称/大小的开始事件（前端首帧显示「开始上传 xxx」）
    let display_name = if paths.len() == 1 {
        paths
            .first()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        format!("{} 个文件", paths.len())
    };
    let display_total: u64 = paths
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok().map(|m| m.len()))
        .sum();
    for hit in &hits {
        emit_start(
            &app,
            &hit.bridge.session_id,
            "upload",
            display_name.clone(),
            display_total,
        );
    }

    // 完成行展示用：保留一份路径（paths 会被 move 进各引擎闭包）
    let paths_for_name = paths.clone();
    let mut joins = Vec::with_capacity(hits.len());
    for hit in hits {
        let sid = hit.bridge.session_id.clone();
        let sid_for_task = sid.clone();
        let cancel = hit.bridge.cancel_flag();
        let finished = hit.bridge.finished_flag();
        let app_task = app.clone();
        let paths = paths.clone();
        let elapsed_ms = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let elapsed_ms_task = std::sync::Arc::clone(&elapsed_ms);
        let elapsed_ms_out = std::sync::Arc::clone(&elapsed_ms);
        // 引擎存活标志：线程一退出立即置 false，宿主随即回退透传
        let engine_alive = hit.bridge.engine_alive_flag();
        // 进度事件也带文件名（用户看到「上传中… report.log」而不是空）
        let name_for_progress = display_name.clone();
        let cancel_flag_for_btn = Arc::clone(&cancel);
        let out_tx_for_abort = hit.outbound_tx.clone();
        let numeric_id = sid
            .strip_prefix("terminal-")
            .and_then(|n| n.parse::<usize>().ok());
        if let Some(nid) = numeric_id {
            let mut enc = rzsz::zmodem::frame::FrameEncoder::new();
            let mut abort_bytes = Vec::new();
            let _ = enc.send_cancel(&mut abort_bytes);
            super::register_active(
                nid,
                Box::new(move || {
                    cancel_flag_for_btn.store(true, std::sync::atomic::Ordering::Relaxed);
                    let _ = out_tx_for_abort.send(abort_bytes.clone());
                }),
            );
        }
        let join = tokio::task::spawn_blocking(move || {
            let started = std::time::Instant::now();
            let reader = BridgeReader::new(hit.inbound_rx);
            let mut reader = rzsz::serial::reader::ModemReader::new(reader, 8192);
            let mut writer = BridgeWriter::new(hit.outbound_tx);
            let total_bytes: u64 = paths
                .iter()
                .filter_map(|p| std::fs::metadata(p).ok().map(|m| m.len()))
                .sum();
            let mut session = rzsz::zmodem::session::Session::new();
            let progress_sid = sid_for_task.clone();
            let progress_app = app_task.clone();
            let mut last_bytes = 0u64;
            let mut last_at = std::time::Instant::now();
            session.on_progress = Some(Box::new(move |bytes: u64| {
                let now = std::time::Instant::now();
                let elapsed = now.duration_since(last_at).as_secs_f64();
                if elapsed < 0.2 {
                    return;
                }
                let speed = ((bytes.saturating_sub(last_bytes)) as f64 / elapsed) as u64;
                last_bytes = bytes;
                last_at = now;
                let _ = progress_app.emit(
                    ZMODEM_EVENT,
                    ZmodemProgress {
                        session_id: progress_sid.clone(),
                        direction: "upload",
                        file_name: name_for_progress.clone(),
                        transferred: bytes,
                        total: total_bytes,
                        speed_bps: speed,
                        elapsed_ms: 0,
                        is_finished: false,
                        error: None,
                    },
                );
            }));
            let cancel_flag = Arc::clone(&cancel);
            session.is_cancelled = Some(Box::new(move || {
                cancel_flag.load(std::sync::atomic::Ordering::Relaxed)
            }));

            let config = rzsz::sender::SenderConfig::default();
            let mut sent = 0usize;
            let total = paths.len();
            for path in &paths {
                let files_left = total - sent - 1;
                match rzsz::sender::send_file(
                    &mut session,
                    &mut reader,
                    &mut writer,
                    path,
                    &config,
                    files_left,
                    total_bytes,
                    None,
                ) {
                    Ok(_) => sent += 1,
                    Err(e) => return Err(format!("{e:?}")),
                }
            }
            let finish = rzsz::sender::finish_session(&mut session, &mut reader, &mut writer);
            elapsed_ms_task.store(
                started.elapsed().as_millis() as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
            engine_alive.store(false, std::sync::atomic::Ordering::Relaxed);
            finish.map_err(|e| format!("{e:?}"))?;
            Ok(sent)
        });
        joins.push((sid, finished, join, elapsed_ms_out));
    }

    for (sid, finished, join, elapsed_ms) in joins {
        let result = join.await;
        finished.store(true, std::sync::atomic::Ordering::Relaxed);
        let (error, count) = match result {
            Ok(Ok(n)) => (None, n),
            Ok(Err(e)) => (Some(e), 0),
            Err(e) => (Some(format!("join error: {e}")), 0),
        };
        if let Some(nid) = sid.strip_prefix("terminal-").and_then(|n| n.parse::<usize>().ok()) {
            super::unregister_active(nid);
        }
        // 完成行展示真实文件名（多文件时显示「首个 等N个」）
        let done_name = if error.is_none() && paths_for_name.len() > 1 {
            format!(
                "{} 等 {} 个文件",
                paths_for_name
                    .first()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                paths_for_name.len()
            )
        } else {
            display_name.clone()
        };
        let _ = count;
        let elapsed_raw = elapsed_ms.load(std::sync::atomic::Ordering::Relaxed);
        let elapsed = elapsed_raw as f64 / 1000.0;
        let avg_speed = if elapsed > 0.0 {
            (display_total as f64 / elapsed) as u64
        } else {
            0
        };
        emit_finish(&app, &sid, "upload", done_name, display_total, display_total, avg_speed, elapsed_raw, error);
    }
}

/// 按批方向分发
pub async fn start_batch(app: AppHandle, direction: &'static str, hits: Vec<PendingHit>) {
    if direction == "download" {
        start_download_batch(app, hits).await;
    } else {
        start_upload_batch(app, hits).await;
    }
}
