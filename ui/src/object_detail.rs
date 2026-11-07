//! The Details tab for the Config and Network sections' kinds, built from the generic
//! `ObjectDetails`: the kind's own rows, then its sections and tables, then
//! events. ConfigMaps and Secrets also get a Data section that can be edited
//! and saved; Secret values stay masked until Show, or until editing starts.

use leptos::prelude::*;
use leptos::task::spawn_local;
use subduer_types::{DataEntry, DataSet, InfoRow, InfoSection, InfoTable, ObjectDetails, ObjectLink, SecretValues};

use crate::app::{Store, format_age};
use crate::logic::object_detail::{
    AccessArgs, DetailsArgs, Draft, Pending, RowValue, SecretArgs, UrlArgs, VERBS, access_answer, ago_text, bad_key,
    binary_note, can_ask, can_patch, can_save, checks_access, conditions_summary, data_summary, data_values, diff, dirty,
    dirty_label, draft_field, drafts, edit_label, editable, events_section, fill, fmt_count, has_fill, head_class,
    hosting_note, info_key, keep_result, kind_section_id, label_chip, link_title, no_access, node_summary, pending_text,
    remove_draft, row_title, row_value, save_request, section_empty, size_line, skip_load, sort_headers, sort_table,
    time_cell, time_text, update_draft, value_rows, wants_node_history, FILL_HEADER,
};
use crate::overview::Meter;
use crate::pod_detail::{EventList, HistoryNote, NodeUsageChart, Resource, SecretValue, Section};
use crate::tauri::invoke;
use crate::workload_detail::{ConditionList, badge_class, relative};

/// Re-fetches the selected object's details. Background refreshes pause
/// while its data is being edited, so they don't replace the draft.
pub fn load(store: Store, quiet: bool) {
    if skip_load(quiet, store.editing.get_untracked()) {
        return;
    }
    let Some(kind) = store.kind.get_untracked() else { return };
    let Some(row) = store.selected.get_untracked() else { return };
    if wants_node_history(&kind) {
        crate::pod_detail::load_history(store, quiet, "node_history", None);
    }
    if store.pod_loading.get_untracked() {
        return;
    }
    if !quiet {
        store.object.set(None);
    }
    store.pod_loading.set(true);
    spawn_local(async move {
        let args = DetailsArgs { kind, namespace: row.namespace.clone(), name: row.name.clone() };
        let result = invoke::<_, ObjectDetails>("object_details", &args).await;
        let current = store.selected.with_untracked(|s| s.as_ref().is_some_and(|s| s.id == row.id));
        // Editing may have started while this was in flight.
        let editing = store.editing.get_untracked();
        let have_good = store.object.with_untracked(|o| matches!(o, Some(Ok(_))));
        if keep_result(quiet, current, editing, result.is_err(), have_good) {
            store.object.set(Some(result));
        }
        store.pod_loading.set(false);
    });
}

#[component]
pub fn ObjectDetailView(store: Store) -> impl IntoView {
    view! {
        <div class="fade" class:stale=move || store.pod_loading.get() && store.object.with(Option::is_some)>
            {move || match store.object.get() {
                None => view! { <p class="muted pad">"Loading…"</p> }.into_any(),
                Some(Err(e)) => view! { <p class="warn">{e}</p> }.into_any(),
                Some(Ok(d)) => view! { <ObjectSections store=store d=d /> }.into_any(),
            }}
        </div>
    }
}

#[component]
fn ObjectSections(store: Store, d: ObjectDetails) -> impl IntoView {
    let kind_id = kind_section_id(&d.kind);
    let status = d.status.clone().unwrap_or_default();
    let data = d.data.clone().map(|data| {
        let (namespace, name) = (d.namespace.clone().unwrap_or_default(), d.name.clone());
        view! { <DataSection store=store data=data namespace=namespace name=name /> }
    });
    let sections = d
        .sections
        .clone()
        .into_iter()
        .map(|s| view! { <InfoSectionView store=store s=s /> })
        .collect_view();
    let events = events_section(&d.kind, d.namespace.as_deref(), &d.events).map(|section| {
        let events = d.events.clone();
        view! {
            <Section store=store id=section.id title=section.title.to_owned() summary=section.summary>
                <EventList store=store events=events.clone() empty=section.empty />
            </Section>
        }
    });
    let check = checks_access(&d.kind).then(|| {
        let (ns, name) = (d.namespace.clone().unwrap_or_default(), d.name.clone());
        view! {
            <Section store=store id="check-access" title="Check access".to_owned() summary="ask the API server".to_owned()>
                <CheckAccess account_namespace=ns.clone() account=name.clone() />
            </Section>
        }
    });
    // A node's CPU, memory and pods come first, charted like a pod's.
    let resources = d.node.clone().map(|n| {
        // Hosting with one pod per node (Fargate) is always full by design: say why the count is missing.
        let hosting = n.one_pod_hosting.clone();
        let (pods, capacity) = (n.pods as f64, n.pod_capacity as f64);
        let summary = node_summary(&n);
        view! {
            <Section store=store id="resources" title="Resources".to_owned() summary=summary>
                <NodeUsageChart store=store resource=Resource::Cpu totals=n.cpu />
                <NodeUsageChart store=store resource=Resource::Memory totals=n.memory />
                {hosting.is_none().then(|| view! {
                    <div class="node-pods-meter">
                        <Meter label="Pods" value=pods total=capacity fmt=fmt_count />
                    </div>
                })}
                {hosting.clone().map(|h| view! {
                    <p class="legend small">{hosting_note(&h)}</p>
                })}
                <HistoryNote store=store subject="this node" />
            </Section>
        }
    });
    view! {
        {resources}
        <Section store=store id=kind_id title=d.kind.clone() summary=status>
            <KindInfo store=store d=d.clone() />
        </Section>
        {data}
        {check}
        {sections}
        {events}
    }
}

#[component]
fn KindInfo(store: Store, d: ObjectDetails) -> impl IntoView {
    let created = d.created.clone().unwrap_or_default();
    let created_text = {
        let created = created.clone();
        move || ago_text(&format_age(store.now.get(), &created), &created)
    };
    let rows = d.info.iter().cloned().map(|r| view! { <InfoRowView store=store r=r /> }).collect_view();
    let labels = d
        .labels
        .iter()
        .map(|(k, v)| view! { <span class="chip">{label_chip(k, v)}</span> })
        .collect_view();
    let annotations = d.annotations.clone();
    let conditions = d.conditions.clone();
    let conditions_summary = conditions_summary(&conditions);
    view! {
        <dl class="kv">
            {d.status.clone().map(|s| view! { <dt>"Status"</dt><dd><span class=badge_class(&s)>{s.clone()}</span></dd> })}
            {rows}
            {(!created.is_empty()).then(|| view! { <dt>"Created"</dt><dd>{created_text}</dd> })}
            {(!d.labels.is_empty()).then(|| view! { <dt>"Labels"</dt><dd class="chips">{labels}</dd> })}
        </dl>
        {(!conditions.is_empty()).then(|| view! {
            <Section store=store id="conditions" title="Conditions".to_owned() summary=conditions_summary nested=true>
                <ConditionList store=store conditions=conditions.clone() />
            </Section>
        })}
        {(!annotations.is_empty()).then(|| {
            let count = annotations.len().to_string();
            view! {
                <Section store=store id="annotations" title="Annotations".to_owned() summary=count nested=true>
                    <dl class="kv">
                        {annotations.iter().map(|(k, v)| view! { <dt class="mono">{k.clone()}</dt><dd class="mono annotation">{v.clone()}</dd> }).collect_view()}
                    </dl>
                </Section>
            }
        })}
    }
}

#[component]
fn InfoRowView(store: Store, r: InfoRow) -> impl IntoView {
    let value = match row_value(&r, !js_sys::Date::parse(&r.value).is_nan()) {
        RowValue::Time => {
            let v = r.value.clone();
            view! { <span>{move || time_text(&relative(store.now.get(), &v), &v)}</span> }.into_any()
        }
        RowValue::Block => view! { <pre class="data-value">{r.value.clone()}</pre> }.into_any(),
        RowValue::Url(url) => view! { <OpenUrl store=store url=url /> }.into_any(),
        RowValue::Link(link) => view! { <Link store=store link=link text=r.value.clone() /> }.into_any(),
        RowValue::Plain => view! { <span>{r.value.clone()}</span> }.into_any(),
    };
    view! { <dt>{r.key.clone()}</dt><dd class:mono=r.mono>{value}</dd> }
}

/// An address with a button that opens it in the browser.
#[component]
fn OpenUrl(store: Store, url: String) -> impl IntoView {
    let open = {
        let url = url.clone();
        move |_| {
            let args = UrlArgs { url: url.clone() };
            spawn_local(async move {
                if let Err(e) = invoke::<_, ()>("open_url", &args).await {
                    store.error.set(Some(e));
                }
            });
        }
    };
    view! {
        <span class="url-row">
            <span class="mono">{url.clone()}</span>
            <button class="btn small" title="Open in your browser" on:click=open>"Open"</button>
        </span>
    }
}

/// Opens another object in its own list.
#[component]
fn Link(store: Store, link: ObjectLink, text: String) -> impl IntoView {
    let title = link_title(&link);
    view! {
        <button
            class="link"
            title=title
            on:click=move |_| crate::app::open_object(store, &link.kind, link.namespace.clone(), link.name.clone())
        >{text}</button>
    }
}

#[component]
fn InfoSectionView(store: Store, s: InfoSection) -> impl IntoView {
    let empty = section_empty(&s);
    let (rows, table, note) = (s.rows.clone(), s.table.clone(), s.empty.clone());
    view! {
        <Section store=store id=s.id.clone() title=s.title.clone() summary=s.summary.clone()>
            {(!rows.is_empty()).then(|| view! {
                <dl class="kv">{rows.iter().cloned().map(|r| view! { <InfoRowView store=store r=r /> }).collect_view()}</dl>
            })}
            {table.clone().filter(|t| !t.rows.is_empty()).map(|t| view! { <InfoTableView store=store t=t id=s.id.clone() /> })}
            {empty.then(|| note.clone().map(|n| view! { <p class="muted">{n}</p> }))}
        </Section>
    }
}

#[component]
fn InfoTableView(store: Store, t: InfoTable, id: String) -> impl IntoView {
    let has_fill = has_fill(&t);
    let mono = t.mono.clone();
    let table = sort_table(&id);
    let head = t
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let class = head_class(&t, i);
            view! { <crate::sort::SortTh sorts=store.sorts table=table.clone() header=c.clone() class=class /> }
        })
        .collect_view();
    let fill_head = has_fill.then(|| view! { <crate::sort::SortTh sorts=store.sorts table=table.clone() header=FILL_HEADER /> });
    let headers = sort_headers(&t);
    let rows = move || {
        let names: Vec<&str> = headers.iter().map(String::as_str).collect();
        let by = store.sorts.by(&table, &names);
        let mut rows = t.rows.clone();
        crate::sort::sort(&mut rows, by, |r, i| info_key(&t, r, i));
        rows.into_iter()
        .map(|r| {
            let cells = r
                .cells
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    if time_cell(&t, i, !js_sys::Date::parse(c).is_nan()) {
                        let c = c.clone();
                        let title = c.clone();
                        return view! { <td class="age" title=title>{move || format!("{} ago", format_age(store.now.get(), &c))}</td> }.into_any();
                    }
                    view! { <td class:num=t.numeric.contains(&i) class:mono=mono.contains(&i)>{c.clone()}</td> }.into_any()
                })
                .collect_view();
            let fill = has_fill.then(|| {
                view! {
                    <td class="fill-cell">
                        {fill(&r).map(|f| {
                            view! {
                                <span class="meter small" title=f.title>
                                    <span class="meter-fill" class:warning=f.warning class:critical=f.critical style=f.style></span>
                                </span>
                            }
                        })}
                    </td>
                }
            });
            let link = r.link.clone();
            let title = row_title(&r);
            view! {
                <tr
                    class:clickable=link.is_some()
                    class:warn-row=r.warn
                    title=title
                    on:click=move |_| {
                        if let Some(l) = &link {
                            crate::app::open_object(store, &l.kind, l.namespace.clone(), l.name.clone());
                        }
                    }
                >{cells}{fill}</tr>
            }
        })
        .collect_view()
    };
    view! {
        <table class="resources itable">
            <thead><tr>{head}{fill_head}</tr></thead>
            <tbody>{rows}</tbody>
        </table>
    }
}

// ---- Check access ----

/// "May this ServiceAccount do X?", answered by a SubjectAccessReview.
#[component]
fn CheckAccess(account_namespace: String, account: String) -> impl IntoView {
    let verb = RwSignal::new("get".to_owned());
    let resource = RwSignal::new(String::new());
    let namespace = RwSignal::new(account_namespace.clone());
    let name = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let answer = RwSignal::new(None::<Result<subduer_types::AccessCheck, String>>);
    let ask = move || {
        if !resource.with_untracked(|r| can_ask(r)) {
            return;
        }
        let args = AccessArgs {
            account_namespace: account_namespace.clone(),
            account: account.clone(),
            verb: verb.get_untracked(),
            resource: resource.get_untracked(),
            namespace: namespace.get_untracked(),
            name: name.get_untracked(),
        };
        busy.set(true);
        spawn_local(async move {
            answer.set(Some(invoke::<_, subduer_types::AccessCheck>("check_access", &args).await));
            busy.set(false);
        });
    };
    let ask_click = {
        let ask = ask.clone();
        move |_| ask()
    };
    let field = move |label: &'static str, value: RwSignal<String>, placeholder: &'static str| {
        let ask = ask.clone();
        view! {
            <label class="access-field">
                <span class="muted small">{label}</span>
                <input
                    class="confirm-input mono"
                    placeholder=placeholder
                    prop:value=move || value.get()
                    on:input=move |ev| {
                        value.set(event_target_value(&ev));
                        answer.set(None);
                    }
                    on:keydown=move |ev| if ev.key() == "Enter" { ask() }
                />
            </label>
        }
    };
    view! {
        <div class="access-form">
            <label class="access-field">
                <span class="muted small">"Verb"</span>
                <select prop:value=move || verb.get() on:change=move |ev| { verb.set(event_target_value(&ev)); answer.set(None); }>
                    {VERBS.map(|v| view! { <option value=v>{v}</option> }).collect_view()}
                </select>
            </label>
            {field("Resource", resource, "pods, deployments.apps, pods/exec")}
            {field("Namespace", namespace, "empty: all namespaces")}
            {field("Name", name, "any")}
            <button class="btn small primary" prop:disabled=move || busy.get() || !resource.with(|r| can_ask(r)) on:click=ask_click>
                {move || if busy.get() { "Asking…" } else { "Check" }}
            </button>
        </div>
        {move || answer.get().map(|a| match a {
            Ok(c) => {
                let (class, text, reason) = access_answer(&c);
                view! { <p class="access-answer"><span class=class>{text}</span>" "<span class="muted">{reason}</span></p> }.into_any()
            }
            Err(e) => view! { <p class="warn">{e}</p> }.into_any(),
        })}
        <p class="muted small">"Asks the API server as this account would (RBAC and any other authorizers). Nothing is changed."</p>
    }
}

// ---- Data: view, edit, save ----

#[component]
fn DataSection(store: Store, data: DataSet, namespace: String, name: String) -> impl IntoView {
    let secret = data.secret;
    let kind = store.kind.get_untracked();
    let can_patch = store.cluster.with_untracked(|c| can_patch(kind.as_ref(), c.as_ref()));
    let editable = editable(data.read_only.as_deref(), can_patch);
    // The values the edit started from, and the draft. Both are dropped on
    // Cancel, save or selection change, so Secret values don't linger.
    let original = RwSignal::new(Vec::<(String, Option<String>)>::new());
    let draft = RwSignal::new(Vec::<Draft>::new());
    let next_id = RwSignal::new(0u32);
    let starting = RwSignal::new(false);
    let start_error = RwSignal::new(None::<String>);

    let entries = data.entries.clone();
    let begin = {
        let (entries, namespace, name) = (entries.clone(), namespace.clone(), name.clone());
        move |_| {
            start_error.set(None);
            let entries_now = entries.clone();
            let entries = entries.clone();
            let to_draft = move |values: Vec<(String, Option<String>)>| {
                let drafts = drafts(&entries, &values);
                next_id.set(drafts.len() as u32);
                original.set(values);
                draft.set(drafts);
                store.editing.set(true);
            };
            if secret {
                starting.set(true);
                let args = SecretArgs { namespace: namespace.clone(), name: name.clone() };
                spawn_local(async move {
                    match invoke::<_, SecretValues>("secret_values", &args).await {
                        Ok(values) => to_draft(values),
                        Err(e) => start_error.set(Some(e)),
                    }
                    starting.set(false);
                });
            } else {
                let values = data_values(&entries_now);
                to_draft(values);
            }
        }
    };
    // Cancel asks before throwing away changes, like leaving does.
    let cancel = move || {
        crate::app::guard(store, move || {
            draft.set(Vec::new());
            original.set(Vec::new());
        })
    };
    // Leaving edit mode from outside (a save, another selection) drops the draft too.
    Effect::new(move |_| {
        if !store.editing.get() && !draft.with_untracked(Vec::is_empty) {
            draft.set(Vec::new());
            original.set(Vec::new());
        }
    });
    let pending = move || draft.with(|d| original.with(|o| diff(o, d)));
    // Anything typed counts as unsaved, including a draft that can't be saved yet.
    let what = dirty_label(secret, &name);
    Effect::new(move |_| {
        let dirty = dirty(store.editing.get(), &pending(), &what);
        if store.dirty.with_untracked(|d| *d != dirty) {
            store.dirty.set(dirty);
        }
    });
    on_cleanup(move || store.dirty.set(None));
    let save = {
        let (namespace, name, kind, rv) = (namespace.clone(), name.clone(), kind.clone(), data.resource_version.clone());
        move |_| {
            let Some((kind, save)) = save_request(kind.clone(), pending(), &rv) else { return };
            crate::workload::request_save(store, kind, namespace.clone(), name.clone(), save);
        }
    };
    let editing = move || store.editing.get();
    let actions = {
        let begin = begin.clone();
        let save = save.clone();
        view! {
            <span class="dsection-actions">
                {move || store.dirty.with(Option::is_some).then(|| view! {
                    <span class="chip unsaved" title="Nothing is saved until you click Save…">"Unsaved"</span>
                })}
                {move || if editing() {
                    let save = save.clone();
                    view! {
                        <button class="btn small" on:click=move |_| cancel()>"Cancel"</button>
                        <button
                            class="btn small primary"
                            prop:disabled=move || !can_save(&pending())
                            on:click=save
                        >"Save…"</button>
                    }
                    .into_any()
                } else if editable {
                    let label = edit_label(secret);
                    view! {
                        <button class="btn small" prop:disabled=move || starting.get() on:click=begin.clone()>
                            {move || if starting.get() { "Reading…" } else { label }}
                        </button>
                    }
                    .into_any()
                } else {
                    ().into_any()
                }}
            </span>
        }
        .into_any()
    };
    let count = data.entries.len();
    let summary = data_summary(count);
    let (read_only, warning) = (data.read_only.clone(), data.warning.clone());
    view! {
        <Section store=store id="data" title="Data".to_owned() summary=summary actions=actions>
            {read_only.clone().map(|r| view! { <p class="muted">{r}</p> })}
            {no_access(read_only.as_deref(), can_patch).then(|| view! { <p class="muted">"Your access doesn't allow changing it."</p> })}
            {warning.clone().map(|w| view! { <p class="warn-text">{w}</p> })}
            {move || start_error.get().map(|e| view! { <p class="warn">{e}</p> })}
            {
                let (entries, namespace, name) = (entries.clone(), namespace.clone(), name.clone());
                move || if editing() {
                    view! { <DataEditor draft=draft next_id=next_id pending=Signal::derive(pending) secret=secret /> }.into_any()
                } else {
                    view! { <DataView store=store entries=entries.clone() secret=secret namespace=namespace.clone() name=name.clone() /> }.into_any()
                }
            }
        </Section>
    }
}

#[component]
fn DataView(store: Store, entries: Vec<DataEntry>, secret: bool, namespace: String, name: String) -> impl IntoView {
    if entries.is_empty() {
        return view! { <p class="muted">"No data."</p> }.into_any();
    }
    entries
        .into_iter()
        .map(|e| {
            let value = if e.binary {
                view! { <p class="muted small">"Binary value: not shown."</p> }.into_any()
            } else if secret {
                view! {
                    <div class="data-secret">
                        <SecretValue store=store namespace=namespace.clone() secret=name.clone() key=e.key.clone() bare=true />
                    </div>
                }
                .into_any()
            } else {
                view! { <pre class="data-value">{e.value.clone().unwrap_or_default()}</pre> }.into_any()
            };
            view! {
                <div class="data-entry">
                    <div class="data-key">
                        <span class="mono">{e.key.clone()}</span>
                        <span class="muted small">{size_line(e.size, e.binary)}</span>
                    </div>
                    {value}
                </div>
            }
        })
        .collect_view()
        .into_any()
}

#[component]
fn DataEditor(
    draft: RwSignal<Vec<Draft>>,
    next_id: RwSignal<u32>,
    pending: Signal<Pending>,
    secret: bool,
) -> impl IntoView {
    let field = move |id: u32, pick: fn(&Draft) -> String| move || draft.with(|d| draft_field(d, id, pick));
    let set = move |id: u32, update: Box<dyn Fn(&mut Draft)>| draft.update(|d| update_draft(d, id, update));
    let add = move |_| {
        let id = next_id.get_untracked();
        next_id.set(id + 1);
        draft.update(|d| d.push(Draft::new(id)));
    };
    view! {
        {secret.then(|| view! { <p class="muted small">"Values are shown while editing. Cancel or save to hide them again."</p> })}
        <For
            each=move || draft.get()
            key=|d| d.id
            children=move |d| {
                let id = d.id;
                let remove = move |_| draft.update(|list| remove_draft(list, id));
                let rows = move || value_rows(&field(id, |x| x.value.clone())());
                let bad_key = move || bad_key(&field(id, |x| x.key.clone())());
                view! {
                    <div class="data-entry editing">
                        <div class="data-key">
                            {if d.binary {
                                view! { <span class="mono">{d.key.clone()}</span> }.into_any()
                            } else {
                                view! {
                                    <input
                                        class="confirm-input data-key-input mono"
                                        class:invalid=bad_key
                                        placeholder="key"
                                        prop:value=field(id, |x| x.key.clone())
                                        on:input=move |ev| {
                                            let v = event_target_value(&ev);
                                            set(id, Box::new(move |x| x.key = v.clone()));
                                        }
                                    />
                                }
                                .into_any()
                            }}
                            <button class="btn small" title="Remove this key" on:click=remove>"Remove"</button>
                        </div>
                        {if d.binary {
                            view! { <p class="muted small">{binary_note(d.size)}</p> }.into_any()
                        } else {
                            view! {
                                <textarea
                                    class="data-value-input mono"
                                    rows=rows
                                    prop:value=field(id, |x| x.value.clone())
                                    on:input=move |ev| {
                                        let v = event_target_value(&ev);
                                        set(id, Box::new(move |x| x.value = v.clone()));
                                    }
                                ></textarea>
                            }
                            .into_any()
                        }}
                    </div>
                }
            }
        />
        <div class="data-edit-foot">
            <button class="btn small" on:click=add>"Add key"</button>
            <span class="muted small">
                {move || match pending.with(pending_text) {
                    Err(e) => view! { <span class="warn-text">{e}</span> }.into_any(),
                    Ok(text) => text.into_any(),
                }}
            </span>
        </div>
    }
}
