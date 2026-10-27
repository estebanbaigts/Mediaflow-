//! MediaFlow desktop shell: wires the engine to the web view through the
//! command whitelist in `commands.rs`. No other capability is granted.

mod commands;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use commands::AppState;
use mediaflow_core::Engine;
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let db = dir.join("library.sqlite");
            match Engine::open(&db) {
                Ok(engine) => {
                    app.manage(AppState {
                        engine: Arc::new(engine),
                        plans: Mutex::new(HashMap::new()),
                        controls: Mutex::new(HashMap::new()),
                    });
                    Ok(())
                }
                Err(e) => {
                    app.dialog()
                        .message(e.user_message())
                        .title("MediaFlow cannot start")
                        .kind(MessageDialogKind::Error)
                        .blocking_show();
                    Err(Box::new(e))
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::list_volumes,
            commands::list_profiles,
            commands::list_grants,
            commands::pick_folder,
            commands::revoke_grant,
            commands::create_plan,
            commands::discard_plan,
            commands::start_import,
            commands::pause_import,
            commands::resume_import,
            commands::cancel_import,
            commands::list_sessions,
            commands::session_detail,
            commands::export_report,
            commands::run_compare,
            commands::library,
            commands::overview,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MediaFlow");
}
