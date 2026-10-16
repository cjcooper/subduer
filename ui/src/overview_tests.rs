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
    use crate::sort::Key;
    use subduer_types::{NodeSummary, ResourceTotals};
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
