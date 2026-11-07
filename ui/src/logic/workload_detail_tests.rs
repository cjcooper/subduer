use super::*;
use crate::logic::overview::{fmt_bytes, fmt_cpu};

fn condition(type_: &str, status: &str) -> Condition {
    Condition { type_: type_.into(), status: status.into(), reason: String::new(), message: String::new(), last_transition: None }
}

#[test]
fn plural_picks_the_form_by_count() {
    assert_eq!(plural(0, "pod", "pods"), "0 pods");
    assert_eq!(plural(1, "pod", "pods"), "1 pod");
    assert_eq!(plural(2, "replica", "replicas"), "2 replicas");
}

#[test]
fn is_time_knows_the_time_rows() {
    for k in ["Started", "Finished", "Last scheduled", "Last successful", "Next run", "Last restart"] {
        assert!(is_time(k), "{k}");
    }
    for k in ["Schedule", "started", "Created", ""] {
        assert!(!is_time(k), "{k}");
    }
}

#[test]
fn relative_at_reads_past_and_future() {
    let now = 10_000_000.0;
    assert_eq!(relative_at(now, now - 300_000.0), "5m ago");
    assert_eq!(relative_at(now, now), "0s ago");
    assert_eq!(relative_at(now, now + 12.0 * 60_000.0), "in 12m");
    assert_eq!(relative_at(now, now + 3.0 * 86_400_000.0), "in 3d");
    assert_eq!(relative_at(now, now + 500.0), "in 0s");
}

#[test]
fn ready_all_needs_every_replica_ready() {
    assert!(ready_all("3/3"));
    assert!(ready_all("1/1"));
    assert!(!ready_all("2/3"));
    assert!(!ready_all("0/0"), "nothing to be ready");
    assert!(!ready_all("3"));
    assert!(!ready_all(""));
}

#[test]
fn badge_class_slugs_the_status() {
    assert_eq!(badge_class("Running"), "badge badge-running");
    assert_eq!(badge_class("Completed (OOM)"), "badge badge-completed-oom");
    assert_eq!(badge_class("Rolling out"), "badge badge-rolling-out");
    assert_eq!(badge_class("Scaled to 0"), "badge badge-scaled-to-0");
    assert_eq!(badge_class("  --CrashLoopBackOff--  "), "badge badge-crashloopbackoff");
    assert_eq!(badge_class(""), "badge badge-");
}

#[test]
fn healthy_flips_for_problem_conditions() {
    assert!(healthy(&condition("Available", "True")));
    assert!(!healthy(&condition("Available", "False")));
    assert!(!healthy(&condition("Progressing", "Unknown")));
    for bad in ["Failed", "FailureTarget", "ReplicaFailure", "NamespaceDeletionContentFailure", "MemoryPressure", "DiskPressure", "Fallback"] {
        assert!(!healthy(&condition(bad, "True")), "{bad} true is a problem");
        assert!(healthy(&condition(bad, "False")), "{bad} false is fine");
    }
    for either in ["Active", "Paused"] {
        assert!(healthy(&condition(either, "True")) && healthy(&condition(either, "False")), "{either} is fine either way");
        assert!(!healthy(&condition(either, "Unknown")));
    }
}

#[test]
fn run_time_is_seconds_precise_under_an_hour() {
    assert_eq!(run_time(0), "0s");
    assert_eq!(run_time(59), "59s");
    assert_eq!(run_time(60), "1m 0s");
    assert_eq!(run_time(138), "2m 18s");
    assert_eq!(run_time(3599), "59m 59s");
    assert_eq!(run_time(3600), "60m");
    assert_eq!(run_time(3 * 3600), "3h");
}

#[test]
fn scaled_multiplies_requests_and_limits_by_pods() {
    let u = Usage { requests: Some(100.0), limits: Some(250.0), used: Some(42.0), node: Some(4000.0) };
    let s = scaled(u, 3);
    assert_eq!(s, Usage { requests: Some(300.0), limits: Some(750.0), used: Some(42.0), node: Some(4000.0) });
    // No pods: shown as one pod's worth.
    assert_eq!(scaled(u, 0), u);
    let unset = Usage { requests: None, limits: None, used: None, node: None };
    assert_eq!(scaled(unset, 5), unset);
}

#[test]
fn per_pod_shows_request_and_limit() {
    let u = Usage { requests: Some(250.0), limits: Some(1500.0), used: None, node: None };
    assert_eq!(per_pod(u, fmt_cpu), "request 250m · limit 1.5 cores");
    let u = Usage { requests: Some(64.0 * 1024.0 * 1024.0), limits: None, used: None, node: None };
    assert_eq!(per_pod(u, fmt_bytes), "request 64.0 MiB · limit not set");
    assert_eq!(per_pod(Usage::default(), fmt_cpu), "request not set · limit not set");
}

#[test]
fn pods_and_jobs_sort_by_value() {
    use crate::sort::Key;
    let p = WorkloadPod {
        name: "web-1".into(),
        status: "Running".into(),
        containers: vec![],
        ready: "1/1".into(),
        restarts: 4,
        node: None,
        created: None,
        updated: true,
        finished: false,
        cpu: Some(12.0),
        memory: None,
    };
    assert_eq!([pod_key(&p, 0), pod_key(&p, 2), pod_key(&p, 3), pod_key(&p, 4), pod_key(&p, 5)],
        [Key::Text("web-1".into()), Key::Num(4.0), Key::Num(12.0), Key::Missing, Key::Missing]);
    let j = JobRun {
        name: "nightly-1".into(),
        status: "Complete".into(),
        completions: "1/1".into(),
        manual: false,
        created: Some("2026-10-16T12:00:00Z".into()),
        started: Some("2026-10-16T12:00:00Z".into()),
        finished: Some("2026-10-16T12:02:18Z".into()),
    };
    assert_eq!(job_key(&j, 3, 0.0), Key::Num(138_000.0));
    let running = JobRun { finished: None, ..j.clone() };
    let start = crate::sort::Key::time("2026-10-16T12:00:00Z");
    let Key::Num(start) = start else { panic!() };
    assert_eq!(job_key(&running, 3, start + 5000.0), Key::Num(5000.0), "a running Job counts up to now");
    assert_eq!(job_key(&JobRun { started: None, ..j.clone() }, 3, 0.0), Key::Missing);
    assert_eq!(job_key(&j, 2, 0.0), Key::Text("1/1".into()));
    assert_eq!(job_key(&j, 0, 0.0), Key::Text("nightly-1".into()));
    assert_eq!(job_key(&j, 1, 0.0), Key::Text("complete".into()));
    assert!(matches!(job_key(&j, 4, 0.0), Key::Num(_)));
    let odd = JobRun { finished: Some("soon".into()), ..j };
    assert_eq!(job_key(&odd, 3, 0.0), Key::Missing, "an unreadable end");
    assert_eq!(pod_key(&p, 1), Key::Text("running".into()));
}

fn pod(name: &str, ready: &str, finished: bool) -> WorkloadPod {
    WorkloadPod {
        name: name.into(),
        status: "Running".into(),
        containers: vec![],
        ready: ready.into(),
        restarts: 0,
        node: None,
        created: None,
        updated: true,
        finished,
        cpu: None,
        memory: None,
    }
}

fn job(status: &str) -> JobRun {
    JobRun { name: "j".into(), status: status.into(), completions: "1/1".into(), manual: false, created: None, started: None, finished: None }
}

fn details(pods: Vec<WorkloadPod>, replicas: Option<ReplicaCounts>) -> WorkloadDetails {
    WorkloadDetails {
        kind: "Deployment".into(),
        name: "web".into(),
        namespace: "shop".into(),
        status: "Available".into(),
        replicas,
        suspended: None,
        info: vec![],
        created: None,
        labels: vec![],
        conditions: vec![],
        cpu: Usage::default(),
        memory: Usage::default(),
        metrics_available: false,
        pods,
        containers: vec![],
        jobs: vec![],
        events: vec![],
    }
}

fn template(image: &str, init: bool, sidecar: bool) -> TemplateContainer {
    TemplateContainer {
        name: "app".into(),
        init,
        sidecar,
        image: image.into(),
        ports: vec![],
        cpu: Usage::default(),
        memory: Usage::default(),
        env: vec![],
        spec: vec![],
    }
}

const REPLICAS: ReplicaCounts = ReplicaCounts { desired: 3, current: 3, ready: 2, updated: 3, available: 2 };

#[test]
fn details_args_serialize() {
    let args = DetailsArgs { kind: crate::app::tests_unit::kind("apps", "Deployment"), namespace: "shop".into(), name: "web".into() };
    let v = serde_json::to_value(args).unwrap();
    assert_eq!((v["namespace"].as_str(), v["name"].as_str(), v["kind"]["kind"].as_str()), (Some("shop"), Some("web"), Some("Deployment")));
}

#[test]
fn section_summaries() {
    assert_eq!(live_pods(&[pod("a", "1/1", false), pod("b", "0/1", true)]), 1);
    assert_eq!(pods_summary(&details(vec![], Some(REPLICAS))), "none");
    assert_eq!(pods_summary(&details(vec![pod("a", "1/1", false), pod("b", "0/1", false)], Some(REPLICAS))), "2 · 1 ready");
    assert_eq!(pods_summary(&details(vec![pod("a", "0/1", false), pod("b", "0/1", true)], None)), "2 · 1 running");
    assert_eq!(jobs_summary(&[]), "none");
    assert_eq!(jobs_summary(&[job("Complete")]), "1");
    assert_eq!(jobs_summary(&[job("Complete"), job("Failed")]), "2 · 1 failed");
    assert_eq!(resources_summary(0, false), "none running · request and limit per pod");
    assert_eq!(resources_summary(0, true), "");
    assert_eq!(resources_summary(1, true), "total of 1 pod");
    assert_eq!(resources_summary(3, false), "total of 3 pods");
}

#[test]
fn info_rows() {
    assert!(is_mono("Selector") && is_mono("Revision") && is_mono("Schedule"));
    assert!(!is_mono("Strategy"));
    assert_eq!(dated("in 3m", "2026-10-16T12:00:00Z"), "in 3m · 2026-10-16T12:00:00Z");
    assert_eq!(replicas_text(REPLICAS), "3 desired · 2 ready · 3 up to date · 2 available");
    let rolling = ReplicaCounts { current: 4, ..REPLICAS };
    assert_eq!(replicas_text(rolling), "3 desired · 2 ready · 3 up to date · 2 available · 4 running during rollout");
}

#[test]
fn condition_summaries_and_badges() {
    assert_eq!(conditions_summary(&[]), "none");
    assert_eq!(conditions_summary(&[condition("Available", "True")]), "1");
    assert_eq!(conditions_summary(&[condition("Available", "True"), condition("Failed", "True")]), "2 · 1 unhealthy");
    assert_eq!(condition_badge(&condition("Available", "True")), "badge badge-ready");
    assert_eq!(condition_badge(&condition("Available", "False")), "badge badge-warning");
}

#[test]
fn pod_and_job_rows() {
    let mut p = pod("web-1", "1/1", false);
    assert_eq!(pod_title(&p), "Open pod web-1");
    p.node = Some("node-1".into());
    assert_eq!(pod_title(&p), "Open pod web-1 · on node-1");
    assert!(!is_old(true, &p));
    p.updated = false;
    assert!(is_old(true, &p));
    assert!(!is_old(false, &p));
    assert_eq!(usage_cell(Some(250.0), fmt_cpu), "250m");
    assert_eq!(usage_cell(None, fmt_cpu), "—");
    assert_eq!(heading_class("CPU", &["CPU", "Memory"]), "num");
    assert_eq!(heading_class("Name", &["CPU", "Memory"]), "");
    assert_eq!(duration(0.0, 138_000.0).as_deref(), Some("2m 18s"));
    assert_eq!(duration(10_000.0, 0.0).as_deref(), Some("0s"), "clock skew shows 0");
    assert_eq!(duration(f64::NAN, 0.0), None);
    assert_eq!(duration(0.0, f64::NAN), None);
}

#[test]
fn template_containers() {
    assert_eq!(template_summary(&template("docker.io/acme/web:1.2", false, false)), "web:1.2");
    assert_eq!(template_summary(&template("migrate:3", true, false)), "Init · migrate:3");
    assert_eq!(template_summary(&template("envoy", true, true)), "Sidecar · envoy");
    assert!(charted(&template("x", false, false)));
    assert!(charted(&template("x", true, true)));
    assert!(!charted(&template("x", true, false)));
    assert_eq!(totals_note(true, 3).as_deref(), Some("Charts show totals over 3 pods."));
    assert_eq!(totals_note(true, 1), None);
    assert_eq!(totals_note(false, 3), None);
}
