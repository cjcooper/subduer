//! YAML edit windows: one native window per object (opening it again focuses
//! the existing one), bound to the cluster it was opened from. Saving is gated
//! like other changes; a window with unsaved edits asks before it closes.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use subduer_core::kube::Client;
use subduer_core::types::{EditSession, EditTarget, edit_events};
use tauri::{AppHandle, Emitter, Manager, Runtime, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::{AppState, Win};

/// Window labels look like `edit-3`; capabilities grant `edit-*`.
pub const LABEL_PREFIX: &str = "edit-";

struct EditWindow {
    client: Client,
    context: String,
    target: EditTarget,
    /// The editor holds changes that haven't been saved.
    dirty: bool,
}

#[derive(Default)]
pub struct EditWindows {
    windows: Mutex<HashMap<String, EditWindow>>,
    next_id: AtomicU64,
}

impl EditWindows {
    fn is_dirty(&self, label: &str) -> bool {
        self.windows.lock().unwrap().get(label).is_some_and(|w| w.dirty)
    }
}

/// Whether a context looks like production, by its name or its cluster's.
fn prod_like(context: &str) -> bool {
    let from_kubeconfig = subduer_core::contexts::load_kubeconfig().ok().and_then(|k| {
        let gate = subduer_core::safety::SafetyGate::default();
        subduer_core::contexts::list_contexts(&k, &gate).contexts.into_iter().find(|c| c.name == context)
    });
    from_kubeconfig.map_or_else(|| subduer_core::safety::is_prod_like(context), |c| c.prod_like)
}

/// Opens (or focuses, if already open) an edit window for one object on the
/// current cluster.
#[tauri::command]
pub async fn open_editor<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>, win: Win, target: EditTarget) -> Result<(), String> {
    if !subduer_core::edit::editable(&target.kind) {
        return Err(format!("{} can't be edited as YAML", target.kind.kind));
    }
    let (client, context) = {
        let session = win.session.read().await;
        let session = session.as_ref().ok_or("not connected to a cluster")?;
        (session.client.clone(), session.context.clone())
    };
    let existing = state
        .editors
        .windows
        .lock()
        .unwrap()
        .iter()
        .find(|(label, w)| w.context == context && w.target == target && state.owner(label).as_ref() == Some(&win.label))
        .map(|(label, _)| label.clone());
    if let Some(window) = existing.and_then(|label| app.get_webview_window(&label)) {
        let _ = window.unminimize();
        return window.set_focus().map_err(|e| e.to_string());
    }
    let id = state.editors.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    let label = format!("{LABEL_PREFIX}{id}");
    let title = target.title(&context);
    state
        .editors
        .windows
        .lock()
        .unwrap()
        .insert(label.clone(), EditWindow { client, context, target, dirty: false });
    state.adopt(&label, &win.label);
    let window = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(format!("index.html?edit={label}").into()))
        .title(&title)
        .inner_size(1000.0, 780.0)
        .min_inner_size(560.0, 360.0)
        .disable_drag_drop_handler()
        .build()
        .map_err(|e| {
            state.editors.windows.lock().unwrap().remove(&label);
            state.forget(&label);
            e.to_string()
        })?;
    let handle = app.clone();
    let events = window.clone();
    window.on_window_event(move |event| match event {
        // Unsaved edits: ask the window first; it closes itself on Discard.
        WindowEvent::CloseRequested { api, .. } if handle.state::<AppState>().editors.is_dirty(&label) => {
            api.prevent_close();
            let _ = events.emit_to(events.label(), edit_events::CLOSE_REQUESTED, ());
        }
        WindowEvent::Destroyed => {
            let state = handle.state::<AppState>();
            state.editors.windows.lock().unwrap().remove(&label);
            state.forget(&label);
        }
        _ => {}
    });
    Ok(())
}

fn window_target<R: Runtime>(window: &WebviewWindow<R>, state: &AppState) -> Result<(Client, String, EditTarget), String> {
    let windows = state.editors.windows.lock().unwrap();
    let w = windows.get(window.label()).ok_or("this edit window is no longer connected")?;
    Ok((w.client.clone(), w.context.clone(), w.target.clone()))
}

/// Called by an edit window when it opens, and on Reload: the object's
/// current YAML and what the window needs to know about its context.
#[tauri::command]
pub async fn editor_start<R: Runtime>(window: WebviewWindow<R>, state: State<'_, AppState>, win: Win) -> Result<EditSession, String> {
    let (client, context, target) = window_target(&window, &state)?;
    let yaml = subduer_core::edit::load(&client, &target.kind, target.namespace.as_deref(), &target.name)
        .await
        .map_err(|e| e.user_message())?;
    Ok(EditSession {
        prod_like: prod_like(&context),
        unlocked: win.gate.is_unlocked(&context),
        title: target.title(&context),
        context,
        target,
        yaml,
    })
}

/// Saves the edited YAML (or, with `dry_run`, only has the server validate
/// it). Returns the saved YAML. After a real save its main window is told,
/// so it can refresh what it shows.
#[tauri::command(rename_all = "snake_case")]
pub async fn editor_save<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, AppState>,
    win: Win,
    yaml: String,
    dry_run: bool,
) -> Result<String, String> {
    let (client, context, target) = window_target(&window, &state)?;
    let saved = subduer_core::edit::save(
        &client,
        &win.gate,
        &context,
        &target.kind,
        target.namespace.as_deref(),
        &target.name,
        &yaml,
        dry_run,
    )
    .await
    .map_err(|e| e.user_message())?;
    if !dry_run {
        if let Some(w) = state.editors.windows.lock().unwrap().get_mut(window.label()) {
            w.dirty = false;
        }
        let _ = window.emit_to(&win.label, edit_events::SAVED, &target);
    }
    Ok(saved)
}

/// The window reports whether it holds unsaved edits.
#[tauri::command]
pub fn editor_dirty<R: Runtime>(window: WebviewWindow<R>, state: State<'_, AppState>, dirty: bool) {
    if let Some(w) = state.editors.windows.lock().unwrap().get_mut(window.label()) {
        w.dirty = dirty;
    }
}

/// Closes the window, discarding any edits (after the window asked).
#[tauri::command]
pub fn editor_close<R: Runtime>(window: WebviewWindow<R>, state: State<'_, AppState>) {
    if let Some(w) = state.editors.windows.lock().unwrap().get_mut(window.label()) {
        w.dirty = false;
    }
    let _ = window.close();
}

/// If an edit window opened from main window `owner` (any, for quitting)
/// holds unsaved YAML, focuses it and has it ask whether to discard (it closes
/// itself then). True when one was asked: the caller holds its close or quit,
/// and the user closes or quits again afterwards.
pub fn ask_dirty<R: Runtime>(app: &AppHandle<R>, owner: Option<&str>) -> bool {
    let state = app.state::<AppState>();
    let dirty = state
        .editors
        .windows
        .lock()
        .unwrap()
        .iter()
        .find(|(label, w)| w.dirty && owner.is_none_or(|o| state.owner(label).as_deref() == Some(o)))
        .map(|(l, _)| l.clone());
    let Some(window) = dirty.and_then(|label| app.get_webview_window(&label)) else { return false };
    let _ = window.unminimize();
    let _ = window.set_focus();
    let _ = window.emit_to(window.label(), edit_events::CLOSE_REQUESTED, ());
    true
}
