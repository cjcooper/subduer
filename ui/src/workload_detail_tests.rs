use super::*;

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
    for bad in ["Failed", "FailureTarget", "ReplicaFailure", "NamespaceDeletionContentFailure", "MemoryPressure", "DiskPressure"] {
        assert!(!healthy(&condition(bad, "True")), "{bad} true is a problem");
        assert!(healthy(&condition(bad, "False")), "{bad} false is fine");
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
