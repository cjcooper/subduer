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
        note: None,
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

    let container = |cpu_limit: QuantityChange, mem_limit: QuantityChange| ContainerPreview {
        container: "app".into(),
        injected: false,
        cpu_request: q(Some("1"), Some("2")),
        cpu_limit,
        mem_request: QuantityChange::default(),
        mem_limit,
    };
    assert_eq!(limit_notes(&container(q(Some("1"), Some("1")), QuantityChange::default())), None);
    assert_eq!(
        limit_notes(&container(q(Some("1"), Some("2")), q(Some("1Gi"), Some("2Gi")))),
        Some("CPU limit raised to match: 1 → 2; memory limit raised to match: 1Gi → 2Gi".into())
    );
}

#[test]
fn the_apply_dialog() {
    let change = RightsizeChange { kind: "Deployment".into(), namespace: "shop".into(), name: "web".into(), containers: vec![] };
    assert_eq!(card_title(&change), "Deployment shop/web");
    let mut p = RightsizePreview {
        change,
        helm_release: None,
        helm_snippet: None,
        containers: vec![],
        blocked: None,
        dry_run_error: None,
        effect: String::new(),
    };
    assert!(ready(&p));
    p.dry_run_error = Some("refused".into());
    assert!(!ready(&p));
    p.dry_run_error = None;
    p.blocked = Some("nothing to change".into());
    assert!(!ready(&p));

    assert!(can_apply(1, false, true));
    assert!(!can_apply(0, false, true));
    assert!(!can_apply(1, true, true));
    assert!(!can_apply(1, false, false));

    let mut outcomes = vec![None, Some(Outcome::of(Ok(()))), None];
    assert!(!all_done(&outcomes));
    assert!(!all_done(&[]));
    outcomes[0] = Some(Outcome::of(Err("refused".into())));
    finish(&mut outcomes);
    assert!(all_done(&outcomes));
    assert_eq!(outcomes, [Some(Outcome::Failed("refused".into())), Some(Outcome::Applied), Some(Outcome::Failed("Not applied".into()))]);
    assert_eq!(outcome_text(&Outcome::Applied), "✓ Applied");
    assert_eq!(outcome_text(&Outcome::Failed("x".into())), "✗ x");

    assert_eq!(apply_text(true, 2), "Applying…");
    assert_eq!(apply_text(false, 1), "Apply to 1 workload");
    assert_eq!(apply_text(false, 3), "Apply to 3 workloads");
    assert_eq!(read_only_text("acme-prd"), "acme-prd is read-only.");
    assert_eq!(prod_text("acme-prd"), "acme-prd looks like production.");
}
