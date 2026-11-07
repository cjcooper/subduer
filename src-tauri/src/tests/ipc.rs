//! Commands invoked the way the UI invokes them: through Tauri's IPC, with
//! the app's own command list. Every async command is reached this way at
//! least once (its IPC glue is only exercised here); the file upload, whose
//! body is the raw file, is tested here in full.

use serde_json::Value;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;

use super::*;

/// The app as built for the UI, on the mock runtime, with its main window.
fn ui_app() -> (App<MockRuntime>, tauri::WebviewWindow<MockRuntime>) {
    let app = crate::handlers(mock_builder()).build(mock_context(noop_assets())).unwrap();
    app.manage(Dialogs::<MockRuntime> { save: answer, open: cancel });
    app.state::<AppState>().add_window(MAIN_LABEL);
    let window = WebviewWindowBuilder::new(&app, MAIN_LABEL, WebviewUrl::default()).build().unwrap();
    (app, window)
}

fn request(cmd: &str, body: InvokeBody, headers: &[(&str, &str)]) -> InvokeRequest {
    let mut map = tauri::http::HeaderMap::new();
    for (name, value) in headers {
        map.insert(tauri::http::HeaderName::from_bytes(name.as_bytes()).unwrap(), value.parse().unwrap());
    }
    InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: if cfg!(windows) { "http://tauri.localhost" } else { "tauri://localhost" }.parse().unwrap(),
        body,
        headers: map,
        invoke_key: INVOKE_KEY.into(),
    }
}

fn invoke(window: &tauri::WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    get_ipc_response(window, request(cmd, InvokeBody::Json(args), &[])).map(|b| b.deserialize().unwrap())
}

/// Every async command, sent everything but its last argument (or that one
/// of the wrong type): the UI hears which argument was wrong. (The IPC glue
/// Tauri generates checks each argument in turn; only these calls reach the
/// check of the last one.)
#[test]
fn every_async_command_checks_its_arguments_over_ipc() {
    let (app, window) = ui_app();
    let kind = json!({"group": "apps", "version": "v1", "kind": "Deployment", "plural": "deployments", "namespaced": true});
    let key = json!({"namespace": "shop", "kind": "Pod", "name": "api-1", "container": "app"});
    let calls = [
        ("connect", json!({})),
        ("watch_resources", json!({"kind": kind, "namespace": 5})),
        ("get_yaml", json!({"kind": kind, "namespace": "shop"})),
        ("pod_details", json!({"namespace": "shop"})),
        ("pod_history", json!({"namespace": "shop"})),
        ("workload_details", json!({"kind": kind, "namespace": "shop"})),
        ("workload_history", json!({"kind": kind, "namespace": "shop"})),
        ("node_history", json!({})),
        ("delete_resource", json!({"context": CTX, "kind": kind, "namespace": "shop"})),
        ("get_replicas", json!({"kind": kind, "namespace": "shop"})),
        ("restart_workload", json!({"context": CTX, "kind": kind, "namespace": "shop"})),
        ("scale_workload", json!({"context": CTX, "kind": kind, "namespace": "shop", "name": "api"})),
        ("suspend_workload", json!({"context": CTX, "kind": kind, "namespace": "shop", "name": "api"})),
        ("run_cron_job", json!({"context": CTX, "namespace": "shop"})),
        ("rightsize_preview", json!({"context": CTX})),
        ("rightsize_apply", json!({"context": CTX})),
        ("object_details", json!({"kind": kind, "namespace": "shop"})),
        ("secret_values", json!({"namespace": "shop"})),
        ("save_config_data", json!({"context": CTX, "kind": kind, "namespace": "shop", "name": "app", "resource_version": "1"})),
        ("set_hpa_range", json!({"context": CTX, "kind": kind, "namespace": "shop", "name": "api", "min": 1})),
        ("expand_claim", json!({"context": CTX, "namespace": "shop", "name": "data"})),
        ("set_reclaim_policy", json!({"context": CTX, "name": "pv-1"})),
        ("force_sync", json!({"context": CTX, "kind": kind, "namespace": "shop"})),
        ("pause_scaling", json!({"context": CTX, "kind": kind, "namespace": "shop", "name": "web", "paused": true, "replicas": "x"})),
        ("set_scaling_range", json!({"context": CTX, "kind": kind, "namespace": "shop", "name": "web", "min": 0})),
        ("check_access", json!({"account_namespace": "shop", "account": "api", "verb": "get", "resource": "pods", "namespace": "shop"})),
        ("helm_rollback", json!({"context": CTX, "namespace": "shop", "name": "web", "revision": 2})),
        ("helm_uninstall", json!({"context": CTX, "namespace": "shop", "name": "web", "keep_history": false})),
        ("create_namespace", json!({"context": CTX})),
        ("close_discarding", json!({})),
        ("secret_value", json!({"namespace": "shop", "secret": "db"})),
        ("open_logs", json!({})),
        ("open_exec", json!({})),
        ("start_capture", json!({})),
        ("forward_ports", json!({"namespace": "shop"})),
        ("start_forward", json!({})),
        ("browse_dir", json!({"namespace": "shop", "pod": "api-1", "container": "app", "path": "/"})),
        ("download_file", json!({"namespace": "shop", "pod": "api-1", "container": "app"})),
        ("open_editor", json!({})),
        ("editor_save", json!({"yaml": ""})),
        ("trace_targets", json!({})),
        ("start_trace", json!({"context": CTX, "scope": "Cluster", "path": "/tmp/t"})),
        ("stop_trace", json!({})),
        ("trace_review", json!({})),
        ("trace_series", json!({"path": "/tmp/t", "key": key, "start": 1.0, "end": 2.0})),
        ("trace_overview", json!({"path": "/tmp/t", "namespace": "shop", "start": 1.0, "end": 2.0})),
        ("trace_daily", json!({"path": "/tmp/t"})),
        ("trace_set_note", json!({"path": "/tmp/t", "key": key})),
        ("trace_compare", json!({})),
        ("export_trace_report", json!({"path": "/tmp/t"})),
        ("add_cluster", json!({})),
        ("cloud_accounts", json!({})),
        ("cloud_clusters", json!({"provider": "gke", "account": "p"})),
        ("add_cloud_cluster", json!({"provider": "gke", "account": "p", "cluster": {"name": "c", "location": "l", "group": "", "version": "", "status": ""}, "alias": ""})),
        ("remove_context", json!({})),
        ("reorder_contexts", json!({})),
    ];
    for (cmd, args) in calls {
        let err = invoke(&window, cmd, args).unwrap_err();
        let err = err.as_str().unwrap_or_default();
        assert!(err.contains("missing required key") || err.contains("invalid type"), "{cmd}: {err}");
    }
    // Commands whose last argument is the calling window: one the app
    // doesn't know about isn't connected.
    let stray = WebviewWindowBuilder::new(&app, "stray", WebviewUrl::default()).build().unwrap();
    for cmd in ["check_connection", "cluster_overview", "trace_status", "editor_start"] {
        assert_eq!(invoke(&stray, cmd, json!({})).unwrap_err(), "this window is no longer connected", "{cmd}");
    }
    // Called properly from the main window.
    for cmd in ["check_connection", "cluster_overview"] {
        assert_eq!(invoke(&window, cmd, json!({})).unwrap_err(), "not connected to a cluster", "{cmd}");
    }
    assert_eq!(invoke(&window, "trace_status", json!({})).unwrap(), Value::Null);
    // The main window isn't a capture or an edit window.
    assert_eq!(invoke(&window, "stop_capture", json!({})).unwrap_err(), "this capture window is no longer connected");
    assert_eq!(invoke(&window, "editor_start", json!({})).unwrap_err(), "this edit window is no longer connected");
    // The Open dialog was cancelled.
    assert_eq!(invoke(&window, "choose_trace_to_open", json!({})).unwrap(), Value::Null);
    // Without the app's state (never so in the app), commands that use it say so.
    #[allow(deprecated)]
    let _ = app.unmanage::<AppState>();
    assert!(invoke(&window, "stop_capture", json!({})).unwrap_err().as_str().unwrap().contains("state not managed"));
    #[allow(deprecated)]
    let _ = app.unmanage::<Dialogs<MockRuntime>>();
    for (cmd, args) in [
        ("choose_capture_file", json!({"suggested": "api"})),
        ("choose_trace_file", json!({"suggested": "shop"})),
        ("choose_trace_to_open", json!({})),
        ("choose_report_file", json!({"trace": "/tmp/shop.trace"})),
        ("choose_cert_file", json!({"title": "CA"})),
    ] {
        assert!(invoke(&window, cmd, args).unwrap_err().as_str().unwrap().contains("state not managed"), "{cmd}");
    }
}

#[test]
fn reading_the_kubeconfig_over_ipc() {
    let (_turn, path) = kubeconfig_file("apiVersion: v1\nkind: Config\ncontexts: []\n");
    let (_app, window) = ui_app();
    assert_eq!(invoke(&window, "kubeconfig_target", json!({})).unwrap(), path.display().to_string());
    assert_eq!(invoke(&window, "managed_contexts", json!({})).unwrap()["contexts"], json!([]));
}

fn upload(window: &tauri::WebviewWindow<MockRuntime>, target: Option<&str>, body: InvokeBody) -> Result<Value, Value> {
    let headers: Vec<(&str, &str)> = target.map(|t| ("x-upload", t)).into_iter().collect();
    get_ipc_response(window, request("upload_file", body, &headers)).map(|b| b.deserialize().unwrap())
}

/// The upload target as the UI sends it: URL-encoded JSON.
fn target(context: &str) -> String {
    let json = json!({"context": context, "namespace": "shop", "pod": "api-1", "container": "app",
                      "path": "/tmp", "file_name": "hello.txt", "overwrite": false});
    percent_encoding::utf8_percent_encode(&json.to_string(), percent_encoding::NON_ALPHANUMERIC).to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn uploads_arrive_as_the_raw_body_with_their_target_in_a_header() {
    use super::fake::{Fake, Step, out, success};
    let (app, window) = ui_app();
    let raw = || InvokeBody::Raw(b"hello".to_vec());
    let call = move |target: Option<String>, body: InvokeBody| {
        let window = window.clone();
        tokio::task::spawn_blocking(move || upload(&window, target.as_deref(), body))
    };
    let err = |r: Result<Value, Value>| r.unwrap_err().as_str().unwrap().to_owned();
    assert_eq!(err(call(None, raw()).await.unwrap()), "missing upload target");
    // From a window the app doesn't know about.
    let stray = WebviewWindowBuilder::new(&app, "stray", WebviewUrl::default()).build().unwrap();
    let from_stray = tokio::task::spawn_blocking(move || upload(&stray, Some("x"), InvokeBody::Raw(vec![]))).await.unwrap();
    assert_eq!(err(from_stray), "this window is no longer connected");
    assert!(err(call(Some("%FF".into()), raw()).await.unwrap()).contains("utf-8"));
    assert!(err(call(Some("not-json".into()), raw()).await.unwrap()).contains("expected"));
    assert_eq!(err(call(Some(target(CTX)), InvokeBody::Json(json!({}))).await.unwrap()), "expected the file as a binary body");
    assert_eq!(err(call(Some(target(CTX)), raw()).await.unwrap()), "not connected to a cluster");

    let fake = Fake::start();
    fake.exec(vec![Step::Wait(|s| s.stdin.len() >= 5), out("/tmp/hello.txt\n5\n"), success()]);
    let main = app.state::<AppState>().window(MAIN_LABEL).unwrap();
    *main.session.write().await = Some(Session { context: CTX.into(), client: fake.client(), history: Default::default() });
    let other = err(call(Some(target("acme-prd")), raw()).await.unwrap());
    assert_eq!(other, "connected to acme-uat now, not acme-prd; nothing was uploaded");
    assert!(err(call(Some(target(CTX)), raw()).await.unwrap()).contains("read-only"));
    main.gate.set_unlocked(CTX, true);
    let done = call(Some(target(CTX)), raw()).await.unwrap().unwrap();
    assert_eq!(done, json!({"Uploaded": {"bytes": 5, "path": "/tmp/hello.txt"}}));
    assert_eq!(fake.seen.lock().unwrap().stdin, b"hello");
}
