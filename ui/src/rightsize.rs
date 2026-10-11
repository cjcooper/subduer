//! Applying a review's suggested requests: one change per workload (all its
//! chosen containers, so it rolls out once), previewed with a server dry run,
//! then applied through the safety gate. Production-like contexts need their
//! name typed. Helm-managed workloads warn that the next upgrade resets the
//! requests, and offer the values to copy into the chart.

use std::collections::{BTreeMap, HashSet};

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{ContainerRequests, QuantityChange, ReviewRow, RightsizeChange, RightsizePreview};

use crate::app::Store;
use crate::modal::Modal;
use crate::tauri::invoke;

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

/// Opens the apply dialog for these rows (applicable ones only).
pub(crate) fn ask(store: Store, rows: &[ReviewRow]) {
    let changes = changes_for(rows);
    if !changes.is_empty() {
        store.apply_prompt.set(Some(changes));
    }
}

#[derive(Clone, PartialEq)]
enum Outcome {
    Applied,
    Failed(String),
}

#[component]
pub(crate) fn ApplyDialog(store: Store) -> impl IntoView {
    // One entry per change: its preview (None while loading), and what applying did.
    let previews = RwSignal::new(Vec::<Option<Result<RightsizePreview, String>>>::new());
    let chosen = RwSignal::new(HashSet::<usize>::new());
    let outcomes = RwSignal::new(Vec::<Option<Outcome>>::new());
    let applying = RwSignal::new(false);
    let typed = RwSignal::new(String::new());
    let copied = RwSignal::new(None::<usize>);

    // Preview every change when the dialog opens.
    Effect::new(move |_| {
        let Some(changes) = store.apply_prompt.get() else { return };
        let context = store.context.get_untracked().unwrap_or_default();
        previews.set(vec![None; changes.len()]);
        outcomes.set(vec![None; changes.len()]);
        chosen.set(HashSet::new());
        typed.set(String::new());
        for (i, change) in changes.into_iter().enumerate() {
            let context = context.clone();
            spawn_local(async move {
                #[derive(Serialize)]
                struct Args {
                    context: String,
                    change: RightsizeChange,
                }
                let result = invoke::<_, RightsizePreview>("rightsize_preview", &Args { context, change }).await;
                // Ready ones start chosen.
                if matches!(&result, Ok(p) if p.blocked.is_none() && p.dry_run_error.is_none()) {
                    chosen.update(|c| {
                        c.insert(i);
                    });
                }
                previews.update(|p| {
                    if let Some(slot) = p.get_mut(i) {
                        *slot = Some(result);
                    }
                });
            });
        }
    });

    let close = move || {
        store.apply_prompt.set(None);
        previews.set(vec![]);
        outcomes.set(vec![]);
    };
    let done = move || outcomes.with(|o| !o.is_empty() && o.iter().all(Option::is_some));
    let context = move || store.current_context();
    let ready = move || {
        let n = chosen.with(HashSet::len);
        let ctx = context();
        n > 0 && !applying.get() && ctx.as_ref().is_some_and(|c| c.unlocked && (!c.prod_like || typed.get().trim() == c.name))
    };
    let apply = move |_| {
        let Some(changes) = store.apply_prompt.get_untracked() else { return };
        let context = store.context.get_untracked().unwrap_or_default();
        let picked = chosen.get_untracked();
        applying.set(true);
        spawn_local(async move {
            for (i, change) in changes.into_iter().enumerate() {
                if !picked.contains(&i) {
                    continue;
                }
                #[derive(Serialize)]
                struct Args {
                    context: String,
                    change: RightsizeChange,
                }
                let result = invoke::<_, ()>("rightsize_apply", &Args { context: context.clone(), change }).await;
                outcomes.update(|o| o[i] = Some(result.map_or_else(Outcome::Failed, |()| Outcome::Applied)));
            }
            // Workloads not picked count as done too.
            outcomes.update(|o| {
                for slot in o.iter_mut().filter(|s| s.is_none()) {
                    *slot = Some(Outcome::Failed("Not applied".into()));
                }
            });
            applying.set(false);
        });
    };

    move || {
        // Hidden while the unlock prompt (rendered earlier) is up.
        if store.unlock_prompt.with(Option::is_some) {
            return None;
        }
        let changes = store.apply_prompt.get()?;
        let ctx = context()?;
        let cards = changes
            .iter()
            .enumerate()
            .map(|(i, change)| {
                let title = format!("{} {}/{}", change.kind, change.namespace, change.name);
                let preview = move || previews.with(|p| p.get(i).cloned().flatten());
                let outcome = move || outcomes.with(|o| o.get(i).cloned().flatten());
                let body = move || match preview() {
                    None => view! { <p class="muted small">"Checking with the server…"</p> }.into_any(),
                    Some(Err(e)) => view! { <p class="warn-text">{e}</p> }.into_any(),
                    Some(Ok(p)) => {
                        let rows = p
                            .containers
                            .iter()
                            .map(|c| {
                                if c.injected {
                                    return view! {
                                        <tr><td class="mono">{c.container.clone()}</td><td colspan="2" class="muted">"Added by a webhook; not in the pod template, so left alone."</td></tr>
                                    }
                                    .into_any();
                                }
                                let limits: Vec<String> = [("CPU limit", &c.cpu_limit), ("memory limit", &c.mem_limit)]
                                    .into_iter()
                                    .filter(|(_, q)| q.changed())
                                    .map(|(name, q)| format!("{name} raised to match: {}", change_cell(q)))
                                    .collect();
                                view! {
                                    <tr>
                                        <td class="mono">{c.container.clone()}</td>
                                        <td class="num">{change_cell(&c.cpu_request)}</td>
                                        <td class="num">{change_cell(&c.mem_request)}</td>
                                    </tr>
                                    {(!limits.is_empty()).then(|| view! { <tr><td></td><td colspan="2" class="warn-text small">{limits.join("; ")}</td></tr> })}
                                }
                                .into_any()
                            })
                            .collect_view();
                        let status = match (&p.blocked, &p.dry_run_error) {
                            (Some(b), _) => view! { <p class="muted">{b.clone()}</p> }.into_any(),
                            (None, Some(e)) => view! { <p class="warn-text">"The server refused the dry run: "{e.clone()}</p> }.into_any(),
                            (None, None) => view! { <p class="ok-text small">"✓ Dry run accepted. "<span class="muted">{p.effect.clone()}</span></p> }.into_any(),
                        };
                        let helm = p.helm_release.clone().map(|release| {
                            let snippet = p.helm_snippet.clone().unwrap_or_default();
                            view! {
                                <div class="helm-note">
                                    <p class="warn-text small">
                                        "Managed by Helm release "<strong>{release}</strong>
                                        ": its next upgrade puts the old requests back unless the chart's values change too."
                                    </p>
                                    <button class="btn small" on:click=move |_| {
                                        crate::forward::copy(&snippet);
                                        copied.set(Some(i));
                                    }>{move || if copied.get() == Some(i) { "Copied" } else { "Copy values snippet" }}</button>
                                </div>
                            }
                        });
                        view! {
                            <table class="figures apply-table">
                                <tr><th>"Container"</th><th class="num">"CPU request"</th><th class="num">"Memory request"</th></tr>
                                {rows}
                            </table>
                            {status}
                            {helm}
                        }
                        .into_any()
                    }
                };
                let selectable = move || matches!(preview(), Some(Ok(p)) if p.blocked.is_none() && p.dry_run_error.is_none());
                view! {
                    <div class="apply-card">
                        <label class="check apply-card-head">
                            <input
                                type="checkbox"
                                prop:checked=move || chosen.with(|c| c.contains(&i))
                                prop:disabled=move || !selectable() || applying.get() || done()
                                on:change=move |_| chosen.update(|c| if !c.remove(&i) { c.insert(i); })
                            />
                            <strong>{title}</strong>
                            {move || outcome().map(|o| match o {
                                Outcome::Applied => view! { <span class="ok-text">"✓ Applied"</span> }.into_any(),
                                Outcome::Failed(e) => view! { <span class="warn-text">{format!("✗ {e}")}</span> }.into_any(),
                            })}
                        </label>
                        {body}
                    </div>
                }
            })
            .collect_view();
        let n = move || chosen.with(HashSet::len);
        let locked = !ctx.unlocked;
        let prod = ctx.prod_like;
        let name = ctx.name.clone();
        let unlock = {
            let ctx = ctx.clone();
            move |_| store.unlock_prompt.set(Some(ctx.clone()))
        };
        Some(view! {
            <Modal on_cancel=move |()| close() busy=applying prod=prod wide=true class="apply-modal">
                <div class="modal-head">
                    <h2>"Apply suggested requests"</h2>
                    <button class="close" title="Close (Esc)" aria-label="Close" prop:disabled=move || applying.get() on:click=move |_| close()>"×"</button>
                </div>
                <p>"On "<strong>{name.clone()}</strong>". Each workload gets one change, so its pods roll out once. Limits only change where a new request would be above them."</p>
                <div class="apply-cards">{cards}</div>
                {locked.then(|| view! {
                    <div class="apply-locked">
                        <span class="warn-text">{format!("{name} is read-only.")}</span>
                        <button class="btn small" on:click=unlock>"Allow changes…"</button>
                    </div>
                })}
                {(prod && !locked).then(|| view! {
                    <p>{format!("{name} looks like production. Type its name to apply:")}</p>
                    <input class="confirm-input" prop:value=move || typed.get() on:input=move |ev| typed.set(event_target_value(&ev)) />
                })}
                <div class="modal-actions">
                    {move || if done() {
                        view! { <button class="primary" on:click=move |_| {
                            store.review_checked.set(HashSet::new());
                            close();
                        }>"Done"</button> }.into_any()
                    } else {
                        view! {
                            <button prop:disabled=move || applying.get() on:click=move |_| close()>"Cancel"</button>
                            <button class="danger" prop:disabled=move || !ready() on:click=apply>
                                {move || if applying.get() {
                                    "Applying…".to_owned()
                                } else if n() == 1 {
                                    "Apply to 1 workload".to_owned()
                                } else {
                                    format!("Apply to {} workloads", n())
                                }}
                            </button>
                        }.into_any()
                    }}
                </div>
            </Modal>
        })
    }
}

#[cfg(test)]
#[path = "rightsize_tests.rs"]
mod tests;
