use super::*;

fn ns(name: &str, pods: usize, cpu: f64, memory: f64) -> NamespaceRequests {
    NamespaceRequests { namespace: name.into(), pods, cpu, memory }
}

#[test]
fn fmt_bytes_uses_binary_units() {
    assert_eq!(fmt_bytes(0.0), "0 B");
    assert_eq!(fmt_bytes(1023.0), "1023 B");
    assert_eq!(fmt_bytes(1024.0), "1.0 KiB");
    assert_eq!(fmt_bytes(1536.0), "1.5 KiB");
    assert_eq!(fmt_bytes(500.0 * 1024.0 * 1024.0), "500.0 MiB");
    assert_eq!(fmt_bytes(3.25 * 1024f64.powi(3)), "3.2 GiB");
    assert_eq!(fmt_bytes(2.0 * 1024f64.powi(4)), "2.0 TiB");
    // TiB is the largest unit.
    assert_eq!(fmt_bytes(2048.0 * 1024f64.powi(4)), "2048.0 TiB");
}

#[test]
fn fmt_cpu_uses_millicores_then_cores() {
    assert_eq!(fmt_cpu(0.0), "0m");
    assert_eq!(fmt_cpu(0.5), "0.5m");
    assert_eq!(fmt_cpu(9.94), "9.9m");
    assert_eq!(fmt_cpu(10.0), "10m");
    assert_eq!(fmt_cpu(250.4), "250m");
    assert_eq!(fmt_cpu(999.0), "999m");
    assert_eq!(fmt_cpu(1000.0), "1 core");
    assert_eq!(fmt_cpu(2000.0), "2 cores");
    assert_eq!(fmt_cpu(1500.0), "1.5 cores");
    assert_eq!(fmt_cpu(12_340.0), "12.3 cores");
}

#[test]
fn pct_rounds_and_flags_tiny_shares() {
    assert_eq!(pct(0.0), "0%");
    assert_eq!(pct(0.001), "<1%");
    assert_eq!(pct(0.0099), "<1%");
    assert_eq!(pct(0.01), "1%");
    assert_eq!(pct(0.456), "46%");
    assert_eq!(pct(1.0), "100%");
    assert_eq!(pct(1.25), "125%");
}

#[test]
fn age_ms_shows_seconds_then_minutes() {
    assert_eq!(age_ms(0.0), "0s");
    assert_eq!(age_ms(-500.0), "0s");
    assert_eq!(age_ms(59_999.0), "59s");
    assert_eq!(age_ms(60_000.0), "1m");
    assert_eq!(age_ms(3_600_000.0), "60m");
}

#[test]
fn severity_levels() {
    assert_eq!(severity(0.0), ("normal", ""));
    assert_eq!(severity(0.7499), ("normal", ""));
    assert_eq!(severity(0.75), ("warning", "⚠ "));
    assert_eq!(severity(0.8999), ("warning", "⚠ "));
    assert_eq!(severity(0.9), ("critical", "⚠ "));
    assert_eq!(severity(1.5), ("critical", "⚠ "));
}

#[test]
fn usage_note_per_source() {
    assert_eq!(usage_note(Some(UsageSource::MetricsServer)), "Usage from metrics-server.");
    assert!(usage_note(Some(UsageSource::Prometheus)).starts_with("Usage from Prometheus"));
    assert!(usage_note(Some(UsageSource::Kubelet)).starts_with("Usage from kubelet stats"));
    assert!(usage_note(None).starts_with("Live usage needs"));
}

#[test]
fn namespace_rows_keep_the_top_ones_and_fold_the_rest() {
    let sorted: Vec<NamespaceRequests> =
        (0..TOP_NAMESPACES + 3).map(|i| ns(&format!("shop-{i}"), i + 1, 100.0 - i as f64, 0.0)).collect();
    let rows = namespace_rows(&sorted, |n| n.cpu);
    assert_eq!(rows.len(), TOP_NAMESPACES + 1);
    assert_eq!(rows[0], ("shop-0".to_owned(), 100.0, 1));
    let (label, value, pods) = rows.last().unwrap();
    assert_eq!(label, "Other (3 namespaces)");
    let rest = sorted[TOP_NAMESPACES..].iter();
    assert_eq!(*value, rest.clone().map(|n| n.cpu).sum::<f64>());
    assert_eq!(*pods, rest.map(|n| n.pods).sum::<usize>());
}

#[test]
fn namespace_rows_skip_zero_values() {
    let sorted = vec![ns("acme", 2, 0.0, 512.0), ns("shop", 3, 50.0, 0.0)];
    assert_eq!(namespace_rows(&sorted, |n| n.cpu), [("shop".to_owned(), 50.0, 3)]);
    assert_eq!(namespace_rows(&sorted, |n| n.memory), [("acme".to_owned(), 512.0, 2)]);
    assert!(namespace_rows(&[], |n| n.cpu).is_empty());
}

#[test]
fn nodes_sort_by_what_each_column_shows() {
    use subduer_types::ResourceTotals;
    let totals = |requests, allocatable, used| ResourceTotals { capacity: allocatable, allocatable, requests, limits: 0.0, used };
    let n = NodeSummary {
        name: "node-2".into(),
        ready: true,
        schedulable: false,
        roles: vec![],
        instance_type: Some("m6i.large".into()),
        kubelet_version: "v1.33.1".into(),
        created: Some("2026-10-16T12:00:00Z".into()),
        pods: 12,
        pod_capacity: 29,
        cpu: totals(500.0, 2000.0, None),
        memory: totals(1.0, 4.0, Some(2.0)),
    };
    assert_eq!(node_key(&n, "Name"), Key::Text("node-2".into()));
    assert_eq!(node_key(&n, "Status"), Key::Num(2.0), "ready but cordoned");
    assert_eq!(node_key(&n, "Pods"), Key::Num(12.0));
    assert_eq!(node_key(&n, "CPU requested"), Key::Num(0.25));
    assert_eq!(node_key(&n, "Memory used"), Key::Num(0.5));
    assert_eq!(node_key(&n, "CPU used"), Key::Missing);
    assert_eq!(node_key(&n, "CPU"), Key::Num(2000.0));
    assert_eq!(node_key(&n, "Type"), Key::Text("m6i.large".into()));
    assert!(matches!(node_key(&n, "Age"), Key::Num(_)));
    assert_eq!(node_key(&n, "Unknown"), Key::Missing);
    let empty = NodeSummary { cpu: totals(1.0, 0.0, None), ..n };
    assert_eq!(node_key(&empty, "CPU requested"), Key::Missing, "nothing allocatable");
}

fn node(name: &str, ready: bool, schedulable: bool) -> NodeSummary {
    use subduer_types::ResourceTotals;
    let totals = |requests, allocatable, used| ResourceTotals { capacity: allocatable, allocatable, requests, limits: 0.0, used };
    NodeSummary {
        name: name.into(),
        ready,
        schedulable,
        roles: vec![],
        instance_type: None,
        kubelet_version: "v1.33.1".into(),
        created: None,
        pods: 3,
        pod_capacity: 29,
        cpu: totals(500.0, 2000.0, None),
        memory: totals(1.0, 4.0, Some(2.0)),
    }
}

fn overview(nodes: Vec<NodeSummary>, pods: PodCounts, warnings: Option<usize>) -> ClusterOverview {
    ClusterOverview {
        server_version: None,
        nodes,
        pods,
        cpu: Default::default(),
        memory: Default::default(),
        namespaces: vec![],
        warnings_last_hour: warnings,
        metrics_available: false,
        usage_source: None,
        history: None,
    }
}

#[test]
fn refresh_fetches_unless_loading_or_fresh() {
    assert!(!needs_fetch(true, false, 0.0, true), "one at a time");
    assert!(needs_fetch(false, false, 0.0, false), "nothing yet");
    assert!(!needs_fetch(false, true, FRESH_MS - 1.0, false), "fresh enough");
    assert!(needs_fetch(false, true, FRESH_MS - 1.0, true), "forced");
    assert!(needs_fetch(false, true, FRESH_MS, false), "stale");
}

#[test]
fn polling_stops_after_repeated_failures() {
    assert!(keep_polling(0));
    assert!(keep_polling(MAX_FAILURES - 1));
    assert!(!keep_polling(MAX_FAILURES));
    assert_eq!(failure(1, "boom".into()), ("boom".into(), false));
    assert_eq!(failure(MAX_FAILURES, "boom".into()), ("Stopped refreshing after repeated failures: boom".into(), true));
}

#[test]
fn subtitle_and_updated() {
    assert_eq!(subtitle("acme".into(), Some("v1.33".into())), "acme · v1.33");
    assert_eq!(subtitle("acme".into(), None), "acme");
    assert_eq!(updated(0.0, 5000.0), None);
    assert_eq!(updated(1000.0, 13_000.0).as_deref(), Some("Updated 12s ago"));
}

#[test]
fn filter_nodes_by_name_any_case() {
    let nodes = vec![node("Node-A", true, true), node("node-b", true, true)];
    assert_eq!(filter_nodes(nodes.clone(), "").len(), 2);
    let a = filter_nodes(nodes, "NODE-a");
    assert_eq!(a.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["Node-A"]);
}

#[test]
fn tiles_tone_by_health() {
    let pods = PodCounts { running: 5, pending: 0, succeeded: 0, failed: 0, unknown: 0 };
    let t = tiles(&overview(vec![node("a", true, true)], pods, Some(0)));
    assert_eq!((t.nodes.as_str(), t.nodes_note.as_str(), t.nodes_tone), ("1", "all ready", "good"));
    assert_eq!((t.pods.as_str(), t.pods_note.as_str(), t.pods_tone), ("5", "0 pending · 0 failed", "good"));
    assert_eq!((t.warnings.as_str(), t.warnings_note.as_str(), t.warnings_tone), ("0", "none", "good"));

    let pending = PodCounts { pending: 2, ..pods };
    let t = tiles(&overview(vec![node("a", true, true), node("b", false, true)], pending, Some(4)));
    assert_eq!((t.nodes_note.as_str(), t.nodes_tone), ("⚠ 1 not ready", "critical"));
    assert_eq!(t.pods_tone, "warning");
    assert_eq!((t.warnings.as_str(), t.warnings_note.as_str(), t.warnings_tone), ("4", "⚠ see Events", "warning"));

    let failed = PodCounts { failed: 1, pending: 2, ..pods };
    let t = tiles(&overview(vec![], failed, None));
    assert_eq!(t.pods_tone, "critical");
    assert_eq!((t.warnings.as_str(), t.warnings_note.as_str(), t.warnings_tone), ("–", "can't read events", ""));
}

#[test]
fn by_cpu_sorts_largest_first() {
    let sorted = by_cpu(&[ns("a", 1, 10.0, 0.0), ns("b", 1, 30.0, 0.0), ns("c", 1, 20.0, 0.0)]);
    assert_eq!(sorted.iter().map(|n| n.namespace.as_str()).collect::<Vec<_>>(), ["b", "c", "a"]);
}

#[test]
fn card_texts() {
    assert_eq!(card_subtitle(3, 2048.0, fmt_bytes), "allocatable across 3 nodes · 2.0 KiB capacity");
    assert_eq!(peak(&[(0.0, 100.0), (1.0, 300.0), (2.0, 200.0)], fmt_cpu), "peak 300m");
    assert_eq!(peak(&[], fmt_cpu), "peak 0m");
}

#[test]
fn meters_show_share_and_severity() {
    assert_eq!(fraction(1.0, 4.0), 0.25);
    assert_eq!(fraction(1.0, 0.0), 0.0);
    assert_eq!(fill_width(0.5), "width: 50.0%");
    assert_eq!(fill_width(1.5), "width: 100.0%");
    let m = meter("Used", 500.0, 1000.0, fmt_cpu);
    assert_eq!(
        m,
        MeterText {
            fill_class: "meter-fill normal".into(),
            text: "500m · 50%".into(),
            width: "width: 50.0%".into(),
            title: "Used: 500m of 1 core".into(),
        }
    );
    let over = meter("Limits", 3000.0, 2000.0, fmt_cpu);
    assert_eq!(over.text, "⚠ 3 cores · 150% · overcommitted");
    assert_eq!(over.fill_class, "meter-fill critical");
    assert_eq!(mini_meter(80.0, 100.0), ("meter-fill warning".into(), "width: 80.0%".into(), "⚠ 80%".into()));
}

#[test]
fn pod_bar_segments() {
    let pods = PodCounts { running: 3, pending: 1, succeeded: 0, failed: 0, unknown: 0 };
    let s = pod_segments(pods);
    assert_eq!(s[0], ("Running", "good", 3));
    assert_eq!(s.iter().map(|s| s.0).collect::<Vec<_>>(), ["Running", "Pending", "Failed", "Unknown", "Succeeded"]);
    assert_eq!(segment("Running", 3, 4), ("flex-grow: 3".into(), "Running: 3 (75%)".into()));
}

#[test]
fn namespace_bars_scale_to_the_largest() {
    let rows = vec![("a".to_owned(), 50.0, 2), ("b".to_owned(), 100.0, 3)];
    assert_eq!(bar_max(&rows), 100.0);
    assert_eq!(bar_max(&[]), 0.0);
    assert_eq!(bar("a", 50.0, 2, 100.0, fmt_cpu), ("width: 50.0%".into(), "a: 50m across 2 pods".into()));
    assert_eq!(bar("a", 0.0, 2, 0.0, fmt_cpu).0, "width: 0.0%");
}

#[test]
fn node_table_columns_and_rows() {
    let plain: Vec<&str> = node_headers(false).iter().map(|h| h.0).collect();
    assert_eq!(plain, ["Name", "Status", "Type", "Pods", "CPU requested", "Memory requested", "CPU", "Memory", "Version", "Age"]);
    let with = node_headers(true);
    assert_eq!(with.len(), 12);
    assert_eq!(with[6], ("CPU used", ""));
    assert_eq!(with[3], ("Pods", "num"));
    assert_eq!(node_status(&node("a", true, true)), ("Ready".into(), "badge badge-ready"));
    assert_eq!(node_status(&node("a", true, false)), ("Ready · Cordoned".into(), "badge badge-pending"));
    assert_eq!(node_status(&node("a", false, false)), ("NotReady · Cordoned".into(), "badge badge-notready"));
    assert_eq!(node_pods(&node("a", true, true)), "3/29");
    let n = node("a", true, true);
    assert_eq!(node_key(&n, "Memory requested"), Key::Num(0.25));
    assert_eq!(node_key(&n, "Memory"), Key::Num(4.0));
    assert_eq!(node_key(&n, "Version"), Key::Text("v1.33.1".into()));
    assert_eq!(node_key(&n, "Type"), Key::Missing);
    let no_mem = NodeSummary { memory: subduer_types::ResourceTotals { used: None, ..n.memory }, ..n };
    assert_eq!(node_key(&no_mem, "Memory used"), Key::Missing);
    let busy = NodeSummary { cpu: subduer_types::ResourceTotals { used: Some(1000.0), ..no_mem.cpu }, ..no_mem };
    assert_eq!(node_key(&busy, "CPU used"), Key::Num(0.5));
}
