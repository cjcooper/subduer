use super::*;
use subduer_types::{Suggestion, TraceKey};

fn row(kind: &str, name: &str, container: &str, suggest: bool) -> ReviewRow {
    ReviewRow {
        key: TraceKey { namespace: "shop".into(), kind: kind.into(), name: name.into(), container: container.into() },
        pods: 1,
        avg_pods: 1.0,
        minutes: 600,
        pod_minutes: 600,
        rollouts: 0,
        local_share: 0.0,
        cpu_request: Some(500.0),
        cpu_limit: None,
        mem_request: None,
        mem_limit: None,
        cpu_p50: 1.0,
        cpu_p95: 2.0,
        cpu_p99: 3.0,
        cpu_max: 4.0,
        mem_p95: 1.0,
        mem_max: 2.0,
        cpu_spare: None,
        mem_spare: None,
        flags: vec![],
        suggestion: suggest.then(|| Suggestion { cpu_request: 15.0, mem_request: 64.0, basis: String::new() }),
    }
}

#[test]
fn one_change_per_workload_in_row_order() {
    let rows = vec![
        row("Deployment", "web", "app", true),
        row("StatefulSet", "db", "db", true),
        row("Deployment", "web", "proxy", true),
        row("Job", "once", "run", true),
        row("DaemonSet", "agent", "agent", false),
    ];
    let changes = changes_for(&rows);
    assert_eq!(changes.len(), 2);
    assert_eq!((changes[0].kind.as_str(), changes[0].name.as_str()), ("Deployment", "web"));
    assert_eq!(changes[0].containers.iter().map(|c| c.container.as_str()).collect::<Vec<_>>(), ["app", "proxy"]);
    assert_eq!((changes[0].containers[0].cpu_request, changes[0].containers[0].mem_request), (Some(15.0), Some(64.0)));
    assert_eq!(changes[1].name, "db");
    assert!(changes_for(&rows[3..]).is_empty());
    assert!(applicable(&rows[0]) && !applicable(&rows[3]) && !applicable(&rows[4]));
    assert!(applicable_kind("CronJob") && !applicable_kind("Pod"));
}

#[test]
fn change_cells() {
    let q = |b: Option<&str>, a: Option<&str>| QuantityChange { before: b.map(Into::into), after: a.map(Into::into) };
    assert_eq!(change_cell(&q(Some("1500m"), Some("15m"))), "1500m → 15m");
    assert_eq!(change_cell(&q(None, Some("50m"))), "not set → 50m");
    assert_eq!(change_cell(&q(Some("2"), Some("2"))), "2");
    assert_eq!(change_cell(&q(None, None)), "not set");
}
