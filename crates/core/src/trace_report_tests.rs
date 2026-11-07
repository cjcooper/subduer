use super::*;
use crate::trace_review::tests::{sample, scratch};

#[test]
fn report_is_one_page_with_table_charts_and_notes() {
    let dir = scratch("report");
    let path = sample(&dir, 8 * 60);
    crate::trace_file::set_note(&path, "", "Before <the> resize & after", 1.0).unwrap();
    crate::trace_file::set_note(&path, &crate::trace_file::note_key("shop", "StatefulSet", "db"), "Needs more memory", 1.0).unwrap();
    let html = report_html(&path, false, &jiff::tz::TimeZone::UTC, "Sun 11 Oct 2026 12:00").unwrap();
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("<title>Trace report · acme-uat</title>"));
    assert!(html.contains("Before &lt;the&gt; resize &amp; after"), "notes are escaped");
    assert!(html.contains("Needs more memory"));
    assert!(html.contains("namespace shop"));
    assert!(html.contains("OOMKilled ×2") && html.contains("Throttled 20%"));
    assert!(html.contains("rows using more than they request"));
    // The overview (CPU and memory), then both rows, which have suggestions: two charts each.
    assert_eq!(html.matches("<figure class=\"chart").count(), 6);
    assert_eq!(html.matches("data-totals=\"1\"").count(), 2);
    assert!(html.find("<h2>Overview</h2>").unwrap() < html.find("<h2>Right-sizing</h2>").unwrap());
    assert!(html.contains("CPU: used up to") && html.contains("CPU: average pod up to"));
    assert!(html.contains("data-kind=\"cpu\""));
    assert!(html.contains("Times are UTC."));
    assert!(!html.contains("<script src"), "self-contained");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_trace_without_usage_still_reports() {
    let dir = scratch("report-empty");
    let path = dir.join("empty.trace");
    let file = crate::trace_file::TraceFile::create(&path, false, "acme-uat", &TraceScope::Cluster, 0.0).unwrap();
    file.finish(60.0, crate::types::TraceEnd::Stopped).unwrap();
    drop(file);
    let html = report_html(&path, false, &jiff::tz::TimeZone::UTC, "now").unwrap();
    assert!(html.contains("<dd>the whole cluster</dd>") && html.contains("<dd>no data</dd>"));
    assert!(!html.contains("<h2>Overview</h2>") && !html.contains("<h2>Over the trace</h2>"));
    assert!(html.contains("of 0 rows with a suggestion"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn rows_needing_attention() {
    let dir = scratch("report-attention");
    let path = sample(&dir, 8 * 60);
    let mut row = review(&path, false).unwrap().rows.remove(0);
    row.suggestion = None;
    row.flags = vec![ReviewFlag::RequestsChanged, ReviewFlag::ShortData { hours: 1.0 }, ReviewFlag::TooLittleData { hours: 0.5 }];
    (row.cpu_spare, row.mem_spare) = (Some(10.0), Some(5.0));
    assert!(!attention(&row), "informational flags and spare capacity don't");
    row.mem_spare = Some(-1.0);
    assert!(attention(&row), "using more memory than requested");
    (row.cpu_spare, row.mem_spare) = (Some(-1.0), None);
    assert!(attention(&row), "using more CPU than requested");
    (row.cpu_spare, row.flags) = (None, vec![ReviewFlag::NoMemoryRequest]);
    assert!(attention(&row), "a warning flag");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn charts_break_at_missing_minutes() {
    let dir = scratch("report-gaps");
    let path = sample(&dir, 8 * 60);
    let key = review(&path, false).unwrap().rows[0].key.clone();
    let mut s = series(&path, &key, None, None, CHART_POINTS).unwrap();
    // Two runs of CPU with a hole between them, and no memory at all.
    let n = s.cpu_avg.len();
    s.cpu_avg.drain(n / 3..2 * n / 3);
    s.mem_avg.clear();
    let cpu = chart(&s, true, "CPU", false, &jiff::tz::TimeZone::UTC);
    let area = cpu.split("<path class=\"area\" d=\"").nth(1).unwrap().split('"').next().unwrap();
    assert_eq!(area.matches('M').count(), 2, "one filled area per run: {area}");
    assert_eq!(area.matches('Z').count(), 2);
    let memory = chart(&s, false, "Memory", false, &jiff::tz::TimeZone::UTC);
    assert!(memory.contains("<path class=\"area\" d=\"\"/>"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn small_helpers() {
    assert_eq!(esc("<a href='x'>&\"</a>"), "&lt;a href=&#39;x&#39;&gt;&amp;&quot;&lt;/a&gt;");
    assert_eq!((fmt_cpu(5.0), fmt_cpu(15.0), fmt_cpu(1000.0), fmt_cpu(1500.0)), ("5.0m".into(), "15m".into(), "1 core".into(), "1.5 cores".into()));
    assert_eq!((fmt_bytes(512.0), fmt_bytes(1536.0 * 1024.0 * 1024.0)), ("512 B".into(), "1.5 GiB".into()));
    assert_eq!((nice(0.0), nice(7.0), nice(1100.0)), (1.0, 8.0, 1200.0));
    assert_eq!(span_text(26.0 * 3600.0), "1d 2h");
    assert_eq!(flag(&ReviewFlag::NoCpuRequest), ("No CPU request".into(), "warning"));
    assert_eq!(scope_text(&TraceScope::Pod { namespace: "shop".into(), name: "a".into() }), "pod shop/a");
    assert_eq!(scope_text(&TraceScope::Namespaces(vec!["a".into(), "b".into()])), "2 namespaces (a, b)");
    let w = |name: &str| crate::types::TraceWorkload {
        kind: crate::testing::kind("apps", "v1", "Deployment", "deployments", true),
        namespace: "shop".into(),
        name: name.into(),
    };
    assert_eq!(scope_text(&TraceScope::Workloads(vec![w("web")])), "Deployment shop/web");
    assert_eq!(scope_text(&TraceScope::Workloads(vec![w("web"), w("api")])), "2 workloads");
    assert_eq!(flag(&ReviewFlag::TooLittleData { hours: 0.5 }), ("Only 30m of data".into(), "info"));
    assert_eq!(span_text(90.0 * 60.0), "1h 30m");
}
