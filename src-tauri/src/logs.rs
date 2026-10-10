//! Logs windows: each is a separate native window (movable anywhere, listed in
//! the Window menu) bound to the cluster it was opened from. Its stream runs
//! until the window closes.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use subduer_core::kube::Client;
use subduer_core::types::{LogBatch, LogSession, LogTarget};
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent, Runtime};

use crate::AppState;

/// Window labels look like `logs-3`; capabilities grant `logs-*`.
const LABEL_PREFIX: &str = "logs-";

struct LogWindow {
    client: Client,
    context: String,
    target: LogTarget,
    task: Option<JoinHandle<()>>,
}

#[derive(Default)]
pub struct LogWindows {
    windows: Mutex<HashMap<String, LogWindow>>,
    next_id: AtomicU64,
}

impl LogWindows {
    fn stop(&self, label: &str) {
        if let Some(w) = self.windows.lock().unwrap().remove(label)
            && let Some(task) = w.task
        {
            task.abort();
        }
    }
}

/// Opens (or focuses, if already open) a logs window for `target` on the
/// current cluster.
#[tauri::command]
pub async fn open_logs<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>, target: LogTarget) -> Result<(), String> {
    let (client, context) = {
        let session = state.session.read().await;
        let session = session.as_ref().ok_or("not connected to a cluster")?;
        (session.client.clone(), session.context.clone())
    };
    let existing = state
        .logs
        .windows
        .lock()
        .unwrap()
        .iter()
        .find(|(_, w)| w.context == context && w.target == target)
        .map(|(label, _)| label.clone());
    if let Some(window) = existing.and_then(|label| app.get_webview_window(&label)) {
        let _ = window.unminimize();
        return window.set_focus().map_err(|e| e.to_string());
    }

    let id = state.logs.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    let label = format!("{LABEL_PREFIX}{id}");
    let title = target.title(&context);
    state.logs.windows.lock().unwrap().insert(
        label.clone(),
        LogWindow { client, context, target, task: None },
    );
    let window = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(format!("index.html?logs={label}").into()))
        .title(&title)
        .inner_size(1100.0, 700.0)
        .min_inner_size(480.0, 280.0)
        .disable_drag_drop_handler()
        .build()
        .map_err(|e| {
            state.logs.windows.lock().unwrap().remove(&label);
            e.to_string()
        })?;
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Destroyed = event {
            handle.state::<AppState>().logs.stop(&label);
        }
    });
    Ok(())
}

/// Called by a logs window once it's listening: (re)starts its stream and
/// says what it's showing. `previous` reads the last terminated container.
#[tauri::command]
pub fn logs_start<R: Runtime>(window: WebviewWindow<R>, state: State<'_, AppState>, previous: bool) -> Result<LogSession, String> {
    let label = window.label().to_owned();
    let mut windows = state.logs.windows.lock().unwrap();
    let w = windows.get_mut(&label).ok_or("this logs window is no longer connected")?;
    if let Some(old) = w.task.take() {
        old.abort();
    }
    // One event name per window: the JS `listen` hears every window's events.
    let event = label.clone();
    let (client, target, emit_event) = (w.client.clone(), w.target.clone(), event.clone());
    w.task = Some(tauri::async_runtime::spawn(async move {
        let send = move |batch: LogBatch| {
            if let Err(e) = window.emit_to(window.label(), &emit_event, batch) {
                tracing::warn!(error = %e, "logs emit failed");
            }
        };
        if let Err(e) = subduer_core::logs::stream(client, target, previous, &send).await {
            send(LogBatch { lines: vec![], notices: vec![e.user_message()] });
        }
    }));
    Ok(LogSession { context: w.context.clone(), title: w.target.title(&w.context), target: w.target.clone(), event })
}

/// Closes every logs, shell and edit window, e.g. when the main window goes away.
pub fn close_all<R: Runtime>(app: &AppHandle<R>) {
    for (label, window) in app.webview_windows() {
        if label.starts_with(LABEL_PREFIX)
            || label.starts_with(crate::exec::LABEL_PREFIX)
            || label.starts_with(crate::editor::LABEL_PREFIX)
        {
            // Destroyed, not closed: the main window has already asked about unsaved edits.
            let _ = window.destroy();
        }
    }
}
