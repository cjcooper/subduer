mod commands;
mod shell_env;

use std::sync::Mutex;
use std::sync::atomic::AtomicU64;

use subduer_core::kube::Client;
use subduer_core::safety::SafetyGate;
use tauri::async_runtime::JoinHandle;

/// The cluster the window is currently connected to.
pub struct Session {
    pub context: String,
    pub client: Client,
}

#[derive(Default)]
pub struct AppState {
    pub gate: SafetyGate,
    pub session: tokio::sync::RwLock<Option<Session>>,
    /// The single live resource watch feeding the table.
    pub watch: Mutex<Option<JoinHandle<()>>>,
    pub next_watch_id: AtomicU64,
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
        .invoke_handler(tauri::generate_handler![
            commands::list_contexts,
            commands::connect,
            commands::set_unlocked,
            commands::watch_resources,
            commands::stop_watch,
            commands::get_yaml,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
