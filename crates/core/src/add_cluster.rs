//! File › Add Cluster…: writes clusters, users and contexts to the kubeconfig
//! kubectl uses, the way `kubectl config set-cluster`, `set-credentials` and
//! `set-context` would. Nothing here is specific to this app: other tools
//! see the new context too.
//!
//! The file is edited as plain YAML, so fields this app doesn't know about
//! are kept. Comments are lost, as they are when kubectl writes the file.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use kube::config::Kubeconfig;
use serde_norway::{Mapping, Value};

use crate::types::{ClusterAuth, NewCluster};
use crate::{Error, Result};

/// What decides where kubeconfigs are: `$KUBECONFIG` and the home folder.
/// [`KubeconfigEnv::current`] in the app; tests point it at a scratch folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KubeconfigEnv {
    /// `$KUBECONFIG`, if set.
    pub kubeconfig: Option<OsString>,
    /// The user's home folder.
    pub home: Option<PathBuf>,
}

impl KubeconfigEnv {
    /// This process's environment.
    pub fn current() -> Self {
        Self { kubeconfig: std::env::var_os("KUBECONFIG"), home: std::env::home_dir() }
    }

    /// The file kubectl writes new entries to (client-go's default filename):
    /// with one file in `$KUBECONFIG`, that file; with several, the first that
    /// exists, else the last; without `$KUBECONFIG`, `~/.kube/config`.
    pub fn path(&self) -> Result<PathBuf> {
        destination(self.kubeconfig.as_deref(), self.home.as_deref(), |p| p.exists())
    }

    /// Every kubeconfig file kubectl reads, in order: those in `$KUBECONFIG`,
    /// else `~/.kube/config`. Some may not exist.
    pub fn files(&self) -> Result<Vec<PathBuf>> {
        let files = split(self.kubeconfig.as_deref());
        if !files.is_empty() {
            return Ok(files);
        }
        Ok(vec![self.path()?])
    }
}

/// [`KubeconfigEnv::path`] for this process.
pub fn kubeconfig_path() -> Result<PathBuf> {
    KubeconfigEnv::current().path()
}

/// [`KubeconfigEnv::files`] for this process.
pub fn kubeconfig_files() -> Result<Vec<PathBuf>> {
    KubeconfigEnv::current().files()
}

/// The paths in a `$KUBECONFIG` value, skipping empty ones.
fn split(env: Option<&OsStr>) -> Vec<PathBuf> {
    env.map(|v| std::env::split_paths(v).filter(|p| !p.as_os_str().is_empty()).collect()).unwrap_or_default()
}

fn destination(env: Option<&OsStr>, home: Option<&Path>, exists: impl Fn(&Path) -> bool) -> Result<PathBuf> {
    let files = split(env);
    if let Some(last) = files.last() {
        return Ok(files.iter().find(|p| exists(p)).unwrap_or(last).clone());
    }
    home.map(|h| h.join(".kube").join("config"))
        .ok_or_else(|| Error::Message("can't find your home folder to put the kubeconfig in".into()))
}

/// Names already used by the merged kubeconfig (every `$KUBECONFIG` file).
#[derive(Debug, Default)]
pub struct TakenNames {
    pub contexts: Vec<String>,
    pub clusters: Vec<String>,
    pub users: Vec<String>,
}

impl TakenNames {
    /// [`TakenNames::read_in`] this process's environment.
    pub fn read() -> Result<Self> {
        Self::read_in(&KubeconfigEnv::current())
    }

    /// The names in the kubeconfig files `env` points to, merged the way
    /// kubectl (and `Kubeconfig::read`) merges them.
    pub fn read_in(env: &KubeconfigEnv) -> Result<Self> {
        // No kubeconfig at all yet is fine: this makes the first one.
        if env.kubeconfig.is_none() && !env.path()?.exists() {
            return Ok(Self::default());
        }
        let config = env.files()?.iter().try_fold(Kubeconfig::default(), |merged, path| merged.merge(Kubeconfig::read_from(path)?))?;
        Ok(Self {
            contexts: config.contexts.into_iter().map(|c| c.name).collect(),
            clusters: config.clusters.into_iter().map(|c| c.name).collect(),
            users: config.auth_infos.into_iter().map(|u| u.name).collect(),
        })
    }

    fn clash(&self, name: &str, with_user: bool) -> Option<String> {
        let used = |list: &[String]| list.iter().any(|n| n == name);
        if used(&self.contexts) {
            Some(format!("A context named {name:?} already exists."))
        } else if used(&self.clusters) {
            Some(format!("A cluster named {name:?} already exists in the kubeconfig."))
        } else if with_user && used(&self.users) {
            Some(format!("A user named {name:?} already exists in the kubeconfig."))
        } else {
            None
        }
    }
}

/// Adds `new` to the kubeconfig at `path`, creating the file if needed.
/// Refuses names already in use (`taken`), so nothing is overwritten.
/// Returns the context's name.
pub fn add_cluster(path: &Path, new: &NewCluster, taken: &TakenNames) -> Result<String> {
    if let Some(problem) = new.problem() {
        return Err(Error::Message(problem.into()));
    }
    let name = new.name.trim().to_owned();
    let has_user = new.auth != ClusterAuth::None;
    if let Some(clash) = taken.clash(&name, has_user) {
        return Err(Error::Message(clash));
    }
    let cluster = cluster_entry(new)?;
    let user = has_user.then(|| user_entry(&new.auth)).transpose()?;
    edit(path, |doc| {
        push_named(doc, "clusters", "cluster", &name, cluster)?;
        let mut context = Mapping::new();
        context.insert("cluster".into(), name.clone().into());
        if let Some(user) = user {
            push_named(doc, "users", "user", &name, user)?;
            context.insert("user".into(), name.clone().into());
        }
        let namespace = new.namespace.trim();
        if !namespace.is_empty() {
            context.insert("namespace".into(), namespace.into());
        }
        push_named(doc, "contexts", "context", &name, context)?;
        if new.make_current {
            doc.insert("current-context".into(), name.clone().into());
        }
        Ok(())
    })?;
    tracing::info!(context = %name, path = %path.display(), "added cluster to kubeconfig");
    Ok(name)
}

/// The kubeconfig's `current-context` at `path`, if it has one.
pub fn current_context(path: &Path) -> Result<Option<String>> {
    let Some(doc) = read(path)? else { return Ok(None) };
    Ok(doc.get("current-context").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned))
}

/// Sets (or, with `None`, clears) `current-context` at `path`.
pub fn set_current_context(path: &Path, context: Option<&str>) -> Result<()> {
    edit(path, |doc| {
        match context {
            Some(name) => doc.insert("current-context".into(), name.into()),
            None => doc.remove("current-context"),
        };
        Ok(())
    })
}

/// Renames context `from` to `to` at `path`, like `kubectl config
/// rename-context`, including `current-context`.
pub fn rename_context(path: &Path, from: &str, to: &str) -> Result<()> {
    edit(path, |doc| {
        let contexts = doc.get_mut("contexts").and_then(Value::as_sequence_mut);
        let entry = contexts
            .into_iter()
            .flatten()
            .filter_map(Value::as_mapping_mut)
            .find(|m| m.get("name").and_then(Value::as_str) == Some(from))
            .ok_or_else(|| Error::NotFound(format!("context {from:?} in {}", path.display())))?;
        entry.insert("name".into(), to.into());
        if doc.get("current-context").and_then(Value::as_str) == Some(from) {
            doc.insert("current-context".into(), to.into());
        }
        Ok(())
    })
}

fn cluster_entry(new: &NewCluster) -> Result<Mapping> {
    let mut cluster = Mapping::new();
    cluster.insert("server".into(), new.server.trim().trim_end_matches('/').into());
    pem_or_file(&mut cluster, "certificate-authority", &new.certificate_authority)?;
    if new.insecure_skip_tls_verify {
        cluster.insert("insecure-skip-tls-verify".into(), true.into());
    }
    let proxy = new.proxy_url.trim();
    if !proxy.is_empty() {
        cluster.insert("proxy-url".into(), proxy.into());
    }
    Ok(cluster)
}

fn user_entry(auth: &ClusterAuth) -> Result<Mapping> {
    let mut user = Mapping::new();
    match auth {
        ClusterAuth::None => {}
        ClusterAuth::Token { token } => {
            user.insert("token".into(), token.trim().into());
        }
        ClusterAuth::ClientCertificate { certificate, key } => {
            pem_or_file(&mut user, "client-certificate", certificate)?;
            pem_or_file(&mut user, "client-key", key)?;
        }
        ClusterAuth::Exec { command, args } => {
            let mut exec = Mapping::new();
            exec.insert("apiVersion".into(), "client.authentication.k8s.io/v1beta1".into());
            exec.insert("command".into(), command.trim().into());
            let args: Vec<Value> = args.iter().map(|a| a.trim()).filter(|a| !a.is_empty()).map(Value::from).collect();
            if !args.is_empty() {
                exec.insert("args".into(), Value::Sequence(args));
            }
            exec.insert("interactiveMode".into(), "IfAvailable".into());
            exec.insert("provideClusterInfo".into(), false.into());
            user.insert("exec".into(), exec.into());
        }
    }
    Ok(user)
}

/// PEM text goes in as `<field>-data` (base64, as kubectl's `--embed-certs`
/// does); anything else is a file path the kubeconfig points to, as kubectl
/// does without it. Empty adds nothing.
fn pem_or_file(map: &mut Mapping, field: &str, value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(());
    }
    if value.starts_with("-----BEGIN ") {
        map.insert(format!("{field}-data").into(), STANDARD.encode(format!("{value}\n")).into());
        return Ok(());
    }
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err(Error::Message(format!("{value}: paste the PEM text, or choose the file (a full path)")));
    }
    if !path.is_file() {
        return Err(Error::Message(format!("{value}: no such file")));
    }
    map.insert(field.into(), value.into());
    Ok(())
}

/// Appends `{name, <key>: entry}` to the list `list`.
fn push_named(doc: &mut Mapping, list: &str, key: &str, name: &str, entry: Mapping) -> Result<()> {
    let seq = doc.entry(list.into()).or_insert_with(|| Value::Sequence(vec![]));
    if seq.is_null() {
        *seq = Value::Sequence(vec![]);
    }
    let seq = seq
        .as_sequence_mut()
        .ok_or_else(|| Error::Message(format!("the kubeconfig's {list} isn't a list")))?;
    let mut named = Mapping::new();
    named.insert("name".into(), name.into());
    named.insert(key.into(), entry.into());
    seq.push(named.into());
    Ok(())
}

/// The kubeconfig at `path` as YAML; `None` if there's no file (or it's empty).
pub(crate) fn read(path: &Path) -> Result<Option<Mapping>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(path, e)),
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    match serde_norway::from_str::<Value>(&text)? {
        Value::Mapping(doc) => Ok(Some(doc)),
        Value::Null => Ok(None),
        _ => Err(Error::Message(format!("{} isn't a kubeconfig", path.display()))),
    }
}

/// What `kubectl` writes into a new kubeconfig.
fn empty_config() -> Mapping {
    let mut doc = Mapping::new();
    doc.insert("apiVersion".into(), "v1".into());
    doc.insert("kind".into(), "Config".into());
    doc.insert("preferences".into(), Mapping::new().into());
    for list in ["clusters", "users", "contexts"] {
        doc.insert(list.into(), Value::Sequence(vec![]));
    }
    doc
}

/// Reads the kubeconfig at `path`, applies `change` and writes it back,
/// holding kubectl's lock file (`<path>.lock`) meanwhile. The file is
/// replaced in one step, so a reader never sees half of it; a symlinked
/// kubeconfig is written through to its target.
pub(crate) fn edit(path: &Path, change: impl FnOnce(&mut Mapping) -> Result<()>) -> Result<()> {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
    }
    let _lock = Lock::take(&path)?;
    let mut doc = read(&path)?.unwrap_or_else(empty_config);
    change(&mut doc)?;
    let text = serde_norway::to_string(&Value::Mapping(doc))?;
    write_replacing(&path, text.as_bytes())
}

/// Writes `bytes` to a temporary file next to `path`, then renames it over
/// `path`, keeping the old file's permissions (owner-only for a new one:
/// kubeconfigs hold credentials).
fn write_replacing(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_file_name(format!(
        ".{}.subduer-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("config"),
        std::process::id()
    ));
    let write = || -> std::io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        match fs::metadata(path) {
            Ok(meta) => file.set_permissions(meta.permissions())?,
            #[cfg(unix)]
            Err(_) => {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(0o600))?;
            }
            #[cfg(not(unix))]
            Err(_) => {}
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    };
    write().map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io_error(path, e)
    })
}

/// client-go's kubeconfig lock: `<path>.lock`, created exclusively and
/// removed when done. kubectl and the cloud CLIs take it too.
struct Lock(PathBuf);

impl Lock {
    fn take(path: &Path) -> Result<Self> {
        let lock = PathBuf::from(format!("{}.lock", path.display()));
        match fs::OpenOptions::new().write(true).create_new(true).open(&lock) {
            Ok(_) => Ok(Self(lock)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(Error::Message(format!(
                "the kubeconfig is being changed by another program ({} exists); try again, or delete that file if nothing is running",
                lock.display()
            ))),
            Err(e) => Err(io_error(&lock, e)),
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn io_error(path: &Path, e: std::io::Error) -> Error {
    Error::Message(format!("{}: {e}", path.display()))
}

#[cfg(test)]
#[path = "add_cluster_tests.rs"]
mod tests;
