//! File upload into and download out of a container. The file arrives as the raw IPC body (no
//! JSON/base64 blow-up); the target rides in an `x-upload` header as
//! URL-encoded JSON. Gated: it changes the container's filesystem.

use subduer_core::types::{DirListing, DownloadResult, UploadOutcome, UploadTarget};
use tauri::ipc::{InvokeBody, Request};

use tauri::Runtime;
use crate::Win;

/// Lists a directory in a container for the upload dialog's browser. Read
/// only, so not gated; `suggest` adds writable places (/tmp, $HOME, mounts).
#[tauri::command]
pub async fn browse_dir(
    state: Win,
    namespace: String,
    pod: String,
    container: String,
    path: String,
    suggest: bool,
) -> Result<DirListing, String> {
    let client = crate::commands::connected_client(&state).await?;
    subduer_core::upload::browse(client, &namespace, &pod, &container, &path, suggest)
        .await
        .map_err(crate::commands::user)
}

#[tauri::command]
pub async fn upload_file(request: Request<'_>, state: Win) -> Result<UploadOutcome, String> {
    let header = request
        .headers()
        .get("x-upload")
        .and_then(|v| v.to_str().ok())
        .ok_or("missing upload target")?;
    let json = percent_encoding::percent_decode_str(header).decode_utf8().map_err(crate::commands::err)?;
    let target: UploadTarget = serde_json::from_str(&json).map_err(crate::commands::err)?;
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
    state.gate.ensure_writable(&target.context).map_err(crate::commands::err)?;
    let (t, bytes) = (&target, data.len());
    tracing::info!(context = %t.context, namespace = %t.namespace, pod = %t.pod, container = %t.container, path = %t.path, bytes, "upload");
    subduer_core::upload::upload(client, &target, &data).await.map_err(crate::commands::user)
}

/// Copies a file out of a container into the Downloads folder, numbering the
/// name if it's taken. Read-only for the cluster, so not gated.
#[tauri::command]
pub async fn download_file<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: Win,
    namespace: String,
    pod: String,
    container: String,
    path: String,
) -> Result<DownloadResult, String> {
    use tauri::Manager;
    download_into(&state, || app.path().download_dir(), &namespace, &pod, &container, &path).await
}

/// [`download_file`], into the folder `downloads` gives.
pub(crate) async fn download_into(
    state: &Win,
    downloads: impl FnOnce() -> tauri::Result<std::path::PathBuf>,
    namespace: &str,
    pod: &str,
    container: &str,
    path: &str,
) -> Result<DownloadResult, String> {
    let client = crate::commands::connected_client(state).await?;
    let dir = downloads().map_err(|e| format!("no Downloads folder: {e}"))?;
    let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
    let dest = subduer_core::download::unique_path(&dir, name);
    tracing::info!(%namespace, %pod, %container, %path, dest = %dest.display(), "download");
    let bytes = subduer_core::download::download(client, namespace, pod, container, path, &dest)
        .await
        .map_err(crate::commands::user)?;
    Ok(DownloadResult { saved_to: dest.display().to_string(), bytes })
}

/// Shows a downloaded file in Finder (or its folder elsewhere). Only paths in
/// the Downloads folder are accepted.
#[tauri::command]
pub fn reveal_download<R: Runtime>(app: tauri::AppHandle<R>, path: String) -> Result<(), String> {
    use tauri::Manager;
    let dir = app.path().download_dir().map_err(crate::commands::err)?;
    let path = std::path::PathBuf::from(path);
    if !path.starts_with(&dir) {
        return Err("only files in the Downloads folder can be revealed".into());
    }
    app.state::<crate::AppState>().launcher.reveal(&path)
}
