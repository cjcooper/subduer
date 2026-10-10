//! Restart and Scale for Deployments and StatefulSets: the header buttons and
//! their confirmation dialog. Gated like Delete, and production-like contexts
//! need the object's name typed out.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{ContextInfo, KindRef, Replicas, ResourceRow};

use crate::app::Store;
use crate::tauri::invoke;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Restart,
    Scale,
}

/// What the dialog acts on, captured when it opens.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkloadTarget {
    pub context: ContextInfo,
    pub kind: KindRef,
    pub namespace: String,
    pub name: String,
    pub op: Op,
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
}

/// Opens the dialog, or the unlock prompt first if the context is read-only.
fn request(store: Store, kind: KindRef, row: &ResourceRow, op: Op) {
    let Some(context) = store.current_context_untracked() else { return };
    let Some(namespace) = row.namespace.clone() else { return };
    let target = WorkloadTarget { context: context.clone(), kind, namespace, name: row.name.clone(), op };
    if context.unlocked {
        store.workload_prompt.set(Some(target));
    } else {
        store.pending_workload.set(Some(target));
        store.unlock_prompt.set(Some(context));
    }
}

/// Restart and Scale header buttons; hidden for other kinds and when the API
/// doesn't allow patching this kind.
#[component]
pub fn WorkloadButtons(store: Store, row: ResourceRow) -> impl IntoView {
    let kind = move || {
        let kind = store.kind.get().filter(subduer_types::is_scalable_workload)?;
        let allowed = store.cluster.with(|c| {
            c.iter()
                .flat_map(|c| &c.kinds)
                .any(|k| k.kind == kind && k.verbs.iter().any(|v| v == "patch"))
        });
        allowed.then_some(kind)
    };
    move || {
        kind().map(|kind| {
            let locked = move || store.current_context().is_some_and(|c| !c.unlocked);
            let title = move |label: &'static str| {
                move || {
                    if locked() {
                        format!("{label}… (asks to allow changes first: the context is read-only)")
                    } else {
                        format!("{label}…")
                    }
                }
            };
            let (restart_kind, restart_row) = (kind.clone(), row.clone());
            let scale_row = row.clone();
            view! {
                <button
                    class="btn small"
                    title=title("Rollout restart")
                    on:click=move |_| request(store, restart_kind.clone(), &restart_row, Op::Restart)
                >"Restart"</button>
                <button
                    class="btn small"
                    title=title("Scale")
                    on:click=move |_| request(store, kind.clone(), &scale_row, Op::Scale)
                >"Scale"</button>
            }
        })
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
    let close = move || {
        typed.set(String::new());
        error.set(None);
        busy.set(false);
        current.set(None);
        wanted.set(String::new());
        store.workload_prompt.set(None);
    };

    // The Scale dialog starts from the live replica count.
    Effect::new(move |_| {
        let Some(target) = store.workload_prompt.get() else { return };
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
            let WorkloadTarget { context, kind, namespace, name, op } = target.clone();
            let prod = context.prod_like;
            let replicas = move || wanted.get().trim().parse::<i32>().ok().filter(|n| *n >= 0);
            let ok = {
                let name = name.clone();
                move || {
                    let typed_ok = !prod || typed.get().trim() == name;
                    let scale_ok = match op {
                        Op::Restart => true,
                        Op::Scale => current.get().zip(replicas()).is_some_and(|(c, n)| c.desired != n),
                    };
                    !busy.get() && typed_ok && scale_ok
                }
            };
            let confirm = {
                let target = target.clone();
                move |_| {
                    let target = target.clone();
                    let replicas = replicas();
                    busy.set(true);
                    error.set(None);
                    spawn_local(async move {
                        let (command, replicas) = match target.op {
                            Op::Restart => ("restart_workload", None),
                            Op::Scale => ("scale_workload", replicas),
                        };
                        let args = Change {
                            context: &target.context.name,
                            target: Target { kind: &target.kind, namespace: &target.namespace, name: &target.name },
                            replicas,
                        };
                        match invoke::<_, ()>(command, &args).await {
                            Ok(()) => close(),
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
            let (title, action, doing) = match op {
                Op::Restart => (format!("Restart {}?", kind.kind), "Restart".to_owned(), "Restarting…"),
                Op::Scale => (format!("Scale {}?", kind.kind), String::new(), "Scaling…"),
            };
            let label = move || {
                if busy.get() {
                    doing.to_owned()
                } else if op == Op::Scale {
                    replicas().map_or_else(|| "Scale".to_owned(), |n| format!("Scale to {n}"))
                } else {
                    action.clone()
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
                            <button on:click=move |_| close()>"Cancel"</button>
                            <button class="primary" prop:disabled=move || !ok() on:click=confirm.clone()>{label}</button>
                        </div>
                    </div>
                </div>
            }
        })
    }
}
