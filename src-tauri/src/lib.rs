pub mod ai;
pub mod command_library;
pub mod ai_agent;
pub mod commands;
pub mod common;
pub mod db;
pub mod editor_storage;
pub mod lifecycle;
pub mod logging;
pub mod sftp;
pub mod terminal;
pub mod text_detect;
pub mod window_state;

#[tauri::command]
fn is_apple_silicon() -> bool {
  cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  let app = tauri::Builder::default()
    .plugin(logging::build_log_plugin())
    // 原生剪贴板插件：终端 Cmd+C/V 经由 Rust 侧读写系统剪贴板，
    // 绕开 WKWebView navigator.clipboard 的粘贴授权弹层（原生「粘贴」按钮）
    .plugin(tauri_plugin_clipboard_manager::init())
    .manage(terminal::manager::TerminalManager::new())
    .manage(sftp::types::SftpState::new())
    .invoke_handler(tauri::generate_handler![
      is_apple_silicon,
      logging::set_log_level,
      logging::get_current_log_level,
      logging::get_log_file_path,
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
      terminal::commands::terminal_write,
      terminal::commands::terminal_resize,
      terminal::commands::terminal_dispose,
      terminal::commands::ssh_list_known_hosts,
      terminal::commands::ssh_remove_known_host,
      terminal::commands::ssh_clear_known_hosts,
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
      sftp::commands::sftp_next_task_id,
      sftp::commands::sftp_cancel_transfer,
      sftp::commands::sftp_connect,
      sftp::commands::sftp_close,
      sftp::commands::sftp_list_dir,
      sftp::commands::sftp_download_file,
      sftp::commands::sftp_upload_file,
      sftp::commands::sftp_pick_upload_file,
      sftp::commands::sftp_pick_download_path,
      sftp::commands::sftp_remove,
      sftp::commands::sftp_copy_file,
      sftp::file_ops::sftp_read_file,
      sftp::file_ops::sftp_write_file,
      editor_storage::editor_load_state,
      editor_storage::editor_save_panel_state,
      editor_storage::editor_save_draft,
      editor_storage::editor_delete_draft,
    ])
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

      // 恢复上次关闭时的窗口大小与位置。配置里的窗口在本函数（Tauri 内部 setup）中先于用户
      // setup 闭包创建，且两者处于同一个事件循环迭代内，因此这里改几何发生在首帧绘制之前，
      // 不会跳变。注：`Manager::get_window` 需要 `unstable` feature，故经 `WebviewWindow` 取底层 `Window`。
      if let Some(webview_window) = app.get_webview_window("main") {
        let webview: &tauri::Webview<_> = webview_window.as_ref();
        window_state::restore(&webview.window());
      }

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
    }
  });
}
