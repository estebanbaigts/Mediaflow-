//! MediaFlow desktop shell: wires the engine to the web view through the
//! command whitelist in `commands.rs`. No other capability is granted.

mod commands;
mod native;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use commands::AppState;
use mediaflow_core::Engine;
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let db = dir.join("library.sqlite");
            match Engine::open(&db) {
                Ok(engine) => {
                    let engine = Arc::new(engine);
                    let native = native::Native::new(app.handle().clone(), engine.clone());
                    app.manage(AppState {
                        engine,
                        native: native.clone(),
                        plans: Mutex::new(HashMap::new()),
                        controls: Mutex::new(HashMap::new()),
                    });
                    // System surfaces are a convenience: if the desktop does
                    // not support a tray icon, the app still works.
                    if let Err(e) = native.install_tray() {
                        eprintln!("tray icon unavailable: {e}");
                    }
                    native.start_volume_watcher();
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
        .on_window_event(|window, event| {
            // Closing the window during an import keeps the copy going in
            // the menu bar instead of interrupting it.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                let running = state
                    .engine
                    .sessions(50)
                    .map(|v| v.iter().any(|s| state.engine.is_running(&s.id)))
                    .unwrap_or(false);
                if running {
                    api.prevent_close();
                    let _ = window.hide();
                    state.native.notify(
                        "MediaFlow keeps copying",
                        "The import continues. Follow it from the menu bar icon.",
                    );
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
            commands::detected_volumes,
            commands::use_detected_volume,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MediaFlow");
}
