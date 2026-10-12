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
fn small_helpers() {
    assert_eq!(esc("<a href='x'>&\"</a>"), "&lt;a href=&#39;x&#39;&gt;&amp;&quot;&lt;/a&gt;");
    assert_eq!((fmt_cpu(5.0), fmt_cpu(15.0), fmt_cpu(1000.0), fmt_cpu(1500.0)), ("5.0m".into(), "15m".into(), "1 core".into(), "1.5 cores".into()));
    assert_eq!((fmt_bytes(512.0), fmt_bytes(1536.0 * 1024.0 * 1024.0)), ("512 B".into(), "1.5 GiB".into()));
    assert_eq!((nice(0.0), nice(7.0), nice(1100.0)), (1.0, 8.0, 1200.0));
    assert_eq!(span_text(26.0 * 3600.0), "1d 2h");
    assert_eq!(flag(&ReviewFlag::NoCpuRequest), ("No CPU request".into(), "warning"));
    assert_eq!(scope_text(&TraceScope::Pod { namespace: "shop".into(), name: "a".into() }), "pod shop/a");
}
