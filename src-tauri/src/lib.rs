mod commands;
mod config;
mod discover;
mod exec;
mod gita;
mod manifest;
mod push;
mod status;

use std::sync::Mutex;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .setup(|app| {
            let config_path = app.path().app_config_dir()?.join("config.json");
            app.manage(commands::AppState {
                config: Mutex::new(config::AppConfig::load(&config_path)),
                config_path,
                manifest_lock: Mutex::new(()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::get_autostart,
            commands::set_autostart,
            commands::load_repos,
            commands::overwrite_manifest,
            commands::discover,
            commands::add_repos,
            commands::remove_repo,
            commands::relocate_repo,
            commands::push,
            commands::push_set_upstream,
            commands::setup_wsl_auth,
            commands::read_manifest,
            commands::clone_entry,
            commands::open_in_explorer,
            commands::open_in_editor,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
