// Every command the UI may call is listed here; tauri-build generates one
// permission per command and the capability file allows exactly these.
const COMMANDS: &[&str] = &[
    "app_info",
    "list_volumes",
    "list_profiles",
    "list_grants",
    "pick_folder",
    "revoke_grant",
    "create_plan",
    "discard_plan",
    "start_import",
    "pause_import",
    "resume_import",
    "cancel_import",
    "list_sessions",
    "session_detail",
    "export_report",
    "run_compare",
    "library",
    "overview",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
