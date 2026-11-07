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
        spec: vec![],
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

fn event(type_: &str, count: i64) -> EventSummary {
    EventSummary {
        type_: type_.into(),
        reason: "Pulled".into(),
        message: String::new(),
        count,
        first_seen: String::new(),
        last_seen: String::new(),
        source: String::new(),
        object: None,
    }
}

fn history(source: HistorySource, pod: UsageHistory) -> PodHistory {
    PodHistory {
        source,
        start: 10.0,
        end: 20.0,
        step: 2.0,
        pod,
        containers: vec![subduer_types::ContainerHistory {
            name: "app".into(),
            usage: UsageHistory { cpu: vec![(1.0, 5.0)], memory: vec![] },
        }],
    }
}

fn env(name: &str) -> EnvVar {
    EnvVar { name: name.into(), value: EnvValue::Literal(String::new()), from: None }
}

#[test]
fn args_serialize_for_the_backend() {
    let pod = serde_json::to_value(PodArgs { namespace: "shop".into(), name: "web-1".into() }).unwrap();
    assert_eq!(pod, serde_json::json!({ "namespace": "shop", "name": "web-1" }));
    let node = serde_json::to_value(HistoryArgs { kind: None, namespace: None, name: "node-1".into() }).unwrap();
    assert_eq!(node, serde_json::json!({ "name": "node-1" }), "cluster-scoped: no namespace");
    let kind = crate::app::tests_unit::kind("apps", "Deployment");
    let workload = serde_json::to_value(HistoryArgs { kind: Some(kind), namespace: Some("shop".into()), name: "web".into() }).unwrap();
    assert_eq!(workload["namespace"], "shop");
    assert_eq!(workload["kind"]["kind"], "Deployment");
    let secret = serde_json::to_value(SecretArgs { namespace: "shop".into(), secret: "db".into(), key: "pw".into() }).unwrap();
    assert_eq!(secret, serde_json::json!({ "namespace": "shop", "secret": "db", "key": "pw" }));
}

#[test]
fn background_failures_keep_the_last_good_render() {
    assert!(replaces(false, true, true), "a requested load shows its error");
    assert!(!replaces(true, true, true));
    assert!(replaces(true, true, false), "nothing good to keep");
    assert!(replaces(true, false, true));
    assert!(history_replaces(true, true));
    assert!(!history_replaces(false, true));
    assert!(history_replaces(false, false));
}

#[test]
fn events_summary_counts_warnings() {
    assert_eq!(events_summary(&[]), "none");
    assert_eq!(events_summary(&[event("Normal", 1)]), "1");
    assert_eq!(events_summary(&[event("Normal", 1), event("Warning", 1)]), "2 · ⚠ 1 warning");
    assert_eq!(events_summary(&[event("Warning", 1), event("Warning", 1)]), "2 · ⚠ 2 warnings");
}

#[test]
fn toggling_and_finding() {
    let mut toggled = std::collections::HashSet::new();
    toggle(&mut toggled, "a");
    assert!(toggled.contains("a"));
    toggle(&mut toggled, "a");
    assert!(toggled.is_empty());
    assert!(finding(true, " x "));
    assert!(!finding(true, "  "));
    assert!(!finding(false, "x"));
}

#[test]
fn resource_classes_and_scales() {
    assert_eq!((Resource::Cpu.css(), Resource::Memory.css()), ("res-cpu", "res-memory"));
    assert_eq!((Resource::Cpu.scale(), Resource::Memory.scale()), (Scale::Decimal, Scale::Binary));
}

#[test]
fn series_for_the_pod_or_a_container() {
    let h: Result<PodHistory, String> = Ok(history(HistorySource::Prometheus, UsageHistory { cpu: vec![(1.0, 9.0)], memory: vec![] }));
    assert_eq!(series_points(None, Resource::Cpu, None), None, "still loading");
    assert_eq!(series_points(Some(&Err("boom".into())), Resource::Cpu, None), Some(vec![]));
    assert_eq!(series_points(Some(&h), Resource::Cpu, None), Some(vec![(1.0, 9.0)]));
    assert_eq!(series_points(Some(&h), Resource::Cpu, Some("app")), Some(vec![(1.0, 5.0)]));
    assert_eq!(series_points(Some(&h), Resource::Cpu, Some("gone")), Some(vec![]));
    assert_eq!(latest(Some(&[(1.0, 2.0), (2.0, 3.0)]), Some(7.0)), Some(3.0));
    assert_eq!(latest(Some(&[]), Some(7.0)), Some(7.0));
    assert_eq!(latest(None, None), None);
}

#[test]
fn chart_data_takes_the_window_and_set_refs() {
    let h: Result<PodHistory, String> = Ok(history(HistorySource::Prometheus, UsageHistory::default()));
    let refs = [("request", Some(100.0), true), ("limit", None, false)];
    let d = chart_data(vec![(12.0, 1.0)], Some(&h), refs);
    assert_eq!((d.start, d.end, d.step), (10.0, 20.0, 2.0));
    assert_eq!(d.points, [(12.0, 1.0)]);
    assert_eq!(d.refs, [RefLine { name: "request", value: 100.0, dashed: true }]);
    let d = chart_data(vec![], None, refs);
    assert_eq!((d.start, d.end, d.step), (0.0, 1.0, 1.0));
}

#[test]
fn history_note_names_the_source() {
    let clock = |t: f64| format!("t{t}");
    let prom = history(HistorySource::Prometheus, UsageHistory::default());
    assert_eq!(history_note(&prom, "this pod", clock), "Last hour from Prometheus · updates every 15 s");
    let local = history(HistorySource::Local, UsageHistory { cpu: vec![(3.0, 1.0)], memory: vec![] });
    assert_eq!(
        history_note(&local, "this pod", clock),
        "No Prometheus data for this pod, so the app records usage itself: since t3 · updates every 15 s"
    );
    let local = history(HistorySource::Local, UsageHistory { cpu: vec![(3.0, 1.0)], memory: vec![(2.0, 1.0)] });
    assert!(history_note(&local, "its pods", clock).contains("since t2"), "memory first");
    let none = history(HistorySource::Local, UsageHistory::default());
    assert_eq!(history_note(&none, "this pod", clock), "No usage source: needs Prometheus, metrics-server or kubelet stats.");
    assert_eq!(history_error("boom"), "Can't load usage history: boom");
}

#[test]
fn pod_info_rows_leave_out_empty_values() {
    let d = PodDetails {
        name: "web-1".into(),
        namespace: "shop".into(),
        status: "Running".into(),
        node: Some("node-1".into()),
        pod_ip: Some(String::new()),
        host_ip: None,
        qos_class: Some("Burstable".into()),
        service_account: None,
        owner: Some("ReplicaSet/web-7d9f".into()),
        restart_policy: None,
        priority_class: None,
        created: None,
        labels: vec![],
        cpu: Usage::default(),
        memory: Usage::default(),
        metrics_available: false,
        containers: vec![],
        events: vec![],
    };
    assert_eq!(
        pod_rows(&d),
        [
            ("Namespace", "shop".to_owned()),
            ("Node", "node-1".into()),
            ("QoS class", "Burstable".into()),
            ("Controlled by", "ReplicaSet/web-7d9f".into()),
        ]
    );
    assert_eq!(ago_at("5m", "2026-01-01T00:00:00Z"), "5m ago · 2026-01-01T00:00:00Z");
    assert_eq!(label_chip("app", "web"), "app=web");
}

#[test]
fn container_texts_and_actions() {
    use subduer_types::containers::State::*;
    let mut c = container("app", false, false, false, Running);
    c.detail = "Running".into();
    assert_eq!(container_summary(&c), "Running");
    c.restarts = 1;
    assert_eq!(container_summary(&c), "Running · 1 restart");
    c.restarts = 4;
    c.sidecar = true;
    assert_eq!(container_summary(&c), "Sidecar · Running · 4 restarts");

    let mut c = container("app", false, false, false, Running);
    assert_eq!(not_ready(&c), None);
    c.ready = false;
    assert_eq!(not_ready(&c), Some(" (not ready)"));
    c.init = true;
    assert_eq!(not_ready(&c), None, "init containers aren't ready-checked");

    let mut c = container("app", false, false, false, Running);
    let a = container_actions(&c);
    assert!(a.running && !a.forward, "no ports, no Forward");
    assert_eq!(a.logs, "Open app logs in a new window");
    assert_eq!(a.shell, "Open a shell in app in a new window (needs changes allowed)");
    assert_eq!(a.upload, "Upload a file into app (needs changes allowed)");
    assert_eq!(a.download, "Download a file from app");
    c.ports = vec!["8080/TCP".into()];
    assert!(container_actions(&c).forward);
    c.state = Stopped;
    let a = container_actions(&c);
    assert!(!a.running && !a.forward);

    assert_eq!(
        logs_target("shop", "web-1", "app"),
        LogTarget { namespace: "shop".into(), source: LogSource::Container { pod: "web-1".into(), container: "app".into() }, filter: None }
    );
}

#[test]
fn environment_rows() {
    assert_eq!(env_summary(&[]), "none");
    assert_eq!(env_summary(&[env(""), env("A"), env("B")]), "2", "unnamed entries don't count");
    assert_eq!(env_shown(&EnvValue::Literal(String::new())), EnvShown::Empty);
    assert_eq!(env_shown(&EnvValue::Literal("x".into())), EnvShown::Literal("x".into()));
    assert_eq!(
        env_shown(&EnvValue::ConfigMap { name: "cfg".into(), key: "k".into(), value: None }),
        EnvShown::ConfigMap { value: None, source: "ConfigMap cfg · k".into() }
    );
    assert_eq!(
        env_shown(&EnvValue::Secret { name: "db".into(), key: "pw".into() }),
        EnvShown::Secret { name: "db".into(), key: "pw".into() }
    );
    assert_eq!(env_shown(&EnvValue::Field("metadata.name".into())), EnvShown::Reference("field metadata.name".into()));
    assert_eq!(env_shown(&EnvValue::Resource("limits.cpu".into())), EnvShown::Reference("resource limits.cpu".into()));
    assert_eq!(
        env_shown(&EnvValue::Unreadable { source: "ConfigMap x".into(), error: "forbidden".into() }),
        EnvShown::Warning("Can't read ConfigMap x: forbidden".into())
    );
    assert_eq!(secret_source("db", "pw"), "Secret db · pw");
}

#[test]
fn event_rows() {
    assert_eq!(event_badge("Warning"), "badge badge-warning");
    assert_eq!(event_badge("Normal"), "badge");
    assert_eq!(event_when(1, "5m"), "5m ago");
    assert_eq!(event_when(3, "5m"), "×3 · last 5m ago");
}
