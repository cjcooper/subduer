//! A trace's review as one self-contained HTML page, for people without the
//! app: the summary, headline totals, notes, the right-sizing table, and
//! CPU and memory charts for the rows that need attention or have a
//! suggestion. Light and dark follow the reader's system; charts show
//! values on hover. Names and numbers only, no secrets.

use std::fmt::Write as _;
use std::path::Path;

use crate::trace_review::{review, series};
use crate::types::{ReviewFlag, ReviewRow, Series, TraceReview, TraceScope, TraceSeries};
use crate::Result;

/// Rows charted, at most (the rest are in the table only).
pub const MAX_CHARTS: usize = 150;
/// Points per chart.
const CHART_POINTS: u32 = 160;
const W: f64 = 600.0;
const H: f64 = 120.0;

/// Builds the report. `tz` is used for dates; `generated` says when.
pub fn report_html(path: &Path, recording: bool, tz: &jiff::tz::TimeZone, generated: &str) -> Result<String> {
    let r = review(path, recording)?;
    let charted: Vec<&ReviewRow> = r.rows.iter().filter(|row| row.suggestion.is_some() || attention(row)).take(MAX_CHARTS).collect();
    let mut charts = Vec::new();
    for row in &charted {
        charts.push(series(path, &row.key, None, None, CHART_POINTS)?);
    }
    // A trace with no usage at all has no overview; the rest still reports.
    let overview = crate::trace_review::overview(path, None, None, None, CHART_POINTS).ok();
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(render(&r, &file, overview.as_ref(), &charted, &charts, tz, generated))
}

fn attention(row: &ReviewRow) -> bool {
    row.flags.iter().any(|f| !matches!(f, ReviewFlag::RequestsChanged | ReviewFlag::ShortData { .. } | ReviewFlag::TooLittleData { .. }))
        || row.cpu_spare.is_some_and(|s| s < 0.0)
        || row.mem_spare.is_some_and(|s| s < 0.0)
}

pub(crate) fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

pub(crate) fn fmt_cpu(m: f64) -> String {
    if m > 0.0 && m < 10.0 {
        format!("{m:.1}m")
    } else if m < 1000.0 {
        format!("{m:.0}m")
    } else {
        let c = m / 1000.0;
        if c.fract() == 0.0 { format!("{c:.0} core{}", if c == 1.0 { "" } else { "s" }) } else { format!("{c:.1} cores") }
    }
}

pub(crate) fn fmt_bytes(b: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let (mut v, mut u) = (b, 0);
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{v:.0} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

fn span_text(secs: f64) -> String {
    let mins = (secs.max(0.0) / 60.0) as u64;
    let (d, h, m) = (mins / 1440, mins / 60 % 24, mins % 60);
    match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h"),
    }
}

fn when(t: f64, tz: &jiff::tz::TimeZone) -> String {
    jiff::Timestamp::from_second(t as i64)
        .map(|ts| ts.to_zoned(tz.clone()).strftime("%a %-d %b %Y %H:%M").to_string())
        .unwrap_or_default()
}

/// `Mon 12 14:05`, for chart axes.
fn short_when(t: f64, tz: &jiff::tz::TimeZone) -> String {
    jiff::Timestamp::from_second(t as i64).map(|ts| ts.to_zoned(tz.clone()).strftime("%a %-d %H:%M").to_string()).unwrap_or_default()
}

fn scope_text(scope: &TraceScope) -> String {
    match scope {
        TraceScope::Cluster => "the whole cluster".into(),
        TraceScope::Namespaces(ns) if ns.len() == 1 => format!("namespace {}", ns[0]),
        TraceScope::Namespaces(ns) => format!("{} namespaces ({})", ns.len(), ns.join(", ")),
        TraceScope::Workloads(ws) if ws.len() == 1 => format!("{} {}/{}", ws[0].kind.kind, ws[0].namespace, ws[0].name),
        TraceScope::Workloads(ws) => format!("{} workloads", ws.len()),
        TraceScope::Pod { namespace, name } => format!("pod {namespace}/{name}"),
    }
}

/// (text, severity) for a flag, as the app shows it.
fn flag(f: &ReviewFlag) -> (String, &'static str) {
    let pct = |v: f64| format!("{:.0}%", v * 100.0);
    match f {
        ReviewFlag::Throttled { worst } => (format!("Throttled {}", pct(*worst)), "serious"),
        ReviewFlag::NearMemoryLimit { share } => (format!("Memory {} of limit", pct(*share)), "serious"),
        ReviewFlag::OomKilled { count } => (format!("OOMKilled ×{count}"), "critical"),
        ReviewFlag::OomEvents { count } => (format!("OOM ×{count}"), "critical"),
        ReviewFlag::Restarts { count } => (format!("Restarts ×{count}"), "warning"),
        ReviewFlag::NoCpuRequest => ("No CPU request".into(), "warning"),
        ReviewFlag::NoMemoryRequest => ("No memory request".into(), "warning"),
        ReviewFlag::RequestsChanged => ("Requests changed".into(), "info"),
        ReviewFlag::ShortData { hours } => (format!("{} of data", span_text(hours * 3600.0)), "info"),
        ReviewFlag::TooLittleData { hours } => (format!("Only {} of data", span_text(hours * 3600.0)), "info"),
        ReviewFlag::AboveMemoryLimit => ("Above the memory limit".into(), "serious"),
    }
}

fn opt(v: Option<f64>, f: fn(f64) -> String) -> String {
    v.map_or("—".into(), f)
}

fn spare(v: Option<f64>, f: fn(f64) -> String) -> String {
    match v {
        None => "<td class=\"num muted\">—</td>".into(),
        Some(v) if v < 0.0 => format!("<td class=\"num short\">short {}</td>", esc(&f(-v))),
        Some(v) => format!("<td class=\"num\">{}</td>", esc(&f(v))),
    }
}

/// Smallest 1/2/2.5/5 × 10ⁿ at or above `v`.
fn nice(v: f64) -> f64 {
    if v <= 0.0 || !v.is_finite() {
        return 1.0;
    }
    let mag = 10f64.powf(v.log10().floor());
    [1.0, 1.2, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0].into_iter().map(|m| m * mag).find(|x| *x >= v * 0.9999).unwrap_or(10.0 * mag)
}

/// One measure's chart as inline SVG, with its data for the hover script.
/// One measure of a series as an SVG chart with hover data. `totals`: the
/// series is every container added up (the overview), so its lines are used,
/// busiest minute and requested, and hover shows what's idle.
fn chart(s: &TraceSeries, cpu: bool, label: &str, totals: bool, tz: &jiff::tz::TimeZone) -> String {
    let (avg, peak, req, lim): (&Series, &Series, &Series, &Series) =
        if cpu { (&s.cpu_avg, &s.cpu_max, &s.cpu_request, &s.cpu_limit) } else { (&s.mem_avg, &s.mem_max, &s.mem_request, &s.mem_limit) };
    let span = (s.end - s.start).max(1.0);
    let steps = |c: &Series| -> Vec<(f64, f64, f64)> {
        c.iter()
            .enumerate()
            .map(|(i, &(t, v))| (t.max(s.start), c.get(i + 1).map_or(s.end, |n| n.0).min(s.end), v))
            .filter(|(a, b, _)| b > a)
            .collect()
    };
    let (req_runs, lim_runs) = (steps(req), steps(lim));
    let top = nice(peak.iter().chain(avg.iter()).map(|p| p.1).chain(req_runs.iter().chain(&lim_runs).map(|r| r.2)).fold(0.0, f64::max) * 1.05);
    let x = |t: f64| (t - s.start) / span * W;
    let y = |v: f64| H - 4.0 - (v / top).clamp(0.0, 1.0) * (H - 14.0);
    let line = |pts: &Series| -> String {
        let mut d = String::new();
        let mut prev: Option<f64> = None;
        for &(t, v) in pts {
            let cmd = if prev.is_none_or(|p| t - p > s.step * 1.5) { 'M' } else { 'L' };
            let _ = write!(d, "{cmd}{:.1},{:.1}", x(t + s.step / 2.0), y(v));
            prev = Some(t);
        }
        d
    };
    let area = {
        let mut d = String::new();
        let mut seg: Vec<(f64, f64)> = vec![];
        let flush = |seg: &mut Vec<(f64, f64)>, d: &mut String| {
            if let (Some(first), Some(last)) = (seg.first(), seg.last()) {
                let _ = write!(d, "M{:.1},{:.1}", first.0, H - 4.0);
                for (px, py) in seg.iter() {
                    let _ = write!(d, "L{px:.1},{py:.1}");
                }
                let _ = write!(d, "L{:.1},{:.1}Z", last.0, H - 4.0);
            }
            seg.clear();
        };
        let mut prev: Option<f64> = None;
        for &(t, v) in avg {
            if prev.is_some_and(|p| t - p > s.step * 1.5) {
                flush(&mut seg, &mut d);
            }
            seg.push((x(t + s.step / 2.0), y(v)));
            prev = Some(t);
        }
        flush(&mut seg, &mut d);
        d
    };
    let runs = |rs: &[(f64, f64, f64)]| rs.iter().map(|&(a, b, v)| format!("M{:.1},{:.1}H{:.1}", x(a), y(v), x(b))).collect::<String>();
    let gaps: String = s
        .gaps
        .iter()
        .map(|g| format!("<rect class=\"gap\" x=\"{:.1}\" y=\"0\" width=\"{:.1}\" height=\"{H}\"/>", x(g.start.max(s.start)), (x(g.end.min(s.end)) - x(g.start.max(s.start))).max(2.0)))
        .collect();
    // Data for hover: [t, average, peak, request, limit] per bucket.
    let at = |c: &Series, t: f64| c.iter().take_while(|p| p.0 <= t).last().map(|p| p.1);
    let data: Vec<String> = avg
        .iter()
        .map(|&(t, a)| {
            let p = peak.iter().find(|q| q.0 == t).map_or("null".into(), |q| format!("{:.1}", q.1));
            let r = at(req, t).map_or("null".into(), |v| format!("{v:.1}"));
            let l = at(lim, t).map_or("null".into(), |v| format!("{v:.1}"));
            format!("[{t:.0},{a:.1},{p},{r},{l}]")
        })
        .collect();
    let fmt: fn(f64) -> String = if cpu { fmt_cpu } else { fmt_bytes };
    let cls = if cpu { "cpu" } else { "mem" };
    let (avg_name, peak_name, totals_attr) = if totals { ("used", "busiest minute", " data-totals=\"1\"") } else { ("average pod", "busiest pod", "") };
    format!(
        r#"<figure class="chart {cls}"><figcaption>{label} <span class="muted">· top {top_text}</span></figcaption><div class="plot" data-start="{start}" data-end="{end}" data-step="{step}" data-kind="{cls}"{totals_attr} data-points='[{data}]'><svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" role="img" aria-label="{label}: {avg_name} up to {avg_max}, {peak_name} up to {peak_max}"><line class="grid" x1="0" x2="{W}" y1="{ytop:.1}" y2="{ytop:.1}"/><line class="grid" x1="0" x2="{W}" y1="{ymid:.1}" y2="{ymid:.1}"/>{gaps}<path class="area" d="{area}"/><path class="peak" d="{peak_d}"/><path class="avg" d="{avg_d}"/><path class="req" d="{req_d}"/><path class="lim" d="{lim_d}"/></svg><div class="tip" hidden></div></div><div class="xaxis"><span>{x0}</span><span>{x1}</span><span>{x2}</span></div></figure>"#,
        x0 = esc(&short_when(s.start, tz)),
        x1 = esc(&short_when((s.start + s.end) / 2.0, tz)),
        x2 = esc(&short_when(s.end, tz)),
        top_text = esc(&fmt(top)),
        start = s.start,
        end = s.end,
        step = s.step,
        data = data.join(","),
        avg_max = esc(&fmt(avg.iter().map(|p| p.1).fold(0.0, f64::max))),
        peak_max = esc(&fmt(peak.iter().map(|p| p.1).fold(0.0, f64::max))),
        ytop = y(top),
        ymid = y(top / 2.0),
        area = area,
        peak_d = line(peak),
        avg_d = line(avg),
        req_d = runs(&req_runs),
        lim_d = runs(&lim_runs),
    )
}

fn render(
    r: &TraceReview,
    file: &str,
    overview: Option<&TraceSeries>,
    charted: &[&ReviewRow],
    charts: &[TraceSeries],
    tz: &jiff::tz::TimeZone,
    generated: &str,
) -> String {
    let s = &r.summary;
    let span = match (s.data_start, s.data_end) {
        (Some(a), Some(b)) => format!("{} ({} → {})", span_text(b - a), when(a, tz), when(b, tz)),
        _ => "no data".into(),
    };
    let cpu_spare: f64 = r.rows.iter().filter_map(|row| row.cpu_spare).filter(|v| *v > 0.0).sum();
    let mem_spare: f64 = r.rows.iter().filter_map(|row| row.mem_spare).filter(|v| *v > 0.0).sum();
    let short = r.rows.iter().filter(|row| row.cpu_spare.is_some_and(|v| v < 0.0) || row.mem_spare.is_some_and(|v| v < 0.0)).count();
    let suggested = r.rows.iter().filter(|row| row.suggestion.is_some()).count();
    let ooms: u32 = r
        .rows
        .iter()
        .flat_map(|row| &row.flags)
        .map(|f| match f {
            ReviewFlag::OomKilled { count } | ReviewFlag::OomEvents { count } => *count,
            _ => 0,
        })
        .sum();

    let mut html = String::new();
    let _ = write!(
        html,
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Trace report · {title}</title><style>{CSS}</style></head><body><main>
<header><h1>Trace report</h1><p class="muted">{file_e} · generated {generated}</p></header>
<dl class="facts"><div><dt>Context</dt><dd>{ctx}</dd></div><div><dt>Traced</dt><dd>{scope}</dd></div><div><dt>Data</dt><dd>{span}</dd></div><div><dt>Source</dt><dd>{source}</dd></div><div><dt>Gaps</dt><dd>{gaps}</dd></div></dl>"#,
        title = esc(&s.context),
        file_e = esc(file),
        generated = esc(generated),
        ctx = esc(&s.context),
        scope = esc(&scope_text(&s.scope)),
        span = esc(&span),
        source = if s.prometheus.is_some() { "Prometheus, plus the app's own samples where it had none" } else { "Sampled by the app (no Prometheus)" },
        gaps = if s.gaps.is_empty() { "None".to_owned() } else { format!("{} ({} in all)", s.gaps.len(), span_text(s.gaps.iter().map(|g| g.end - g.start).sum())) },
    );
    if let Some(note) = &r.note {
        let _ = write!(html, "<section class=\"note\"><h2>Note</h2><p>{}</p></section>", esc(note).replace('\n', "<br>"));
    }
    let _ = write!(
        html,
        r#"<section class="tiles"><div class="tile"><span class="big">{}</span><span class="muted">CPU requested but unused (p95)</span></div><div class="tile"><span class="big">{}</span><span class="muted">memory requested but unused (peak)</span></div><div class="tile"><span class="big">{short}</span><span class="muted">rows using more than they request</span></div><div class="tile"><span class="big">{ooms}</span><span class="muted">OOM kills</span></div><div class="tile"><span class="big">{suggested}</span><span class="muted">of {} rows with a suggestion</span></div></section>"#,
        esc(&fmt_cpu(cpu_spare)),
        esc(&fmt_bytes(mem_spare)),
        r.rows.len(),
    );
    if let Some(o) = overview {
        let _ = write!(
            html,
            "<section><h2>Overview</h2><p class=\"muted\">Every container added up: what's used against what's requested. <span class=\"key avg\"></span>used <span class=\"key peak\"></span>busiest minute <span class=\"key req\"></span>requested <span class=\"key gap\"></span>no data. Hover for values and what's idle.</p><div class=\"charts\">{}{}</div></section>",
            chart(o, true, "CPU", true, tz),
            chart(o, false, "Memory", true, tz),
        );
    }
    html.push_str(
        r#"<section><h2>Right-sizing</h2><p class="muted">Per workload container. Percentiles are over every pod-minute; peaks are the highest any pod reached. Spare = (request − p95 CPU or peak memory) × average pods. Suggested: CPU = p95, memory = peak × 1.15, rounded up; none under 6 hours of data.</p><div class="scroll"><table><thead><tr><th>Workload</th><th class="num">Pods</th><th class="num">CPU request</th><th class="num">CPU p95</th><th class="num">CPU spare</th><th class="num">Memory request</th><th class="num">Memory peak</th><th class="num">Memory spare</th><th class="num">Suggested</th><th>Flags</th></tr></thead><tbody>"#,
    );
    for row in &r.rows {
        let flags: String = row
            .flags
            .iter()
            .map(flag)
            .map(|(t, level)| format!("<span class=\"flag {level}\"><i></i>{}</span>", esc(&t)))
            .collect();
        let note = row.note.as_ref().map(|n| format!("<div class=\"rownote\">{}</div>", esc(n))).unwrap_or_default();
        let _ = write!(
            html,
            "<tr><td><strong>{}</strong><div class=\"muted small\">{} · {} · {}</div>{note}</td><td class=\"num\">{:.1}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>{}<td class=\"num\">{}</td><td class=\"num\">{}</td>{}<td class=\"num\">{}</td><td>{flags}</td></tr>",
            esc(&row.key.name),
            esc(&row.key.kind),
            esc(&row.key.container),
            esc(&row.key.namespace),
            row.avg_pods,
            esc(&opt(row.cpu_request, fmt_cpu)),
            esc(&fmt_cpu(row.cpu_p95)),
            spare(row.cpu_spare, fmt_cpu),
            esc(&opt(row.mem_request, fmt_bytes)),
            esc(&fmt_bytes(row.mem_max)),
            spare(row.mem_spare, fmt_bytes),
            row.suggestion.as_ref().map_or("—".into(), |s| esc(&format!("{} · {}", fmt_cpu(s.cpu_request), fmt_bytes(s.mem_request)))),
        );
    }
    html.push_str("</tbody></table></div></section>");

    if !charted.is_empty() {
        let more = r.rows.iter().filter(|row| row.suggestion.is_some() || attention(row)).count();
        let _ = write!(
            html,
            "<section><h2>Over the trace</h2><p class=\"muted\">Rows with a suggestion or needing attention{}. <span class=\"key avg\"></span>average pod <span class=\"key peak\"></span>busiest pod <span class=\"key req\"></span>request <span class=\"key lim\"></span>limit <span class=\"key gap\"></span>no data. Hover for values.</p>",
            if more > charted.len() { format!(" (the first {} of {more})", charted.len()) } else { String::new() },
        );
        for (row, s) in charted.iter().zip(charts) {
            let suggestion = row
                .suggestion
                .as_ref()
                .map(|sg| {
                    format!(
                        "<p><strong>Suggested:</strong> CPU {} → {}, memory {} → {}. <span class=\"muted\">Based on {}.</span></p>",
                        esc(&opt(row.cpu_request, fmt_cpu)),
                        esc(&fmt_cpu(sg.cpu_request)),
                        esc(&opt(row.mem_request, fmt_bytes)),
                        esc(&fmt_bytes(sg.mem_request)),
                        esc(&sg.basis)
                    )
                })
                .unwrap_or_default();
            let _ = write!(
                html,
                "<article class=\"card\"><h3>{} <span class=\"muted\">{} · {} · {}</span></h3>{suggestion}{}<div class=\"charts\">{}{}</div></article>",
                esc(&row.key.name),
                esc(&row.key.kind),
                esc(&row.key.container),
                esc(&row.key.namespace),
                row.note.as_ref().map(|n| format!("<p class=\"rownote\">{}</p>", esc(n))).unwrap_or_default(),
                chart(s, true, "CPU", false, tz),
                chart(s, false, "Memory", false, tz),
            );
        }
        html.push_str("</section>");
    }
    let _ = write!(html, "<footer class=\"muted small\">Made by k8s-subduer from {}. Times are {}.</footer></main><script>{JS}</script></body></html>", esc(file), esc(tz.iana_name().unwrap_or("local time")));
    html
}

const CSS: &str = r#"
:root { --bg:#f6f7f9; --panel:#fff; --border:#dfe3e8; --text:#1c2430; --muted:#6b7686; --cpu:#2563eb; --mem:#9333ea; --danger:#c6362c;
  --critical:#d03b3b; --serious:#ec835a; --warning:#fab219; --chip:#f1f4f8; color-scheme: light dark; }
@media (prefers-color-scheme: dark) { :root { --bg:#16191e; --panel:#1d2128; --border:#2d333c; --text:#e3e7ed; --muted:#8b95a5;
  --cpu:#60a5fa; --mem:#c084fc; --danger:#ef6a5f; --chip:#252a32; } }
* { box-sizing: border-box; }
body { margin:0; background:var(--bg); color:var(--text); font:14px/1.45 -apple-system,BlinkMacSystemFont,"Segoe UI",system-ui,sans-serif; }
main { max-width:1180px; margin:0 auto; padding:24px 16px 48px; }
h1 { margin:0; font-size:24px; } h2 { font-size:17px; margin:28px 0 8px; } h3 { font-size:15px; margin:0 0 6px; }
.muted { color:var(--muted); } .small { font-size:12px; }
.facts { display:flex; flex-wrap:wrap; gap:8px 28px; margin:16px 0; } .facts dt { color:var(--muted); font-size:12px; } .facts dd { margin:0; }
.note, .card, .tile { background:var(--panel); border:1px solid var(--border); border-radius:10px; }
.note { padding:4px 16px 8px; }
.tiles { display:grid; grid-template-columns:repeat(auto-fit,minmax(170px,1fr)); gap:10px; margin:16px 0; }
.tile { padding:12px 14px; display:flex; flex-direction:column; gap:2px; } .big { font-size:22px; font-weight:600; font-variant-numeric:tabular-nums; }
.scroll { overflow-x:auto; background:var(--panel); border:1px solid var(--border); border-radius:10px; }
table { width:100%; border-collapse:collapse; font-variant-numeric:tabular-nums; }
th, td { padding:6px 10px; border-bottom:1px solid var(--border); text-align:left; vertical-align:top; white-space:nowrap; }
th { color:var(--muted); font-weight:600; font-size:12px; position:sticky; top:0; background:var(--panel); }
.num { text-align:right; } td.short { color:var(--danger); } td:last-child { white-space:normal; min-width:150px; } td:first-child { white-space:normal; min-width:180px; }
.xaxis { display:flex; justify-content:space-between; font-size:11px; color:var(--muted); margin-top:3px; }
.flag { display:inline-flex; align-items:center; gap:5px; margin:0 4px 3px 0; padding:1px 7px; border-radius:9px; font-size:12px; background:var(--chip); }
.flag i { width:7px; height:7px; border-radius:50%; background:var(--muted); } .flag.critical i { background:var(--critical); }
.flag.serious i { background:var(--serious); } .flag.warning i { background:var(--warning); }
.rownote { margin-top:4px; padding:4px 8px; border-left:3px solid var(--border); white-space:normal; font-size:13px; }
.card { padding:14px 16px; margin:12px 0; } .card p { margin:4px 0; }
.charts { display:grid; grid-template-columns:repeat(auto-fit,minmax(320px,1fr)); gap:14px; margin-top:8px; }
figure { margin:0; } figcaption { font-size:13px; font-weight:600; margin-bottom:4px; }
.chart.cpu { --series:var(--cpu); } .chart.mem { --series:var(--mem); }
.plot { position:relative; height:130px; } .plot svg { width:100%; height:100%; display:block; overflow:visible; }
.grid { stroke:var(--border); stroke-width:1; vector-effect:non-scaling-stroke; }
.area { fill:var(--series); opacity:.12; } .avg { fill:none; stroke:var(--series); stroke-width:2; vector-effect:non-scaling-stroke; }
.peak { fill:none; stroke:var(--series); stroke-width:1.5; opacity:.45; vector-effect:non-scaling-stroke; }
.req { fill:none; stroke:var(--text); stroke-width:1.5; stroke-dasharray:5 4; opacity:.7; vector-effect:non-scaling-stroke; }
.lim { fill:none; stroke:var(--text); stroke-width:1.5; opacity:.7; vector-effect:non-scaling-stroke; }
.gap { fill:var(--muted); opacity:.15; }
.tip { position:absolute; top:4px; pointer-events:none; padding:5px 8px; border-radius:6px; font-size:12px; white-space:nowrap;
  background:var(--panel); border:1px solid var(--border); box-shadow:0 4px 14px rgb(0 0 0 / .15); font-variant-numeric:tabular-nums; }
.cross { position:absolute; top:0; bottom:0; width:1px; background:var(--muted); pointer-events:none; }
.key { display:inline-block; width:16px; margin:0 4px 3px 10px; border-top:2px solid var(--cpu); vertical-align:middle; }
.key.peak { opacity:.45; } .key.req { border-top:1.5px dashed var(--text); } .key.lim { border-top:1.5px solid var(--text); }
.key.gap { height:10px; border:none; background:var(--muted); opacity:.3; }
footer { margin-top:32px; }
"#;

const JS: &str = r#"
const cpu = (m) => m < 10 ? m.toFixed(1) + 'm' : m < 1000 ? Math.round(m) + 'm' : (m / 1000).toFixed(1) + ' cores';
const mem = (b) => { const u = ['B', 'KiB', 'MiB', 'GiB', 'TiB']; let i = 0; while (b >= 1024 && i < 4) { b /= 1024; i++; } return i ? b.toFixed(1) + ' ' + u[i] : Math.round(b) + ' B'; };
for (const plot of document.querySelectorAll('.plot')) {
  const pts = JSON.parse(plot.dataset.points), start = +plot.dataset.start, end = +plot.dataset.end, step = +plot.dataset.step;
  const fmt = plot.dataset.kind === 'cpu' ? cpu : mem, tip = plot.querySelector('.tip');
  const cross = document.createElement('div'); cross.className = 'cross'; cross.hidden = true; plot.appendChild(cross);
  plot.addEventListener('pointermove', (e) => {
    const r = plot.getBoundingClientRect(), f = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)), t = start + f * (end - start);
    let best = null; for (const p of pts) if (!best || Math.abs(p[0] + step / 2 - t) < Math.abs(best[0] + step / 2 - t)) best = p;
    if (!best) return;
    const x = (best[0] + step / 2 - start) / (end - start) * 100;
    const row = (name, v, extra = '') => v == null ? '' : `<div><span style="color:var(--muted)">${name}</span> ${fmt(v)}${extra}</div>`;
    const [avg, peak, req] = plot.dataset.totals ? ['used', 'busiest minute', 'requested'] : ['average pod', 'busiest pod', 'request'];
    // Totals: what's requested but idle, never rounded up to 100% while some is used.
    const share = (s) => s < 1 && s >= 0.995 ? 'over 99%' : Math.round(s * 100) + '%';
    const idle = plot.dataset.totals && best[3] > 0 && best[1] < best[3] ? row('idle', best[3] - best[1], ` (${share((best[3] - best[1]) / best[3])} of requested)`) : '';
    tip.innerHTML = `<div style="color:var(--muted)">${new Date(best[0] * 1000).toLocaleString()}</div>` + row(avg, best[1]) + row(peak, best[2]) + row(req, best[3]) + idle + row('limit', best[4]);
    tip.hidden = false; cross.hidden = false; cross.style.left = x + '%';
    tip.style.left = x > 60 ? '' : `calc(${x}% + 8px)`; tip.style.right = x > 60 ? `calc(${100 - x}% + 8px)` : '';
  });
  plot.addEventListener('pointerleave', () => { tip.hidden = true; cross.hidden = true; });
}
"#;

#[cfg(test)]
#[path = "trace_report_tests.rs"]
mod tests;
