//! Comparing traces: two or more files side by side (oldest data first),
//! rows matched by workload and container, with each trace's requests and
//! usage and the change from the first to the last. For checking a resize:
//! requests should drop while usage stays put and no OOM kills appear.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{CompareCell, CompareRow, TraceComparison};

use crate::app::{Store, guard};
use crate::tauri::invoke;
use crate::trace::{duration_text, scope_text};
use crate::trace_chart::Measure;
use crate::trace_review::{change_text, file_name};

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
                    if present.len() < n {
                        let only = present.iter().map(usize::to_string).collect::<Vec<_>>().join(", ");
                        return view! {
                            <tr>
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
                        <tr>
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
                    <span class="muted small">{format!("{count} of {} rows", c.rows.len())}</span>
                </div>
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

#[cfg(test)]
#[path = "compare_tests.rs"]
mod tests;
