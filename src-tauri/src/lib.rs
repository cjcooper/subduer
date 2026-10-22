mod about;
mod accent;
mod app_icon;
mod capture;
mod commands;
mod desktop_entry;
mod editor;
mod exec;
mod forward;
mod logs;
mod menu;
mod shell_env;
mod trace;
mod upload;
mod vibrancy;

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use std::sync::Arc;

use subduer_core::history::HistoryStore;
use subduer_core::kube::Client;
use subduer_core::safety::SafetyGate;
use tauri::ipc::{CommandArg, CommandItem, InvokeError};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};
use tauri::async_runtime::JoinHandle;

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
    /// The single live resource watch feeding the table.
    pub watch: Mutex<Option<JoinHandle<()>>>,
    pub next_watch_id: AtomicU64,
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
    match vibrancy::build_main(app, &config) {
        Ok(window) => {
            watch_main_window(app, &window);
            Ok(window)
        }
        Err(e) => {
            state.remove_window(label);
            Err(e)
        }
    }
}

/// Close handling for a main window: asks first about unsaved edits or a
/// running trace, and when it's gone closes its logs, shell and edit windows
/// and stops its watch, trace and port forwards.
fn watch_main_window<R: Runtime>(app: &AppHandle<R>, main: &WebviewWindow<R>) {
    let handle = app.clone();
    let window = main.clone();
    main.on_window_event(move |event| {
        let label = window.label();
        match event {
            // An edit window with unsaved YAML asks first (and is focused).
            tauri::WindowEvent::CloseRequested { api, .. } if editor::ask_dirty(&handle, Some(label)) => {
                api.prevent_close();
            }
            tauri::WindowEvent::CloseRequested { api, .. }
                if handle.state::<AppState>().window(label).is_some_and(|w| w.must_ask()) =>
            {
                api.prevent_close();
                let _ = window.emit_to(label, UNSAVED_CLOSE, false);
            }
            tauri::WindowEvent::Destroyed => {
                logs::close_all(&handle, label);
                if let Some(state) = handle.state::<AppState>().remove_window(label) {
                    state.stop_watch();
                    state.forwards.stop_all();
                    trace::stop_on_exit(&state.traces);
                }
            }
            _ => {}
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,kube=warn".into()),
        )
        .init();
    // Apps launched from Finder get a minimal PATH; kubeconfig exec plugins
    // such as `aws` need the user's shell PATH.
    shell_env::inherit_login_path();
    // The Pi's GPU driver garbles WebKitGTK's first frame.
    shell_env::work_around_pi_rendering();
    // Linux desktops take the window's icon from a desktop entry; give runs
    // that weren't installed from a package one.
    desktop_entry::ensure();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(app_icon::init())
        .manage(AppState::default())
        .setup(|app| {
            menu::install(app.handle())?;
            // Logs and shell windows belong to the main window; they close with it.
            open_main_window(app.handle(), MAIN_LABEL)?;
            Ok(())
        })
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
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Cmd+Q with unsaved edits or a running trace: ask the UI first.
            // `close_discarding` stops the trace and clears the flag before
            // exiting.
            match event {
                tauri::RunEvent::ExitRequested { api, .. } => {
                    if editor::ask_dirty(app, None) {
                        api.prevent_exit();
                    } else if let Some(label) = app.state::<AppState>().asking() {
                        // One window at a time: its answer quits again, which
                        // asks the next.
                        api.prevent_exit();
                        if let Some(window) = app.get_webview_window(&label) {
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                        let _ = app.emit_to(&label, UNSAVED_CLOSE, true);
                    }
                }
                // Quitting that can't be held (macOS: the Dock's Quit, logging
                // out): still close the trace's file, with its last pull.
                tauri::RunEvent::Exit => {
                    for window in app.state::<AppState>().main_windows() {
                        trace::stop_on_exit(&window.traces);
                    }
                }
                _ => {}
            }
        });
}

#[cfg(test)]
mod tests;
