pub mod ai;
pub mod commands;
pub mod common;
pub mod db;
pub mod editor_storage;
pub mod logging;
pub mod sftp;
pub mod terminal;
pub mod text_detect;

#[tauri::command]
fn is_apple_silicon() -> bool {
  cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .plugin(logging::build_log_plugin())
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
      sftp::file_ops::sftp_read_file,
      sftp::file_ops::sftp_write_file,
      editor_storage::editor_load_state,
      editor_storage::editor_save_panel_state,
      editor_storage::editor_save_draft,
      editor_storage::editor_delete_draft,
    ])
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
    .run(tauri::generate_context!())
    .expect("error while building tauri application");
}
