//! Changes confirmed in one dialog: Restart and Scale for Deployments and
//! StatefulSets, Run now for CronJobs, Suspend/Resume for Jobs and CronJobs,
//! Edit range for HPAs, ScaledObjects and ScaledJobs, Pause/Resume for
//! ScaledObjects and ScaledJobs, saving a ConfigMap's or Secret's edited
//! data, and Force sync for ExternalSecrets and ClusterExternalSecrets.
//! Gated like Delete, and production-like contexts need the object's name
//! typed out. The logic lives in [`crate::logic::workload`].

use leptos::prelude::*;
use leptos::task::spawn_local;
use subduer_types::{ContextInfo, KindRef, Replicas, ResourceRow, is_keda_scaler};

use crate::app::Store;
use crate::logic::workload::{
    Choices, Form, Prefill, after_success, button_title, can_confirm, cancel_label, change, chose_failed, confirm_class,
    confirm_label, current_revision, force_sync_note, force_sync_text, grows, has_ops, helm_note, helm_release, history,
    key_list, listed, manual_job, needs_name, new_policy, object_row, ops_for, owner_note, parse_count, parse_range,
    pause_detail, pause_hold, pause_note, prefill, provisioned_count, range_floor, range_intro, range_now, range_rule,
    reclaim_note, release_objects, replicas_now, replicas_text, restart_note, revision_label, rollback_note, run_note,
    scale_note, suspend_note, target_args, target_for, title, will_be, zero_note,
};
pub use crate::logic::workload::{Op, SaveData, WorkloadTarget};
use crate::modal::Modal;
use crate::tauri::invoke;

/// Opens the dialog, or the unlock prompt first if the context is read-only.
fn request(store: Store, kind: KindRef, row: &ResourceRow, op: Op) {
    open(store, |context| target_for(context, kind, row, op));
}

/// Asks to save edited data of the selected ConfigMap or Secret.
pub(crate) fn request_save(store: Store, kind: KindRef, namespace: String, name: String, save: SaveData) {
    open(store, |context| WorkloadTarget { context, kind, namespace, name, op: Op::Save, save: Some(save) });
}

fn open(store: Store, target: impl FnOnce(ContextInfo) -> WorkloadTarget) {
    let Some(context) = store.current_context_untracked() else { return };
    let target = target(context.clone());
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
        let kind = store.kind.get().filter(has_ops)?;
        // The loaded details are tracked so buttons appear once they load.
        let ops = store.cluster.with(|c| {
            store.object.with(|o| store.workload.with(|w| ops_for(&kind, &selected, c.as_ref(), o, w)))
        });
        Some((kind, ops))
    };
    move || {
        ops().map(|(kind, ops)| {
            let locked = move || store.current_context().is_some_and(|c| !c.unlocked);
            ops.into_iter()
                .map(|op| {
                    let hint = op.hint(&kind);
                    let title = move || button_title(hint, locked());
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

#[component]
pub fn WorkloadDialog(store: Store) -> impl IntoView {
    let typed = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let current = RwSignal::new(None::<Replicas>);
    let wanted = RwSignal::new(String::new());
    let (min, max) = (RwSignal::new(String::new()), RwSignal::new(String::new()));
    // Pause a ScaledObject at a replica count rather than where it is.
    let hold_on = RwSignal::new(false);
    let hold = RwSignal::new(String::new());
    let size = RwSignal::new(String::new());
    let rollback_to = RwSignal::new(String::new());
    let wait = RwSignal::new(true);
    let keep_history = RwSignal::new(false);
    let close = move || {
        typed.set(String::new());
        keep_history.set(false);
        hold_on.set(false);
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
        match store.object.with_untracked(|o| prefill(&target, o)) {
            Prefill::Range(lo, hi) => {
                min.set(lo);
                max.set(hi);
            }
            Prefill::Hold(n) => hold.set(n),
            Prefill::Size(s) => size.set(s),
            Prefill::Rollback(r) => rollback_to.set(r),
            Prefill::Nothing => {}
            Prefill::FetchReplicas => spawn_local(async move {
                let result = invoke::<_, Replicas>("get_replicas", &target_args(&target)).await;
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
            }),
        }
    });

    move || {
        store.workload_prompt.get().map(|target| {
            let WorkloadTarget { context, kind, namespace, name, op, save } = target.clone();
            let prod = context.prod_like;
            let row = {
                let name = name.clone();
                move |key: &str| store.object.with_untracked(|o| object_row(o, &name, key))
            };
            let replicas = move || wanted.with(|w| parse_count(w));
            let keda = is_keda_scaler(&kind);
            let floor = range_floor(keda);
            let range = move || min.with(|lo| max.with(|hi| parse_range(lo, hi, floor)));
            let held = move || pause_hold(hold_on.get(), &hold.get());
            let current_size = row("Requested").unwrap_or_default();
            let policy = store.object.with_untracked(|o| new_policy(o, &name));
            let form = {
                let (name, current_size) = (name.clone(), current_size.clone());
                move || Form {
                    busy: busy.get(),
                    typed: typed.get(),
                    current: current.get(),
                    replicas: replicas(),
                    range: range(),
                    range_now: store.object.with_untracked(|o| range_now(o, &name, keda)),
                    grows: size.with(|s| grows(s, &current_size)),
                    size: size.get(),
                    rollback_to: rollback_to.get(),
                    held: held(),
                    policy,
                }
            };
            let ok = {
                let (name, form) = (name.clone(), form.clone());
                move || can_confirm(op, prod, &name, &form())
            };
            let confirm = {
                let target = target.clone();
                move |_| {
                    let target = target.clone();
                    let choices = Choices {
                        replicas: replicas(),
                        range: range(),
                        held: held().flatten(),
                        size: size.get_untracked().trim().to_owned(),
                        revision: rollback_to.get_untracked().parse::<i64>().ok(),
                        policy,
                        wait: wait.get_untracked(),
                        keep_history: keep_history.get_untracked(),
                    };
                    busy.set(true);
                    error.set(None);
                    spawn_local(async move {
                        let (command, args) = change(&target, &choices);
                        // Run now answers with the new Job's name, Force sync with a
                        // count; the others with nothing.
                        match invoke::<_, serde_json::Value>(command, &args).await {
                            Ok(_) => {
                                close();
                                let (stop_editing, deselect) = after_success(target.op);
                                if stop_editing {
                                    store.editing.set(false);
                                }
                                if deselect {
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
                    <p class="muted">"It's named "<span class="mono">{manual_job(&name)}</span>" and listed with its other Jobs."</p>
                    {store.workload.with_untracked(|w| run_note(w, &name)).map(|n| view! { <p class="warn-text">{n}</p> })}
                }
                .into_any(),
                Op::Suspend | Op::Resume => view! {
                    <p>{what}{will_be(op)}</p>
                    <p>{suspend_note(&kind, op == Op::Suspend)}</p>
                }
                .into_any(),
                Op::Range => {
                    let owner = row("Controlled by");
                    let input = |label: &'static str, value: RwSignal<String>| view! {
                        <label class="replicas">
                            {label}
                            <input
                                class="confirm-input replicas-input"
                                type="number"
                                min=floor
                                prop:value=move || value.get()
                                on:input=move |ev| value.set(event_target_value(&ev))
                            />
                        </label>
                    };
                    let jobs = kind.kind == "ScaledJob";
                    let paused = row("Paused").is_some();
                    let helm = if keda { store.object.with_untracked(|o| helm_release(o, &name)) } else { None };
                    let rule = range_rule(keda);
                    view! {
                        <p>{range_intro(jobs)}{what}"."</p>
                        {input("Min ", min)}
                        {input("Max ", max)}
                        {move || range().is_none().then(|| view! { <p class="warn-text">{rule}</p> })}
                        {move || zero_note(keda, jobs, range()).then(|| view! {
                            <p class="muted">"With min 0, KEDA scales it to zero when no trigger is active, once the cooldown has passed."</p>
                        })}
                        {(keda && paused).then(|| view! { <p class="warn-text">"It's paused: the new range applies once it's resumed."</p> })}
                        {helm.map(|r| view! { <p class="warn-text">{helm_note(&r)}</p> })}
                        {owner.map(|o| view! { <p class="warn-text">{owner_note(&o)}</p> })}
                    }
                    .into_any()
                }
                Op::Pause | Op::Unpause => {
                    let pause = op == Op::Pause;
                    let object = kind.kind == "ScaledObject";
                    let now = store.object.with_untracked(|o| replicas_now(o, &name));
                    view! {
                        <p>{what}{will_be(op)}</p>
                        <p>{pause_note(&kind, pause)}</p>
                        {(pause && object).then(|| view! {
                            <label class="check-row">
                                <input type="checkbox" prop:checked=move || hold_on.get() on:change=move |ev| hold_on.set(event_target_checked(&ev)) />
                                " Scale it to a replica count first"
                            </label>
                            {move || hold_on.get().then(|| view! {
                                <label class="replicas">
                                    "Replicas "
                                    <input
                                        class="confirm-input replicas-input"
                                        type="number"
                                        min="0"
                                        prop:value=move || hold.get()
                                        on:input=move |ev| hold.set(event_target_value(&ev))
                                    />
                                </label>
                                {move || (held() == Some(None)).then(|| view! { <p class="warn-text">"Enter 0 or more replicas."</p> })}
                            })}
                            <p class="muted">{move || pause_detail(held().flatten(), now)}</p>
                        })}
                        {(pause && !object).then(|| view! { <p class="muted">"Sets autoscaling.keda.sh/paused."</p> })}
                        {(!pause).then(|| view! { <p class="muted">"Removes the autoscaling.keda.sh/paused and paused-replicas annotations."</p> })}
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
                        let current_size = current_size.clone();
                        move || (!size.with(|s| grows(s, &current_size))).then(|| view! { <p class="warn-text">"Enter a size bigger than now, like 20Gi."</p> })
                    }
                    <p class="muted">"Claims can't shrink again. The filesystem grows when the driver resizes it, which for some drivers needs the pod restarted."</p>
                }
                .into_any(),
                Op::Uninstall => {
                    let objects = store.object.with_untracked(|o| release_objects(o, &name));
                    let (shown, more) = listed(objects.len());
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
                    let revisions = store.object.with_untracked(|o| history(o, &name));
                    let options = revisions
                        .iter()
                        .skip(1)
                        .map(|r| view! { <option value=r.0.to_string()>{revision_label(r)}</option> })
                        .collect_view();
                    let note = rollback_note(&revisions);
                    let now = revisions.first().map(current_revision);
                    view! {
                        <p>"Roll "{what}" back."</p>
                        {now.map(|n| view! { <p class="muted">{n}</p> })}
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
                        {move || rollback_to.with(|r| chose_failed(&revisions, r)).then(|| view! {
                            <p class="warn-text">"That revision failed when it was deployed."</p>
                        })}
                        <p class="muted">{note}</p>
                        <p class="muted small">"Runs helm rollback with your kubeconfig and this context."</p>
                    }
                    .into_any()
                }
                Op::ForceSync => {
                    let cluster = kind.kind == "ClusterExternalSecret";
                    let children = if cluster { store.object.with_untracked(|o| provisioned_count(o, &name)) } else { 0 };
                    view! {
                        <p>
                            {what}" gets the annotation "<span class="mono">"force-sync"</span>
                            {force_sync_text(cluster, children)}
                        </p>
                        {store.object.with_untracked(|o| force_sync_note(o, &name)).map(|n| view! { <p class="warn-text">{n}</p> })}
                        <p class="muted">"Pods that read the Secret as environment variables keep the old values until they restart; mounted files update within about a minute."</p>
                    }
                    .into_any()
                }
                Op::Reclaim => {
                    let (note, class) = reclaim_note(policy);
                    view! {
                        <p>"Change the reclaim policy of "{what}" to "<strong>{policy}</strong>"?"</p>
                        <p class=class>{note}</p>
                    }
                    .into_any()
                }
                Op::Save => {
                    let save = save.clone().unwrap_or_default();
                    let line = |label: &'static str, keys: &[String]| {
                        key_list(keys).map(|keys| view! { <li>{label}" "<span class="mono">{keys}</span></li> })
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
                                <p class="muted">{replicas_text(r)}</p>
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
            let (title, doing) = title(op, &kind);
            let label = move || confirm_label(op, doing, &form());
            view! {
                <Modal on_cancel=move |()| close() busy=busy prod=context.prod_like wide=op == Op::Rollback>
                        <h2>{title}</h2>
                        {body}
                        {context.prod_like.then(|| view! {
                            <p>"This context looks like "<strong>"PRODUCTION"</strong>"."</p>
                        })}
                        {needs_name(context.prod_like, op).then(|| view! {
                            <p>"Type the name to confirm:"</p>
                            <input
                                class="confirm-input"
                                prop:value=move || typed.get()
                                on:input=move |ev| typed.set(event_target_value(&ev))
                            />
                        })}
                        {move || error.get().map(|e| view! { <p class="warn">{e}</p> })}
                        <div class="modal-actions">
                            <button prop:disabled=move || busy.get() on:click=move |_| close()>{cancel_label(op)}</button>
                            <button
                                class=confirm_class(op)
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
