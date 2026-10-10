mod commands;
mod exec;
mod forward;
mod logs;
mod menu;
mod shell_env;
mod upload;

use std::sync::Mutex;
use std::sync::atomic::AtomicU64;

use std::sync::Arc;

use subduer_core::history::HistoryStore;
use subduer_core::kube::Client;
use subduer_core::safety::SafetyGate;
use tauri::Manager;
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
}

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
                main.on_window_event(move |event| {
                    if let tauri::WindowEvent::Destroyed = event {
                        logs::close_all(&handle);
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_contexts,
            commands::connect,
            commands::set_unlocked,
            commands::watch_resources,
            commands::stop_watch,
            commands::get_yaml,
            commands::cluster_overview,
            commands::pod_details,
            commands::pod_history,
            commands::delete_resource,
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
            commands::secret_value,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
