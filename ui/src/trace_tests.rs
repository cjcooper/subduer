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
    }
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
    assert_eq!(more::<String>(&[], |s| s.clone()), "");

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
