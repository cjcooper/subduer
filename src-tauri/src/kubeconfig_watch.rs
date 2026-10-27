//! Notices kubeconfig changes made outside the app (`kubectl config
//! delete-context`, an editor, a cloud's CLI) and keeps the windows on
//! contexts that exist: a window whose context went moves to kubectl's
//! current context (else the first), and when none are left all main
//! windows but one close and that one offers Add Cluster.
//!
//! The files are checked every [`EVERY`]; a change is acted on only once it
//! has settled (the same on the next check, and no `<file>.lock`), so a file
//! another program is halfway through writing isn't read as "no contexts".

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use subduer_core::types::{ContextRemoved, cluster_events};
use subduer_core::{add_cluster, manage_clusters};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::{AppState, MAIN_LABEL};

const EVERY: Duration = Duration::from_secs(2);

/// Each file's modified time and size (`None` if missing), and whether its lock is held.
type Signature = Vec<(PathBuf, Option<(SystemTime, u64)>, bool)>;

fn signature(files: &[PathBuf]) -> Signature {
    files
        .iter()
        .map(|f| {
            let meta = std::fs::metadata(f).ok().and_then(|m| Some((m.modified().ok()?, m.len())));
            let locked = PathBuf::from(format!("{}.lock", f.display())).exists();
            (f.clone(), meta, locked)
        })
        .collect()
}

/// Watches until the app quits.
pub async fn run<R: Runtime>(app: AppHandle<R>) {
    let files = || add_cluster::kubeconfig_files().unwrap_or_default();
    let mut seen = signature(&files());
    let mut pending = false;
    loop {
        tokio::time::sleep(EVERY).await;
        let now = signature(&files());
        if now != seen {
            seen = now;
            pending = true;
            continue;
        }
        if !pending || now.iter().any(|(_, _, locked)| *locked) {
            continue;
        }
        pending = false;
        settle(&app, &files()).await;
    }
}

/// The kubeconfig changed: every window's context menu follows, and windows
/// on a context that's gone move.
async fn settle<R: Runtime>(app: &AppHandle<R>, files: &[PathBuf]) {
    // Unreadable (being rewritten, or broken): wait for the next change.
    let Ok(list) = manage_clusters::list(files) else { return };
    let names: Vec<String> = list.contexts.into_iter().map(|c| c.name).collect();
    let state = app.state::<AppState>();
    for window in state.main_windows() {
        let _ = app.emit_to(&window.label, cluster_events::CONTEXTS_CHANGED, ());
    }
    let windows: Vec<(String, Option<String>)> =
        state.main_windows().into_iter().map(|w| (w.label.clone(), w.chosen.lock().unwrap().clone())).collect();
    let focused = app.webview_windows().into_iter().find(|(_, w)| w.is_focused().unwrap_or(false)).map(|(l, _)| l);
    let plan = plan(&windows, &names, list.current.as_deref(), focused.as_deref());
    for label in &plan.close {
        tracing::info!(window = %label, "closing: the kubeconfig has no contexts");
        if let Some(w) = app.get_webview_window(label) {
            // Asks first about unsaved edits or a running trace, as any close does.
            let _ = w.close();
        }
    }
    if let Some(label) = &plan.empty {
        if let Some(w) = state.window(label) {
            w.stop_watch();
            *w.session.write().await = None;
            *w.chosen.lock().unwrap() = None;
        }
        let _ = app.emit_to(label, cluster_events::NO_CONTEXTS, ());
        crate::add_cluster::in_use_changed(app);
    }
    for (label, moved) in plan.switch {
        tracing::info!(window = %label, removed = %moved.removed, to = %moved.switch_to, "context removed from the kubeconfig");
        let _ = app.emit_to(&label, cluster_events::CONTEXT_REMOVED, moved);
    }
}

/// What to do to the main windows (label, context picked) now that the
/// kubeconfig has `names`.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Plan {
    /// Windows to close: there's nothing left to show in them.
    pub close: Vec<String>,
    /// The one window kept when there are no contexts.
    pub empty: Option<String>,
    /// Windows whose context went, and where they go.
    pub switch: Vec<(String, ContextRemoved)>,
}

pub(crate) fn plan(windows: &[(String, Option<String>)], names: &[String], current: Option<&str>, focused: Option<&str>) -> Plan {
    let mut plan = Plan::default();
    if windows.is_empty() {
        return plan;
    }
    if names.is_empty() {
        // The focused window stays, else the first one, else any.
        let labels: Vec<&str> = windows.iter().map(|(l, _)| l.as_str()).collect();
        let keep = focused.filter(|f| labels.contains(f)).or_else(|| labels.iter().copied().find(|l| *l == MAIN_LABEL)).unwrap_or(labels[0]);
        plan.close = labels.iter().filter(|l| **l != keep).map(|l| (*l).to_owned()).collect();
        plan.empty = Some(keep.to_owned());
        return plan;
    }
    let to = current.filter(|c| names.iter().any(|n| n == c)).unwrap_or(&names[0]);
    for (label, chosen) in windows {
        if let Some(removed) = chosen.as_ref().filter(|c| !names.contains(c)) {
            plan.switch.push((label.clone(), ContextRemoved { removed: removed.clone(), switch_to: to.to_owned() }));
        }
    }
    plan
}

#[cfg(test)]
#[path = "kubeconfig_watch_tests.rs"]
mod tests;
