//! Sessions that need a real connection (connecting from the kubeconfig,
//! shells, downloads, captures), the windows that follow them, and the
//! Manage Clusters commands that read and write the kubeconfig.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use subduer_core::types::{CaptureFilter, CapturePhase, CaptureRequest, CaptureSession, CaptureTraffic, CaptureUpdate, EditTarget, NewCluster};
use tauri::Listener;

use super::fake::{Fake, Step, failure, out, success};
use super::*;

/// A kubeconfig with one context, `name`, on `server`.
fn kubeconfig_for(name: &str, server: &str) -> String {
    format!(
        "apiVersion: v1\nkind: Config\ncurrent-context: {name}\n\
         clusters:\n- name: {name}\n  cluster: {{server: '{server}'}}\n\
         contexts:\n- name: {name}\n  context: {{cluster: {name}, user: me, namespace: shop}}\n\
         users:\n- name: me\n  user: {{token: fake}}\n"
    )
}

#[tokio::test]
async fn connecting_reads_the_kubeconfig_and_discovers_the_cluster() {
    let fake = Fake::start();
    fake.cluster();
    let _kubeconfig = kubeconfig_file(&kubeconfig_for(CTX, &fake.url));
    let app = app();
    let in_use = Arc::new(Mutex::new(0));
    let count = in_use.clone();
    app.listen_any(subduer_core::types::cluster_events::IN_USE_ELSEWHERE, move |_| *count.lock().unwrap() += 1);
    // The window gets the context in its title.
    WebviewWindowBuilder::new(&app, MAIN_LABEL, WebviewUrl::default()).build().unwrap();
    let info = commands::connect(app.handle().clone(), w(&app), CTX.into()).await.unwrap();
    assert_eq!((info.context.as_str(), info.namespaces.as_slice()), (CTX, ["shop".to_owned()].as_slice()));
    assert!(info.kinds.iter().any(|k| k.kind.kind == "Pod"));
    assert_eq!(w(&app).session.read().await.as_ref().unwrap().context, CTX);
    // Connecting again to the same context doesn't tell the other windows.
    let before = *in_use.lock().unwrap();
    commands::connect(app.handle().clone(), w(&app), CTX.into()).await.unwrap();
    assert_eq!(*in_use.lock().unwrap(), before);
}

#[tokio::test]
async fn a_cluster_that_stops_answering_midway_times_out() {
    let fake = Fake::start();
    fake.on("GET", "/version", json!({"major": "1", "minor": "31", "gitVersion": "v1.31.0"})).hang("/api").hang("/apis");
    let app = app();
    let err = commands::open_session(app.handle(), &w(&app), CTX.into(), fake.client(), 0, Duration::from_millis(300)).await.unwrap_err();
    assert!(err.contains("stopped answering"), "{err}");
    assert!(w(&app).session.read().await.is_none());
}

#[tokio::test]
async fn a_connect_overtaken_during_discovery_gives_way() {
    let fake = Fake::start();
    fake.cluster();
    let app = app();
    let win = w(&app);
    let bump = win.clone();
    // Another context is picked while the namespaces are being listed.
    fake.hook(move |route| {
        if route == "GET /api/v1/namespaces" {
            bump.connect_attempt.fetch_add(1, Ordering::SeqCst);
        }
    });
    let err = commands::open_session(app.handle(), &win, CTX.into(), fake.client(), 0, commands::DISCOVER_TIMEOUT).await.unwrap_err();
    assert_eq!(err, commands::SUPERSEDED);
    assert!(win.session.read().await.is_none());
}

/// A mock app whose session is on the fake server.
async fn on_fake(fake: &Fake) -> App<MockRuntime> {
    let app = app();
    *w(&app).session.write().await = Some(Session { context: CTX.into(), client: fake.client(), history: Default::default() });
    unlock(&app);
    app
}

fn shell_target() -> ExecTarget {
    ExecTarget { namespace: "shop".into(), pod: "api-1".into(), container: "app".into() }
}

/// Waits (a few seconds at most) for `done`.
async fn until(what: &str, done: impl Fn() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn shells_close_when_the_user_leaves_and_stay_when_they_fail() {
    let fake = Fake::start();
    // A second window's session; then this one's first is replaced by a
    // reconnect, the next fails, and the last exits.
    let open = || vec![Step::Wait(|s| s.closed)];
    fake.exec(open()).exec(open()).exec(vec![out("$ "), failure("container not found")]).exec(vec![out("$ "), success()]);
    let app = on_fake(&fake).await;
    let output = Arc::new(Mutex::new(Vec::new()));
    let into = output.clone();
    app.listen_any("exec-1", move |e| into.lock().unwrap().push(e.payload().to_owned()));
    crate::exec::open_exec(app.handle().clone(), app.state(), w(&app), shell_target()).await.unwrap();
    // Another session on the same container is numbered.
    crate::exec::open_exec(app.handle().clone(), app.state(), w(&app), shell_target()).await.unwrap();
    assert_eq!(crate::exec::exec_start(app.get_webview_window("exec-2").unwrap(), app.state(), w(&app), 80, 24).unwrap().title, "app · shop/api-1 · acme-uat · Shell 2");
    let window = app.get_webview_window("exec-1").unwrap();
    crate::exec::exec_start(window.clone(), app.state(), w(&app), 80, 24).unwrap();
    until("the first session", || fake.requests.lock().unwrap().iter().filter(|r| r.contains("/exec")).count() >= 2).await;
    crate::exec::exec_start(window.clone(), app.state(), w(&app), 80, 24).unwrap();
    until("the failure", || output.lock().unwrap().iter().any(|o| o.contains("container not found"))).await;
    crate::exec::exec_start(window.clone(), app.state(), w(&app), 100, 30).unwrap();
    until("the last session", || fake.requests.lock().unwrap().iter().filter(|r| r.contains("/exec")).count() >= 4).await;
    // Leaving the shell closes the window, which ends its session.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!output.lock().unwrap().iter().any(|o| o.contains("Shell ended")));
    // Window labels that are taken: nothing is left half-made.
    WebviewWindowBuilder::new(&app, "exec-3", WebviewUrl::default()).build().unwrap();
    assert!(crate::exec::open_exec(app.handle().clone(), app.state(), w(&app), shell_target()).await.is_err());
    assert!(app.state::<AppState>().owner("exec-3").is_none());
}

#[tokio::test]
async fn logs_windows_report_a_stream_that_fails() {
    let api = MockApi::new();
    let app = connected(&api).await;
    let notices = Arc::new(Mutex::new(Vec::new()));
    let into = notices.clone();
    app.listen_any("logs-1", move |e| into.lock().unwrap().push(e.payload().to_owned()));
    let target = LogTarget { namespace: "shop".into(), source: LogSource::Workload { kind: deployment(), name: "gone".into() }, filter: None };
    crate::logs::open_logs(app.handle().clone(), app.state(), w(&app), target.clone()).await.unwrap();
    let window = app.get_webview_window("logs-1").unwrap();
    crate::logs::logs_start(window.clone(), app.state(), false).unwrap();
    // Restarting replaces the stream.
    crate::logs::logs_start(window, app.state(), true).unwrap();
    until("the notice", || notices.lock().unwrap().iter().any(|n| n.contains("isn't faked"))).await;
    // A label that's taken: nothing is left half-made.
    WebviewWindowBuilder::new(&app, "logs-2", WebviewUrl::default()).build().unwrap();
    let other = LogTarget { filter: Some("error".into()), ..target };
    assert!(crate::logs::open_logs(app.handle().clone(), app.state(), w(&app), other).await.is_err());
    assert!(app.state::<AppState>().owner("logs-2").is_none());
}

#[tokio::test]
async fn edit_windows_that_cannot_open_leave_nothing_behind() {
    let app = connected(&MockApi::new()).await;
    WebviewWindowBuilder::new(&app, "edit-1", WebviewUrl::default()).build().unwrap();
    let target = EditTarget { kind: kind("", "v1", "ConfigMap", "configmaps", true), namespace: Some("shop".into()), name: "app".into() };
    assert!(crate::editor::open_editor(app.handle().clone(), app.state(), w(&app), target).await.is_err());
    assert!(app.state::<AppState>().owner("edit-1").is_none());
    assert!(!crate::editor::ask_dirty(app.handle(), None));
}

#[tokio::test]
async fn closing_a_main_window_closes_every_kind_of_child() {
    let app = app();
    let state = app.state::<AppState>();
    for child in ["logs-7", "exec-7", "edit-7", "capture-7", "logs-8"] {
        if child != "logs-8" {
            WebviewWindowBuilder::new(&app, child, WebviewUrl::default()).build().unwrap();
        }
        state.adopt(child, MAIN_LABEL);
    }
    // Destroyed: all but the capture window (not one of its kind) and the
    // logs window whose webview never opened.
    crate::logs::close_all(app.handle(), MAIN_LABEL);
    assert_eq!(state.children(MAIN_LABEL).len(), 5, "forgotten only once they're gone");
}

fn no_downloads() -> tauri::Result<PathBuf> {
    Err(tauri::Error::UnknownPath)
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_land_in_the_downloads_folder() {
    let fake = Fake::start();
    fake.exec(vec![out("hello"), success()]);
    let app = on_fake(&fake).await;
    let dir = std::env::temp_dir().join(format!("subduer-tauri-downloads-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let into = dir.clone();
    let got = crate::upload::download_into(&w(&app), move || Ok(into), "shop", "api-1", "app", "/etc/motd").await.unwrap();
    assert_eq!(got.bytes, 5);
    assert_eq!(std::fs::read_to_string(&got.saved_to).unwrap(), "hello");
    // No Downloads folder, and not connected at all.
    let none = crate::upload::download_into(&w(&app), no_downloads, "shop", "api-1", "app", "/etc/motd").await;
    assert!(none.unwrap_err().starts_with("no Downloads folder"));
    let other = self::app();
    assert!(crate::upload::download_into(&w(&other), no_downloads, "shop", "api-1", "app", "/x").await.is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn port_forwards_open_in_the_browser() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/pods/api-1", pod("shop", "api-1", "node-a", json!({
        "spec": {"containers": [{"name": "app", "image": "acme/app:1", "ports": [{"containerPort": 8080}]}]}})));
    let app = connected(&api).await;
    let request = ForwardRequest { namespace: "shop".into(), source: ForwardSource::Pod { pod: "api-1".into() }, remote_port: 8080, local_port: 0 };
    let info = crate::forward::start_forward(app.handle().clone(), w(&app), request).await.unwrap();
    crate::forward::open_forward(app.state(), w(&app), info.id).unwrap();
    assert!(launched_with(&format!("http://127.0.0.1:{}/", info.local_port)));
    // Stopping one that's already gone just sends the list again.
    crate::forward::stop_forward(app.handle().clone(), w(&app), info.id + 100);
    assert_eq!(crate::forward::list_forwards(w(&app)).len(), 1);
}

fn capture_request(path: &std::path::Path) -> CaptureRequest {
    CaptureRequest {
        namespace: "shop".into(),
        pod: "api-1".into(),
        image: "acme/netshoot:1".into(),
        filter: CaptureFilter { traffic: CaptureTraffic::All, handshakes_only: false },
        secs: 60,
        path: path.display().to_string(),
    }
}

#[tokio::test]
async fn a_capture_starts_and_opens_its_window() {
    let api = MockApi::new();
    let path = "/api/v1/namespaces/shop/pods/api-1/ephemeralcontainers";
    api.on("PATCH", path, pod("shop", "api-1", "node-a", json!({})));
    let app = connected(&api).await;
    unlock(&app);
    let dir = std::env::temp_dir().join(format!("subduer-tauri-capture-start-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let label = crate::capture::start_capture(app.handle().clone(), app.state(), w(&app), capture_request(&dir.join("api"))).await.unwrap();
    assert_eq!(label, "capture-1");
    assert!(dir.join("api.pcap").exists());
    let session = crate::capture::capture_session(app.get_webview_window(&label).unwrap(), app.state()).unwrap();
    assert_eq!((session.pod.as_str(), session.event.as_str()), ("api-1", label.as_str()));
    // The pod can't be read here, so the capture fails while starting; there's nothing to stop.
    until("the capture to fail", || {
        let session = crate::capture::capture_session(app.get_webview_window(&label).unwrap(), app.state()).unwrap();
        session.update.phase == CapturePhase::Failed
    })
    .await;
    crate::capture::stop_capture(app.handle().clone(), app.get_webview_window(&label).unwrap(), app.state()).await.unwrap();
    // A window the app doesn't know about isn't a capture.
    let stray = WebviewWindowBuilder::new(&app, "capture-99", WebviewUrl::default()).build().unwrap();
    assert!(crate::capture::capture_session(stray.clone(), app.state()).is_err());
    assert!(crate::capture::stop_capture(app.handle().clone(), stray, app.state()).await.is_err());
    // A folder that can't be written fails before the pod is changed.
    let nowhere = capture_request(&dir.join("missing").join("api"));
    let err = crate::capture::start_capture(app.handle().clone(), app.state(), w(&app), nowhere).await.unwrap_err();
    assert!(err.starts_with("can't write"), "{err}");
    // A label that's taken: the capture runs, but its window can't open.
    WebviewWindowBuilder::new(&app, "capture-2", WebviewUrl::default()).build().unwrap();
    assert!(crate::capture::start_capture(app.handle().clone(), app.state(), w(&app), capture_request(&dir.join("b"))).await.is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A capture of `api-1` by container `cap`, into `path`.
fn capture_session(path: &std::path::Path) -> CaptureSession {
    CaptureSession {
        context: CTX.into(),
        namespace: "shop".into(),
        pod: "api-1".into(),
        container: "cap".into(),
        image: "acme/netshoot:1".into(),
        expression: String::new(),
        filter: CaptureFilter { traffic: CaptureTraffic::All, handshakes_only: false },
        secs: 60,
        path: path.display().to_string(),
        pod_ips: vec!["10.0.0.7".into(), "not-an-ip".into()],
        event: "capture-5".into(),
        update: crate::capture::update(CapturePhase::Starting),
    }
}

/// Runs one capture against `fake` and returns its last update.
async fn run_capture(fake: &Fake, path: &std::path::Path, update_every: Duration) -> CaptureUpdate {
    let app = app();
    let session = capture_session(path);
    let shared = Arc::new(Mutex::new(session.update.clone()));
    let timing = crate::capture::Timing { start_limit: Duration::from_millis(10), update_every };
    crate::capture::run(app.handle().clone(), fake.client(), session, shared.clone(), timing).await;
    shared.lock().unwrap().clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn captures_run_tcpdump_into_the_file() {
    let dir = std::env::temp_dir().join(format!("subduer-tauri-capture-run-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.pcap");
    let fake = Fake::start();
    // The pod can't be read: it fails while starting.
    let u = run_capture(&fake, &file, Duration::ZERO).await;
    assert_eq!(u.phase, CapturePhase::Failed);
    let running = pod("shop", "api-1", "node-a", json!({"status": {"ephemeralContainerStatuses": [
        {"name": "cap", "image": "acme/netshoot:1", "imageID": "acme/netshoot@sha256:1", "ready": false, "restartCount": 0,
         "state": {"running": {}}}]}}));
    fake.on("GET", "/api/v1/namespaces/shop/pods/api-1", running);
    // A folder that can't be written.
    let u = run_capture(&fake, &dir.join("missing").join("a.pcap"), Duration::ZERO).await;
    assert!(u.message.unwrap().starts_with("can't write"));
    // tcpdump can't be started (no exec allowed here).
    let u = run_capture(&fake, &file, Duration::ZERO).await;
    assert_eq!(u.phase, CapturePhase::Failed);
    // Two pieces, each reported as it comes, then tcpdump's own summary.
    fake.exec(vec![out("pcap-1"), out("pcap-2"), Step::Out(2, b"2 packets captured".to_vec()), success()]);
    let u = run_capture(&fake, &file, Duration::ZERO).await;
    assert_eq!((u.phase, u.bytes, u.image_id.as_deref()), (CapturePhase::Done, 12, Some("acme/netshoot@sha256:1")));
    assert_eq!(u.message.as_deref(), Some("2 packets captured"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "pcap-1pcap-2");
    // Reporting less often; nothing said.
    fake.exec(vec![out("pcap"), success()]);
    let u = run_capture(&fake, &file, Duration::from_secs(3600)).await;
    assert_eq!((u.phase, u.bytes, u.message), (CapturePhase::Done, 4, None));
    std::fs::remove_dir_all(&dir).unwrap();
}

const CLUSTERS: &str = "apiVersion: v1\nkind: Config\ncurrent-context: acme-uat\n\
    clusters:\n- name: uat\n  cluster: {server: 'https://127.0.0.1:1'}\n- name: prd-cluster\n  cluster: {server: 'https://127.0.0.1:2'}\n\
    contexts:\n- name: acme-uat\n  context: {cluster: uat, user: me}\n- name: lab\n  context: {cluster: prd-cluster, user: me}\n\
    users:\n- name: me\n  user: {token: fake}\n";

#[tokio::test]
async fn manage_clusters_reads_and_writes_the_kubeconfig() {
    use crate::add_cluster;
    let (_turn, path) = kubeconfig_file(CLUSTERS);
    let app = app();
    let changed = Arc::new(Mutex::new(0));
    let count = changed.clone();
    app.listen_any(subduer_core::types::cluster_events::CONTEXTS_CHANGED, move |_| *count.lock().unwrap() += 1);
    assert_eq!(add_cluster::kubeconfig_target().await.unwrap(), path.display().to_string());
    let listed = add_cluster::managed_contexts().await.unwrap();
    assert_eq!(listed.contexts.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["acme-uat", "lab"]);

    // Marking a context (its cluster's name looks like production).
    add_cluster::set_production(app.handle().clone(), "lab".into(), false).unwrap();
    add_cluster::set_production(app.handle().clone(), "acme-uat".into(), true).unwrap();
    let list = commands::list_contexts(app.state(), w(&app)).unwrap();
    assert!(list.contexts.iter().all(|c| c.prod_like == (c.name == "acme-uat")));

    // Adding one by hand; a name in use is refused.
    let cluster = NewCluster {
        name: "dev".into(),
        server: "https://10.0.0.1:6443".into(),
        certificate_authority: String::new(),
        insecure_skip_tls_verify: true,
        proxy_url: String::new(),
        auth: subduer_core::types::ClusterAuth::Token { token: "t".into() },
        namespace: String::new(),
        make_current: false,
    };
    assert_eq!(add_cluster::add_cluster(app.handle().clone(), cluster.clone()).await.unwrap().context, "dev");
    assert!(add_cluster::add_cluster(app.handle().clone(), cluster).await.unwrap_err().contains("dev"));
    add_cluster::reorder_contexts(app.handle().clone(), vec!["dev".into(), "lab".into(), "acme-uat".into()]).await.unwrap();
    assert_eq!(add_cluster::managed_contexts().await.unwrap().contexts[0].name, "dev");
    // Removing one forgets its mark.
    add_cluster::remove_context(app.handle().clone(), "lab".into()).await.unwrap();
    add_cluster::remove_context(app.handle().clone(), "dev".into()).await.unwrap();
    assert!(add_cluster::remove_context(app.handle().clone(), "nope".into()).await.is_err());
    assert_eq!(*changed.lock().unwrap(), 6);

    // Cloud clusters: refused before any CLI runs.
    let cloud = subduer_core::types::CloudProvider::Gke;
    assert!(add_cluster::cloud_clusters(cloud, "-p".into(), String::new()).await.unwrap_err().contains("not a valid"));
    let found = subduer_core::types::CloudCluster { name: "c".into(), location: "l".into(), entry: "e".into(), group: String::new(), version: String::new(), status: String::new(), server: String::new() };
    let err = add_cluster::add_cloud_cluster(app.handle().clone(), cloud, "p".into(), found, "-bad".into(), false).await.unwrap_err();
    assert!(err.contains("not a valid"), "{err}");

    // Marks that can't be saved, and a kubeconfig that can't be read.
    *app.state::<AppState>().prod.lock().unwrap() = subduer_core::safety::ProdMarks::load(std::path::Path::new("/dev/null/marks.json"));
    assert!(add_cluster::set_production(app.handle().clone(), "acme-uat".into(), false).is_err());
    std::fs::write(&path, "contexts: [").unwrap();
    assert!(add_cluster::set_production(app.handle().clone(), "acme-uat".into(), true).is_err());
    assert!(add_cluster::managed_contexts().await.is_err());
}

#[tokio::test]
async fn removing_a_context_tolerates_a_mark_that_cannot_be_saved() {
    let (_turn, _) = kubeconfig_file(CLUSTERS);
    let app = app();
    let marks = std::env::temp_dir().join(format!("subduer-tauri-marks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&marks);
    *app.state::<AppState>().prod.lock().unwrap() = subduer_core::safety::ProdMarks::load(&marks.join("production.json"));
    crate::add_cluster::set_production(app.handle().clone(), "lab".into(), false).unwrap();
    // The folder the marks live in becomes a file: forgetting can't save.
    std::fs::remove_dir_all(&marks).unwrap();
    std::fs::write(&marks, "").unwrap();
    crate::add_cluster::remove_context(app.handle().clone(), "lab".into()).await.unwrap();
    std::fs::remove_file(&marks).unwrap();
}

#[tokio::test]
async fn file_dialogs_answer_with_paths() {
    use crate::{add_cluster, capture, trace};
    let app = app();
    let h = || app.handle().clone();
    let chosen = chosen().display().to_string();
    assert_eq!(capture::choose_capture_file(h(), "api".into(), app.state()).await.unwrap(), Some(format!("{chosen}.pcap")));
    assert_eq!(trace::choose_trace_file(h(), "shop".into(), app.state()).await.unwrap(), Some(format!("{chosen}.trace")));
    assert_eq!(trace::choose_trace_to_open(h(), app.state()).await.unwrap(), Some(chosen.clone()));
    assert_eq!(trace::choose_report_file(h(), "/tmp/shop.trace".into(), app.state()).await.unwrap(), Some(format!("{chosen}.html")));
    assert_eq!(trace::choose_report_file(h(), String::new(), app.state()).await.unwrap(), Some(format!("{chosen}.html")));
    assert_eq!(add_cluster::choose_cert_file(h(), "CA".into(), app.state()).await.unwrap(), Some(chosen));

    // Cancelled.
    let cancelled = dialog_app(Dialogs { save: cancel, open: cancel });
    assert_eq!(capture::choose_capture_file(cancelled.handle().clone(), "api".into(), cancelled.state()).await.unwrap(), None);
    assert_eq!(add_cluster::choose_cert_file(cancelled.handle().clone(), "CA".into(), cancelled.state()).await.unwrap(), None);
    // Not a local file.
    let web = dialog_app(Dialogs { save: answer_url, open: answer_url });
    assert!(trace::choose_trace_to_open(web.handle().clone(), web.state()).await.is_err());
    // Without a folder to start in, the dialog opens wherever the system likes.
    let dialog = tauri_plugin_dialog::DialogExt::dialog(app.handle()).file();
    let _ = crate::system::starting_in(dialog, None::<PathBuf>);
}

/// Counts the `name` events the app sends.
fn count(app: &App<MockRuntime>, name: &str) -> Arc<Mutex<usize>> {
    let seen = Arc::new(Mutex::new(0));
    let into = seen.clone();
    app.listen_any(name, move |_| *into.lock().unwrap() += 1);
    seen
}

#[tokio::test(flavor = "multi_thread")]
async fn watches_send_their_table_to_the_window() {
    let api = MockApi::new();
    api.on("GET", "/apis/apps/v1/deployments", list("DeploymentList", vec![object("apps/v1", "Deployment", Some("shop"), "api", json!({}))]));
    let app = connected(&api).await;
    let tables = count(&app, subduer_core::types::events::RESOURCE_TABLE);
    commands::watch_resources(app.handle().clone(), w(&app), deployment(), None).await.unwrap();
    until("a table", || *tables.lock().unwrap() > 0).await;
    commands::stop_watch(w(&app));
}

#[tokio::test(flavor = "multi_thread")]
async fn port_forwards_report_their_connections() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/pods/api-1", pod("shop", "api-1", "node-a", json!({
        "spec": {"containers": [{"name": "app", "image": "acme/app:1", "ports": [{"containerPort": 8080}]}]}})));
    let app = connected(&api).await;
    let changes = count(&app, subduer_core::types::forward_events::CHANGED);
    let request = ForwardRequest { namespace: "shop".into(), source: ForwardSource::Pod { pod: "api-1".into() }, remote_port: 8080, local_port: 0 };
    let info = crate::forward::start_forward(app.handle().clone(), w(&app), request).await.unwrap();
    let started = *changes.lock().unwrap();
    // A connection to the local port (which can't reach the pod here) is reported.
    let _socket = tokio::net::TcpStream::connect(("127.0.0.1", info.local_port)).await.unwrap();
    until("the connection to be reported", || *changes.lock().unwrap() > started).await;
}

#[test]
fn new_main_windows_take_the_configured_size() {
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    let main = tauri::utils::config::WindowConfig { label: MAIN_LABEL.into(), width: 1400.0, ..Default::default() };
    context.config_mut().app.windows.push(main);
    let app = tauri::test::mock_builder().build(context).unwrap();
    app.manage(app_state());
    app.state::<AppState>().opaque.store(true, Ordering::SeqCst);
    let window = crate::open_main_window(app.handle(), "main-2").unwrap();
    assert_eq!(window.label(), "main-2");
}
