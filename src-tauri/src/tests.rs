//! Command tests: the Tauri commands called on a mock app (Tauri's test
//! runtime) whose session talks to a fake Kubernetes API
//! (`subduer_core::testing`, or `fake` where a real connection is needed).
//! Nothing here contacts a cluster, opens a browser, a dialog or Finder, or
//! runs `helm` for real: the launcher and dialogs are stand-ins.

pub(crate) mod fake;
mod ipc;
mod lifecycle;
mod sessions;

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use serde_json::json;
use subduer_core::testing::{MockApi, kind, list, object, pod};
use subduer_core::types::{DataChange, ExecTarget, ForwardRequest, ForwardSource, LogSource, LogTarget};
use tauri::test::MockRuntime;
use tauri::{App, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{FileDialogBuilder, FilePath};

use crate::system::{Dialogs, Launcher, Reply};
use crate::{AppState, MAIN_LABEL, Session, Win, commands};

const CTX: &str = "acme-uat";

/// Every program the stand-in launcher was asked to start, as `program arg…`.
static LAUNCHED: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn record_launch(cmd: &mut Command) -> std::io::Result<()> {
    let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    LAUNCHED.lock().unwrap().push(format!("{} {}", cmd.get_program().to_string_lossy(), args.join(" ")));
    Ok(())
}

/// Whether a program was started with `arg` among its arguments.
fn launched_with(arg: &str) -> bool {
    LAUNCHED.lock().unwrap().iter().any(|l| l.split(' ').any(|a| a == arg))
}

/// App state that records programs instead of starting them.
fn app_state() -> AppState {
    AppState { launcher: Launcher(record_launch), ..Default::default() }
}

/// The path the stand-in dialogs answer with.
fn chosen() -> PathBuf {
    std::env::temp_dir().join("subduer-dialog").join("chosen")
}

fn answer(_: FileDialogBuilder<MockRuntime>, reply: Reply) {
    reply(Some(FilePath::Path(chosen())));
}

fn cancel(_: FileDialogBuilder<MockRuntime>, reply: Reply) {
    reply(None);
}

/// A web address, which isn't a local path.
fn answer_url(_: FileDialogBuilder<MockRuntime>, reply: Reply) {
    reply(Some(FilePath::Url("https://acme.io/file".parse().unwrap())));
}

/// One test's turn with the shared kubeconfig, until dropped. (Not a mutex
/// guard: tests hold it across awaits, on whichever thread.)
struct Turn;

static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl Turn {
    fn take() -> Turn {
        use std::sync::atomic::Ordering::SeqCst;
        while BUSY.compare_exchange(false, true, SeqCst, SeqCst).is_err() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Turn
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        BUSY.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The kubeconfig the app reads in these tests (`KUBECONFIG`, set once for
/// the whole run), holding `text` until the guard is dropped. Tests that read
/// or write it take turns.
fn kubeconfig_file(text: &str) -> (Turn, PathBuf) {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    let turn = Turn::take();
    let path = PATH.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("subduer-tauri-kubeconfig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");
        // SAFETY: set once, before any test reads it (they all come here first).
        unsafe { std::env::set_var("KUBECONFIG", &path) };
        path
    });
    std::fs::write(path, text).unwrap();
    (turn, path.clone())
}

/// A mock app with the dialog plugin, whose dialogs are `dialogs`.
fn dialog_app(dialogs: Dialogs<MockRuntime>) -> App<MockRuntime> {
    let app = tauri::test::mock_builder().plugin(tauri_plugin_dialog::init()).build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
    app.manage(dialogs);
    app
}

fn app() -> App<MockRuntime> {
    let app = dialog_app(Dialogs { save: answer, open: answer });
    app.manage(app_state());
    app.state::<AppState>().add_window(MAIN_LABEL);
    app
}

/// The state of the app's main window.
fn w(app: &App<MockRuntime>) -> Win {
    app.state::<AppState>().window(MAIN_LABEL).unwrap()
}

/// A mock app connected to `api` as context `acme-uat`.
async fn connected(api: &MockApi) -> App<MockRuntime> {
    let app = app();
    *w(&app).session.write().await =
        Some(Session { context: CTX.into(), client: api.client(), history: Default::default() });
    app
}

fn unlock(app: &App<MockRuntime>) {
    commands::set_unlocked(w(app), CTX.into(), true);
}

fn deployment() -> subduer_core::types::KindRef {
    kind("apps", "v1", "Deployment", "deployments", true)
}

#[tokio::test]
async fn reads_need_a_connection() {
    let app = app();
    let s = || w(&app);
    let not = "not connected to a cluster";
    assert_eq!(commands::pod_details(s(), "shop".into(), "api".into()).await.unwrap_err(), not);
    assert_eq!(commands::pod_history(s(), "shop".into(), "api".into()).await.unwrap_err(), not);
    assert_eq!(commands::cluster_overview(s()).await.unwrap_err(), not);
    assert_eq!(commands::check_connection(s()).await.unwrap_err(), not);
    assert_eq!(commands::get_yaml(s(), deployment(), Some("shop".into()), "api".into()).await.unwrap_err(), not);
    assert_eq!(commands::workload_details(s(), deployment(), "shop".into(), "api".into()).await.unwrap_err(), not);
    assert_eq!(commands::workload_history(s(), deployment(), "shop".into(), "api".into()).await.unwrap_err(), not);
    assert_eq!(commands::node_history(s(), "node-a".into()).await.unwrap_err(), not);
    assert_eq!(commands::get_replicas(s(), deployment(), "shop".into(), "api".into()).await.unwrap_err(), not);
    assert_eq!(commands::object_details(s(), deployment(), Some("shop".into()), "api".into()).await.unwrap_err(), not);
    assert_eq!(commands::secret_values(s(), "shop".into(), "db".into()).await.unwrap_err(), not);
    assert_eq!(commands::secret_value(s(), "shop".into(), "db".into(), "pw".into()).await.unwrap_err(), not);
    let check = commands::check_access(s(), "shop".into(), "api".into(), "get".into(), "pods".into(), String::new(), String::new());
    assert_eq!(check.await.unwrap_err(), not);
    assert_eq!(crate::forward::forward_ports(s(), "shop".into(), ForwardSource::Pod { pod: "api-1".into() }).await.unwrap_err(), not);
    let browse = crate::upload::browse_dir(s(), "shop".into(), "api-1".into(), "app".into(), "/".into(), false);
    assert_eq!(browse.await.unwrap_err(), not);
    let watch = commands::watch_resources(app.handle().clone(), s(), deployment(), None);
    assert_eq!(watch.await.unwrap_err(), not);
}

#[tokio::test]
async fn connection_check_reports_expired_credentials() {
    let api = MockApi::new();
    api.on("GET", "/version", json!({"major": "1", "minor": "36", "gitVersion": "v1.36.0"}));
    let app = connected(&api).await;
    commands::check_connection(w(&app)).await.unwrap();
    api.on_error("GET", "/version", 401, "Unauthorized", "Unauthorized");
    assert!(commands::check_connection(w(&app)).await.unwrap_err().contains("Unauthorized"));
}

#[tokio::test]
async fn changes_need_the_same_context_and_an_unlocked_gate() {
    let api = MockApi::new();
    api.on("DELETE", "/apis/apps/v1/namespaces/shop/deployments/api", json!({"kind": "Status", "status": "Success"}));
    let app = connected(&api).await;
    let s = || w(&app);
    // The dialog's context isn't the connected one.
    let err = commands::delete_resource(s(), "acme-prd".into(), deployment(), Some("shop".into()), "api".into()).await.unwrap_err();
    assert_eq!(err, "connected to acme-uat now, not acme-prd; nothing was deleted");
    // Locked.
    let err = commands::delete_resource(s(), CTX.into(), deployment(), Some("shop".into()), "api".into()).await.unwrap_err();
    assert!(err.contains("read-only"), "{err}");
    assert!(api.calls().is_empty(), "nothing reached the API");
    // Unlocked: it goes through.
    unlock(&app);
    commands::delete_resource(s(), CTX.into(), deployment(), Some("shop".into()), "api".into()).await.unwrap();
    assert_eq!(api.calls_to("DELETE", "/apis/apps/v1/namespaces/shop/deployments/api").len(), 1);
    // Locking again refuses.
    commands::set_unlocked(s(), CTX.into(), false);
    assert!(commands::delete_resource(s(), CTX.into(), deployment(), Some("shop".into()), "api".into()).await.is_err());

    // Not connected at all.
    let other = self::app();
    let err = commands::restart_workload(w(&other), CTX.into(), deployment(), "shop".into(), "api".into()).await.unwrap_err();
    assert_eq!(err, "not connected to a cluster");
}

#[tokio::test]
async fn workload_changes_reach_the_api() {
    let api = MockApi::new();
    let deploy_path = "/apis/apps/v1/namespaces/shop/deployments/api";
    let scale = json!({"apiVersion": "autoscaling/v1", "kind": "Scale", "metadata": {"name": "api", "namespace": "shop"},
                       "spec": {"replicas": 3}, "status": {"replicas": 2}});
    api.on("PATCH", deploy_path, object("apps/v1", "Deployment", Some("shop"), "api", json!({})));
    api.on("GET", &format!("{deploy_path}/scale"), scale.clone());
    api.on("PATCH", &format!("{deploy_path}/scale"), scale);
    let cron = kind("batch", "v1", "CronJob", "cronjobs", true);
    api.on("PATCH", "/apis/batch/v1/namespaces/shop/cronjobs/nightly", object("batch/v1", "CronJob", Some("shop"), "nightly", json!({})));
    api.on("GET", "/apis/batch/v1/namespaces/shop/cronjobs/nightly", object("batch/v1", "CronJob", Some("shop"), "nightly", json!({
        "spec": {"schedule": "0 3 * * *", "jobTemplate": {"spec": {"template": {"spec": {"containers": [{"name": "run", "image": "busybox"}]}}}}}
    })));
    api.on("POST", "/apis/batch/v1/namespaces/shop/jobs", object("batch/v1", "Job", Some("shop"), "nightly-manual-abcde", json!({})));
    let app = connected(&api).await;
    unlock(&app);
    let s = || w(&app);

    let r = commands::get_replicas(s(), deployment(), "shop".into(), "api".into()).await.unwrap();
    assert_eq!((r.desired, r.current), (3, 2));
    commands::restart_workload(s(), CTX.into(), deployment(), "shop".into(), "api".into()).await.unwrap();
    let restart = api.calls_to("PATCH", deploy_path).pop().unwrap().json();
    assert!(restart["spec"]["template"]["metadata"]["annotations"]["kubectl.kubernetes.io/restartedAt"].is_string());
    commands::scale_workload(s(), CTX.into(), deployment(), "shop".into(), "api".into(), 5).await.unwrap();
    assert_eq!(api.calls_to("PATCH", &format!("{deploy_path}/scale")).pop().unwrap().json(), json!({"spec": {"replicas": 5}}));
    commands::suspend_workload(s(), CTX.into(), cron.clone(), "shop".into(), "nightly".into(), true).await.unwrap();
    let patch = api.calls_to("PATCH", "/apis/batch/v1/namespaces/shop/cronjobs/nightly").pop().unwrap().json();
    assert_eq!(patch, json!({"spec": {"suspend": true}}));
    let job = commands::run_cron_job(s(), CTX.into(), "shop".into(), "nightly".into()).await.unwrap();
    assert_eq!(job, "nightly-manual-abcde");
    // A dialog from another context changes nothing.
    let err = commands::suspend_workload(s(), "acme-prd".into(), cron, "shop".into(), "nightly".into(), false).await.unwrap_err();
    assert!(err.ends_with("nothing was resumed"), "{err}");
    let err = commands::run_cron_job(s(), "acme-prd".into(), "shop".into(), "nightly".into()).await.unwrap_err();
    assert!(err.ends_with("nothing was started"), "{err}");
}

#[tokio::test]
async fn config_and_storage_changes_reach_the_api() {
    let api = MockApi::new();
    let cm_path = "/api/v1/namespaces/shop/configmaps/app";
    api.on("PATCH", cm_path, object("v1", "ConfigMap", Some("shop"), "app", json!({})));
    api.on("PATCH", "/apis/autoscaling/v2/namespaces/shop/horizontalpodautoscalers/api", object("autoscaling/v2", "HorizontalPodAutoscaler", Some("shop"), "api", json!({})));
    let pvc = object("v1", "PersistentVolumeClaim", Some("shop"), "data", json!({"spec": {"resources": {"requests": {"storage": "10Gi"}}}}));
    api.on("GET", "/api/v1/namespaces/shop/persistentvolumeclaims/data", pvc.clone());
    api.on("PATCH", "/api/v1/namespaces/shop/persistentvolumeclaims/data", pvc);
    api.on("PATCH", "/api/v1/persistentvolumes/pvc-1", object("v1", "PersistentVolume", None, "pvc-1", json!({})));
    api.on("PATCH", "/apis/external-secrets.io/v1/namespaces/shop/externalsecrets/db", object("external-secrets.io/v1", "ExternalSecret", Some("shop"), "db", json!({})));
    api.on("POST", "/api/v1/namespaces", object("v1", "Namespace", None, "team-sandbox", json!({})));
    let so_path = "/apis/keda.sh/v1alpha1/namespaces/shop/scaledobjects/web";
    api.on("GET", so_path, object("keda.sh/v1alpha1", "ScaledObject", Some("shop"), "web", json!({"spec": {}})));
    api.on("PATCH", so_path, object("keda.sh/v1alpha1", "ScaledObject", Some("shop"), "web", json!({})));
    let app = connected(&api).await;
    unlock(&app);
    let s = || w(&app);

    let cm = kind("", "v1", "ConfigMap", "configmaps", true);
    let changes = vec![DataChange { key: "a".into(), value: Some("1".into()) }];
    commands::save_config_data(s(), CTX.into(), cm, "shop".into(), "app".into(), "42".into(), changes).await.unwrap();
    assert_eq!(
        api.calls_to("PATCH", cm_path).pop().unwrap().json(),
        json!({"metadata": {"resourceVersion": "42"}, "data": {"a": "1"}})
    );
    let hpa = kind("autoscaling", "v2", "HorizontalPodAutoscaler", "horizontalpodautoscalers", true);
    commands::set_hpa_range(s(), CTX.into(), hpa, "shop".into(), "api".into(), 2, 6).await.unwrap();
    commands::expand_claim(s(), CTX.into(), "shop".into(), "data".into(), "20Gi".into()).await.unwrap();
    let grow = api.calls_to("PATCH", "/api/v1/namespaces/shop/persistentvolumeclaims/data").pop().unwrap().json();
    assert_eq!(grow, json!({"spec": {"resources": {"requests": {"storage": "20Gi"}}}}));
    let err = commands::expand_claim(s(), CTX.into(), "shop".into(), "data".into(), "5Gi".into()).await.unwrap_err();
    assert!(err.contains("only grow"), "{err}");
    commands::set_reclaim_policy(s(), CTX.into(), "pvc-1".into(), "Retain".into()).await.unwrap();
    let es = kind("external-secrets.io", "v1", "ExternalSecret", "externalsecrets", true);
    assert_eq!(commands::force_sync(s(), CTX.into(), es.clone(), "shop".into(), "db".into()).await.unwrap(), 1);
    let synced = api.calls_to("PATCH", "/apis/external-secrets.io/v1/namespaces/shop/externalsecrets/db").pop().unwrap().json();
    assert!(synced["metadata"]["annotations"]["force-sync"].is_string());
    let err = commands::force_sync(s(), "other".into(), es, "shop".into(), "db".into()).await.unwrap_err();
    assert!(err.contains("nothing was synced"), "{err}");
    let so = kind("keda.sh", "v1alpha1", "ScaledObject", "scaledobjects", true);
    commands::pause_scaling(s(), CTX.into(), so.clone(), "shop".into(), "web".into(), true, Some(2)).await.unwrap();
    let paused = api.calls_to("PATCH", so_path).pop().unwrap().json();
    assert_eq!(paused["metadata"]["annotations"]["autoscaling.keda.sh/paused-replicas"], "2");
    commands::set_scaling_range(s(), CTX.into(), so.clone(), "shop".into(), "web".into(), 0, 8).await.unwrap();
    assert_eq!(api.calls_to("PATCH", so_path).pop().unwrap().json(), json!({"spec": {"minReplicaCount": 0, "maxReplicaCount": 8}}));
    let err = commands::pause_scaling(s(), "other".into(), so, "shop".into(), "web".into(), false, None).await.unwrap_err();
    assert!(err.contains("nothing was resumed"), "{err}");
    commands::create_namespace(s(), CTX.into(), "team-sandbox".into()).await.unwrap();
    assert_eq!(api.calls_to("POST", "/api/v1/namespaces").pop().unwrap().json()["metadata"]["name"], "team-sandbox");
    let err = commands::create_namespace(s(), CTX.into(), "Bad_Name".into()).await.unwrap_err();
    assert!(err.contains("lowercase"), "{err}");
}

#[tokio::test]
async fn helm_changes_are_refused_before_helm_runs() {
    let api = MockApi::new();
    let app = connected(&api).await;
    let s = || w(&app);
    // Locked: refused before `helm` would run.
    let err = commands::helm_rollback(s(), CTX.into(), "shop".into(), "web".into(), 2, false).await.unwrap_err();
    assert!(err.contains("read-only"), "{err}");
    let err = commands::helm_uninstall(s(), CTX.into(), "shop".into(), "web".into(), false, false).await.unwrap_err();
    assert!(err.contains("read-only"), "{err}");
    // Unlocked but invalid input: refused before `helm` would run too.
    unlock(&app);
    let err = commands::helm_rollback(s(), CTX.into(), "shop".into(), "--all".into(), 2, false).await.unwrap_err();
    assert!(err.contains("not a valid release"), "{err}");
    let err = commands::helm_rollback(s(), CTX.into(), "shop".into(), "web".into(), 0, false).await.unwrap_err();
    assert!(err.contains("pick a revision"), "{err}");
    let err = commands::helm_uninstall(s(), CTX.into(), "-n".into(), "web".into(), false, false).await.unwrap_err();
    assert!(err.contains("not a valid release"), "{err}");
    // Wrong context.
    let err = commands::helm_uninstall(s(), "acme-prd".into(), "shop".into(), "web".into(), false, false).await.unwrap_err();
    assert!(err.ends_with("nothing was deleted"), "{err}");
}

#[tokio::test]
async fn reads_come_back_from_the_api() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/pods/api-1", pod("shop", "api-1", "node-a", json!({})));
    api.on("GET", "/api/v1/namespaces/shop/events", list("EventList", vec![]));
    api.on("GET", "/api/v1/namespaces/shop/configmaps/app", object("v1", "ConfigMap", Some("shop"), "app", json!({"data": {"a": "1"}})));
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![]));
    api.on("GET", "/api/v1/namespaces/shop/secrets/db", object("v1", "Secret", Some("shop"), "db", json!({"data": {"pw": "aHVudGVyMg=="}})));
    api.on("POST", "/apis/authorization.k8s.io/v1/subjectaccessreviews", json!({
        "apiVersion": "authorization.k8s.io/v1", "kind": "SubjectAccessReview",
        "spec": {}, "status": {"allowed": true, "reason": "RBAC: allowed by RoleBinding"}}));
    let app = connected(&api).await;
    let s = || w(&app);

    let pod = commands::pod_details(s(), "shop".into(), "api-1".into()).await.unwrap();
    assert_eq!((pod.name.as_str(), pod.node.as_deref()), ("api-1", Some("node-a")));
    let yaml = commands::get_yaml(s(), kind("", "v1", "ConfigMap", "configmaps", true), Some("shop".into()), "app".into()).await.unwrap();
    assert!(yaml.contains("name: app"), "{yaml}");
    let details = commands::object_details(s(), kind("", "v1", "ConfigMap", "configmaps", true), Some("shop".into()), "app".into()).await.unwrap();
    assert_eq!(details.data.unwrap().entries[0].value.as_deref(), Some("1"));
    assert_eq!(commands::secret_value(s(), "shop".into(), "db".into(), "pw".into()).await.unwrap(), "hunter2");
    let values = commands::secret_values(s(), "shop".into(), "db".into()).await.unwrap();
    assert_eq!(values, vec![("pw".to_owned(), Some("hunter2".to_owned()))]);
    let check = commands::check_access(s(), "shop".into(), "api".into(), "get".into(), "pods".into(), "shop".into(), String::new())
        .await
        .unwrap();
    assert!(check.allowed && check.reason.contains("RoleBinding"));
    // Errors come back as the server's message.
    let err = commands::pod_details(s(), "shop".into(), "gone".into()).await.unwrap_err();
    assert!(err.contains("isn't faked"), "{err}");
}

#[tokio::test]
async fn history_falls_back_to_local_samples() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/services", list("ServiceList", vec![]));
    api.on("GET", "/api/v1/namespaces/shop/pods/api-1", pod("shop", "api-1", "node-a", json!({})));
    let app = connected(&api).await;
    let history = commands::pod_history(w(&app), "shop".into(), "api-1".into()).await.unwrap();
    assert_eq!(history.source, subduer_core::types::HistorySource::Local);
}

#[tokio::test]
async fn small_commands() {
    let app = app();
    let s = || w(&app);
    commands::set_unsaved(s(), true);
    assert!(s().unsaved.load(std::sync::atomic::Ordering::SeqCst));
    // No main window in the mock: closing just clears the flag.
    commands::close_discarding(app.handle().clone(), s(), false).await.unwrap();
    assert!(!s().unsaved.load(std::sync::atomic::Ordering::SeqCst));
    // Transparency is remembered for later windows; the mock has no real
    // window to set effects on.
    let state = app.state::<crate::AppState>();
    assert!(!state.opaque.load(std::sync::atomic::Ordering::SeqCst), "on by default");
    let supported = crate::vibrancy::set_transparency(app.handle().clone(), app.state(), false);
    // Windows depends on the build (Mica is Windows 11 only).
    if !cfg!(windows) {
        assert_eq!(supported, cfg!(target_os = "macos"));
    }
    assert!(state.opaque.load(std::sync::atomic::Ordering::SeqCst));
    crate::vibrancy::set_transparency(app.handle().clone(), app.state(), true);
    assert!(!state.opaque.load(std::sync::atomic::Ordering::SeqCst));
    commands::stop_watch(s());
    for bad in ["file:///etc/passwd", "javascript:alert(1)", "https://acme.io/a b", "ftp://acme.io", "https://acme.io/\n"] {
        assert!(commands::open_url(app.state(), bad.into()).is_err(), "{bad}");
    }
    commands::open_url(app.state(), "https://acme.io/small-commands".into()).unwrap();
    assert!(launched_with("https://acme.io/small-commands"));
    assert!(crate::forward::open_forward(app.state(), s(), 99).is_err());
    assert!(crate::forward::list_forwards(s()).is_empty());
    assert!(crate::upload::reveal_download(app.handle().clone(), "/etc/hosts".into()).is_err());
    let download = app.path().download_dir().unwrap().join("small-commands.txt");
    crate::upload::reveal_download(app.handle().clone(), download.display().to_string()).unwrap();
    assert!(LAUNCHED.lock().unwrap().iter().any(|l| l.contains("small-commands.txt")));
    let cmd = &mut std::process::Command::new("true");
    let _ = crate::shell_env::hide_console(cmd);
    // With a main window, closing discards and closes it (the mock just forgets it).
    WebviewWindowBuilder::new(&app, MAIN_LABEL, WebviewUrl::default()).build().unwrap();
    commands::set_unsaved(s(), true);
    commands::close_discarding(app.handle().clone(), s(), false).await.unwrap();
    assert!(!s().unsaved.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn themes_go_to_the_runtime_and_the_icons_follow() {
    let app = app();
    static SET: Mutex<Vec<Option<tauri::Theme>>> = Mutex::new(Vec::new());
    for theme in ["light", "dark", "auto"] {
        commands::apply_theme(app.handle(), theme, |_, t| SET.lock().unwrap().push(t));
    }
    assert_eq!(*SET.lock().unwrap(), [Some(tauri::Theme::Light), Some(tauri::Theme::Dark), None]);
    // The real command asks Tauri's runtime, which the mock leaves unimplemented.
    let handle = app.handle().clone();
    let asked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| commands::set_app_theme(handle, "dark".into())));
    assert!(panic_text(asked.unwrap_err()).contains("not implemented"));
}

/// What a caught panic said.
fn panic_text(panic: Box<dyn std::any::Any + Send>) -> String {
    panic.downcast_ref::<String>().cloned().or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned())).unwrap_or_default()
}

#[test]
fn failures_the_app_carries_on_after_are_logged() {
    crate::log_err(Ok::<(), String>(()), "nothing to log");
    crate::log_err(Err::<(), _>(tauri::Error::WindowNotFound), "setting the window icon failed");
}

#[tokio::test]
async fn connect_refuses_an_unknown_context() {
    let (_turn, path) = kubeconfig_file("apiVersion: v1\nkind: Config\ncontexts: []\n");
    let err = commands::connect(app().handle().clone(), w(&app()), "no-such-context-anywhere".into()).await.unwrap_err();
    assert!(err.contains("no-such-context-anywhere"), "{err}");
    // An unreadable kubeconfig says so.
    std::fs::write(&path, "contexts: [").unwrap();
    let app = app();
    assert!(commands::connect(app.handle().clone(), w(&app), CTX.into()).await.is_err());
    assert!(commands::list_contexts(app.state(), w(&app)).is_err());
}

/// A cluster with only Pods, just enough for discovery to succeed.
fn empty_cluster() -> MockApi {
    let api = MockApi::new();
    api.on("GET", "/version", json!({"major": "1", "minor": "31", "gitVersion": "v1.31.0"}));
    api.on("GET", "/api", json!({"kind": "APIVersions", "versions": ["v1"], "serverAddressByClientCIDRs": []}));
    api.on("GET", "/api/v1", json!({"kind": "APIResourceList", "groupVersion": "v1", "resources": [
        {"name": "pods", "singularName": "pod", "namespaced": true, "kind": "Pod", "verbs": ["get", "list", "watch"]},
    ]}));
    api.on("GET", "/apis", json!({"kind": "APIGroupList", "apiVersion": "v1", "groups": []}));
    api
}

#[tokio::test]
async fn opening_a_session_connects_the_window() {
    let app = app();
    let api = empty_cluster();
    let info = commands::open_session(app.handle(), &w(&app), CTX.into(), api.client(), 0, commands::DISCOVER_TIMEOUT).await.unwrap();
    assert_eq!(info.context, CTX);
    assert_eq!(w(&app).session.read().await.as_ref().unwrap().context, CTX);
}

#[tokio::test]
async fn a_cluster_that_does_not_answer_fails_before_discovery() {
    let app = app();
    let api = MockApi::new();
    api.on_error("GET", "/version", 401, "Unauthorized", "token expired");
    let err = commands::open_session(app.handle(), &w(&app), CTX.into(), api.client(), 0, commands::DISCOVER_TIMEOUT).await.unwrap_err();
    assert!(err.contains("Unauthorized"), "{err}");
    assert!(api.calls_to("GET", "/apis").is_empty());
    assert!(w(&app).session.read().await.is_none());
}

#[tokio::test]
async fn an_older_connect_does_not_replace_a_newer_one() {
    let app = connected(&MockApi::new()).await;
    // Another context was chosen while attempt 1 was still connecting.
    w(&app).connect_attempt.store(2, std::sync::atomic::Ordering::SeqCst);
    let api = empty_cluster();
    let err = commands::open_session(app.handle(), &w(&app), "acme-dev".into(), api.client(), 1, commands::DISCOVER_TIMEOUT).await.unwrap_err();
    assert_eq!(err, commands::SUPERSEDED);
    assert!(api.calls_to("GET", "/apis").is_empty());
    assert_eq!(w(&app).session.read().await.as_ref().unwrap().context, CTX);
}

#[tokio::test]
async fn watching_returns_increasing_ids() {
    let api = MockApi::new();
    let app = connected(&api).await;
    let first = commands::watch_resources(app.handle().clone(), w(&app), deployment(), None).await.unwrap();
    let second = commands::watch_resources(app.handle().clone(), w(&app), deployment(), Some("shop".into())).await.unwrap();
    assert_eq!((first, second), (1, 2));
    commands::stop_watch(w(&app));
    assert!(w(&app).watch.lock().unwrap().is_none());
}

#[tokio::test]
async fn logs_windows_open_once_per_target() {
    let api = MockApi::new();
    api.on_text("GET", "/api/v1/namespaces/shop/pods/api-1/log", 200, "hello\n");
    let app = connected(&api).await;
    let target = LogTarget { namespace: "shop".into(), source: LogSource::Container { pod: "api-1".into(), container: "app".into() }, filter: None };
    crate::logs::open_logs(app.handle().clone(), app.state(), w(&app), target.clone()).await.unwrap();
    // The same target focuses the existing window instead of opening another.
    crate::logs::open_logs(app.handle().clone(), app.state(), w(&app), target.clone()).await.unwrap();
    let windows = app.webview_windows();
    let labels: Vec<&String> = windows.keys().filter(|l| l.starts_with("logs-")).collect();
    assert_eq!(labels.len(), 1);
    let window = app.get_webview_window(labels[0]).unwrap();
    let session = crate::logs::logs_start(window.clone(), app.state(), false).unwrap();
    assert_eq!(session.title, "app · shop/api-1 · acme-uat · Logs");
    assert_eq!(session.event, *labels[0]);
    crate::logs::close_all(app.handle(), MAIN_LABEL);

    // A window the app doesn't know about isn't connected.
    let stray = WebviewWindowBuilder::new(&app, "logs-99", WebviewUrl::default()).build().unwrap();
    assert!(crate::logs::logs_start(stray, app.state(), false).is_err());
    let none = self::app();
    assert!(crate::logs::open_logs(none.handle().clone(), none.state(), w(&none), target).await.is_err());
}

#[tokio::test]
async fn shell_windows_need_an_unlocked_context() {
    let api = MockApi::new();
    let app = connected(&api).await;
    let target = ExecTarget { namespace: "shop".into(), pod: "api-1".into(), container: "app".into() };
    let err = crate::exec::open_exec(app.handle().clone(), app.state(), w(&app), target.clone()).await.unwrap_err();
    assert!(err.contains("read-only"), "{err}");
    unlock(&app);
    crate::exec::open_exec(app.handle().clone(), app.state(), w(&app), target.clone()).await.unwrap();
    let label = app.webview_windows().into_keys().find(|l| l.starts_with("exec-")).unwrap();
    let window = app.get_webview_window(&label).unwrap();
    let session = crate::exec::exec_start(window.clone(), app.state(), w(&app), 80, 24).unwrap();
    assert_eq!(session.title, "app · shop/api-1 · acme-uat · Shell");
    crate::exec::exec_input(window.clone(), app.state(), "ls\n".into());
    crate::exec::exec_resize(window.clone(), app.state(), 100, 30);
    // Locking the context stops a reconnect.
    commands::set_unlocked(w(&app), CTX.into(), false);
    assert!(crate::exec::exec_start(window, app.state(), w(&app), 80, 24).is_err());
    let none = self::app();
    assert!(crate::exec::open_exec(none.handle().clone(), none.state(), w(&none), target).await.is_err());
}

#[tokio::test]
async fn port_forwards_start_list_and_stop() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/pods/api-1", pod("shop", "api-1", "node-a", json!({
        "spec": {"containers": [{"name": "app", "image": "acme/app:1", "ports": [{"name": "http", "containerPort": 8080}]}]}})));
    let app = connected(&api).await;
    let ports = crate::forward::forward_ports(w(&app), "shop".into(), ForwardSource::Pod { pod: "api-1".into() }).await.unwrap();
    assert_eq!(ports[0].port, 8080);
    let request = ForwardRequest { namespace: "shop".into(), source: ForwardSource::Pod { pod: "api-1".into() }, remote_port: 8080, local_port: 0 };
    let info = crate::forward::start_forward(app.handle().clone(), w(&app), request.clone()).await.unwrap();
    assert!(info.local_port > 0);
    assert_eq!((info.context.as_str(), info.remote_port), (CTX, 8080));
    assert_eq!(crate::forward::list_forwards(w(&app)).len(), 1);
    crate::forward::stop_forward(app.handle().clone(), w(&app), info.id);
    assert!(crate::forward::list_forwards(w(&app)).is_empty());
    let none = self::app();
    assert!(crate::forward::start_forward(none.handle().clone(), w(&none), request).await.is_err());
}

#[tokio::test]
async fn main_windows_keep_to_themselves() {
    let uat = MockApi::new();
    uat.on("GET", "/api/v1/namespaces/shop/pods/api-1", pod("shop", "api-1", "node-a", json!({
        "spec": {"containers": [{"name": "app", "image": "acme/app:1", "ports": [{"containerPort": 8080}]}]}})));
    let app = connected(&uat).await;
    let state = app.state::<AppState>();
    state.add_window("main-2");
    let second = state.window("main-2").unwrap();
    assert_eq!(second.label, "main-2");

    // Not connected just because the first window is.
    assert_eq!(commands::check_connection(second.clone()).await.unwrap_err(), "not connected to a cluster");
    // Connected to the same context, allowing changes in one doesn't in the other.
    *second.session.write().await = Some(Session { context: CTX.into(), client: uat.client(), history: Default::default() });
    unlock(&app);
    assert!(w(&app).gate.is_unlocked(CTX) && !second.gate.is_unlocked(CTX));
    let target = ExecTarget { namespace: "shop".into(), pod: "api-1".into(), container: "app".into() };
    let err = crate::exec::open_exec(app.handle().clone(), app.state(), second.clone(), target).await.unwrap_err();
    assert!(err.contains("read-only"), "{err}");

    // Each window lists only its own forwards.
    let request = ForwardRequest { namespace: "shop".into(), source: ForwardSource::Pod { pod: "api-1".into() }, remote_port: 8080, local_port: 0 };
    crate::forward::start_forward(app.handle().clone(), w(&app), request).await.unwrap();
    assert_eq!(crate::forward::list_forwards(w(&app)).len(), 1);
    assert!(crate::forward::list_forwards(second.clone()).is_empty());

    // The same logs open once per window, each resolving to its own main
    // window, which is the only one that closes it.
    let logs = LogTarget { namespace: "shop".into(), source: LogSource::Container { pod: "api-1".into(), container: "app".into() }, filter: None };
    crate::logs::open_logs(app.handle().clone(), app.state(), w(&app), logs.clone()).await.unwrap();
    crate::logs::open_logs(app.handle().clone(), app.state(), second.clone(), logs).await.unwrap();
    let labels: Vec<String> = app.webview_windows().into_keys().filter(|l| l.starts_with("logs-")).collect();
    assert_eq!(labels.len(), 2);
    let owners: Vec<String> = labels.iter().map(|l| state.owner(l).unwrap()).collect();
    assert!(owners.contains(&MAIN_LABEL.to_owned()) && owners.contains(&"main-2".to_owned()));
    let first_child = &labels[owners.iter().position(|o| o == MAIN_LABEL).unwrap()];
    assert_eq!(state.window(first_child).unwrap().label, MAIN_LABEL);
    assert_eq!(state.children(MAIN_LABEL), vec![first_child.clone()]);
    assert_eq!(state.children("main-2").len(), 1);
}

#[tokio::test]
async fn container_files_report_api_errors() {
    let api = MockApi::new();
    let app = connected(&api).await;
    // The fake server can't upgrade to a websocket: a clear error, no panic.
    let err = crate::upload::browse_dir(w(&app), "shop".into(), "api-1".into(), "app".into(), "/".into(), true).await.unwrap_err();
    assert!(!err.is_empty());
    let err = crate::upload::download_file(app.handle().clone(), w(&app), "shop".into(), "api-1".into(), "app".into(), "/etc/hosts".into())
        .await
        .unwrap_err();
    assert!(!err.is_empty());
}

// `menu::install` is covered by `tests/app.rs`: macOS only builds native
// menu items on the main thread, and only that test runs there.

#[test]
fn contexts_come_from_the_kubeconfig() {
    let _kubeconfig = kubeconfig_file(
        "apiVersion: v1\nkind: Config\ncurrent-context: acme-uat\n\
         clusters:\n- name: acme-uat\n  cluster: {server: 'https://127.0.0.1:1'}\n- name: acme-prd\n  cluster: {server: 'https://127.0.0.1:2'}\n\
         contexts:\n- name: acme-uat\n  context: {cluster: acme-uat, user: me, namespace: shop}\n- name: acme-prd\n  context: {cluster: acme-prd, user: me}\n\
         users:\n- name: me\n  user: {token: fake}\n",
    );
    let app = app();
    commands::set_unlocked(w(&app), "acme-uat".into(), true);
    let list = commands::list_contexts(app.state(), w(&app)).unwrap();
    assert_eq!(list.current.as_deref(), Some("acme-uat"));
    let uat = list.contexts.iter().find(|c| c.name == "acme-uat").unwrap();
    let prd = list.contexts.iter().find(|c| c.name == "acme-prd").unwrap();
    assert!(uat.unlocked && !uat.prod_like && uat.namespace.as_deref() == Some("shop"));
    assert!(prd.prod_like && !prd.unlocked);
}

#[tokio::test]
async fn overview_and_workloads_come_back_from_the_api() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/nodes", list("NodeList", vec![object("v1", "Node", None, "node-a", json!({
        "status": {"allocatable": {"cpu": "4", "memory": "16Gi", "pods": "110"}, "capacity": {"cpu": "4", "memory": "16Gi"},
                   "conditions": [{"type": "Ready", "status": "True"}]}}))]));
    api.on("GET", "/api/v1/pods", list("PodList", vec![pod("shop", "api-1", "node-a", json!({}))]));
    api.on("GET", "/api/v1/events", list("EventList", vec![]));
    api.on("GET", "/api/v1/services", list("ServiceList", vec![]));
    let deploy = object("apps/v1", "Deployment", Some("shop"), "api", json!({
        "spec": {"replicas": 1, "selector": {"matchLabels": {"app": "api"}}, "template": {"spec": {"containers": [{"name": "app", "image": "acme/app:1"}]}}},
        "status": {"replicas": 1, "readyReplicas": 1, "updatedReplicas": 1, "availableReplicas": 1}}));
    api.on("GET", "/apis/apps/v1/namespaces/shop/deployments/api", deploy);
    api.on("GET", "/apis/apps/v1/namespaces/shop/replicasets", list("ReplicaSetList", vec![]));
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![]));
    api.on("GET", "/api/v1/namespaces/shop/events", list("EventList", vec![]));
    let app = connected(&api).await;

    let overview = commands::cluster_overview(w(&app)).await.unwrap();
    assert_eq!(overview.nodes.len(), 1);
    assert_eq!(overview.nodes[0].name, "node-a");
    let details = commands::workload_details(w(&app), deployment(), "shop".into(), "api".into()).await.unwrap();
    assert_eq!((details.kind.as_str(), details.status.as_str()), ("Deployment", "Available"));
    let history = commands::workload_history(w(&app), deployment(), "shop".into(), "api".into()).await.unwrap();
    assert_eq!(history.source, subduer_core::types::HistorySource::Local);
    // No Prometheus, kubelet stats or metrics-server here: an empty local series.
    let history = commands::node_history(w(&app), "node-a".into()).await.unwrap();
    assert_eq!(history.source, subduer_core::types::HistorySource::Local);
    assert!(history.pod.cpu.is_empty());
    let err = commands::workload_details(w(&app), deployment(), "shop".into(), "gone".into()).await.unwrap_err();
    assert!(!err.is_empty());
}

#[tokio::test]
async fn edit_windows_load_save_and_guard_unsaved_edits() {
    use subduer_core::types::EditTarget;
    let api = MockApi::new();
    let path = "/api/v1/namespaces/shop/configmaps/app";
    api.on("GET", path, object("v1", "ConfigMap", Some("shop"), "app", json!({"data": {"a": "1"}})));
    api.on("PUT", path, object("v1", "ConfigMap", Some("shop"), "app", json!({"data": {"a": "2"}})));
    let app = connected(&api).await;
    let cm = kind("", "v1", "ConfigMap", "configmaps", true);
    let target = EditTarget { kind: cm.clone(), namespace: Some("shop".into()), name: "app".into() };

    // Helm releases aren't editable; opening twice focuses the same window.
    let helm = EditTarget { kind: subduer_core::helm::release_kind(), namespace: Some("shop".into()), name: "web".into() };
    assert!(crate::editor::open_editor(app.handle().clone(), app.state(), w(&app), helm).await.is_err());
    crate::editor::open_editor(app.handle().clone(), app.state(), w(&app), target.clone()).await.unwrap();
    crate::editor::open_editor(app.handle().clone(), app.state(), w(&app), target.clone()).await.unwrap();
    let labels: Vec<String> = app.webview_windows().into_keys().filter(|l| l.starts_with("edit-")).collect();
    assert_eq!(labels.len(), 1);
    let window = app.get_webview_window(&labels[0]).unwrap();

    let session = {
        // Whether it looks like production comes from the kubeconfig.
        let _kubeconfig = kubeconfig_file(
            "apiVersion: v1\nkind: Config\nclusters:\n- name: c\n  cluster: {server: 'https://127.0.0.1:1'}\n\
             contexts:\n- name: other\n  context: {cluster: c, user: me}\n- name: acme-uat\n  context: {cluster: c, user: me}\n\
             users:\n- name: me\n  user: {token: t}\n",
        );
        crate::editor::editor_start(window.clone(), app.state(), w(&app)).await.unwrap()
    };
    assert!(!session.prod_like);
    // A context that's gone from the kubeconfig goes by its production mark.
    {
        let _kubeconfig = kubeconfig_file("apiVersion: v1\nkind: Config\ncontexts: []\n");
        app.state::<AppState>().prod.lock().unwrap().set(CTX, None, true).unwrap();
        assert!(crate::editor::editor_start(window.clone(), app.state(), w(&app)).await.unwrap().prod_like);
        app.state::<AppState>().prod.lock().unwrap().set(CTX, None, false).unwrap();
    }
    assert_eq!(session.title, "ConfigMap shop/app · acme-uat · Edit");
    assert!(!session.unlocked && session.yaml.contains("a: '1'"));
    let edited = session.yaml.replace("a: '1'", "a: '2'");

    // A dry run works while locked; a real save needs the context unlocked.
    let checked = crate::editor::editor_save(window.clone(), app.state(), w(&app), edited.clone(), true).await.unwrap();
    assert!(checked.contains("a: '2'"));
    assert!(api.calls_to("PUT", path).pop().unwrap().query.contains("dryRun=All"));
    let err = crate::editor::editor_save(window.clone(), app.state(), w(&app), edited.clone(), false).await.unwrap_err();
    assert!(err.contains("read-only"), "{err}");
    let err = crate::editor::editor_save(window.clone(), app.state(), w(&app), "kind: [".into(), true).await.unwrap_err();
    assert!(err.starts_with("Not valid YAML"), "{err}");

    // Unsaved edits hold closing the main window or quitting.
    crate::editor::editor_dirty(window.clone(), app.state(), true);
    assert!(crate::editor::ask_dirty(app.handle(), None));
    unlock(&app);
    crate::editor::editor_save(window.clone(), app.state(), w(&app), edited, false).await.unwrap();
    assert!(!api.calls_to("PUT", path).pop().unwrap().query.contains("dryRun"));
    // Saving clears the unsaved flag.
    assert!(!crate::editor::ask_dirty(app.handle(), None));
    crate::editor::editor_dirty(window.clone(), app.state(), true);
    crate::editor::editor_close(window.clone(), app.state());
    assert!(!crate::editor::ask_dirty(app.handle(), None));

    // A window the app doesn't know about isn't connected; nor is a session-less app.
    let stray = WebviewWindowBuilder::new(&app, "edit-99", WebviewUrl::default()).build().unwrap();
    assert!(crate::editor::editor_start(stray.clone(), app.state(), w(&app)).await.is_err());
    assert!(crate::editor::editor_save(stray, app.state(), w(&app), String::new(), true).await.is_err());
    let none = self::app();
    assert!(crate::editor::open_editor(none.handle().clone(), none.state(), w(&none), target).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn traces_start_report_and_stop() {
    use crate::trace;
    use subduer_core::types::{TraceEnd, TraceScope};
    let api = MockApi::new();
    // No Prometheus: the trace keeps only the app's own samples.
    api.on("GET", "/api/v1/services", list("ServiceList", vec![]));
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![pod("shop", "api-1", "node-a", json!({}))]));
    for (path, kind) in [
        ("/apis/apps/v1/namespaces/shop/deployments", "DeploymentList"),
        ("/apis/apps/v1/namespaces/shop/statefulsets", "StatefulSetList"),
        ("/apis/apps/v1/namespaces/shop/daemonsets", "DaemonSetList"),
        ("/apis/batch/v1/namespaces/shop/cronjobs", "CronJobList"),
    ] {
        api.on("GET", path, list(kind, vec![]));
    }
    let job = |name: &str, owner: Option<&str>| {
        let refs: Vec<serde_json::Value> =
            owner.iter().map(|o| json!({"apiVersion": "batch/v1", "kind": "CronJob", "name": o, "uid": "u"})).collect();
        object("batch/v1", "Job", Some("shop"), name, json!({"metadata": {"ownerReferences": refs}}))
    };
    api.on("GET", "/apis/batch/v1/namespaces/shop/jobs", list("JobList", vec![job("nightly-1", Some("nightly")), job("once", None)]));
    let app = connected(&api).await;
    let s = || w(&app);
    let dir = std::env::temp_dir().join(format!("subduer-tauri-trace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("shop").display().to_string();
    let scope = TraceScope::Namespaces(vec!["shop".into()]);
    const EVERY: i64 = subduer_core::trace::DEFAULT_PULL_EVERY_SECS;

    let targets = trace::trace_targets(s(), "shop".into()).await.unwrap();
    assert_eq!(targets.workloads.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), vec!["once"]);
    assert_eq!(targets.pods, vec!["api-1"]);

    // Only the connected context.
    let other = trace::start_trace(app.handle().clone(), s(), "acme-prd".into(), scope.clone(), file.clone(), EVERY);
    assert_eq!(other.await.unwrap_err(), "not connected to acme-prd");
    // Only the Start dialog's intervals.
    let often = trace::start_trace(app.handle().clone(), s(), CTX.into(), scope.clone(), file.clone(), 300);
    assert_eq!(often.await.unwrap_err(), "can't pull every 300 s");
    assert_eq!(trace::trace_status(s()).await.unwrap(), None);
    assert!(!s().must_ask());

    let status = trace::start_trace(app.handle().clone(), s(), CTX.into(), scope.clone(), file.clone(), 1800).await.unwrap();
    assert_eq!(status.path, format!("{file}.trace"));
    // No Prometheus here; the hour of retention assumed still allows 30 minutes.
    assert_eq!((status.context.as_str(), status.prometheus, status.stopping, status.pull_every), (CTX, false, false, 1800.0));
    assert!(s().must_ask(), "closing and quitting ask while a trace runs");
    let again = trace::start_trace(app.handle().clone(), s(), CTX.into(), scope.clone(), file.clone(), EVERY);
    assert!(again.await.unwrap_err().contains("already running"));
    // Another window may trace too, but not into the same file.
    let second_window = app.state::<AppState>().add_window("main-2");
    *second_window.session.write().await = Some(Session { context: CTX.into(), client: api.client(), history: Default::default() });
    let w2 = app.state::<AppState>().window("main-2").unwrap();
    let same_file = trace::start_trace(app.handle().clone(), w2.clone(), CTX.into(), scope, file.clone(), EVERY);
    assert!(same_file.await.unwrap_err().contains("another window"));
    assert!(!w2.must_ask());
    assert_eq!(trace::trace_status(s()).await.unwrap().unwrap().path, status.path);
    // Only files this app recorded can be revealed.
    assert!(trace::reveal_trace(app.state(), s(), "/etc/hosts".into()).unwrap_err().contains("only traces recorded"));
    trace::reveal_trace(app.state(), s(), status.path.clone()).unwrap();
    let folder = dir.display().to_string();
    assert!(LAUNCHED.lock().unwrap().iter().any(|l| l.contains(&folder)));
    // The trace being recorded can be read, compared and reported on meanwhile.
    let review = trace::trace_review(s(), status.path.clone()).await.unwrap();
    assert!(review.summary.end_reason.is_none());
    let _ = trace::trace_compare(s(), vec![status.path.clone(), status.path.clone()]).await;
    let live = dir.join("live.html").display().to_string();
    assert_eq!(trace::export_trace_report(s(), status.path.clone(), live.clone()).await.unwrap(), live);
    let nowhere = dir.join("missing").join("r.html").display().to_string();
    assert!(trace::export_trace_report(s(), status.path.clone(), nowhere).await.unwrap_err().starts_with("can't write"));

    let summary = trace::stop_trace(app.handle().clone(), s(), TraceEnd::Stopped).await.unwrap();
    assert_eq!(summary.end_reason, Some(TraceEnd::Stopped));
    assert_eq!(summary.context, CTX);
    assert!(!s().must_ask());
    assert_eq!(trace::trace_status(s()).await.unwrap(), None);
    assert_eq!(trace::stop_trace(app.handle().clone(), s(), TraceEnd::Stopped).await.unwrap_err(), "no trace is running");

    // Quitting stops a running trace, with Quit as the reason.
    let scope = TraceScope::Pod { namespace: "shop".into(), name: "api-1".into() };
    let second = dir.join("second.trace").display().to_string();
    trace::start_trace(app.handle().clone(), s(), CTX.into(), scope, second.clone(), EVERY).await.unwrap();
    commands::close_discarding(app.handle().clone(), s(), false).await.unwrap();
    assert!(!s().must_ask());
    let read = subduer_core::trace::read_summary(std::path::Path::new(&second)).await.unwrap();
    assert_eq!(read.end_reason, Some(TraceEnd::Quit));

    // So does exiting without having asked (the Dock's Quit); with none running it returns at once.
    let third = dir.join("third.trace").display().to_string();
    let scope = TraceScope::Pod { namespace: "shop".into(), name: "api-1".into() };
    trace::start_trace(app.handle().clone(), s(), CTX.into(), scope, third.clone(), EVERY).await.unwrap();
    let handle = app.handle().clone();
    tokio::task::spawn_blocking(move || {
        trace::stop_on_exit(&handle.state::<AppState>().window(MAIN_LABEL).unwrap().traces);
        trace::stop_on_exit(&handle.state::<AppState>().window(MAIN_LABEL).unwrap().traces);
    })
    .await
    .unwrap();
    assert!(!s().must_ask());
    let read = subduer_core::trace::read_summary(std::path::Path::new(&third)).await.unwrap();
    assert_eq!((read.end_reason, read.unfinished), (Some(TraceEnd::Quit), false));

    // Review works on the file alone; this one has no usage (nothing was faked).
    let review = trace::trace_review(s(), second.clone()).await.unwrap();
    assert_eq!((review.summary.end_reason, review.rows.len()), (Some(TraceEnd::Quit), 0));
    let key = subduer_core::types::TraceKey { namespace: "shop".into(), kind: "Pod".into(), name: "api-1".into(), container: "app".into() };
    let err = trace::trace_series(second.clone(), key.clone(), None, None, 100).await.unwrap_err();
    assert!(err.contains("isn't in this trace"), "{err}");
    let err = trace::trace_overview(second.clone(), Some("shop".into()), None, None, 100).await.unwrap_err();
    assert_eq!(err, "no usage in shop in this trace");
    assert!(trace::trace_daily(second.clone(), key.clone()).await.is_err());
    assert!(trace::trace_review(s(), "/etc/hosts".into()).await.unwrap_err().contains("trace file"));

    // Notes, a comparison and a report work on the files alone.
    trace::trace_set_note(second.clone(), None, "Before the resize".into()).await.unwrap();
    trace::trace_set_note(second.clone(), Some(key.clone()), "Agreed".into()).await.unwrap();
    assert_eq!(trace::trace_review(s(), second.clone()).await.unwrap().note.as_deref(), Some("Before the resize"));
    let first = format!("{file}.trace");
    let cmp = trace::trace_compare(s(), vec![first.clone(), second.clone()]).await.unwrap();
    assert_eq!(cmp.traces.len(), 2);
    assert!(trace::trace_compare(s(), vec![first]).await.unwrap_err().contains("at least two"));
    let out = dir.join("report.html").display().to_string();
    assert_eq!(trace::export_trace_report(s(), second.clone(), out.clone()).await.unwrap(), out);
    assert!(std::fs::read_to_string(&out).unwrap().contains("Before the resize"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn rightsizing_needs_the_context_and_the_gate() {
    use subduer_core::types::{ContainerRequests, RightsizeChange};
    let path = "/apis/apps/v1/namespaces/shop/deployments/web";
    let api = MockApi::new();
    let web = object("apps/v1", "Deployment", Some("shop"), "web", json!({"spec": {"template": {"spec": {"containers": [
        {"name": "app", "image": "acme/web:1", "resources": {"requests": {"cpu": "500m"}}}]}}}}));
    api.on("GET", path, web.clone());
    api.on("PATCH", path, web);
    let app = connected(&api).await;
    let s = || w(&app);
    let change = RightsizeChange {
        kind: "Deployment".into(),
        namespace: "shop".into(),
        name: "web".into(),
        containers: vec![ContainerRequests { container: "app".into(), cpu_request: Some(50.0), mem_request: None }],
    };
    let other = commands::rightsize_preview(s(), "acme-prd".into(), change.clone()).await.unwrap_err();
    assert_eq!(other, "connected to acme-uat now, not acme-prd; nothing was previewed");
    let preview = commands::rightsize_preview(s(), CTX.into(), change.clone()).await.unwrap();
    assert_eq!(preview.containers[0].cpu_request.after.as_deref(), Some("50m"));
    assert!(api.calls_to("PATCH", path)[0].query.contains("dryRun=All"));
    let locked = commands::rightsize_apply(s(), CTX.into(), change.clone()).await.unwrap_err();
    assert!(locked.contains("read-only"), "{locked}");
    unlock(&app);
    commands::rightsize_apply(s(), CTX.into(), change).await.unwrap();
    assert!(!api.calls_to("PATCH", path).pop().unwrap().query.contains("dryRun"));
}

#[tokio::test]
async fn captures_are_gated_checked_and_dry_run_first() {
    use subduer_core::types::{CaptureFilter, CaptureRequest, CaptureTraffic};
    let all = || CaptureFilter { traffic: CaptureTraffic::All, handshakes_only: true };
    let api = MockApi::new();
    let app = connected(&api).await;
    let s = || w(&app);
    let dir = std::env::temp_dir().join(format!("subduer-tauri-capture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("api-1.pcap");
    let request = |secs: u32, filter: CaptureFilter| CaptureRequest {
        namespace: "shop".into(),
        pod: "api-1".into(),
        image: "acme/netshoot:1".into(),
        filter,
        secs,
        path: file.display().to_string(),
    };
    let start = |r: CaptureRequest| crate::capture::start_capture(app.handle().clone(), app.state(), s(), r);

    // It changes the pod, so it's refused while the context is locked.
    let locked = start(request(60, all())).await.unwrap_err();
    assert!(locked.contains("read-only"), "{locked}");
    unlock(&app);
    for (secs, filter, why) in [
        (0, all(), "1 to 30 minutes"),
        (subduer_core::capture::MAX_SECS + 1, all(), "1 to 30 minutes"),
        (60, CaptureFilter { traffic: CaptureTraffic::Custom("  ".into()), handshakes_only: true }, "type a filter"),
    ] {
        let err = start(request(secs, filter)).await.unwrap_err();
        assert!(err.contains(why), "{err}");
    }
    let nowhere = CaptureRequest { path: " ".into(), ..request(60, all()) };
    assert_eq!(start(nowhere).await.unwrap_err(), "choose where to save the capture");
    assert!(api.calls_to("PATCH", "/api/v1/namespaces/shop/pods/api-1/ephemeralcontainers").is_empty(), "nothing asked of the cluster yet");

    // The server refuses the dry run (here: RBAC): nothing is attached and no window opens.
    let path = "/api/v1/namespaces/shop/pods/api-1/ephemeralcontainers";
    api.on_error("PATCH", path, 403, "Forbidden", "cannot patch resource \"pods/ephemeralcontainers\"");
    let refused = start(request(60, all())).await.unwrap_err();
    assert!(refused.contains("cannot patch"), "{refused}");
    let patches = api.calls_to("PATCH", path);
    assert_eq!(patches.len(), 1);
    assert!(patches[0].query.contains("dryRun=All"));
    assert!(app.get_webview_window(&format!("{}1", crate::capture::LABEL_PREFIX)).is_none());
    assert!(!file.exists(), "no file is made when the cluster refuses");

    // The dry run passes but the real attach is refused: the file made for it goes again.
    let api = MockApi::new();
    api.on_query("PATCH", path, "dryRun", pod("shop", "api-1", "n1", json!({})));
    api.on_error("PATCH", path, 409, "Conflict", "the object has been modified");
    *w(&app).session.write().await = Some(Session { context: CTX.into(), client: api.client(), history: Default::default() });
    let refused = start(request(60, all())).await.unwrap_err();
    assert!(refused.contains("modified"), "{refused}");
    assert_eq!(api.calls_to("PATCH", path).len(), 2);
    assert!(!file.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn about_carries_every_license() {
    let app = app();
    let info = crate::about::about_info(app.handle().clone());
    assert!(info.license.starts_with("MIT License"));
    assert_eq!(info.maker, crate::menu::MAKER);
    // The crates the app is built on, and the vendored JavaScript.
    for name in ["\n  tauri ", "\n  kube ", "\n  leptos ", "Vendored JavaScript: ace", "Vendored JavaScript: xterm"] {
        assert!(info.notices.contains(name), "{name:?} missing from the notices");
    }
    // Our own crates aren't third-party.
    assert!(!info.notices.contains("\n  subduer-"));
}

#[test]
fn knows_which_contexts_other_windows_are_on() {
    let app = app();
    let state = app.state::<AppState>();
    *w(&app).chosen.lock().unwrap() = Some("acme-uat".into());
    *state.add_window("main-2").chosen.lock().unwrap() = Some("acme-prd".into());
    state.add_window("main-3");
    assert_eq!(state.in_use_elsewhere(MAIN_LABEL), ["acme-prd"]);
    assert_eq!(state.in_use_elsewhere("main-3"), ["acme-prd", "acme-uat"]);
    assert_eq!(crate::add_cluster::contexts_in_use_elsewhere(app.state(), w(&app)), ["acme-prd"]);
}

#[tokio::test]
async fn wont_remove_a_context_a_window_is_on() {
    let app = app();
    // Picked in another window, still connecting: no session yet.
    *app.state::<AppState>().add_window("main-2").chosen.lock().unwrap() = Some("acme-prd".into());
    let err = crate::add_cluster::remove_context(app.handle().clone(), "acme-prd".into()).await.unwrap_err();
    assert!(err.contains("Switch it to another context first"), "{err}");
    // Connected in this one.
    *w(&app).session.write().await = Some(Session { context: CTX.into(), client: MockApi::new().client(), history: Default::default() });
    let err = crate::add_cluster::remove_context(app.handle().clone(), CTX.into()).await.unwrap_err();
    assert!(err.contains(CTX), "{err}");
}
