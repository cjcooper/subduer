//! Comparing traces: two or more files side by side (oldest data first),
//! rows matched by workload and container, with each trace's requests and
//! usage and the change from the first to the last. For checking a resize:
//! requests should drop while usage stays put and no OOM kills appear.
//!
//! The logic is in `logic::compare`.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{TraceComparison, TraceKey, TraceSeries, TraceSummary};

use crate::app::{Store, guard};
use crate::logic::compare::{
    OVERLAY_POINTS, cell_flags, cell_lines, change_heading, change_lines, count_text_of, data_start, ends, errors, filter, head_text, headers, legend_text,
    lines, only_text, overlaid, pick, shown_text, sorted, title, trace_header,
};
use crate::logic::trace::scope_text;
use crate::tauri::invoke;
use crate::logic::trace_overlay::{Align, picked, trace_slot, trace_var};
use crate::logic::trace_review::{SeriesArgs, file_name, key_subtitle};
use crate::trace_chart::Measure;
use crate::trace_overlay::{OverlayChart, local_offset};

const TABLE: &str = "compare";

/// Shows the comparison of these files.
pub(crate) fn open(store: Store, paths: Vec<String>) {
    guard(store, move || {
        store.selected.set(None);
        store.forwards_open.set(false);
        store.traces_open.set(true);
        store.comparison.set(None);
        store.compare_paths.set(Some(paths.clone()));
        spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                paths: Vec<String>,
            }
            let result = invoke::<_, TraceComparison>("trace_compare", &Args { paths: paths.clone() }).await;
            if store.compare_paths.get_untracked().as_ref() == Some(&paths) {
                store.comparison.set(Some(result));
            }
        });
    });
}

/// A date in this browser's locale.
fn local_date(t: f64) -> String {
    js_sys::Date::new(&(t * 1000.0).into())
        .to_locale_date_string("default", &wasm_bindgen::JsValue::UNDEFINED)
        .as_string()
        .unwrap_or_default()
}

#[component]
pub(crate) fn CompareView(store: Store) -> impl IntoView {
    let text = RwSignal::new(String::new());
    let changed_only = RwSignal::new(false);
    // The row whose charts are overlaid, and how the traces line up.
    let sel = RwSignal::new(None::<TraceKey>);
    let align = RwSignal::new(Align::SinceStart);
    let pick = move |key: TraceKey| sel.update(|s| pick(s, key));
    let back = move |_| {
        store.compare_paths.set(None);
        store.comparison.set(None);
    };
    let body = move || match store.comparison.get() {
        None => view! { <p class="muted pad">"Reading the traces…"</p> }.into_any(),
        Some(Err(e)) => view! { <div class="warn">{e}</div> }.into_any(),
        Some(Ok(c)) => {
            let heads = c
                .traces
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let started = local_date(data_start(t));
                    let header = trace_header(i);
                    let (h1, h2) = (header.clone(), header.clone());
                    view! {
                        <th class="cmp-head" title="Sort by CPU request in this trace">
                            <button class="th-sort cmp-index" class:on=move || store.sorts.get(TABLE).is_some_and(|(s, _)| s == h1) on:click=move |_| store.sorts.click(TABLE, &header)>
                                // The trace's colour on overlays, beside its number.
                                {trace_slot(i, c.traces.len()).map(|slot| view! {
                                    <span class="ochart-swatch" style=trace_var(slot) aria-hidden="true"></span>
                                })}
                                {(i + 1).to_string()}
                                <span class="sort-arrow">{move || store.sorts.arrow(TABLE, &h2)}</span>
                            </button>
                            <span class="cmp-file" title=t.path.clone()>{file_name(&t.path).to_owned()}</span>
                            <span class="muted small">{head_text(t, &started)}</span>
                            <span class="muted small">{scope_text(&t.scope)}</span>
                        </th>
                    }
                })
                .collect_view();
            let n = c.traces.len();
            let (overlay_traces, overlay_rows) = (c.traces.clone(), c.rows.clone());
            let headers = headers(n);
            let names: Vec<&str> = headers.iter().map(String::as_str).collect();
            let rows = sorted(filter(&c.rows, &text.get(), changed_only.get()), store.sorts.by(TABLE, &names), n);
            let count = rows.len();
            let body = rows
                .into_iter()
                .map(|row| {
                    let cells = row
                        .cells
                        .iter()
                        .map(|cell| match cell {
                            None => view! { <td class="muted cmp-cell">"not in this trace"</td> }.into_any(),
                            Some(c) => {
                                let (cpu, memory, pods) = cell_lines(c);
                                let (ooms, throttled) = cell_flags(c);
                                view! {
                                    <td class="cmp-cell">
                                        <div class="res-cpu"><span class="res-dot" aria-hidden="true"></span>{cpu}</div>
                                        <div class="res-memory"><span class="res-dot" aria-hidden="true"></span>{memory}</div>
                                        <div class="muted small">
                                            {pods}
                                            {ooms.map(|t| view! { <span class="flag critical"><span class="flag-dot" aria-hidden="true"></span>{t}</span> })}
                                            {throttled.map(|t| view! { <span class="flag serious"><span class="flag-dot" aria-hidden="true"></span>{t}</span> })}
                                        </div>
                                    </td>
                                }
                                .into_any()
                            }
                        })
                        .collect_view();
                    let key = row.key.clone();
                    let selected = { let key = key.clone(); move || sel.with(|s| s.as_ref() == Some(&key)) };
                    if let Some(only) = only_text(&row, n) {
                        return view! {
                            <tr class="cmp-row" class:selected=selected on:click=move |_| pick(key.clone())>
                                <td>
                                    <div class="review-name">{row.key.name.clone()}</div>
                                    <div class="muted small">{key_subtitle(&row.key)}</div>
                                </td>
                                {cells}
                                <td class="muted">{only}</td>
                            </tr>
                        }
                        .into_any();
                    }
                    let change = ends(&row).map(|(a, b)| {
                        let [cpu, p95, memory, peak] = change_lines(a, b);
                        view! {
                            <td class="cmp-cell">
                                <div class="res-cpu">"Request "{cpu}</div>
                                <div class="muted small">"p95 "{p95}</div>
                                <div class="res-memory">"Request "{memory}</div>
                                <div class="muted small">"peak "{peak}</div>
                            </td>
                        }
                    });
                    view! {
                        <tr class="cmp-row" class:selected=selected on:click=move |_| pick(key.clone())>
                            <td>
                                <div class="review-name">{row.key.name.clone()}</div>
                                <div class="muted small">{key_subtitle(&row.key)}</div>
                            </td>
                            {cells}
                            {change}
                        </tr>
                    }
                    .into_any()
                })
                .collect_view();
            view! {
                <div class="review-filters">
                    <input class="review-search" type="search" placeholder="Filter workloads…"
                        prop:value=move || text.get()
                        on:input=move |ev| text.set(event_target_value(&ev)) />
                    <label class="check">
                        <input type="checkbox" prop:checked=move || changed_only.get() on:change=move |_| changed_only.update(|v| *v = !*v) />
                        "Requests changed only"
                    </label>
                    <span class="spacer"></span>
                    <span class="muted small">{count_text_of(count, c.rows.len())}</span>
                </div>
                {move || {
                    let key = sel.get()?;
                    let present: Vec<bool> = overlay_rows.iter().find(|r| r.key == key)?.cells.iter().map(Option::is_some).collect();
                    Some(view! {
                        <CompareOverlay key=key traces=overlay_traces.clone() present=present align=align on_close=Callback::new(move |()| sel.set(None)) />
                    })
                }}
                <div class="review-table-wrap cmp-wrap">
                    <table class="resources review-table compare-table">
                        <thead><tr>
                            <crate::sort::SortTh sorts=store.sorts table=TABLE header="Workload" />
                            {heads}
                            <th title="Sort by the change in CPU request (memory where CPU has none)">
                                <button class="th-sort" class:on=move || store.sorts.get(TABLE).is_some_and(|(s, _)| s == "Change") on:click=move |_| store.sorts.click(TABLE, "Change")>
                                    {change_heading(n)}<span class="sort-arrow">{move || store.sorts.arrow(TABLE, "Change")}</span>
                                </button>
                            </th>
                        </tr></thead>
                        <tbody>{body}</tbody>
                    </table>
                </div>
            }
            .into_any()
        }
    };
    view! {
        <div class="traces-page review">
            <div class="resource-header page-head">
                <button class="btn small" on:click=back title="Back to the list of traces">"← Traces"</button>
                <h1>{move || store.compare_paths.with(|p| title(p.as_ref()))}</h1>
                <span class="spacer"></span>
            </div>
            <p class="muted pad-x">"Oldest data first. After a resize, requests should drop while usage stays about the same and no OOM kills or throttling appear."</p>
            {body}
        </div>
    }
}

/// One row's CPU and memory from each trace that has it, overlaid.
#[component]
fn CompareOverlay(key: TraceKey, traces: Vec<TraceSummary>, present: Vec<bool>, align: RwSignal<Align>, on_close: Callback<()>) -> impl IntoView {
    // The comparison's overlaid traces that have this row; each keeps its colour.
    let n = traces.len();
    let shown = overlaid(n, &present);
    let note = shown_text(&picked(n), n);
    // Each shown trace's series, by its index in the comparison.
    let loaded = RwSignal::new(Vec::<(usize, Result<TraceSeries, String>)>::new());
    for &i in &shown {
        let (path, key) = (traces[i].path.clone(), key.clone());
        spawn_local(async move {
            let result = invoke::<_, TraceSeries>("trace_series", &SeriesArgs::new(path, key, None, OVERLAY_POINTS)).await;
            // Dropped if the row was closed or another chosen meanwhile.
            let _ = loaded.try_update(|l| l.push((i, result)));
        });
    }
    let with_offset = |t: f64| (t, local_offset(t));
    let first = shown.first().map(|&i| with_offset(data_start(&traces[i]))).unwrap_or_default();
    let starts: Vec<(f64, f64)> = traces.iter().map(|t| with_offset(data_start(t))).collect();
    let lines = Signal::derive(move || {
        let a = align.get();
        loaded.with(|l| lines(l, &starts, first, a))
    });
    let errors = move || loaded.with(|l| errors(l));
    let toggle = move |to: Align, label: &'static str| {
        view! { <button class="btn small toggle" class:on=move || align.get() == to on:click=move |_| align.set(to)>{label}</button> }
    };
    let legend = shown
        .iter()
        .map(|&i| {
            let (t, slot) = (&traces[i], trace_slot(i, n).unwrap_or_default());
            let started = local_date(data_start(t));
            view! {
                <span style=trace_var(slot)>
                    <span class="key ochart-key"></span>
                    {legend_text(i, t, &started)}
                </span>
            }
        })
        .collect_view();
    view! {
        <section class="review-overview cmp-overlay">
            <div class="overview-head">
                <h3 class="chart-title">{key.name.clone()}</h3>
                <span class="muted small">{key_subtitle(&key)}</span>
                <span class="spacer"></span>
                <span class="muted small">"Line up by"</span>
                {toggle(Align::SinceStart, "Since start")}
                {toggle(Align::TimeOfWeek, "Time of week")}
                <button class="close" title="Close" aria-label="Close" on:click=move |_| on_close.run(())>"×"</button>
            </div>
            {note.map(|n| view! { <p class="muted small">{n}</p> })}
            {move || Some(errors()).filter(|e| !e.is_empty()).map(|e| view! { <p class="warn-text">{e}</p> })}
            <div class="overview-charts">
                <div>
                    <h4 class="chart-title">"CPU"</h4>
                    <OverlayChart lines=lines measure=Measure::Cpu align=align />
                </div>
                <div>
                    <h4 class="chart-title">"Memory"</h4>
                    <OverlayChart lines=lines measure=Measure::Memory align=align />
                </div>
            </div>
            <div class="tchart-legend">
                {legend}
                <span><span class="key key-request"></span>"Request (dashed, in the trace's colour)"</span>
            </div>
        </section>
    }
}
