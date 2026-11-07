//! File › Manage Clusters…: lists, removes and reorders the kubeconfig's
//! contexts, the way `kubectl config delete-context` (and `delete-cluster`,
//! `delete-user`) would. The order is the order of each file's `contexts`
//! list: kubectl ignores it, but this app's context menu follows it.
//!
//! Edits go through [`crate::add_cluster`]'s, so they hold kubectl's lock,
//! keep fields this app doesn't know about, and replace the file in one step.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_norway::{Mapping, Value};

use crate::add_cluster::{edit, read};
use crate::types::{CloudProvider, ManagedContext, ManagedContexts, RemovedContext};
use crate::{Error, Result};

/// Each file with its YAML (`None` if missing or empty).
fn read_all(files: &[PathBuf]) -> Result<Vec<(PathBuf, Option<Mapping>)>> {
    files.iter().map(|f| Ok((f.clone(), read(f)?))).collect()
}

/// The named entries of `list` (`contexts`, `clusters`, `users`).
fn entries<'a>(doc: &'a Mapping, list: &str) -> impl Iterator<Item = &'a Mapping> {
    doc.get(list).and_then(Value::as_sequence).into_iter().flatten().filter_map(Value::as_mapping)
}

fn name(entry: &Mapping) -> &str {
    entry.get("name").and_then(Value::as_str).unwrap_or("")
}

/// `field` of a named entry's body (`context.cluster`, `cluster.server`…).
fn inner<'a>(entry: &'a Mapping, body: &str, field: &str) -> &'a str {
    entry.get(body).and_then(|b| b.get(field)).and_then(Value::as_str).unwrap_or("")
}

/// The first entry named `wanted` in `list`, across files (the merged kubeconfig's).
fn first<'a>(docs: &'a [(PathBuf, Option<Mapping>)], list: &str, wanted: &str) -> Option<&'a Mapping> {
    docs.iter().filter_map(|(_, d)| d.as_ref()).flat_map(|d| entries(d, list)).find(|e| name(e) == wanted)
}

/// The kubeconfig's contexts, in the order kubectl merges them.
pub fn list(files: &[PathBuf]) -> Result<ManagedContexts> {
    let docs = read_all(files)?;
    let mut seen = HashSet::new();
    let mut contexts = vec![];
    for (path, doc) in &docs {
        let Some(doc) = doc else { continue };
        for entry in entries(doc, "contexts") {
            let context = name(entry);
            if context.is_empty() || !seen.insert(context.to_owned()) {
                continue;
            }
            let cluster = inner(entry, "context", "cluster");
            let user = inner(entry, "context", "user");
            let server = first(&docs, "clusters", cluster).map(|c| inner(c, "cluster", "server")).unwrap_or("");
            let exec = first(&docs, "users", user)
                .and_then(|u| u.get("user"))
                .and_then(|u| u.get("exec"))
                .and_then(|e| e.get("command"))
                .and_then(Value::as_str)
                .unwrap_or("");
            contexts.push(ManagedContext {
                name: context.to_owned(),
                cluster: cluster.to_owned(),
                user: user.to_owned(),
                server: server.to_owned(),
                provider: provider(context, server, exec),
                file: path.display().to_string(),
            });
        }
    }
    let current = docs
        .iter()
        .filter_map(|(_, d)| d.as_ref())
        .find_map(|d| d.get("current-context").and_then(Value::as_str).filter(|c| !c.is_empty()))
        .map(str::to_owned);
    Ok(ManagedContexts { contexts, files: files.iter().map(|f| f.display().to_string()).collect(), current })
}

/// The cloud a context looks to be in: its API server's host, else its
/// user's exec plugin, else (GKE's servers are bare addresses) the name
/// `gcloud` gives it.
pub(crate) fn provider(context: &str, server: &str, exec: &str) -> Option<CloudProvider> {
    let host = server.split("://").nth(1).unwrap_or(server).split(['/', ':']).next().unwrap_or("").to_ascii_lowercase();
    // A Windows path too, whatever system reads the file.
    let file = exec.rsplit(['/', '\\']).next().unwrap_or("");
    let command = file.strip_suffix(".exe").unwrap_or(file);
    if host.ends_with(".eks.amazonaws.com") || host.ends_with(".eks.amazonaws.com.cn") || matches!(command, "aws" | "aws-iam-authenticator") {
        Some(CloudProvider::Eks)
    } else if host.ends_with(".azmk8s.io") || command == "kubelogin" {
        Some(CloudProvider::Aks)
    } else if command == "gke-gcloud-auth-plugin" || context.starts_with("gke_") {
        Some(CloudProvider::Gke)
    } else {
        None
    }
}

/// Removes `context` from the first file that has it, with its cluster and
/// user entries in that file unless another context (in any file) still uses
/// them. Clears that file's `current-context` if it was this one.
pub fn remove(files: &[PathBuf], context: &str) -> Result<RemovedContext> {
    let docs = read_all(files)?;
    let file = docs
        .iter()
        .find(|(_, d)| d.as_ref().is_some_and(|d| entries(d, "contexts").any(|e| name(e) == context)))
        .map(|(f, _)| f.clone())
        .ok_or_else(|| Error::NotFound(format!("context {context:?} in the kubeconfig")))?;
    // What the other files' contexts use; this file's are checked after the removal.
    let used_elsewhere = |field: &str| -> HashSet<String> {
        docs.iter()
            .filter(|(f, _)| *f != file)
            .filter_map(|(_, d)| d.as_ref())
            .flat_map(|d| entries(d, "contexts"))
            .map(|e| inner(e, "context", field).to_owned())
            .collect()
    };
    let elsewhere = (used_elsewhere("cluster"), used_elsewhere("user"));
    let mut removed = RemovedContext { context: context.to_owned(), cluster: None, user: None };
    edit(&file, |doc| {
        removed = remove_from(doc, &file, context, &elsewhere)?;
        Ok(())
    })?;
    tracing::info!(context, path = %file.display(), cluster = ?removed.cluster, user = ?removed.user, "removed context from kubeconfig");
    Ok(removed)
}

/// [`remove`]'s change to `file` as it is once locked (`doc`), which may
/// have changed since it was read. `elsewhere`: the clusters and users the
/// other files' contexts use.
fn remove_from(doc: &mut Mapping, file: &Path, context: &str, elsewhere: &(HashSet<String>, HashSet<String>)) -> Result<RemovedContext> {
    let gone = || Error::NotFound(format!("context {context:?} in {}", file.display()));
    let contexts = doc.get_mut("contexts").and_then(Value::as_sequence_mut).ok_or_else(gone)?;
    let at = contexts.iter().position(|e| e.as_mapping().is_some_and(|m| name(m) == context)).ok_or_else(gone)?;
    let entry = contexts.remove(at);
    let entry = entry.as_mapping().cloned().unwrap_or_default();
    let mut removed = RemovedContext { context: context.to_owned(), cluster: None, user: None };
    for (field, list, elsewhere, out) in [
        ("cluster", "clusters", &elsewhere.0, &mut removed.cluster),
        ("user", "users", &elsewhere.1, &mut removed.user),
    ] {
        let target = inner(&entry, "context", field);
        let still_used = target.is_empty() || elsewhere.contains(target) || entries(doc, "contexts").any(|e| inner(e, "context", field) == target);
        if still_used {
            continue;
        }
        if let Some(seq) = doc.get_mut(list).and_then(Value::as_sequence_mut) {
            let before = seq.len();
            seq.retain(|e| !e.as_mapping().is_some_and(|m| name(m) == target));
            if seq.len() < before {
                *out = Some(target.to_owned());
            }
        }
    }
    if doc.get("current-context").and_then(Value::as_str) == Some(context) {
        doc.remove("current-context");
    }
    Ok(removed)
}

/// Puts each file's contexts in the order of `order` (every context's name,
/// as the merged list shows them). Names `order` leaves out keep their place
/// after the rest. Files whose order doesn't change aren't written.
pub fn reorder(files: &[PathBuf], order: &[String]) -> Result<()> {
    let position = |entry: &Value| -> usize {
        let n = entry.as_mapping().map(name).unwrap_or("");
        order.iter().position(|o| o == n).unwrap_or(usize::MAX)
    };
    let sorted = |doc: &Mapping| -> Option<Vec<Value>> {
        let contexts = doc.get("contexts")?.as_sequence()?;
        let mut new = contexts.clone();
        new.sort_by_key(position);
        (new != *contexts).then_some(new)
    };
    for (file, doc) in read_all(files)? {
        if doc.as_ref().and_then(sorted).is_none() {
            continue;
        }
        edit(&file, |doc| {
            if let Some(new) = sorted(doc) {
                doc.insert("contexts".into(), Value::Sequence(new));
            }
            Ok(())
        })?;
        tracing::info!(path = %file.display(), "reordered kubeconfig contexts");
    }
    Ok(())
}

#[cfg(test)]
#[path = "manage_clusters_tests.rs"]
mod tests;
