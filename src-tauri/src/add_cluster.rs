//! File › Manage Clusters…: lists, reorders and removes the kubeconfig's
//! contexts, and adds one (typed in, or fetched by a cloud's CLI) to the
//! kubeconfig kubectl uses. Every main window's context list is refreshed
//! afterwards (event `CONTEXTS_CHANGED`).

use subduer_core::types::{AddedCluster, CloudAccounts, CloudCluster, CloudProvider, ManagedContexts, NewCluster, RemovedContext, cluster_events};
use subduer_core::{add_cluster, cloud, manage_clusters};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::oneshot;

use crate::{AppState, Win};
use crate::commands::CmdResult;

/// Runs file work off the async runtime.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> subduer_core::Result<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?.map_err(|e| e.user_message())
}

fn contexts_changed<R: Runtime>(app: &AppHandle<R>) {
    for window in app.state::<AppState>().main_windows() {
        let _ = app.emit_to(&window.label, cluster_events::CONTEXTS_CHANGED, ());
    }
}

/// Tells each main window which contexts the others are on (a window picked
/// another, or closed).
pub(crate) fn in_use_changed<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState>();
    for window in state.main_windows() {
        let _ = app.emit_to(&window.label, cluster_events::IN_USE_ELSEWHERE, state.in_use_elsewhere(&window.label));
    }
}

/// The contexts the other main windows are on, for a window that just opened.
#[tauri::command]
pub fn contexts_in_use_elsewhere(state: tauri::State<'_, AppState>, win: Win) -> Vec<String> {
    state.in_use_elsewhere(&win.label)
}

/// Marks `context` as production or not, overriding what its name suggests.
#[tauri::command]
pub fn set_production<R: Runtime>(app: AppHandle<R>, context: String, production: bool) -> CmdResult<()> {
    let kubeconfig = subduer_core::contexts::load_kubeconfig().map_err(|e| e.user_message())?;
    let cluster = kubeconfig.contexts.iter().find(|c| c.name == context).and_then(|c| c.context.as_ref()).map(|c| c.cluster.clone());
    app.state::<AppState>().prod.lock().unwrap().set(&context, cluster.as_deref(), production).map_err(|e| e.user_message())?;
    tracing::info!(context, production, "marked context");
    contexts_changed(&app);
    Ok(())
}

/// The kubeconfig new clusters go to, for the dialog to say.
#[tauri::command]
pub async fn kubeconfig_target() -> CmdResult<String> {
    blocking(|| add_cluster::kubeconfig_path().map(|p| p.display().to_string())).await
}

/// Writes a hand-entered cluster, refusing names already in use.
#[tauri::command]
pub async fn add_cluster<R: Runtime>(app: AppHandle<R>, cluster: NewCluster) -> CmdResult<AddedCluster> {
    let context = blocking(move || {
        let taken = add_cluster::TakenNames::read()?;
        add_cluster::add_cluster(&add_cluster::kubeconfig_path()?, &cluster, &taken)
    })
    .await?;
    contexts_changed(&app);
    Ok(AddedCluster { context, output: String::new() })
}

#[tauri::command]
pub async fn cloud_accounts(provider: CloudProvider) -> CmdResult<CloudAccounts> {
    cloud::accounts(provider).await.map_err(|e| e.user_message())
}

#[tauri::command]
pub async fn cloud_clusters(provider: CloudProvider, account: String, region: String) -> CmdResult<Vec<CloudCluster>> {
    cloud::clusters(provider, &account, &region).await.map_err(|e| e.user_message())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn add_cloud_cluster<R: Runtime>(
    app: AppHandle<R>,
    provider: CloudProvider,
    account: String,
    cluster: CloudCluster,
    alias: String,
    make_current: bool,
) -> CmdResult<AddedCluster> {
    let added = cloud::add(provider, &account, &cluster, &alias, make_current).await.map_err(|e| e.user_message())?;
    contexts_changed(&app);
    Ok(added)
}

/// Every context, in the order the context menu shows them.
#[tauri::command]
pub async fn managed_contexts() -> CmdResult<ManagedContexts> {
    blocking(|| manage_clusters::list(&add_cluster::kubeconfig_files()?)).await
}

/// Removes a context (and its cluster and user, if nothing else uses them).
/// Refused while a window is on it, connected or still connecting.
#[tauri::command]
pub async fn remove_context<R: Runtime>(app: AppHandle<R>, context: String) -> CmdResult<RemovedContext> {
    for window in app.state::<AppState>().main_windows() {
        let chosen = window.chosen.lock().unwrap().as_ref() == Some(&context);
        if chosen || window.session.read().await.as_ref().is_some_and(|s| s.context == context) {
            return Err(format!("A window is on {context:?}. Switch it to another context first."));
        }
    }
    let name = context.clone();
    let removed = blocking(move || manage_clusters::remove(&add_cluster::kubeconfig_files()?, &name)).await?;
    // Its production mark goes with it, so a new context of that name starts afresh.
    if let Err(e) = app.state::<AppState>().prod.lock().unwrap().forget(&context) {
        tracing::warn!(context, error = %e, "couldn't forget the context's production mark");
    }
    contexts_changed(&app);
    Ok(removed)
}

/// Puts the contexts in `order` (each only moves within its own file).
#[tauri::command]
pub async fn reorder_contexts<R: Runtime>(app: AppHandle<R>, order: Vec<String>) -> CmdResult<()> {
    blocking(move || manage_clusters::reorder(&add_cluster::kubeconfig_files()?, &order)).await?;
    contexts_changed(&app);
    Ok(())
}

/// The native Open dialog for a certificate or key. `None` if cancelled.
#[tauri::command]
pub async fn choose_cert_file<R: Runtime>(app: AppHandle<R>, title: String) -> CmdResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = oneshot::channel();
    let mut dialog = app.dialog().file().set_title(title);
    if let Some(home) = std::env::home_dir() {
        dialog = dialog.set_directory(home);
    }
    dialog.pick_file(move |path| {
        let _ = tx.send(path);
    });
    let Some(path) = rx.await.ok().flatten() else { return Ok(None) };
    Ok(Some(path.into_path().map_err(|e| e.to_string())?.display().to_string()))
}
