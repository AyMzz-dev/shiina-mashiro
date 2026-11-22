mod commands;
mod state;

use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "shiina_mashiro=info,mashiro_core=info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .setup(|app| {
            // tokio 上下文在 async_runtime 任务里才有，start_services 内部要 tokio::spawn
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                handle.state::<AppState>().start_services();
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            commands::list_accounts,
            commands::save_account,
            commands::qr_bind_start,
            commands::qr_bind_poll,
            commands::remove_account,
            commands::bot_status,
            commands::start_bot,
            commands::stop_bot,
            commands::get_stats,
            commands::list_plugins,
            commands::plugin_handlers,
            commands::install_plugin,
            commands::scaffold_plugin,
            commands::set_plugin_enabled,
            commands::unload_plugin,
            commands::reload_plugin,
            commands::set_plugin_accounts,
            commands::open_path,
            commands::remove_plugin,
            commands::get_setting,
            commands::set_setting,
            commands::check_program,
            commands::data_dir,
            commands::list_logs,
            commands::delete_log,
            commands::clear_logs,
            commands::purge_logs,
            commands::list_timers,
            commands::set_timer_enabled,
            commands::remove_timer,
            commands::list_conversations,
            commands::chat_history,
            commands::send_chat,
            commands::material_list,
            commands::material_upload,
            commands::material_delete,
            commands::group_bot_state,
            commands::send_auth_card,
            commands::agent_generate,
            commands::agent_install,
            commands::agent_models,
            commands::agent_test,
        ])
        .run(tauri::generate_context!())
        .expect("error while running shiina-mashiro");
}
