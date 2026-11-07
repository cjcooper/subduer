//! Copies a file into a container, like `kubectl cp` but without needing
//! `tar`: a small `sh` script resolves the destination and `head -c` reads
//! the bytes from stdin. Also lists directories so the dialog can browse to a
//! writable place. Callers must check the safety gate before uploading.
//! Images without `/bin/sh` (distroless) can't do either, as with `kubectl cp`.

use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;
use kube::Client;
use kube::api::{Api, AttachParams};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::types::{DirEntry, DirListing, UploadOutcome, UploadTarget};
use crate::{Error, Result};

/// Largest upload accepted; the whole file is held in memory.
pub const MAX_BYTES: usize = 512 * 1024 * 1024;
/// Entries listed per directory.
const MAX_ENTRIES: usize = 1000;

/// $1 = path, $2 = original file name, $3 = 1 to overwrite, $4 = byte count.
/// Prints the final path, then `head -c` takes exactly $4 bytes from stdin and
/// the size written is printed. Exit 17: exists; 18: directory missing.
/// Arguments are never spliced into the script.
///
/// Reading a known length matters: when the client ends stdin, the runtime
/// discards whatever the process hasn't read yet, so `cat` until end-of-file
/// kept only the first few KiB of anything sent while the script started up.
const UPLOAD_SCRIPT: &str = r#"
p="$1"
case "$p" in
  */) p="$p$2" ;;
  *) if [ -d "$p" ]; then p="$p/$2"; fi ;;
esac
dir="${p%/*}"
[ -n "$dir" ] || dir=/
if [ ! -d "$dir" ]; then echo "$dir"; exit 18; fi
if [ "$3" != 1 ] && [ -e "$p" ]; then echo "$p"; exit 17; fi
echo "$p"
head -c "$4" > "$p" || exit
wc -c < "$p"
"#;

/// How long after the last byte to wait for the container to finish reading
/// before ending stdin anyway (then the size check reports what's missing).
const STDIN_GRACE: Duration = Duration::from_secs(60);

/// $1 = directory, $2 = 1 to include suggestions. Output lines: resolved
/// path; `w` or `-`; `S <path>` suggestions; `E<d|f><w|-><r|-> <name>`
/// entries; `T` if truncated.
const BROWSE_SCRIPT: &str = r#"
cd -- "$1" 2>/dev/null || { echo "Can't open $1" >&2; exit 3; }
pwd
if [ -w . ]; then echo w; else echo -; fi
if [ "$2" = 1 ]; then
  {
    echo /tmp
    [ -n "$HOME" ] && echo "$HOME"
    while read -r _ m _ o _; do
      case "$o" in
        rw|rw,*) case "$m" in /proc*|/sys*|/dev*|/etc/hosts|/etc/hostname|/etc/resolv.conf) ;; *) echo "$m" ;; esac ;;
      esac
    done < /proc/mounts 2>/dev/null
  } | while read -r s; do
    if [ -d "$s" ] && [ -w "$s" ]; then echo "S $s"; fi
  done
fi
n=0
for f in * .[!.]* ..?*; do
  [ -e "$f" ] || [ -L "$f" ] || continue
  n=$((n + 1))
  if [ "$n" -gt MAX ]; then echo T; break; fi
  if [ -d "$f" ]; then t=d; else t=f; fi
  if [ -w "$f" ]; then w=w; else w=-; fi
  if [ -r "$f" ]; then r=r; else r=-; fi
  echo "E$t$w$r $f"
done
"#;

pub async fn upload(client: Client, target: &UploadTarget, data: &[u8]) -> Result<UploadOutcome> {
    upload_within(client, target, data, STDIN_GRACE).await
}

/// [`upload`], ending stdin `grace` after the last byte.
async fn upload_within(client: Client, target: &UploadTarget, data: &[u8], grace: Duration) -> Result<UploadOutcome> {
    if !target.path.starts_with('/') {
        return Err(Error::Message(format!("The destination must be an absolute path, not {:?}.", target.path)));
    }
    if target.file_name.is_empty() || target.file_name.contains('/') {
        return Err(Error::Message("The file needs a name without slashes.".into()));
    }
    if data.len() > MAX_BYTES {
        return Err(Error::Message(format!("Files over {} MiB aren't supported.", MAX_BYTES / 1024 / 1024)));
    }
    let overwrite = if target.overwrite { "1" } else { "0" };
    let size = data.len().to_string();
    let pods: Api<Pod> = Api::namespaced(client, &target.namespace);
    let params = AttachParams::default().container(target.container.clone()).stdin(true).stdout(true).stderr(true);
    let args = ["sh", "-c", UPLOAD_SCRIPT, "sh", target.path.as_str(), target.file_name.as_str(), overwrite, size.as_str()];
    let mut process = pods.exec(&target.pod, args, &params).await?;
    let (((mut stdin, mut stdout), mut stderr), status) =
        opened(process.stdin().zip(process.stdout()).zip(process.stderr()).zip(process.take_status()), EXEC_STREAMS)?;
    // Stdin stays open until the container exits by itself, having read every
    // byte; ending it sooner would drop data still in flight.
    let write = async move {
        for chunk in data.chunks(64 * 1024) {
            stdin.write_all(chunk).await?;
        }
        tokio::time::sleep(grace).await;
        stdin.shutdown().await
    };
    let read = async { tokio::join!(read_all(&mut stdout), read_all(&mut stderr)) };
    tokio::pin!(read);
    // A broken pipe is expected when the script exits early (exists, no dir).
    let (out, err) = tokio::select! {
        r = &mut read => r,
        _ = write => read.await,
    };
    let mut lines = out.lines();
    let path = lines.next().unwrap_or_default().to_owned();
    let written = lines.next().and_then(|n| n.trim().parse::<u64>().ok());
    match status.await {
        Some(s) if s.status.as_deref() == Some("Success") => match written {
            Some(n) if n == data.len() as u64 => Ok(UploadOutcome::Uploaded { bytes: n, path }),
            n => Err(Error::Message(format!(
                "Only {} of {} bytes reached {path}; the file in the container is incomplete.",
                n.map_or_else(|| "some".to_owned(), |n| n.to_string()),
                data.len()
            ))),
        },
        Some(s) => match exit_code(&s).as_deref() {
            Some("17") => Ok(UploadOutcome::Exists { path }),
            Some("18") => Err(Error::Message(format!("The directory {path} doesn't exist in the container."))),
            _ => Err(Error::Message(friendly(&err, &s))),
        },
        None => Err(Error::Message("The connection closed before the container reported a result.".into())),
    }
}

/// Lists `path` in the container. `suggest` also returns writable places.
pub async fn browse(client: Client, namespace: &str, pod: &str, container: &str, path: &str, suggest: bool) -> Result<DirListing> {
    let pods: Api<Pod> = Api::namespaced(client, namespace);
    let params = AttachParams::default().container(container.to_owned()).stdout(true).stderr(true);
    let script = BROWSE_SCRIPT.replace("MAX", &MAX_ENTRIES.to_string());
    let suggest = if suggest { "1" } else { "0" };
    let mut process = pods.exec(pod, ["sh", "-c", script.as_str(), "sh", path, suggest], &params).await?;
    let ((mut stdout, mut stderr), status) = opened(process.stdout().zip(process.stderr()).zip(process.take_status()), EXEC_STREAMS)?;
    let (out, err) = tokio::join!(read_all(&mut stdout), read_all(&mut stderr));
    match status.await {
        Some(s) if s.status.as_deref() == Some("Success") => Ok(parse_listing(&out)),
        Some(s) => Err(Error::Message(friendly(&err, &s))),
        None => Err(Error::Message("The connection closed before the container reported a result.".into())),
    }
}

fn parse_listing(out: &str) -> DirListing {
    let mut lines = out.lines();
    let path = lines.next().unwrap_or("/").to_owned();
    let writable = lines.next() == Some("w");
    let (mut entries, mut suggestions, mut truncated) = (vec![], vec![], false);
    for line in lines {
        if let Some(s) = line.strip_prefix("S ") {
            if !suggestions.iter().any(|x| x == s) {
                suggestions.push(s.to_owned());
            }
        } else if line == "T" {
            truncated = true;
        } else if let Some(rest) = line.strip_prefix('E')
            && let (Some(kind), Some(w), Some(r), Some(name)) =
                (rest.get(0..1), rest.get(1..2), rest.get(2..3), rest.get(4..))
        {
            entries.push(DirEntry { name: name.to_owned(), dir: kind == "d", writable: w == "w", readable: r == "r" });
        }
    }
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.cmp(&b.name)));
    DirListing { path, writable, entries, truncated, suggestions }
}

/// What's said when an exec call didn't open the streams it asked for.
pub(crate) const EXEC_STREAMS: &str = "The API server didn't open the exec streams.";

/// The streams an exec call opened, or `missing` as the error. kube opens
/// every stream asked for, so this is a safeguard.
pub(crate) fn opened<T>(streams: Option<T>, missing: &str) -> Result<T> {
    streams.ok_or_else(|| Error::Message(missing.into()))
}

pub(crate) async fn read_all(r: &mut (impl AsyncReadExt + Unpin)) -> String {
    let mut text = String::new();
    let _ = r.read_to_string(&mut text).await;
    text
}

pub(crate) fn exit_code(s: &Status) -> Option<String> {
    s.details
        .as_ref()?
        .causes
        .as_ref()?
        .iter()
        .find(|c| c.reason.as_deref() == Some("ExitCode"))
        .and_then(|c| c.message.clone())
}

/// Reads better in the dialog than the runtime's raw message.
pub(crate) fn friendly(stderr: &str, s: &Status) -> String {
    let raw = match stderr.trim() {
        "" => s.message.clone().unwrap_or_default(),
        e => e.to_owned(),
    };
    if raw.contains("/bin/sh") || raw.contains("executable file not found") {
        "This image has no shell, so files can't be browsed or uploaded (e.g. a distroless image).".into()
    } else if raw.contains("Read-only file system") {
        "That location is on a read-only filesystem. Pick a writable folder, such as /tmp.".into()
    } else if raw.contains("Permission denied") {
        "The container user can't write there. Pick a writable folder.".into()
    } else {
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{StatusCause, StatusDetails};

    #[test]
    fn reads_exit_code_from_status() {
        let s = Status {
            status: Some("Failure".into()),
            reason: Some("NonZeroExitCode".into()),
            details: Some(StatusDetails {
                causes: Some(vec![StatusCause { reason: Some("ExitCode".into()), message: Some("17".into()), field: None }]),
                ..StatusDetails::default()
            }),
            ..Status::default()
        };
        assert_eq!(exit_code(&s).as_deref(), Some("17"));
        assert!(friendly("exec: \"/bin/sh\": stat /bin/sh: no such file", &s).contains("no shell"));
        assert!(friendly("sh: can't create /etc/x: Read-only file system", &s).contains("read-only"));
    }

    #[test]
    fn parses_listings() {
        let out = "/data\nw\nS /tmp\nS /data\nS /tmp\nEdwr cache\nEf-r config.yaml\nEd-- .hidden dir\nEfwr a b.txt\n";
        let l = parse_listing(out);
        assert_eq!(l.path, "/data");
        assert!(l.writable && !l.truncated);
        assert_eq!(l.suggestions, ["/tmp", "/data"]);
        let names: Vec<&str> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, [".hidden dir", "cache", "a b.txt", "config.yaml"]);
        assert!(l.entries[1].writable && l.entries[1].dir && !l.entries[0].writable);
        assert!(!l.entries[0].readable && l.entries[3].readable);
    }
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod coverage_tests;
