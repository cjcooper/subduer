//! Comparing traces: two or more files side by side (oldest data first),
//! rows matched by workload and container, with each trace's requests and
//! usage and the change from the first to the last. For checking a resize:
//! requests should drop while usage stays put and no OOM kills appear.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{CompareCell, CompareRow, TraceComparison, TraceKey, TraceSeries, TraceSummary};

use crate::app::{Store, guard};
use crate::tauri::invoke;
use crate::trace::{duration_text, scope_text};
use crate::trace_chart::Measure;
use crate::trace_overlay::{Align, Line, MAX_TRACES, OverlayChart, local_offset, picked, shift};
use crate::trace_review::{change_text, file_name};

/// Buckets per trace on an overlay.
const OVERLAY_POINTS: u32 = 400;

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

/// The first and last traces that have the row.
fn ends(row: &CompareRow) -> Option<(&CompareCell, &CompareCell)> {
    let mut present = row.cells.iter().flatten();
    let first = present.next()?;
    Some((first, present.last().unwrap_or(first)))
}

/// A request or limit differs between the traces that have the row.
pub(crate) fn requests_changed(row: &CompareRow) -> bool {
    let mut cells = row.cells.iter().flatten();
    let Some(first) = cells.next() else { return false };
    cells.any(|c| c.cpu_request != first.cpu_request || c.mem_request != first.mem_request)
}

/// Rows matching the text, and only changed ones if asked.
pub(crate) fn filter(rows: &[CompareRow], text: &str, changed_only: bool) -> Vec<CompareRow> {
    let text = text.trim().to_lowercase();
    rows.iter()
        .filter(|r| text.is_empty() || [&r.key.name, &r.key.container, &r.key.namespace].iter().any(|s| s.to_lowercase().contains(&text)))
        .filter(|r| !changed_only || requests_changed(r))
        .cloned()
        .collect()
}

/// `1.5 cores → 15m (−99%)`, or the value alone when it didn't change.
pub(crate) fn from_to(a: Option<f64>, b: Option<f64>, m: Measure) -> String {
    match (a, b) {
        (Some(a), Some(b)) if a == b => m.fmt(a),
        (Some(a), Some(b)) => format!("{} → {} ({})", m.fmt(a), m.fmt(b), change_text(a, b)),
        (None, Some(b)) => format!("not set → {}", m.fmt(b)),
        (Some(a), None) => format!("{} → not set", m.fmt(a)),
        (None, None) => "not set".into(),
    }
}

fn opt(v: Option<f64>, m: Measure) -> String {
    v.map_or("—".into(), |v| m.fmt(v))
}

#[component]
pub(crate) fn CompareView(store: Store) -> impl IntoView {
    let text = RwSignal::new(String::new());
    let changed_only = RwSignal::new(false);
    // The row whose charts are overlaid, and how the traces line up.
    let sel = RwSignal::new(None::<TraceKey>);
    let align = RwSignal::new(Align::SinceStart);
    let pick = move |key: TraceKey| sel.update(|s| *s = if s.as_ref() == Some(&key) { None } else { Some(key) });
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
                    let span = match (t.data_start, t.data_end) {
                        (Some(a), Some(b)) => duration_text(b - a),
                        _ => "no data".into(),
                    };
                    let started = js_sys::Date::new(&(t.data_start.unwrap_or(t.started) * 1000.0).into())
                        .to_locale_date_string("default", &wasm_bindgen::JsValue::UNDEFINED)
                        .as_string()
                        .unwrap_or_default();
                    view! {
                        <th class="cmp-head">
                            <span class="cmp-index">{format!("{}", i + 1)}</span>
                            <span class="cmp-file" title=t.path.clone()>{file_name(&t.path).to_owned()}</span>
                            <span class="muted small">{format!("{started} · {span} · {}", t.context)}</span>
                            <span class="muted small">{scope_text(&t.scope)}</span>
                        </th>
                    }
                })
                .collect_view();
            let n = c.traces.len();
            let (overlay_traces, overlay_rows) = (c.traces.clone(), c.rows.clone());
            let rows = filter(&c.rows, &text.get(), changed_only.get());
            let count = rows.len();
            let body = rows
                .into_iter()
                .map(|row| {
                    let cells = row
                        .cells
                        .iter()
                        .map(|cell| match cell {
                            None => view! { <td class="muted cmp-cell">"not in this trace"</td> }.into_any(),
                            Some(c) => view! {
                                <td class="cmp-cell">
                                    <div class="res-cpu"><span class="res-dot" aria-hidden="true"></span>{format!("{} · p95 {}", opt(c.cpu_request, Measure::Cpu), Measure::Cpu.fmt(c.cpu_p95))}</div>
                                    <div class="res-memory"><span class="res-dot" aria-hidden="true"></span>{format!("{} · peak {}", opt(c.mem_request, Measure::Memory), Measure::Memory.fmt(c.mem_max))}</div>
                                    <div class="muted small">
                                        {format!("{} pods · {}", format!("{:.1}", c.avg_pods).trim_end_matches(".0"), duration_text(c.hours * 3600.0))}
                                        {(c.ooms > 0).then(|| view! { <span class="flag critical"><span class="flag-dot" aria-hidden="true"></span>{format!("OOM ×{}", c.ooms)}</span> })}
                                        {c.throttled.map(|t| view! { <span class="flag serious"><span class="flag-dot" aria-hidden="true"></span>{format!("Throttled {:.0}%", t * 100.0)}</span> })}
                                    </div>
                                </td>
                            }
                            .into_any(),
                        })
                        .collect_view();
                    let present: Vec<usize> = row.cells.iter().enumerate().filter(|(_, c)| c.is_some()).map(|(i, _)| i + 1).collect();
                    let key = row.key.clone();
                    let selected = { let key = key.clone(); move || sel.with(|s| s.as_ref() == Some(&key)) };
                    if present.len() < n {
                        let only = present.iter().map(usize::to_string).collect::<Vec<_>>().join(", ");
                        return view! {
                            <tr class="cmp-row" class:selected=selected on:click=move |_| pick(key.clone())>
                                <td>
                                    <div class="review-name">{row.key.name.clone()}</div>
                                    <div class="muted small">{format!("{} · {} · {}", row.key.kind, row.key.container, row.key.namespace)}</div>
                                </td>
                                {cells}
                                <td class="muted">{format!("Only in trace {only}")}</td>
                            </tr>
                        }
                        .into_any();
                    }
                    let change = ends(&row).map(|(a, b)| {
                        view! {
                            <td class="cmp-cell">
                                <div class="res-cpu">"Request "{from_to(a.cpu_request, b.cpu_request, Measure::Cpu)}</div>
                                <div class="muted small">"p95 "{from_to(Some(a.cpu_p95), Some(b.cpu_p95), Measure::Cpu)}</div>
                                <div class="res-memory">"Request "{from_to(a.mem_request, b.mem_request, Measure::Memory)}</div>
                                <div class="muted small">"peak "{from_to(Some(a.mem_max), Some(b.mem_max), Measure::Memory)}</div>
                            </td>
                        }
                    });
                    view! {
                        <tr class="cmp-row" class:selected=selected on:click=move |_| pick(key.clone())>
                            <td>
                                <div class="review-name">{row.key.name.clone()}</div>
                                <div class="muted small">{format!("{} · {} · {}", row.key.kind, row.key.container, row.key.namespace)}</div>
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
                    <span class="muted small">{format!("{count} of {} rows · select one to overlay its charts", c.rows.len())}</span>
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
                        <thead><tr><th>"Workload"</th>{heads}<th>{format!("Change, 1 → {n}")}</th></tr></thead>
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
                <h1>{move || format!("Comparing {} traces", store.compare_paths.with(|p| p.as_ref().map_or(0, Vec::len)))}</h1>
                <span class="spacer"></span>
            </div>
            <p class="muted pad-x">"Oldest data first. After a resize, requests should drop while usage stays about the same and no OOM kills or throttling appear."</p>
            {body}
        </div>
    }
}

/// When a trace's data starts.
fn data_start(t: &TraceSummary) -> f64 {
    t.data_start.unwrap_or(t.started)
}

/// "Showing traces 1, 4 and 5 of 5", when the overlay can't hold them all.
pub(crate) fn shown_text(shown: &[usize], of: usize) -> Option<String> {
    (shown.len() < of).then(|| {
        let mut n: Vec<String> = shown.iter().map(|i| (i + 1).to_string()).collect();
        let last = n.pop().unwrap_or_default();
        format!("Showing traces {} and {last} of {of}: an overlay holds {MAX_TRACES} at most.", n.join(", "))
    })
}

/// One row's CPU and memory from each trace that has it, overlaid.
#[component]
fn CompareOverlay(key: TraceKey, traces: Vec<TraceSummary>, present: Vec<bool>, align: RwSignal<Align>, on_close: Callback<()>) -> impl IntoView {
    let have: Vec<usize> = (0..traces.len()).filter(|&i| present.get(i) == Some(&true)).collect();
    let shown: Vec<usize> = picked(have.len()).into_iter().map(|i| have[i]).collect();
    let note = shown_text(&shown, have.len());
    // Each shown trace's series, by its index in the comparison.
    let loaded = RwSignal::new(Vec::<(usize, Result<TraceSeries, String>)>::new());
    for &i in &shown {
        let (path, key) = (traces[i].path.clone(), key.clone());
        spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                path: String,
                key: TraceKey,
                start: Option<f64>,
                end: Option<f64>,
                points: u32,
            }
            let result = invoke::<_, TraceSeries>("trace_series", &Args { path, key, start: None, end: None, points: OVERLAY_POINTS }).await;
            // Dropped if the row was closed or another chosen meanwhile.
            let _ = loaded.try_update(|l| l.push((i, result)));
        });
    }
    let (first, first_offset) = shown.first().map(|&i| (data_start(&traces[i]), local_offset(data_start(&traces[i])))).unwrap_or_default();
    let starts: Vec<(f64, f64)> = traces.iter().map(|t| (data_start(t), local_offset(data_start(t)))).collect();
    let order = shown.clone();
    let lines = Signal::derive(move || {
        let a = align.get();
        loaded.with(|l| {
            let mut lines: Vec<Line> = l
                .iter()
                .filter_map(|(i, r)| {
                    let series = r.as_ref().ok()?.clone();
                    let (start, offset) = starts[*i];
                    Some(Line { n: i + 1, slot: order.iter().position(|o| o == i)?, series, shift: shift(a, start, offset, first, first_offset) })
                })
                .collect();
            lines.sort_by_key(|l| l.n);
            lines
        })
    });
    let errors = move || {
        loaded.with(|l| l.iter().filter_map(|(i, r)| r.as_ref().err().map(|e| format!("Trace {}: {e}", i + 1))).collect::<Vec<_>>().join(" "))
    };
    let toggle = move |to: Align, label: &'static str| {
        view! { <button class="btn small toggle" class:on=move || align.get() == to on:click=move |_| align.set(to)>{label}</button> }
    };
    let legend = shown
        .iter()
        .enumerate()
        .map(|(slot, &i)| {
            let t = &traces[i];
            let started = js_sys::Date::new(&(data_start(t) * 1000.0).into())
                .to_locale_date_string("default", &wasm_bindgen::JsValue::UNDEFINED)
                .as_string()
                .unwrap_or_default();
            view! {
                <span style=format!("--trace: var(--trace-{})", slot + 1)>
                    <span class="key ochart-key"></span>
                    {format!("Trace {} · {} · {started}", i + 1, file_name(&t.path))}
                </span>
            }
        })
        .collect_view();
    view! {
        <section class="review-overview cmp-overlay">
            <div class="overview-head">
                <h3 class="chart-title">{key.name.clone()}</h3>
                <span class="muted small">{format!("{} · {} · {}", key.kind, key.container, key.namespace)}</span>
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

#[cfg(test)]
#[path = "compare_tests.rs"]
mod tests;
