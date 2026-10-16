use super::*;
use crate::testing::{MockApi, kind, list, object, pod};
use serde_json::json;
use std::cell::RefCell;
use std::time::Duration;

fn container(pod: &str, container: &str) -> LogTarget {
    LogTarget { namespace: "shop".into(), source: LogSource::Container { pod: pod.into(), container: container.into() }, filter: None }
}

fn workload(name: &str) -> LogTarget {
    LogTarget {
        namespace: "shop".into(),
        source: LogSource::Workload { kind: kind("apps", "v1", "Deployment", "deployments", true), name: name.into() },
        filter: None,
    }
}

/// Runs `stream` to the end and returns every batch it emitted.
async fn batches(client: Client, target: LogTarget, previous: bool) -> (Result<()>, Vec<LogBatch>) {
    let out = RefCell::new(vec![]);
    let result = tokio::time::timeout(Duration::from_secs(10), stream(client, target, previous, |b| out.borrow_mut().push(b)))
        .await
        .expect("logs didn't end");
    (result, out.into_inner())
}

fn lines(batches: &[LogBatch]) -> Vec<LogLine> {
    batches.iter().flat_map(|b| b.lines.clone()).collect()
}

fn notices(batches: &[LogBatch]) -> Vec<String> {
    batches.iter().flat_map(|b| b.notices.clone()).collect()
}

const WEB_LOG: &str = "/api/v1/namespaces/shop/pods/web/log";

#[tokio::test]
async fn one_container_follows_with_timestamps() {
    let api = MockApi::new();
    api.on_text(
        "GET",
        WEB_LOG,
        200,
        "2026-10-10T00:00:01.000000000Z \u{1b}[32mINFO\u{1b}[0m ready\n2026-10-10T00:00:02.000000000Z second\nplain\n",
    );
    let (result, out) = batches(api.client(), container("web", "app"), false).await;
    result.unwrap();
    let got = lines(&out);
    assert_eq!(got.len(), 3);
    assert_eq!(got[0], LogLine { source: None, time: Some("2026-10-10T00:00:01.000000000Z".into()), text: "INFO ready".into() });
    assert_eq!(got[2].time, None);
    assert_eq!(got[2].text, "plain");
    // Single containers say nothing when their stream ends; the window does.
    assert_eq!(notices(&out), ["All streams ended."]);
    let call = &api.calls_to("GET", WEB_LOG)[0];
    for part in ["container=app", "follow=true", "tailLines=1000", "timestamps=true"] {
        assert!(call.query.contains(part), "{} lacks {part}", call.query);
    }
}

#[tokio::test]
async fn previous_logs_read_the_last_container() {
    let api = MockApi::new();
    api.on_text("GET", WEB_LOG, 200, "boom\n");
    let (result, out) = batches(api.client(), container("web", "app"), true).await;
    result.unwrap();
    assert_eq!(lines(&out)[0].text, "boom");
    assert_eq!(notices(&out), ["End of previous logs."]);
    let call = &api.calls_to("GET", WEB_LOG)[0];
    assert!(call.query.contains("previous=true") && !call.query.contains("follow=true"), "{}", call.query);
}

#[tokio::test]
async fn log_errors_become_notices() {
    let api = MockApi::new();
    api.on_error("GET", WEB_LOG, 400, "BadRequest", "container \"app\" in pod \"web\" is waiting to start");
    let (result, out) = batches(api.client(), container("web", "app"), false).await;
    result.unwrap();
    assert!(lines(&out).is_empty());
    assert_eq!(notices(&out), ["app: container \"app\" in pod \"web\" is waiting to start", "All streams ended."]);
}

#[tokio::test]
async fn big_bursts_are_split_into_batches() {
    let api = MockApi::new();
    let text: String = (0..2500).map(|i| format!("line {i}\n")).collect();
    api.on_text("GET", WEB_LOG, 200, &text);
    let (result, out) = batches(api.client(), container("web", "app"), false).await;
    result.unwrap();
    assert_eq!(lines(&out).len(), 2500);
    assert!(out.iter().all(|b| b.lines.len() <= MAX_BATCH));
    assert!(out.iter().filter(|b| !b.lines.is_empty()).count() >= 2);
    // Order is kept for one container.
    assert_eq!(lines(&out)[2499].text, "line 2499");
}

const DEPLOY: &str = "/apis/apps/v1/namespaces/shop/deployments/web";
const PODS: &str = "/api/v1/namespaces/shop/pods";

fn deployment(selector: serde_json::Value) -> serde_json::Value {
    object("apps/v1", "Deployment", Some("shop"), "web", json!({"spec": {"selector": selector}}))
}

#[tokio::test]
async fn workloads_merge_every_container_by_time() {
    let api = MockApi::new();
    api.on("GET", DEPLOY, deployment(json!({"matchLabels": {"app": "web"}})));
    let with_sidecar = pod("shop", "web-a", "node-1", json!({"spec": {"initContainers": [
        {"name": "proxy", "image": "acme/proxy:1", "restartPolicy": "Always"},
        {"name": "migrate", "image": "acme/app:1"}
    ]}}));
    let terminating = pod("shop", "web-old", "node-1", json!({"metadata": {"deletionTimestamp": "2026-10-10T00:00:00Z"}}));
    let plain = pod("shop", "web-b", "node-2", json!({}));
    api.on_query("GET", PODS, "labelSelector=app%3Dweb", list("PodList", vec![plain, terminating, with_sidecar]));
    api.on_text("GET", "/api/v1/namespaces/shop/pods/web-a/log", 200, "2026-10-10T00:00:03.000000000Z a3\n2026-10-10T00:00:01.000000000Z a1\n");
    api.on_text("GET", "/api/v1/namespaces/shop/pods/web-b/log", 200, "2026-10-10T00:00:02.000000000Z b2\n");
    let (result, out) = batches(api.client(), workload("web"), false).await;
    result.unwrap();

    let all = notices(&out);
    assert_eq!(all[0], "Following 3 containers in 2 pods (pods started later aren't added).");
    for label in ["web-a/proxy", "web-a/app", "web-b/app"] {
        assert!(all.contains(&format!("{label}: stream ended")), "{all:?}");
    }
    assert_eq!(all.last().unwrap(), "All streams ended.");
    assert!(api.calls_to("GET", "/api/v1/namespaces/shop/pods/web-old/log").is_empty());
    let log_calls = api.calls_to("GET", "/api/v1/namespaces/shop/pods/web-a/log");
    assert_eq!(log_calls.len(), 2);
    assert!(log_calls.iter().all(|c| c.query.contains("tailLines=200")));
    let got = lines(&out);
    assert_eq!(got.len(), 5);
    assert!(got.iter().all(|l| l.source.as_deref().is_some_and(|s| s.starts_with("web-"))));
    // Within each batch, lines are sorted by time.
    for b in &out {
        assert!(b.lines.windows(2).all(|w| w[0].time <= w[1].time), "{:?}", b.lines);
    }
}

#[tokio::test]
async fn workloads_without_selector_or_pods() {
    let api = MockApi::new();
    api.on("GET", DEPLOY, deployment(json!(null)));
    let (result, out) = batches(api.client(), workload("web"), false).await;
    result.unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].notices, ["Deployment web has no pod selector.", "No running containers to follow."]);

    let api = MockApi::new();
    api.on("GET", DEPLOY, deployment(json!({"app": "web"})));
    api.on("GET", PODS, list("PodList", vec![pod("shop", "web-a", "node-1", json!({}))]));
    api.on_text("GET", "/api/v1/namespaces/shop/pods/web-a/log", 200, "");
    let (_, out) = batches(api.client(), workload("web"), false).await;
    assert_eq!(notices(&out)[0], "Following 1 container in 1 pod (pods started later aren't added).");

    let api = MockApi::new();
    api.on("GET", DEPLOY, deployment(json!({"matchLabels": {"app": "web"}})));
    api.on("GET", PODS, list("PodList", vec![]));
    let (_, out) = batches(api.client(), workload("web"), false).await;
    assert_eq!(out[0].notices, ["Following 0 containers in 0 pods (pods started later aren't added).", "No running containers to follow."]);
}

#[tokio::test]
async fn workloads_follow_at_most_twenty_pods() {
    let api = MockApi::new();
    api.on("GET", DEPLOY, deployment(json!({"matchLabels": {"app": "web"}})));
    let pods: Vec<_> = (0..25).map(|i| pod("shop", &format!("web-{i:02}"), "node-1", json!({}))).collect();
    api.on("GET", PODS, list("PodList", pods));
    let (result, out) = batches(api.client(), workload("web"), false).await;
    result.unwrap();
    let all = notices(&out);
    assert_eq!(all[0], "Following the first 20 of 25 pods.");
    assert!(all[1].starts_with("Following 20 containers in 20 pods"));
    // Pods sort by name; the last five are left out.
    assert!(api.calls_to("GET", "/api/v1/namespaces/shop/pods/web-19/log").len() == 1);
    assert!(api.calls_to("GET", "/api/v1/namespaces/shop/pods/web-20/log").is_empty());
    // Their logs 404, which shows as a notice per container.
    assert!(all.iter().any(|n| n.starts_with("web-00/app: ")), "{all:?}");
}

#[tokio::test]
async fn missing_workload_is_an_error() {
    let api = MockApi::new();
    let (result, out) = batches(api.client(), workload("web"), false).await;
    assert!(matches!(result, Err(crate::Error::Kube(kube::Error::Api(s))) if s.code == 404));
    assert!(out.is_empty());
}

#[test]
fn selector_forms() {
    let sel = json!({"matchLabels": {"app": "web"}, "matchExpressions": [
        {"key": "env", "operator": "NotIn", "values": ["dev"]},
        {"key": "tier", "operator": "Exists"},
    ]});
    assert_eq!(label_selector(&sel).as_deref(), Some("app=web,env notin (dev),tier"));
    assert_eq!(label_selector(&json!({"matchExpressions": [{"key": "a", "operator": "Gt"}]})), None);
    assert_eq!(label_selector(&json!({"matchExpressions": [{"operator": "Exists"}]})), None);
    assert_eq!(label_selector(&json!({"matchExpressions": [{"key": "a"}]})), None);
    assert_eq!(label_selector(&json!({"app": 3})), None);
    assert_eq!(label_selector(&json!("app=web")), None);
    assert_eq!(label_selector(&json!({"matchLabels": {}})), None);
}

#[test]
fn text_helpers() {
    assert_eq!(plural(1), "");
    assert_eq!(plural(0), "s");
    // Too short, or not a date, is kept as text.
    assert_eq!(split_timestamp("2026-10-10Z x"), (None, "2026-10-10Z x"));
    assert_eq!(split_timestamp("aaaaaaaaaaaaaaaaaaaaaaZ x"), (None, "aaaaaaaaaaaaaaaaaaaaaaZ x"));
    assert_eq!(split_timestamp("nospace"), (None, "nospace"));
    // Non-CSI escapes drop one character; unterminated CSI drops the rest.
    assert_eq!(strip_ansi("a\u{1b}7b"), "ab");
    assert_eq!(strip_ansi("a\u{1b}[1;3"), "a");
    assert_eq!(strip_ansi("end\u{1b}"), "end");
}

#[test]
fn pod_streams_need_a_spec() {
    let bare: Pod = serde_json::from_value(json!({"metadata": {"name": "x"}})).unwrap();
    assert!(pod_streams(&bare).is_empty());
}

#[test]
fn flush_skips_empty_batches() {
    let emitted = RefCell::new(0);
    let mut batch = LogBatch::default();
    flush(&mut batch, true, &|_| *emitted.borrow_mut() += 1);
    assert_eq!(*emitted.borrow(), 0);
    batch.notices.push("x".into());
    flush(&mut batch, false, &|_| *emitted.borrow_mut() += 1);
    assert_eq!(*emitted.borrow(), 1);
    assert!(batch.notices.is_empty());
}
