//! The Details tab for the Config and Network sections' kinds, built from the generic
//! `ObjectDetails`: the kind's own rows, then its sections and tables, then
//! events. ConfigMaps and Secrets also get a Data section that can be edited
//! and saved; Secret values stay masked until Show, or until editing starts.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{
    DataChange, DataEntry, DataSet, InfoRow, InfoSection, InfoTable, KindRef, ObjectDetails, ObjectLink, SecretValues,
};

use crate::app::{Store, format_age};
use crate::pod_detail::{EventList, SecretValue, Section};
use crate::tauri::invoke;
use crate::workload::SaveData;
use crate::workload_detail::{ConditionList, badge_class, healthy, relative};

/// Re-fetches the selected object's details. Background refreshes pause
/// while its data is being edited, so they don't replace the draft.
pub fn load(store: Store, quiet: bool) {
    #[derive(Serialize)]
    struct Args {
        kind: KindRef,
        namespace: Option<String>,
        name: String,
    }
    if quiet && store.editing.get_untracked() {
        return;
    }
    let Some(kind) = store.kind.get_untracked() else { return };
    let Some(row) = store.selected.get_untracked() else { return };
    if store.pod_loading.get_untracked() {
        return;
    }
    if !quiet {
        store.object.set(None);
    }
    store.pod_loading.set(true);
    spawn_local(async move {
        let args = Args { kind, namespace: row.namespace.clone(), name: row.name.clone() };
        let result = invoke::<_, ObjectDetails>("object_details", &args).await;
        let current = store.selected.with_untracked(|s| s.as_ref().is_some_and(|s| s.id == row.id));
        // Editing may have started while this was in flight.
        if current && !(quiet && store.editing.get_untracked()) {
            // A failed background refresh keeps the last good render.
            if !(quiet && result.is_err() && store.object.with_untracked(|o| matches!(o, Some(Ok(_))))) {
                store.object.set(Some(result));
            }
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
    let kind_id = d.kind.to_lowercase();
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
    // A namespace's own list is the recent warnings from anything in it.
    let in_namespace = d.kind == "Namespace";
    let events = (d.namespace.is_some() || in_namespace).then(|| {
        let events = d.events.clone();
        let warnings = events.iter().filter(|e| e.type_ == "Warning").count();
        let summary = match (events.len(), warnings) {
            (0, _) => "none".to_owned(),
            (n, 0) => n.to_string(),
            (n, w) => format!("{n} · ⚠ {w}"),
        };
        view! {
            <Section
                store=store
                id=if in_namespace { "ns-warnings" } else { "events" }
                title=if in_namespace { "Recent warnings".to_owned() } else { "Events".to_owned() }
                summary=summary
            >
                <EventList
                    store=store
                    events=events.clone()
                    empty=if in_namespace { "No warnings in it." } else { "No events for it." }
                />
            </Section>
        }
    });
    let check = (d.kind == "ServiceAccount").then(|| {
        let (ns, name) = (d.namespace.clone().unwrap_or_default(), d.name.clone());
        view! {
            <Section store=store id="check-access" title="Check access".to_owned() summary="ask the API server".to_owned()>
                <CheckAccess account_namespace=ns.clone() account=name.clone() />
            </Section>
        }
    });
    view! {
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
        move || format!("{} ago · {}", format_age(store.now.get(), &created), created)
    };
    let rows = d.info.iter().cloned().map(|r| view! { <InfoRowView store=store r=r /> }).collect_view();
    let labels = d
        .labels
        .iter()
        .map(|(k, v)| view! { <span class="chip">{format!("{k}={v}")}</span> })
        .collect_view();
    let annotations = d.annotations.clone();
    let conditions = d.conditions.clone();
    let unhealthy = conditions.iter().filter(|c| !healthy(c)).count();
    let conditions_summary = match (conditions.len(), unhealthy) {
        (n, 0) => n.to_string(),
        (n, u) => format!("{n} · {u} unhealthy"),
    };
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
    let value = if r.time && !js_sys::Date::parse(&r.value).is_nan() {
        let v = r.value.clone();
        view! { <span>{move || format!("{} · {v}", relative(store.now.get(), &v))}</span> }.into_any()
    } else if r.value.contains('\n') {
        view! { <pre class="data-value">{r.value.clone()}</pre> }.into_any()
    } else if let Some(url) = r.url.clone() {
        view! { <OpenUrl store=store url=url /> }.into_any()
    } else if let Some(link) = r.link.clone() {
        view! { <Link store=store link=link text=r.value.clone() /> }.into_any()
    } else {
        view! { <span>{r.value.clone()}</span> }.into_any()
    };
    view! { <dt>{r.key.clone()}</dt><dd class:mono=r.mono>{value}</dd> }
}

/// An address with a button that opens it in the browser.
#[component]
fn OpenUrl(store: Store, url: String) -> impl IntoView {
    #[derive(Serialize)]
    struct Args {
        url: String,
    }
    let open = {
        let url = url.clone();
        move |_| {
            let args = Args { url: url.clone() };
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
    let title = format!("Open {} {}", link.kind.rsplit('/').next().unwrap_or_default(), link.name);
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
    let empty = s.rows.is_empty() && s.table.as_ref().is_none_or(|t| t.rows.is_empty());
    let (rows, table, note) = (s.rows.clone(), s.table.clone(), s.empty.clone());
    view! {
        <Section store=store id=s.id.clone() title=s.title.clone() summary=s.summary.clone()>
            {(!rows.is_empty()).then(|| view! {
                <dl class="kv">{rows.iter().cloned().map(|r| view! { <InfoRowView store=store r=r /> }).collect_view()}</dl>
            })}
            {table.clone().filter(|t| !t.rows.is_empty()).map(|t| view! { <InfoTableView store=store t=t /> })}
            {empty.then(|| note.clone().map(|n| view! { <p class="muted">{n}</p> }))}
        </Section>
    }
}

#[component]
fn InfoTableView(store: Store, t: InfoTable) -> impl IntoView {
    let has_fill = t.rows.iter().any(|r| r.fill.is_some());
    let (numeric, mono) = (t.numeric.clone(), t.mono.clone());
    let head = t
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| view! { <th class:num=numeric.contains(&i)>{c.clone()}</th> })
        .collect_view();
    let rows = t
        .rows
        .into_iter()
        .map(|r| {
            let cells = r
                .cells
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    if t.times.contains(&i) && !js_sys::Date::parse(c).is_nan() {
                        let c = c.clone();
                        let title = c.clone();
                        return view! { <td class="age" title=title>{move || format!("{} ago", format_age(store.now.get(), &c))}</td> }.into_any();
                    }
                    view! { <td class:num=t.numeric.contains(&i) class:mono=mono.contains(&i)>{c.clone()}</td> }.into_any()
                })
                .collect_view();
            let fill = has_fill.then(|| {
                let pct = r.fill.map(|f| (f * 100.0).round());
                view! {
                    <td class="fill-cell">
                        {pct.map(|p| {
                            let (warning, critical) = (r.warn && p < 100.0, p >= 100.0);
                            view! {
                                <span class="meter small" title=format!("{p}% used")>
                                    <span class="meter-fill" class:warning=warning class:critical=critical style=format!("width:{p}%")></span>
                                </span>
                            }
                        })}
                    </td>
                }
            });
            let link = r.link.clone();
            let title = link.as_ref().map(|l| format!("Open {}", l.name));
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
        .collect_view();
    view! {
        <table class="resources itable">
            <thead><tr>{head}{has_fill.then(|| view! { <th></th> })}</tr></thead>
            <tbody>{rows}</tbody>
        </table>
    }
}

// ---- Check access ----

/// "May this ServiceAccount do X?", answered by a SubjectAccessReview.
#[component]
fn CheckAccess(account_namespace: String, account: String) -> impl IntoView {
    #[derive(Serialize)]
    struct Args {
        account_namespace: String,
        account: String,
        verb: String,
        resource: String,
        namespace: String,
        name: String,
    }
    let verb = RwSignal::new("get".to_owned());
    let resource = RwSignal::new(String::new());
    let namespace = RwSignal::new(account_namespace.clone());
    let name = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let answer = RwSignal::new(None::<Result<subduer_types::AccessCheck, String>>);
    let ask = move || {
        if resource.with_untracked(|r| r.trim().is_empty()) {
            return;
        }
        let args = Args {
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
                    spellcheck="false"
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
    let verbs = ["get", "list", "watch", "create", "update", "patch", "delete", "deletecollection", "*"];
    view! {
        <div class="access-form">
            <label class="access-field">
                <span class="muted small">"Verb"</span>
                <select prop:value=move || verb.get() on:change=move |ev| { verb.set(event_target_value(&ev)); answer.set(None); }>
                    {verbs.map(|v| view! { <option value=v>{v}</option> }).collect_view()}
                </select>
            </label>
            {field("Resource", resource, "pods, deployments.apps, pods/exec")}
            {field("Namespace", namespace, "empty: all namespaces")}
            {field("Name", name, "any")}
            <button class="btn small primary" prop:disabled=move || busy.get() || resource.with(|r| r.trim().is_empty()) on:click=ask_click>
                {move || if busy.get() { "Asking…" } else { "Check" }}
            </button>
        </div>
        {move || answer.get().map(|a| match a {
            Ok(c) => {
                let (class, text) = if c.allowed { ("badge badge-ready", "Allowed") } else { ("badge badge-warning", "Denied") };
                let reason = if c.reason.is_empty() && !c.allowed { "No rule allows it.".to_owned() } else { c.reason.clone() };
                view! { <p class="access-answer"><span class=class>{text}</span>" "<span class="muted">{reason}</span></p> }.into_any()
            }
            Err(e) => view! { <p class="warn">{e}</p> }.into_any(),
        })}
        <p class="muted small">"Asks the API server as this account would (RBAC and any other authorizers). Nothing is changed."</p>
    }
}

// ---- Data: view, edit, save ----

/// One entry of the draft while editing.
#[derive(Debug, Clone, PartialEq)]
struct Draft {
    id: u32,
    key: String,
    value: String,
    binary: bool,
    size: usize,
}

/// Key names the API accepts in ConfigMaps and Secrets.
fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 253
        && key != "."
        && key != ".."
        && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// What a save would change (`None`: nothing yet), or why it can't be saved.
type Pending = Result<Option<(Vec<DataChange>, SaveSummary)>, String>;

fn diff(original: &[(String, Option<String>)], draft: &[Draft]) -> Pending {
    let mut seen = std::collections::HashSet::new();
    for d in draft {
        if !valid_key(&d.key) {
            return Err(if d.key.is_empty() {
                "Every key needs a name.".to_owned()
            } else {
                format!("“{}” isn't a valid key: use letters, digits, '-', '_' and '.'.", d.key)
            });
        }
        if !seen.insert(d.key.as_str()) {
            return Err(format!("“{}” is used twice.", d.key));
        }
    }
    let mut changes = Vec::new();
    let mut summary = SaveSummary::default();
    for (key, _) in original {
        if !seen.contains(key.as_str()) {
            changes.push(DataChange { key: key.clone(), value: None });
            summary.removed.push(key.clone());
        }
    }
    for d in draft.iter().filter(|d| !d.binary) {
        match original.iter().find(|(k, _)| *k == d.key) {
            Some((_, Some(old))) if *old == d.value => {}
            Some(_) => {
                changes.push(DataChange { key: d.key.clone(), value: Some(d.value.clone()) });
                summary.changed.push(d.key.clone());
            }
            None => {
                changes.push(DataChange { key: d.key.clone(), value: Some(d.value.clone()) });
                summary.added.push(d.key.clone());
            }
        }
    }
    Ok((!changes.is_empty()).then_some((changes, summary)))
}

#[derive(Debug, Default, Clone, PartialEq)]
struct SaveSummary {
    added: Vec<String>,
    changed: Vec<String>,
    removed: Vec<String>,
}

/// A ConfigMap's text values as the edit's starting point.
fn data_values(entries: &[DataEntry]) -> Vec<(String, Option<String>)> {
    entries.iter().map(|e| (e.key.clone(), e.value.clone())).collect()
}

fn size_text(bytes: usize) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KiB", b as f64 / 1024.0),
        b => format!("{:.1} MiB", b as f64 / 1024.0 / 1024.0),
    }
}

#[component]
fn DataSection(store: Store, data: DataSet, namespace: String, name: String) -> impl IntoView {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        name: String,
    }
    let secret = data.secret;
    let kind = store.kind.get_untracked();
    let can_patch = kind.as_ref().is_some_and(|k| {
        store.cluster.with_untracked(|c| {
            c.iter()
                .flat_map(|c| &c.kinds)
                .any(|i| i.kind == *k && i.verbs.iter().any(|v| v == "patch"))
        })
    });
    let editable = data.read_only.is_none() && can_patch;
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
                let drafts: Vec<Draft> = entries
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
                        let value = values.iter().find(|(k, _)| *k == e.key).and_then(|(_, v)| v.clone());
                        Draft { id: i as u32, key: e.key.clone(), binary: value.is_none(), value: value.unwrap_or_default(), size: e.size }
                    })
                    .collect();
                next_id.set(drafts.len() as u32);
                original.set(values);
                draft.set(drafts);
                store.editing.set(true);
            };
            if secret {
                starting.set(true);
                let args = Args { namespace: namespace.clone(), name: name.clone() };
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
    let what = format!("{} {name}", if secret { "Secret" } else { "ConfigMap" });
    Effect::new(move |_| {
        let dirty = (store.editing.get() && !matches!(pending(), Ok(None))).then(|| what.clone());
        if store.dirty.with_untracked(|d| *d != dirty) {
            store.dirty.set(dirty);
        }
    });
    on_cleanup(move || store.dirty.set(None));
    let save = {
        let (namespace, name, kind, rv) = (namespace.clone(), name.clone(), kind.clone(), data.resource_version.clone());
        move |_| {
            let (Some(kind), Ok(Some((changes, summary)))) = (kind.clone(), pending()) else { return };
            let save = SaveData {
                resource_version: rv.clone(),
                changes,
                added: summary.added,
                changed: summary.changed,
                removed: summary.removed,
            };
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
                            prop:disabled=move || !matches!(pending(), Ok(Some(_)))
                            on:click=save
                        >"Save…"</button>
                    }
                    .into_any()
                } else if editable {
                    let label = if secret { "Edit (shows values)" } else { "Edit" };
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
    let summary = if count == 0 { "empty".to_owned() } else { format!("{count} {}", if count == 1 { "key" } else { "keys" }) };
    let (read_only, warning) = (data.read_only.clone(), data.warning.clone());
    view! {
        <Section store=store id="data" title="Data".to_owned() summary=summary actions=actions>
            {read_only.clone().map(|r| view! { <p class="muted">{r}</p> })}
            {(!can_patch && read_only.is_none()).then(|| view! { <p class="muted">"Your access doesn't allow changing it."</p> })}
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
                        <span class="muted small">{size_text(e.size)}{e.binary.then_some(" · binary")}</span>
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
    let field = move |id: u32, pick: fn(&Draft) -> String| {
        move || draft.with(|d| d.iter().find(|x| x.id == id).map(pick).unwrap_or_default())
    };
    let set = move |id: u32, update: Box<dyn Fn(&mut Draft)>| {
        draft.update(|d| {
            if let Some(x) = d.iter_mut().find(|x| x.id == id) {
                update(x);
            }
        })
    };
    let add = move |_| {
        let id = next_id.get_untracked();
        next_id.set(id + 1);
        draft.update(|d| d.push(Draft { id, key: String::new(), value: String::new(), binary: false, size: 0 }));
    };
    view! {
        {secret.then(|| view! { <p class="muted small">"Values are shown while editing. Cancel or save to hide them again."</p> })}
        <For
            each=move || draft.get()
            key=|d| d.id
            children=move |d| {
                let id = d.id;
                let remove = move |_| draft.update(|list| list.retain(|x| x.id != id));
                let rows = move || {
                    let lines = field(id, |x| x.value.clone())().lines().count();
                    (lines + 1).clamp(2, 16).to_string()
                };
                let bad_key = move || {
                    let k = field(id, |x| x.key.clone())();
                    !k.is_empty() && !valid_key(&k)
                };
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
                                        spellcheck="false"
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
                            view! { <p class="muted small">{format!("Binary value, {}: kept as is.", size_text(d.size))}</p> }.into_any()
                        } else {
                            view! {
                                <textarea
                                    class="data-value-input mono"
                                    spellcheck="false"
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
                {move || match pending.get() {
                    Err(e) => view! { <span class="warn-text">{e}</span> }.into_any(),
                    Ok(None) => "No changes yet.".into_any(),
                    Ok(Some((_, s))) => {
                        let n = s.added.len() + s.changed.len() + s.removed.len();
                        format!("{n} {} to save.", if n == 1 { "change" } else { "changes" }).into_any()
                    }
                }}
            </span>
        </div>
    }
}
