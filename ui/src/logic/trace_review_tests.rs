use super::*;
use subduer_types::{Suggestion, TraceScope};

fn summary() -> TraceSummary {
    TraceSummary {
        path: "/tmp/a.trace".into(),
        context: "acme-uat".into(),
        scope: TraceScope::Cluster,
        started: 0.0,
        ended: Some(10.0),
        end_reason: Some(TraceEnd::Stopped),
        unfinished: false,
        data_start: None,
        data_end: None,
        prometheus: Some("monitoring/prometheus".into()),
        retention_secs: None,
        pods: 1,
        containers: 1,
        prometheus_samples: 1000,
        local_samples: 0,
        gaps: vec![],
        file_bytes: 0,
        pull_every: None,
        pulled_bytes: 0,
    }
}

fn review(rows: Vec<ReviewRow>) -> Option<Result<TraceReview, String>> {
    Some(Ok(TraceReview { summary: summary(), rows, note: None }))
}

fn series(full_start: f64, full_end: f64) -> TraceSeries {
    TraceSeries {
        start: full_start,
        end: full_end,
        step: 60.0,
        cpu_avg: vec![],
        cpu_max: vec![],
        mem_avg: vec![],
        mem_max: vec![],
        pods: vec![],
        cpu_request: vec![],
        cpu_limit: vec![],
        mem_request: vec![],
        mem_limit: vec![],
        events: vec![],
        gaps: vec![],
        full_start,
        full_end,
    }
}

#[test]
fn applying_needs_the_traced_cluster() {
    let r = review(vec![]);
    assert_eq!(apply_blocked(r.as_ref(), true, Some("acme-uat")), None);
    let blocked = Some("Connect to acme-uat to apply suggestions.".to_owned());
    assert_eq!(apply_blocked(r.as_ref(), false, Some("acme-uat")), blocked, "disconnected");
    assert_eq!(apply_blocked(r.as_ref(), true, Some("acme-prd")), blocked, "another cluster");
    assert_eq!(apply_blocked(r.as_ref(), true, None), blocked);
    assert_eq!(apply_blocked(None, true, None), None, "nothing under review");
    assert_eq!(apply_blocked(Some(&Err("x".into())), true, None), None);
    assert_eq!(apply_note("Job", None), Some("A Job's requests can't be changed here.".into()));
    assert_eq!(apply_note("Deployment", blocked.clone()), blocked);
    assert_eq!(apply_note("Deployment", None), None);
    assert_eq!(apply_selected_title(blocked), "Connect to acme-uat to apply suggestions.");
    assert_eq!(apply_selected_title(None), "Preview, then apply the suggested requests");
    assert_eq!(apply_selected_text(2), "Apply selected (2)…");
}

#[test]
fn table_cells_and_badges() {
    let mut r = row("web", "shop", Some(-5.0), vec![
        ReviewFlag::RequestsChanged,
        ReviewFlag::NoCpuRequest,
        ReviewFlag::OomKilled { count: 1 },
        ReviewFlag::Throttled { worst: 0.5 },
        ReviewFlag::Restarts { count: 2 },
    ]);
    let (shown, more) = shown_flags(&r);
    assert_eq!(shown, vec![&ReviewFlag::OomKilled { count: 1 }, &ReviewFlag::Throttled { worst: 0.5 }]);
    assert_eq!(more, 2, "info flags aren't counted");
    assert_eq!((more_text(2), more_text(0)), (Some("+2".into()), None));
    assert_eq!(flag_class(&ReviewFlag::NoCpuRequest), "flag warning");
    assert_eq!(spare(None, Measure::Cpu), None);
    assert_eq!(spare(Some(-5.0), Measure::Cpu), Some((true, "short 5.0m".into())));
    assert_eq!(spare(Some(50.0), Measure::Cpu), Some((false, "50m".into())));
    assert_eq!(suggestion_text(&r), "10m · 16 B");
    r.avg_pods = 1.5;
    assert_eq!(pods_text(&r), "1.5");
    r.suggestion = None;
    assert_eq!(suggestion_text(&r), "—");
    assert_eq!(key_subtitle(&r.key), "Deployment · app · shop");
    assert_eq!(detail_subtitle(&r.key), "Deployment · container app · shop");
    assert_eq!((check_title(true), check_title(false)), ("Choose for Apply selected", "No suggestion that can be applied"));
    assert_eq!(rows_text(1, true), "1 row · select one for its charts");
    assert_eq!(rows_text(3, false), "3 rows");
    assert_eq!(rows_text(0, true), "0 rows");
}

#[test]
fn sort_values_and_scores() {
    let r = row("web", "shop", Some(5.0), vec![ReviewFlag::Throttled { worst: 0.5 }, ReviewFlag::RequestsChanged]);
    assert_eq!(sort_value(&r, SortBy::Workload), None, "sorted by name");
    assert_eq!(sort_value(&r, SortBy::CpuSpare), Some(5.0));
    assert_eq!(flag_score(&r), 1e4 + 1.0, "one serious, one info");
}

#[test]
fn sort_headings() {
    assert_eq!(next_sort((SortBy::CpuSpare, true), SortBy::CpuSpare), (SortBy::CpuSpare, false));
    assert_eq!(next_sort((SortBy::CpuSpare, true), SortBy::Pods), (SortBy::Pods, true));
    assert_eq!(next_sort((SortBy::CpuSpare, true), SortBy::Workload), (SortBy::Workload, false), "names start ascending");
    assert_eq!(sort_arrow((SortBy::Pods, true), SortBy::CpuSpare), "");
    assert_eq!(sort_arrow((SortBy::Pods, true), SortBy::Pods), " ↓");
    assert_eq!(sort_arrow((SortBy::Pods, false), SortBy::Pods), " ↑");
}

#[test]
fn the_reviews_rows() {
    let rows = vec![row("web", "shop", Some(5.0), vec![]), row("api", "ops", Some(50.0), vec![]), row("db", "shop", None, vec![])];
    let r = review(rows.clone());
    assert_eq!(namespaces(r.as_ref()), ["ops", "shop"]);
    assert!(namespaces(None).is_empty());
    assert_eq!((namespace_choice(String::new()), namespace_choice("ops".into())), (None, Some("ops".into())));
    let shop = Filter { namespace: Some("shop".into()), ..Filter::default() };
    let names = |rows: Vec<ReviewRow>| rows.into_iter().map(|r| r.key.name).collect::<Vec<_>>();
    assert_eq!(names(visible_rows(r.as_ref(), &shop, (SortBy::CpuSpare, true))), ["web", "db"]);
    assert!(visible_rows(None, &shop, (SortBy::CpuSpare, true)).is_empty());

    assert!(keeps_selection(&TraceReview { summary: summary(), rows: rows.clone(), note: None }, &rows[1].key));
    let gone = TraceKey { name: "gone".into(), ..rows[0].key.clone() };
    assert!(!keeps_selection(&TraceReview { summary: summary(), rows: rows.clone(), note: None }, &gone));
    assert_eq!(find_row(r.as_ref(), &rows[1].key).map(|r| r.key.name), Some("api".into()));
    assert_eq!(find_row(r.as_ref(), &gone), None);
    assert_eq!(find_row(Some(&Err("x".into())), &gone), None);
    assert_eq!(find_row(None, &gone), None);
}

#[test]
fn choosing_rows_to_apply() {
    let mut job = row("once", "shop", None, vec![]);
    job.key.kind = "Job".into();
    let rows = vec![row("web", "shop", None, vec![]), job, row("api", "shop", None, vec![])];
    let keys = applicable_keys(rows.clone());
    assert_eq!(keys.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), ["web", "api"]);
    let mut checked = HashSet::new();
    assert!(!all_checked(&[], &checked), "nothing to check");
    assert!(!all_checked(&keys, &checked));
    toggle(&mut checked, &keys[0]);
    assert!(!all_checked(&keys, &checked));
    toggle_all(&mut checked, keys.clone());
    assert!(all_checked(&keys, &checked));
    assert_eq!(checked_rows(review(rows.clone()).as_ref(), &checked).len(), 2);
    assert!(checked_rows(None, &checked).is_empty());
    toggle_all(&mut checked, keys.clone());
    assert!(checked.is_empty());
    toggle(&mut checked, &keys[0]);
    toggle(&mut checked, &keys[0]);
    assert!(checked.is_empty());

    let mut picked = BTreeSet::new();
    toggle_ordered(&mut picked, &"a".to_owned());
    assert!(picked.contains("a"));
    toggle_ordered(&mut picked, &"a".to_owned());
    assert!(picked.is_empty());
    assert_eq!(compare_pair("a".into(), "b".into()), Ok(vec!["a".to_owned(), "b".to_owned()]));
    assert_eq!(compare_pair("a".into(), "a".into()), Err("Choose a different trace to compare with."));
}

#[test]
fn notes() {
    let mut proxy = row("web", "shop", None, vec![]);
    proxy.key.container = "proxy".into();
    let mut r = TraceReview { summary: summary(), rows: vec![row("web", "shop", None, vec![]), proxy, row("api", "shop", None, vec![])], note: None };
    save_note(&mut r, None, "  checked in Oct ");
    assert_eq!(r.note.as_deref(), Some("checked in Oct"));
    save_note(&mut r, None, "   ");
    assert_eq!(r.note, None, "blank clears it");
    let key = r.rows[0].key.clone();
    save_note(&mut r, Some(&key), "resized");
    assert_eq!(r.rows.iter().map(|row| row.note.as_deref()).collect::<Vec<_>>(), [Some("resized"), Some("resized"), None], "every container of the workload");

    assert_eq!(note_what(None), "the note on this trace");
    assert_eq!(note_what(Some(&key)), "the note on web");
    assert_eq!(note_dirty("a ", "a", "x"), None);
    assert_eq!(note_dirty("b", "a", "x"), Some("x".into()));
    assert_eq!(add_note_text("Note on this trace"), "Add a note on this trace…");
}

#[test]
fn the_page_texts() {
    let status = TraceStatus {
        context: "acme-uat".into(),
        path: "/a.trace".into(),
        scope: TraceScope::Cluster,
        started: 0.0,
        data_start: 0.0,
        data_end: 0.0,
        prometheus: true,
        last_pull: None,
        last_error: None,
        gaps: 0,
        sampled_pods: 0,
        stopping: false,
        pull_every: 0.0,
    };
    assert_eq!(recording_text(&status), "Recording the whole cluster of acme-uat");
    let recent = RecentTrace { path: "/a.trace".into(), context: "acme-uat".into(), what: "namespace shop".into(), at: 0.0 };
    assert_eq!(recent_text(&recent, "today"), "acme-uat · namespace shop · today");
    assert_eq!(compare_text(3), "Compare 3…");
    assert_eq!(report_error_text("disk full"), "The report wasn't saved: disk full");
    assert_eq!(overview_title(Some("shop")), "Overview of shop");
    assert_eq!(overview_title(None), "Overview of the whole trace");
    assert_eq!((gaps_text(0), gaps_text(2)), ("None".to_owned(), "2".to_owned()));
    assert_eq!(basis_text("p95"), "Based on p95.");
}

#[test]
fn the_reviews_header() {
    let mut s = summary();
    assert!(!recording(review(vec![]).as_ref()));
    assert_eq!(state_text(&s), "Stopped");
    for (reason, text) in [
        (TraceEnd::Quit, "Stopped when the app quit"),
        (TraceEnd::ContextChanged, "Stopped on a context switch"),
        (TraceEnd::ConnectionLost, "Stopped: connection lost"),
    ] {
        s.end_reason = Some(reason);
        assert_eq!(state_text(&s), text);
    }
    s.ended = None;
    assert_eq!(state_text(&s), "Still recording");
    assert!(recording(Some(&Ok(TraceReview { summary: s.clone(), rows: vec![], note: None }))));
    s.unfinished = true;
    assert_eq!(state_text(&s), "Ended unexpectedly (data up to the last pull is intact)");
    assert!(!recording(Some(&Ok(TraceReview { summary: s.clone(), rows: vec![], note: None }))));
    assert!(!recording(None));

    assert_eq!(source_text(&s), "Prometheus");
    s.local_samples = 5;
    assert_eq!(source_text(&s), "Prometheus, <1% sampled by the app");
    s.local_samples = 250;
    assert_eq!(source_text(&s), "Prometheus, 20% sampled by the app");
    s.prometheus = None;
    assert_eq!(source_text(&s), "Sampled by the app (no Prometheus)");
}

#[test]
fn the_selected_rows_details() {
    let key = TraceKey { namespace: "shop".into(), kind: "Deployment".into(), name: "web".into(), container: "app".into() };
    let args = SeriesArgs::new("/a.trace".into(), key.clone(), Some((1.0, 2.0)), 50);
    assert_eq!((args.start, args.end, args.points, args.key), (Some(1.0), Some(2.0), 50, key.clone()));
    let all = OverviewArgs::new("/a.trace".into(), None, None);
    assert_eq!((all.start, all.end, all.points, all.namespace), (None, None, CHART_POINTS, None));

    let s = series(0.0, 100_000.0);
    assert!(quick_active(None, None, None));
    assert!(quick_active(Some(6.0), Some((100_000.0 - 6.0 * 3600.0, 100_000.0)), Some(&s)));
    assert!(!quick_active(Some(1.0), Some((100_000.0 - 6.0 * 3600.0, 100_000.0)), Some(&s)));
    assert!(!quick_active(Some(6.0), Some((0.0, 6.0 * 3600.0)), Some(&s)), "not up to the end");
    assert!(!quick_active(None, Some((0.0, 1.0)), Some(&s)));
    assert!(!quick_active(Some(6.0), None, Some(&s)));
    assert_eq!(quick_zoom(None, None), Some(None));
    assert_eq!(quick_zoom(Some(1.0), Some(&s)), Some(Some((100_000.0 - 3600.0, 100_000.0))));
    assert_eq!(quick_zoom(Some(48.0), Some(&s)), Some(Some((0.0, 100_000.0))), "no earlier than the trace");
    assert_eq!(quick_zoom(Some(1.0), None), None);

    assert_eq!(suggest_line(Measure::Cpu, Some(1500.0), 15.0), ("CPU request".into(), "1.5 cores".into(), "15m".into(), "−99%".into()));
    assert_eq!(suggest_line(Measure::Memory, None, 1024.0), ("Memory request".into(), "—".into(), "1.0 KiB".into(), String::new()));

    let mut r = row("web", "shop", None, vec![]);
    assert_eq!(facts_text(&r), "2 pods seen, 2.0 running on average · 10h 0m of data · 0 rollouts");
    r.local_share = 0.25;
    r.rollouts = 1;
    assert_eq!(facts_text(&r), "2 pods seen, 2.0 running on average · 10h 0m of data · 1 rollout · 25% sampled by the app");
}

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
    for by in [SortBy::Pods, SortBy::CpuRequest, SortBy::CpuP95, SortBy::MemRequest, SortBy::MemPeak, SortBy::MemSpare, SortBy::Suggested, SortBy::Flags] {
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
    let mut memory = row("cache", "shop", None, vec![]);
    memory.mem_spare = Some(-1.0);
    let mut roomy = row("idle", "shop", Some(5.0), vec![]);
    roomy.mem_spare = Some(1.0);
    let rows = vec![memory, roomy];
    assert_eq!(filter_rows(&rows, &Filter { flagged_only: true, ..Filter::default() }).len(), 1, "memory above its request");
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

#[test]
fn suggested_and_flags_sort_too() {
    let names = |rows: &[ReviewRow]| rows.iter().map(|r| r.key.name.clone()).collect::<Vec<_>>();
    let mut many_warnings = row("warned", "shop", None, vec![ReviewFlag::NoCpuRequest, ReviewFlag::NoMemoryRequest, ReviewFlag::Restarts { count: 3 }]);
    many_warnings.suggestion = None;
    let mut rows = vec![
        row("quiet", "shop", None, vec![ReviewFlag::RequestsChanged]),
        many_warnings,
        row("oom", "shop", None, vec![ReviewFlag::OomKilled { count: 1 }]),
    ];
    rows[2].suggestion.as_mut().unwrap().cpu_request = 500.0;
    sort_rows(&mut rows, SortBy::Flags, true);
    assert_eq!(names(&rows), ["oom", "warned", "quiet"], "one critical outweighs three warnings");
    assert!(flag_score(&rows[0]) > flag_score(&rows[1]));
    sort_rows(&mut rows, SortBy::Suggested, true);
    assert_eq!(names(&rows), ["oom", "quiet", "warned"], "no suggestion goes last");
    sort_rows(&mut rows, SortBy::Suggested, false);
    assert_eq!(names(&rows), ["quiet", "oom", "warned"]);
}
