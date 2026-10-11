//! IPC commands invoked from the UI. Errors are returned as display strings.

use std::sync::atomic::Ordering;

use subduer_core::types::{
    ClusterInfo, ClusterOverview, ContextList, DataChange, KindRef, ObjectDetails, PodDetails, PodHistory, Replicas,
    SecretValues, WorkloadDetails, events,
};
use subduer_core::{actions, cluster, config, contexts, overview, pod_details, watch, workload};
use tauri::{AppHandle, Emitter, State, Runtime};

use crate::{AppState, Session};

pub(crate) type CmdResult<T> = Result<T, String>;

pub(crate) fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Re-reads the kubeconfig every time so external edits show up.
#[tauri::command]
pub fn list_contexts(state: State<'_, AppState>) -> CmdResult<ContextList> {
    let kubeconfig = contexts::load_kubeconfig().map_err(err)?;
    Ok(contexts::list_contexts(&kubeconfig, &state.gate))
}

#[tauri::command]
pub async fn connect(state: State<'_, AppState>, context: String) -> CmdResult<ClusterInfo> {
    state.stop_watch();
    let kubeconfig = contexts::load_kubeconfig().map_err(err)?;
    let client = contexts::client_for(&kubeconfig, &context).await.map_err(err)?;
    let kinds = cluster::discover_kinds(&client).await.map_err(err)?;
    // Listing namespaces can be forbidden by RBAC; fall back to the context's default.
    let namespaces = match cluster::list_namespaces(&client).await {
        Ok(ns) => ns,
        Err(e) => {
            tracing::warn!(error = %e, "cannot list namespaces");
            vec![client.default_namespace().to_owned()]
        }
    };
    *state.session.write().await = Some(Session {
        context: context.clone(),
        client,
        history: Default::default(),
    });
    Ok(ClusterInfo {
        context,
        kinds,
        namespaces,
    })
}

/// Asked by the UI after a watch or a refresh gives up: an error means the
/// connection itself is gone (an expired token, say), and the UI offers
/// Connect instead of retrying.
#[tauri::command]
pub async fn check_connection(state: State<'_, AppState>) -> CmdResult<()> {
    let client = connected_client(&state).await?;
    cluster::check_connection(&client).await.map_err(err)
}

/// Unlocking is confirmed in the UI (twice for production-like contexts).
#[tauri::command]
pub fn set_unlocked(state: State<'_, AppState>, context: String, unlocked: bool) {
    tracing::info!(%context, unlocked, "safety gate changed");
    state.gate.set_unlocked(&context, unlocked);
}

/// Replaces the current watch. Snapshots arrive as `resource-table` events
/// tagged with the returned id.
#[tauri::command]
pub async fn watch_resources<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    kind: KindRef,
    namespace: Option<String>,
) -> CmdResult<u64> {
    let client = connected_client(&state).await?;
    let watch_id = state.next_watch_id.fetch_add(1, Ordering::Relaxed) + 1;
    let handle = tauri::async_runtime::spawn(async move {
        watch::watch_table(client, kind, namespace, watch_id, |table| {
            if let Err(e) = app.emit(events::RESOURCE_TABLE, table) {
                tracing::warn!(error = %e, "emit failed");
            }
        })
        .await;
    });
    if let Some(old) = state.watch.lock().unwrap().replace(handle) {
        old.abort();
    }
    Ok(watch_id)
}

#[tauri::command]
pub fn stop_watch(state: State<'_, AppState>) {
    state.stop_watch();
}

#[tauri::command]
pub async fn get_yaml(
    state: State<'_, AppState>,
    kind: KindRef,
    namespace: Option<String>,
    name: String,
) -> CmdResult<String> {
    let client = connected_client(&state).await?;
    cluster::get_yaml(&client, &kind, namespace.as_deref(), &name)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn cluster_overview(state: State<'_, AppState>) -> CmdResult<ClusterOverview> {
    let (client, history) = {
        let session = state.session.read().await;
        let session = session.as_ref().ok_or("not connected to a cluster")?;
        (session.client.clone(), session.history.clone())
    };
    overview::cluster_overview(&client, &history).await.map_err(err)
}

#[tauri::command]
pub async fn pod_details(state: State<'_, AppState>, namespace: String, name: String) -> CmdResult<PodDetails> {
    let client = connected_client(&state).await?;
    pod_details::pod_details(&client, &namespace, &name).await.map_err(err)
}

/// The pod's CPU and memory over the last hour, polled by the UI every 15 s.
#[tauri::command]
pub async fn pod_history(state: State<'_, AppState>, namespace: String, name: String) -> CmdResult<PodHistory> {
    let (client, history) = {
        let session = state.session.read().await;
        let session = session.as_ref().ok_or("not connected to a cluster")?;
        (session.client.clone(), session.history.clone())
    };
    history.pod_history(&client, &namespace, &name).await.map_err(err)
}

/// Details tab data for a Deployment or StatefulSet.
#[tauri::command]
pub async fn workload_details(
    state: State<'_, AppState>,
    kind: KindRef,
    namespace: String,
    name: String,
) -> CmdResult<WorkloadDetails> {
    let client = connected_client(&state).await?;
    workload::workload_details(&client, &kind, &namespace, &name)
        .await
        .map_err(|e| e.user_message())
}

/// Last hour of a workload's CPU/memory, summed over its pods; polled every 15 s.
#[tauri::command]
pub async fn workload_history(
    state: State<'_, AppState>,
    kind: KindRef,
    namespace: String,
    name: String,
) -> CmdResult<PodHistory> {
    let (client, history) = {
        let session = state.session.read().await;
        let session = session.as_ref().ok_or("not connected to a cluster")?;
        (session.client.clone(), session.history.clone())
    };
    history
        .workload_history(&client, &kind, &namespace, &name)
        .await
        .map_err(|e| e.user_message())
}

/// Deletes one object. Refused unless the current context is unlocked, and
/// unless `context` (what the confirmation showed) is still the connected one.
#[tauri::command]
pub async fn delete_resource(
    state: State<'_, AppState>,
    context: String,
    kind: KindRef,
    namespace: Option<String>,
    name: String,
) -> CmdResult<()> {
    let client = client_for_change(&state, &context, "deleted").await?;
    actions::delete(&client, &state.gate, &context, &kind, namespace.as_deref(), &name, false)
        .await
        .map_err(|e| e.user_message())
}

/// Desired and current replicas of a Deployment or StatefulSet, for the Scale dialog.
#[tauri::command]
pub async fn get_replicas(
    state: State<'_, AppState>,
    kind: KindRef,
    namespace: String,
    name: String,
) -> CmdResult<Replicas> {
    let client = connected_client(&state).await?;
    actions::replicas(&client, &kind, &namespace, &name)
        .await
        .map_err(|e| e.user_message())
}

/// Rollout restart, checked like `delete_resource`.
#[tauri::command]
pub async fn restart_workload(
    state: State<'_, AppState>,
    context: String,
    kind: KindRef,
    namespace: String,
    name: String,
) -> CmdResult<()> {
    let client = client_for_change(&state, &context, "restarted").await?;
    actions::restart(&client, &state.gate, &context, &kind, &namespace, &name, false)
        .await
        .map_err(|e| e.user_message())
}

/// Sets replicas, checked like `delete_resource`.
#[tauri::command]
pub async fn scale_workload(
    state: State<'_, AppState>,
    context: String,
    kind: KindRef,
    namespace: String,
    name: String,
    replicas: i32,
) -> CmdResult<()> {
    let client = client_for_change(&state, &context, "scaled").await?;
    actions::scale(&client, &state.gate, &context, &kind, &namespace, &name, replicas, false)
        .await
        .map_err(|e| e.user_message())
}

/// Suspends or resumes a Job or CronJob, checked like `delete_resource`.
#[tauri::command]
pub async fn suspend_workload(
    state: State<'_, AppState>,
    context: String,
    kind: KindRef,
    namespace: String,
    name: String,
    suspend: bool,
) -> CmdResult<()> {
    let client = client_for_change(&state, &context, if suspend { "suspended" } else { "resumed" }).await?;
    actions::set_suspended(&client, &state.gate, &context, &kind, &namespace, &name, suspend, false)
        .await
        .map_err(|e| e.user_message())
}

/// Starts a Job from a CronJob now, checked like `delete_resource`. Returns the Job's name.
#[tauri::command]
pub async fn run_cron_job(state: State<'_, AppState>, context: String, namespace: String, name: String) -> CmdResult<String> {
    let client = client_for_change(&state, &context, "started").await?;
    actions::run_cron_job(&client, &state.gate, &context, &namespace, &name, false)
        .await
        .map_err(|e| e.user_message())
}

/// The connected client, provided `context` (what a confirmation dialog
/// showed) is still the connected one. `done` names the change for the error.
/// What applying new requests to one workload would do: planned values, the
/// Helm release, and a server dry run. Needs the change's context to be the
/// connected one; changes nothing, so not gated.
#[tauri::command]
pub async fn rightsize_preview(
    state: State<'_, AppState>,
    context: String,
    change: subduer_types::RightsizeChange,
) -> CmdResult<subduer_types::RightsizePreview> {
    let client = client_for_change(&state, &context, "previewed").await?;
    subduer_core::rightsize::preview(&client, &change).await.map_err(|e| e.user_message())
}

/// Applies new requests to one workload (one patch, one rollout), checked
/// like `delete_resource`.
#[tauri::command]
pub async fn rightsize_apply(state: State<'_, AppState>, context: String, change: subduer_types::RightsizeChange) -> CmdResult<()> {
    let client = client_for_change(&state, &context, "changed").await?;
    subduer_core::rightsize::apply(&client, &state.gate, &context, &change).await.map_err(|e| e.user_message())
}

async fn client_for_change(state: &AppState, context: &str, done: &str) -> CmdResult<subduer_core::kube::Client> {
    let session = state.session.read().await;
    let session = session.as_ref().ok_or("not connected to a cluster")?;
    if session.context != context {
        return Err(format!("connected to {} now, not {context}; nothing was {done}", session.context));
    }
    Ok(session.client.clone())
}

/// Details tab of a Config-section kind (ConfigMap, Secret, HPA, PDB, …).
#[tauri::command]
pub async fn object_details(
    state: State<'_, AppState>,
    kind: KindRef,
    namespace: Option<String>,
    name: String,
) -> CmdResult<ObjectDetails> {
    let client = connected_client(&state).await?;
    config::object_details(&client, &kind, namespace.as_deref(), &name)
        .await
        .map_err(|e| e.user_message())
}

/// Every value of a Secret, when the user starts editing it. Returned to the
/// UI only; never logged.
#[tauri::command]
pub async fn secret_values(state: State<'_, AppState>, namespace: String, name: String) -> CmdResult<SecretValues> {
    let client = connected_client(&state).await?;
    tracing::info!(%namespace, secret = %name, "secret values revealed for editing");
    pod_details::secret_values(&client, &namespace, &name).await.map_err(err)
}

/// Saves edited ConfigMap or Secret data, checked like `delete_resource`.
#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
pub async fn save_config_data(
    state: State<'_, AppState>,
    context: String,
    kind: KindRef,
    namespace: String,
    name: String,
    resource_version: String,
    changes: Vec<DataChange>,
) -> CmdResult<()> {
    let client = client_for_change(&state, &context, "saved").await?;
    actions::save_data(&client, &state.gate, &context, &kind, &namespace, &name, &resource_version, &changes, false)
        .await
        .map_err(|e| e.user_message())
}

/// Sets an HPA's min and max replicas, checked like `delete_resource`.
#[tauri::command]
pub async fn set_hpa_range(
    state: State<'_, AppState>,
    context: String,
    kind: KindRef,
    namespace: String,
    name: String,
    min: i32,
    max: i32,
) -> CmdResult<()> {
    let client = client_for_change(&state, &context, "changed").await?;
    actions::set_hpa_range(&client, &state.gate, &context, &kind, &namespace, &name, min, max, false)
        .await
        .map_err(|e| e.user_message())
}

/// Grows a PersistentVolumeClaim, checked like `delete_resource`.
#[tauri::command]
pub async fn expand_claim(
    state: State<'_, AppState>,
    context: String,
    namespace: String,
    name: String,
    size: String,
) -> CmdResult<()> {
    let client = client_for_change(&state, &context, "changed").await?;
    actions::expand_claim(&client, &state.gate, &context, &namespace, &name, &size, false)
        .await
        .map_err(|e| e.user_message())
}

/// Sets a PersistentVolume's reclaim policy, checked like `delete_resource`.
#[tauri::command]
pub async fn set_reclaim_policy(state: State<'_, AppState>, context: String, name: String, policy: String) -> CmdResult<()> {
    let client = client_for_change(&state, &context, "changed").await?;
    actions::set_reclaim_policy(&client, &state.gate, &context, &name, &policy, false)
        .await
        .map_err(|e| e.user_message())
}

/// Whether a ServiceAccount may do something (a SubjectAccessReview). A
/// question to the API server: nothing is changed, so it isn't gated.
#[tauri::command(rename_all = "snake_case")]
pub async fn check_access(
    state: State<'_, AppState>,
    account_namespace: String,
    account: String,
    verb: String,
    resource: String,
    namespace: String,
    name: String,
) -> CmdResult<subduer_core::types::AccessCheck> {
    let client = connected_client(&state).await?;
    subduer_core::rbac::check_access(&client, &account_namespace, &account, &verb, &resource, &namespace, &name)
        .await
        .map_err(|e| e.user_message())
}

/// Rolls a Helm release back with the helm CLI, checked like
/// `delete_resource`. Returns Helm's output.
#[tauri::command]
pub async fn helm_rollback(
    state: State<'_, AppState>,
    context: String,
    namespace: String,
    name: String,
    revision: i64,
    wait: bool,
) -> CmdResult<String> {
    client_for_change(&state, &context, "rolled back").await?;
    subduer_core::helm::rollback(&state.gate, &context, &namespace, &name, revision, wait, false)
        .await
        .map_err(|e| e.user_message())
}

/// Deletes a Helm release with `helm uninstall`, checked like
/// `delete_resource`. Returns Helm's output.
#[tauri::command(rename_all = "snake_case")]
pub async fn helm_uninstall(
    state: State<'_, AppState>,
    context: String,
    namespace: String,
    name: String,
    keep_history: bool,
    wait: bool,
) -> CmdResult<String> {
    client_for_change(&state, &context, "deleted").await?;
    subduer_core::helm::uninstall(&state.gate, &context, &namespace, &name, keep_history, wait, false)
        .await
        .map_err(|e| e.user_message())
}

/// Creates a namespace, checked like `delete_resource`.
#[tauri::command]
pub async fn create_namespace(state: State<'_, AppState>, context: String, name: String) -> CmdResult<()> {
    let client = client_for_change(&state, &context, "created").await?;
    subduer_core::namespace::create_namespace(&client, &state.gate, &context, &name, false)
        .await
        .map_err(|e| e.user_message())
}

/// Opens an Ingress URL in the browser. Only plain http(s) addresses are
/// accepted.
#[tauri::command]
pub fn open_url(url: String) -> CmdResult<()> {
    let ok = (url.starts_with("http://") || url.starts_with("https://"))
        && url.len() < 2048
        && !url.chars().any(|c| c.is_whitespace() || c.is_control());
    if !ok {
        return Err(format!("not an http(s) address: {url:?}"));
    }
    crate::forward::open_in_browser(&url)
}

/// Sets every window's native appearance (title bar, scroll bars, form
/// controls): `light`, `dark`, or `auto` to follow the system.
#[tauri::command]
pub fn set_app_theme<R: Runtime>(app: AppHandle<R>, theme: String) {
    let theme = match theme.as_str() {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    };
    app.set_theme(theme);
}

/// The UI reports whether it holds unsaved edits, so closing the window or
/// quitting can ask first.
#[tauri::command]
pub fn set_unsaved(state: State<'_, AppState>, unsaved: bool) {
    state.unsaved.store(unsaved, Ordering::SeqCst);
}

/// Closes the main window (or quits) after the user chose to discard edits
/// and stop any running trace. The trace's file is closed first.
#[tauri::command]
pub async fn close_discarding<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>, quit: bool) -> CmdResult<()> {
    if let Some(Err(e)) = state.traces.stop(subduer_types::TraceEnd::Quit).await {
        tracing::warn!(error = %e, "closing the trace on quit failed");
    }
    state.unsaved.store(false, Ordering::SeqCst);
    if quit {
        app.exit(0);
    } else if let Some(main) = tauri::Manager::get_webview_window(&app, "main") {
        let _ = main.close();
    }
    Ok(())
}

/// Reveals one Secret value on explicit user request. The value is returned
/// to the UI only; it is never logged.
#[tauri::command]
pub async fn secret_value(state: State<'_, AppState>, namespace: String, secret: String, key: String) -> CmdResult<String> {
    let client = connected_client(&state).await?;
    tracing::info!(%namespace, %secret, %key, "secret value revealed");
    pod_details::secret_value(&client, &namespace, &secret, &key).await.map_err(err)
}

pub(crate) async fn connected_client(state: &AppState) -> CmdResult<subduer_core::kube::Client> {
    state
        .session
        .read()
        .await
        .as_ref()
        .map(|s| s.client.clone())
        .ok_or_else(|| "not connected to a cluster".to_owned())
}
