//! Copies a file out of a container onto this machine: `cat` in the container,
//! streamed straight to disk (never held in memory). Read-only for the
//! cluster, so not gated. Needs `/bin/sh` in the image, like uploads.

use std::path::{Path, PathBuf};

use k8s_openapi::api::core::v1::Pod;
use kube::Client;
use kube::api::{Api, AttachParams};

use crate::upload::{exit_code, friendly, read_all};
use crate::{Error, Result};

/// $1 = file. Exit 21: missing; 19: not a regular file; 20: not readable.
const SCRIPT: &str = r#"
[ -e "$1" ] || exit 21
[ -f "$1" ] || exit 19
[ -r "$1" ] || exit 20
exec cat "$1"
"#;

/// Streams `path` from the container into `dest` (via a `.part` file that is
/// renamed on success and removed on failure). Returns the byte count.
pub async fn download(
    client: Client,
    namespace: &str,
    pod: &str,
    container: &str,
    path: &str,
    dest: &Path,
) -> Result<u64> {
    if !path.starts_with('/') {
        return Err(Error::Message(format!("The file must be an absolute path, not {path:?}.")));
    }
    let pods: Api<Pod> = Api::namespaced(client, namespace);
    let params = AttachParams::default().container(container.to_owned()).stdout(true).stderr(true);
    let mut process = pods.exec(pod, ["sh", "-c", SCRIPT, "sh", path], &params).await?;
    let (Some(mut stdout), Some(mut stderr), Some(status)) = (process.stdout(), process.stderr(), process.take_status())
    else {
        return Err(Error::Message("The API server didn't open the exec streams.".into()));
    };
    let part = dest.with_extension(match dest.extension() {
        Some(ext) => format!("{}.part", ext.to_string_lossy()),
        None => "part".into(),
    });
    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(|e| Error::Message(format!("Can't create {}: {e}", part.display())))?;
    let (copied, err) = tokio::join!(tokio::io::copy(&mut stdout, &mut file), read_all(&mut stderr));
    drop(file);
    let outcome = match status.await {
        Some(s) if s.status.as_deref() == Some("Success") => match copied {
            Ok(bytes) => Ok(bytes),
            Err(e) => Err(Error::Message(format!("Writing {} failed: {e}", part.display()))),
        },
        Some(s) => Err(Error::Message(match exit_code(&s).as_deref() {
            Some("21") => format!("{path} doesn't exist in the container."),
            Some("19") => format!("{path} isn't a regular file (a folder or device?)."),
            Some("20") => format!("The container user can't read {path}."),
            _ => friendly(&err, &s),
        })),
        None => Err(Error::Message("The connection closed before the container reported a result.".into())),
    };
    match outcome {
        Ok(bytes) => {
            tokio::fs::rename(&part, dest)
                .await
                .map_err(|e| Error::Message(format!("Can't save {}: {e}", dest.display())))?;
            Ok(bytes)
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(e)
        }
    }
}

/// `dir/name`, or `dir/name (1).ext`, `(2)`, … if that's taken, like browsers do.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let name = sanitize(name);
    let candidate = dir.join(&name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_owned(), format!(".{ext}")),
        _ => (name.clone(), String::new()),
    };
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("some free name")
}

/// A safe local file name from the container file's name.
fn sanitize(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    match cleaned.trim() {
        "" | "." | ".." => "download".into(),
        s => s.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_free_names() {
        let dir = std::env::temp_dir().join(format!("subduer-dl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "app.log"), dir.join("app.log"));
        std::fs::write(dir.join("app.log"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "app.log"), dir.join("app (1).log"));
        std::fs::write(dir.join("app (1).log"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "app.log"), dir.join("app (2).log"));
        std::fs::write(dir.join(".env"), b"x").unwrap();
        assert_eq!(unique_path(&dir, ".env"), dir.join(".env (1)"));
        assert_eq!(unique_path(&dir, ".."), dir.join("download"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod coverage_tests;
