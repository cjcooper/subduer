//! The app as `main` starts it, on Tauri's mock runtime: process setup,
//! the app's setup (menus, the first main window, the kubeconfig watch),
//! commands over IPC, and quitting once the last window closes.
//!
//! No test harness: this runs on the process's main thread, the only one
//! macOS builds menus on. It changes the process's environment (a scratch
//! home, kubeconfig and login shell, and a stand-in `aws`) before any other
//! thread starts, so nothing here touches the real ones.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::menu::MenuItemKind;
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use tauri::{AppHandle, Manager, WebviewWindow};

/// A stand-in for the AWS CLI: two profiles' worth of answers, and
/// `update-kubeconfig` writing one context.
const AWS: &str = r#"#!/bin/sh
case "$*" in
  "configure list-profiles") echo dev ;;
  *"configure get region") echo eu-west-1 ;;
  *"eks list-clusters"*) echo '{"clusters": ["shop"]}' ;;
  *"eks describe-cluster"*) echo '{"cluster": {"version": "1.31", "status": "ACTIVE"}}' ;;
  *"eks update-kubeconfig"*)
    printf 'apiVersion: v1\nkind: Config\ncurrent-context: shop\nclusters:\n- name: shop\n  cluster: {server: "https://127.0.0.1:1"}\nusers:\n- name: shop\n  user: {token: t}\ncontexts:\n- name: shop\n  context: {cluster: shop, user: shop}\n' > "$KUBECONFIG"
    echo "Added new context shop" ;;
  *) echo "unexpected: $*" >&2; exit 1 ;;
esac
"#;

fn main() {
    let dir = std::env::temp_dir().join(format!("subduer-app-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let kubeconfig = dir.join("kubeconfig");
    std::fs::write(&kubeconfig, "apiVersion: v1\nkind: Config\ncontexts: []\n").unwrap();
    // SAFETY: no other thread has started yet.
    unsafe {
        std::env::set_var("HOME", dir.join("home"));
        std::env::set_var("XDG_DATA_HOME", dir.join("data"));
        std::env::set_var("XDG_DATA_DIRS", dir.join("system-data"));
        std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
        std::env::set_var("KUBECONFIG", &kubeconfig);
        std::env::set_var("SHELL", "/bin/sh");
        std::env::remove_var("AWS_PROFILE");
        std::env::remove_var("AWS_REGION");
        std::env::remove_var("AWS_DEFAULT_REGION");
    }
    subduer_lib::prepare();
    // After the login shell's PATH is in: the stand-in `aws` comes first.
    install_aws(&bin);

    // Main windows open without the system material: the mock's windows have
    // no native window to put it on.
    let (handles, handle) = mpsc::channel();
    let opaque = tauri::plugin::Builder::<MockRuntime>::new("test-opaque")
        .setup(move |app, _| {
            app.state::<subduer_lib::AppState>().opaque.store(true, Ordering::SeqCst);
            handles.send(app.clone()).unwrap();
            Ok(())
        })
        .build();
    let script = std::thread::spawn(move || drive(handle.recv().unwrap(), &kubeconfig));
    subduer_lib::start(mock_builder().plugin(opaque), mock_context(noop_assets()));
    script.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    println!("app: ok");
}

#[cfg(unix)]
fn install_aws(bin: &Path) {
    use std::os::unix::fs::PermissionsExt;
    // And an Azure CLI that isn't signed in.
    for (name, script) in [("aws", AWS), ("az", "#!/bin/sh\necho 'Please run az login.' >&2\nexit 1\n")] {
        let cli = bin.join(name);
        std::fs::write(&cli, script).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    // SAFETY: still no other thread.
    unsafe { std::env::set_var("PATH", path) };
}

#[cfg(not(unix))]
fn install_aws(_: &Path) {}

/// What the user does while the app runs, then closing its window.
fn drive(app: AppHandle<MockRuntime>, kubeconfig: &PathBuf) {
    let main = wait_for_main(&app);
    check_menu(&app);
    let call = |cmd: &str, args: Value| invoke(&main, cmd, args);
    assert_eq!(call("contexts_in_use_elsewhere", json!({})), Ok(json!([])));
    assert_eq!(call("trace_status", json!({})), Ok(Value::Null));
    if cfg!(unix) {
        let accounts = call("cloud_accounts", json!({"provider": "eks"})).unwrap();
        assert_eq!(accounts["accounts"][0], json!({"id": "dev", "label": "dev", "region": "eu-west-1"}));
        assert_eq!(accounts["region"], "eu-west-1");
        let clusters = call("cloud_clusters", json!({"provider": "eks", "account": "dev", "region": "eu-west-1"})).unwrap();
        assert_eq!((clusters[0]["name"].as_str(), clusters[0]["version"].as_str()), (Some("shop"), Some("1.31")));
        let args = json!({"provider": "eks", "account": "dev", "cluster": clusters[0], "alias": "", "make_current": true});
        let added = call("add_cloud_cluster", args).unwrap();
        assert_eq!(added["context"], "shop");
        assert!(std::fs::read_to_string(kubeconfig).unwrap().contains("current-context: shop"));
        let signed_out = call("cloud_accounts", json!({"provider": "aks"})).unwrap_err();
        assert!(signed_out.as_str().unwrap().contains("az login"), "{signed_out}");
    }
    // Nothing unsaved: closing the last window quits.
    main.close().unwrap();
}

fn wait_for_main(app: &AppHandle<MockRuntime>) -> WebviewWindow<MockRuntime> {
    for _ in 0..500 {
        if let Some(main) = app.get_webview_window("main") {
            return main;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("the main window never opened");
}

/// The menu as installed: About, Settings…, New Window, Manage Clusters…,
/// Trace… and our own Quit are all there.
fn check_menu(app: &AppHandle<MockRuntime>) {
    let (tx, rx) = mpsc::channel();
    let menu = app.menu().expect("a menu");
    app.run_on_main_thread(move || {
        let mut ids = Vec::new();
        for item in menu.items().unwrap() {
            if let MenuItemKind::Submenu(sub) = item {
                ids.extend(sub.items().unwrap().iter().map(|i| i.id().as_ref().to_owned()));
            }
        }
        tx.send(ids).unwrap();
    })
    .unwrap();
    let ids = rx.recv().unwrap();
    for id in ["about", "settings", "new-window", "manage-clusters", "trace", "quit"] {
        assert!(ids.iter().any(|i| i == id), "{id} missing from {ids:?}");
    }
}

fn invoke(window: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    let request = InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: if cfg!(windows) { "http://tauri.localhost" } else { "tauri://localhost" }.parse().unwrap(),
        body: InvokeBody::Json(args),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.into(),
    };
    get_ipc_response(window, request).map(|b| b.deserialize().unwrap())
}
