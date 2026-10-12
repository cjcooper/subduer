use std::path::PathBuf;

use serde_json::{Value, json};

use super::*;
use crate::testing::{MockApi, kind, list, object, pod};

const PROM: &str = "/api/v1/namespaces/monitoring/services/prometheus-server:9090/proxy/api/v1";
/// A fixed "now", on a minute.
const NOW: i64 = 1_800_000_000;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("subduer-trace-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn key(pod: &str, container: &str) -> ContainerKey {
    ContainerKey::new("shop", pod, container)
}

fn success(data: Value) -> Value {
    json!({"status": "success", "data": data})
}

fn matrix(results: Vec<Value>) -> Value {
    success(json!({"resultType": "matrix", "result": results}))
}

fn vector(results: Vec<Value>) -> Value {
    success(json!({"resultType": "vector", "result": results}))
}

fn points(points: &[(i64, f64)]) -> Value {
    points.iter().map(|(t, v)| json!([t, v.to_string()])).collect()
}

fn deploy(name: &str) -> TraceWorkload {
    TraceWorkload { kind: kind("apps", "v1", "Deployment", "deployments", true), namespace: "shop".into(), name: name.into() }
}

/// A Prometheus at monitoring/prometheus-server:9090 with 2 h retention
/// that answers every query with nothing.
fn with_prometheus(api: &MockApi) {
    let svc = object("v1", "Service", Some("monitoring"), "prometheus-server", json!({"spec": {"ports": [{"port": 9090}], "clusterIP": "10.0.0.9"}}));
    api.on("GET", "/api/v1/services", list("ServiceList", vec![svc]));
    api.on("GET", &format!("{PROM}/query"), vector(vec![]));
    api.on("GET", &format!("{PROM}/query_range"), matrix(vec![]));
    api.on("GET", &format!("{PROM}/status/flags"), success(json!({"storage.tsdb.retention.time": "2h"})));
}

fn param(call: &crate::testing::Call, name: &str) -> String {
    form_urlencoded::parse(call.query.as_bytes()).find(|(k, _)| k == name).map(|(_, v)| v.into_owned()).unwrap_or_default()
}

fn queries(api: &MockApi, endpoint: &str) -> Vec<(String, String, String)> {
    api.calls_to("GET", &format!("{PROM}/{endpoint}"))
        .iter()
        .map(|c| (param(c, "query"), param(c, "start"), param(c, "end")))
        .collect()
}

async fn start(api: &MockApi, tag: &str, scope: TraceScope) -> (Trace, PathBuf) {
    let dir = scratch(tag);
    let trace = Trace::start(api.client(), "acme-uat", scope, &dir.join("t.trace"), false, DEFAULT_PULL_EVERY_SECS).await.unwrap();
    (trace, dir)
}

fn rows(trace: &Trace, sql: &str) -> Vec<Vec<Value>> {
    let file = trace.file.lock().unwrap();
    let mut stmt = file.conn().prepare(sql).unwrap();
    let n = stmt.column_count();
    stmt.query_map([], |r| {
        Ok((0..n)
            .map(|i| match r.get_ref(i).unwrap() {
                rusqlite::types::ValueRef::Null => Value::Null,
                rusqlite::types::ValueRef::Integer(v) => json!(v),
                rusqlite::types::ValueRef::Real(v) => json!(v),
                rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                rusqlite::types::ValueRef::Blob(_) => Value::Null,
            })
            .collect())
    })
    .unwrap()
    .map(rusqlite::Result::unwrap)
    .collect()
}

// --- Pure parts ---------------------------------------------------------------

#[test]
fn durations_and_alignment() {
    assert_eq!(parse_duration("2h"), Some(7200.0));
    assert_eq!(parse_duration("1h30m"), Some(5400.0));
    assert_eq!(parse_duration("15d"), Some(15.0 * 86400.0));
    assert_eq!(parse_duration("1w1y"), Some(8.0 * 86400.0 + 365.0 * 86400.0 - 86400.0));
    assert_eq!(parse_duration("500ms"), Some(0.5));
    assert_eq!(parse_duration("0s"), Some(0.0));
    for bad in ["", "h", "2", "2x", "2h3"] {
        assert_eq!(parse_duration(bad), None, "{bad}");
    }
    assert_eq!((align_down(119), align_down(120), align_down(-1)), (60, 120, -60));
    assert_eq!((align_up(61), align_up(120)), (120, 120));
}

#[test]
fn pulls_come_as_chosen_within_half_the_retention() {
    assert_eq!(PULL_EVERY_CHOICES.map(|c| effective_pull_every(c, 7200.0)), [600, 1200, 1800]);
    // 40 minutes kept: 30 would risk losing data between pulls.
    assert_eq!(effective_pull_every(1800, 2400.0), 1200);
    assert_eq!(effective_pull_every(600, 60.0), 60);
}

#[tokio::test]
async fn pulling_more_than_once_a_minute_is_refused() {
    let api = MockApi::new();
    let dir = scratch("every");
    let err = Trace::start(api.client(), "c", TraceScope::Cluster, &dir.join("t.trace"), false, 30).await.err().unwrap();
    assert_eq!(err.to_string(), "pull at most once a minute");
    assert!(!dir.join("t.trace").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn scopes_become_label_matchers() {
    assert_eq!(selectors(&TraceScope::Cluster), vec![Selector { pods: String::new(), namespaces: String::new() }]);
    let ns = selectors(&TraceScope::Namespaces(vec!["shop".into(), "a.b".into()]));
    assert_eq!(ns[0].pods, r#"namespace=~"shop|a\\.b""#);
    assert_eq!(ns[0].namespaces, ns[0].pods);
    let pod = selectors(&TraceScope::Pod { namespace: "shop".into(), name: "web-1".into() });
    assert_eq!(pod[0].pods, r#"namespace="shop",pod="web-1""#);
    // Workloads are grouped by namespace, one regex alternative each.
    let mut other = deploy("api");
    other.namespace = "ops".into();
    let ws = selectors(&TraceScope::Workloads(vec![deploy("web"), other, deploy("cart")]));
    assert_eq!(ws.len(), 2);
    assert_eq!(ws[0].namespaces, r#"namespace="ops""#);
    assert_eq!(ws[1].pods, r#"namespace="shop",pod=~"(web-[a-z0-9]{1,10}-[a-z0-9]{5})|(cart-[a-z0-9]{1,10}-[a-z0-9]{5})""#);
    assert_eq!(matchers(&["", "a=\"b\"", "", "c!=\"\""]), r#"a="b",c!="""#);
}

#[test]
fn empty_or_unsupported_scopes_are_refused() {
    for scope in [
        TraceScope::Namespaces(vec![]),
        TraceScope::Workloads(vec![]),
        TraceScope::Pod { namespace: "shop".into(), name: String::new() },
    ] {
        assert!(validate(&scope).unwrap_err().to_string().contains("nothing to trace"));
    }
    let mut cm = deploy("x");
    cm.kind = kind("", "v1", "ConfigMap", "configmaps", true);
    assert_eq!(validate(&TraceScope::Workloads(vec![cm])).unwrap_err().to_string(), "can't trace a ConfigMap");
    assert!(validate(&TraceScope::Cluster).is_ok());
}

#[test]
fn raw_points_fold_into_minutes() {
    let k = key("web-1", "app");
    let raw = Raw {
        cpu: vec![(k.clone(), vec![(60, 1.0), (75, 3.0), (90, 2.0), (105, 6.0), (120, 5.0)])],
        memory: vec![(k.clone(), vec![(60, 100.0), (120, 200.0)])],
        throttled: vec![(k.clone(), vec![(60, 0.1), (75, 0.4)]), (key("idle-1", "app"), vec![(60, 0.5)])],
        ..Raw::default()
    };
    let batch = build_batch(&raw, &mut Memory::default());
    assert_eq!(batch.source, Some(Source::Prometheus));
    let stat = |min, avg, max| Some(Stat { min, avg, max });
    assert_eq!(batch.usage, vec![
        (k.clone(), Bucket { t: 60, cpu: stat(1.0, 3.0, 6.0), memory: stat(100.0, 100.0, 100.0), throttled: Some(0.4) }),
        (k, Bucket { t: 120, cpu: stat(5.0, 5.0, 5.0), memory: stat(200.0, 200.0, 200.0), throttled: None }),
    ]);
}

#[test]
fn resources_are_stored_as_changes() {
    let k = key("web-1", "app");
    let series = |resource: &str, pts: &[(i64, f64)]| ((k.clone(), resource.to_owned()), pts.to_vec());
    let raw = Raw {
        requests: vec![series("cpu", &[(60, 0.25), (120, 0.25), (180, 0.5)]), series("memory", &[(60, 1e8), (120, 1e8), (180, 1e8)])],
        limits: vec![series("memory", &[(60, 2e8), (120, 2e8), (180, 2e8)]), series("gpu", &[(60, 1.0)])],
        ..Raw::default()
    };
    let mut memory = Memory::default();
    let batch = build_batch(&raw, &mut memory);
    let first = Resources { cpu_request: Some(250.0), cpu_limit: None, mem_request: Some(1e8), mem_limit: Some(2e8) };
    let second = Resources { cpu_request: Some(500.0), ..first };
    assert_eq!(batch.resources, vec![(k.clone(), 60, first), (k.clone(), 180, second)]);
    // An overlapping pull of the same points writes nothing new.
    assert!(build_batch(&raw, &mut memory).resources.is_empty());
}

#[test]
fn counters_become_events() {
    assert_eq!(increases(&[(60, 3.0), (120, 3.0), (180, 5.0), (240, 0.0), (300, 1.0)], None), (vec![(180, 2), (300, 1)], Some((300, 1.0))));
    // Points up to the last one seen are skipped; the last value is the baseline.
    assert_eq!(increases(&[(60, 3.0), (120, 4.0), (180, 6.0)], Some((120, 4.0))), (vec![(180, 2)], Some((180, 6.0))));
    assert_eq!(increases(&[], Some((1, 1.0))), (vec![], Some((1, 1.0))));

    let k = key("web-1", "app");
    let raw = Raw {
        restarts: vec![(k.clone(), vec![(60, 0.0), (120, 1.0), (180, 2.0)])],
        reasons: vec![
            ((k.clone(), "Error".into()), vec![(120, 1.0)]),
            ((k.clone(), "OOMKilled".into()), vec![(180, 1.0), (240, 0.0)]),
        ],
        oom: vec![(k.clone(), vec![(60, 0.0), (180, 1.0)])],
        ..Raw::default()
    };
    let mut memory = Memory::default();
    let events: Vec<Event> = build_batch(&raw, &mut memory).events.into_iter().map(|(_, e)| e).collect();
    assert_eq!(events, vec![
        Event { t: 120, kind: EventKind::Restart, reason: Some("Error".into()), count: 1 },
        Event { t: 180, kind: EventKind::Restart, reason: Some("OOMKilled".into()), count: 1 },
        Event { t: 180, kind: EventKind::Oom, reason: None, count: 1 },
    ]);
    assert!(build_batch(&raw, &mut memory).events.is_empty());
}

#[test]
fn owners_resolve_to_workloads() {
    let map = |entries: &[(&str, &str, &str)]| -> HashMap<(String, String), (String, String)> {
        entries.iter().map(|(n, k, o)| (("shop".to_owned(), n.to_string()), (k.to_string(), o.to_string()))).collect()
    };
    let rs = map(&[("web-7d9f", "Deployment", "web"), ("bare-rs", "<none>", "<none>")]);
    let jobs = map(&[("nightly-29861700", "CronJob", "nightly")]);
    let resolve = |kind: &str, name: &str| resolve_owner("shop", "the-pod", kind, name, &rs, &jobs);
    let owner = |kind: &str, name: &str, revision: Option<&str>| Owner { kind: kind.into(), name: name.into(), revision: revision.map(Into::into) };
    assert_eq!(resolve("ReplicaSet", "web-7d9f"), owner("Deployment", "web", Some("7d9f")));
    assert_eq!(resolve("ReplicaSet", "bare-rs"), owner("ReplicaSet", "bare-rs", None));
    assert_eq!(resolve("Job", "nightly-29861700"), owner("CronJob", "nightly", None));
    assert_eq!(resolve("Job", "once"), owner("Job", "once", None));
    assert_eq!(resolve("StatefulSet", "db"), owner("StatefulSet", "db", None));
    assert_eq!(resolve("<none>", "<none>"), owner("Pod", "the-pod", None));

    let raw = Raw {
        pod_owners: [(("shop".into(), "web-7d9f-abcde".into()), ("ReplicaSet".into(), "web-7d9f".into()))].into(),
        rs_owners: rs.clone(),
        ..Raw::default()
    };
    let batch = build_batch(&raw, &mut Memory::default());
    assert_eq!(batch.owners, vec![(("shop".into(), "web-7d9f-abcde".into()), owner("Deployment", "web", Some("7d9f")))]);
}

#[test]
fn series_reads_matrices_and_vectors() {
    let data = json!({"result": [
        {"metric": {"namespace": "shop", "pod": "a"}, "values": [[60, "1"], [75.4, "NaN"], [90, "x"], [105, "2"]]},
        {"metric": {"namespace": "shop"}, "values": [[60, "1"]]},
        {"metric": {"namespace": "shop", "pod": "b"}, "value": [120, "3"]},
    ]});
    assert_eq!(series(&data, &["namespace", "pod"]), vec![
        (vec!["shop".into(), "a".into()], vec![(60, 1.0), (105, 2.0)]),
        (vec!["shop".into(), "b".into()], vec![(120, 3.0)]),
    ]);
    assert!(series(&json!({}), &["pod"]).is_empty());
}

#[test]
fn pods_tell_their_owner_and_resources() {
    let p: Pod = serde_json::from_value(pod("shop", "web-7d9f-abcde", "n1", json!({
        "metadata": {"labels": {"pod-template-hash": "7d9f"},
                     "ownerReferences": [{"apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "web-7d9f", "uid": "u", "controller": true}]},
        "spec": {"containers": [{"name": "app", "resources": {"requests": {"cpu": "250m", "memory": "128Mi"}, "limits": {"memory": "256Mi"}}}]}
    })))
    .unwrap();
    let local = local_pod(&p);
    assert_eq!(local.owner, Some(Owner { kind: "Deployment".into(), name: "web".into(), revision: Some("7d9f".into()) }));
    assert_eq!(local.node.as_deref(), Some("n1"));
    assert_eq!(local.resources, vec![("app".into(), Resources {
        cpu_request: Some(250.0),
        cpu_limit: None,
        mem_request: Some(128.0 * 1024.0 * 1024.0),
        mem_limit: Some(256.0 * 1024.0 * 1024.0),
    })]);

    let bare: Pod = serde_json::from_value(pod("shop", "debug", "n1", json!({}))).unwrap();
    assert_eq!(pod_owner(&bare), Owner { kind: "Pod".into(), name: "debug".into(), revision: None });
    let job: Pod = serde_json::from_value(pod("shop", "nightly-29861700-x1y2z", "n1", json!({"metadata": {
        "ownerReferences": [{"apiVersion": "batch/v1", "kind": "Job", "name": "nightly-29861700", "uid": "u", "controller": true}]}})))
    .unwrap();
    let owner = pod_owner(&job);
    let mut cron = deploy("nightly");
    cron.kind = kind("batch", "v1", "CronJob", "cronjobs", true);
    assert!(owned_by(&owner, &cron));
    assert!(!owned_by(&owner, &deploy("nightly")));
}

// --- Against a fake API -------------------------------------------------------

#[tokio::test]
async fn start_backfills_in_chunks_within_retention() {
    let api = MockApi::new();
    with_prometheus(&api);
    let (trace, dir) = start(&api, "backfill", TraceScope::Namespaces(vec!["shop".into()])).await;
    assert!(trace.has_prometheus());
    assert_eq!(trace.scope(), &TraceScope::Namespaces(vec!["shop".into()]));
    assert_eq!(trace.pull_every(), Duration::from_secs(600));
    let cpu: Vec<_> = queries(&api, "query_range").into_iter().filter(|q| q.0.contains("container_cpu_usage_seconds_total")).collect();
    // 1 h 55 min back: an hour, then the rest.
    assert_eq!(cpu.len(), 2);
    let (s0, e0): (i64, i64) = (cpu[0].1.parse().unwrap(), cpu[0].2.parse().unwrap());
    let (s1, e1): (i64, i64) = (cpu[1].1.parse().unwrap(), cpu[1].2.parse().unwrap());
    assert_eq!(s0 % 60, 0);
    assert_eq!(e0, s0 + CHUNK_SECS - RAW_STEP_SECS);
    assert_eq!(s1, s0 + CHUNK_SECS);
    // Both ends sit on minutes, so the span is short by up to a minute.
    let span = e1 + RAW_STEP_SECS - s0;
    let full = 7200 - RETENTION_MARGIN_SECS;
    assert!((full - 60..=full).contains(&span), "{span}");
    assert!(cpu[0].0.contains(r#"namespace=~"shop",container!="",container!="POD""#), "{}", cpu[0].0);
    let s = trace.summary(true).await.unwrap();
    assert_eq!((s.prometheus.as_deref(), s.retention_secs), (Some("monitoring/prometheus-server"), Some(7200.0)));
    assert_eq!(rows(&trace, "SELECT COUNT(*) FROM pulls WHERE ok = 1"), vec![vec![json!(2)]]);
    // Each chunk's eleven responses are counted.
    let pulled = rows(&trace, "SELECT SUM(bytes), MIN(bytes) FROM pulls");
    assert_eq!(pulled[0][0], json!(s.pulled_bytes));
    assert!(pulled[0][1].as_i64().unwrap() > 11 * 20, "{pulled:?}");
    assert_eq!(s.pull_every, Some(600.0));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn pulls_write_rows_and_overlap() {
    let api = MockApi::new();
    with_prometheus(&api);
    let (trace, dir) = start(&api, "pull", TraceScope::Workloads(vec![deploy("web")])).await;
    let pod = json!({"namespace": "shop", "pod": "web-7d9f-abcde", "container": "app"});
    let at = |offset: i64| NOW - 600 + offset;
    api.on_query("GET", &format!("{PROM}/query_range"), "container_cpu_usage_seconds_total", matrix(vec![
        json!({"metric": pod, "values": points(&[(at(0), 10.0), (at(15), 30.0), (at(60), 5.0)])}),
    ]));
    api.on_query("GET", &format!("{PROM}/query_range"), "container_memory_working_set_bytes", matrix(vec![
        json!({"metric": pod, "values": points(&[(at(0), 1e6), (at(60), 2e6)])}),
    ]));
    let mut cpu_req = pod.clone();
    cpu_req["resource"] = json!("cpu");
    api.on_query("GET", &format!("{PROM}/query_range"), "kube_pod_container_resource_requests", matrix(vec![
        json!({"metric": cpu_req, "values": points(&[(at(0), 0.1)])}),
    ]));
    api.on_query("GET", &format!("{PROM}/query"), "kube_pod_owner", vector(vec![
        json!({"metric": {"namespace": "shop", "pod": "web-7d9f-abcde", "owner_kind": "ReplicaSet", "owner_name": "web-7d9f"}, "value": [NOW, "1"]}),
    ]));
    api.on_query("GET", &format!("{PROM}/query"), "kube_replicaset_owner", vector(vec![
        json!({"metric": {"namespace": "shop", "replicaset": "web-7d9f", "owner_kind": "Deployment", "owner_name": "web"}, "value": [NOW, "1"]}),
    ]));
    trace.state.lock().await.covered_until = NOW - 600;
    let before = api.calls().len();
    let report = trace.pull_at(NOW).await.unwrap();
    assert_eq!((report.start, report.end, report.series), (NOW - 900, NOW, 1));
    assert_eq!(rows(&trace, "SELECT bytes FROM pulls ORDER BY rowid DESC LIMIT 1"), vec![vec![json!(report.bytes)]]);
    let calls = &api.calls()[before..];
    let range = calls.iter().find(|c| param(c, "query").contains("container_cpu_usage_seconds_total")).unwrap();
    assert!(param(range, "query").contains(r#"namespace="shop",pod=~"(web-[a-z0-9]{1,10}-[a-z0-9]{5})""#));
    assert_eq!((param(range, "start"), param(range, "step")), ((NOW - 900).to_string(), "30".into()));
    let owners = calls.iter().find(|c| param(c, "query").contains("kube_replicaset_owner")).unwrap();
    assert!(param(owners, "query").contains(r#"{namespace="shop"}[960s]"#), "{}", param(owners, "query"));

    assert_eq!(
        rows(&trace, "SELECT t, cpu_min, cpu_avg, cpu_max, mem_max, source FROM usage ORDER BY t"),
        vec![vec![json!(at(0)), json!(10000), json!(20000), json!(30000), json!(1_000_000), json!(0)], vec![
            json!(at(60)),
            json!(5000),
            json!(5000),
            json!(5000),
            json!(2_000_000),
            json!(0)
        ]]
    );
    assert_eq!(rows(&trace, "SELECT t, cpu_request, mem_request FROM resources"), vec![vec![json!(at(0)), json!(100.0), Value::Null]]);
    assert_eq!(rows(&trace, "SELECT workload_kind, workload_name, revision FROM pods"), vec![vec![
        json!("Deployment"),
        json!("web"),
        json!("7d9f")
    ]]);
    assert_eq!(trace.state.lock().await.covered_until, NOW);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn falling_behind_retention_leaves_a_gap_and_failures_are_retried() {
    let api = MockApi::new();
    with_prometheus(&api);
    let (trace, dir) = start(&api, "gap", TraceScope::Cluster).await;
    trace.state.lock().await.covered_until = NOW - 10_000;
    trace.pull_at(NOW).await.unwrap();
    let floor = NOW - 7200 + RETENTION_MARGIN_SECS;
    let gaps = trace.summary(true).await.unwrap().gaps;
    assert_eq!(gaps, vec![crate::types::TraceGap {
        start: (NOW - 10_000) as f64,
        end: floor as f64,
        reason: "Prometheus no longer held this data".into()
    }]);

    // Prometheus stops answering: the pull fails, is logged, and nothing moves.
    api.on_error("GET", &format!("{PROM}/query_range"), 503, "ServiceUnavailable", "down");
    trace.state.lock().await.covered_until = NOW - 120;
    let err = trace.pull_at(NOW).await.unwrap_err();
    assert_eq!(err.to_string(), "Prometheus didn't answer");
    assert_eq!(trace.state.lock().await.covered_until, NOW - 120);
    assert_eq!(rows(&trace, "SELECT message FROM pulls WHERE ok = 0"), vec![vec![json!("Prometheus didn't answer")]]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn pods_prometheus_misses_are_sampled_by_the_app() {
    let api = MockApi::new();
    with_prometheus(&api);
    let (trace, dir) = start(&api, "local", TraceScope::Namespaces(vec!["shop".into()])).await;
    let running = |name: &str, node: &str| {
        pod("shop", name, node, json!({"spec": {"containers": [{"name": "app", "resources": {"requests": {"cpu": "100m"}}}]}}))
    };
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![running("web-1", "n1"), running("fargate-1", "fargate-n2")]));
    // Prometheus reports web-1 only.
    api.on_query("GET", &format!("{PROM}/query"), "count+by", vector(vec![json!({"metric": {"namespace": "shop", "pod": "web-1"}, "value": [NOW, "1"]})]));
    // No metrics-server; the kubelet answers.
    let summary = json!({"pods": [{"podRef": {"name": "fargate-1", "namespace": "shop"},
        "containers": [{"name": "app", "cpu": {"usageNanoCores": 20_000_000}, "memory": {"workingSetBytes": 5000}}]}]});
    api.on("GET", "/api/v1/nodes/fargate-n2/proxy/stats/summary", summary);

    assert_eq!(trace.sample_local_at(NOW + 5).await.unwrap(), 1);
    assert_eq!(trace.sample_local_at(NOW + 35).await.unwrap(), 1);
    // Nothing written until the minute is over.
    assert!(rows(&trace, "SELECT * FROM usage").is_empty());
    assert_eq!(api.calls_to("GET", "/api/v1/namespaces/shop/pods").len(), 1, "the pod list is reused for a pull interval");
    assert_eq!(trace.sample_local_at(NOW + 65).await.unwrap(), 1);
    assert_eq!(rows(&trace, "SELECT p.name, u.t, u.cpu_avg, u.mem_max, u.source FROM usage u JOIN containers c ON c.id = u.container_id JOIN pods p ON p.id = c.pod_id"), vec![vec![
        json!("fargate-1"),
        json!(NOW),
        json!(20000),
        json!(5000),
        json!(1)
    ]]);
    assert_eq!(rows(&trace, "SELECT t, cpu_request FROM resources"), vec![vec![json!(NOW), json!(100.0)]]);
    assert_eq!(rows(&trace, "SELECT workload_kind FROM pods"), vec![vec![json!("Pod")]]);
    // Ten minutes on (the pull interval), it's looked up again.
    assert_eq!(trace.sample_local_at(NOW + 605).await.unwrap(), 1);
    assert_eq!(api.calls_to("GET", "/api/v1/namespaces/shop/pods").len(), 2);

    // Stopping writes the unfinished minute and closes the file.
    let s = trace.finish(TraceEnd::Stopped).await.unwrap();
    assert_eq!(s.local_samples, 3);
    assert_eq!(s.end_reason, Some(TraceEnd::Stopped));
    assert!(s.gaps.is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn metrics_server_is_preferred_for_the_apps_samples() {
    let api = MockApi::new();
    let dir = scratch("metrics");
    // No Prometheus at all.
    api.on("GET", "/api/v1/services", list("ServiceList", vec![]));
    let scope = TraceScope::Pod { namespace: "shop".into(), name: "web-1".into() };
    let trace = Trace::start(api.client(), "c", scope, &dir.join("t.trace"), false, DEFAULT_PULL_EVERY_SECS).await.unwrap();
    assert!(!trace.has_prometheus());
    assert_eq!(trace.pull_at(NOW).await.unwrap(), PullReport::default());
    api.on("GET", "/api/v1/namespaces/shop/pods/web-1", pod("shop", "web-1", "n1", json!({})));
    let metrics = object("metrics.k8s.io/v1beta1", "PodMetrics", Some("shop"), "web-1", json!({"containers": [{"name": "app", "usage": {"cpu": "50m", "memory": "1Ki"}}]}));
    api.on("GET", "/apis/metrics.k8s.io/v1beta1/namespaces/shop/pods", json!({"apiVersion": "metrics.k8s.io/v1beta1", "kind": "PodMetricsList", "metadata": {}, "items": [metrics]}));
    assert_eq!(trace.sample_local_at(NOW).await.unwrap(), 1);
    let s = trace.finish(TraceEnd::Quit).await.unwrap();
    assert_eq!(s.local_samples, 1);
    assert!(s.gaps.is_empty(), "no Prometheus, so no failed final pull");
    assert!(api.calls_to("GET", "/api/v1/nodes/n1/proxy/stats/summary").is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_failed_final_pull_is_a_gap() {
    let api = MockApi::new();
    with_prometheus(&api);
    let (trace, dir) = start(&api, "final", TraceScope::Cluster).await;
    api.on_error("GET", &format!("{PROM}/query_range"), 503, "ServiceUnavailable", "down");
    // The Cluster scope lists every pod; there are none.
    api.on("GET", "/api/v1/pods", list("PodList", vec![]));
    let summary = trace.run(async { TraceEnd::ConnectionLost }).await.unwrap();
    assert_eq!(summary.end_reason, Some(TraceEnd::ConnectionLost));
    assert_eq!(summary.gaps.len(), 1);
    assert_eq!(summary.gaps[0].reason, "the last pull before stopping failed");
    let read = read_summary(&dir.join("t.trace")).await.unwrap();
    assert_eq!((read.ended, read.unfinished), (summary.ended, false));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn listing_failures_keep_the_previous_pods() {
    let api = MockApi::new();
    with_prometheus(&api);
    let (trace, dir) = start(&api, "listfail", TraceScope::Workloads(vec![deploy("web")])).await;
    let pods_listed = || api.calls_to("GET", "/api/v1/namespaces/shop/pods").len();
    let local_pods = async || trace.state.lock().await.local.pods.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    // The pods list isn't faked (404): nothing to sample, no error, and not
    // asked again until the next interval.
    assert_eq!(trace.sample_local_at(NOW).await.unwrap(), 0);
    assert_eq!(trace.state.lock().await.local.listed_at, Some(NOW));
    trace.sample_local_at(NOW + 30).await.unwrap();
    assert_eq!(pods_listed(), 1);

    // Prometheus has none of the deployment's pods: the app samples the one in scope.
    let owned = pod("shop", "web-7d9f-abcde", "n1", json!({"metadata": {"labels": {"pod-template-hash": "7d9f"},
        "ownerReferences": [{"apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "web-7d9f", "uid": "u", "controller": true}]}}));
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![owned, pod("shop", "other", "n1", json!({}))]));
    trace.sample_local_at(NOW + 600).await.unwrap();
    assert_eq!(local_pods().await, vec!["web-7d9f-abcde"]);

    // Prometheus not answering keeps that list, rather than sampling every
    // pod, and skips listing pods it couldn't filter.
    api.on_error("GET", &format!("{PROM}/query"), 503, "ServiceUnavailable", "down");
    let before = pods_listed();
    trace.sample_local_at(NOW + 1200).await.unwrap();
    assert_eq!(local_pods().await, vec!["web-7d9f-abcde"]);
    assert_eq!(pods_listed(), before);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn status_follows_pulls_samples_and_gaps() {
    let api = MockApi::new();
    with_prometheus(&api);
    let (trace, dir) = start(&api, "status", TraceScope::Cluster).await;
    let s = trace.status();
    assert_eq!((s.context.as_str(), s.prometheus, s.gaps, s.stopping, s.pull_every), ("acme-uat", true, 0, false, 600.0));
    assert!(s.path.ends_with("t.trace"));
    assert!(s.last_pull.is_some() && s.last_error.is_none());
    // Backfill reaches about two hours before the start; the data runs up to now.
    assert!((s.data_end - s.data_start - 6900.0).abs() <= 60.0, "{s:?}");

    trace.state.lock().await.covered_until = NOW - 10_000;
    trace.pull_at(NOW).await.unwrap();
    assert_eq!(trace.status().gaps, 1);
    assert_eq!(trace.status().data_end, NOW as f64);

    api.on_error("GET", &format!("{PROM}/query_range"), 503, "ServiceUnavailable", "down");
    let _ = trace.pull_at(NOW + 600).await;
    assert_eq!(trace.status().last_error.as_deref(), Some("Prometheus didn't answer"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn targets_list_workloads_and_running_pods() {
    let api = MockApi::new();
    let meta = |kind: &str, items: Vec<Value>| list(kind, items);
    let named = |api_version: &str, kind: &str, name: &str, extra: Value| object(api_version, kind, Some("shop"), name, extra);
    api.on("GET", "/apis/apps/v1/namespaces/shop/deployments", meta("DeploymentList", vec![named("apps/v1", "Deployment", "web", json!({}))]));
    api.on("GET", "/apis/apps/v1/namespaces/shop/statefulsets", meta("StatefulSetList", vec![named("apps/v1", "StatefulSet", "db", json!({}))]));
    api.on("GET", "/apis/apps/v1/namespaces/shop/daemonsets", meta("DaemonSetList", vec![]));
    api.on("GET", "/apis/batch/v1/namespaces/shop/cronjobs", meta("CronJobList", vec![named("batch/v1", "CronJob", "nightly", json!({}))]));
    let owned = json!({"metadata": {"ownerReferences": [{"apiVersion": "batch/v1", "kind": "CronJob", "name": "nightly", "uid": "u"}]}});
    api.on("GET", "/apis/batch/v1/namespaces/shop/jobs", meta("JobList", vec![
        named("batch/v1", "Job", "nightly-29861700", owned),
        named("batch/v1", "Job", "migrate", json!({})),
    ]));
    api.on("GET", "/api/v1/namespaces/shop/pods", meta("PodList", vec![pod("shop", "web-2", "n1", json!({})), pod("shop", "db-0", "n1", json!({}))]));
    let t = targets(&api.client(), "shop").await.unwrap();
    let names: Vec<(&str, &str)> = t.workloads.iter().map(|w| (w.kind.kind.as_str(), w.name.as_str())).collect();
    assert_eq!(names, vec![("StatefulSet", "db"), ("Job", "migrate"), ("CronJob", "nightly"), ("Deployment", "web")]);
    assert_eq!(t.workloads[0].kind.group, "apps");
    assert_eq!(t.pods, vec!["db-0", "web-2"]);
    assert!(api.calls_to("GET", "/api/v1/namespaces/shop/pods")[0].query.contains("status.phase"));
    // One list failing fails the whole listing.
    api.on_error("GET", "/apis/apps/v1/namespaces/shop/daemonsets", 403, "Forbidden", "daemonsets is forbidden");
    assert!(targets(&api.client(), "shop").await.unwrap_err().to_string().contains("forbidden"));
}
