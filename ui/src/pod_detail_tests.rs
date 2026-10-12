use super::*;

fn usage(requests: Option<f64>, limits: Option<f64>, used: Option<f64>, node: Option<f64>) -> Usage {
    Usage { requests, limits, used, node }
}

fn container(name: &str, init: bool, sidecar: bool, ephemeral: bool, state: subduer_types::containers::State) -> ContainerDetails {
    ContainerDetails {
        name: name.into(),
        init,
        sidecar,
        ephemeral,
        image: "acme/x:1".into(),
        state,
        detail: String::new(),
        ready: true,
        restarts: 0,
        started: None,
        last_termination: None,
        ports: vec![],
        cpu: Usage::default(),
        memory: Usage::default(),
        env: vec![],
    }
}

#[test]
fn containers_are_grouped_running_init_ephemeral() {
    use subduer_types::containers::State::*;
    let all = [
        container("migrate", true, false, false, Stopped),
        container("proxy", true, true, false, Running),
        container("app", false, false, false, Running),
        container("worker", false, false, false, Failed),
        container("subduer-capture-1", false, false, true, Running),
    ];
    let groups = container_groups(&all);
    let names = |g: &Group| g.containers.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
    assert_eq!(groups.iter().map(|g| (g.title, g.closed)).collect::<Vec<_>>(), [
        ("Running containers", false),
        ("Init containers", true),
        ("Ephemeral containers", true),
    ]);
    assert_eq!(names(&groups[0]), ["proxy", "app", "worker"], "sidecars run with the app");
    assert_eq!(names(&groups[1]), ["migrate"]);
    assert_eq!(names(&groups[2]), ["subduer-capture-1"]);
    assert_eq!((group_summary(&groups[0].containers), group_summary(&groups[1].containers)), ("3 · 1 failed".into(), "1".into()));
    // Empty groups are left out.
    let plain = container_groups(&all[2..3]);
    assert_eq!(plain.iter().map(|g| g.id).collect::<Vec<_>>(), ["containers:running"]);
}

#[test]
fn sections_that_start_closed_remember_being_opened() {
    let mut toggled = std::collections::HashSet::new();
    assert!(section_open(&toggled, "containers:running", false));
    assert!(!section_open(&toggled, "containers:init", true));
    toggled.insert(toggle_key("containers:running", false));
    toggled.insert(toggle_key("containers:init", true));
    assert!(!section_open(&toggled, "containers:running", false));
    assert!(section_open(&toggled, "containers:init", true));
    // The two kinds don't share keys: opening the closed one leaves an open one with the same id alone.
    assert!(section_open(&toggled, "containers:init", false));
}

#[test]
fn resource_labels_and_formats() {
    assert_eq!(Resource::Cpu.label(), "CPU");
    assert_eq!(Resource::Memory.label(), "Memory");
    assert_eq!(Resource::Cpu.fmt()(1500.0), "1.5 cores");
    assert_eq!(Resource::Memory.fmt()(2048.0), "2.0 KiB");
}

#[test]
fn resource_series_picks_cpu_or_memory() {
    let h = UsageHistory { cpu: vec![(1.0, 10.0)], memory: vec![(1.0, 2048.0), (2.0, 4096.0)] };
    assert_eq!(Resource::Cpu.series(&h), &[(1.0, 10.0)]);
    assert_eq!(Resource::Memory.series(&h).len(), 2);
}

#[test]
fn usage_header_against_the_limit() {
    let u = usage(Some(100.0), Some(500.0), Some(250.0), Some(4000.0));
    assert_eq!(usage_header(u, fmt_cpu), "now 250m · 50% of limit · req 100m · limit 500m");
}

#[test]
fn usage_header_warns_from_three_quarters() {
    let u = usage(None, Some(1000.0), Some(750.0), None);
    assert_eq!(usage_header(u, fmt_cpu), "⚠ now 750m · 75% of limit · req not set · limit 1 core");
    let u = usage(None, Some(1000.0), Some(749.0), None);
    assert!(!usage_header(u, fmt_cpu).starts_with('⚠'));
}

#[test]
fn usage_header_falls_back_to_the_node() {
    let u = usage(Some(100.0), None, Some(40.0), Some(4000.0));
    assert_eq!(usage_header(u, fmt_cpu), "now 40m · 1% of node · req 100m · limit unlimited (node 4 cores)");
    let u = usage(Some(100.0), None, Some(20.0), Some(4000.0));
    assert_eq!(usage_header(u, fmt_cpu), "now 20m · <1% of node · req 100m · limit unlimited (node 4 cores)");
}

#[test]
fn usage_header_without_numbers() {
    assert_eq!(usage_header(Usage::default(), fmt_cpu), "now — · req not set · limit unlimited");
    // Zero usage shows 0%, and a zero ceiling shows no share.
    let u = usage(None, Some(500.0), Some(0.0), None);
    assert_eq!(usage_header(u, fmt_cpu), "now 0m · 0% of limit · req not set · limit 500m");
    let u = usage(None, None, Some(5.0), Some(0.0));
    assert_eq!(usage_header(u, fmt_cpu), "now 5.0m · req not set · limit unlimited (node 0m)");
}

#[test]
fn node_header_shares_of_allocatable() {
    let cpu = |c: f64| format!("{c:.0}m");
    let t = ResourceTotals { capacity: 2000.0, allocatable: 1930.0, requests: 1750.0, limits: 0.0, used: Some(123.0) };
    assert_eq!(node_header(Some(123.0), t, cpu), "⚠ now 123m (6%) · req 1750m (91%) · limits none · allocatable 1930m");
    let t = ResourceTotals { requests: 500.0, limits: 900.0, ..t };
    assert_eq!(node_header(Some(5.0), t, cpu), "now 5m (<1%) · req 500m (26%) · limits 900m · allocatable 1930m");
    assert_eq!(node_header(None, t, cpu), "now — · req 500m (26%) · limits 900m · allocatable 1930m");
    let t = ResourceTotals { allocatable: 0.0, ..t };
    assert_eq!(node_header(Some(5.0), t, cpu), "now 5m · req 500m · limits 900m · allocatable 0m");
}
