#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod db;
mod encryption;
mod launcher_detect;
mod game_detect;
mod credentials;
mod switcher;
mod cdp;

use std::sync::Mutex;
use tauri::Manager;

/// App state shared across commands
pub struct AppState {
    pub db: Mutex<db::Database>,
}

fn main() {
    env_logger::init();

    tauri::Builder::default()
        .setup(|app| {
            let app_dir = app.path_resolver().app_data_dir().unwrap();
            std::fs::create_dir_all(&app_dir).ok();
            let db_path = app_dir.join("credential-lab.db");
            let database = db::Database::new(&db_path)
                .expect("Failed to initialize database");
            app.manage(AppState {
                db: Mutex::new(database),
            });
            log::info!("Credential Lab initialized. DB at {:?}", db_path);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Launcher detection
            launcher_detect::detect_launchers,
            launcher_detect::get_launcher_current_user,
            // Game detection
            game_detect::detect_games,
            // Credential management
            credentials::sync_current_credential,
            credentials::list_credentials,
            credentials::remove_credential,
            credentials::wipe_launcher_login,
            // Account switching
            switcher::switch_account,
            switcher::test_all_accounts,
            switcher::verify_launcher_state,
            // Auto-login
            cdp::cdp_probe_launcher,
            cdp::cdp_login,
            cdp::cdp_list_pages,
            cdp::save_login_account,
            cdp::list_login_accounts,
            cdp::remove_login_account,
            cdp::autologin_with_account,
            // Logging
            get_logs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Get recent operation logs
#[tauri::command]
fn get_logs(state: tauri::State<AppState>) -> Result<Vec<db::LogEntry>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_recent_logs(100)
}
