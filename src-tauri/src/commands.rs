//! IPC commands invoked from the UI. Errors are returned as display strings.

use std::sync::atomic::Ordering;

use subduer_core::types::{ClusterInfo, ContextList, KindRef, events};
use subduer_core::{cluster, contexts, watch};
use tauri::{AppHandle, Emitter, State};

use crate::{AppState, Session};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
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
    });
    Ok(ClusterInfo {
        context,
        kinds,
        namespaces,
    })
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
pub async fn watch_resources(
    app: AppHandle,
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

async fn connected_client(state: &AppState) -> CmdResult<subduer_core::kube::Client> {
    state
        .session
        .read()
        .await
        .as_ref()
        .map(|s| s.client.clone())
        .ok_or_else(|| "not connected to a cluster".to_owned())
}
