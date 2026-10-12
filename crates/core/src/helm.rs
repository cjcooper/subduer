//! Helm 3 releases, read from the Secrets Helm stores them in
//! (`type: helm.sh/release.v1`, label `owner=helm`, one per revision).
//!
//! Releases appear as a virtual kind. The list comes from a metadata-only
//! watch, so it doesn't download release payloads. Chart and app version are
//! decoded from each release's latest revision; only `chart.metadata` and
//! `info` are kept. Values and manifests (which can hold secrets) are dropped
//! straight after decoding and never shown; the Details tab takes only each
//! manifest object's kind and name first.
//!
//! Rollback and uninstall run the `helm` CLI (`helm rollback`, `helm
//! uninstall`), so hooks, history and resource policies behave exactly as
//! they do from a terminal.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::time::Duration;

use base64::Engine;
use futures::{StreamExt, stream};
use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, ListParams};
use kube::core::PartialObjectMeta;
use kube::runtime::{WatchStreamExt, watcher};
use kube::{Client, ResourceExt};
use serde_json::{Value, json};

use crate::safety::SafetyGate;
use crate::watch::{Backoff, Failures};
use crate::types::{
    Category, Column, ColumnKind, InfoRow, InfoSection, InfoTable, InfoTableRow, KindInfo, KindRef, ObjectDetails,
    ObjectLink, ResourceRow, ResourceTable,
};
use crate::{Error, Result};

const SELECTOR: &str = "owner=helm";
const THROTTLE: Duration = Duration::from_millis(250);

/// The virtual kind the sidebar lists under Helm.
pub fn release_kind() -> KindRef {
    KindRef {
        group: "helm.sh".into(),
        version: "v3".into(),
        kind: "Release".into(),
        plural: "releases".into(),
        namespaced: true,
    }
}

pub fn is_release(kind: &KindRef) -> bool {
    kind.group == "helm.sh" && kind.kind == "Release"
}

/// The Release kind if the cluster has any Helm releases we can see. Read-only:
/// deleting a release's Secrets is not `helm uninstall`.
pub async fn discover(client: &Client) -> Option<KindInfo> {
    let api: Api<Secret> = Api::all(client.clone());
    let lp = ListParams::default().labels(SELECTOR).limit(1);
    let found = !api.list_metadata(&lp).await.ok()?.items.is_empty();
    found.then(|| KindInfo {
        kind: release_kind(),
        verbs: vec!["list".into(), "watch".into(), "get".into()],
        category: Category::Helm,
    })
}

/// One revision, from a release Secret's labels.
#[derive(Debug, Clone, PartialEq)]
struct Revision {
    namespace: String,
    secret: String,
    release: String,
    revision: i64,
    status: String,
    /// When the revision was created, RFC 3339.
    updated: String,
}

impl Revision {
    fn from_meta(meta: &PartialObjectMeta<Secret>) -> Option<Self> {
        let labels = meta.labels();
        Some(Self {
            namespace: meta.namespace()?,
            secret: meta.name_any(),
            release: labels.get("name")?.clone(),
            revision: labels.get("version")?.parse().ok()?,
            status: labels.get("status").cloned().unwrap_or_default(),
            // Each revision's Secret is created at deploy time. The `modifiedAt`
            // label isn't reliable: some setups leave it unchanged across revisions.
            updated: meta.creation_timestamp().map(|t| t.0.to_string()).unwrap_or_default(),
        })
    }
}

/// What the table shows from a decoded release.
#[derive(Debug, Clone, Default, PartialEq)]
struct ChartInfo {
    chart: String,
    app_version: String,
}

fn columns() -> Vec<Column> {
    use ColumnKind::*;
    [
        ("Name", Name),
        ("Namespace", Namespace),
        ("Chart", Text),
        ("App Version", Text),
        ("Revision", Number),
        ("Status", Badge),
        ("Updated", Age),
    ]
    .into_iter()
    .map(|(header, kind)| Column { header: header.into(), kind })
    .collect()
}

/// Latest revision of each release, keyed `namespace/release`.
fn latest(revisions: &BTreeMap<String, Revision>) -> BTreeMap<String, &Revision> {
    let mut out: BTreeMap<String, &Revision> = BTreeMap::new();
    for r in revisions.values() {
        let key = format!("{}/{}", r.namespace, r.release);
        if out.get(&key).is_none_or(|cur| r.revision > cur.revision) {
            out.insert(key, r);
        }
    }
    out
}

fn rows(revisions: &BTreeMap<String, Revision>, charts: &HashMap<String, ChartInfo>) -> Vec<ResourceRow> {
    latest(revisions)
        .into_iter()
        .map(|(id, r)| {
            let info = charts.get(&secret_key(&r.namespace, &r.secret)).cloned().unwrap_or_default();
            ResourceRow {
                id,
                name: r.release.clone(),
                namespace: Some(r.namespace.clone()),
                cells: vec![
                    r.release.clone(),
                    r.namespace.clone(),
                    info.chart,
                    info.app_version,
                    r.revision.to_string(),
                    r.status.clone(),
                    r.updated.clone(),
                ],
            }
        })
        .collect()
}

fn secret_key(namespace: &str, name: &str) -> String {
    format!("{namespace}/{name}")
}

/// Like [`crate::watch::watch_table`], for Helm releases.
pub async fn watch_releases<F>(client: Client, namespace: Option<String>, watch_id: u64, emit: F)
where
    F: Fn(ResourceTable),
{
    // A `PartialObjectMeta` API makes metadata-only requests: no payloads.
    let api: Api<PartialObjectMeta<Secret>> = match &namespace {
        Some(ns) => Api::namespaced(client.clone(), ns),
        None => Api::all(client.clone()),
    };
    let config = watcher::Config::default().labels(SELECTOR);
    let mut stream = watcher(api, config).backoff(Backoff::new()).boxed();
    let mut revisions: BTreeMap<String, Revision> = BTreeMap::new();
    let mut pending: Option<BTreeMap<String, Revision>> = None;
    // Decoded chart info per Secret; a revision's chart never changes.
    let mut charts: HashMap<String, ChartInfo> = HashMap::new();
    let mut dirty = false;
    let mut failures = Failures::default();
    let mut ticker = tokio::time::interval(THROTTLE);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let table = |rows: Vec<ResourceRow>, error: Option<String>| ResourceTable {
        watch_id,
        columns: columns(),
        rows,
        error,
        stopped: false,
    };

    loop {
        tokio::select! {
            event = stream.next() => {
                let Some(event) = event else { break };
                failures.seen(&event);
                let insert = |map: &mut BTreeMap<String, Revision>, meta: &PartialObjectMeta<Secret>| {
                    if let Some(r) = Revision::from_meta(meta) {
                        map.insert(secret_key(&r.namespace, &r.secret), r);
                    }
                };
                match event {
                    Ok(watcher::Event::Init) => pending = Some(BTreeMap::new()),
                    Ok(watcher::Event::InitApply(meta)) => insert(pending.get_or_insert_with(BTreeMap::new), &meta),
                    Ok(watcher::Event::InitDone) => {
                        revisions = pending.take().unwrap_or_default();
                        dirty = true;
                    }
                    Ok(watcher::Event::Apply(meta)) => {
                        insert(&mut revisions, &meta);
                        dirty = true;
                    }
                    Ok(watcher::Event::Delete(meta)) => {
                        revisions.remove(&secret_key(&meta.namespace().unwrap_or_default(), &meta.name_any()));
                        dirty = true;
                    }
                    Err(err) => {
                        tracing::warn!(%err, "helm release watch error");
                        let stopped = failures.failed();
                        emit(ResourceTable { stopped, ..table(rows(&revisions, &charts), Some(err.to_string())) });
                        if stopped {
                            tracing::warn!("helm release watch gave up");
                            return;
                        }
                    }
                }
            }
            _ = ticker.tick(), if dirty && pending.is_none() => {
                dirty = false;
                let missing: Vec<(String, String)> = latest(&revisions)
                    .values()
                    .filter(|r| !charts.contains_key(&secret_key(&r.namespace, &r.secret)))
                    .map(|r| (r.namespace.clone(), r.secret.clone()))
                    .collect();
                if !missing.is_empty() {
                    // Show the list straight away; chart columns fill in after.
                    emit(table(rows(&revisions, &charts), None));
                    let fetched: Vec<(String, ChartInfo)> = stream::iter(missing)
                        .map(|(ns, name)| {
                            let client = client.clone();
                            async move {
                                let info = read_release(&client, &ns, &name).await.map(|r| chart_info(&r));
                                (secret_key(&ns, &name), info.unwrap_or_default())
                            }
                        })
                        .buffer_unordered(8)
                        .collect()
                        .await;
                    charts.extend(fetched);
                }
                emit(table(rows(&revisions, &charts), None));
            }
        }
    }
}

/// Decodes one revision: Secret data is base64 (handled by k8s-openapi), then
/// Helm's own base64, usually gzipped JSON. Values and manifest are removed
/// before returning.
async fn read_release(client: &Client, namespace: &str, secret: &str) -> Option<Value> {
    read_release_with_objects(client, namespace, secret).await.map(|(r, _)| r)
}

/// One object a release created, from its manifest.
#[derive(Debug, Clone, PartialEq)]
struct ManifestObject {
    api_version: String,
    kind: String,
    name: String,
    namespace: Option<String>,
}

/// Like [`read_release`], also listing the manifest's objects (kind and name
/// only) before the manifest is dropped.
async fn read_release_with_objects(client: &Client, namespace: &str, secret: &str) -> Option<(Value, Vec<ManifestObject>)> {
    let api: Api<Secret> = Api::namespaced(client.clone(), namespace);
    let s = api.get(secret).await.ok()?;
    let raw = s.data.as_ref()?.get("release")?;
    let mut release = decode(&raw.0)?;
    let objects = manifest_objects(release["manifest"].as_str().unwrap_or_default());
    if let Some(obj) = release.as_object_mut() {
        obj.remove("config");
        obj.remove("manifest");
        obj.remove("hooks");
    }
    if let Some(chart) = release["chart"].as_object_mut() {
        chart.retain(|k, _| k == "metadata");
    }
    Some((release, objects))
}

/// Kind, name and namespace of each document in a rendered manifest. Nothing
/// else is kept, so Secret data in the manifest goes no further.
fn manifest_objects(manifest: &str) -> Vec<ManifestObject> {
    let mut docs = vec![String::new()];
    for line in manifest.lines() {
        if line.starts_with("---") {
            docs.push(String::new());
        } else if let Some(doc) = docs.last_mut() {
            doc.push_str(line);
            doc.push('\n');
        }
    }
    docs.iter()
        .filter_map(|doc| {
            let v: Value = serde_norway::from_str(doc).ok()?;
            let text = |x: &Value| x.as_str().map(str::to_owned);
            Some(ManifestObject {
                api_version: text(&v["apiVersion"])?,
                kind: text(&v["kind"])?,
                name: text(&v["metadata"]["name"])?,
                namespace: text(&v["metadata"]["namespace"]),
            })
        })
        .collect()
}

fn decode(helm_b64: &[u8]) -> Option<Value> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(helm_b64).ok()?;
    let json = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&bytes[..]).read_to_end(&mut out).ok()?;
        out
    } else {
        bytes
    };
    serde_json::from_slice(&json).ok()
}

fn chart_info(release: &Value) -> ChartInfo {
    let meta = &release["chart"]["metadata"];
    let (name, version) = (meta["name"].as_str().unwrap_or_default(), meta["version"].as_str().unwrap_or_default());
    ChartInfo {
        chart: if version.is_empty() { name.to_owned() } else { format!("{name}-{version}") },
        app_version: meta["appVersion"].as_str().unwrap_or_default().to_owned(),
    }
}

/// The detail panel's text for a release: chart, status, notes and revision
/// history. Values and manifests are deliberately left out.
pub async fn release_yaml(client: &Client, namespace: &str, release: &str) -> Result<String> {
    let api: Api<Secret> = Api::namespaced(client.clone(), namespace);
    let lp = ListParams::default().labels(&format!("{SELECTOR},name={release}"));
    let mut history: Vec<Revision> = api
        .list_metadata(&lp)
        .await?
        .items
        .iter()
        .filter_map(Revision::from_meta)
        .collect();
    history.sort_by_key(|r| std::cmp::Reverse(r.revision));
    let latest = history
        .first()
        .ok_or_else(|| Error::NotFound(format!("Helm release {namespace}/{release}")))?;
    let decoded = read_release(client, namespace, &latest.secret).await.unwrap_or(Value::Null);
    let meta = &decoded["chart"]["metadata"];
    let info = &decoded["info"];
    let notes = info["notes"].as_str().map(|n| {
        // Trailing spaces force quoted YAML; trimmed, notes print as a block.
        let lines: Vec<&str> = n.lines().map(str::trim_end).collect();
        Value::from(lines.join("\n").trim_matches('\n'))
    });
    let history: Vec<Value> = history
        .iter()
        .map(|r| json!({"revision": r.revision, "status": r.status, "updated": r.updated}))
        .collect();
    let fields = [
        ("release", json!(release)),
        ("namespace", json!(namespace)),
        ("revision", json!(latest.revision)),
        ("status", json!(latest.status)),
        ("chart", meta["name"].clone()),
        ("chartVersion", meta["version"].clone()),
        ("appVersion", meta["appVersion"].clone()),
        ("description", info["description"].clone()),
        ("firstDeployed", info["first_deployed"].clone()),
        ("lastDeployed", info["last_deployed"].clone()),
        ("history", Value::from(history)),
        ("notes", notes.unwrap_or(Value::Null)),
    ];
    // A YAML mapping keeps this order; a JSON object would sort the keys.
    let mut summary = serde_norway::Mapping::new();
    for (key, value) in fields {
        if !value.is_null() && value.as_str() != Some("") {
            summary.insert(key.into(), serde_norway::to_value(value)?);
        }
    }
    let mut text = String::from("# Helm release summary. Values and manifest are not shown.\n");
    text.push_str(&serde_norway::to_string(&summary)?);
    Ok(text)
}

/// The Details tab of a release: chart, status, history with each
/// revision's chart and description, notes and the objects it created.
/// Values are never read out.
pub async fn release_details(client: &Client, namespace: &str, release: &str) -> Result<ObjectDetails> {
    let api: Api<Secret> = Api::namespaced(client.clone(), namespace);
    let lp = ListParams::default().labels(&format!("{SELECTOR},name={release}"));
    let mut history: Vec<Revision> = api.list_metadata(&lp).await?.items.iter().filter_map(Revision::from_meta).collect();
    history.sort_by_key(|r| std::cmp::Reverse(r.revision));
    let latest = history.first().cloned().ok_or_else(|| Error::NotFound(format!("Helm release {namespace}/{release}")))?;
    let (decoded, objects) = read_release_with_objects(client, namespace, &latest.secret).await.unwrap_or((Value::Null, Vec::new()));
    // Older revisions: chart, app version and description, newest 20.
    let older: Vec<(i64, Value)> = stream::iter(history.iter().skip(1).take(19).cloned())
        .map(|r| async move { (r.revision, read_release(client, namespace, &r.secret).await.unwrap_or(Value::Null)) })
        .buffer_unordered(8)
        .collect()
        .await;
    let by_revision: HashMap<i64, Value> = older.into_iter().chain([(latest.revision, decoded.clone())]).collect();
    Ok(describe_release(namespace, release, &history, &by_revision, &objects))
}

fn describe_release(
    namespace: &str,
    release: &str,
    history: &[Revision],
    decoded: &HashMap<i64, Value>,
    objects: &[ManifestObject],
) -> ObjectDetails {
    let latest = &history[0];
    let current = decoded.get(&latest.revision).cloned().unwrap_or(Value::Null);
    let meta = &current["chart"]["metadata"];
    let info = &current["info"];
    let text = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    let mut rows = vec![];
    let chart = chart_info(&current);
    if !chart.chart.is_empty() {
        rows.push(InfoRow::new("Chart", chart.chart.clone()).mono());
    }
    if !chart.app_version.is_empty() {
        rows.push(InfoRow::new("App version", chart.app_version.clone()));
    }
    rows.push(InfoRow::new("Revision", latest.revision.to_string()));
    if let Some(d) = text(&info["description"]) {
        rows.push(InfoRow::new("Last change", d));
    }
    if let Some(t) = text(&info["first_deployed"]) {
        rows.push(InfoRow::new("First deployed", t).time());
    }
    if let Some(t) = text(&info["last_deployed"]) {
        rows.push(InfoRow::new("Last deployed", t).time());
    }
    if let Some(d) = text(&meta["description"]) {
        rows.push(InfoRow::new("About the chart", d));
    }
    if let Some(h) = text(&meta["home"]) {
        rows.push(InfoRow::new("Chart home", h.clone()).url(h));
    }
    rows.push(InfoRow::new("Values", "not shown: they can hold secrets"));

    let history_rows: Vec<InfoTableRow> = history
        .iter()
        .map(|r| {
            let d = decoded.get(&r.revision).cloned().unwrap_or(Value::Null);
            let c = chart_info(&d);
            let mut row = InfoTableRow::new(vec![
                r.revision.to_string(),
                r.status.clone(),
                c.chart,
                c.app_version,
                r.updated.clone(),
                text(&d["info"]["description"]).unwrap_or_default(),
            ]);
            row.warn = r.status == "failed";
            row
        })
        .collect();
    let mut sections = vec![InfoSection {
        id: "helm-history".into(),
        title: "History".into(),
        summary: crate::config::plural(history.len(), "revision"),
        rows: Vec::new(),
        table: Some(InfoTable {
            columns: ["Revision", "Status", "Chart", "App version", "Updated", "Description"].map(String::from).to_vec(),
            numeric: vec![0],
            times: vec![4],
            rows: history_rows,
            ..Default::default()
        }),
        empty: None,
    }];
    let object_rows: Vec<InfoTableRow> = objects
        .iter()
        .map(|o| {
            let group = o.api_version.split_once('/').map_or("core", |(g, _)| g);
            let ns = o.namespace.clone().unwrap_or_else(|| namespace.to_owned());
            let mut row = InfoTableRow::new(vec![o.kind.clone(), o.name.clone(), o.namespace.clone().unwrap_or_default()]);
            row.link = Some(ObjectLink { kind: format!("{group}/{}", o.kind), namespace: Some(ns), name: o.name.clone() });
            row
        })
        .collect();
    let n = object_rows.len();
    sections.push(InfoSection {
        id: "helm-resources".into(),
        title: "Resources".into(),
        summary: if n == 0 { "none".into() } else { crate::config::plural(n, "object") },
        rows: Vec::new(),
        table: Some(InfoTable { columns: vec!["Kind".into(), "Name".into(), "Namespace".into()], rows: object_rows, ..Default::default() }),
        empty: Some("Its manifest lists no objects.".into()),
    });
    if let Some(notes) = text(&info["notes"]) {
        let lines: Vec<&str> = notes.lines().map(str::trim_end).collect();
        sections.push(InfoSection {
            id: "helm-notes".into(),
            title: "Notes".into(),
            summary: String::new(),
            rows: vec![InfoRow::new("", lines.join("\n").trim_matches('\n').to_owned()).mono()],
            table: None,
            empty: None,
        });
    }
    ObjectDetails {
        kind: "Release".into(),
        name: release.into(),
        namespace: Some(namespace.into()),
        status: Some(latest.status.clone()),
        info: rows,
        created: history.last().map(|r| r.updated.clone()),
        labels: Vec::new(),
        annotations: Vec::new(),
        conditions: Vec::new(),
        data: None,
        sections,
        events: Vec::new(),
        node: None,
    }
}

/// Helm names: lowercase letters, digits, `-` and `.`, starting with a letter
/// or digit (so one can't pass for a flag).
fn valid_name(s: &str) -> bool {
    s.len() <= 253
        && s.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
}

/// `helm rollback` arguments. Dry runs are server-side: `--dry-run=server`
/// on Helm 4, `--dry-run` on Helm 3.
fn rollback_args(context: &str, namespace: &str, release: &str, revision: i64, wait: bool, dry_run: bool, helm4: bool) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "rollback".into(),
        release.into(),
        revision.to_string(),
        // `--flag=value`, so a value can never be read as another flag.
        format!("--namespace={namespace}"),
        format!("--kube-context={context}"),
    ];
    if wait {
        args.extend(["--wait".into(), "--timeout".into(), "5m0s".into()]);
    }
    if dry_run {
        args.push(if helm4 { "--dry-run=server".into() } else { "--dry-run".into() });
    }
    args
}

/// Rolls a release back to `revision` with the `helm` CLI, which creates a
/// new revision from the old one, as `helm rollback` does. Returns Helm's
/// output. A `dry_run` asks the server to validate it and changes nothing.
#[allow(clippy::too_many_arguments)]
pub async fn rollback(
    gate: &SafetyGate,
    context: &str,
    namespace: &str,
    release: &str,
    revision: i64,
    wait: bool,
    dry_run: bool,
) -> Result<String> {
    if !valid_name(release) || !valid_name(namespace) {
        return Err(Error::Message("not a valid release or namespace name".into()));
    }
    if revision < 1 {
        return Err(Error::Message("pick a revision to roll back to".into()));
    }
    if !dry_run {
        gate.ensure_writable(context)?;
    }
    let helm4 = helm_major().await? >= 4;
    let args = rollback_args(context, namespace, release, revision, wait, dry_run, helm4);
    let result = run_helm(&args).await;
    tracing::info!(%context, %namespace, %release, revision, wait, dry_run, ok = result.is_ok(), "helm rollback");
    result
}

/// `helm uninstall` arguments.
fn uninstall_args(context: &str, namespace: &str, release: &str, keep_history: bool, wait: bool, dry_run: bool) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "uninstall".into(),
        release.into(),
        format!("--namespace={namespace}"),
        format!("--kube-context={context}"),
    ];
    if keep_history {
        args.push("--keep-history".into());
    }
    if wait {
        args.extend(["--wait".into(), "--timeout".into(), "5m0s".into()]);
    }
    if dry_run {
        args.push("--dry-run".into());
    }
    args
}

/// Deletes a release with `helm uninstall`: its resources (except those
/// marked `helm.sh/resource-policy: keep`, and PVCs made by StatefulSets)
/// and, unless `keep_history`, its revision records. A `dry_run` changes
/// nothing.
#[allow(clippy::too_many_arguments)]
pub async fn uninstall(
    gate: &SafetyGate,
    context: &str,
    namespace: &str,
    release: &str,
    keep_history: bool,
    wait: bool,
    dry_run: bool,
) -> Result<String> {
    if !valid_name(release) || !valid_name(namespace) {
        return Err(Error::Message("not a valid release or namespace name".into()));
    }
    if !dry_run {
        gate.ensure_writable(context)?;
    }
    helm_major().await?;
    let result = run_helm(&uninstall_args(context, namespace, release, keep_history, wait, dry_run)).await;
    tracing::info!(%context, %namespace, %release, keep_history, wait, dry_run, ok = result.is_ok(), "helm uninstall");
    result
}

/// `helm` (`helm.exe` on Windows, found on `PATH`), without a console window.
fn helm_command() -> tokio::process::Command {
    #[allow(unused_mut)]
    let mut cmd = tokio::process::Command::new("helm");
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

/// The installed Helm's major version; an error if `helm` can't be run.
async fn helm_major() -> Result<u32> {
    let version = helm_command()
        .args(["version", "--short"])
        .output()
        .await
        .map_err(|_| Error::Message("The helm command isn't installed or isn't on PATH (brew install helm).".into()))?;
    let text = String::from_utf8_lossy(&version.stdout);
    Ok(text.trim_start().trim_start_matches('v').split('.').next().and_then(|m| m.parse().ok()).unwrap_or(3))
}

/// Runs `helm` with `args` (no shell) and returns its output, or its error
/// output as the error. Gives up after 6 minutes.
async fn run_helm(args: &[String]) -> Result<String> {
    let what = format!("helm {}", args.first().map_or("", String::as_str));
    let run = helm_command().args(args).kill_on_drop(true).output();
    let out = tokio::time::timeout(Duration::from_secs(6 * 60), run)
        .await
        .map_err(|_| Error::Message(format!("{what} took more than 6 minutes; check the release's history")))?
        .map_err(|e| Error::Message(format!("couldn't run helm: {e}")))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)).trim().to_owned();
    if out.status.success() {
        Ok(text)
    } else {
        Err(Error::Message(if text.is_empty() { format!("{what} failed ({})", out.status) } else { text }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn rev(ns: &str, release: &str, revision: i64, status: &str) -> (String, Revision) {
        let secret = format!("sh.helm.release.v1.{release}.v{revision}");
        (
            secret_key(ns, &secret),
            Revision {
                namespace: ns.into(),
                secret,
                release: release.into(),
                revision,
                status: status.into(),
                updated: "2026-10-01T00:00:00Z".into(),
            },
        )
    }

    #[test]
    fn one_row_per_release_at_its_latest_revision() {
        let revisions = BTreeMap::from([
            rev("apps", "web", 1, "superseded"),
            rev("apps", "web", 10, "deployed"),
            rev("apps", "web", 9, "superseded"),
            rev("ops", "web", 2, "failed"),
        ]);
        let charts = HashMap::from([(
            secret_key("apps", "sh.helm.release.v1.web.v10"),
            ChartInfo { chart: "web-1.2.0".into(), app_version: "3.4".into() },
        )]);
        let rows = rows(&revisions, &charts);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "apps/web");
        assert_eq!(rows[0].cells[2..6], ["web-1.2.0", "3.4", "10", "deployed"]);
        assert_eq!(rows[1].cells[4..6], ["2", "failed"]);
    }

    #[test]
    fn manifest_objects_keep_only_kind_and_name() {
        let manifest = "---\n# Source: web/templates/secret.yaml\napiVersion: v1\nkind: Secret\nmetadata:\n  name: web-creds\ndata:\n  password: aHVudGVyMg==\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: shop\nspec:\n  replicas: 2\n---\n# empty\n";
        let objects = manifest_objects(manifest);
        assert_eq!(objects.len(), 2);
        assert_eq!((objects[0].kind.as_str(), objects[0].name.as_str(), objects[0].namespace.clone()), ("Secret", "web-creds", None));
        assert_eq!((objects[1].api_version.as_str(), objects[1].namespace.as_deref()), ("apps/v1", Some("shop")));
        assert!(!format!("{objects:?}").contains("aHVudGVy"));
    }

    #[test]
    fn release_details_history_and_links() {
        let history: Vec<Revision> = [rev("shop", "web", 3, "deployed"), rev("shop", "web", 2, "superseded"), rev("shop", "web", 1, "failed")]
            .into_iter()
            .map(|(_, r)| r)
            .collect();
        let decoded = HashMap::from([
            (3, json!({"chart": {"metadata": {"name": "web", "version": "1.3.0", "appVersion": "2.0"}},
                       "info": {"description": "Upgrade complete", "notes": "Visit https://shop.acme.io  \n\n"}})),
            (2, json!({"chart": {"metadata": {"name": "web", "version": "1.2.0", "appVersion": "1.9"}}, "info": {"description": "Install complete"}})),
        ]);
        let objects = vec![ManifestObject { api_version: "apps/v1".into(), kind: "Deployment".into(), name: "web".into(), namespace: None }];
        let d = describe_release("shop", "web", &history, &decoded, &objects);
        assert_eq!(d.status.as_deref(), Some("deployed"));
        let h = d.sections[0].table.as_ref().unwrap();
        assert_eq!(h.rows[0].cells[..4], ["3", "deployed", "web-1.3.0", "2.0"]);
        assert_eq!(h.rows[1].cells[5], "Install complete");
        assert!(h.rows[2].warn);
        let link = d.sections[1].table.as_ref().unwrap().rows[0].link.clone().unwrap();
        assert_eq!((link.kind.as_str(), link.namespace.as_deref()), ("apps/Deployment", Some("shop")));
        assert_eq!(d.sections[2].rows[0].value, "Visit https://shop.acme.io");
    }

    #[test]
    fn rollback_arguments() {
        let args = rollback_args("acme-uat", "shop", "web", 2, true, true, true);
        assert_eq!(
            args,
            ["rollback", "web", "2", "--namespace=shop", "--kube-context=acme-uat", "--wait", "--timeout", "5m0s", "--dry-run=server"]
        );
        assert_eq!(rollback_args("c", "n", "r", 1, false, true, false).last().unwrap(), "--dry-run");
        assert!(!rollback_args("c", "n", "r", 1, false, false, true).iter().any(|a| a.starts_with("--dry-run")));
        assert!(valid_name("corporate-wallet") && !valid_name("web; rm") && !valid_name("--all"));
    }

    #[test]
    fn uninstall_arguments() {
        assert_eq!(
            uninstall_args("acme-uat", "shop", "web", true, true, true),
            ["uninstall", "web", "--namespace=shop", "--kube-context=acme-uat", "--keep-history", "--wait", "--timeout", "5m0s", "--dry-run"]
        );
        assert_eq!(uninstall_args("c", "n", "r", false, false, false), ["uninstall", "r", "--namespace=n", "--kube-context=c"]);
    }

    #[tokio::test]
    async fn uninstall_is_gated_and_checked() {
        let gate = SafetyGate::default();
        let err = uninstall(&gate, "acme-prd", "shop", "web", false, false, false).await.unwrap_err();
        assert!(matches!(err, Error::Locked(_)));
        let err = uninstall(&gate, "acme-prd", "shop", "-a", false, false, true).await.unwrap_err();
        assert!(matches!(err, Error::Message(_)));
    }

    #[tokio::test]
    async fn rollback_is_gated_and_checked() {
        let gate = SafetyGate::default();
        let err = rollback(&gate, "acme-prd", "shop", "web", 2, false, false).await.unwrap_err();
        assert!(matches!(err, Error::Locked(_)));
        let err = rollback(&gate, "acme-prd", "shop", "--all", 2, false, true).await.unwrap_err();
        assert!(matches!(err, Error::Message(_)));
        let err = rollback(&gate, "acme-prd", "shop", "web", 0, false, true).await.unwrap_err();
        assert!(matches!(err, Error::Message(_)));
    }

    #[test]
    fn decodes_gzipped_release() {
        let json = br#"{"name":"web","chart":{"metadata":{"name":"web","version":"1.2.0","appVersion":"3.4"}}}"#;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(json).unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(gz.finish().unwrap());
        let release = decode(encoded.as_bytes()).unwrap();
        assert_eq!(chart_info(&release), ChartInfo { chart: "web-1.2.0".into(), app_version: "3.4".into() });
    }
}

#[cfg(test)]
#[path = "helm_tests.rs"]
mod coverage_tests;
