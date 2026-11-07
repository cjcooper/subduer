//! The logic behind the workload dialog (`crate::workload`): which actions an
//! object offers, what the loaded details say, the dialog's validation,
//! texts and the backend request each action sends.

use serde::Serialize;
use subduer_types::{
    ClusterInfo, ContextInfo, DataChange, KindRef, ObjectDetails, Replicas, ResourceRow, WorkloadDetails, can_force_sync,
    has_workload_details, is_keda_scaler, is_scalable_workload,
};

/// A signal's value: not loaded yet, loaded, or failed.
pub(crate) type Loaded<T> = Option<Result<T, String>>;

/// A Helm release revision: (revision, status, chart, app version, updated).
pub(crate) type Revision = (i64, String, String, String, String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Restart,
    Scale,
    /// Start a Job from a CronJob now.
    Run,
    Suspend,
    Resume,
    /// An HPA's, ScaledObject's or ScaledJob's min and max replicas.
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
    /// Stop KEDA scaling a ScaledObject or ScaledJob.
    Pause,
    /// Let KEDA scale it again.
    Unpause,
}

impl Op {
    pub(crate) fn label(self) -> &'static str {
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
            Op::Pause => "Pause",
            Op::Unpause => "Resume",
        }
    }

    pub(crate) fn hint(self, kind: &KindRef) -> &'static str {
        match (self, kind.kind.as_str()) {
            (Op::Restart, _) => "Rollout restart",
            (Op::Scale, _) => "Scale",
            (Op::Run, _) => "Start a Job from its template now",
            (Op::Suspend, "CronJob") => "Stop starting new Jobs",
            (Op::Suspend, _) => "Stop its running pods until resumed",
            (Op::Resume, "CronJob") => "Start Jobs on its schedule again",
            (Op::Resume, _) => "Start its pods again",
            (Op::Range, "ScaledJob") => "Change how many Jobs it may run at once",
            (Op::Range, _) => "Change its min and max replicas",
            (Op::Save, _) => "Save the edited data",
            (Op::Expand, _) => "Make the claim bigger",
            (Op::Reclaim, _) => "Keep or delete the data when its claim is deleted",
            (Op::Rollback, _) => "Go back to an earlier revision with helm rollback",
            (Op::Uninstall, _) => "Delete the release and everything it created, with helm uninstall",
            (Op::ForceSync, "ClusterExternalSecret") => "Have External Secrets fetch from the store now, for every namespace",
            (Op::ForceSync, _) => "Have External Secrets fetch from the store now and update the Secret",
            (Op::Pause, "ScaledJob") => "Stop KEDA starting new Jobs",
            (Op::Pause, _) => "Stop KEDA scaling it, where it is or at a replica count",
            (Op::Unpause, _) => "Let KEDA scale it from its triggers again",
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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SaveData {
    pub resource_version: String,
    pub changes: Vec<DataChange>,
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Target<'a> {
    kind: &'a KindRef,
    namespace: &'a str,
    name: &'a str,
}

/// The object a target names, as the backend's commands take it.
pub(crate) fn target_args(t: &WorkloadTarget) -> Target<'_> {
    Target { kind: &t.kind, namespace: &t.namespace, name: &t.name }
}

#[derive(Debug, Serialize)]
pub(crate) struct Change<'a> {
    context: &'a str,
    #[serde(flatten)]
    target: Target<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    replicas: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    suspend: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    paused: Option<bool>,
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

/// The dialog's entries when Confirm is clicked.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Choices {
    pub(crate) replicas: Option<i32>,
    pub(crate) range: Option<(i32, i32)>,
    pub(crate) held: Option<i32>,
    pub(crate) size: String,
    pub(crate) revision: Option<i64>,
    pub(crate) policy: &'static str,
    pub(crate) wait: bool,
    pub(crate) keep_history: bool,
}

/// The backend command for the target's action and its arguments.
pub(crate) fn change<'a>(t: &'a WorkloadTarget, c: &'a Choices) -> (&'static str, Change<'a>) {
    let paused = match t.op {
        Op::Pause => Some(true),
        Op::Unpause => Some(false),
        _ => None,
    };
    let (command, replicas, suspend) = match t.op {
        Op::Restart => ("restart_workload", None, None),
        Op::Scale => ("scale_workload", c.replicas, None),
        Op::Run => ("run_cron_job", None, None),
        Op::Suspend => ("suspend_workload", None, Some(true)),
        Op::Resume => ("suspend_workload", None, Some(false)),
        Op::Range if is_keda_scaler(&t.kind) => ("set_scaling_range", None, None),
        Op::Range => ("set_hpa_range", None, None),
        Op::Save => ("save_config_data", None, None),
        Op::Expand => ("expand_claim", None, None),
        Op::Reclaim => ("set_reclaim_policy", None, None),
        Op::Rollback => ("helm_rollback", None, None),
        Op::Uninstall => ("helm_uninstall", None, None),
        Op::ForceSync => ("force_sync", None, None),
        Op::Pause => ("pause_scaling", c.held, None),
        Op::Unpause => ("pause_scaling", None, None),
    };
    let args = Change {
        context: &t.context.name,
        target: target_args(t),
        replicas,
        suspend,
        paused,
        min: c.range.map(|r| r.0),
        max: c.range.map(|r| r.1),
        resource_version: t.save.as_ref().map(|s| s.resource_version.as_str()),
        changes: t.save.as_ref().map(|s| s.changes.as_slice()),
        size: (t.op == Op::Expand).then_some(c.size.as_str()),
        policy: (t.op == Op::Reclaim).then_some(c.policy),
        revision: (t.op == Op::Rollback).then_some(c.revision).flatten(),
        wait: matches!(t.op, Op::Rollback | Op::Uninstall).then_some(c.wait),
        keep_history: (t.op == Op::Uninstall).then_some(c.keep_history),
    };
    (command, args)
}

/// After a change succeeds: (leave data editing, clear the selection). An
/// uninstalled release is gone, so the selection goes and nothing reloads.
pub(crate) fn after_success(op: Op) -> (bool, bool) {
    (op == Op::Save, op == Op::Uninstall)
}

/// A target for `op` on `row`, which has no namespace when cluster-scoped.
pub(crate) fn target_for(context: ContextInfo, kind: KindRef, row: &ResourceRow, op: Op) -> WorkloadTarget {
    WorkloadTarget { context, kind, namespace: row.namespace.clone().unwrap_or_default(), name: row.name.clone(), op, save: None }
}

// ---- Which buttons ----

/// Kinds that may get workload buttons.
pub(crate) fn has_ops(k: &KindRef) -> bool {
    has_workload_details(k)
        || can_force_sync(k)
        || is_keda_scaler(k)
        || matches!(
            k.key().as_str(),
            "autoscaling/HorizontalPodAutoscaler" | "core/PersistentVolumeClaim" | "core/PersistentVolume" | "helm.sh/Release"
        )
}

/// Whether the API lists `verb` for the kind with `key`.
pub(crate) fn allowed(cluster: Option<&ClusterInfo>, key: &str, verb: &str) -> bool {
    cluster.iter().flat_map(|c| &c.kinds).any(|k| k.kind.key() == key && k.verbs.iter().any(|v| v == verb))
}

/// The actions offered for the selected `row` of `kind`. Each is left out
/// when the API doesn't allow it; some wait for the loaded details.
pub(crate) fn ops_for(
    kind: &KindRef,
    row: &ResourceRow,
    cluster: Option<&ClusterInfo>,
    object: &Loaded<ObjectDetails>,
    workload: &Loaded<WorkloadDetails>,
) -> Vec<Op> {
    let patch = allowed(cluster, &kind.key(), "patch");
    let detail = |key: &str| object_row(object, &row.name, key);
    let mut ops = Vec::new();
    if kind.key() == "helm.sh/Release" {
        // Helm runs it with your own credentials, so the listed verbs don't apply.
        let revisions = history(object, &row.name).len();
        if revisions > 1 {
            ops.push(Op::Rollback);
        }
        if revisions > 0 {
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
    } else if can_force_sync(kind) {
        if patch {
            ops.push(Op::ForceSync);
        }
    } else if is_keda_scaler(kind) {
        // Both start from the loaded details: the range, and whether it's paused.
        let loaded = matches!(object, Some(Ok(d)) if d.name == row.name);
        if patch && loaded {
            ops.push(Op::Range);
            ops.push(if detail("Paused").is_some() { Op::Unpause } else { Op::Pause });
        }
    } else if is_scalable_workload(kind) {
        if patch {
            ops.extend([Op::Restart, Op::Scale]);
        }
    } else {
        if kind.kind == "CronJob" && allowed(cluster, "batch/Job", "create") {
            ops.push(Op::Run);
        }
        let state = match workload {
            Some(Ok(d)) if d.name == row.name && row.namespace.as_ref() == Some(&d.namespace) && d.kind == kind.kind => {
                Some((d.suspended, d.status.clone()))
            }
            _ => None,
        };
        // A finished Job has nothing left to suspend.
        if let Some((Some(suspended), status)) = state
            && patch
            && !(kind.kind == "Job" && matches!(status.as_str(), "Complete" | "Failed"))
        {
            ops.push(if suspended { Op::Resume } else { Op::Suspend });
        }
    }
    ops
}

/// A button's tooltip.
pub(crate) fn button_title(hint: &str, locked: bool) -> String {
    if locked { format!("{hint}… (asks to allow changes first: the context is read-only)") } else { format!("{hint}…") }
}

// ---- The loaded details ----

/// The selected CronJob's concurrency policy and whether a Job is running.
pub(crate) fn cron_state(w: &Loaded<WorkloadDetails>, name: &str) -> Option<(String, bool)> {
    match w {
        Some(Ok(d)) if d.name == name && d.kind == "CronJob" => {
            let policy = d.info.iter().find(|(k, _)| k == "Concurrency").map(|(_, v)| v.clone())?;
            Some((policy, d.status == "Running"))
        }
        _ => None,
    }
}

pub(crate) fn run_note(w: &Loaded<WorkloadDetails>, name: &str) -> Option<&'static str> {
    match cron_state(w, name)? {
        (p, _) if p == "Forbid" => Some("Scheduled runs are skipped while this Job runs (concurrency: Forbid)."),
        (p, true) if p == "Replace" => Some("Its running Job is not replaced; but the next scheduled run replaces this one if it's still running."),
        (p, false) if p == "Replace" => Some("The next scheduled run replaces this Job if it's still running (concurrency: Replace)."),
        _ => None,
    }
}

/// A release's revisions from its loaded History, newest first.
pub(crate) fn history(o: &Loaded<ObjectDetails>, name: &str) -> Vec<Revision> {
    match o {
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
    }
}

/// The objects a release created, from its loaded Resources: (kind, name).
pub(crate) fn release_objects(o: &Loaded<ObjectDetails>, name: &str) -> Vec<(String, String)> {
    match o {
        Some(Ok(d)) if d.name == name && d.kind == "Release" => d
            .sections
            .iter()
            .find(|s| s.id == "helm-resources")
            .and_then(|s| s.table.as_ref())
            .map(|t| t.rows.iter().filter_map(|r| Some((r.cells.first()?.clone(), r.cells.get(1)?.clone()))).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Why Force sync won't fetch, from the loaded ExternalSecret's refresh rows.
pub(crate) fn force_sync_note(o: &Loaded<ObjectDetails>, name: &str) -> Option<&'static str> {
    if object_row(o, name, "Refresh policy").is_some_and(|p| p.starts_with("CreatedOnce")) {
        return Some("Its refresh policy is CreatedOnce: External Secrets fetched it once and ignores this annotation.");
    }
    if object_row(o, name, "Refresh interval").is_some_and(|i| i.contains("fetched once")) {
        return Some("Its refresh interval is 0: External Secrets fetched it once and ignores this annotation.");
    }
    None
}

/// How many ExternalSecrets the loaded ClusterExternalSecret has made.
pub(crate) fn provisioned_count(o: &Loaded<ObjectDetails>, name: &str) -> usize {
    match o {
        Some(Ok(d)) if d.name == name => d
            .sections
            .iter()
            .find(|s| s.id == "externalsecret-children")
            .and_then(|s| s.table.as_ref())
            .map_or(0, |t| t.rows.iter().filter(|r| r.link.is_some()).count()),
        _ => 0,
    }
}

/// A size like `20Gi`, `500M` or `1e9` in bytes.
pub(crate) fn bytes(s: &str) -> Option<f64> {
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
pub(crate) fn object_row(o: &Loaded<ObjectDetails>, name: &str, key: &str) -> Option<String> {
    match o {
        Some(Ok(d)) if d.name == name => d.info.iter().find(|r| r.key == key).map(|r| r.value.clone()),
        _ => None,
    }
}

/// A ScaledObject's or ScaledJob's range from its loaded details:
/// `Range: 0 to 10 replicas`, `Jobs at once: 0 to 20`.
pub(crate) fn keda_range(o: &Loaded<ObjectDetails>, name: &str) -> Option<(i32, i32)> {
    let text = object_row(o, name, "Range").or_else(|| object_row(o, name, "Jobs at once"))?;
    let (lo, rest) = text.split_once(" to ")?;
    let hi = rest.split_whitespace().next()?;
    Some((lo.trim().parse().ok()?, hi.parse().ok()?))
}

/// `Some(release)` when the loaded object is managed by Helm (the release
/// name may be empty if it isn't annotated).
pub(crate) fn helm_release(o: &Loaded<ObjectDetails>, name: &str) -> Option<String> {
    match o {
        Some(Ok(d)) if d.name == name => {
            let helm = d.labels.iter().any(|(k, v)| k == "app.kubernetes.io/managed-by" && v == "Helm");
            helm.then(|| d.annotations.iter().find(|(k, _)| k == "meta.helm.sh/release-name").map(|(_, v)| v.clone()).unwrap_or_default())
        }
        _ => None,
    }
}

/// A ScaledObject's replicas now, from `Replicas: 3 now · 4 wanted`.
pub(crate) fn replicas_now(o: &Loaded<ObjectDetails>, name: &str) -> Option<i32> {
    object_row(o, name, "Replicas")?.split_whitespace().next()?.parse().ok()
}

/// The reclaim policy a PersistentVolume switches to: the other one.
pub(crate) fn new_policy(o: &Loaded<ObjectDetails>, name: &str) -> &'static str {
    match object_row(o, name, "Reclaim policy") {
        Some(p) if p.starts_with("Retain") => "Delete",
        _ => "Retain",
    }
}

/// The range set now: a KEDA scaler's, or an HPA's min and max.
pub(crate) fn range_now(o: &Loaded<ObjectDetails>, name: &str, keda: bool) -> Option<(i32, i32)> {
    if keda {
        return keda_range(o, name);
    }
    let get = |k: &str| object_row(o, name, k).and_then(|v| v.parse::<i32>().ok());
    get("Min replicas").zip(get("Max replicas"))
}

/// What the dialog's fields start at when it opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Prefill {
    /// Min and max.
    Range(String, String),
    /// The replica count to pause at.
    Hold(String),
    /// The claim's size.
    Size(String),
    /// The revision to roll back to.
    Rollback(String),
    /// Scale starts from the live replica count, read from the backend.
    FetchReplicas,
    Nothing,
}

pub(crate) fn prefill(t: &WorkloadTarget, o: &Loaded<ObjectDetails>) -> Prefill {
    match t.op {
        Op::Range if is_keda_scaler(&t.kind) => {
            let (lo, hi) = keda_range(o, &t.name).map_or_else(Default::default, |(lo, hi)| (lo.to_string(), hi.to_string()));
            Prefill::Range(lo, hi)
        }
        Op::Range => Prefill::Range(
            object_row(o, &t.name, "Min replicas").unwrap_or_default(),
            object_row(o, &t.name, "Max replicas").unwrap_or_default(),
        ),
        Op::Pause => Prefill::Hold(replicas_now(o, &t.name).unwrap_or(0).to_string()),
        Op::Expand => Prefill::Size(object_row(o, &t.name, "Requested").unwrap_or_default()),
        Op::Rollback => {
            // The latest earlier revision that didn't fail.
            let revisions = history(o, &t.name);
            let pick = revisions.iter().skip(1).find(|r| r.1 != "failed").or_else(|| revisions.get(1));
            Prefill::Rollback(pick.map(|r| r.0.to_string()).unwrap_or_default())
        }
        Op::Scale => Prefill::FetchReplicas,
        _ => Prefill::Nothing,
    }
}

// ---- The dialog's entries ----

/// A replica count: 0 or more.
pub(crate) fn parse_count(s: &str) -> Option<i32> {
    s.trim().parse::<i32>().ok().filter(|n| *n >= 0)
}

/// The lowest min: KEDA can scale to zero; an HPA can't.
pub(crate) fn range_floor(keda: bool) -> i32 {
    if keda { 0 } else { 1 }
}

/// A valid (min, max): min at least `floor`, max at least 1 and at least min.
pub(crate) fn parse_range(min: &str, max: &str, floor: i32) -> Option<(i32, i32)> {
    let parse = |s: &str| s.trim().parse::<i32>().ok();
    parse(min).zip(parse(max)).filter(|(lo, hi)| *lo >= floor && *hi >= 1 && hi >= lo)
}

/// Pausing at a count: `None` when not asked, `Some(None)` when the count is bad.
pub(crate) fn pause_hold(on: bool, count: &str) -> Option<Option<i32>> {
    on.then(|| parse_count(count))
}

/// Whether `size` is bigger than the claim's `current` size.
pub(crate) fn grows(size: &str, current: &str) -> bool {
    bytes(size.trim()).zip(bytes(current)).is_some_and(|(new, old)| new > old)
}

/// The dialog's state, for its checks and its button.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Form {
    pub(crate) busy: bool,
    pub(crate) typed: String,
    /// The live replica count (Scale).
    pub(crate) current: Option<Replicas>,
    pub(crate) replicas: Option<i32>,
    pub(crate) range: Option<(i32, i32)>,
    pub(crate) range_now: Option<(i32, i32)>,
    pub(crate) grows: bool,
    pub(crate) size: String,
    pub(crate) rollback_to: String,
    pub(crate) held: Option<Option<i32>>,
    pub(crate) policy: &'static str,
}

/// Whether the name has to be typed: only to delete a whole release.
/// Production-like contexts had theirs typed when changes were allowed.
pub(crate) fn needs_name(op: Op) -> bool {
    op == Op::Uninstall
}

/// Whether Confirm can be clicked.
pub(crate) fn can_confirm(op: Op, name: &str, f: &Form) -> bool {
    let typed_ok = !needs_name(op) || f.typed.trim() == name;
    let input_ok = match op {
        Op::Scale => f.current.zip(f.replicas).is_some_and(|(c, n)| c.desired != n),
        Op::Range => f.range.is_some_and(|r| Some(r) != f.range_now),
        Op::Expand => f.grows,
        Op::Rollback => f.rollback_to.parse::<i64>().is_ok(),
        Op::Pause => f.held != Some(None),
        _ => true,
    };
    !f.busy && typed_ok && input_ok
}

/// The dialog's title and its button's text while the change runs.
pub(crate) fn title(op: Op, kind: &KindRef) -> (String, &'static str) {
    let k = &kind.kind;
    match op {
        Op::Restart => (format!("Restart {k}?"), "Restarting…"),
        Op::Scale => (format!("Scale {k}?"), "Scaling…"),
        Op::Run => (format!("Run {k} now?"), "Starting…"),
        Op::Suspend => (format!("Suspend {k}?"), "Suspending…"),
        Op::Resume => (format!("Resume {k}?"), "Resuming…"),
        Op::Range if k == "ScaledJob" => ("Edit Jobs at once?".to_owned(), "Saving…"),
        Op::Range if is_keda_scaler(kind) => ("Edit replica range?".to_owned(), "Saving…"),
        Op::Range => ("Edit HPA range?".to_owned(), "Saving…"),
        Op::Pause => (format!("Pause {k}?"), "Pausing…"),
        Op::Unpause => (format!("Resume {k}?"), "Resuming…"),
        Op::Expand => ("Expand claim?".to_owned(), "Expanding…"),
        Op::Reclaim => ("Change reclaim policy?".to_owned(), "Saving…"),
        Op::Rollback => ("Roll back Helm release?".to_owned(), "Rolling back…"),
        Op::Uninstall => ("Delete Helm release?".to_owned(), "Deleting…"),
        Op::Save => (format!("Save {k}?"), "Saving…"),
        Op::ForceSync => (format!("Force sync {k}?"), "Syncing…"),
    }
}

/// The Confirm button's text.
pub(crate) fn confirm_label(op: Op, doing: &str, f: &Form) -> String {
    if f.busy {
        return doing.to_owned();
    }
    match op {
        Op::Scale => f.replicas.map_or_else(|| "Scale".to_owned(), |n| format!("Scale to {n}")),
        Op::Uninstall => "Delete release".to_owned(),
        Op::Rollback if f.rollback_to.is_empty() => "Rollback".to_owned(),
        Op::Rollback => format!("Roll back to {}", f.rollback_to),
        Op::Reclaim => format!("Set {}", f.policy),
        Op::Expand if f.size.trim().is_empty() => "Expand".to_owned(),
        Op::Expand => format!("Expand to {}", f.size.trim()),
        Op::Pause => f.held.flatten().map_or_else(|| "Pause".to_owned(), |n| format!("Pause at {n}")),
        Op::Range => f.range.map_or_else(|| "Save".to_owned(), |(lo, hi)| format!("Set {lo}–{hi}")),
        _ => op.label().to_owned(),
    }
}

/// The Confirm button's class.
pub(crate) fn confirm_class(op: Op) -> &'static str {
    if op == Op::Uninstall { "danger" } else { "primary" }
}

/// Leaving the Save dialog never saves; it goes back to the edit.
pub(crate) fn cancel_label(op: Op) -> &'static str {
    if op == Op::Save { "Back to editing" } else { "Cancel" }
}

// ---- The dialog's texts ----

/// What happens to the object: " will be suspended." and so on.
pub(crate) fn will_be(op: Op) -> &'static str {
    match op {
        Op::Suspend => " will be suspended.",
        Op::Pause => " will be paused.",
        _ => " will be resumed.",
    }
}

/// The name of a Job started by Run now.
pub(crate) fn manual_job(name: &str) -> String {
    format!("{name}-manual-…")
}

pub(crate) fn pause_note(kind: &KindRef, pause: bool) -> &'static str {
    match (kind.kind.as_str(), pause) {
        ("ScaledJob", true) => "KEDA starts no new Jobs until it's resumed. Jobs already running carry on.",
        (_, true) => "KEDA stops scaling it: its triggers are no longer acted on until it's resumed.",
        ("ScaledJob", false) => "KEDA starts Jobs from its triggers again.",
        (_, false) => "KEDA scales it from its triggers again, within its range.",
    }
}

/// What pausing a ScaledObject sets, given the count to pause at and the
/// replicas it has now.
pub(crate) fn pause_detail(held: Option<i32>, now: Option<i32>) -> String {
    match (held, now) {
        (Some(n), _) => format!("Sets autoscaling.keda.sh/paused-replicas to {n}."),
        (None, Some(n)) => format!("Sets autoscaling.keda.sh/paused; it stays at {n} replicas."),
        (None, None) => "Sets autoscaling.keda.sh/paused; it stays at the replicas it has.".to_owned(),
    }
}

pub(crate) fn suspend_note(kind: &KindRef, suspend: bool) -> &'static str {
    match (kind.kind.as_str(), suspend) {
        ("CronJob", true) => "No new Jobs start until it's resumed. Jobs already running carry on.",
        ("CronJob", false) => "Jobs start on its schedule again. If a run was missed while suspended, one may start right away.",
        (_, true) => "Its running pods are stopped. Pods that already succeeded still count when it's resumed.",
        (_, false) => "New pods start for the work that's left.",
    }
}

pub(crate) fn restart_note(kind: &KindRef) -> &'static str {
    if kind.kind == "StatefulSet" {
        "Its pods are replaced one at a time, highest ordinal first."
    } else {
        "New pods replace the current ones following its rollout strategy."
    }
}

pub(crate) fn scale_note(kind: &KindRef, from: i32, to: i32) -> Option<&'static str> {
    if to == 0 && from > 0 {
        Some("Every pod is stopped.")
    } else if to < from && kind.kind == "StatefulSet" {
        Some("Pods with the highest ordinals are removed; their volume claims are kept.")
    } else {
        None
    }
}

/// "Now 3 desired, 2 running."
pub(crate) fn replicas_text(r: Replicas) -> String {
    format!("Now {} desired, {} running.", r.desired, r.current)
}

/// The start of the Edit range text.
pub(crate) fn range_intro(jobs: bool) -> &'static str {
    if jobs { "Change how many Jobs at once " } else { "Change the replica range of " }
}

/// What makes a range valid.
pub(crate) fn range_rule(keda: bool) -> &'static str {
    if keda { "Min can't be below 0, max must be at least 1 and at least min." } else { "Min must be at least 1, and max at least min." }
}

/// Whether to explain scaling to zero: a ScaledObject with min 0.
pub(crate) fn zero_note(keda: bool, jobs: bool, range: Option<(i32, i32)>) -> bool {
    keda && !jobs && range.is_some_and(|(lo, _)| lo == 0)
}

/// The warning for a scaler a Helm release manages.
pub(crate) fn helm_note(release: &str) -> String {
    let release = if release.is_empty() { String::new() } else { format!(" (release {release})") };
    format!("Managed by Helm{release}: the next helm upgrade puts the chart's min and max back. Change them in its values too.")
}

/// The warning for an HPA another object controls.
pub(crate) fn owner_note(owner: &str) -> String {
    format!("It's controlled by {owner}, which sets the range itself: change it there, or this edit is undone.")
}

/// How many of a release's objects are listed, and how many more there are.
pub(crate) fn listed(count: usize) -> (usize, usize) {
    let shown = count.min(8);
    (shown, count - shown)
}

/// A revision in the Rollback list: `2 · shop-1.1.0 (app 1.1) · superseded`.
pub(crate) fn revision_label(r: &Revision) -> String {
    let (rev, status, chart, app, _) = r;
    format!("{rev} · {chart}{} · {status}", if app.is_empty() { String::new() } else { format!(" (app {app})") })
}

/// "Now at revision 3 (shop-1.2.0, deployed)."
pub(crate) fn current_revision(r: &Revision) -> String {
    format!("Now at revision {} ({}, {}).", r.0, r.2, r.1)
}

/// What a rollback does, given the revisions newest first.
pub(crate) fn rollback_note(revisions: &[Revision]) -> String {
    let next = revisions.first().map_or(0, |c| c.0 + 1);
    format!(
        "Helm re-applies that revision's manifest and values as a new revision {next} and runs its rollback hooks. Changes made outside Helm may be overwritten."
    )
}

/// Whether the chosen revision failed when it was deployed.
pub(crate) fn chose_failed(revisions: &[Revision], chosen: &str) -> bool {
    chosen.parse::<i64>().ok().and_then(|r| revisions.iter().find(|x| x.0 == r)).is_some_and(|c| c.1 == "failed")
}

/// What Force sync does, for a ClusterExternalSecret (`cluster`) with
/// `children` ExternalSecrets, or for an ExternalSecret.
pub(crate) fn force_sync_text(cluster: bool, children: usize) -> String {
    if cluster {
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
    }
}

/// What the new reclaim policy means, and the note's class.
pub(crate) fn reclaim_note(policy: &str) -> (&'static str, &'static str) {
    if policy == "Delete" {
        ("Once its claim is deleted, the volume and all its data are deleted too.", "warn-text")
    } else {
        ("Once its claim is deleted, the volume and its data are kept; you clean them up yourself.", "muted")
    }
}

/// The keys of a Save line, or nothing when there are none.
pub(crate) fn key_list(keys: &[String]) -> Option<String> {
    (!keys.is_empty()).then(|| keys.join(", "))
}

#[cfg(test)]
#[path = "workload_tests.rs"]
mod tests;
