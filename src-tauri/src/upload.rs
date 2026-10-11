//! File upload into and download out of a container. The file arrives as the raw IPC body (no
//! JSON/base64 blow-up); the target rides in an `x-upload` header as
//! URL-encoded JSON. Gated: it changes the container's filesystem.

use subduer_core::types::{DirListing, DownloadResult, UploadOutcome, UploadTarget};
use tauri::State;
use tauri::ipc::{InvokeBody, Request};

use tauri::Runtime;
use crate::AppState;

/// Lists a directory in a container for the upload dialog's browser. Read
/// only, so not gated; `suggest` adds writable places (/tmp, $HOME, mounts).
#[tauri::command]
pub async fn browse_dir(
    state: State<'_, AppState>,
    namespace: String,
    pod: String,
    container: String,
    path: String,
    suggest: bool,
) -> Result<DirListing, String> {
    let client = crate::commands::connected_client(&state).await?;
    subduer_core::upload::browse(client, &namespace, &pod, &container, &path, suggest)
        .await
        .map_err(|e| e.user_message())
}

#[tauri::command]
pub async fn upload_file(state: State<'_, AppState>, request: Request<'_>) -> Result<UploadOutcome, String> {
    let header = request
        .headers()
        .get("x-upload")
        .and_then(|v| v.to_str().ok())
        .ok_or("missing upload target")?;
    let json = percent_encoding::percent_decode_str(header).decode_utf8().map_err(|e| e.to_string())?;
    let target: UploadTarget = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let InvokeBody::Raw(data) = request.body() else {
        return Err("expected the file as a binary body".into());
    };
    let data = data.clone();
    let client = {
        let session = state.session.read().await;
        let session = session.as_ref().ok_or("not connected to a cluster")?;
        if session.context != target.context {
            return Err(format!("connected to {} now, not {}; nothing was uploaded", session.context, target.context));
        }
        session.client.clone()
    };
    state.gate.ensure_writable(&target.context).map_err(|e| e.to_string())?;
    tracing::info!(
        context = %target.context, namespace = %target.namespace, pod = %target.pod,
        container = %target.container, path = %target.path, bytes = data.len(), "upload"
    );
    subduer_core::upload::upload(client, &target, &data).await.map_err(|e| e.user_message())
}

/// Copies a file out of a container into the Downloads folder, numbering the
/// name if it's taken. Read-only for the cluster, so not gated.
#[tauri::command]
pub async fn download_file<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    namespace: String,
    pod: String,
    container: String,
    path: String,
) -> Result<DownloadResult, String> {
    use tauri::Manager;
    let client = crate::commands::connected_client(&state).await?;
    let dir = app.path().download_dir().map_err(|e| format!("no Downloads folder: {e}"))?;
    let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
    let dest = subduer_core::download::unique_path(&dir, name);
    tracing::info!(%namespace, %pod, %container, %path, dest = %dest.display(), "download");
    let bytes = subduer_core::download::download(client, &namespace, &pod, &container, &path, &dest)
        .await
        .map_err(|e| e.user_message())?;
    Ok(DownloadResult { saved_to: dest.display().to_string(), bytes })
}

/// Shows a downloaded file in Finder (or its folder elsewhere). Only paths in
/// the Downloads folder are accepted.
#[tauri::command]
pub fn reveal_download<R: Runtime>(app: tauri::AppHandle<R>, path: String) -> Result<(), String> {
    use tauri::Manager;
    let dir = app.path().download_dir().map_err(|e| e.to_string())?;
    let path = std::path::PathBuf::from(path);
    if !path.starts_with(&dir) {
        return Err("only files in the Downloads folder can be revealed".into());
    }
    reveal(&path)
}

/// Shows a file in Finder or Explorer (selected), or opens its folder on Linux.
pub(crate) fn reveal(path: &std::path::Path) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg("-R").arg(path);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("explorer");
        c.arg(format!("/select,{}", path.display()));
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path.parent().unwrap_or(path));
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}
