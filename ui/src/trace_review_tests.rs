use super::*;
use subduer_types::Suggestion;

fn row(name: &str, ns: &str, cpu_spare: Option<f64>, flags: Vec<ReviewFlag>) -> ReviewRow {
    ReviewRow {
        key: TraceKey { namespace: ns.into(), kind: "Deployment".into(), name: name.into(), container: "app".into() },
        pods: 2,
        avg_pods: 2.0,
        minutes: 600,
        pod_minutes: 1200,
        rollouts: 0,
        local_share: 0.0,
        cpu_request: cpu_spare.map(|s| s + 10.0),
        cpu_limit: None,
        mem_request: None,
        mem_limit: None,
        cpu_p50: 5.0,
        cpu_p95: 10.0,
        cpu_p99: 12.0,
        cpu_max: 20.0,
        mem_p95: 1.0,
        mem_max: 2.0,
        cpu_spare,
        mem_spare: None,
        flags,
        suggestion: Some(Suggestion { cpu_request: 10.0, mem_request: 16.0, basis: String::new() }),
        note: None,
    }
}

#[test]
fn sorting_puts_missing_values_last() {
    let mut rows = vec![row("b", "shop", Some(5.0), vec![]), row("a", "shop", None, vec![]), row("c", "ops", Some(50.0), vec![])];
    sort_rows(&mut rows, SortBy::CpuSpare, true);
    assert_eq!(rows.iter().map(|r| r.key.name.as_str()).collect::<Vec<_>>(), ["c", "b", "a"]);
    sort_rows(&mut rows, SortBy::CpuSpare, false);
    assert_eq!(rows.iter().map(|r| r.key.name.as_str()).collect::<Vec<_>>(), ["b", "c", "a"]);
    sort_rows(&mut rows, SortBy::Workload, false);
    assert_eq!(rows.iter().map(|r| r.key.name.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
    sort_rows(&mut rows, SortBy::Workload, true);
    assert_eq!(rows[0].key.name, "c");
    for by in [SortBy::Pods, SortBy::CpuRequest, SortBy::CpuP95, SortBy::MemRequest, SortBy::MemPeak, SortBy::MemSpare] {
        sort_rows(&mut rows, by, true);
    }
}

#[test]
fn filtering() {
    let rows = vec![
        row("web", "shop", Some(5.0), vec![ReviewFlag::ShortData { hours: 10.0 }]),
        row("db", "shop", Some(-5.0), vec![]),
        row("api", "ops", Some(5.0), vec![ReviewFlag::OomKilled { count: 1 }]),
    ];
    let names = |f: Filter| filter_rows(&rows, &f).into_iter().map(|r| r.key.name).collect::<Vec<_>>();
    assert_eq!(names(Filter::default()), ["web", "db", "api"]);
    assert_eq!(names(Filter { namespace: Some("ops".into()), ..Filter::default() }), ["api"]);
    assert_eq!(names(Filter { text: " WE ".into(), ..Filter::default() }), ["web"]);
    // Attention: a non-info flag, or usage above the request.
    assert_eq!(names(Filter { flagged_only: true, ..Filter::default() }), ["db", "api"]);
}

#[test]
fn flags_read_well() {
    let all = [
        (ReviewFlag::Throttled { worst: 0.38 }, "Throttled 38%", "serious"),
        (ReviewFlag::NearMemoryLimit { share: 0.95 }, "Memory 95% of limit", "serious"),
        (ReviewFlag::OomKilled { count: 2 }, "OOMKilled ×2", "critical"),
        (ReviewFlag::OomEvents { count: 1 }, "OOM ×1", "critical"),
        (ReviewFlag::Restarts { count: 3 }, "Restarts ×3", "warning"),
        (ReviewFlag::NoCpuRequest, "No CPU request", "warning"),
        (ReviewFlag::NoMemoryRequest, "No memory request", "warning"),
        (ReviewFlag::RequestsChanged, "Requests changed", "info"),
        (ReviewFlag::ShortData { hours: 8.0 }, "8h 0m of data", "info"),
        (ReviewFlag::TooLittleData { hours: 2.1 }, "Only 2h 6m of data", "info"),
        (ReviewFlag::AboveMemoryLimit, "Above the memory limit", "serious"),
    ];
    for (flag, text, level) in all {
        assert_eq!(flag_text(&flag), text);
        assert_eq!(flag_level(&flag), level);
        assert!(flag_explain(&flag).ends_with('.'), "{flag:?}");
    }
}

#[test]
fn small_texts() {
    assert_eq!(change_text(1500.0, 15.0), "−99%");
    assert_eq!(change_text(100.0, 112.0), "+12%");
    assert_eq!(change_text(100.0, 100.2), "no change");
    assert_eq!(change_text(0.0, 5.0), "");
    assert_eq!(file_name("/Users/me/Documents/a.trace"), "a.trace");
    assert_eq!(file_name(r"C:\Users\me\a.trace"), "a.trace");
    assert_eq!(file_name("a.trace"), "a.trace");
}

#[test]
fn plurals() {
    assert_eq!(plural(1, "pod", "pods"), "1 pod");
    assert_eq!(plural(0, "rollout", "rollouts"), "0 rollouts");
    assert_eq!(severity(&ReviewFlag::OomKilled { count: 1 }), 0);
    assert_eq!(severity(&ReviewFlag::RequestsChanged), 3);
}
