//! The Traces page: recent trace files and Open trace…, then a file's review:
//! the right-sizing table (one row per workload container, sortable and
//! filterable) and, for the selected row, its suggestion, flags, CPU and
//! memory over the trace, and usage by day and hour. Works without a cluster.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{ReviewFlag, ReviewRow, TraceDaily, TraceKey, TraceReview, TraceSeries};

use crate::app::{Store, guard};
use crate::prefs::RecentTrace;
use crate::tauri::{NoArgs, invoke};
use crate::trace::{duration_text, scope_text};
use crate::trace_chart::{Measure, TraceChart, TraceHeat};

/// Buckets asked for per chart: about one per two pixels.
const CHART_POINTS: u32 = 400;

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
            let recent = RecentTrace { path, context: r.summary.context.clone(), what: scope_text(&r.summary.scope), at: js_sys::Date::now() };
            store.recent_traces.set(crate::prefs::remember_trace(recent));
            // Keep the selection if the row is still there.
            if let Some(sel) = store.review_sel.get_untracked()
                && !r.rows.iter().any(|row| row.key == sel)
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

/// The file's name without its folder.
pub(crate) fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

// --- Flags ----------------------------------------------------------------------

/// Severity for a flag's badge: critical, serious, warning, or info.
pub(crate) fn flag_level(f: &ReviewFlag) -> &'static str {
    match f {
        ReviewFlag::OomKilled { .. } | ReviewFlag::OomEvents { .. } => "critical",
        ReviewFlag::Throttled { .. } | ReviewFlag::NearMemoryLimit { .. } | ReviewFlag::AboveMemoryLimit => "serious",
        ReviewFlag::Restarts { .. } | ReviewFlag::NoCpuRequest | ReviewFlag::NoMemoryRequest => "warning",
        ReviewFlag::RequestsChanged | ReviewFlag::ShortData { .. } | ReviewFlag::TooLittleData { .. } => "info",
    }
}

fn pct(v: f64) -> String {
    format!("{:.0}%", v * 100.0)
}

fn times(n: u32) -> String {
    format!("×{n}")
}

/// A short label for a badge.
pub(crate) fn flag_text(f: &ReviewFlag) -> String {
    match f {
        ReviewFlag::Throttled { worst } => format!("Throttled {}", pct(*worst)),
        ReviewFlag::NearMemoryLimit { share } => format!("Memory {} of limit", pct(*share)),
        ReviewFlag::OomKilled { count } => format!("OOMKilled {}", times(*count)),
        ReviewFlag::OomEvents { count } => format!("OOM {}", times(*count)),
        ReviewFlag::Restarts { count } => format!("Restarts {}", times(*count)),
        ReviewFlag::NoCpuRequest => "No CPU request".into(),
        ReviewFlag::NoMemoryRequest => "No memory request".into(),
        ReviewFlag::RequestsChanged => "Requests changed".into(),
        ReviewFlag::ShortData { hours } => format!("{} of data", duration_text(hours * 3600.0)),
        ReviewFlag::TooLittleData { hours } => format!("Only {} of data", duration_text(hours * 3600.0)),
        ReviewFlag::AboveMemoryLimit => "Above the memory limit".into(),
    }
}

/// What a flag means, for the detail panel.
pub(crate) fn flag_explain(f: &ReviewFlag) -> String {
    match f {
        ReviewFlag::Throttled { worst } => format!(
            "CPU was throttled by its limit: in the worst minute {} of scheduling periods were held back. Raising or removing the CPU limit helps more than the request.",
            pct(*worst)
        ),
        ReviewFlag::NearMemoryLimit { share } => format!("Peak memory reached {} of the limit. Close to an OOM kill.", pct(*share)),
        ReviewFlag::OomKilled { count } => format!(
            "The container was OOM-killed and restarted {}: it needs more memory than its limit.",
            plural(u64::from(*count), "time", "times")
        ),
        ReviewFlag::OomEvents { count } => {
            format!("The kernel killed a process for memory {} without the container restarting.", plural(u64::from(*count), "time", "times"))
        }
        ReviewFlag::Restarts { count } => format!("The container restarted {} for reasons other than memory.", plural(u64::from(*count), "time", "times")),
        ReviewFlag::NoCpuRequest => "No CPU request is set, so the scheduler assumes it needs none.".into(),
        ReviewFlag::NoMemoryRequest => "No memory request is set, so the scheduler assumes it needs none and it's evicted first under pressure.".into(),
        ReviewFlag::RequestsChanged => "Requests changed during the trace; the figures cover both settings. The current values are shown.".into(),
        ReviewFlag::ShortData { hours } => format!("Only {} of data: daily or weekly peaks may be missing.", duration_text(hours * 3600.0)),
        ReviewFlag::TooLittleData { hours } => {
            format!("Only {} of data, too little to suggest anything. Keep tracing for at least 6 hours, ideally a few days.", duration_text(hours * 3600.0))
        }
        ReviewFlag::AboveMemoryLimit => "The suggested memory request is above the memory limit; the limit must be raised with it.".into(),
    }
}

/// Sort key: critical first.
fn severity(f: &ReviewFlag) -> u8 {
    match flag_level(f) {
        "critical" => 0,
        "serious" => 1,
        "warning" => 2,
        _ => 3,
    }
}

/// `1 pod`, `3 pods`.
pub(crate) fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Rows worth attention: anything but the data-length notes.
fn flagged(row: &ReviewRow) -> bool {
    row.flags.iter().any(|f| flag_level(f) != "info") || row.cpu_spare.is_some_and(|s| s < 0.0) || row.mem_spare.is_some_and(|s| s < 0.0)
}

// --- Sorting and filtering ------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortBy {
    Workload,
    Pods,
    CpuRequest,
    CpuP95,
    CpuSpare,
    MemRequest,
    MemPeak,
    MemSpare,
}

/// Sorts rows; numbers descending by default, names ascending. Missing values go last.
pub(crate) fn sort_rows(rows: &mut [ReviewRow], by: SortBy, descending: bool) {
    let num = |r: &ReviewRow| -> Option<f64> {
        match by {
            SortBy::Workload => None,
            SortBy::Pods => Some(r.avg_pods),
            SortBy::CpuRequest => r.cpu_request,
            SortBy::CpuP95 => Some(r.cpu_p95),
            SortBy::CpuSpare => r.cpu_spare,
            SortBy::MemRequest => r.mem_request,
            SortBy::MemPeak => Some(r.mem_max),
            SortBy::MemSpare => r.mem_spare,
        }
    };
    rows.sort_by(|a, b| {
        if by == SortBy::Workload {
            let ord = (&a.key.name, &a.key.container, &a.key.namespace).cmp(&(&b.key.name, &b.key.container, &b.key.namespace));
            return if descending { ord.reverse() } else { ord };
        }
        match (num(a), num(b)) {
            (Some(x), Some(y)) => {
                let ord = x.total_cmp(&y);
                if descending { ord.reverse() } else { ord }
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Filter {
    pub(crate) namespace: Option<String>,
    pub(crate) text: String,
    pub(crate) flagged_only: bool,
}

pub(crate) fn filter_rows(rows: &[ReviewRow], f: &Filter) -> Vec<ReviewRow> {
    let text = f.text.trim().to_lowercase();
    rows.iter()
        .filter(|r| f.namespace.as_ref().is_none_or(|ns| *ns == r.key.namespace))
        .filter(|r| {
            text.is_empty() || [&r.key.name, &r.key.container, &r.key.namespace, &r.key.kind].iter().any(|s| s.to_lowercase().contains(&text))
        })
        .filter(|r| !f.flagged_only || flagged(r))
        .cloned()
        .collect()
}

/// `−99%`, `+12%`: the change from `from` to `to`.
pub(crate) fn change_text(from: f64, to: f64) -> String {
    if from <= 0.0 {
        return String::new();
    }
    let p = (to - from) / from * 100.0;
    if p.abs() < 0.5 { "no change".into() } else if p < 0.0 { format!("−{:.0}%", -p) } else { format!("+{p:.0}%") }
}

/// Spare capacity: blank without a request, "short" (in the danger ink) when usage is above it.
fn spare_cell(v: Option<f64>, measure: Measure) -> impl IntoView {
    match v {
        None => view! { <td class="num muted">"—"</td> }.into_any(),
        Some(v) if v < 0.0 => view! { <td class="num short" title="Usage above the request">{format!("short {}", measure.fmt(-v))}</td> }.into_any(),
        Some(v) => view! { <td class="num">{measure.fmt(v)}</td> }.into_any(),
    }
}

fn opt(v: Option<f64>, measure: Measure) -> String {
    v.map_or("—".into(), |v| measure.fmt(v))
}

// --- The page --------------------------------------------------------------------

#[component]
pub(crate) fn TracesPage(store: Store) -> impl IntoView {
    move || {
        if store.review_path.with(Option::is_some) {
            view! { <ReviewView store=store /> }.into_any()
        } else {
            view! { <TraceList store=store /> }.into_any()
        }
    }
}

#[component]
fn TraceList(store: Store) -> impl IntoView {
    let running = move || {
        store.trace.get().map(|s| {
            let path = s.path.clone();
            view! {
                <div class="trace-card running">
                    <span class="rec-dot" aria-hidden="true"></span>
                    <div class="trace-card-text">
                        <strong>{file_name(&s.path).to_owned()}</strong>
                        <span class="muted small">{format!("Recording {} of {}", scope_text(&s.scope), s.context)}</span>
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
                let (path, forget_path) = (t.path.clone(), t.path.clone());
                let when = js_sys::Date::new(&t.at.into()).to_locale_string("default", &wasm_bindgen::JsValue::UNDEFINED).as_string().unwrap_or_default();
                view! {
                    <div class="trace-card">
                        <button class="trace-card-open" title=t.path.clone() on:click=move |_| open_file(store, path.clone())>
                            <strong>{file_name(&t.path).to_owned()}</strong>
                            <span class="muted small">{format!("{} · {} · {when}", t.context, t.what)}</span>
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
                <button class="btn primary" on:click=move |_| choose_and_open(store)>"Open trace…"</button>
            </div>
            <p class="muted">"Review CPU and memory recorded over hours or days, against requests, to right-size workloads. Opening a trace doesn't need a cluster."</p>
            {running}
            <h2 class="section-title">"Recent"</h2>
            <div class="trace-cards">{recent}</div>
        </div>
    }
}

#[component]
fn ReviewView(store: Store) -> impl IntoView {
    let sort = RwSignal::new((SortBy::CpuSpare, true));
    let filter = RwSignal::new(Filter::default());
    let back = move |_| {
        store.review_path.set(None);
        store.review.set(None);
        store.review_sel.set(None);
    };
    let reveal = move |_| {
        #[derive(Serialize)]
        struct Args {
            path: String,
        }
        let Some(path) = store.review_path.get_untracked() else { return };
        spawn_local(async move {
            if let Err(e) = invoke::<_, ()>("reveal_trace", &Args { path }).await {
                store.error.set(Some(e));
            }
        });
    };
    let recording = move || store.review.with(|r| matches!(r, Some(Ok(r)) if r.summary.ended.is_none() && !r.summary.unfinished));

    let header = move || {
        let r = store.review.get()?.ok()?;
        let s = &r.summary;
        let span = match (s.data_start, s.data_end) {
            (Some(a), Some(b)) => duration_text(b - a),
            _ => "no data".into(),
        };
        let state = if s.unfinished {
            "Ended unexpectedly (data up to the last pull is intact)".to_owned()
        } else if s.ended.is_none() {
            "Still recording".to_owned()
        } else {
            match s.end_reason {
                Some(subduer_types::TraceEnd::Quit) => "Stopped when the app quit".into(),
                Some(subduer_types::TraceEnd::ContextChanged) => "Stopped on a context switch".into(),
                Some(subduer_types::TraceEnd::ConnectionLost) => "Stopped: connection lost".into(),
                _ => "Stopped".into(),
            }
        };
        let local = s.local_samples as f64 / (s.local_samples + s.prometheus_samples).max(1) as f64;
        let source = if s.prometheus.is_none() {
            "Sampled by the app (no Prometheus)".to_owned()
        } else if local > 0.0 {
            format!("Prometheus, {} sampled by the app", if local < 0.01 { "<1%".into() } else { format!("{:.0}%", local * 100.0) })
        } else {
            "Prometheus".to_owned()
        };
        let gaps = s.gaps.len();
        Some(view! {
            <dl class="review-facts">
                <div><dt>"Context"</dt><dd>{s.context.clone()}</dd></div>
                <div><dt>"Traced"</dt><dd>{scope_text(&s.scope)}</dd></div>
                <div><dt>"Covers"</dt><dd>{span}</dd></div>
                <div><dt>"State"</dt><dd class:warn-text=s.unfinished>{state}</dd></div>
                <div><dt>"Data from"</dt><dd>{source}</dd></div>
                <div><dt>"Gaps"</dt><dd class:warn-text={gaps > 0}>{if gaps == 0 { "None".to_owned() } else { gaps.to_string() }}</dd></div>
            </dl>
        })
    };

    let namespaces = move || {
        store.review.with(|r| {
            let mut ns: Vec<String> = r.iter().flatten().flat_map(|r| r.rows.iter().map(|row| row.key.namespace.clone())).collect();
            ns.sort();
            ns.dedup();
            ns
        })
    };
    let rows = move || {
        store.review.with(|r| match r {
            Some(Ok(r)) => {
                let mut rows = filter_rows(&r.rows, &filter.get());
                let (by, desc) = sort.get();
                sort_rows(&mut rows, by, desc);
                rows
            }
            _ => vec![],
        })
    };
    let th = move |label: &'static str, by: SortBy, num: bool| {
        let active = move || sort.get().0 == by;
        let arrow = move || if !active() { "" } else if sort.get().1 { " ↓" } else { " ↑" };
        view! {
            <th class:num=num>
                <button class="th-sort" class:on=active on:click=move |_| sort.update(|(b, d)| {
                    if *b == by { *d = !*d } else { *b = by; *d = by != SortBy::Workload }
                })>{label}{arrow}</button>
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
                let mut shown: Vec<&ReviewFlag> = row.flags.iter().filter(|f| flag_level(f) != "info").collect();
                shown.sort_by_key(|f| severity(f));
                let more = shown.len().saturating_sub(2);
                let flags = shown
                    .into_iter()
                    .take(2)
                    .map(|f| view! { <span class=format!("flag {}", flag_level(f)) title=flag_explain(f)><span class="flag-dot" aria-hidden="true"></span>{flag_text(f)}</span> })
                    .collect_view();
                let more = (more > 0).then(|| view! { <span class="flag more" title="Select the row to see them all">{format!("+{more}")}</span> });
                let suggestion = match &row.suggestion {
                    Some(s) => format!("{} · {}", Measure::Cpu.fmt(s.cpu_request), Measure::Memory.fmt(s.mem_request)),
                    None => "—".into(),
                };
                view! {
                    <tr class:selected=selected on:click=move |_| store.review_sel.set(Some(key.clone()))>
                        <td>
                            <div class="review-name">{row.key.name.clone()}</div>
                            <div class="muted small">{format!("{} · {} · {}", row.key.kind, row.key.container, row.key.namespace)}</div>
                        </td>
                        <td class="num">{format!("{:.1}", row.avg_pods).trim_end_matches(".0").to_owned()}</td>
                        <td class="num">{opt(row.cpu_request, Measure::Cpu)}</td>
                        <td class="num">{Measure::Cpu.fmt(row.cpu_p95)}</td>
                        {spare_cell(row.cpu_spare, Measure::Cpu)}
                        <td class="num">{opt(row.mem_request, Measure::Memory)}</td>
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
                        {th("Workload", SortBy::Workload, false)}
                        {th("Pods", SortBy::Pods, true)}
                        {th("CPU request", SortBy::CpuRequest, true)}
                        {th("CPU p95", SortBy::CpuP95, true)}
                        {th("CPU spare", SortBy::CpuSpare, true)}
                        {th("Memory request", SortBy::MemRequest, true)}
                        {th("Memory peak", SortBy::MemPeak, true)}
                        {th("Memory spare", SortBy::MemSpare, true)}
                        <th class="num">"Suggested"</th>
                        <th>"Flags"</th>
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
                <button class="btn small" on:click=reveal>{crate::prefs::reveal_label()}</button>
            </div>
            {move || match store.review.get() {
                None => view! { <p class="muted pad">"Reading the trace…"</p> }.into_any(),
                Some(Err(e)) => view! { <div class="warn">{e}</div> }.into_any(),
                Some(Ok(_)) => view! {
                    {header}
                    <div class="review-filters">
                        <label class="field">
                            "Namespace"
                            <select on:change=move |ev| {
                                let v = event_target_value(&ev);
                                filter.update(|f| f.namespace = (!v.is_empty()).then_some(v));
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
                        <span class="muted small">{move || {
                            let n = rows().len();
                            let hint = if store.review_sel.with(Option::is_none) && n > 0 { " · select one for its charts" } else { "" };
                            format!("{}{hint}", plural(n as u64, "row", "rows"))
                        }}</span>
                    </div>
                    <div class="review-split">
                        <div class="review-table-wrap">{table}</div>
                        <ReviewDetail store=store />
                    </div>
                }.into_any(),
            }}
        </div>
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
        store.review.with(|r| r.as_ref()?.as_ref().ok()?.rows.iter().find(|row| row.key == sel).cloned())
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
            #[derive(Serialize)]
            struct Args {
                path: String,
                key: TraceKey,
                start: Option<f64>,
                end: Option<f64>,
                points: u32,
            }
            let args = Args { path, key: key.clone(), start: range.map(|r| r.0), end: range.map(|r| r.1), points: CHART_POINTS };
            let result = invoke::<_, TraceSeries>("trace_series", &args).await;
            if store.review_sel.get_untracked().as_ref() == Some(&key) && zoom.get_untracked() == range {
                series.set(Some(result));
            }
        });
    });
    let chart_data = Signal::derive(move || series.get().and_then(Result::ok));
    let on_zoom = Callback::new(move |(a, b): (f64, f64)| zoom.set(Some((a, b))));
    let quick = move |label: &'static str, hours: Option<f64>| {
        let active = move || match (hours, zoom.get(), chart_data.get()) {
            (None, None, _) => true,
            (Some(h), Some((a, b)), Some(s)) => (b - s.full_end).abs() < 1.0 && (b - a - h * 3600.0).abs() < 1.0,
            _ => false,
        };
        view! {
            <button class="btn small toggle" class:on=active on:click=move |_| {
                match (hours, chart_data.get_untracked()) {
                    (None, _) => zoom.set(None),
                    (Some(h), Some(s)) => zoom.set(Some(((s.full_end - h * 3600.0).max(s.full_start), s.full_end))),
                    _ => {}
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
                    let change = now.map(|n| change_text(n, to)).unwrap_or_default();
                    view! {
                        <div class=format!("suggest-line {}", m.class())>
                            <span class="res-dot" aria-hidden="true"></span>
                            <span class="suggest-what">{format!("{} request", m.label())}</span>
                            <span class="suggest-from">{opt(now, m)}</span>
                            <span class="muted">"→"</span>
                            <strong>{m.fmt(to)}</strong>
                            <span class="muted small">{change}</span>
                        </div>
                    }
                };
                view! {
                    <div class="suggest-card">
                        <h3>"Suggested requests"</h3>
                        {line(Measure::Cpu, row.cpu_request, s.cpu_request)}
                        {line(Measure::Memory, row.mem_request, s.mem_request)}
                        <p class="muted small">{format!("Based on {}.", s.basis)}</p>
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
                    <span class=format!("flag {}", flag_level(f))><span class="flag-dot" aria-hidden="true"></span>{flag_text(f)}</span>
                    <span>{flag_explain(f)}</span>
                </li>
            })
            .collect_view();
        let facts = format!(
            "{} seen, {:.1} running on average · {} of data · {}{}",
            plural(u64::from(row.pods), "pod", "pods"),
            row.avg_pods,
            duration_text(row.minutes as f64 * 60.0),
            plural(u64::from(row.rollouts), "rollout", "rollouts"),
            if row.local_share > 0.0 { format!(" · {:.0}% sampled by the app", row.local_share * 100.0) } else { String::new() },
        );
        let figures = view! {
            <table class="figures">
                <tr><th></th><th class="num">"Request"</th><th class="num">"Limit"</th><th class="num">"p50"</th><th class="num">"p95"</th><th class="num">"p99"</th><th class="num">"Peak"</th></tr>
                <tr class="res-cpu">
                    <th><span class="res-dot" aria-hidden="true"></span>"CPU"</th>
                    <td class="num">{opt(row.cpu_request, Measure::Cpu)}</td>
                    <td class="num">{opt(row.cpu_limit, Measure::Cpu)}</td>
                    <td class="num">{Measure::Cpu.fmt(row.cpu_p50)}</td>
                    <td class="num">{Measure::Cpu.fmt(row.cpu_p95)}</td>
                    <td class="num">{Measure::Cpu.fmt(row.cpu_p99)}</td>
                    <td class="num">{Measure::Cpu.fmt(row.cpu_max)}</td>
                </tr>
                <tr class="res-memory">
                    <th><span class="res-dot" aria-hidden="true"></span>"Memory"</th>
                    <td class="num">{opt(row.mem_request, Measure::Memory)}</td>
                    <td class="num">{opt(row.mem_limit, Measure::Memory)}</td>
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
                        <div class="muted small">{format!("{} · container {} · {}", row.key.kind, row.key.container, row.key.namespace)}</div>
                    </div>
                    <button class="close" title="Close" aria-label="Close" on:click=move |_| store.review_sel.set(None)>"×"</button>
                </div>
                {suggestion}
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

#[cfg(test)]
#[path = "trace_review_tests.rs"]
mod tests;
