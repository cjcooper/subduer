//! Port-forwards: started from the main window, listed on the Port Forwards
//! page, and kept running across context switches until stopped or the app
//! quits. Each keeps the client of the cluster it was started on.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use subduer_core::forward::{self, ForwardStats};
use subduer_core::types::{ForwardInfo, ForwardRequest, ForwardSource, PortOption, forward_events};
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::AppState;

struct Forward {
    context: String,
    request: ForwardRequest,
    /// The bound port (differs from the request when it asked for 0).
    local_port: u16,
    stats: Arc<Mutex<ForwardStats>>,
    task: JoinHandle<()>,
}

#[derive(Default)]
pub struct Forwards {
    forwards: Mutex<BTreeMap<u64, Forward>>,
    next_id: AtomicU64,
}

impl Forwards {
    fn list(&self) -> Vec<ForwardInfo> {
        self.forwards
            .lock()
            .unwrap()
            .iter()
            .map(|(id, f)| {
                let s = f.stats.lock().unwrap();
                ForwardInfo {
                    id: *id,
                    context: f.context.clone(),
                    namespace: f.request.namespace.clone(),
                    source: f.request.source.clone(),
                    remote_port: f.request.remote_port,
                    local_port: f.local_port,
                    backend: s.backend.clone(),
                    active: s.active,
                    total: s.total,
                    last_error: s.last_error.clone(),
                }
            })
            .collect()
    }
}

/// Sends the current list to the main window.
fn broadcast(app: &AppHandle) {
    let list = app.state::<AppState>().forwards.list();
    let _ = app.emit_to("main", forward_events::CHANGED, list);
}

/// Ports offered by the start dialog.
#[tauri::command]
pub async fn forward_ports(
    state: State<'_, AppState>,
    namespace: String,
    source: ForwardSource,
) -> Result<Vec<PortOption>, String> {
    let client = crate::commands::connected_client(&state).await?;
    forward::port_options(&client, &namespace, &source).await.map_err(|e| e.user_message())
}

#[tauri::command]
pub async fn start_forward(app: AppHandle, state: State<'_, AppState>, request: ForwardRequest) -> Result<ForwardInfo, String> {
    let (client, context) = {
        let session = state.session.read().await;
        let session = session.as_ref().ok_or("not connected to a cluster")?;
        (session.client.clone(), session.context.clone())
    };
    let (listener, backend) = forward::bind(&client, &request).await.map_err(|e| e.user_message())?;
    let local_port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let stats = Arc::new(Mutex::new(ForwardStats::default()));
    let notify_app = app.clone();
    let notify: forward::Notify = Arc::new(move || broadcast(&notify_app));
    let task = tauri::async_runtime::spawn(forward::serve(client, request.clone(), listener, backend, stats.clone(), notify));
    let id = state.forwards.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::info!(%context, id, local_port, source = %request.source.label(), "port-forward started");
    state.forwards.forwards.lock().unwrap().insert(id, Forward { context, request, local_port, stats, task });
    broadcast(&app);
    let info = state.forwards.list().into_iter().find(|f| f.id == id).ok_or("forward vanished")?;
    Ok(info)
}

#[tauri::command]
pub fn list_forwards(state: State<'_, AppState>) -> Vec<ForwardInfo> {
    state.forwards.list()
}

#[tauri::command]
pub fn stop_forward(app: AppHandle, state: State<'_, AppState>, id: u64) {
    if let Some(f) = state.forwards.forwards.lock().unwrap().remove(&id) {
        f.task.abort();
        tracing::info!(id, "port-forward stopped");
    }
    broadcast(&app);
}

/// Opens `http://127.0.0.1:<port>` for a forward in the default browser.
#[tauri::command]
pub fn open_forward(state: State<'_, AppState>, id: u64) -> Result<(), String> {
    let port = state.forwards.forwards.lock().unwrap().get(&id).map(|f| f.local_port).ok_or("no such forward")?;
    let url = format!("http://127.0.0.1:{port}/");
    let opener = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C".into(), "start".into(), String::new(), url])
    } else {
        ("xdg-open", vec![url])
    };
    std::process::Command::new(opener.0).args(opener.1).spawn().map(|_| ()).map_err(|e| e.to_string())
}
