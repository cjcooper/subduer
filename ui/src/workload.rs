//! Changes confirmed in one dialog: Restart and Scale for Deployments and
//! StatefulSets, Run now for CronJobs, Suspend/Resume for Jobs and CronJobs,
//! Edit range for HPAs, saving a ConfigMap's or Secret's edited data, and
//! Force sync for ExternalSecrets and ClusterExternalSecrets.
//! Gated like Delete, and production-like contexts need the object's name
//! typed out.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{
    ContextInfo, DataChange, KindRef, Replicas, ResourceRow, can_force_sync, has_workload_details,
    is_scalable_workload,
};

use crate::modal::Modal;
use crate::app::Store;
use crate::tauri::invoke;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Restart,
    Scale,
    /// Start a Job from a CronJob now.
    Run,
    Suspend,
    Resume,
    /// An HPA's min and max replicas.
    Range,
    /// Edited ConfigMap or Secret data.
    Save,
    /// Grow a PersistentVolumeClaim.
    Expand,
    /// Switch a PersistentVolume between Retain and Delete.
    Reclaim,
    /// Roll a Helm release back to an earlier revision.
    Rollback,
    /// Delete a Helm release with `helm uninstall`.
    Uninstall,
    /// Annotate an ExternalSecret or ClusterExternalSecret with `force-sync`.
    ForceSync,
}

impl Op {
    fn label(self) -> &'static str {
        match self {
            Op::Restart => "Restart",
            Op::Scale => "Scale",
            Op::Run => "Run now",
            Op::Suspend => "Suspend",
            Op::Resume => "Resume",
            Op::Range => "Edit range",
            Op::Save => "Save",
            Op::Expand => "Expand",
            Op::Reclaim => "Reclaim policy",
            Op::Rollback => "Rollback",
            Op::Uninstall => "Delete",
            Op::ForceSync => "Force sync",
        }
    }

    fn hint(self, kind: &KindRef) -> &'static str {
        match (self, kind.kind.as_str()) {
            (Op::Restart, _) => "Rollout restart",
            (Op::Scale, _) => "Scale",
            (Op::Run, _) => "Start a Job from its template now",
            (Op::Suspend, "CronJob") => "Stop starting new Jobs",
            (Op::Suspend, _) => "Stop its running pods until resumed",
            (Op::Resume, "CronJob") => "Start Jobs on its schedule again",
            (Op::Resume, _) => "Start its pods again",
            (Op::Range, _) => "Change its min and max replicas",
            (Op::Save, _) => "Save the edited data",
            (Op::Expand, _) => "Make the claim bigger",
            (Op::Reclaim, _) => "Keep or delete the data when its claim is deleted",
            (Op::Rollback, _) => "Go back to an earlier revision with helm rollback",
            (Op::Uninstall, _) => "Delete the release and everything it created, with helm uninstall",
            (Op::ForceSync, "ClusterExternalSecret") => "Have External Secrets fetch from the store now, for every namespace",
            (Op::ForceSync, _) => "Have External Secrets fetch from the store now and update the Secret",
        }
    }
}

/// What the dialog acts on, captured when it opens.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkloadTarget {
    pub context: ContextInfo,
    pub kind: KindRef,
    pub namespace: String,
    pub name: String,
    pub op: Op,
    /// What a Save writes.
    pub save: Option<SaveData>,
}

/// A data edit ready to save, with its summary for the dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveData {
    pub resource_version: String,
    pub changes: Vec<DataChange>,
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Serialize)]
struct Target<'a> {
    kind: &'a KindRef,
    namespace: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
struct Change<'a> {
    context: &'a str,
    #[serde(flatten)]
    target: Target<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    replicas: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    suspend: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource_version: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    changes: Option<&'a [DataChange]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    policy: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keep_history: Option<bool>,
}

/// Opens the dialog, or the unlock prompt first if the context is read-only.
fn request(store: Store, kind: KindRef, row: &ResourceRow, op: Op) {
    // Cluster-scoped objects (PersistentVolumes) have no namespace.
    let namespace = row.namespace.clone().unwrap_or_default();
    open(store, kind, namespace, row.name.clone(), op, None);
}

/// Asks to save edited data of the selected ConfigMap or Secret.
pub(crate) fn request_save(store: Store, kind: KindRef, namespace: String, name: String, save: SaveData) {
    open(store, kind, namespace, name, Op::Save, Some(save));
}

fn open(store: Store, kind: KindRef, namespace: String, name: String, op: Op, save: Option<SaveData>) {
    let Some(context) = store.current_context_untracked() else { return };
    let target = WorkloadTarget { context: context.clone(), kind, namespace, name, op, save };
    if context.unlocked {
        store.workload_prompt.set(Some(target));
    } else {
        store.pending_workload.set(Some(target));
        store.unlock_prompt.set(Some(context));
    }
}

/// The header buttons for the selected workload. Each is hidden when the API
/// doesn't allow it; Suspend/Resume wait for the details to know which applies.
#[component]
pub fn WorkloadButtons(store: Store, row: ResourceRow) -> impl IntoView {
    let selected = row.clone();
    let ops = move || {
        let row = &selected;
        let kind = store
            .kind
            .get()
            .filter(|k| {
                has_workload_details(k)
                    || can_force_sync(k)
                    || matches!(
                        k.key().as_str(),
                        "autoscaling/HorizontalPodAutoscaler"
                            | "core/PersistentVolumeClaim"
                            | "core/PersistentVolume"
                            | "helm.sh/Release"
                    )
            })?;
        let allowed = |key: &str, verb: &str| {
            store.cluster.with(|c| {
                c.iter()
                    .flat_map(|c| &c.kinds)
                    .any(|k| k.kind.key() == key && k.verbs.iter().any(|v| v == verb))
            })
        };
        let patch = allowed(&kind.key(), "patch");
        let mut ops = Vec::new();
        // Rows of the loaded details, tracked so buttons appear once they load.
        let detail = |key: &str| {
            store.object.with(|o| match o {
                Some(Ok(d)) if d.name == row.name => d.info.iter().find(|r| r.key == key).map(|r| r.value.clone()),
                _ => None,
            })
        };
        if kind.key() == "helm.sh/Release" {
            // Helm runs it with your own credentials, so the listed verbs don't apply.
            if history(store, &row.name, true).len() > 1 {
                ops.push(Op::Rollback);
            }
            if !history(store, &row.name, true).is_empty() {
                ops.push(Op::Uninstall);
            }
        } else if kind.kind == "HorizontalPodAutoscaler" {
            if patch {
                ops.push(Op::Range);
            }
        } else if kind.kind == "PersistentVolumeClaim" {
            if patch && detail("Can expand").as_deref() == Some("yes") {
                ops.push(Op::Expand);
            }
        } else if kind.kind == "PersistentVolume" {
            let policy = detail("Reclaim policy");
            if patch && policy.is_some_and(|p| p.starts_with("Retain") || p.starts_with("Delete")) {
                ops.push(Op::Reclaim);
            }
        } else if can_force_sync(&kind) {
            if patch {
                ops.push(Op::ForceSync);
            }
        } else if is_scalable_workload(&kind) {
            if patch {
                ops.extend([Op::Restart, Op::Scale]);
            }
        } else {
            if kind.kind == "CronJob" && allowed("batch/Job", "create") {
                ops.push(Op::Run);
            }
            let state = store.workload.with(|w| match w {
                Some(Ok(d)) if d.name == row.name && row.namespace.as_ref() == Some(&d.namespace) && d.kind == kind.kind => {
                    Some((d.suspended, d.status.clone()))
                }
                _ => None,
            });
            // A finished Job has nothing left to suspend.
            if let Some((Some(suspended), status)) = state
                && patch
                && !(kind.kind == "Job" && matches!(status.as_str(), "Complete" | "Failed"))
            {
                ops.push(if suspended { Op::Resume } else { Op::Suspend });
            }
        }
        Some((kind, ops))
    };
    move || {
        ops().map(|(kind, ops)| {
            let locked = move || store.current_context().is_some_and(|c| !c.unlocked);
            ops.into_iter()
                .map(|op| {
                    let hint = op.hint(&kind);
                    let title = move || {
                        if locked() {
                            format!("{hint}… (asks to allow changes first: the context is read-only)")
                        } else {
                            format!("{hint}…")
                        }
                    };
                    let (kind, row) = (kind.clone(), row.clone());
                    view! {
                        <button
                            class="btn small"
                            class:danger-outline=op == Op::Uninstall
                            title=title
                            on:click=move |_| request(store, kind.clone(), &row, op)
                        >
                            {op.label()}
                        </button>
                    }
                })
                .collect_view()
        })
    }
}

/// The selected CronJob's concurrency policy and whether a Job is running.
fn cron_state(store: Store, name: &str) -> Option<(String, bool)> {
    store.workload.with_untracked(|w| match w {
        Some(Ok(d)) if d.name == name && d.kind == "CronJob" => {
            let policy = d.info.iter().find(|(k, _)| k == "Concurrency").map(|(_, v)| v.clone())?;
            Some((policy, d.status == "Running"))
        }
        _ => None,
    })
}

fn run_note(store: Store, name: &str) -> Option<&'static str> {
    match cron_state(store, name)? {
        (p, _) if p == "Forbid" => Some("Scheduled runs are skipped while this Job runs (concurrency: Forbid)."),
        (p, true) if p == "Replace" => Some("Its running Job is not replaced; but the next scheduled run replaces this one if it's still running."),
        (p, false) if p == "Replace" => Some("The next scheduled run replaces this Job if it's still running (concurrency: Replace)."),
        _ => None,
    }
}

/// A release's revisions from its loaded History: (revision, status, chart,
/// app version, updated), newest first.
fn history(store: Store, name: &str, tracked: bool) -> Vec<(i64, String, String, String, String)> {
    let read = |o: &Option<Result<subduer_types::ObjectDetails, String>>| match o {
        Some(Ok(d)) if d.name == name && d.kind == "Release" => d
            .sections
            .iter()
            .find(|s| s.id == "helm-history")
            .and_then(|s| s.table.as_ref())
            .map(|t| {
                t.rows
                    .iter()
                    .filter_map(|r| {
                        let c = &r.cells;
                        Some((c.first()?.parse().ok()?, c.get(1)?.clone(), c.get(2)?.clone(), c.get(3)?.clone(), c.get(4)?.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    if tracked { store.object.with(read) } else { store.object.with_untracked(read) }
}

/// The objects a release created, from its loaded Resources: (kind, name).
fn release_objects(store: Store, name: &str) -> Vec<(String, String)> {
    store.object.with_untracked(|o| match o {
        Some(Ok(d)) if d.name == name && d.kind == "Release" => d
            .sections
            .iter()
            .find(|s| s.id == "helm-resources")
            .and_then(|s| s.table.as_ref())
            .map(|t| t.rows.iter().filter_map(|r| Some((r.cells.first()?.clone(), r.cells.get(1)?.clone()))).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    })
}

/// Why Force sync won't fetch, from the loaded ExternalSecret's refresh rows.
fn force_sync_note(store: Store, name: &str) -> Option<&'static str> {
    if object_row(store, name, "Refresh policy").is_some_and(|p| p.starts_with("CreatedOnce")) {
        return Some("Its refresh policy is CreatedOnce: External Secrets fetched it once and ignores this annotation.");
    }
    if object_row(store, name, "Refresh interval").is_some_and(|i| i.contains("fetched once")) {
        return Some("Its refresh interval is 0: External Secrets fetched it once and ignores this annotation.");
    }
    None
}

/// How many ExternalSecrets the loaded ClusterExternalSecret has made.
fn provisioned_count(store: Store, name: &str) -> usize {
    store.object.with_untracked(|o| match o {
        Some(Ok(d)) if d.name == name => d
            .sections
            .iter()
            .find(|s| s.id == "externalsecret-children")
            .and_then(|s| s.table.as_ref())
            .map_or(0, |t| t.rows.iter().filter(|r| r.link.is_some()).count()),
        _ => 0,
    })
}

/// A size like `20Gi`, `500M` or `1e9` in bytes.
fn bytes(s: &str) -> Option<f64> {
    // An exponent (`1e9`) is part of the number, so the suffix is matched
    // from the end rather than split off at the first non-digit.
    const SUFFIXES: [(&str, f64); 9] = [
        ("Ki", 1024.0),
        ("Mi", 1048576.0),
        ("Gi", 1073741824.0),
        ("Ti", 1099511627776.0),
        ("Pi", 1125899906842624.0),
        ("k", 1e3),
        ("M", 1e6),
        ("G", 1e9),
        ("T", 1e12),
    ];
    let (number, k) =
        SUFFIXES.iter().find_map(|(suffix, k)| s.strip_suffix(suffix).map(|n| (n, *k))).unwrap_or((s, 1.0));
    // Plain decimals only: no sign, `inf` or `NaN`.
    if !number.starts_with(|c: char| c.is_ascii_digit() || c == '.')
        || !number.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
    {
        return None;
    }
    let n: f64 = number.parse().ok()?;
    Some(n * k)
}

/// An info row of the selected object's details, e.g. an HPA's `Min replicas`.
fn object_row(store: Store, name: &str, key: &str) -> Option<String> {
    store.object.with_untracked(|o| match o {
        Some(Ok(d)) if d.name == name => d.info.iter().find(|r| r.key == key).map(|r| r.value.clone()),
        _ => None,
    })
}

fn suspend_note(kind: &KindRef, suspend: bool) -> &'static str {
    match (kind.kind.as_str(), suspend) {
        ("CronJob", true) => "No new Jobs start until it's resumed. Jobs already running carry on.",
        ("CronJob", false) => "Jobs start on its schedule again. If a run was missed while suspended, one may start right away.",
        (_, true) => "Its running pods are stopped. Pods that already succeeded still count when it's resumed.",
        (_, false) => "New pods start for the work that's left.",
    }
}

fn restart_note(kind: &KindRef) -> &'static str {
    if kind.kind == "StatefulSet" {
        "Its pods are replaced one at a time, highest ordinal first."
    } else {
        "New pods replace the current ones following its rollout strategy."
    }
}

fn scale_note(kind: &KindRef, from: i32, to: i32) -> Option<&'static str> {
    if to == 0 && from > 0 {
        Some("Every pod is stopped.")
    } else if to < from && kind.kind == "StatefulSet" {
        Some("Pods with the highest ordinals are removed; their volume claims are kept.")
    } else {
        None
    }
}

#[component]
pub fn WorkloadDialog(store: Store) -> impl IntoView {
    let typed = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let current = RwSignal::new(None::<Replicas>);
    let wanted = RwSignal::new(String::new());
    let (min, max) = (RwSignal::new(String::new()), RwSignal::new(String::new()));
    let size = RwSignal::new(String::new());
    let rollback_to = RwSignal::new(String::new());
    let wait = RwSignal::new(true);
    let keep_history = RwSignal::new(false);
    let close = move || {
        typed.set(String::new());
        keep_history.set(false);
        error.set(None);
        busy.set(false);
        current.set(None);
        wanted.set(String::new());
        store.workload_prompt.set(None);
    };

    // The Scale dialog starts from the live replica count, Edit range from
    // the HPA's current range.
    Effect::new(move |_| {
        let Some(target) = store.workload_prompt.get() else { return };
        if target.op == Op::Range {
            min.set(object_row(store, &target.name, "Min replicas").unwrap_or_default());
            max.set(object_row(store, &target.name, "Max replicas").unwrap_or_default());
            return;
        }
        if target.op == Op::Expand {
            size.set(object_row(store, &target.name, "Requested").unwrap_or_default());
            return;
        }
        if target.op == Op::Rollback {
            // The latest earlier revision that didn't fail.
            let revisions = history(store, &target.name, false);
            let pick = revisions.iter().skip(1).find(|r| r.1 != "failed").or_else(|| revisions.get(1));
            rollback_to.set(pick.map(|r| r.0.to_string()).unwrap_or_default());
            return;
        }
        if target.op != Op::Scale {
            return;
        }
        spawn_local(async move {
            let args = Target { kind: &target.kind, namespace: &target.namespace, name: &target.name };
            let result = invoke::<_, Replicas>("get_replicas", &args).await;
            if store.workload_prompt.with_untracked(|p| p.as_ref() != Some(&target)) {
                return;
            }
            match result {
                Ok(r) => {
                    wanted.set(r.desired.to_string());
                    current.set(Some(r));
                }
                Err(e) => error.set(Some(e)),
            }
        });
    });

    move || {
        store.workload_prompt.get().map(|target| {
            let WorkloadTarget { context, kind, namespace, name, op, save } = target.clone();
            let prod = context.prod_like;
            let replicas = move || wanted.get().trim().parse::<i32>().ok().filter(|n| *n >= 0);
            let range = move || {
                let parse = |s: RwSignal<String>| s.get().trim().parse::<i32>().ok();
                parse(min).zip(parse(max)).filter(|(lo, hi)| *lo >= 1 && hi >= lo)
            };
            let current_size = object_row(store, &name, "Requested").unwrap_or_default();
            let grows = {
                let current = current_size.clone();
                move || size.with(|s| bytes(s.trim()).zip(bytes(&current)).is_some_and(|(new, old)| new > old))
            };
            // Reclaim switches to the other policy.
            let new_policy = match object_row(store, &name, "Reclaim policy") {
                Some(p) if p.starts_with("Retain") => "Delete",
                _ => "Retain",
            };
            let range_now = {
                let name = name.clone();
                move || {
                    let get = |k: &str| object_row(store, &name, k).and_then(|v| v.parse::<i32>().ok());
                    get("Min replicas").zip(get("Max replicas"))
                }
            };
            let ok = {
                let name = name.clone();
                let grows = grows.clone();
                move || {
                    // Deleting a whole release always needs the name typed.
                    let typed_ok = !(prod || op == Op::Uninstall) || typed.get().trim() == name;
                    let scale_ok = match op {
                        Op::Scale => current.get().zip(replicas()).is_some_and(|(c, n)| c.desired != n),
                        Op::Range => range().is_some_and(|r| Some(r) != range_now()),
                        Op::Expand => grows(),
                        Op::Rollback => rollback_to.with(|r| r.parse::<i64>().is_ok()),
                        _ => true,
                    };
                    !busy.get() && typed_ok && scale_ok
                }
            };
            let confirm = {
                let target = target.clone();
                move |_| {
                    let target = target.clone();
                    let replicas = replicas();
                    let range = range();
                    let size = size.get_untracked().trim().to_owned();
                    let revision = rollback_to.get_untracked().parse::<i64>().ok();
                    let wait = wait.get_untracked();
                    let keep_history = keep_history.get_untracked();
                    busy.set(true);
                    error.set(None);
                    spawn_local(async move {
                        let (command, replicas, suspend) = match target.op {
                            Op::Restart => ("restart_workload", None, None),
                            Op::Scale => ("scale_workload", replicas, None),
                            Op::Run => ("run_cron_job", None, None),
                            Op::Suspend => ("suspend_workload", None, Some(true)),
                            Op::Resume => ("suspend_workload", None, Some(false)),
                            Op::Range => ("set_hpa_range", None, None),
                            Op::Save => ("save_config_data", None, None),
                            Op::Expand => ("expand_claim", None, None),
                            Op::Reclaim => ("set_reclaim_policy", None, None),
                            Op::Rollback => ("helm_rollback", None, None),
                            Op::Uninstall => ("helm_uninstall", None, None),
                            Op::ForceSync => ("force_sync", None, None),
                        };
                        let args = Change {
                            context: &target.context.name,
                            target: Target { kind: &target.kind, namespace: &target.namespace, name: &target.name },
                            replicas,
                            suspend,
                            min: range.map(|r| r.0),
                            max: range.map(|r| r.1),
                            resource_version: target.save.as_ref().map(|s| s.resource_version.as_str()),
                            changes: target.save.as_ref().map(|s| s.changes.as_slice()),
                            size: (target.op == Op::Expand).then_some(size.as_str()),
                            policy: (target.op == Op::Reclaim).then_some(new_policy),
                            revision: (target.op == Op::Rollback).then_some(revision).flatten(),
                            wait: matches!(target.op, Op::Rollback | Op::Uninstall).then_some(wait),
                            keep_history: (target.op == Op::Uninstall).then_some(keep_history),
                        };
                        // Run now answers with the new Job's name, Force sync with a
                        // count; the others with nothing.
                        match invoke::<_, serde_json::Value>(command, &args).await {
                            Ok(_) => {
                                close();
                                if target.op == Op::Save {
                                    store.editing.set(false);
                                }
                                if target.op == Op::Uninstall {
                                    // It's gone; the table drops it when the watch catches up.
                                    store.selected.set(None);
                                    return;
                                }
                                crate::pod_detail::load(store, true);
                            }
                            Err(e) => {
                                busy.set(false);
                                error.set(Some(e));
                            }
                        }
                    });
                }
            };
            let what = view! {
                <strong class="mono">{name.clone()}</strong>
                {(!namespace.is_empty()).then(|| view! { " in namespace "<strong>{namespace.clone()}</strong> })}
                " on context "<strong>{context.name.clone()}</strong>
            };
            let body = match op {
                Op::Restart => view! {
                    <p>{what}" will be restarted."</p>
                    <p>{restart_note(&kind)}</p>
                }
                .into_any(),
                Op::Run => view! {
                    <p>"A Job is started now from the template of "{what}", outside its schedule."</p>
                    <p class="muted">"It's named "<span class="mono">{format!("{name}-manual-…")}</span>" and listed with its other Jobs."</p>
                    {run_note(store, &name).map(|n| view! { <p class="warn-text">{n}</p> })}
                }
                .into_any(),
                Op::Suspend | Op::Resume => {
                    let verb = if op == Op::Suspend { " will be suspended." } else { " will be resumed." };
                    view! {
                        <p>{what}{verb}</p>
                        <p>{suspend_note(&kind, op == Op::Suspend)}</p>
                    }
                    .into_any()
                }
                Op::Range => {
                    let owner = object_row(store, &name, "Controlled by");
                    let input = |label: &'static str, value: RwSignal<String>| view! {
                        <label class="replicas">
                            {label}
                            <input
                                class="confirm-input replicas-input"
                                type="number"
                                min="1"
                                prop:value=move || value.get()
                                on:input=move |ev| value.set(event_target_value(&ev))
                            />
                        </label>
                    };
                    view! {
                        <p>"Change the replica range of "{what}"."</p>
                        {input("Min ", min)}
                        {input("Max ", max)}
                        {move || range().is_none().then(|| view! { <p class="warn-text">"Min must be at least 1, and max at least min."</p> })}
                        {owner.map(|o| view! {
                            <p class="warn-text">{format!("It's controlled by {o}, which sets the range itself: change it there, or this edit is undone.")}</p>
                        })}
                    }
                    .into_any()
                }
                Op::Expand => view! {
                    <p>"Grow "{what}" from "<strong>{current_size.clone()}</strong>"."</p>
                    <label class="replicas">
                        "New size "
                        <input
                            class="confirm-input replicas-input size-input"
                            placeholder="e.g. 20Gi"
                            prop:value=move || size.get()
                            on:input=move |ev| size.set(event_target_value(&ev))
                        />
                    </label>
                    {
                        let grows = grows.clone();
                        move || (!grows()).then(|| view! { <p class="warn-text">"Enter a size bigger than now, like 20Gi."</p> })
                    }
                    <p class="muted">"Claims can't shrink again. The filesystem grows when the driver resizes it, which for some drivers needs the pod restarted."</p>
                }
                .into_any(),
                Op::Uninstall => {
                    let objects = release_objects(store, &name);
                    let shown = objects.len().min(8);
                    let more = objects.len() - shown;
                    let list = objects
                        .iter()
                        .take(shown)
                        .map(|(kind, obj)| view! { <li>{kind.clone()}" "<span class="mono">{obj.clone()}</span></li> })
                        .collect_view();
                    view! {
                        <p>{what}" will be uninstalled with helm uninstall."</p>
                        {(!objects.is_empty()).then(|| view! {
                            <p>"It deletes what the release created:"</p>
                            <ul class="change-list">
                                {list}
                                {(more > 0).then(|| view! { <li class="muted">{format!("and {more} more")}</li> })}
                            </ul>
                        })}
                        <p class="warn-text">"Data in those objects is lost. Objects marked helm.sh/resource-policy: keep stay, and so do volume claims a StatefulSet created."</p>
                        <label class="check-row">
                            <input type="checkbox" prop:checked=move || keep_history.get() on:change=move |ev| keep_history.set(event_target_checked(&ev)) />
                            " Keep its history, so it can be rolled back later"
                        </label>
                        <label class="check-row">
                            <input type="checkbox" prop:checked=move || wait.get() on:change=move |ev| wait.set(event_target_checked(&ev)) />
                            " Wait until everything is deleted (up to 5 minutes)"
                        </label>
                    }
                    .into_any()
                }
                Op::Rollback => {
                    let revisions = history(store, &name, false);
                    let current = revisions.first().cloned();
                    let options = revisions
                        .iter()
                        .skip(1)
                        .map(|(rev, status, chart, app, _)| {
                            let label = format!("{rev} · {chart}{} · {status}", if app.is_empty() { String::new() } else { format!(" (app {app})") });
                            view! { <option value=rev.to_string()>{label}</option> }
                        })
                        .collect_view();
                    let chosen = {
                        let revisions = revisions.clone();
                        move || {
                            let r: i64 = rollback_to.get().parse().ok()?;
                            revisions.iter().find(|x| x.0 == r).cloned()
                        }
                    };
                    let next = current.as_ref().map_or(0, |c| c.0 + 1);
                    view! {
                        <p>"Roll "{what}" back."</p>
                        {current.map(|(rev, status, chart, _, _)| view! {
                            <p class="muted">{format!("Now at revision {rev} ({chart}, {status}).")}</p>
                        })}
                        <label class="rollback-field">
                            "To revision"
                            <select class="rollback-select" prop:value=move || rollback_to.get() on:change=move |ev| rollback_to.set(event_target_value(&ev))>
                                {options}
                            </select>
                        </label>
                        <label class="check-row">
                            <input type="checkbox" prop:checked=move || wait.get() on:change=move |ev| wait.set(event_target_checked(&ev)) />
                            " Wait until its resources are ready (up to 5 minutes)"
                        </label>
                        {move || chosen().filter(|c| c.1 == "failed").map(|_| view! {
                            <p class="warn-text">"That revision failed when it was deployed."</p>
                        })}
                        <p class="muted">{format!(
                            "Helm re-applies that revision's manifest and values as a new revision {next} and runs its rollback hooks. Changes made outside Helm may be overwritten."
                        )}</p>
                        <p class="muted small">"Runs helm rollback with your kubeconfig and this context."</p>
                    }
                    .into_any()
                }
                Op::ForceSync => {
                    let cluster = kind.kind == "ClusterExternalSecret";
                    let children = if cluster { provisioned_count(store, &name) } else { 0 };
                    view! {
                        <p>
                            {what}" gets the annotation "<span class="mono">"force-sync"</span>
                            {if cluster {
                                format!(
                                    " set to now, and so {}. External Secrets then fetches from the store and updates each Secret.",
                                    match children {
                                        0 => "will each ExternalSecret it makes".to_owned(),
                                        1 => "does its ExternalSecret".to_owned(),
                                        n => format!("do its {n} ExternalSecrets"),
                                    }
                                )
                            } else {
                                " set to now. External Secrets then fetches from the store and updates the Secret.".to_owned()
                            }}
                        </p>
                        {force_sync_note(store, &name).map(|n| view! { <p class="warn-text">{n}</p> })}
                        <p class="muted">"Pods that read the Secret as environment variables keep the old values until they restart; mounted files update within about a minute."</p>
                    }
                    .into_any()
                }
                Op::Reclaim => {
                    let note = if new_policy == "Delete" {
                        "Once its claim is deleted, the volume and all its data are deleted too."
                    } else {
                        "Once its claim is deleted, the volume and its data are kept; you clean them up yourself."
                    };
                    view! {
                        <p>"Change the reclaim policy of "{what}" to "<strong>{new_policy}</strong>"?"</p>
                        <p class=if new_policy == "Delete" { "warn-text" } else { "muted" }>{note}</p>
                    }
                    .into_any()
                }
                Op::Save => {
                    let save = save.clone().unwrap_or_else(|| SaveData {
                        resource_version: String::new(),
                        changes: Vec::new(),
                        added: Vec::new(),
                        changed: Vec::new(),
                        removed: Vec::new(),
                    });
                    let line = |label: &'static str, keys: &[String]| {
                        (!keys.is_empty()).then(|| view! { <li>{label}" "<span class="mono">{keys.join(", ")}</span></li> })
                    };
                    let note = "Pods that read it as environment variables keep the old values until they restart; mounted files update within about a minute (not subPath mounts).";
                    view! {
                        <p>"Save changes to "{what}"?"</p>
                        <ul class="change-list">
                            {line("Changed:", &save.changed)}
                            {line("Added:", &save.added)}
                            {line("Removed:", &save.removed)}
                        </ul>
                        <p class="muted">{note}</p>
                    }
                    .into_any()
                }
                Op::Scale => {
                    let kind = kind.clone();
                    view! {
                        <p>"Scale "{what}"."</p>
                        {move || match current.get() {
                            None if error.with(Option::is_none) => view! { <p class="muted">"Reading replicas…"</p> }.into_any(),
                            None => ().into_any(),
                            Some(r) => view! {
                                <p class="muted">{format!("Now {} desired, {} running.", r.desired, r.current)}</p>
                                <label class="replicas">
                                    "Replicas "
                                    <input
                                        class="confirm-input replicas-input"
                                        type="number"
                                        min="0"
                                        prop:value=move || wanted.get()
                                        on:input=move |ev| wanted.set(event_target_value(&ev))
                                    />
                                </label>
                            }
                            .into_any(),
                        }}
                        {move || {
                            let note = current.get().zip(replicas()).and_then(|(c, n)| scale_note(&kind, c.desired, n));
                            note.map(|n| view! { <p class="warn-text">{n}</p> })
                        }}
                    }
                    .into_any()
                }
            };
            let (title, doing) = match op {
                Op::Restart => (format!("Restart {}?", kind.kind), "Restarting…"),
                Op::Scale => (format!("Scale {}?", kind.kind), "Scaling…"),
                Op::Run => (format!("Run {} now?", kind.kind), "Starting…"),
                Op::Suspend => (format!("Suspend {}?", kind.kind), "Suspending…"),
                Op::Resume => (format!("Resume {}?", kind.kind), "Resuming…"),
                Op::Range => ("Edit HPA range?".to_owned(), "Saving…"),
                Op::Expand => ("Expand claim?".to_owned(), "Expanding…"),
                Op::Reclaim => ("Change reclaim policy?".to_owned(), "Saving…"),
                Op::Rollback => ("Roll back Helm release?".to_owned(), "Rolling back…"),
                Op::Uninstall => ("Delete Helm release?".to_owned(), "Deleting…"),
                Op::Save => (format!("Save {}?", kind.kind), "Saving…"),
                Op::ForceSync => (format!("Force sync {}?", kind.kind), "Syncing…"),
            };
            let label = move || {
                if busy.get() {
                    doing.to_owned()
                } else if op == Op::Scale {
                    replicas().map_or_else(|| "Scale".to_owned(), |n| format!("Scale to {n}"))
                } else if op == Op::Uninstall {
                    "Delete release".to_owned()
                } else if op == Op::Rollback {
                    rollback_to.with(|r| if r.is_empty() { "Rollback".to_owned() } else { format!("Roll back to {r}") })
                } else if op == Op::Reclaim {
                    format!("Set {new_policy}")
                } else if op == Op::Expand {
                    size.with(|s| if s.trim().is_empty() { "Expand".to_owned() } else { format!("Expand to {}", s.trim()) })
                } else if op == Op::Range {
                    range().map_or_else(|| "Save".to_owned(), |(lo, hi)| format!("Set {lo}–{hi}"))
                } else {
                    op.label().to_owned()
                }
            };
            view! {
                <Modal on_cancel=move |()| close() busy=busy prod=context.prod_like wide=op == Op::Rollback>
                        <h2>{title}</h2>
                        {body}
                        {context.prod_like.then(|| view! {
                            <p>"This context looks like "<strong>"PRODUCTION"</strong>"."</p>
                        })}
                        {(context.prod_like || op == Op::Uninstall).then(|| view! {
                            <p>"Type the name to confirm:"</p>
                            <input
                                class="confirm-input"
                                prop:value=move || typed.get()
                                on:input=move |ev| typed.set(event_target_value(&ev))
                            />
                        })}
                        {move || error.get().map(|e| view! { <p class="warn">{e}</p> })}
                        <div class="modal-actions">
                            // Leaving the Save dialog never saves; it goes back to the edit.
                            <button prop:disabled=move || busy.get() on:click=move |_| close()>{if op == Op::Save { "Back to editing" } else { "Cancel" }}</button>
                            <button
                                class=if op == Op::Uninstall { "danger" } else { "primary" }
                                prop:disabled=move || !ok()
                                on:click=confirm.clone()
                            >{label}</button>
                        </div>
                </Modal>
            }
        })
    }
}

#[cfg(test)]
#[path = "workload_tests.rs"]
mod tests_unit;
