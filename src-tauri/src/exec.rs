//! Shell windows: a native window per `kubectl exec -it` session, bound to the
//! cluster it was opened from. Opening and (re)connecting both require the
//! context to be unlocked: a shell can change anything the container can.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use subduer_core::kube::Client;
use subduer_core::types::{ExecOutput, ExecSession, ExecTarget};
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent, Runtime};
use tokio::sync::mpsc::UnboundedSender;

use crate::{AppState, Win};

/// Window labels look like `exec-3`; capabilities grant `exec-*`.
pub const LABEL_PREFIX: &str = "exec-";

struct ShellWindow {
    client: Client,
    context: String,
    target: ExecTarget,
    title: String,
    input: Option<UnboundedSender<Vec<u8>>>,
    resize: Option<UnboundedSender<(u16, u16)>>,
    task: Option<JoinHandle<()>>,
}

#[derive(Default)]
pub struct ShellWindows {
    windows: Mutex<HashMap<String, ShellWindow>>,
    next_id: AtomicU64,
}

impl ShellWindows {
    fn stop(&self, label: &str) {
        if let Some(w) = self.windows.lock().unwrap().remove(label)
            && let Some(task) = w.task
        {
            task.abort();
        }
    }
}

/// Opens a new shell window for `target` on the current cluster. Each call
/// opens another session, like separate terminal tabs.
#[tauri::command]
pub async fn open_exec<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>, win: Win, target: ExecTarget) -> Result<(), String> {
    let (client, context) = {
        let session = win.session.read().await;
        let session = session.as_ref().ok_or("not connected to a cluster")?;
        (session.client.clone(), session.context.clone())
    };
    win.gate.ensure_writable(&context).map_err(|e| e.to_string())?;
    let id = state.shells.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    let label = format!("{LABEL_PREFIX}{id}");
    let mut title = target.title(&context);
    // Number repeat sessions so the Window menu can tell them apart.
    let same = state.shells.windows.lock().unwrap().values().filter(|w| w.context == context && w.target == target).count();
    if same > 0 {
        title.push_str(&format!(" {}", same + 1));
    }
    state.shells.windows.lock().unwrap().insert(
        label.clone(),
        ShellWindow { client, context: context.clone(), target, title: title.clone(), input: None, resize: None, task: None },
    );
    state.adopt(&label, &win.label);
    tracing::info!(%context, %label, "shell window opened");
    let window = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(format!("index.html?exec={label}").into()))
        .title(&title)
        .inner_size(960.0, 600.0)
        .min_inner_size(420.0, 240.0)
        .disable_drag_drop_handler()
        .build()
        .map_err(|e| {
            state.shells.windows.lock().unwrap().remove(&label);
            state.forget(&label);
            e.to_string()
        })?;
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Destroyed = event {
            let state = handle.state::<AppState>();
            state.shells.stop(&label);
            state.forget(&label);
        }
    });
    Ok(())
}

/// Called by a shell window once it's listening and knows its size:
/// (re)connects the session.
#[tauri::command]
pub fn exec_start<R: Runtime>(window: WebviewWindow<R>, state: State<'_, AppState>, win: Win, cols: u16, rows: u16) -> Result<ExecSession, String> {
    let label = window.label().to_owned();
    let mut windows = state.shells.windows.lock().unwrap();
    let w = windows.get_mut(&label).ok_or("this shell window is no longer connected")?;
    win.gate.ensure_writable(&w.context).map_err(|e| e.to_string())?;
    if let Some(old) = w.task.take() {
        old.abort();
    }
    let (input, input_rx) = tokio::sync::mpsc::unbounded_channel();
    let (resize, resize_rx) = tokio::sync::mpsc::unbounded_channel();
    w.input = Some(input);
    w.resize = Some(resize);
    // One event name per window: the JS `listen` hears every window's events.
    let event = label.clone();
    let (client, target, emit_event) = (w.client.clone(), w.target.clone(), event.clone());
    w.task = Some(tauri::async_runtime::spawn(async move {
        let emitter = window.clone();
        let send = move |out: ExecOutput| {
            if let Err(e) = emitter.emit_to(emitter.label(), &emit_event, out) {
                tracing::warn!(error = %e, "shell emit failed");
            }
        };
        match subduer_core::exec::run(client, &target, (cols, rows), input_rx, resize_rx, &send).await {
            // The user left the shell: close the window, like a terminal tab.
            Ok(subduer_core::exec::Ended::Exited) => {
                let _ = window.close();
            }
            // The message is already on screen, with Reconnect.
            Ok(subduer_core::exec::Ended::Failed(_)) => {}
            Err(e) => send(ExecOutput::Exited(e.user_message())),
        }
    }));
    Ok(ExecSession { context: w.context.clone(), title: w.title.clone(), target: w.target.clone(), event })
}

/// Keystrokes from the terminal.
#[tauri::command]
pub fn exec_input<R: Runtime>(window: WebviewWindow<R>, state: State<'_, AppState>, data: String) {
    if let Some(input) = state.shells.windows.lock().unwrap().get(window.label()).and_then(|w| w.input.as_ref()) {
        let _ = input.send(data.into_bytes());
    }
}

#[tauri::command]
pub fn exec_resize<R: Runtime>(window: WebviewWindow<R>, state: State<'_, AppState>, cols: u16, rows: u16) {
    if let Some(resize) = state.shells.windows.lock().unwrap().get(window.label()).and_then(|w| w.resize.as_ref()) {
        let _ = resize.send((cols, rows));
    }
}
