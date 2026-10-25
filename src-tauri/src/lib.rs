pub mod about;
pub mod ai;
pub mod ai_agent;
pub mod bash_analysis;
pub mod cert;
pub mod gif2mp4;
pub mod command_library;
pub mod commands;
pub mod common;
pub mod db;
pub mod db_browser;
pub mod editor_storage;
pub mod excel;
pub mod grpc_client;
pub mod grpc_describe;
pub mod grpc_tls;
pub mod host_password_history;
pub mod lifecycle;
pub mod logging;
pub mod pdf;
pub mod port_forward;
pub mod relays;
pub mod safety_guard;
pub mod sftp;
pub mod sqlite_viewer;
pub mod terminal;
pub mod terminal_recording;
pub mod text_detect;
pub mod tools;
pub mod window_state;
pub mod zk;
pub mod zk_hosts;

#[tauri::command]
fn is_apple_silicon() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // rustls 进程级 CryptoProvider 显式钉 aws-lc-rs：依赖图里 reqwest 0.12
    // （__rustls-ring）与 quinn/tonic/mysql_async（aws-lc-rs）并存，rustls 的
    // crate-feature 自动探测在双 provider 下返回空——首次走进程默认 provider
    // 的构造（PG/MySQL TLS 的 ClientConfig::builder()）会运行期 panic。
    // 启动时显式安装消除歧义；已装时 install 返回 Err，忽略即幂等。
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let app = tauri::Builder::default()
        .plugin(logging::build_log_plugin())
        // 原生剪贴板插件：终端 Cmd+C/V 经由 Rust 侧读写系统剪贴板，
        // 绕开 WKWebView navigator.clipboard 的粘贴授权弹层（原生「粘贴」按钮）
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(terminal::manager::TerminalManager::new())
        // 联动组内 ZMODEM 请求聚合（一次交互，N 路扇出）；Arc 形态与
        // TerminalManager 的 try_state 取值类型一致
        .manage(std::sync::Arc::new(
            terminal::zmodem::ZmodemAggregator::new(),
        ))
        .manage(port_forward::PortForwardManager::default())
        .manage(sftp::types::SftpState::new())
        .manage(zk::ZkState::new())
        .manage(db_browser::DbState::new())
        .invoke_handler(tauri::generate_handler![
            is_apple_silicon,
            about::show_about_window,
            terminal_recording::export_text_file,
            terminal_recording::export_recording_gif,
            logging::set_log_level,
            logging::get_current_log_level,
            logging::get_log_file_path,
            commands::command_history::command_history_list,
            commands::command_history::command_history_clear,
            commands::command_history::command_history_get_enabled,
            commands::command_history::command_history_set_enabled,
            commands::session::list_sessions,
            commands::session::save_session,
            commands::session::delete_session,
            commands::session::db_bootstrap_state,
            commands::session::db_get_all_kv,
            commands::session::db_get_kv,
            commands::session::db_set_kv,
            commands::session::db_delete_kv,
            commands::session::db_load_host_store,
            commands::session::db_save_host_store,
            terminal::commands::terminal_create,
            terminal::commands::terminal_set_broadcast_group,
            terminal::commands::terminal_cancel_zmodem,
            terminal::commands::terminal_write,
            terminal::commands::terminal_resize,
            terminal::commands::terminal_dispose,
            terminal::commands::ssh_list_known_hosts,
            terminal::commands::ssh_remove_known_host,
            terminal::commands::ssh_clear_known_hosts,
            terminal::commands::ssh_resource_snapshot,
            port_forward::port_forward_list,
            port_forward::port_forward_save,
            port_forward::port_forward_delete,
            port_forward::port_forward_set_enabled,
            port_forward::port_forward_states,
            zk::zk_connect,
            zk::zk_disconnect,
            db_browser::db_connection_list,
            db_browser::db_connection_save,
            db_browser::db_connection_delete,
            db_browser::db_connection_test,
            db_browser::db_connect,
            db_browser::db_disconnect,
            db_browser::db_catalog,
            db_browser::db_sql_completions,
            db_browser::db_sql_completions_invalidate,
            db_browser::db_format_sql,
            db_browser::db_query,
            db_browser::db_query_cancel,
            db_browser::db_explain,
            db_browser::db_list_databases,
            db_browser::db_set_auto_commit,
            db_browser::db_get_auto_commit,
            db_browser::db_get_tx_status,
            db_browser::db_commit,
            db_browser::db_rollback,
            db_browser::db_use_database,
            db_browser::db_list_databases_for_record,
            db_browser::db_object_definition,
            db_browser::db_table_detail,
            db_browser::redis_read_key,
            db_browser::redis_write_key,
            db_browser::redis_slowlog,
            db_browser::redis_bigkeys,
            db_browser::db_odbc_drivers,
            db_browser::db_odbc_data_sources,
            db_browser::sqlite_file_pick,
            zk::zk_list,
            zk::zk_get,
            zk::zk_set_data,
            zk::zk_create,
            zk::zk_delete,
            zk::zk_set_acl,
            zk::zk_digest_id,
            zk::zk_auth,
            relays::relay_list,
            relays::relay_save,
            relays::relay_delete,
            relays::relay_test,
            host_password_history::host_password_history_list,
            host_password_history::host_password_history_clear,
            pdf::render_pdf_page,
	            pdf::pdf_temp_path,
	            pdf::prepare_image_preview,
	            pdf::release_pdf_temp_files,
		            excel::read_excel_workbook,
		            excel::read_excel_sheet_data,
            cert::parse_certificate,
            sqlite_viewer::read_sqlite_metadata,
            sqlite_viewer::query_sqlite_data,
            bash_analysis::analyze_bash_document,
            gif2mp4::gif2mp4_convert,
            gif2mp4::gif2mp4_cancel,
            gif2mp4::export_recording_mp4,
            safety_guard::inspect_command_safety,
            zk_hosts::zk_host_list,
            zk_hosts::zk_host_save,
            zk_hosts::zk_host_delete,
            grpc_client::grpc_call,
            grpc_client::grpc_list,
            grpc_client::grpc_describe,
            grpc_client::grpc_method_schema,
            grpc_tls::grpc_tls_list,
            grpc_tls::grpc_tls_save,
            grpc_tls::grpc_tls_delete,
            ai::ai_provider_list,
            ai::ai_provider_save,
            ai::ai_provider_delete,
            ai::ai_chat,
            ai::ai_provider_test,
            ai::ai_list_models,
            ai_agent::ai_agent_chat,
            ai_agent::ai_tool_result,
            command_library::command_list,
            command_library::command_save,
            command_library::command_delete,
            command_library::command_used,
            command_library::command_export,
            command_library::command_import,
            command_library::command_export_file,
            command_library::command_import_start,
            command_library::command_import_apply,
            sftp::commands::sftp_next_task_id,
            sftp::commands::sftp_cancel_transfer,
            sftp::commands::sftp_connect,
            sftp::commands::sftp_close,
            sftp::commands::sftp_list_dir,
            sftp::commands::sftp_download_file,
            sftp::commands::sftp_upload_file,
            sftp::commands::sftp_pick_upload_files,
            sftp::commands::sftp_pick_download_path,
            sftp::commands::sftp_pick_download_dir,
            sftp::commands::sftp_remove,
            sftp::commands::sftp_scan_tree,
            sftp::commands::sftp_remove_tree,
            sftp::commands::sftp_preview_fast_remove,
            sftp::commands::sftp_remove_fast,
            sftp::commands::sftp_dir_size,
            sftp::commands::sftp_chmod,
            sftp::commands::sftp_chown,
            sftp::commands::sftp_list_users_groups,
            sftp::commands::sftp_exec_available,
            sftp::commands::sftp_bash_remote_commands,
            sftp::commands::sftp_copy_file,
		            sftp::file_ops::sftp_read_file,
		            sftp::file_ops::sftp_read_file_hex,
		            sftp::file_ops::sftp_read_file_tail,
		            sftp::file_ops::sftp_read_file_range,
		            sftp::file_ops::sftp_sudo_read,
            sftp::file_ops::sftp_sudo_write,
            sftp::file_ops::sftp_write_file,
            editor_storage::editor_load_state,
	            editor_storage::editor_save_panel_state,
	            editor_storage::editor_save_draft,
	            editor_storage::editor_delete_draft,
	            tools::jq::execute_jq_filter,
	        ])
        .on_menu_event(|app, event| {
            use tauri::Emitter;
            let id = event.id().as_ref();
            if id == "veloterm-about" {
                about::show_about(app);
            } else if id == "veloterm-manual" {
                let _ = app.emit("open-manual", ());
            }
        })
        .on_window_event(|window, event| {
            if !window_state::is_main_window(window.label()) {
                return;
            }
            match event {
                // Resized / Moved 随拖拽高频触发，observe 内部按 800ms 节流落库（缓存每次都会更新）
                tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Moved(_) => {
                    window_state::observe(window)
                }
                // 关闭时实时读窗口几何落库，保证「刚拖完就退出」也不会丢
                tauri::WindowEvent::CloseRequested { .. } => window_state::persist(window),
                _ => {}
            }
        })
        .setup(|app| {
            use tauri::Manager;

            let master_key = std::env::var("VELOTERM_DB_KEY")
                .ok()
                .filter(|k| !k.trim().is_empty())
                .unwrap_or_else(|| db::DEFAULT_MASTER_KEY.to_string());
            let db_pool = db::init_db(app.handle(), &master_key)?;

            // Windows：启动时幂等确保 tools/ 下 coreutils 硬链接短名入口就绪
            // （ls.exe/grep.exe/… → 同级目录 coreutils.exe；缺失创建、存在跳过）
            #[cfg(target_os = "windows")]
            tools::hardlinks::ensure_installed_tools();

            // 从 SQLCipher 数据库恢复已保存的日志等级阈值
            if let Ok(Some(saved_level)) = db::get_kv(&db_pool, "veloterm:settings:log-level") {
                let _ = logging::set_log_level(saved_level);
            } else {
                logging::apply_log_level_filter(logging::current_level_filter());
            }

            app.manage(db_pool);

            // 注入 AppHandle 到 SftpState，供 SFTP 生命周期事件发射
            if let Some(sftp_state) = app.try_state::<sftp::types::SftpState>() {
                sftp_state.set_app(app.handle().clone());
            }

            // 注入 AppHandle 到 ZkState，供 zk-watch 事件发射
            if let Some(zk_state) = app.try_state::<zk::ZkState>() {
                zk_state.set_app(app.handle().clone());
            }

            // 恢复上次关闭时的窗口大小与位置。配置里的窗口在本函数（Tauri 内部 setup）中先于用户
            // setup 闭包创建，且两者处于同一个事件循环迭代内，因此这里改几何发生在首帧绘制之前，
            // 不会跳变。注：`Manager::get_window` 需要 `unstable` feature，故经 `WebviewWindow` 取底层 `Window`。
            if let Some(webview_window) = app.get_webview_window("main") {
                let webview: &tauri::Webview<_> = webview_window.as_ref();
                window_state::restore(&webview.window());

                // 关于窗 modal 语义的焦点弹回：主窗被点击激活时（遮罩挡不住
                // 窗口级激活），若关于窗还开着就把焦点弹回去——主窗在关于窗
                // 存在期间不可交互；配合关于窗 always_on_top，主窗也盖不住它
                let bounce_handle = app.handle().clone();
                webview_window.on_window_event(move |event| {
                    if matches!(event, tauri::WindowEvent::Focused(true))
                        && let Some(about) = bounce_handle.get_webview_window(about::ABOUT_WINDOW_LABEL)
                    {
                        let _ = about.set_focus();
                    }
                });
            }

            #[cfg(target_os = "macos")]
            about::install_macos_menu(app.handle());

            log::info!("[VeloTerm] application initialized");

            // macOS 毛玻璃底座（对应 ZCode Electron 的 vibrancy: under-window）：
            // 窗口需 transparent + macOSPrivateApi，前端窗口底座 bg-background-alt 半透明叠在其上。
            #[cfg(target_os = "macos")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    window_vibrancy::apply_vibrancy(
                        &window,
                        window_vibrancy::NSVisualEffectMaterial::UnderWindowBackground,
                        None,
                        None,
                    )?;
                }
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        // 兜底落库：macOS ⌘Q / 应用菜单退出等路径不经过窗口 CloseRequested
        if let tauri::RunEvent::ExitRequested { .. } = event {
            window_state::persist_from_handle(app_handle);
            // PDF 预览的临时文件（下载的源文件 + 渲染的 PNG）兜底清理：
            // 前端正常关闭预览时会主动调 release_pdf_temp_files，
            // 这里覆盖崩溃/强退等未走前端的路径
            pdf::cleanup_all_temp_files();
        }
    });
}
