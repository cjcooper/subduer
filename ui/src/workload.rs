//! Changes confirmed in one dialog: Restart and Scale for Deployments and
//! StatefulSets, Run now for CronJobs, Suspend/Resume for Jobs and CronJobs,
//! Edit range for HPAs, and saving a ConfigMap's or Secret's edited data.
//! Gated like Delete, and production-like contexts need the object's name
//! typed out.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{
    ContextInfo, DataChange, KindRef, Replicas, ResourceRow, has_workload_details, is_scalable_workload,
};

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
}

/// Opens the dialog, or the unlock prompt first if the context is read-only.
fn request(store: Store, kind: KindRef, row: &ResourceRow, op: Op) {
    let Some(namespace) = row.namespace.clone() else { return };
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
            .filter(|k| has_workload_details(k) || k.key() == "autoscaling/HorizontalPodAutoscaler")?;
        let allowed = |key: &str, verb: &str| {
            store.cluster.with(|c| {
                c.iter()
                    .flat_map(|c| &c.kinds)
                    .any(|k| k.kind.key() == key && k.verbs.iter().any(|v| v == verb))
            })
        };
        let patch = allowed(&kind.key(), "patch");
        let mut ops = Vec::new();
        if kind.kind == "HorizontalPodAutoscaler" {
            if patch {
                ops.push(Op::Range);
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
                        <button class="btn small" title=title on:click=move |_| request(store, kind.clone(), &row, op)>
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
    let close = move || {
        typed.set(String::new());
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
            let range_now = {
                let name = name.clone();
                move || {
                    let get = |k: &str| object_row(store, &name, k).and_then(|v| v.parse::<i32>().ok());
                    get("Min replicas").zip(get("Max replicas"))
                }
            };
            let ok = {
                let name = name.clone();
                move || {
                    let typed_ok = !prod || typed.get().trim() == name;
                    let scale_ok = match op {
                        Op::Scale => current.get().zip(replicas()).is_some_and(|(c, n)| c.desired != n),
                        Op::Range => range().is_some_and(|r| Some(r) != range_now()),
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
                        };
                        // Run now answers with the new Job's name; the others with nothing.
                        match invoke::<_, Option<String>>(command, &args).await {
                            Ok(_) => {
                                close();
                                if target.op == Op::Save {
                                    store.editing.set(false);
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
                " in namespace "<strong>{namespace.clone()}</strong>
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
                Op::Save => (format!("Save {}?", kind.kind), "Saving…"),
            };
            let label = move || {
                if busy.get() {
                    doing.to_owned()
                } else if op == Op::Scale {
                    replicas().map_or_else(|| "Scale".to_owned(), |n| format!("Scale to {n}"))
                } else if op == Op::Range {
                    range().map_or_else(|| "Save".to_owned(), |(lo, hi)| format!("Set {lo}–{hi}"))
                } else {
                    op.label().to_owned()
                }
            };
            view! {
                <div class="modal-backdrop">
                    <div class="modal" class:prod=context.prod_like>
                        <h2>{title}</h2>
                        {body}
                        {context.prod_like.then(|| view! {
                            <p>"This context looks like "<strong>"PRODUCTION"</strong>"."</p>
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
                            <button on:click=move |_| close()>{if op == Op::Save { "Back to editing" } else { "Cancel" }}</button>
                            <button class="primary" prop:disabled=move || !ok() on:click=confirm.clone()>{label}</button>
                        </div>
                    </div>
                </div>
            }
        })
    }
}
