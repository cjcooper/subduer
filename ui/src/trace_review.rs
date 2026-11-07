//! The Traces page: recent trace files and Open trace…, then a file's review:
//! the right-sizing table (one row per workload container, sortable and
//! filterable) and, for the selected row, its suggestion, flags, CPU and
//! memory over the trace, and usage by day and hour. Works without a cluster.
//!
//! The logic is in `logic::trace_review`.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{ReviewRow, TraceDaily, TraceKey, TraceReview, TraceSeries};

use crate::app::{Store, guard};
use crate::logic::trace::{recent, scope_text, span_text};
use crate::logic::trace_review::{
    CHART_POINTS, Filter, OverviewArgs, SeriesArgs, SortBy, add_note_text, all_checked, apply_note, apply_selected_text, apply_selected_title,
    applicable_keys, basis_text, check_title, checked_rows, compare_pair, compare_text, detail_subtitle, facts_text, file_name, find_row, flag_class,
    flag_explain, flag_level, flag_text, gaps_text, keeps_selection, key_subtitle, more_text, namespace_choice, namespaces, next_sort, note_dirty, note_what,
    overview_title, pods_text, quick_active, quick_zoom, recent_text, recording, recording_text, report_error_text, rows_text, save_note, shown_flags,
    sort_arrow, source_text, spare, state_text, suggest_line, suggestion_text, toggle, toggle_all, toggle_ordered, visible_rows,
};
use crate::tauri::{NoArgs, invoke};
use crate::trace_chart::{Measure, TraceChart, TraceHeat};

/// Shows the Traces page.
pub(crate) fn open_page(store: Store) {
    guard(store, move || {
        store.selected.set(None);
        store.forwards_open.set(false);
        store.traces_open.set(true);
    });
}

/// Shows the Traces page with `path` under review.
pub(crate) fn open_file(store: Store, path: String) {
    guard(store, move || {
        store.selected.set(None);
        store.forwards_open.set(false);
        store.traces_open.set(true);
        store.review_sel.set(None);
        store.review_checked.set(Default::default());
        store.compare_paths.set(None);
        store.report_saved.set(None);
        store.review_path.set(Some(path));
        load(store);
    });
}

/// (Re)reads the file under review.
fn load(store: Store) {
    let Some(path) = store.review_path.get_untracked() else { return };
    store.review.set(None);
    spawn_local(async move {
        #[derive(Serialize)]
        struct Args {
            path: String,
        }
        let result = invoke::<_, TraceReview>("trace_review", &Args { path: path.clone() }).await;
        if store.review_path.get_untracked().as_deref() != Some(path.as_str()) {
            return;
        }
        if let Ok(r) = &result {
            store.recent_traces.set(crate::prefs::remember_trace(recent(path, &r.summary, js_sys::Date::now())));
            // Keep the selection if the row is still there.
            if let Some(sel) = store.review_sel.get_untracked()
                && !keeps_selection(r, &sel)
            {
                store.review_sel.set(None);
            }
        }
        store.review.set(Some(result));
    });
}

fn choose_and_open(store: Store) {
    spawn_local(async move {
        match invoke::<_, Option<String>>("choose_trace_to_open", &NoArgs {}).await {
            Ok(Some(path)) => open_file(store, path),
            Ok(None) => {}
            Err(e) => store.error.set(Some(e)),
        }
    });
}

/// Whether suggestions from this review can be applied now: only while
/// connected to the cluster it was recorded on. The reason when not.
pub(crate) fn apply_blocked(store: Store) -> Option<String> {
    let connected = store.cluster.with(Option::is_some) && store.disconnected.with(Option::is_none);
    let context = store.context.get();
    store.review.with(|r| crate::logic::trace_review::apply_blocked(r.as_ref(), connected, context.as_deref()))
}

/// Spare capacity: blank without a request, "short" (in the danger ink) when usage is above it.
fn spare_cell(v: Option<f64>, measure: Measure) -> impl IntoView {
    match spare(v, measure) {
        None => view! { <td class="num muted">"—"</td> }.into_any(),
        Some((true, text)) => view! { <td class="num short" title="Usage above the request">{text}</td> }.into_any(),
        Some((false, text)) => view! { <td class="num">{text}</td> }.into_any(),
    }
}

/// Apply… for one row, or why it can't be applied.
fn apply_row(store: Store, row: &ReviewRow) -> impl IntoView + use<> {
    let note = apply_note(&row.key.kind, apply_blocked(store));
    let row = row.clone();
    match note {
        Some(n) => view! { <p class="muted small apply-note">{n}</p> }.into_any(),
        None => view! {
            <div class="apply-row">
                <button class="btn small primary" on:click=move |_| crate::rightsize::ask(store, std::slice::from_ref(&row))>"Apply…"</button>
                <span class="muted small">"Previews with the server first."</span>
            </div>
        }
        .into_any(),
    }
}

/// Picks another trace and compares it with the one under review.
fn compare_with(store: Store) {
    let Some(current) = store.review_path.get_untracked() else { return };
    spawn_local(async move {
        match invoke::<_, Option<String>>("choose_trace_to_open", &NoArgs {}).await {
            Ok(Some(other)) => match compare_pair(current, other) {
                Ok(paths) => crate::compare::open(store, paths),
                Err(e) => store.error.set(Some(e.into())),
            },
            Ok(None) => {}
            Err(e) => store.error.set(Some(e)),
        }
    });
}

/// Saves the review as an HTML page where the user chooses.
fn export_report(store: Store) {
    let Some(path) = store.review_path.get_untracked() else { return };
    spawn_local(async move {
        #[derive(Serialize)]
        struct Choose {
            trace: String,
        }
        #[derive(Serialize)]
        struct Export {
            path: String,
            out: String,
        }
        let out = match invoke::<_, Option<String>>("choose_report_file", &Choose { trace: path.clone() }).await {
            Ok(Some(out)) => out,
            Ok(None) => return,
            Err(e) => {
                store.report_saved.set(Some(Err(e)));
                return;
            }
        };
        store.report_saved.set(None);
        let result = invoke::<_, String>("export_trace_report", &Export { path, out }).await;
        store.report_saved.set(Some(result));
    });
}

/// Shows a file in Finder (or the system's file manager).
fn reveal(store: Store, path: String) {
    #[derive(Serialize)]
    struct Args {
        path: String,
    }
    spawn_local(async move {
        if let Err(e) = invoke::<_, ()>("reveal_trace", &Args { path }).await {
            store.error.set(Some(e));
        }
    });
}

/// "Report saved to …" with Show in Finder, after Export report….
fn report_line(store: Store) -> impl IntoView {
    move || {
        store.report_saved.get().map(|r| match r {
            Ok(out) => {
                let reveal_path = out.clone();
                view! {
                    <div class="report-line">
                        <span class="ok-text">"✓ Report saved: "</span>
                        <span class="mono">{file_name(&out).to_owned()}</span>
                        <button class="btn small" on:click=move |_| reveal(store, reveal_path.clone())>{crate::prefs::reveal_label()}</button>
                        <button class="close" title="Dismiss" aria-label="Dismiss" on:click=move |_| store.report_saved.set(None)>"×"</button>
                    </div>
                }
                .into_any()
            }
            Err(e) => view! { <div class="report-line warn-text">{report_error_text(&e)}</div> }.into_any(),
        })
    }
}

/// A note shown as text with Edit, or being edited. Saved into the trace
/// file, so it travels with it. Unsaved text holds leaving like other edits.
#[component]
fn NoteBox(store: Store, key: Option<TraceKey>, text: Option<String>, label: &'static str) -> impl IntoView {
    let editing = RwSignal::new(false);
    let draft = RwSignal::new(text.clone().unwrap_or_default());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let original = StoredValue::new(text.clone().unwrap_or_default());
    let what = note_what(key.as_ref());
    let stop = move || {
        editing.set(false);
        store.dirty.set(None);
    };
    let save = {
        let key = key.clone();
        move |_| {
            let Some(path) = store.review_path.get_untracked() else { return };
            let (key, text) = (key.clone(), draft.get_untracked());
            busy.set(true);
            spawn_local(async move {
                #[derive(Serialize)]
                struct Args {
                    path: String,
                    key: Option<TraceKey>,
                    text: String,
                }
                match invoke::<_, ()>("trace_set_note", &Args { path, key: key.clone(), text: text.clone() }).await {
                    Ok(()) => {
                        original.set_value(text.trim().to_owned());
                        stop();
                        store.review.update(|r| {
                            if let Some(Ok(r)) = r {
                                save_note(r, key.as_ref(), &text);
                            }
                        });
                    }
                    Err(e) => error.set(Some(e)),
                }
                busy.set(false);
            });
        }
    };
    move || {
        if editing.get() {
            let what = what.clone();
            let save = save.clone();
            view! {
                <div class="note-box editing">
                    <label class="note-label">{label}</label>
                    <textarea
                        rows="3"
                        prop:value=move || draft.get()
                        on:input=move |ev| {
                            draft.set(event_target_value(&ev));
                            store.dirty.set(note_dirty(&draft.get_untracked(), &original.get_value(), &what));
                        }
                    ></textarea>
                    {move || error.get().map(|e| view! { <p class="warn-text small">{e}</p> })}
                    <div class="note-actions">
                        <button class="btn small" prop:disabled=move || busy.get() on:click=move |_| {
                            draft.set(original.get_value());
                            error.set(None);
                            stop();
                        }>"Cancel"</button>
                        <button class="btn small primary" prop:disabled=move || busy.get() on:click=save>
                            {move || if busy.get() { "Saving…" } else { "Save note" }}
                        </button>
                    </div>
                </div>
            }
            .into_any()
        } else {
            let current = original.get_value();
            if current.is_empty() {
                view! { <button class="btn small note-add" on:click=move |_| editing.set(true)>{add_note_text(label)}</button> }.into_any()
            } else {
                view! {
                    <div class="note-box">
                        <span class="note-label">{label}</span>
                        <p class="note-text">{current}</p>
                        <button class="btn small" on:click=move |_| editing.set(true)>"Edit"</button>
                    </div>
                }
                .into_any()
            }
        }
    }
}

// --- The page --------------------------------------------------------------------

#[component]
pub(crate) fn TracesPage(store: Store) -> impl IntoView {
    move || {
        if store.compare_paths.with(Option::is_some) {
            view! { <crate::compare::CompareView store=store /> }.into_any()
        } else if store.review_path.with(Option::is_some) {
            view! { <ReviewView store=store /> }.into_any()
        } else {
            view! { <TraceList store=store /> }.into_any()
        }
    }
}

#[component]
fn TraceList(store: Store) -> impl IntoView {
    let picked = RwSignal::new(std::collections::BTreeSet::<String>::new());
    let running = move || {
        store.trace.get().map(|s| {
            let path = s.path.clone();
            view! {
                <div class="trace-card running">
                    <span class="rec-dot" aria-hidden="true"></span>
                    <div class="trace-card-text">
                        <strong>{file_name(&s.path).to_owned()}</strong>
                        <span class="muted small">{recording_text(&s)}</span>
                    </div>
                    <button class="btn small" on:click=move |_| open_file(store, path.clone())>"Review so far"</button>
                </div>
            }
        })
    };
    let recent = move || {
        let list = store.recent_traces.get();
        if list.is_empty() {
            return view! { <p class="muted">"No traces yet. Start one with Trace… in the top bar, or open a file someone shared."</p> }.into_any();
        }
        list.into_iter()
            .map(|t| {
                let (path, forget_path, pick_path) = (t.path.clone(), t.path.clone(), t.path.clone());
                let is_picked = { let p = t.path.clone(); move || picked.with(|s| s.contains(&p)) };
                let when = js_sys::Date::new(&t.at.into()).to_locale_string("default", &wasm_bindgen::JsValue::UNDEFINED).as_string().unwrap_or_default();
                view! {
                    <div class="trace-card">
                        <input
                            type="checkbox"
                            title="Choose for Compare"
                            aria-label="Choose for Compare"
                            prop:checked=is_picked
                            on:change=move |_| picked.update(|s| toggle_ordered(s, &pick_path))
                        />
                        <button class="trace-card-open" title=t.path.clone() on:click=move |_| open_file(store, path.clone())>
                            <strong>{file_name(&t.path).to_owned()}</strong>
                            <span class="muted small">{recent_text(&t, &when)}</span>
                        </button>
                        <button class="close" title="Remove from this list (the file stays)" aria-label="Remove from the list"
                            on:click=move |_| store.recent_traces.set(crate::prefs::forget_trace(&forget_path))>"×"</button>
                    </div>
                }
            })
            .collect_view()
            .into_any()
    };
    view! {
        <div class="traces-page">
            <div class="resource-header page-head">
                <h1>"Traces"</h1>
                <span class="spacer"></span>
                {move || {
                    let n = picked.with(|s| s.len());
                    (n >= 2).then(|| view! {
                        <button class="btn" on:click=move |_| crate::compare::open(store, picked.get_untracked().into_iter().collect())>
                            {compare_text(n)}
                        </button>
                    })
                }}
                <button class="btn primary" on:click=move |_| choose_and_open(store)>"Open trace…"</button>
            </div>
            <p class="muted">"Review CPU and memory recorded over hours or days, against requests, to right-size workloads. Opening a trace doesn't need a cluster."</p>
            {running}
            <h2 class="section-title">"Recent"</h2>
            <p class="muted small">"Tick two or more to compare them, e.g. before and after a resize."</p>
            <div class="trace-cards">{recent}</div>
        </div>
    }
}

#[component]
fn ReviewView(store: Store) -> impl IntoView {
    let sort = RwSignal::new((SortBy::CpuSpare, true));
    let filter = RwSignal::new(Filter::default());
    let back = move |_| {
        guard(store, move || {
            store.review_path.set(None);
            store.review.set(None);
            store.review_sel.set(None);
            store.report_saved.set(None);
        })
    };
    let reveal_file = move |_| {
        if let Some(path) = store.review_path.get_untracked() {
            reveal(store, path);
        }
    };
    let recording = move || store.review.with(|r| recording(r.as_ref()));

    let header = move || {
        let r = store.review.get()?.ok()?;
        let s = &r.summary;
        let gaps = s.gaps.len();
        Some(view! {
            <dl class="review-facts">
                <div><dt>"Context"</dt><dd>{s.context.clone()}</dd></div>
                <div><dt>"Traced"</dt><dd>{scope_text(&s.scope)}</dd></div>
                <div><dt>"Covers"</dt><dd>{span_text(s.data_start, s.data_end)}</dd></div>
                <div><dt>"State"</dt><dd class:warn-text=s.unfinished>{state_text(s)}</dd></div>
                <div><dt>"Data from"</dt><dd>{source_text(s)}</dd></div>
                <div><dt>"Gaps"</dt><dd class:warn-text={gaps > 0}>{gaps_text(gaps)}</dd></div>
            </dl>
        })
    };

    let namespaces = move || store.review.with(|r| namespaces(r.as_ref()));
    let rows = move || store.review.with(|r| visible_rows(r.as_ref(), &filter.get(), sort.get()));
    let th = move |label: &'static str, by: SortBy, num: bool| {
        let active = move || sort.get().0 == by;
        let arrow = move || sort_arrow(sort.get(), by);
        view! {
            <th class:num=num>
                <button class="th-sort" class:on=active on:click=move |_| sort.update(|s| *s = next_sort(*s, by))>{label}{arrow}</button>
            </th>
        }
    };
    let table = move || {
        let list = rows();
        if store.review.with(|r| matches!(r, Some(Ok(_)))) && list.is_empty() {
            return view! { <p class="muted pad">"No rows match."</p> }.into_any();
        }
        let body = list
            .into_iter()
            .map(|row| {
                let key = row.key.clone();
                let selected = { let key = key.clone(); move || store.review_sel.with(|s| s.as_ref() == Some(&key)) };
                // The two most severe; the detail panel lists them all.
                let (shown, more) = shown_flags(&row);
                let flags = shown
                    .into_iter()
                    .map(|f| view! { <span class=flag_class(f) title=flag_explain(f)><span class="flag-dot" aria-hidden="true"></span>{flag_text(f)}</span> })
                    .collect_view();
                let more = more_text(more).map(|more| view! { <span class="flag more" title="Select the row to see them all">{more}</span> });
                let suggestion = suggestion_text(&row);
                let can = crate::rightsize::applicable(&row);
                let check_key = row.key.clone();
                let checked = { let k = row.key.clone(); move || store.review_checked.with(|c| c.contains(&k)) };
                view! {
                    <tr class:selected=selected on:click=move |_| store.review_sel.set(Some(key.clone()))>
                        <td class="pick" on:click=|ev| ev.stop_propagation()>
                            <input
                                type="checkbox"
                                aria-label="Choose for Apply selected"
                                title=check_title(can)
                                prop:disabled=!can
                                prop:checked=checked
                                on:change=move |_| store.review_checked.update(|c| toggle(c, &check_key))
                            />
                        </td>
                        <td>
                            <div>
                                <span class="review-name">{row.key.name.clone()}</span>
                                {row.note.clone().map(|n| view! { <span class="note-mark" title=n aria-label="Has a note">"✎"</span> })}
                            </div>
                            <div class="muted small">{key_subtitle(&row.key)}</div>
                        </td>
                        <td class="num">{pods_text(&row)}</td>
                        <td class="num">{Measure::Cpu.opt(row.cpu_request)}</td>
                        <td class="num">{Measure::Cpu.fmt(row.cpu_p95)}</td>
                        {spare_cell(row.cpu_spare, Measure::Cpu)}
                        <td class="num">{Measure::Memory.opt(row.mem_request)}</td>
                        <td class="num">{Measure::Memory.fmt(row.mem_max)}</td>
                        {spare_cell(row.mem_spare, Measure::Memory)}
                        <td class="num">{suggestion}</td>
                        <td class="flags">{flags}{more}</td>
                    </tr>
                }
            })
            .collect_view();
        view! {
            <table class="resources review-table">
                <thead>
                    <tr>
                        <th class="pick">
                            <input
                                type="checkbox"
                                aria-label="Choose every row shown"
                                title="Choose every row shown that has a suggestion"
                                prop:checked=move || {
                                    let shown = applicable_keys(rows());
                                    store.review_checked.with(|c| all_checked(&shown, c))
                                }
                                on:change=move |_| {
                                    let shown = applicable_keys(rows());
                                    store.review_checked.update(|c| toggle_all(c, shown));
                                }
                            />
                        </th>
                        {th("Workload", SortBy::Workload, false)}
                        {th("Pods", SortBy::Pods, true)}
                        {th("CPU request", SortBy::CpuRequest, true)}
                        {th("CPU p95", SortBy::CpuP95, true)}
                        {th("CPU spare", SortBy::CpuSpare, true)}
                        {th("Memory request", SortBy::MemRequest, true)}
                        {th("Memory peak", SortBy::MemPeak, true)}
                        {th("Memory spare", SortBy::MemSpare, true)}
                        {th("Suggested", SortBy::Suggested, true)}
                        {th("Flags", SortBy::Flags, false)}
                    </tr>
                </thead>
                <tbody>{body}</tbody>
            </table>
        }
        .into_any()
    };

    view! {
        <div class="traces-page review">
            <div class="resource-header page-head">
                <button class="btn small" on:click=back title="Back to the list of traces">"← Traces"</button>
                <h1 class="review-title" title=move || store.review_path.get().unwrap_or_default()>
                    {move || store.review_path.get().map(|p| file_name(&p).to_owned())}
                </h1>
                {move || recording().then(|| view! { <span class="rec-badge"><span class="rec-dot" aria-hidden="true"></span>"Recording"</span> })}
                <span class="spacer"></span>
                <button class="btn small" on:click=move |_| load(store) title="Read the file again">"Reload"</button>
                <button class="btn small" on:click=move |_| compare_with(store) title="Pick another trace to put beside this one">"Compare with…"</button>
                <button class="btn small" on:click=move |_| export_report(store) title="Save this review as a web page to share">"Export report…"</button>
                <button class="btn small" on:click=reveal_file>{crate::prefs::reveal_label()}</button>
            </div>
            {move || match store.review.get() {
                None => view! { <p class="muted pad">"Reading the trace…"</p> }.into_any(),
                Some(Err(e)) => view! { <div class="warn">{e}</div> }.into_any(),
                Some(Ok(r)) => view! {
                    {header}
                    {report_line(store)}
                    <div class="review-note">
                        <NoteBox store=store key=None text=r.note.clone() label="Note on this trace" />
                    </div>
                    <div class="review-filters">
                        <label class="field">
                            "Namespace"
                            <select on:change=move |ev| {
                                let v = event_target_value(&ev);
                                filter.update(|f| f.namespace = namespace_choice(v));
                            }>
                                <option value="">"All"</option>
                                {move || namespaces().into_iter().map(|ns| view! { <option value=ns.clone()>{ns.clone()}</option> }).collect_view()}
                            </select>
                        </label>
                        <input class="review-search" type="search" placeholder="Filter workloads…"
                            prop:value=move || filter.get().text
                            on:input=move |ev| { let v = event_target_value(&ev); filter.update(|f| f.text = v); } />
                        <label class="check">
                            <input type="checkbox" prop:checked=move || filter.get().flagged_only
                                on:change=move |_| filter.update(|f| f.flagged_only = !f.flagged_only) />
                            "Needs attention only"
                        </label>
                        <span class="spacer"></span>
                        {move || {
                            let n = store.review_checked.with(|c| c.len());
                            let blocked = apply_blocked(store);
                            (n > 0).then(|| view! {
                                <button
                                    class="btn small primary"
                                    prop:disabled=blocked.is_some()
                                    title=apply_selected_title(blocked.clone())
                                    on:click=move |_| {
                                        let checked = store.review_checked.get_untracked();
                                        let rows = store.review.with_untracked(|r| checked_rows(r.as_ref(), &checked));
                                        crate::rightsize::ask(store, &rows);
                                    }
                                >{apply_selected_text(n)}</button>
                            })
                        }}
                        <span class="muted small">{move || rows_text(rows().len(), store.review_sel.with(Option::is_none))}</span>
                    </div>
                    <ReviewOverview store=store namespace=Signal::derive(move || filter.get().namespace) />
                    <div class="review-split">
                        <div class="review-table-wrap">{table}</div>
                        <ReviewDetail store=store />
                    </div>
                }.into_any(),
            }}
        </div>
    }
}

/// Every container of the trace (or of the namespace chosen) added up: how
/// much is used against how much is requested, over time.
#[component]
fn ReviewOverview(store: Store, #[prop(into)] namespace: Signal<Option<String>>) -> impl IntoView {
    let shown = RwSignal::new(true);
    let zoom = RwSignal::new(None::<(f64, f64)>);
    let series = RwSignal::new(None::<Result<TraceSeries, String>>);
    // Another namespace starts zoomed out.
    Effect::new(move |_| {
        namespace.track();
        zoom.set(None);
    });
    Effect::new(move |_| {
        let Some(path) = store.review_path.get() else { return };
        if !shown.get() {
            return;
        }
        store.review.track();
        let (ns, range) = (namespace.get(), zoom.get());
        series.set(None);
        spawn_local(async move {
            let result = invoke::<_, TraceSeries>("trace_overview", &OverviewArgs::new(path.clone(), ns.clone(), range)).await;
            if store.review_path.get_untracked().as_ref() == Some(&path) && namespace.get_untracked() == ns && zoom.get_untracked() == range {
                series.set(Some(result));
            }
        });
    });
    let data = Signal::derive(move || series.get().and_then(Result::ok));
    let on_zoom = Callback::new(move |range: (f64, f64)| zoom.set(Some(range)));
    let title = move || namespace.with(|ns| overview_title(ns.as_deref()));
    view! {
        <section class="review-overview">
            <div class="overview-head">
                <h3 class="chart-title">{title}</h3>
                <span class="muted small">"Every container added up: what's used against what's requested. Drag across to zoom."</span>
                <span class="spacer"></span>
                {move || (shown.get() && zoom.get().is_some()).then(|| view! {
                    <button class="btn small" on:click=move |_| zoom.set(None)>"Zoom out"</button>
                })}
                <button class="btn small" aria-expanded=move || shown.get().to_string() on:click=move |_| shown.update(|s| *s = !*s)>
                    {move || if shown.get() { "Hide" } else { "Show" }}
                </button>
            </div>
            {move || shown.get().then(|| view! {
                {move || series.get().and_then(Result::err).map(|e| view! { <p class="warn-text">{e}</p> })}
                <div class="overview-charts">
                    <div>
                        <h4 class="chart-title">"CPU"</h4>
                        <TraceChart series=data measure=Measure::Cpu on_zoom=on_zoom totals=true />
                    </div>
                    <div>
                        <h4 class="chart-title">"Memory"</h4>
                        <TraceChart series=data measure=Measure::Memory on_zoom=on_zoom totals=true />
                    </div>
                </div>
            })}
        </section>
    }
}

/// The selected row: suggestion, flags, charts and the day-and-hour grid.
#[component]
fn ReviewDetail(store: Store) -> impl IntoView {
    let zoom = RwSignal::new(None::<(f64, f64)>);
    let series = RwSignal::new(None::<Result<TraceSeries, String>>);
    let daily = RwSignal::new(None::<TraceDaily>);
    let heat = RwSignal::new(Measure::Cpu);
    let row = move || {
        let sel = store.review_sel.get()?;
        store.review.with(|r| find_row(r.as_ref(), &sel))
    };
    // A new selection starts zoomed out.
    Effect::new(move |_| {
        store.review_sel.track();
        zoom.set(None);
        daily.set(None);
        let (Some(path), Some(key)) = (store.review_path.get_untracked(), store.review_sel.get_untracked()) else { return };
        spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                path: String,
                key: TraceKey,
            }
            let result = invoke::<_, TraceDaily>("trace_daily", &Args { path, key: key.clone() }).await;
            if store.review_sel.get_untracked().as_ref() == Some(&key) {
                daily.set(result.ok());
            }
        });
    });
    Effect::new(move |_| {
        let (Some(path), Some(key)) = (store.review_path.get(), store.review_sel.get()) else { return };
        store.review.track();
        let range = zoom.get();
        series.set(None);
        spawn_local(async move {
            let result = invoke::<_, TraceSeries>("trace_series", &SeriesArgs::new(path, key.clone(), range, CHART_POINTS)).await;
            if store.review_sel.get_untracked().as_ref() == Some(&key) && zoom.get_untracked() == range {
                series.set(Some(result));
            }
        });
    });
    let chart_data = Signal::derive(move || series.get().and_then(Result::ok));
    let on_zoom = Callback::new(move |(a, b): (f64, f64)| zoom.set(Some((a, b))));
    let quick = move |label: &'static str, hours: Option<f64>| {
        let active = move || chart_data.with(|s| quick_active(hours, zoom.get(), s.as_ref()));
        view! {
            <button class="btn small toggle" class:on=active on:click=move |_| {
                if let Some(to) = chart_data.with_untracked(|s| quick_zoom(hours, s.as_ref())) {
                    zoom.set(to);
                }
            }>{label}</button>
        }
    };

    move || {
        let Some(row) = row() else {
            // The table gets the full width until a row is chosen.
            return ().into_any();
        };
        let suggestion = match &row.suggestion {
            Some(s) => {
                let line = |m: Measure, now: Option<f64>, to: f64| {
                    let (what, from, to, change) = suggest_line(m, now, to);
                    view! {
                        <div class=format!("suggest-line {}", m.class())>
                            <span class="res-dot" aria-hidden="true"></span>
                            <span class="suggest-what">{what}</span>
                            <span class="suggest-from">{from}</span>
                            <span class="muted">"→"</span>
                            <strong>{to}</strong>
                            <span class="muted small">{change}</span>
                        </div>
                    }
                };
                view! {
                    <div class="suggest-card">
                        <h3>"Suggested requests"</h3>
                        {line(Measure::Cpu, row.cpu_request, s.cpu_request)}
                        {line(Measure::Memory, row.mem_request, s.mem_request)}
                        <p class="muted small">{basis_text(&s.basis)}</p>
                        {apply_row(store, &row)}
                    </div>
                }
                .into_any()
            }
            None => view! { <div class="suggest-card"><h3>"No suggestion"</h3><p class="muted small">"Not enough data yet."</p></div> }.into_any(),
        };
        let flags = row
            .flags
            .iter()
            .map(|f| view! {
                <li class=format!("flag-line {}", flag_level(f))>
                    <span class=flag_class(f)><span class="flag-dot" aria-hidden="true"></span>{flag_text(f)}</span>
                    <span>{flag_explain(f)}</span>
                </li>
            })
            .collect_view();
        let facts = facts_text(&row);
        let figures = view! {
            <table class="figures">
                <tr><th></th><th class="num">"Request"</th><th class="num">"Limit"</th><th class="num">"p50"</th><th class="num">"p95"</th><th class="num">"p99"</th><th class="num">"Peak"</th></tr>
                <tr class="res-cpu">
                    <th><span class="res-dot" aria-hidden="true"></span>"CPU"</th>
                    <td class="num">{Measure::Cpu.opt(row.cpu_request)}</td>
                    <td class="num">{Measure::Cpu.opt(row.cpu_limit)}</td>
                    <td class="num">{Measure::Cpu.fmt(row.cpu_p50)}</td>
                    <td class="num">{Measure::Cpu.fmt(row.cpu_p95)}</td>
                    <td class="num">{Measure::Cpu.fmt(row.cpu_p99)}</td>
                    <td class="num">{Measure::Cpu.fmt(row.cpu_max)}</td>
                </tr>
                <tr class="res-memory">
                    <th><span class="res-dot" aria-hidden="true"></span>"Memory"</th>
                    <td class="num">{Measure::Memory.opt(row.mem_request)}</td>
                    <td class="num">{Measure::Memory.opt(row.mem_limit)}</td>
                    <td class="num muted">"—"</td>
                    <td class="num">{Measure::Memory.fmt(row.mem_p95)}</td>
                    <td class="num muted">"—"</td>
                    <td class="num">{Measure::Memory.fmt(row.mem_max)}</td>
                </tr>
            </table>
        };
        view! {
            <aside class="review-detail">
                <div class="modal-head">
                    <div>
                        <h2>{row.key.name.clone()}</h2>
                        <div class="muted small">{detail_subtitle(&row.key)}</div>
                    </div>
                    <span class="spacer"></span>
                    <crate::find::FindButton store=store />
                    <button class="close" title="Close" aria-label="Close" on:click=move |_| store.review_sel.set(None)>"×"</button>
                </div>
                <crate::find::FindBar store=store root="aside.review-detail" />
                {suggestion}
                <NoteBox store=store key=Some(row.key.clone()) text=row.note.clone() label="Note on this workload" />
                {(!row.flags.is_empty()).then(|| view! { <ul class="flag-list">{flags}</ul> })}
                {figures}
                <p class="muted small">{facts}</p>
                <div class="range-row">
                    {quick("All", None)}
                    {quick("Last 24h", Some(24.0))}
                    {quick("Last 6h", Some(6.0))}
                    {quick("Last 1h", Some(1.0))}
                    <span class="muted small">"Drag across a chart to zoom."</span>
                </div>
                {move || series.get().and_then(Result::err).map(|e| view! { <p class="warn-text">{e}</p> })}
                <h3 class="chart-title">"CPU"</h3>
                <TraceChart series=chart_data measure=Measure::Cpu on_zoom=on_zoom />
                <h3 class="chart-title">"Memory"</h3>
                <TraceChart series=chart_data measure=Measure::Memory on_zoom=on_zoom />
                <div class="heat-head-row">
                    <h3 class="chart-title">"By day and hour"</h3>
                    <span class="spacer"></span>
                    <button class="btn small toggle" class:on=move || heat.get() == Measure::Cpu on:click=move |_| heat.set(Measure::Cpu)>"CPU p95"</button>
                    <button class="btn small toggle" class:on=move || heat.get() == Measure::Memory on:click=move |_| heat.set(Measure::Memory)>"Memory peak"</button>
                </div>
                {move || {
                    let m = heat.get();
                    view! { <TraceHeat daily=Signal::derive(move || daily.get()) measure=m /> }
                }}
            </aside>
        }
        .into_any()
    }
}
