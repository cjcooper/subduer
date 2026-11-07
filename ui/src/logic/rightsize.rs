//! Applying suggested requests: which rows can be applied, the changes for
//! them (one per workload), and the apply dialog's texts and states.

use std::collections::BTreeMap;

use subduer_types::{ContainerPreview, ContainerRequests, QuantityChange, ReviewRow, RightsizeChange, RightsizePreview};

/// Kinds whose pod template can take new requests.
pub(crate) fn applicable_kind(kind: &str) -> bool {
    matches!(kind, "Deployment" | "StatefulSet" | "DaemonSet" | "CronJob")
}

/// Rows that have a suggestion that can be applied.
pub(crate) fn applicable(row: &ReviewRow) -> bool {
    row.suggestion.is_some() && applicable_kind(&row.key.kind)
}

/// The changes for these rows, one per workload, in the rows' order.
pub(crate) fn changes_for(rows: &[ReviewRow]) -> Vec<RightsizeChange> {
    let mut by_workload: BTreeMap<(usize, String, String, String), Vec<ContainerRequests>> = BTreeMap::new();
    let mut first: BTreeMap<(String, String, String), usize> = BTreeMap::new();
    for (i, row) in rows.iter().filter(|r| applicable(r)).enumerate() {
        let k = (row.key.namespace.clone(), row.key.kind.clone(), row.key.name.clone());
        let order = *first.entry(k.clone()).or_insert(i);
        let s = row.suggestion.as_ref().expect("applicable rows have one");
        by_workload.entry((order, k.0, k.1, k.2)).or_default().push(ContainerRequests {
            container: row.key.container.clone(),
            cpu_request: Some(s.cpu_request),
            mem_request: Some(s.mem_request),
        });
    }
    by_workload
        .into_iter()
        .map(|((_, namespace, kind, name), containers)| RightsizeChange { kind, namespace, name, containers })
        .collect()
}

/// `1500m → 15m`, or just the value when unchanged.
pub(crate) fn change_cell(q: &QuantityChange) -> String {
    let show = |v: &Option<String>| v.clone().unwrap_or_else(|| "not set".into());
    if q.changed() { format!("{} → {}", show(&q.before), show(&q.after)) } else { show(&q.before) }
}

/// What applying one change did.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    Applied,
    Failed(String),
}

impl Outcome {
    pub(crate) fn of(result: Result<(), String>) -> Self {
        result.map_or_else(Outcome::Failed, |()| Outcome::Applied)
    }
}

/// Changes not picked count as done too, once applying ends.
pub(crate) fn finish(outcomes: &mut [Option<Outcome>]) {
    for slot in outcomes.iter_mut().filter(|s| s.is_none()) {
        *slot = Some(Outcome::Failed("Not applied".into()));
    }
}

/// Every change has an outcome.
pub(crate) fn all_done(outcomes: &[Option<Outcome>]) -> bool {
    !outcomes.is_empty() && outcomes.iter().all(Option::is_some)
}

/// A preview the server accepted: it can be chosen (and starts chosen).
pub(crate) fn ready(p: &RightsizePreview) -> bool {
    p.blocked.is_none() && p.dry_run_error.is_none()
}

/// Whether Apply can be pressed: something chosen, not busy, changes
/// allowed, and a production-like context's name typed.
pub(crate) fn can_apply(chosen: usize, applying: bool, unlocked: bool, prod: bool, typed: &str, name: &str) -> bool {
    chosen > 0 && !applying && unlocked && (!prod || typed.trim() == name)
}

/// A change's card heading: `Deployment shop/web`.
pub(crate) fn card_title(change: &RightsizeChange) -> String {
    format!("{} {}/{}", change.kind, change.namespace, change.name)
}

/// Limits raised to make room for a container's new requests.
pub(crate) fn limit_notes(c: &ContainerPreview) -> Option<String> {
    let limits: Vec<String> = [("CPU limit", &c.cpu_limit), ("memory limit", &c.mem_limit)]
        .into_iter()
        .filter(|(_, q)| q.changed())
        .map(|(name, q)| format!("{name} raised to match: {}", change_cell(q)))
        .collect();
    (!limits.is_empty()).then(|| limits.join("; "))
}

pub(crate) fn outcome_text(o: &Outcome) -> String {
    match o {
        Outcome::Applied => "✓ Applied".into(),
        Outcome::Failed(e) => format!("✗ {e}"),
    }
}

/// The Apply button.
pub(crate) fn apply_text(applying: bool, n: usize) -> String {
    if applying {
        "Applying…".to_owned()
    } else if n == 1 {
        "Apply to 1 workload".to_owned()
    } else {
        format!("Apply to {n} workloads")
    }
}

/// A locked context's note.
pub(crate) fn read_only_text(name: &str) -> String {
    format!("{name} is read-only.")
}

/// A production-like context's prompt, to `verb` (apply, capture).
pub(crate) fn prod_text(name: &str, verb: &str) -> String {
    format!("{name} looks like production. Type its name to {verb}:")
}

#[cfg(test)]
#[path = "rightsize_tests.rs"]
mod tests;
