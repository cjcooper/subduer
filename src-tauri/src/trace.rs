//! The running trace: at most one, for the connected context. It records in
//! the background until Stop, a context switch, or quitting (which waits for
//! the file to be closed). `TRACE_CHANGED` tells the UI when one starts or
//! ends.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use subduer_core::trace::Trace;
use subduer_core::types::{
    TraceComparison, TraceDaily, TraceEnd, TraceKey, TraceReview, TraceScope, TraceSeries, TraceStatus, TraceSummary, TraceTargets,
};
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Runtime, State};
use tokio::sync::oneshot;

use crate::AppState;
use crate::commands::{CmdResult, connected_client, err};

/// Sent to every window with the running trace's status, or `null` once it ended.
pub const TRACE_CHANGED: &str = "trace-changed";
/// What a trace file is called in the Save dialog's file type list.
const FILE_TYPE: &str = "Trace";
pub const EXTENSION: &str = "trace";

#[derive(Default)]
pub struct TraceRunner {
    running: tokio::sync::Mutex<Option<Running>>,
    /// Mirrors `running.is_some()` for the window-close hold, which can't await.
    active: AtomicBool,
    /// Files this app recorded or opened, which Show in Finder may reveal.
    known: std::sync::Mutex<HashSet<PathBuf>>,
}

struct Running {
    trace: Arc<Trace>,
    stop: Option<oneshot::Sender<TraceEnd>>,
    done: JoinHandle<subduer_core::Result<TraceSummary>>,
}

impl TraceRunner {
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// Stops the running trace (if any) and waits for its file to be closed.
    pub async fn stop(&self, reason: TraceEnd) -> Option<Result<TraceSummary, String>> {
        let mut running = self.running.lock().await;
        let mut r = running.take()?;
        if let Some(stop) = r.stop.take() {
            let _ = stop.send(reason);
        }
        tracing::info!(?reason, path = %r.trace.status().path, "stopping trace");
        let result = match (&mut r.done).await {
            Ok(summary) => summary.map_err(|e| e.user_message()),
            Err(e) => Err(format!("the trace stopped unexpectedly: {e}")),
        };
        self.active.store(false, Ordering::SeqCst);
        Some(result)
    }

    async fn status(&self) -> Option<TraceStatus> {
        let running = self.running.lock().await;
        running.as_ref().map(|r| TraceStatus { stopping: r.stop.is_none(), ..r.trace.status() })
    }
}

/// Ends a trace the app is exiting under without having asked (on macOS,
/// the Dock's Quit or logging out), so its file still gets an end. Gives up
/// a little after the last pull's own limit; the file keeps what it has.
pub fn stop_on_exit(runner: &TraceRunner) {
    if !runner.is_active() {
        return;
    }
    let limit = subduer_core::trace::FINAL_PULL_TIMEOUT + std::time::Duration::from_secs(5);
    let stopped = tauri::async_runtime::block_on(tokio::time::timeout(limit, runner.stop(TraceEnd::Quit)));
    match stopped {
        Ok(Some(Err(e))) => tracing::warn!(error = %e, "closing the trace on exit failed"),
        Err(_) => tracing::warn!("closing the trace on exit timed out"),
        _ => {}
    }
}

/// Workloads and running pods in a namespace, for the start dialog.
#[tauri::command]
pub async fn trace_targets(state: State<'_, AppState>, namespace: String) -> CmdResult<TraceTargets> {
    let client = connected_client(&state).await?;
    subduer_core::trace::targets(&client, &namespace).await.map_err(err)
}

/// The native Save dialog for a new trace file, starting in Documents with
/// `suggested` as the name. `None` if cancelled. The dialog has already
/// asked about replacing an existing file.
#[tauri::command]
pub async fn choose_trace_file<R: Runtime>(app: AppHandle<R>, suggested: String) -> CmdResult<Option<String>> {
    use tauri::Manager;
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = oneshot::channel();
    let mut dialog = app.dialog().file().add_filter(FILE_TYPE, &[EXTENSION]).set_file_name(&suggested).set_title("Save trace as");
    if let Ok(dir) = app.path().document_dir() {
        dialog = dialog.set_directory(dir);
    }
    dialog.save_file(move |path| {
        let _ = tx.send(path);
    });
    let Some(path) = rx.await.ok().flatten() else { return Ok(None) };
    let path = path.into_path().map_err(|e| e.to_string())?;
    Ok(Some(with_extension(&path).display().to_string()))
}

/// Adds `.trace` unless the name already ends with it.
fn with_extension(path: &Path) -> PathBuf {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION)) {
        path.to_owned()
    } else {
        let mut name = path.as_os_str().to_owned();
        name.push(".");
        name.push(EXTENSION);
        PathBuf::from(name)
    }
}

/// Starts recording `scope` of `context` (which must be the connected one)
/// into `path`, replacing any file there, pulling Prometheus every
/// `pull_every` seconds (one of the Start dialog's choices).
#[tauri::command(rename_all = "snake_case")]
pub async fn start_trace<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    context: String,
    scope: TraceScope,
    path: String,
    pull_every: i64,
) -> CmdResult<TraceStatus> {
    let session_context = state.session.read().await.as_ref().map(|s| s.context.clone());
    if session_context.as_deref() != Some(context.as_str()) {
        return Err(format!("not connected to {context}"));
    }
    if !subduer_core::trace::PULL_EVERY_CHOICES.contains(&pull_every) {
        return Err(format!("can't pull every {pull_every} s"));
    }
    let client = connected_client(&state).await?;
    let runner = &state.traces;
    let mut running = runner.running.lock().await;
    if running.is_some() {
        return Err("a trace is already running; stop it first".into());
    }
    let path = with_extension(Path::new(&path));
    tracing::info!(%context, path = %path.display(), ?scope, pull_every, "starting trace");
    let trace = Arc::new(Trace::start(client, &context, scope, &path, true, pull_every).await.map_err(err)?);
    let (stop, stopped) = oneshot::channel();
    let recording = trace.clone();
    let done = tauri::async_runtime::spawn(async move {
        // A dropped sender (the app going away) ends it as a quit.
        recording.run(async { stopped.await.unwrap_or(TraceEnd::Quit) }).await
    });
    let status = trace.status();
    *running = Some(Running { trace, stop: Some(stop), done });
    runner.active.store(true, Ordering::SeqCst);
    runner.known.lock().unwrap().insert(path);
    drop(running);
    let _ = app.emit(TRACE_CHANGED, Some(&status));
    Ok(status)
}

#[tauri::command]
pub async fn trace_status(state: State<'_, AppState>) -> CmdResult<Option<TraceStatus>> {
    Ok(state.traces.status().await)
}

/// Stops the running trace, waiting (a few seconds at most) for its last
/// pull and the file to close.
#[tauri::command]
pub async fn stop_trace<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>, reason: TraceEnd) -> CmdResult<TraceSummary> {
    // Tell windows it's stopping while the last pull runs.
    if let Some(status) = state.traces.status().await {
        let _ = app.emit(TRACE_CHANGED, Some(&TraceStatus { stopping: true, ..status }));
    }
    let result = state.traces.stop(reason).await.ok_or_else(|| "no trace is running".to_owned())?;
    let _ = app.emit(TRACE_CHANGED, None::<TraceStatus>);
    result
}

/// Shows a trace file this app recorded in Finder (or its folder elsewhere).
#[tauri::command]
pub fn reveal_trace(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    let path = PathBuf::from(path);
    if !state.traces.known.lock().unwrap().contains(&path) {
        return Err("only traces recorded or opened in this session can be revealed".into());
    }
    crate::upload::reveal(&path)
}

/// The native Open dialog for a trace file, starting in Documents. `None` if cancelled.
#[tauri::command]
pub async fn choose_trace_to_open<R: Runtime>(app: AppHandle<R>) -> CmdResult<Option<String>> {
    use tauri::Manager;
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = oneshot::channel();
    let mut dialog = app.dialog().file().add_filter(FILE_TYPE, &[EXTENSION]).set_title("Open trace");
    if let Ok(dir) = app.path().document_dir() {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_file(move |path| {
        let _ = tx.send(path);
    });
    let Some(path) = rx.await.ok().flatten() else { return Ok(None) };
    Ok(Some(path.into_path().map_err(|e| e.to_string())?.display().to_string()))
}

/// Runs a file read off the async runtime.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> subduer_core::Result<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?.map_err(|e| e.user_message())
}

/// Reads a trace file for review: its header and the right-sizing table.
/// Works without a cluster; a trace still being recorded can be read too.
#[tauri::command]
pub async fn trace_review(state: State<'_, AppState>, path: String) -> CmdResult<TraceReview> {
    let path = PathBuf::from(path);
    let recording = state.traces.status().await.is_some_and(|s| Path::new(&s.path) == path);
    let read = path.clone();
    let review = blocking(move || subduer_core::trace_review::review(&read, recording)).await?;
    state.traces.known.lock().unwrap().insert(path);
    Ok(review)
}

/// One review row's usage between `start` and `end` (its whole span when
/// absent), in about `points` buckets.
#[tauri::command]
pub async fn trace_series(path: String, key: TraceKey, start: Option<f64>, end: Option<f64>, points: u32) -> CmdResult<TraceSeries> {
    blocking(move || subduer_core::trace_review::series(Path::new(&path), &key, start, end, points.clamp(10, 2000))).await
}

/// One review row's usage by local day and hour.
#[tauri::command]
pub async fn trace_daily(path: String, key: TraceKey) -> CmdResult<TraceDaily> {
    blocking(move || subduer_core::trace_review::daily(Path::new(&path), &key, &jiff::tz::TimeZone::system())).await
}

/// Saves (or, when blank, removes) a note in a trace file: on the whole
/// trace, or on `key`'s workload (its container is ignored).
#[tauri::command]
pub async fn trace_set_note(path: String, key: Option<TraceKey>, text: String) -> CmdResult<()> {
    let note_key = key.map_or_else(String::new, |k| subduer_core::trace_file::note_key(&k.namespace, &k.kind, &k.name));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
    blocking(move || subduer_core::trace_file::set_note(Path::new(&path), &note_key, &text, now)).await
}

/// Two or more trace files side by side, oldest data first.
#[tauri::command]
pub async fn trace_compare(state: State<'_, AppState>, paths: Vec<String>) -> CmdResult<TraceComparison> {
    let recording = state.traces.status().await.map(|s| PathBuf::from(s.path));
    let files: Vec<(PathBuf, bool)> = paths
        .into_iter()
        .map(PathBuf::from)
        .map(|p| {
            let rec = recording.as_ref() == Some(&p);
            (p, rec)
        })
        .collect();
    let known = files.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>();
    let comparison = blocking(move || subduer_core::trace_review::compare(&files)).await?;
    state.traces.known.lock().unwrap().extend(known);
    Ok(comparison)
}

/// The native Save dialog for an HTML report, next to its trace.
#[tauri::command]
pub async fn choose_report_file<R: Runtime>(app: AppHandle<R>, trace: String) -> CmdResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let trace = PathBuf::from(trace);
    let name = format!("{}-report.html", trace.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "trace".into()));
    let (tx, rx) = oneshot::channel();
    let mut dialog = app.dialog().file().add_filter("Web page", &["html"]).set_file_name(&name).set_title("Save report as");
    if let Some(dir) = trace.parent() {
        dialog = dialog.set_directory(dir);
    }
    dialog.save_file(move |path| {
        let _ = tx.send(path);
    });
    let Some(path) = rx.await.ok().flatten() else { return Ok(None) };
    let mut path = path.into_path().map_err(|e| e.to_string())?;
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm")) {
        path.set_extension("html");
    }
    Ok(Some(path.display().to_string()))
}

/// Writes a trace's review as a self-contained HTML page to `out` (the Save
/// dialog has already asked about replacing it). Returns where it went.
#[tauri::command]
pub async fn export_trace_report(state: State<'_, AppState>, path: String, out: String) -> CmdResult<String> {
    let recording = state.traces.status().await.is_some_and(|s| Path::new(&s.path) == Path::new(&path));
    let out_path = PathBuf::from(&out);
    let target = out_path.clone();
    blocking(move || {
        let tz = jiff::tz::TimeZone::system();
        let generated = jiff::Zoned::now().with_time_zone(tz.clone()).strftime("%a %-d %b %Y %H:%M").to_string();
        let html = subduer_core::trace_report::report_html(Path::new(&path), recording, &tz, &generated)?;
        std::fs::write(&target, html).map_err(|e| subduer_core::Error::Message(format!("can't write {}: {e}", target.display())))
    })
    .await?;
    tracing::info!(out = %out_path.display(), "trace report written");
    state.traces.known.lock().unwrap().insert(out_path);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_files_get_their_extension() {
        assert_eq!(with_extension(Path::new("/tmp/a.trace")), PathBuf::from("/tmp/a.trace"));
        assert_eq!(with_extension(Path::new("/tmp/a.TRACE")), PathBuf::from("/tmp/a.TRACE"));
        assert_eq!(with_extension(Path::new("/tmp/a")), PathBuf::from("/tmp/a.trace"));
        assert_eq!(with_extension(Path::new("/tmp/a.db")), PathBuf::from("/tmp/a.db.trace"));
    }
}
