//! The complete API exposed to the web view. Commands take *grant ids*, never
//! raw paths: the UI cannot name a folder the user did not pick in a native
//! dialog. The engine re-checks authorization at execution time.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use mediaflow_core::compare::{CompareMode, CompareReport};
use mediaflow_core::db::{EventRow, FileRow, Grant, GrantKind, LibraryRow, OpRow};
use mediaflow_core::plan::ImportPlan;
use mediaflow_core::profile::{builtin, builtin_profiles, Profile};
use mediaflow_core::report::{ReportFormat, SessionReport};
use mediaflow_core::state::SessionState;
use mediaflow_core::volumes::{list_volumes as core_volumes, Volume};
use mediaflow_core::{Control, Engine, MfError, PlanRequest, RunOptions, SessionInfo};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

pub struct AppState {
    pub engine: Arc<Engine>,
    pub plans: Mutex<HashMap<String, ImportPlan>>,
    pub controls: Mutex<HashMap<String, Control>>,
}

/// Error shape sent to the UI: stable code + sentence for humans.
#[derive(Debug, Serialize)]
pub struct UiError {
    pub code: String,
    pub message: String,
}

impl From<MfError> for UiError {
    fn from(e: MfError) -> Self {
        UiError { code: e.code().into(), message: e.user_message() }
    }
}

fn ui(code: &str, message: impl Into<String>) -> UiError {
    UiError { code: code.into(), message: message.into() }
}

type R<T> = Result<T, UiError>;

fn blocking<T: Send + 'static>(f: impl FnOnce() -> R<T> + Send + 'static) -> impl std::future::Future<Output = R<T>> {
    async move {
        tauri::async_runtime::spawn_blocking(f).await.map_err(|e| ui("internal", format!("background task failed: {e}")))?
    }
}

#[derive(Serialize)]
pub struct AppInfo {
    version: String,
    library_path: Option<String>,
    network: &'static str,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        library_path: state.engine.db_path().map(|p| p.to_string_lossy().into_owned()),
        network: "MediaFlow makes no network connections.",
    }
}

#[tauri::command]
pub fn list_volumes() -> Vec<Volume> {
    core_volumes()
}

#[tauri::command]
pub fn list_profiles() -> Vec<Profile> {
    builtin_profiles()
}

#[derive(Serialize)]
pub struct GrantView {
    #[serde(flatten)]
    grant: Grant,
    available: bool,
    free_bytes: Option<u64>,
}

fn grant_view(g: Grant) -> GrantView {
    let p = PathBuf::from(&g.path);
    GrantView { available: p.is_dir(), free_bytes: mediaflow_core::volumes::available_space(&p), grant: g }
}

#[tauri::command]
pub fn list_grants(state: State<'_, AppState>) -> R<Vec<GrantView>> {
    Ok(state.engine.grants()?.into_iter().map(grant_view).collect())
}

/// Open the native folder picker. The chosen folder becomes a grant; the
/// path never travels from the UI to the engine.
#[tauri::command]
pub async fn pick_folder(app: AppHandle, state: State<'_, AppState>, kind: String) -> R<Option<GrantView>> {
    let kind = match kind.as_str() {
        "source" => GrantKind::Source,
        "destination" => GrantKind::Destination,
        _ => return Err(ui("invalid_input", "unknown folder kind")),
    };
    let title = if kind == GrantKind::Source { "Choose the card or folder to import" } else { "Choose a destination folder" };
    let picked = {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || app.dialog().file().set_title(title).blocking_pick_folder())
            .await
            .map_err(|e| ui("internal", e.to_string()))?
    };
    let Some(fp) = picked else { return Ok(None) };
    let path = fp.into_path().map_err(|_| ui("invalid_input", "this location is not a local folder"))?;
    let g = state.engine.authorize(&path, kind)?;
    Ok(Some(grant_view(g)))
}

#[tauri::command]
pub fn revoke_grant(state: State<'_, AppState>, id: String) -> R<()> {
    Ok(state.engine.revoke(&id)?)
}

#[derive(Deserialize)]
pub struct PlanArgs {
    source_grant: String,
    destination_grants: Vec<String>,
    profile_id: String,
    project: String,
    label: Option<String>,
    quick: bool,
}

#[derive(Serialize)]
pub struct PlanView {
    plan_id: String,
    #[serde(flatten)]
    plan: ImportPlan,
}

#[derive(Serialize, Clone)]
struct PlanProgress {
    phase: String,
    done: usize,
    total: usize,
}

#[tauri::command]
pub async fn create_plan(app: AppHandle, state: State<'_, AppState>, args: PlanArgs) -> R<PlanView> {
    let engine = state.engine.clone();
    let src = engine.grant(&args.source_grant)?;
    let mut dests = Vec::new();
    for id in &args.destination_grants {
        let g = engine.grant(id)?;
        if g.kind != GrantKind::Destination {
            return Err(ui("not_authorized", "this folder was added as a source, not a destination"));
        }
        dests.push(PathBuf::from(g.path));
    }
    let profile = builtin(&args.profile_id).ok_or_else(|| ui("invalid_input", "unknown profile"))?;
    let label = args.label.filter(|l| !l.trim().is_empty());
    let req = PlanRequest {
        source: PathBuf::from(src.path),
        source_label: label,
        destinations: dests,
        project: args.project.trim().to_string(),
        profile,
        hash_candidates: !args.quick,
    };
    let plan = blocking(move || {
        let mut last = std::time::Instant::now();
        Ok(engine.plan(&req, &Control::new(), |phase, done, total| {
            if last.elapsed().as_millis() > 120 || done == total {
                last = std::time::Instant::now();
                let _ = app.emit("plan-progress", PlanProgress { phase: phase.into(), done, total });
            }
        })?)
    })
    .await?;
    let plan_id = uuid::Uuid::new_v4().to_string();
    state.plans.lock().unwrap_or_else(|e| e.into_inner()).insert(plan_id.clone(), plan.clone());
    Ok(PlanView { plan_id, plan })
}

#[tauri::command]
pub fn discard_plan(state: State<'_, AppState>, plan_id: String) {
    state.plans.lock().unwrap_or_else(|e| e.into_inner()).remove(&plan_id);
}

fn spawn_run(app: AppHandle, state: &AppState, id: String) -> R<()> {
    let engine = state.engine.clone();
    if engine.is_running(&id) {
        return Err(ui("invalid_input", "this import is already running"));
    }
    let control = Control::new();
    state.controls.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone(), control.clone());
    std::thread::Builder::new()
        .name(format!("import-{}", &id[..8]))
        .spawn(move || {
            let res = engine.run_session(&id, &control, RunOptions::default(), &mut |p| {
                let _ = app.emit("import-progress", p);
            });
            let payload = match res {
                Ok(s) => serde_json::json!({ "session_id": id, "state": s }),
                Err(e) => serde_json::json!({ "session_id": id, "state": SessionState::Failed, "error": UiError::from(e) }),
            };
            let _ = app.emit("import-finished", payload);
        })
        .map_err(|e| ui("internal", e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub fn start_import(app: AppHandle, state: State<'_, AppState>, plan_id: String) -> R<String> {
    let plan = state
        .plans
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&plan_id)
        .ok_or_else(|| ui("not_found", "This plan is no longer available. Prepare the import again."))?;
    let id = state.engine.create_session(&plan)?;
    spawn_run(app, &state, id.clone())?;
    Ok(id)
}

#[tauri::command]
pub fn pause_import(state: State<'_, AppState>, session_id: String) -> R<()> {
    match state.controls.lock().unwrap_or_else(|e| e.into_inner()).get(&session_id) {
        Some(c) if state.engine.is_running(&session_id) => {
            c.pause();
            Ok(())
        }
        _ => Err(ui("invalid_input", "This import is not running.")),
    }
}

#[tauri::command]
pub fn resume_import(app: AppHandle, state: State<'_, AppState>, session_id: String) -> R<()> {
    spawn_run(app, &state, session_id)
}

#[tauri::command]
pub fn cancel_import(state: State<'_, AppState>, session_id: String) -> R<()> {
    if state.engine.is_running(&session_id) {
        if let Some(c) = state.controls.lock().unwrap_or_else(|e| e.into_inner()).get(&session_id) {
            c.cancel();
        }
        Ok(())
    } else {
        Ok(state.engine.cancel_session(&session_id)?)
    }
}

#[derive(Serialize)]
pub struct SessionListItem {
    #[serde(flatten)]
    info: SessionInfo,
    running: bool,
}

#[tauri::command]
pub fn list_sessions(state: State<'_, AppState>, limit: Option<usize>) -> R<Vec<SessionListItem>> {
    Ok(state
        .engine
        .sessions(limit.unwrap_or(200).min(5000))?
        .into_iter()
        .map(|info| SessionListItem { running: state.engine.is_running(&info.id), info })
        .collect())
}

#[derive(Serialize)]
pub struct SessionDetail {
    info: SessionInfo,
    running: bool,
    files: Vec<FileRow>,
    ops: Vec<OpRow>,
    events: Vec<EventRow>,
    report: SessionReport,
}

#[tauri::command]
pub async fn session_detail(state: State<'_, AppState>, session_id: String) -> R<SessionDetail> {
    let engine = state.engine.clone();
    blocking(move || {
        let (files, ops) = engine.session_ops(&session_id)?;
        Ok(SessionDetail {
            running: engine.is_running(&session_id),
            info: engine.session(&session_id)?,
            events: engine.events(&session_id)?,
            report: engine.report(&session_id)?,
            files,
            ops,
        })
    })
    .await
}

/// Ask where to save, then write. Exports never overwrite an existing file.
#[tauri::command]
pub async fn export_report(app: AppHandle, state: State<'_, AppState>, session_id: String, format: String) -> R<Option<String>> {
    let fmt = match format.as_str() {
        "json" => ReportFormat::Json,
        "csv" => ReportFormat::Csv,
        _ => return Err(ui("invalid_input", "unknown format")),
    };
    let report = state.engine.report(&session_id)?;
    let ext = if fmt == ReportFormat::Json { "json" } else { "csv" };
    let name = format!("mediaflow-report-{}.{ext}", &session_id[..8.min(session_id.len())]);
    let picked = {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.dialog().file().set_file_name(&name).add_filter(ext.to_uppercase(), &[ext]).blocking_save_file()
        })
        .await
        .map_err(|e| ui("internal", e.to_string()))?
    };
    let Some(fp) = picked else { return Ok(None) };
    let path = fp.into_path().map_err(|_| ui("invalid_input", "not a local file"))?;
    if path.exists() {
        return Err(ui("conflict", "A file with this name already exists. MediaFlow never overwrites files; choose another name."));
    }
    report.write(&path, fmt)?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[derive(Deserialize)]
pub struct CompareArgs {
    source_grant: String,
    destination_grants: Vec<String>,
    verified: bool,
    profile_id: Option<String>,
}

#[tauri::command]
pub async fn run_compare(app: AppHandle, state: State<'_, AppState>, args: CompareArgs) -> R<CompareReport> {
    let engine = state.engine.clone();
    let src = PathBuf::from(engine.grant(&args.source_grant)?.path);
    let dests = args
        .destination_grants
        .iter()
        .map(|id| engine.grant(id).map(|g| PathBuf::from(g.path)))
        .collect::<Result<Vec<_>, _>>()?;
    let exclusions = args.profile_id.as_deref().and_then(builtin).map(|p| p.exclusions).unwrap_or_default();
    let mode = if args.verified { CompareMode::Verified } else { CompareMode::Quick };
    blocking(move || {
        let mut last = std::time::Instant::now();
        Ok(engine.compare(&src, &dests, mode, &exclusions, &Control::new(), |done, total| {
            if last.elapsed().as_millis() > 120 || done == total {
                last = std::time::Instant::now();
                let _ = app.emit("compare-progress", PlanProgress { phase: "comparing".into(), done, total });
            }
        })?)
    })
    .await
}

#[tauri::command]
pub fn library(state: State<'_, AppState>, query: String) -> R<Vec<LibraryRow>> {
    Ok(state.engine.library(&query, 500)?)
}

#[derive(Serialize)]
pub struct Overview {
    recent: Vec<SessionListItem>,
    attention: Vec<SessionListItem>,
    destinations: Vec<GrantView>,
    sources: Vec<GrantView>,
    volumes: Vec<Volume>,
}

#[tauri::command]
pub fn overview(state: State<'_, AppState>) -> R<Overview> {
    let all: Vec<SessionListItem> = list_sessions(state.clone(), Some(100))?;
    let attention = all
        .iter()
        .filter(|s| {
            !s.running
                && matches!(
                    s.info.state,
                    SessionState::Interrupted | SessionState::Paused | SessionState::Failed | SessionState::CompletedWithErrors | SessionState::Planned
                )
        })
        .map(|s| SessionListItem { info: s.info.clone(), running: s.running })
        .collect();
    let grants: Vec<GrantView> = list_grants(state)?;
    let (destinations, sources) = grants.into_iter().partition(|g| g.grant.kind == GrantKind::Destination);
    Ok(Overview { recent: all.into_iter().take(6).collect(), attention, destinations, sources, volumes: core_volumes() })
}
