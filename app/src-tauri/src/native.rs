//! System integration: menu-bar / notification-area icon, Dock and taskbar
//! progress, native notifications, removable-volume detection and the status
//! snapshot read by the macOS widget.
//!
//! Everything shown here comes from engine events or the journal. Nothing in
//! this module starts an import on its own: a detected card only produces a
//! suggestion the user has to accept.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mediaflow_core::progress::Progress;
use mediaflow_core::state::SessionState;
use mediaflow_core::util::human_bytes;
use mediaflow_core::Engine;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

/// App Group shared with the macOS widget extension (see `macos-widget/`).
/// Must match the `com.apple.security.application-groups` entitlement of
/// both targets when the app is signed.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const APP_GROUP: &str = "group.org.mediaflow.app";

const UI_THROTTLE: Duration = Duration::from_millis(500);
const SNAPSHOT_THROTTLE: Duration = Duration::from_secs(3);

pub use mediaflow_core::volumes::DetectedVolume;
use mediaflow_core::volumes::DetectionRegistry;

struct Inner {
    tray: Option<TrayIcon>,
    status_item: Option<MenuItem<tauri::Wry>>,
    toggle_item: Option<MenuItem<tauri::Wry>>,
    running: Option<String>,
    paused: bool,
    last_ui: Instant,
    last_snapshot: Instant,
    detected: DetectionRegistry,
}

#[derive(Clone)]
pub struct Native {
    app: AppHandle,
    engine: Arc<Engine>,
    inner: Arc<Mutex<Inner>>,
}

/// Where the status snapshot is written. On macOS this is the App Group
/// container the sandboxed widget can read; elsewhere the app data folder.
pub fn status_path(app: &AppHandle) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join("Library/Group Containers")
                .join(APP_GROUP)
                .join("status.json")
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        app.path()
            .app_data_dir()
            .ok()
            .map(|d| d.join("status.json"))
    }
}

impl Native {
    pub fn new(app: AppHandle, engine: Arc<Engine>) -> Self {
        let past = Instant::now() - Duration::from_secs(60);
        Native {
            app,
            engine,
            inner: Arc::new(Mutex::new(Inner {
                tray: None,
                status_item: None,
                toggle_item: None,
                running: None,
                paused: false,
                last_ui: past,
                last_snapshot: past,
                detected: DetectionRegistry::default(),
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Build the menu-bar (macOS) / notification-area (Windows, Linux) icon.
    pub fn install_tray(&self) -> tauri::Result<()> {
        let app = &self.app;
        let status = MenuItem::with_id(app, "status", "No import running", false, None::<&str>)?;
        let toggle = MenuItem::with_id(app, "toggle", "Pause import", false, None::<&str>)?;
        let open = MenuItem::with_id(app, "open", "Open MediaFlow", true, None::<&str>)?;
        let new_import = MenuItem::with_id(app, "new_import", "New import…", true, None::<&str>)?;
        let history = MenuItem::with_id(app, "history", "History", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", "Quit MediaFlow", true, None::<&str>)?;
        let menu = Menu::with_items(
            app,
            &[
                &status,
                &toggle,
                &PredefinedMenuItem::separator(app)?,
                &open,
                &new_import,
                &history,
                &PredefinedMenuItem::separator(app)?,
                &quit,
            ],
        )?;
        let icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;
        let me = self.clone();
        let tray = TrayIconBuilder::with_id("mediaflow")
            .icon(icon)
            .icon_as_template(true)
            .tooltip("MediaFlow")
            .menu(&menu)
            .show_menu_on_left_click(true)
            .on_menu_event(move |app, ev| me.on_menu(app, ev.id().as_ref()))
            .build(app)?;
        let mut g = self.lock();
        g.tray = Some(tray);
        g.status_item = Some(status);
        g.toggle_item = Some(toggle);
        drop(g);
        self.refresh_idle();
        Ok(())
    }

    fn on_menu(&self, app: &AppHandle, id: &str) {
        match id {
            "open" => show_main(app),
            "new_import" => {
                show_main(app);
                let _ = app.emit("navigate", serde_json::json!({ "page": "import" }));
            }
            "history" => {
                show_main(app);
                let _ = app.emit("navigate", serde_json::json!({ "page": "history" }));
            }
            "toggle" => {
                let (running, paused) = {
                    let g = self.lock();
                    (g.running.clone(), g.paused)
                };
                if let Some(id) = running {
                    let state = app.state::<crate::commands::AppState>();
                    let res = if paused {
                        crate::commands::spawn_run(app.clone(), &state, id)
                    } else {
                        crate::commands::pause(&state, &id)
                    };
                    if let Err(e) = res {
                        self.notify("MediaFlow", &e.message);
                    }
                }
            }
            "quit" => {
                let busy = {
                    let g = self.lock();
                    g.running.is_some() && !g.paused
                };
                if busy {
                    // Never kill a copy silently: pause first, then quit.
                    show_main(app);
                    self.notify(
                        "An import is running",
                        "Pause it first (menu bar → Pause import), then quit. Nothing is lost if you quit later.",
                    );
                } else {
                    app.exit(0);
                }
            }
            _ => {}
        }
    }

    pub fn notify(&self, title: &str, body: &str) {
        let _ = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show();
    }

    fn set_status_text(&self, text: &str) {
        let g = self.lock();
        if let Some(item) = &g.status_item {
            let _ = item.set_text(text);
        }
        if let Some(tray) = &g.tray {
            let _ = tray.set_tooltip(Some(format!("MediaFlow — {text}")));
        }
    }

    fn set_toggle(&self, text: &str, enabled: bool) {
        if let Some(item) = &self.lock().toggle_item {
            let _ = item.set_text(text);
            let _ = item.set_enabled(enabled);
        }
    }

    fn set_dock_progress(&self, status: ProgressBarStatus, progress: Option<u64>) {
        if let Some(w) = self.app.get_webview_window("main") {
            let _ = w.set_progress_bar(ProgressBarState {
                status: Some(status),
                progress,
            });
        }
    }

    fn set_badge(&self, attention: usize) {
        if let Some(w) = self.app.get_webview_window("main") {
            // macOS Dock / Linux (Unity) badge. Windows has no count badge;
            // the taskbar progress colour carries the state there.
            let _ = w.set_badge_count(if attention > 0 {
                Some(attention as i64)
            } else {
                None
            });
        }
    }

    /// Called for every engine progress event (throttled here).
    pub fn on_progress(&self, p: &Progress) {
        let now = Instant::now();
        let (ui_due, snap_due) = {
            let mut g = self.lock();
            g.running = Some(p.session_id.clone());
            g.paused = false;
            let ui = now.duration_since(g.last_ui) >= UI_THROTTLE;
            let snap = now.duration_since(g.last_snapshot) >= SNAPSHOT_THROTTLE;
            if ui {
                g.last_ui = now;
            }
            if snap {
                g.last_snapshot = now;
            }
            (ui, snap)
        };
        if ui_due {
            let pct = (p.fraction * 100.0).floor() as u64;
            let phase = match p.state {
                Some(SessionState::Verifying) => "Final check",
                _ => "Copying & verifying",
            };
            let file = p
                .current
                .as_ref()
                .map(|c| short_name(&c.rel_path))
                .unwrap_or_default();
            self.set_status_text(&format!("{phase} {pct}%  {file}"));
            self.set_toggle("Pause import", true);
            self.set_dock_progress(ProgressBarStatus::Normal, Some(pct.min(100)));
        }
        if snap_due {
            self.write_snapshot(Some(p));
        }
    }

    /// Called once when a run stops, whatever the reason.
    pub fn on_finished(&self, session_id: &str, state: SessionState) {
        {
            let mut g = self.lock();
            g.paused = state == SessionState::Paused;
            g.running = if g.paused {
                Some(session_id.to_string())
            } else {
                None
            };
            g.last_ui = Instant::now() - Duration::from_secs(60);
        }
        let info = self.engine.session(session_id).ok();
        let name = info
            .as_ref()
            .map(|s| {
                if s.project.is_empty() {
                    s.source_label.clone()
                } else {
                    s.project.clone()
                }
            })
            .unwrap_or_else(|| "Import".into());
        match state {
            SessionState::Completed => {
                let c = info.as_ref().map(|s| {
                    (
                        s.counts.done + s.counts.skipped_duplicate,
                        s.counts.bytes_done,
                    )
                });
                let (n, b) = c.unwrap_or((0, 0));
                self.notify(
                    &format!("{name}: every file is verified"),
                    &format!(
                        "{n} file(s), {} copied and checked with SHA-256.",
                        human_bytes(b)
                    ),
                );
                self.set_dock_progress(ProgressBarStatus::None, None);
            }
            SessionState::CompletedWithErrors => {
                self.notify(
                    &format!("{name}: finished with problems"),
                    "Some files were not copied. Do not format the card yet — open MediaFlow to see which ones.",
                );
                self.set_dock_progress(ProgressBarStatus::Error, Some(100));
            }
            SessionState::Interrupted => {
                let why = info
                    .and_then(|s| s.last_error)
                    .unwrap_or_else(|| "The import stopped.".into());
                self.notify(
                    &format!("{name}: interrupted"),
                    &format!("{why} Resume it from MediaFlow."),
                );
                self.set_dock_progress(ProgressBarStatus::Error, Some(100));
            }
            SessionState::Paused => {
                self.set_dock_progress(ProgressBarStatus::Paused, None);
            }
            SessionState::Failed => {
                self.notify(
                    &format!("{name}: failed"),
                    "Open MediaFlow for details. Verified copies were kept.",
                );
                self.set_dock_progress(ProgressBarStatus::Error, Some(100));
            }
            _ => self.set_dock_progress(ProgressBarStatus::None, None),
        }
        if state == SessionState::Paused {
            self.set_status_text(&format!("Paused — {name}"));
            self.set_toggle("Resume import", true);
        } else {
            self.refresh_idle();
        }
        self.write_snapshot(None);
    }

    /// Menu text, badge and snapshot when nothing is running.
    pub fn refresh_idle(&self) {
        if self.lock().running.is_some() {
            return;
        }
        let snap = self.engine.status_snapshot(None).ok();
        let attention = snap.as_ref().map(|s| s.attention).unwrap_or(0);
        let text = match snap.as_ref().and_then(|s| s.last.as_ref()) {
            _ if attention > 0 => format!("{attention} import(s) need attention"),
            Some(l) if l.verified_ok => format!(
                "Last import verified — {}",
                if l.project.is_empty() {
                    &l.source_label
                } else {
                    &l.project
                }
            ),
            Some(_) => "Last import needs attention".into(),
            None => "No import yet".into(),
        };
        self.set_status_text(&text);
        self.set_toggle("Pause import", false);
        self.set_badge(attention);
        self.write_snapshot(None);
    }

    fn write_snapshot(&self, live: Option<&Progress>) {
        let Some(path) = status_path(&self.app) else {
            return;
        };
        if let Ok(s) = self.engine.status_snapshot(live) {
            let _ = s.write_atomic(&path);
        }
    }

    /// Watch for removable volumes appearing. Volumes present at launch are
    /// not announced. A new card produces a notification and a `volume-detected`
    /// event; the UI may then offer to use it as a source (user decision).
    pub fn start_volume_watcher(&self) {
        let me = self.clone();
        let _ = std::thread::Builder::new().name("volume-watcher".into()).spawn(move || {
            me.lock().detected = DetectionRegistry::new(&mediaflow_core::volumes::list_volumes());
            loop {
                std::thread::sleep(Duration::from_secs(3));
                let now = mediaflow_core::volumes::list_volumes();
                let fresh = me.lock().detected.update(&now);
                for d in fresh {
                    me.notify(
                        &format!("{} connected", d.name),
                        "Open MediaFlow to prepare a verified import. Nothing starts automatically.",
                    );
                    let _ = me.app.emit("volume-detected", &d);
                }
            }
        });
    }

    pub fn detected_volumes(&self) -> Vec<DetectedVolume> {
        self.lock().detected.all()
    }

    /// Authorize a detected card by id (read-only source grant).
    pub fn authorize_detected(
        &self,
        id: &str,
    ) -> mediaflow_core::Result<mediaflow_core::db::Grant> {
        let g = self.lock();
        self.engine.authorize_detected(&g.detected, id)
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn short_name(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    if name.chars().count() > 32 {
        format!("{}…", name.chars().take(31).collect::<String>())
    } else {
        name.to_string()
    }
}
