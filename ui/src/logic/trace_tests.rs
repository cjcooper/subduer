use super::*;

fn kind(group: &str, kind: &str) -> KindRef {
    KindRef { group: group.into(), version: "v1".into(), kind: kind.into(), plural: String::new(), namespaced: true }
}

fn workload(name: &str) -> TraceWorkload {
    TraceWorkload { kind: kind("apps", "Deployment"), namespace: "shop".into(), name: name.into() }
}

fn status() -> TraceStatus {
    TraceStatus {
        context: "acme-uat".into(),
        path: "/tmp/a.trace".into(),
        scope: TraceScope::Cluster,
        started: 1000.0,
        data_start: 0.0,
        data_end: 0.0,
        prometheus: true,
        last_pull: None,
        last_error: None,
        gaps: 0,
        sampled_pods: 0,
        stopping: false,
        pull_every: 600.0,
    }
}

fn summary() -> TraceSummary {
    TraceSummary {
        path: "/tmp/a.trace".into(),
        context: "acme-uat".into(),
        scope: TraceScope::Namespaces(vec!["shop".into()]),
        started: 0.0,
        ended: None,
        end_reason: None,
        unfinished: false,
        data_start: None,
        data_end: None,
        prometheus: None,
        retention_secs: None,
        pods: 3,
        containers: 5,
        prometheus_samples: 0,
        local_samples: 0,
        gaps: vec![],
        file_bytes: 0,
        pull_every: None,
        pulled_bytes: 0,
    }
}

#[test]
fn editing_a_draft() {
    let mut d = Draft::for_view(Some("shop".into()));
    d.filter = "we".into();
    d.set_level(Level::Pod);
    assert_eq!((d.level, d.filter.as_str()), (Level::Pod, ""));
    assert_eq!(d.targets_namespace(), Some("shop"));
    d.pod = "web-1".into();
    d.filter = "x".into();
    d.set_namespace("ops");
    assert_eq!((d.namespace.as_str(), d.pod.as_str(), d.filter.as_str()), ("ops", "", ""));
    d.namespace.clear();
    assert_eq!(d.targets_namespace(), None, "no namespace chosen");
    d.namespace = "ops".into();
    d.set_level(Level::Namespaces);
    assert_eq!(d.targets_namespace(), None, "namespaces list themselves");

    d.filter = "WE".into();
    assert!(d.shows("Web-1") && !d.shows("api"));
    assert_eq!(d.chosen_text(), "1 chosen");
    d.level = Level::Workloads;
    d.namespace = "shop".into();
    d.toggle_workload(&workload("web"));
    assert_eq!(d.chosen_text(), "1 chosen");
    d.namespace = "ops".into();
    assert_eq!(d.chosen_text(), "1 chosen (1 in other namespaces)");
}

#[test]
fn opening_and_switching() {
    assert_eq!(open_blocked(false, false), Some("Connect to a cluster to start a trace."));
    assert_eq!((open_blocked(true, false), open_blocked(false, true)), (None, None));
    let s = status();
    assert!(stops_on_switch(Some(&s), "acme-prd"));
    assert!(!stops_on_switch(Some(&s), "acme-uat"));
    assert!(!stops_on_switch(None, "acme-prd"));
    assert_eq!(trace_this_title("Deployment"), "Record this Deployment's CPU and memory over time");
    assert_eq!(indicator_title(&s), "Tracing the whole cluster of acme-uat into /tmp/a.trace. Click for details.");
    let r = recent("/x.trace".into(), &summary(), 5.0);
    assert_eq!((r.path.as_str(), r.context.as_str(), r.what.as_str(), r.at), ("/x.trace", "acme-uat", "namespace shop", 5.0));
}

#[test]
fn stamps_and_spans() {
    // Mon 5 Oct 2026 14:05 UTC, an hour east.
    assert_eq!(stamp(1_791_158_400.0 + 14.0 * 3600.0 + 5.0 * 60.0, 3600.0), "2026-10-05-1505");
    assert_eq!(span_text(Some(0.0), Some(7200.0)), "2h 0m");
    assert_eq!((span_text(None, Some(1.0)), span_text(Some(1.0), None)), ("no data".to_owned(), "no data".to_owned()));
    assert_eq!(every_text(1200), "20 min");
}

#[test]
fn the_running_panel() {
    let mut s = status();
    assert_eq!(pull_text(&s, 2000.0), "Not pulled from Prometheus yet.");
    s.last_pull = Some(1880.0);
    assert_eq!(pull_text(&s, 2000.0), "Last pulled from Prometheus 2m ago.");
    s.last_error = Some("timeout".into());
    assert_eq!(pull_text(&s, 2000.0), "The last pull failed: timeout. The next one tries again.");
    assert_eq!(pulls_text(&s), Some("Every 10 min".into()));
    s.prometheus = false;
    assert_eq!(pull_text(&s, 2000.0), "No Prometheus found: only the app's own samples are kept, while it runs.");
    assert_eq!(pulls_text(&s), None);
    s.prometheus = true;
    s.pull_every = 0.0;
    assert_eq!(pulls_text(&s), None, "older backends don't say");

    assert_eq!(running_gaps_text(0), None);
    assert_eq!(running_gaps_text(1), Some("1 gap: Prometheus no longer had some data when it was pulled.".into()));
    assert_eq!(running_gaps_text(3), Some("3 gaps: Prometheus no longer had some data when it was pulled.".into()));
    assert_eq!(sampled_text(0), None);
    assert_eq!(sampled_text(1), Some("1 running pod isn't in Prometheus; the app samples it every 30 s.".into()));
    assert_eq!(sampled_text(4), Some("4 running pods aren't in Prometheus; the app samples them every 30 s.".into()));
}

#[test]
fn the_saved_and_quit_dialogs() {
    let mut s = summary();
    assert_eq!(holds_text(&s), "3 pods, 5 containers");
    assert_eq!(pulled_text(&s), None);
    s.pulled_bytes = 2048;
    assert_eq!(pulled_text(&s), Some("2.0 KiB from Prometheus".into()));
    assert_eq!(saved_gaps_text(0), None);
    assert_eq!(saved_gaps_text(1), Some("1 gap with no data.".into()));
    assert_eq!(saved_gaps_text(2), Some("2 gaps with no data.".into()));

    let mut t = status();
    assert_eq!(quit_what(&t, 1000.0 + 3600.0), "the whole cluster of acme-uat (1h 0m)");
    t.gaps = 1;
    assert_eq!(quit_what(&t, 1060.0), "the whole cluster of acme-uat (1m, 1 gap)");
    t.gaps = 2;
    assert_eq!(quit_what(&t, 1060.0), "the whole cluster of acme-uat (1m, 2 gaps)");
    assert_eq!(quit_texts(true), ("Quitting stops it. The file is saved first.", "Stop trace and quit"));
    assert_eq!(quit_texts(false), ("Closing the window stops it. The file is saved first.", "Stop trace and close"));
}

#[test]
fn drafts_start_from_the_view_or_the_object() {
    let all = Draft::for_view(None);
    assert_eq!((all.level, all.scope()), (Level::Cluster, Some(TraceScope::Cluster)));
    let shop = Draft::for_view(Some("shop".into()));
    assert_eq!(shop.scope(), Some(TraceScope::Namespaces(vec!["shop".into()])));

    let ns = Draft::for_object(&kind("", "Namespace"), None, "ops").unwrap();
    assert_eq!(ns.scope(), Some(TraceScope::Namespaces(vec!["ops".into()])));
    let pod = Draft::for_object(&kind("", "Pod"), Some("shop".into()), "web-1").unwrap();
    assert_eq!(pod.scope(), Some(TraceScope::Pod { namespace: "shop".into(), name: "web-1".into() }));
    let deploy = Draft::for_object(&kind("apps", "Deployment"), Some("shop".into()), "web").unwrap();
    assert_eq!(deploy.scope(), Some(TraceScope::Workloads(vec![workload("web")])));
    for k in ["DaemonSet", "StatefulSet"] {
        assert!(Draft::for_object(&kind("apps", k), Some("shop".into()), "x").is_some(), "{k}");
    }
    assert!(Draft::for_object(&kind("batch", "CronJob"), Some("shop".into()), "x").is_some());
    assert!(Draft::for_object(&kind("", "ConfigMap"), Some("shop".into()), "x").is_none());
    assert!(Draft::for_object(&kind("apps", "ReplicaSet"), Some("shop".into()), "x").is_none());
    assert!(Draft::for_object(&kind("", "Pod"), None, "x").is_none());
}

#[test]
fn a_draft_needs_a_target() {
    let mut d = Draft::for_view(Some("shop".into()));
    d.toggle_namespace("shop");
    assert_eq!(d.scope(), None);
    d.toggle_namespace("b");
    d.toggle_namespace("a");
    assert_eq!(d.namespaces, vec!["a", "b"]);
    d.level = Level::Workloads;
    assert_eq!(d.scope(), None);
    d.toggle_workload(&workload("web"));
    d.toggle_workload(&workload("api"));
    d.toggle_workload(&workload("web"));
    assert_eq!(d.scope(), Some(TraceScope::Workloads(vec![workload("api")])));
    d.level = Level::Pod;
    assert_eq!(d.scope(), None);
    d.pod = "web-1".into();
    assert_eq!(d.scope(), Some(TraceScope::Pod { namespace: "shop".into(), name: "web-1".into() }));
    d.namespace.clear();
    assert_eq!(d.scope(), None);
}

#[test]
fn names_and_texts() {
    let ns = TraceScope::Namespaces(vec!["shop".into(), "ops".into(), "db".into()]);
    assert_eq!(file_name("acme-uat", &ns, "2026-10-11-1405"), "acme-uat-shop+2-2026-10-11-1405.trace");
    assert_eq!(file_name("arn:aws/x", &TraceScope::Cluster, "s"), "arn-aws-x-cluster-s.trace");
    assert_eq!(file_name("c", &TraceScope::Workloads(vec![workload("web")]), "s"), "c-web-s.trace");
    let pod = TraceScope::Pod { namespace: "shop".into(), name: "web-1".into() };
    assert_eq!(file_name("c", &pod, "s"), "c-web-1-s.trace");
    assert_eq!(more(&[]), "");

    assert_eq!(scope_text(&TraceScope::Cluster), "the whole cluster");
    assert_eq!(scope_text(&TraceScope::Namespaces(vec!["shop".into()])), "namespace shop");
    assert_eq!(scope_text(&ns), "3 namespaces");
    assert_eq!(scope_text(&TraceScope::Workloads(vec![workload("web")])), "Deployment web");
    assert_eq!(scope_text(&TraceScope::Workloads(vec![workload("a"), workload("b")])), "2 workloads");
    assert_eq!(scope_text(&pod), "pod web-1");

    assert_eq!(duration_text(-5.0), "0m");
    assert_eq!(duration_text(59.0 * 60.0), "59m");
    assert_eq!(duration_text(3.0 * 3600.0 + 12.0 * 60.0), "3h 12m");
    assert_eq!(duration_text(2.0 * 86400.0 + 4.5 * 3600.0), "2d 4h");
    assert_eq!(ago_text(30.0), "just now");
    assert_eq!(ago_text(180.0), "3m ago");
}

#[test]
fn the_indicator() {
    let mut s = status();
    assert_eq!(indicator_text(&s, 1000.0 + 7500.0), "Trace · 2h 5m");
    assert!(!needs_attention(&s));
    s.stopping = true;
    assert_eq!(indicator_text(&s, 1060.0), "Trace · 1m · stopping…");
    s.gaps = 1;
    assert!(needs_attention(&s));
    s.gaps = 0;
    s.last_error = Some("Prometheus didn't answer".into());
    assert!(needs_attention(&s));
}
