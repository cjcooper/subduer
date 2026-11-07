//! Capture windows: a native window per packet capture of one pod. Starting
//! one attaches an ephemeral capture container (gated: it changes the pod),
//! then the capture runs in the background, writing a pcap file where the
//! user chose (for Wireshark later) and sending the window an update each second: phase,
//! bytes, and the TLS connections seen so far.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use subduer_core::capture;
use subduer_core::kube::Client;
use subduer_core::tls_scan::Scanner;
use subduer_core::types::{CapturePhase, CaptureRequest, CaptureSession, CaptureUpdate, TlsScan};
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager, Runtime, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::{AppState, Win};
use crate::commands::{CmdResult, connected_client};

/// Window labels look like `capture-3`; capabilities grant `capture-*`.
pub const LABEL_PREFIX: &str = "capture-";
/// How often a running capture tells its window how it's going.
const UPDATE_EVERY: Duration = Duration::from_secs(1);
/// How long the capture container may take to start (pulling its image).
const START_LIMIT: Duration = Duration::from_secs(180);
/// The container sleeps this much longer than the capture, so it's there to the end.
const GRACE_SECS: u32 = 60;
/// How long a closed window's capture may take to finish its file.
const FINISH_GRACE: Duration = Duration::from_secs(5);

struct CaptureWindow {
    client: Client,
    session: CaptureSession,
    /// The latest update, shared with the running task.
    update: Arc<Mutex<CaptureUpdate>>,
    task: JoinHandle<()>,
}

#[derive(Default)]
pub struct CaptureWindows {
    windows: Mutex<HashMap<String, CaptureWindow>>,
    next_id: AtomicU64,
}

impl CaptureWindows {
    /// The window closed: interrupt tcpdump if it's still running, and forget it.
    pub(crate) fn closed(&self, label: &str) {
        let Some(w) = self.windows.lock().unwrap().remove(label) else { return };
        let running = matches!(w.update.lock().unwrap().phase, CapturePhase::Starting | CapturePhase::Capturing | CapturePhase::Stopping);
        if running {
            let (client, s) = (w.client.clone(), w.session.clone());
            tauri::async_runtime::spawn(async move {
                let _ = capture::stop(&client, &s.namespace, &s.pod, &s.container).await;
            });
        }
        // Give tcpdump a moment to finish the file, then stop reading.
        let task = w.task;
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(FINISH_GRACE).await;
            task.abort();
        });
    }
}

pub(crate) fn update(phase: CapturePhase) -> CaptureUpdate {
    CaptureUpdate { phase, message: None, image_id: None, started: None, ends: None, bytes: 0, scan: TlsScan::default() }
}

fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or_default()
}

/// What a capture file is called in the Save dialog's file type list.
const FILE_TYPE: &str = "Packet capture";
const EXTENSION: &str = "pcap";

/// Adds `.pcap` unless the name already ends with it.
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

/// The native Save dialog for a new capture, starting in Documents with
/// `suggested` as the name. `None` if cancelled. The dialog has already
/// asked about replacing an existing file.
#[tauri::command]
pub async fn choose_capture_file<R: Runtime>(app: AppHandle<R>, suggested: String, dialogs: State<'_, crate::system::Dialogs<R>>) -> CmdResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app.dialog().file().add_filter(FILE_TYPE, &[EXTENSION]).set_file_name(&suggested).set_title("Save capture as");
    let dialog = crate::system::starting_in(dialog, app.path().document_dir().ok());
    let path = crate::system::ask(&dialogs, dialog, crate::system::Ask::Save).await?;
    Ok(path.map(|path| with_extension(&path).display().to_string()))
}

/// Attaches a capture container to the pod (after a dry run, so a refusal
/// shows in the dialog) and opens the capture's window, which follows it.
#[tauri::command]
pub async fn start_capture<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>, win: Win, request: CaptureRequest) -> CmdResult<String> {
    let client = connected_client(&win).await?;
    let context = win.session.read().await.as_ref().map(|s| s.context.clone()).ok_or("not connected to a cluster")?;
    win.gate.ensure_writable(&context).map_err(crate::commands::err)?;
    let CaptureRequest { namespace, pod, image, filter, secs, path } = request.clone();
    if secs == 0 || secs > capture::MAX_SECS {
        return Err(format!("a capture lasts 1 to {} minutes", capture::MAX_SECS / 60));
    }
    if matches!(&filter.traffic, capture::Traffic::Custom(e) if e.trim().is_empty()) {
        return Err("type a filter, or choose one of the others".into());
    }
    if path.trim().is_empty() {
        return Err("choose where to save the capture".into());
    }
    let gate = &win.gate;
    let lives = secs + GRACE_SECS;
    capture::attach(&client, gate, &context, &namespace, &pod, &image, lives, true).await.map_err(crate::commands::user)?;
    // The file first: a folder that can't be written fails here, before the pod is changed.
    let path = with_extension(Path::new(path.trim()));
    std::fs::File::create(&path).map_err(|e| format!("can't write {}: {e}", path.display()))?;
    let container = match capture::attach(&client, gate, &context, &namespace, &pod, &image, lives, false).await {
        Ok(name) => name,
        Err(e) => {
            // Nothing will be written to it.
            let _ = std::fs::remove_file(&path);
            return Err(e.user_message());
        }
    };
    // Without them connections are listed as neither in nor out, and an inbound
    // filter takes the whole port; the capture still works.
    let ips = capture::pod_ips(&client, &namespace, &pod).await.unwrap_or_default();

    let id = state.captures.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    let label = format!("{LABEL_PREFIX}{id}");
    let session = CaptureSession {
        context: context.clone(),
        namespace: namespace.clone(),
        pod: pod.clone(),
        container: container.clone(),
        image: image.trim().to_owned(),
        expression: capture::expression(&filter, &ips),
        filter,
        secs,
        path: path.display().to_string(),
        pod_ips: ips,
        event: label.clone(),
        update: update(CapturePhase::Starting),
    };
    let shared = Arc::new(Mutex::new(session.update.clone()));
    let timing = Timing { start_limit: START_LIMIT, update_every: UPDATE_EVERY };
    let task = tauri::async_runtime::spawn(run(app.clone(), client.clone(), session.clone(), shared.clone(), timing));
    state.captures.windows.lock().unwrap().insert(label.clone(), CaptureWindow { client, session, update: shared, task });
    tracing::info!(%context, %namespace, %pod, %container, %label, "capture started");

    // Closing it stops the capture (`CaptureWindows::closed`).
    let title = format!("Capture · {pod} · {context}");
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(format!("index.html?capture={label}").into()))
        .title(&title)
        .inner_size(1100.0, 720.0)
        .min_inner_size(560.0, 360.0)
        .disable_drag_drop_handler()
        .build()
        .map_err(crate::commands::err)?;
    Ok(label)
}

/// How long a capture waits for its container, and how often it reports.
pub(crate) struct Timing {
    pub start_limit: Duration,
    pub update_every: Duration,
}

/// The capture: wait for the container, run tcpdump into the file and the
/// scanner, and keep the window told.
pub(crate) async fn run<R: Runtime>(app: AppHandle<R>, client: Client, s: CaptureSession, shared: Arc<Mutex<CaptureUpdate>>, timing: Timing) {
    let send = |u: &CaptureUpdate| {
        *shared.lock().unwrap() = u.clone();
        let _ = app.emit_to(&s.event, &s.event, u.clone());
    };
    let mut u = update(CapturePhase::Starting);
    let fail = |mut u: CaptureUpdate, message: String| {
        u.phase = CapturePhase::Failed;
        u.message = Some(message);
        u
    };
    match capture::wait_running(&client, &s.namespace, &s.pod, &s.container, timing.start_limit).await {
        Ok(image_id) => u.image_id = Some(image_id),
        Err(e) => return send(&fail(u, e.user_message())),
    }
    let mut file = match tokio::fs::File::create(&s.path).await {
        Ok(f) => f,
        Err(e) => return send(&fail(u, format!("can't write {}: {e}", s.path))),
    };
    let now = unix_now();
    (u.phase, u.started, u.ends) = (CapturePhase::Capturing, Some(now), Some(now + f64::from(s.secs)));
    send(&u);
    let ips: Vec<IpAddr> = s.pod_ips.iter().filter_map(|i| i.parse().ok()).collect();
    let mut scanner = Scanner::new(ips);
    let mut last = Instant::now();
    let mut bytes = 0u64;
    let result = capture::run(&client, &s.namespace, &s.pod, &s.container, &s.expression, s.secs, &mut file, |piece| {
        bytes += piece.len() as u64;
        let _ = scanner.feed(piece);
        if last.elapsed() >= timing.update_every {
            last = Instant::now();
            let mut now = u.clone();
            // Stop may have been asked for meanwhile.
            now.phase = shared.lock().unwrap().phase;
            (now.bytes, now.scan) = (bytes, scanner.result().clone());
            send(&now);
        }
    })
    .await;
    (u.bytes, u.scan) = (bytes, scanner.result().clone());
    match result {
        Ok(done) => {
            u.phase = CapturePhase::Done;
            u.message = Some(done.messages).filter(|m| !m.is_empty());
            send(&u);
        }
        Err(e) => send(&fail(u, e.user_message())),
    }
}

/// Called by a capture window once it's listening: its capture, and how it's going.
#[tauri::command]
pub fn capture_session<R: Runtime>(window: WebviewWindow<R>, state: State<'_, AppState>) -> CmdResult<CaptureSession> {
    let windows = state.captures.windows.lock().unwrap();
    let w = windows.get(window.label()).ok_or("this capture window is no longer connected")?;
    let mut session = w.session.clone();
    session.update = w.update.lock().unwrap().clone();
    Ok(session)
}

/// Ends the window's capture early; tcpdump finishes the file.
#[tauri::command]
pub async fn stop_capture<R: Runtime>(app: AppHandle<R>, window: WebviewWindow<R>, state: State<'_, AppState>) -> CmdResult<()> {
    let (client, s, shared) = {
        let windows = state.captures.windows.lock().unwrap();
        let w = windows.get(window.label()).ok_or("this capture window is no longer connected")?;
        (w.client.clone(), w.session.clone(), w.update.clone())
    };
    let stopping = {
        let mut u = shared.lock().unwrap();
        if u.phase != CapturePhase::Capturing {
            return Ok(());
        }
        u.phase = CapturePhase::Stopping;
        u.clone()
    };
    let _ = app.emit_to(&s.event, &s.event, stopping);
    capture::stop(&client, &s.namespace, &s.pod, &s.container).await.map_err(crate::commands::user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::fake::{Fake, success};
    use subduer_core::types::{CaptureFilter, CaptureTraffic};

    /// Adds capture window `label` in `phase`, reading forever.
    fn add(state: &AppState, label: &str, client: Client, phase: CapturePhase) {
        let session = CaptureSession {
            context: "acme-uat".into(),
            namespace: "shop".into(),
            pod: "api-1".into(),
            container: "cap".into(),
            image: "acme/netshoot:1".into(),
            expression: String::new(),
            filter: CaptureFilter { traffic: CaptureTraffic::All, handshakes_only: false },
            secs: 60,
            path: "/tmp/cap.pcap".into(),
            pod_ips: vec![],
            event: label.into(),
            update: update(phase),
        };
        let task = tauri::async_runtime::spawn(std::future::pending());
        let w = CaptureWindow { client, update: Arc::new(Mutex::new(session.update.clone())), session, task };
        state.captures.windows.lock().unwrap().insert(label.into(), w);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stopping_and_closing_interrupt_tcpdump() {
        let fake = Fake::start();
        // Stop, then the closed window's stop.
        fake.exec(vec![success()]).exec(vec![success()]);
        let app = tauri::test::mock_app();
        app.manage(AppState::default());
        let state = app.state::<AppState>();
        add(&state, "capture-1", fake.client(), CapturePhase::Capturing);
        let window = WebviewWindowBuilder::new(&app, "capture-1", WebviewUrl::default()).build().unwrap();
        stop_capture(app.handle().clone(), window.clone(), app.state()).await.unwrap();
        assert_eq!(capture_session(window.clone(), app.state()).unwrap().update.phase, CapturePhase::Stopping);
        let execs = || fake.requests.lock().unwrap().iter().filter(|r| r.contains("/exec")).count();
        assert_eq!(execs(), 1);
        // Already stopping: nothing more to do.
        stop_capture(app.handle().clone(), window.clone(), app.state()).await.unwrap();
        assert_eq!(execs(), 1);
        // Closing a running capture interrupts it, and stops reading after a grace period.
        state.captures.closed("capture-1");
        assert!(capture_session(window, app.state()).is_err());
        tokio::time::sleep(FINISH_GRACE + Duration::from_millis(300)).await;
        assert_eq!(execs(), 2);
        // A finished capture closing interrupts nothing; nor does one already gone.
        add(&state, "capture-2", fake.client(), CapturePhase::Done);
        state.captures.closed("capture-2");
        state.captures.closed("capture-2");
        assert_eq!(execs(), 2);
    }

    #[test]
    fn capture_files_get_their_extension() {
        assert_eq!(with_extension(Path::new("/tmp/a.pcap")), PathBuf::from("/tmp/a.pcap"));
        assert_eq!(with_extension(Path::new("/tmp/a.PCAP")), PathBuf::from("/tmp/a.PCAP"));
        assert_eq!(with_extension(Path::new("/tmp/a")), PathBuf::from("/tmp/a.pcap"));
    }
}
