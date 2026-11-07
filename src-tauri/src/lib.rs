mod about;
mod accent;
mod add_cluster;
mod app_icon;
mod capture;
mod commands;
mod desktop_entry;
mod editor;
mod exec;
mod forward;
mod kubeconfig_watch;
mod logs;
mod menu;
mod shell_env;
mod system;
mod trace;
mod upload;
mod vibrancy;

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use std::sync::Arc;

use subduer_core::history::HistoryStore;
use subduer_core::kube::Client;
use subduer_core::safety::{ProdMarks, SafetyGate};
use subduer_core::types::edit_events;
use tauri::async_runtime::JoinHandle;
use tauri::ipc::{CommandArg, CommandItem, InvokeError};
use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime, WebviewWindow, WindowEvent};

/// The cluster a main window is currently connected to.
pub struct Session {
    pub context: String,
    pub client: Client,
    /// Prometheus lookup and locally recorded usage, reset on reconnect.
    pub history: Arc<HistoryStore>,
}

/// What one main window has to itself. Main windows never share any of it, so
/// two windows can be on the same cluster or different ones without either
/// noticing the other.
#[derive(Default)]
pub struct WindowState {
    /// "Allow changes", per context, for this window only.
    pub gate: SafetyGate,
    pub session: tokio::sync::RwLock<Option<Session>>,
    /// The context last picked here, set as its connect starts: the window
    /// is on it even while connecting, or after it failed or dropped.
    pub chosen: Mutex<Option<String>>,
    /// The single live resource watch feeding the table.
    pub watch: Mutex<Option<JoinHandle<()>>>,
    pub next_watch_id: AtomicU64,
    /// Bumped by every `connect`; an older one still waiting on an unreachable
    /// cluster then doesn't replace the session a newer one opened.
    pub connect_attempt: AtomicU64,
    /// The UI holds edits that haven't been saved: closing the window or
    /// quitting asks it first (event `UNSAVED_CLOSE`).
    pub unsaved: AtomicBool,
    /// The running trace, if any; closing or quitting asks first too.
    pub traces: trace::TraceRunner,
    /// Port forwards started here; they stop when the window closes.
    pub forwards: forward::Forwards,
}

impl WindowState {
    /// Closing or quitting needs the UI to ask first.
    pub fn must_ask(&self) -> bool {
        self.unsaved.load(Ordering::SeqCst) || self.traces.is_active()
    }

    pub fn stop_watch(&self) {
        if let Some(handle) = self.watch.lock().unwrap().take() {
            handle.abort();
        }
    }
}

#[derive(Default)]
pub struct AppState {
    /// Main windows by label (`main`, `main-2`, …).
    windows: Mutex<HashMap<String, Arc<WindowState>>>,
    /// Logs, shell, edit and capture windows, by label, to the main window
    /// that opened them.
    owners: Mutex<HashMap<String, String>>,
    next_window: AtomicU64,
    pub logs: logs::LogWindows,
    pub shells: exec::ShellWindows,
    pub editors: editor::EditWindows,
    /// Packet captures, one per capture window.
    pub captures: capture::CaptureWindows,
    /// Settings › Transparency is off: main windows open without the
    /// system material (`vibrancy`).
    pub opaque: AtomicBool,
    /// Contexts marked as production, or as not (File › Manage Clusters…).
    pub prod: Mutex<ProdMarks>,
    /// Starts the browser and Finder.
    pub launcher: system::Launcher,
}

/// The first main window, from `tauri.conf.json`; later ones are `main-2`, ….
pub const MAIN_LABEL: &str = "main";

impl AppState {
    /// Gives a new main window its own state.
    pub fn add_window(&self, label: &str) -> Arc<WindowState> {
        let state = Arc::new(WindowState::default());
        self.windows.lock().unwrap().insert(label.to_owned(), state.clone());
        state
    }

    fn remove_window(&self, label: &str) -> Option<Arc<WindowState>> {
        self.owners.lock().unwrap().retain(|_, owner| owner != label);
        self.windows.lock().unwrap().remove(label)
    }

    /// The state of main window `label` (not of a window it opened).
    fn main_window(&self, label: &str) -> Option<Arc<WindowState>> {
        self.windows.lock().unwrap().get(label).cloned()
    }

    /// The main window `label` is, or belongs to.
    pub fn owner(&self, label: &str) -> Option<String> {
        if self.windows.lock().unwrap().contains_key(label) {
            return Some(label.to_owned());
        }
        self.owners.lock().unwrap().get(label).cloned()
    }

    /// The state of the main window `label` is, or belongs to.
    pub fn window(&self, label: &str) -> Option<Win> {
        let owner = self.owner(label)?;
        let state = self.windows.lock().unwrap().get(&owner)?.clone();
        Some(Win { label: owner, state })
    }

    /// Records that the child window `child` was opened from main window `owner`.
    pub fn adopt(&self, child: &str, owner: &str) {
        self.owners.lock().unwrap().insert(child.to_owned(), owner.to_owned());
    }

    /// The child windows opened from main window `owner`.
    pub fn children(&self, owner: &str) -> Vec<String> {
        let owners = self.owners.lock().unwrap();
        owners.iter().filter(|(_, o)| *o == owner).map(|(child, _)| child.clone()).collect()
    }

    fn forget(&self, child: &str) {
        self.owners.lock().unwrap().remove(child);
    }

    /// The contexts main windows other than `label` are on.
    pub(crate) fn in_use_elsewhere(&self, label: &str) -> Vec<String> {
        let mut names: Vec<String> =
            self.main_windows().into_iter().filter(|w| w.label != label).filter_map(|w| w.chosen.lock().unwrap().clone()).collect();
        names.sort();
        names.dedup();
        names
    }

    pub(crate) fn main_windows(&self) -> Vec<Win> {
        let windows = self.windows.lock().unwrap();
        windows.iter().map(|(label, state)| Win { label: label.clone(), state: state.clone() }).collect()
    }

    /// Main windows with unsaved edits or a running trace.
    fn asking(&self) -> Option<String> {
        self.main_windows().into_iter().find(|w| w.must_ask()).map(|w| w.label)
    }
}

/// The calling main window's state, as a command argument. A logs, shell,
/// edit or capture window gets the state of the main window it came from.
#[derive(Clone)]
pub struct Win {
    /// The main window's label, for events meant for it alone.
    pub label: String,
    state: Arc<WindowState>,
}

impl std::ops::Deref for Win {
    type Target = WindowState;

    fn deref(&self) -> &WindowState {
        &self.state
    }
}

impl<'de, R: Runtime> CommandArg<'de, R> for Win {
    fn from_command(command: CommandItem<'de, R>) -> Result<Self, InvokeError> {
        let webview = command.message.webview();
        let state = webview.state::<AppState>();
        state.window(webview.label()).ok_or_else(|| InvokeError::from("this window is no longer connected"))
    }
}

/// Sent to a main window when its close (or the app's quit) is held for
/// unsaved edits or a running trace; the payload is `true` for quitting.
pub const UNSAVED_CLOSE: &str = "unsaved-close";

/// Logs a failure the app carries on after (an event that couldn't be sent,
/// a window that couldn't be opened).
pub(crate) fn log_err<T, E: std::fmt::Display>(result: Result<T, E>, what: &str) {
    if let Err(e) = result {
        tracing::warn!(error = %e, "{what}");
    }
}

/// Opens another main window, starting unconnected like the first.
pub fn new_main_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let n = app.state::<AppState>().next_window.fetch_add(1, Ordering::Relaxed) + 2;
    let window = open_main_window(app, &format!("{MAIN_LABEL}-{n}"))?;
    let _ = window.set_focus();
    Ok(())
}

/// Builds main window `label` from the `main` window's config (which Tauri
/// doesn't create itself: `vibrancy` decides transparency at creation).
fn open_main_window<R: Runtime>(app: &AppHandle<R>, label: &str) -> tauri::Result<WebviewWindow<R>> {
    let state = app.state::<AppState>();
    let mut config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == MAIN_LABEL)
        .cloned()
        .unwrap_or_default();
    config.label = label.to_owned();
    config.create = true;
    state.add_window(label);
    vibrancy::build_main(app, &config).inspect_err(|_| {
        state.remove_window(label);
    })
}

/// Process setup, before the app (or any other thread) starts: logging, the
/// login shell's `PATH`, and per-system workarounds.
pub fn prepare() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,kube=warn".into()))
        .try_init();
    // Apps launched from Finder get a minimal PATH; kubeconfig exec plugins
    // such as `aws` need the user's shell PATH.
    shell_env::inherit_login_path();
    // The Pi's GPU driver garbles WebKitGTK's first frame.
    shell_env::work_around_pi_rendering();
    // Linux desktops take the window's icon from a desktop entry; give runs
    // that weren't installed from a package one.
    desktop_entry::ensure();
}

/// Builds and runs the app until it quits. `main` passes Tauri's runtime and
/// the generated context; tests pass the mock runtime.
pub fn start<R: Runtime>(builder: tauri::Builder<R>, context: tauri::Context<R>) {
    handlers(builder)
        .manage(system::Dialogs::<R>::default())
        .setup(setup)
        .build(context)
        .expect("error while building tauri application")
        .run(on_run_event);
}

/// The menu, production marks, the first main window and the kubeconfig watch.
fn setup<R: Runtime>(app: &mut tauri::App<R>) -> Result<(), Box<dyn std::error::Error>> {
    menu::install(app.handle())?;
    if let Ok(dir) = app.path().app_config_dir() {
        *app.state::<AppState>().prod.lock().unwrap() = ProdMarks::load(&dir.join("production.json"));
    }
    open_main_window(app.handle(), MAIN_LABEL)?;
    tauri::async_runtime::spawn(kubeconfig_watch::run(app.handle().clone()));
    Ok(())
}

/// The plugins, the app's state and every command the UI invokes.
fn handlers<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(app_icon::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_contexts,
            commands::connect,
            commands::check_connection,
            commands::set_unlocked,
            commands::watch_resources,
            commands::stop_watch,
            commands::get_yaml,
            commands::cluster_overview,
            commands::pod_details,
            commands::pod_history,
            commands::delete_resource,
            commands::get_replicas,
            commands::workload_details,
            commands::workload_history,
            commands::node_history,
            commands::restart_workload,
            commands::scale_workload,
            commands::suspend_workload,
            commands::run_cron_job,
            commands::object_details,
            commands::secret_values,
            commands::save_config_data,
            commands::set_hpa_range,
            logs::open_logs,
            logs::logs_start,
            exec::open_exec,
            capture::start_capture,
            capture::capture_session,
            capture::stop_capture,
            capture::choose_capture_file,
            exec::exec_start,
            exec::exec_input,
            exec::exec_resize,
            forward::forward_ports,
            forward::start_forward,
            forward::list_forwards,
            forward::stop_forward,
            forward::open_forward,
            upload::upload_file,
            upload::browse_dir,
            upload::download_file,
            upload::reveal_download,
            commands::secret_value,
            commands::set_unsaved,
            commands::set_app_theme,
            accent::system_accent,
            vibrancy::set_transparency,
            about::about_info,
            commands::open_url,
            editor::open_editor,
            editor::editor_start,
            editor::editor_save,
            editor::editor_dirty,
            editor::editor_close,
            commands::create_namespace,
            commands::helm_rollback,
            commands::helm_uninstall,
            commands::check_access,
            commands::expand_claim,
            commands::set_reclaim_policy,
            commands::force_sync,
            commands::pause_scaling,
            commands::set_scaling_range,
            commands::close_discarding,
            trace::trace_targets,
            trace::choose_trace_file,
            trace::start_trace,
            trace::trace_status,
            trace::stop_trace,
            trace::reveal_trace,
            trace::choose_trace_to_open,
            trace::trace_review,
            trace::trace_series,
            trace::trace_overview,
            trace::trace_daily,
            trace::trace_set_note,
            trace::trace_compare,
            trace::choose_report_file,
            trace::export_trace_report,
            commands::rightsize_preview,
            commands::rightsize_apply,
            add_cluster::kubeconfig_target,
            add_cluster::add_cluster,
            add_cluster::cloud_accounts,
            add_cluster::cloud_clusters,
            add_cluster::add_cloud_cluster,
            add_cluster::choose_cert_file,
            add_cluster::managed_contexts,
            add_cluster::remove_context,
            add_cluster::reorder_contexts,
            add_cluster::contexts_in_use_elsewhere,
            add_cluster::set_production,
        ])
}

/// Events from the running app: windows asking to close or closing, and
/// quitting. Tauri hands every window's events here too, so the handling
/// lives in one place for every kind of window.
pub(crate) fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: RunEvent) {
    match event {
        // Cmd+Q with unsaved edits or a running trace: ask the UI first.
        // `close_discarding` stops the trace and clears the flag before
        // exiting.
        RunEvent::ExitRequested { api, .. } if hold_exit(app) => api.prevent_exit(),
        RunEvent::WindowEvent { label, event: WindowEvent::CloseRequested { api, .. }, .. } if hold_close(app, &label) => {
            api.prevent_close();
        }
        RunEvent::WindowEvent { label, event, .. } => window_event(app, &label, &event),
        // Quitting that can't be held (macOS: the Dock's Quit, logging out):
        // still close the trace's file, with its last pull.
        RunEvent::Exit => {
            for window in app.state::<AppState>().main_windows() {
                trace::stop_on_exit(&window.traces);
            }
        }
        _ => {}
    }
}

/// Whether quitting waits: an edit window with unsaved YAML, or a main
/// window with unsaved edits or a running trace, asks first (one window at a
/// time: its answer quits again, which asks the next).
fn hold_exit<R: Runtime>(app: &AppHandle<R>) -> bool {
    if editor::ask_dirty(app, None) {
        return true;
    }
    let Some(label) = app.state::<AppState>().asking() else { return false };
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    let _ = app.emit_to(&label, UNSAVED_CLOSE, true);
    true
}

/// Whether closing window `label` waits for the UI to ask first: an edit
/// window with unsaved YAML (it closes itself on Discard), or a main window
/// whose edit windows hold some, or with unsaved edits or a running trace.
fn hold_close<R: Runtime>(app: &AppHandle<R>, label: &str) -> bool {
    let state = app.state::<AppState>();
    if state.editors.is_dirty(label) {
        let _ = app.emit_to(label, edit_events::CLOSE_REQUESTED, ());
        return true;
    }
    let Some(main) = state.main_window(label) else { return false };
    // An edit window with unsaved YAML asks first (and is focused).
    if editor::ask_dirty(app, Some(label)) {
        return true;
    }
    if !main.must_ask() {
        return false;
    }
    let _ = app.emit_to(label, UNSAVED_CLOSE, false);
    true
}

/// Window events other than a close request.
fn window_event<R: Runtime>(app: &AppHandle<R>, label: &str, event: &WindowEvent) {
    match event {
        WindowEvent::Destroyed => closed(app, label),
        WindowEvent::ThemeChanged(theme) => app_icon::theme_changed(app, label, *theme),
        _ => {}
    }
}

/// Window `label` is gone. A logs, shell, edit or capture window stops what
/// it was running. A main window closes its logs, shell and edit windows and
/// stops its watch, trace and port forwards.
fn closed<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let state = app.state::<AppState>();
    state.logs.stop(label);
    state.shells.stop(label);
    state.editors.forget(label);
    state.captures.closed(label);
    state.forget(label);
    let Some(main) = state.main_window(label) else { return };
    logs::close_all(app, label);
    state.remove_window(label);
    main.stop_watch();
    main.forwards.stop_all();
    trace::stop_on_exit(&main.traces);
    // Its context is free to remove now.
    add_cluster::in_use_changed(app);
}

/// The app on phones and tablets (not built); desktops start in `main.rs`.
#[cfg(mobile)]
#[tauri::mobile_entry_point]
pub fn run() {
    prepare();
    start(tauri::Builder::default(), tauri::generate_context!());
}

#[cfg(test)]
mod tests;
