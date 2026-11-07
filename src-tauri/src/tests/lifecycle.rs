//! Windows opening, asking before they close, closing, and quitting; the
//! menu's items; and following kubeconfig changes.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use subduer_core::types::{EditTarget, cluster_events, edit_events};
use tauri::menu::{MenuEvent, MenuId};
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::{Listener, RunEvent, WindowEvent};

use super::*;
use crate::{UNSAVED_CLOSE, menu};

/// Every `name` event the app sends, as its JSON payload.
fn record(app: &App<MockRuntime>, name: &str) -> Arc<Mutex<Vec<String>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let into = seen.clone();
    app.listen_any(name, move |event| into.lock().unwrap().push(event.payload().to_owned()));
    seen
}

/// An app whose main windows open without the system material: the mock's
/// windows have no native window to put it on.
fn opaque_app() -> App<MockRuntime> {
    let app = app();
    app.state::<AppState>().opaque.store(true, Ordering::SeqCst);
    app
}

/// Opens an edit window from the main window and marks it as holding unsaved YAML.
async fn dirty_editor(app: &App<MockRuntime>) -> String {
    let target = EditTarget { kind: kind("", "v1", "ConfigMap", "configmaps", true), namespace: Some("shop".into()), name: "app".into() };
    crate::editor::open_editor(app.handle().clone(), app.state(), w(app), target).await.unwrap();
    let label = app.webview_windows().into_keys().find(|l| l.starts_with("edit-")).unwrap();
    crate::editor::editor_dirty(app.get_webview_window(&label).unwrap(), app.state(), true);
    label
}

#[tokio::test]
async fn closing_asks_first_about_unsaved_work() {
    let app = connected(&MockApi::new()).await;
    let asked = record(&app, UNSAVED_CLOSE);
    let edit_asked = record(&app, edit_events::CLOSE_REQUESTED);
    // Nothing unsaved: windows close, and the app quits.
    assert!(!crate::hold_close(app.handle(), MAIN_LABEL));
    assert!(!crate::hold_close(app.handle(), "logs-1"));
    assert!(!crate::hold_exit(app.handle()));

    // Unsaved edits in the main window: it asks.
    commands::set_unsaved(w(&app), true);
    assert!(crate::hold_close(app.handle(), MAIN_LABEL));
    assert!(crate::hold_exit(app.handle()), "no webview to focus, but it still asks");
    WebviewWindowBuilder::new(&app, MAIN_LABEL, WebviewUrl::default()).build().unwrap();
    assert!(crate::hold_exit(app.handle()));
    assert_eq!(*asked.lock().unwrap(), ["false", "true", "true"]);
    commands::set_unsaved(w(&app), false);

    // An edit window with unsaved YAML asks for itself, for its main window and for quitting.
    let edit = dirty_editor(&app).await;
    assert!(crate::hold_close(app.handle(), &edit));
    assert!(crate::hold_close(app.handle(), MAIN_LABEL));
    assert!(crate::hold_exit(app.handle()));
    assert_eq!(edit_asked.lock().unwrap().len(), 3);
    assert_eq!(asked.lock().unwrap().len(), 3, "the edit window asked, not the main window");
}

#[tokio::test]
async fn closed_windows_stop_what_they_ran() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/pods/api-1", pod("shop", "api-1", "node-a", json!({
        "spec": {"containers": [{"name": "app", "image": "acme/app:1", "ports": [{"containerPort": 8080}]}]}})));
    let app = connected(&api).await;
    unlock(&app);
    let state = app.state::<AppState>();
    let in_use = record(&app, cluster_events::IN_USE_ELSEWHERE);
    let logs = LogTarget { namespace: "shop".into(), source: LogSource::Container { pod: "api-1".into(), container: "app".into() }, filter: None };
    crate::logs::open_logs(app.handle().clone(), app.state(), w(&app), logs).await.unwrap();
    let shell = ExecTarget { namespace: "shop".into(), pod: "api-1".into(), container: "app".into() };
    crate::exec::open_exec(app.handle().clone(), app.state(), w(&app), shell).await.unwrap();
    let edit = dirty_editor(&app).await;
    let request = ForwardRequest { namespace: "shop".into(), source: ForwardSource::Pod { pod: "api-1".into() }, remote_port: 8080, local_port: 0 };
    crate::forward::start_forward(app.handle().clone(), w(&app), request).await.unwrap();
    commands::watch_resources(app.handle().clone(), w(&app), deployment(), None).await.unwrap();
    let children = state.children(MAIN_LABEL);
    assert_eq!(children.len(), 3);
    let child = |prefix: &str| children.iter().find(|l| l.starts_with(prefix)).unwrap().clone();
    let (logs, shell) = (child("logs-"), child("exec-"));
    crate::logs::logs_start(app.get_webview_window(&logs).unwrap(), app.state(), false).unwrap();
    crate::exec::exec_start(app.get_webview_window(&shell).unwrap(), app.state(), w(&app), 80, 24).unwrap();

    // A child closing forgets it, and stops its session; its main window stays.
    crate::window_event(app.handle(), &edit, &WindowEvent::Destroyed);
    assert!(!state.editors.is_dirty(&edit));
    crate::window_event(app.handle(), &shell, &WindowEvent::Destroyed);
    assert!(crate::exec::exec_start(app.get_webview_window(&shell).unwrap(), app.state(), w(&app), 80, 24).is_err());
    assert_eq!(state.children(MAIN_LABEL), std::slice::from_ref(&logs));
    assert!(in_use.lock().unwrap().is_empty());
    // Other events change nothing.
    crate::window_event(app.handle(), MAIN_LABEL, &WindowEvent::Focused(true));
    crate::window_event(app.handle(), MAIN_LABEL, &WindowEvent::ThemeChanged(tauri::Theme::Dark));
    assert!(state.window(MAIN_LABEL).is_some());

    // The main window closing closes the rest and stops its watch and forwards.
    let main = w(&app);
    crate::window_event(app.handle(), MAIN_LABEL, &WindowEvent::Destroyed);
    assert!(state.window(MAIN_LABEL).is_none());
    assert!(main.watch.lock().unwrap().is_none());
    assert!(crate::forward::list_forwards(main).is_empty());
    // Its logs and shell windows were destroyed (the mock never reports
    // them gone, so their own clean-up is checked above), and forgotten.
    assert!(state.children(MAIN_LABEL).is_empty());
    // Its logs window, destroyed with it, stops its stream as it goes.
    crate::window_event(app.handle(), &logs, &WindowEvent::Destroyed);
    assert!(crate::logs::logs_start(app.get_webview_window(&logs).unwrap(), app.state(), false).is_err());
    // Closing it again, or a window the app never knew, is harmless.
    crate::window_event(app.handle(), MAIN_LABEL, &WindowEvent::Destroyed);
    crate::window_event(app.handle(), "about", &WindowEvent::Destroyed);
}

#[test]
fn the_running_app_holds_closing_and_quitting() {
    let app = mock_builder().build(mock_context(noop_assets())).unwrap();
    app.manage(app_state());
    app.state::<AppState>().opaque.store(true, Ordering::SeqCst);
    crate::open_main_window(app.handle(), MAIN_LABEL).unwrap();
    let asked = record(&app, UNSAVED_CLOSE);
    let main = app.state::<AppState>().window(MAIN_LABEL).unwrap();
    main.unsaved.store(true, Ordering::SeqCst);
    let handle = app.handle().clone();
    let started = Arc::new(Mutex::new(false));
    let ready = started.clone();
    let step = || std::thread::sleep(Duration::from_millis(1300));
    let script = std::thread::spawn(move || {
        // The app is running once it's ready.
        while !*ready.lock().unwrap() {
            std::thread::sleep(Duration::from_millis(20));
        }
        let window = handle.get_webview_window(MAIN_LABEL).unwrap();
        // Unsaved edits: closing the main window waits for the UI.
        window.close().unwrap();
        step();
        // It goes anyway (as the Dock's Quit would): quitting waits too.
        window.destroy().unwrap();
        step();
        // Saved; a last window opens and closes, and the app quits.
        main.unsaved.store(false, Ordering::SeqCst);
        let (tx, rx) = std::sync::mpsc::channel();
        let opener = handle.clone();
        handle
            .run_on_main_thread(move || {
                tx.send(WebviewWindowBuilder::new(&opener, "late", WebviewUrl::default()).build().unwrap()).unwrap();
            })
            .unwrap();
        rx.recv().unwrap().close().unwrap();
    });
    app.run(move |app, event| {
        if matches!(event, RunEvent::Ready) {
            *started.lock().unwrap() = true;
        }
        crate::on_run_event(app, event);
    });
    script.join().unwrap();
    assert_eq!(*asked.lock().unwrap(), ["false", "true"]);
}

#[test]
fn about_opens_once() {
    let app = app();
    crate::about::open(app.handle()).unwrap();
    crate::about::open(app.handle()).unwrap();
    assert_eq!(app.webview_windows().keys().filter(|l| *l == crate::about::LABEL).count(), 1);
}

#[test]
fn menu_items_do_what_they_say() {
    let app = opaque_app();
    let item = |id: &str| menu::on_menu_event(app.handle(), MenuEvent { id: MenuId::new(id) });
    // Nothing to send Settings… to yet.
    let settings = record(&app, menu::OPEN_SETTINGS);
    item("settings");
    assert!(settings.lock().unwrap().is_empty());
    item("about");
    assert!(app.get_webview_window(crate::about::LABEL).is_some());
    item("new-window");
    assert!(app.get_webview_window("main-2").is_some() && app.state::<AppState>().window("main-2").is_some());
    // A label that's taken: the window can't open, and isn't half-made.
    WebviewWindowBuilder::new(&app, "main-3", WebviewUrl::default()).build().unwrap();
    item("new-window");
    assert!(app.state::<AppState>().window("main-3").is_none());
    // With the first main window open, the dialog items go to it.
    WebviewWindowBuilder::new(&app, MAIN_LABEL, WebviewUrl::default()).build().unwrap();
    for (id, event) in [("settings", menu::OPEN_SETTINGS), ("trace", menu::OPEN_TRACE), ("manage-clusters", menu::OPEN_MANAGE_CLUSTERS)] {
        let seen = record(&app, event);
        item(id);
        assert_eq!(seen.lock().unwrap().len(), 1, "{id}");
    }
    item("something-else");
    // Quit asks Tauri's runtime to exit, which the mock leaves unimplemented.
    let quit = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| item("quit")));
    assert!(panic_text(quit.unwrap_err()).contains("not implemented"));
}

#[tokio::test]
async fn closing_and_quitting_after_discarding() {
    let app = app();
    let handle = app.handle().clone();
    let quit = tokio::spawn(commands::close_discarding(handle, w(&app), true)).await.unwrap_err();
    assert!(panic_text(quit.into_panic()).contains("not implemented"), "asked the runtime to exit");
}

/// A kubeconfig with `contexts`, all on one cluster, the first current.
fn kubeconfig(contexts: &[&str]) -> String {
    let mut text = String::from("apiVersion: v1\nkind: Config\nclusters:\n- name: c\n  cluster: {server: 'https://127.0.0.1:1'}\nusers:\n- name: me\n  user: {token: t}\n");
    text.push_str(if contexts.is_empty() { "contexts: []\n" } else { "contexts:\n" });
    for name in contexts {
        text.push_str(&format!("- name: {name}\n  context: {{cluster: c, user: me}}\n"));
    }
    if let Some(first) = contexts.first() {
        text.push_str(&format!("current-context: {first}\n"));
    }
    text
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("subduer-tauri-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test(flavor = "multi_thread")]
async fn kubeconfig_changes_are_followed_once_settled() {
    let dir = scratch("watch");
    let file = dir.join("config");
    std::fs::write(&file, kubeconfig(&["gone", "lab"])).unwrap();
    let app = app();
    *w(&app).chosen.lock().unwrap() = Some("gone".into());
    let changed = record(&app, cluster_events::CONTEXTS_CHANGED);
    let removed = record(&app, cluster_events::CONTEXT_REMOVED);
    let files = vec![file.clone()];
    let watch = tokio::spawn(crate::kubeconfig_watch::watch(app.handle().clone(), move || files.clone(), Duration::from_millis(40)));
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(changed.lock().unwrap().is_empty(), "nothing changed yet");
    // Another program is writing it: nothing happens while its lock is held.
    let lock = dir.join("config.lock");
    std::fs::write(&lock, "").unwrap();
    std::fs::write(&file, kubeconfig(&["lab", "ops"])).unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(changed.lock().unwrap().is_empty(), "held while locked");
    std::fs::remove_file(&lock).unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    watch.abort();
    assert_eq!(changed.lock().unwrap().len(), 1);
    assert_eq!(removed.lock().unwrap().len(), 1);
    let moved = removed.lock().unwrap()[0].clone();
    assert!(moved.contains("gone") && moved.contains("lab"), "{moved}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn with_no_contexts_left_one_window_stays_and_offers_add_cluster() {
    let dir = scratch("empty");
    let file = dir.join("config");
    let app = connected(&MockApi::new()).await;
    let state = app.state::<AppState>();
    *w(&app).chosen.lock().unwrap() = Some(CTX.into());
    // Another main window with a webview, and one whose webview never opened.
    state.add_window("main-2");
    WebviewWindowBuilder::new(&app, "main-2", WebviewUrl::default()).build().unwrap();
    state.add_window("main-3");
    let none = record(&app, cluster_events::NO_CONTEXTS);
    // Unreadable: wait for the next change.
    std::fs::write(&file, "contexts: [").unwrap();
    crate::kubeconfig_watch::settle(app.handle(), std::slice::from_ref(&file)).await;
    assert!(none.lock().unwrap().is_empty());
    std::fs::write(&file, kubeconfig(&[])).unwrap();
    crate::kubeconfig_watch::settle(app.handle(), std::slice::from_ref(&file)).await;
    assert_eq!(none.lock().unwrap().len(), 1);
    assert!(w(&app).session.read().await.is_none());
    assert!(w(&app).chosen.lock().unwrap().is_none());
    // A kept window that has gone meanwhile is left be.
    crate::kubeconfig_watch::empty(app.handle(), "main-9").await;
    assert_eq!(none.lock().unwrap().len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}
