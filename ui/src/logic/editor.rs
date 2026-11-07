//! The YAML edit window: its URL label, the line diff shown before saving,
//! the steps of Save, the discard prompts, and the texts it shows.

use serde::Serialize;
use subduer_types::{EditSession, EditTarget, KindInfo, KindRef, ResourceRow};

use crate::logic::delete::allows;

/// `open_editor` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct OpenArgs {
    pub(crate) target: EditTarget,
}

/// `editor_dirty` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct DirtyArgs {
    pub(crate) dirty: bool,
}

/// `set_unlocked` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct UnlockArgs {
    pub(crate) context: String,
    pub(crate) unlocked: bool,
}

/// `editor_save` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct SaveArgs<'a> {
    pub(crate) yaml: &'a str,
    pub(crate) dry_run: bool,
}

/// What the Edit button opens, when the API allows updating the kind; never
/// for Helm releases (their YAML is a summary, not an object).
pub(crate) fn edit_target(kind: Option<KindRef>, kinds: &[KindInfo], row: &ResourceRow) -> Option<EditTarget> {
    let kind = kind.filter(|k| !(k.group == "helm.sh" && k.kind == "Release"))?;
    allows(kinds, &kind, "update").then(|| EditTarget { kind, namespace: row.namespace.clone(), name: row.name.clone() })
}

/// The `edit` parameter of a URL's query string, when it's a window label.
pub(crate) fn label_in(search: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|kv| kv.strip_prefix("edit="))
        // Only window labels (`edit-3`): other pages may use `edit` for something else.
        .filter(|l| l.strip_prefix("edit-").is_some_and(|id| !id.is_empty()))
        .map(str::to_owned)
}

// ---- Diff ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Line {
    Same(String),
    Added(String),
    Removed(String),
    /// A run of unchanged lines left out of the summary.
    Skipped(usize),
}

/// Line diff of `old` → `new`: common prefix and suffix are trimmed, the rest
/// compared with an LCS (or shown as replaced when it's too big to compare).
pub(crate) fn line_diff(old: &str, new: &str) -> Vec<Line> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (ma, mb) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let mut out: Vec<Line> = a[..prefix].iter().map(|l| Line::Same((*l).to_owned())).collect();
    if ma.len() * mb.len() > 4_000_000 {
        out.extend(ma.iter().map(|l| Line::Removed((*l).to_owned())));
        out.extend(mb.iter().map(|l| Line::Added((*l).to_owned())));
    } else {
        // lcs[i][j]: longest common subsequence of ma[i..] and mb[j..].
        let mut lcs = vec![vec![0u32; mb.len() + 1]; ma.len() + 1];
        for i in (0..ma.len()).rev() {
            for j in (0..mb.len()).rev() {
                lcs[i][j] = if ma[i] == mb[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < ma.len() || j < mb.len() {
            if i < ma.len() && j < mb.len() && ma[i] == mb[j] {
                out.push(Line::Same(ma[i].to_owned()));
                i += 1;
                j += 1;
            } else if i < ma.len() && (j == mb.len() || lcs[i + 1][j] >= lcs[i][j + 1]) {
                // Removals before additions, as diffs usually read.
                out.push(Line::Removed(ma[i].to_owned()));
                i += 1;
            } else {
                out.push(Line::Added(mb[j].to_owned()));
                j += 1;
            }
        }
    }
    out.extend(a[a.len() - suffix..].iter().map(|l| Line::Same((*l).to_owned())));
    out
}

/// Keeps `context` unchanged lines around each change and folds the rest.
pub(crate) fn summarize(lines: Vec<Line>, context: usize) -> Vec<Line> {
    let changed: Vec<usize> =
        lines.iter().enumerate().filter(|(_, l)| !matches!(l, Line::Same(_))).map(|(i, _)| i).collect();
    let near = |i: usize| changed.iter().any(|c| c.abs_diff(i) <= context);
    let mut out = Vec::new();
    let mut skipped = 0;
    for (i, line) in lines.into_iter().enumerate() {
        if matches!(line, Line::Same(_)) && !near(i) {
            skipped += 1;
            continue;
        }
        if skipped > 0 {
            out.push(Line::Skipped(skipped));
            skipped = 0;
        }
        out.push(line);
    }
    if skipped > 0 {
        out.push(Line::Skipped(skipped));
    }
    out
}

/// (added, removed) line counts.
pub(crate) fn counts(lines: &[Line]) -> (usize, usize) {
    lines.iter().fold((0, 0), |(a, r), l| match l {
        Line::Added(_) => (a + 1, r),
        Line::Removed(_) => (a, r + 1),
        _ => (a, r),
    })
}

/// "1 line added, 2 removed."
pub(crate) fn change_summary(added: usize, removed: usize) -> String {
    let lines = |n: usize| if n == 1 { "line" } else { "lines" };
    match (added, removed) {
        (a, 0) => format!("{a} {} added.", lines(a)),
        (0, r) => format!("{r} {} removed.", lines(r)),
        (a, r) => format!("{a} {} added, {r} removed.", lines(a)),
    }
}

/// "… 12 unchanged lines" for a folded run.
pub(crate) fn skipped_text(n: usize) -> String {
    format!("… {n} unchanged lines")
}

/// Keys offered by completion, besides the words already in the document.
pub(crate) const KUBERNETES_KEYS: &[&str] = &[
    "apiVersion", "kind", "metadata", "name", "namespace", "labels", "annotations", "spec", "status", "replicas",
    "selector", "matchLabels", "matchExpressions", "template", "containers", "initContainers", "image",
    "imagePullPolicy", "command", "args", "ports", "containerPort", "protocol", "env", "envFrom", "value",
    "valueFrom", "configMapKeyRef", "secretKeyRef", "configMapRef", "secretRef", "resources", "requests", "limits",
    "cpu", "memory", "volumeMounts", "mountPath", "subPath", "readOnly", "volumes", "configMap", "secret",
    "secretName", "persistentVolumeClaim", "claimName", "emptyDir", "livenessProbe", "readinessProbe",
    "startupProbe", "httpGet", "path", "port", "tcpSocket", "exec", "initialDelaySeconds", "periodSeconds",
    "timeoutSeconds", "failureThreshold", "serviceAccountName", "nodeSelector", "tolerations", "affinity",
    "securityContext", "runAsNonRoot", "runAsUser", "fsGroup", "restartPolicy", "strategy", "rollingUpdate",
    "maxSurge", "maxUnavailable", "type", "targetPort", "data", "stringData", "rules", "host", "http", "paths",
    "pathType", "backend", "service", "number", "tls", "hosts", "schedule", "suspend", "jobTemplate",
    "concurrencyPolicy", "backoffLimit", "accessModes", "storageClassName", "storage", "minReplicas",
    "maxReplicas", "metrics", "target", "averageUtilization",
];

/// Ace's theme for the Appearance setting.
pub(crate) fn ace_theme(dark: bool) -> &'static str {
    if dark { "ace/theme/cloud_editor_dark" } else { "ace/theme/cloud_editor" }
}

// ---- Saving ----

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Modal {
    /// The context is read-only: allow changes first.
    Unlock,
    /// What will change, and the final go-ahead.
    Confirm(Vec<Line>),
    /// Throw the edits away (Cancel, Reload or closing the window).
    Discard(After),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum After {
    Close,
    Reload,
}

/// Step 1 of Save, before asking the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Check {
    /// Busy, or a prompt is open.
    Ignore,
    /// A syntax error: the message, and the row to go to.
    Fix(u32, String),
    NoChanges,
    /// Ask the server to validate (a dry run).
    Validate,
}

pub(crate) fn check(busy: bool, prompting: bool, errors: &[(u32, String)], text: &str, original: &str) -> Check {
    if busy || prompting {
        return Check::Ignore;
    }
    if let Some((row, error)) = errors.first() {
        return Check::Fix(*row, format!("Fix the YAML first: line {}: {error}", row + 1));
    }
    if text == original { Check::NoChanges } else { Check::Validate }
}

/// What will change: the diff with 3 lines of context.
pub(crate) fn confirm(original: &str, text: &str) -> Modal {
    Modal::Confirm(summarize(line_diff(original, text), 3))
}

/// After the server validated: confirm, or allow changes first.
pub(crate) fn after_check(unlocked: bool, original: &str, text: &str) -> Modal {
    if unlocked { confirm(original, text) } else { Modal::Unlock }
}

/// Whether the session allows saving without unlocking.
pub(crate) fn unlocked(session: &Option<Result<EditSession, String>>) -> bool {
    matches!(session, Some(Ok(s)) if s.unlocked)
}

/// Cancel, Reload or closing the window: with unsaved edits, ask first;
/// `None` goes ahead.
pub(crate) fn leave(dirty: bool, after: After) -> Option<Modal> {
    dirty.then_some(Modal::Discard(after))
}

/// The discard prompt's title and button.
pub(crate) fn discard_texts(after: After) -> (&'static str, &'static str) {
    match after {
        After::Close => ("Discard your edits?", "Discard and close"),
        After::Reload => ("Reload and discard your edits?", "Discard and reload"),
    }
}

/// Whether a prompt's go-ahead is enabled: production-like contexts need
/// `expected` typed.
pub(crate) fn can_go(busy: bool, prod: bool, typed: &str, expected: &str) -> bool {
    !busy && (!prod || typed.trim() == expected)
}

pub(crate) fn save_heading(kind: &str) -> String {
    format!("Save {kind}?")
}

pub(crate) fn heading(session: Option<Result<EditSession, String>>) -> String {
    match session {
        None => "Loading…".to_owned(),
        Some(Ok(s)) => s.title,
        Some(Err(_)) => "Edit".to_owned(),
    }
}

/// The status bar's first syntax error: its row and "Line 3: … (+2 more)".
pub(crate) fn first_error(errors: &[(u32, String)]) -> Option<(u32, String)> {
    let (row, text) = errors.first()?;
    let more = if errors.len() > 1 { format!(" (+{} more)", errors.len() - 1) } else { String::new() };
    Some((*row, format!("Line {}: {text}{more}", row + 1)))
}

pub(crate) fn dirty_text(dirty: bool) -> &'static str {
    if dirty { "Edited · not saved" } else { "No changes" }
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
