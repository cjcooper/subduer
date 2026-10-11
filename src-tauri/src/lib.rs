mod commands;
mod editor;
mod exec;
mod forward;
mod logs;
mod menu;
mod shell_env;
mod upload;

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use std::sync::Arc;

use subduer_core::history::HistoryStore;
use subduer_core::kube::Client;
use subduer_core::safety::SafetyGate;
use tauri::{Emitter, Manager};
use tauri::async_runtime::JoinHandle;

/// The cluster the window is currently connected to.
pub struct Session {
    pub context: String,
    pub client: Client,
    /// Prometheus lookup and locally recorded usage, reset on reconnect.
    pub history: Arc<HistoryStore>,
}

#[derive(Default)]
pub struct AppState {
    pub gate: SafetyGate,
    pub session: tokio::sync::RwLock<Option<Session>>,
    /// The single live resource watch feeding the table.
    pub watch: Mutex<Option<JoinHandle<()>>>,
    pub next_watch_id: AtomicU64,
    pub logs: logs::LogWindows,
    pub shells: exec::ShellWindows,
    pub forwards: forward::Forwards,
    pub editors: editor::EditWindows,
    /// The UI holds edits that haven't been saved: closing the window or
    /// quitting asks it first (event `UNSAVED_CLOSE`).
    pub unsaved: AtomicBool,
}

/// Sent to the main window when a close or quit is held for unsaved edits;
/// the payload is `true` for quitting the app.
pub const UNSAVED_CLOSE: &str = "unsaved-close";

impl AppState {
    pub fn stop_watch(&self) {
        if let Some(handle) = self.watch.lock().unwrap().take() {
            handle.abort();
        }
    }
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

    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
            menu::install(app.handle())?;
            // Logs and shell windows belong to the main window; they close with it.
            if let Some(main) = app.get_webview_window("main") {
                let handle = app.handle().clone();
                let window = main.clone();
                main.on_window_event(move |event| match event {
                    // An edit window with unsaved YAML asks first (and is focused).
                    tauri::WindowEvent::CloseRequested { api, .. } if editor::ask_dirty(&handle) => {
                        api.prevent_close();
                    }
                    tauri::WindowEvent::CloseRequested { api, .. }
                        if handle.state::<AppState>().unsaved.load(Ordering::SeqCst) =>
                    {
                        api.prevent_close();
                        let _ = window.emit_to("main", UNSAVED_CLOSE, false);
                    }
                    tauri::WindowEvent::Destroyed => logs::close_all(&handle),
                    _ => {}
                });
            }
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
            commands::close_discarding,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Cmd+Q with unsaved edits: ask the UI first. `app.exit` from
            // `close_discarding` clears the flag before exiting.
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if editor::ask_dirty(app) {
                    api.prevent_exit();
                } else if app.state::<AppState>().unsaved.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    let _ = app.emit_to("main", UNSAVED_CLOSE, true);
                }
            }
        });
}

#[cfg(test)]
mod tests;
