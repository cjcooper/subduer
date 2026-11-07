#![allow(dead_code)]
//! Test support: a fake Kubernetes API server. Routes match on method and
//! path (and optionally a query substring) and answer with canned JSON;
//! anything else gets a 404 `Status`, like a real server. Every request is
//! recorded, so tests can check what was sent (patch bodies, dry runs, …).

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, Full};
use kube::Client;
use kube::client::Body;
use serde_json::{Value, json};

#[derive(Clone)]
struct Route {
    method: String,
    path: String,
    query: Option<String>,
    status: u16,
    body: Vec<u8>,
    content_type: &'static str,
}

/// One request the fake server received.
#[derive(Clone, Debug)]
pub struct Call {
    pub method: String,
    pub path: String,
    pub query: String,
    pub body: Vec<u8>,
}

impl Call {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

#[derive(Clone, Default)]
pub struct MockApi {
    routes: Arc<Mutex<Vec<Route>>>,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl MockApi {
    pub fn new() -> Self {
        #[cfg(test)]
        record_all_traces();
        Self::default()
    }

    fn add(&self, method: &str, path: &str, query: Option<&str>, status: u16, body: Vec<u8>, content_type: &'static str) -> &Self {
        self.routes.lock().unwrap().push(Route {
            method: method.into(),
            path: path.into(),
            query: query.map(str::to_owned),
            status,
            body,
            content_type,
        });
        self
    }

    /// Answers `method path` with 200 and `body`.
    pub fn on(&self, method: &str, path: &str, body: Value) -> &Self {
        self.add(method, path, None, 200, body.to_string().into_bytes(), "application/json")
    }

    /// Like [`on`](Self::on), only when the query string contains `query`.
    /// Query routes win over plain ones.
    pub fn on_query(&self, method: &str, path: &str, query: &str, body: Value) -> &Self {
        self.add(method, path, Some(query), 200, body.to_string().into_bytes(), "application/json")
    }

    /// Answers with an API error `Status`.
    pub fn on_error(&self, method: &str, path: &str, code: u16, reason: &str, message: &str) -> &Self {
        self.add(method, path, None, code, status(code, reason, message).to_string().into_bytes(), "application/json")
    }

    /// Answers with plain text (logs, kubelet and Prometheus proxies, …).
    pub fn on_text(&self, method: &str, path: &str, code: u16, text: &str) -> &Self {
        self.add(method, path, None, code, text.as_bytes().to_vec(), "text/plain")
    }

    pub fn client(&self) -> Client {
        let me = self.clone();
        let service = tower::service_fn(move |req: Request<Body>| {
            let me = me.clone();
            async move {
                let (parts, body) = req.into_parts();
                let body = body.collect().await.map(|b| b.to_bytes().to_vec()).unwrap_or_default();
                let path = parts.uri.path().to_owned();
                let query = parts.uri.query().unwrap_or_default().to_owned();
                let method = parts.method.as_str().to_owned();
                me.calls.lock().unwrap().push(Call { method: method.clone(), path: path.clone(), query: query.clone(), body });
                let routes = me.routes.lock().unwrap().clone();
                let found = routes
                    .iter()
                    .rev()
                    .find(|r| r.method == method && r.path == path && r.query.as_ref().is_some_and(|q| query.contains(q.as_str())))
                    .or_else(|| routes.iter().rev().find(|r| r.method == method && r.path == path && r.query.is_none()));
                let (code, body, content_type) = match found {
                    Some(r) => (r.status, r.body.clone(), r.content_type),
                    None => (
                        404,
                        status(404, "NotFound", &format!("{method} {path} isn't faked")).to_string().into_bytes(),
                        "application/json",
                    ),
                };
                let response = Response::builder()
                    .status(code)
                    .header("content-type", content_type)
                    .body(Full::new(Bytes::from(body)))
                    .unwrap();
                Ok::<_, std::convert::Infallible>(response)
            }
        });
        Client::new(service, "default")
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    /// Requests matching `method` and `path`.
    pub fn calls_to(&self, method: &str, path: &str) -> Vec<Call> {
        self.calls().into_iter().filter(|c| c.method == method && c.path == path).collect()
    }
}

/// Turns every `tracing` call site on for the rest of the process, so the
/// field expressions in log lines (`%kind.key()`, `?err`, …) run in tests
/// the way they do in the app, where a subscriber is always installed.
/// Nothing is printed. Called by [`MockApi::new`] in this crate's tests.
pub fn record_all_traces() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Another subscriber may already be set (e.g. by the app); keep it.
        let _ = tracing::subscriber::set_global_default(Quiet);
    });
}

/// A subscriber that accepts everything and formats each field into nothing.
struct Quiet;

struct Discard;

impl tracing::field::Visit for Discard {
    fn record_debug(&mut self, _field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let _ = format!("{value:?}");
    }
}

impl tracing::Subscriber for Quiet {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        span.record(&mut Discard);
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        values.record(&mut Discard);
    }
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        event.record(&mut Discard);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// An API error body, e.g. `status(403, "Forbidden", "pods is forbidden")`.
pub fn status(code: u16, reason: &str, message: &str) -> Value {
    json!({"kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Failure",
           "message": message, "reason": reason, "code": code})
}

/// A list response, e.g. `list("PodList", vec![pod])`.
pub fn list(kind: &str, items: Vec<Value>) -> Value {
    json!({"apiVersion": "v1", "kind": kind, "metadata": {"resourceVersion": "1"}, "items": items})
}

/// A minimal object with metadata, merged with `extra`.
pub fn object(api_version: &str, kind: &str, namespace: Option<&str>, name: &str, extra: Value) -> Value {
    let mut v = json!({"apiVersion": api_version, "kind": kind,
        "metadata": {"name": name, "uid": format!("uid-{name}"), "resourceVersion": "7",
                     "creationTimestamp": "2026-10-01T00:00:00Z"}});
    if let Some(ns) = namespace {
        v["metadata"]["namespace"] = json!(ns);
    }
    merge(&mut v, extra);
    v
}

/// A running pod with one container, on `node`, merged with `extra`.
pub fn pod(namespace: &str, name: &str, node: &str, extra: Value) -> Value {
    let mut v = object("v1", "Pod", Some(namespace), name, json!({
        "spec": {"nodeName": node, "containers": [{"name": "app", "image": "acme/app:1"}]},
        "status": {"phase": "Running", "containerStatuses": [
            {"name": "app", "ready": true, "restartCount": 0, "image": "acme/app:1", "imageID": "", "state": {"running": {}}}]}
    }));
    merge(&mut v, extra);
    v
}

/// Deep-merges `extra` into `v` (objects merge; anything else replaces).
pub fn merge(v: &mut Value, extra: Value) {
    match (v, extra) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, val) in b {
                merge(a.entry(k).or_insert(Value::Null), val);
            }
        }
        (slot, other) => *slot = other,
    }
}

/// A `KindRef` from `group/version/Kind/plural`.
pub fn kind(group: &str, version: &str, kind: &str, plural: &str, namespaced: bool) -> crate::types::KindRef {
    crate::types::KindRef {
        group: group.into(),
        version: version.into(),
        kind: kind.into(),
        plural: plural.into(),
        namespaced,
    }
}

#[tokio::test]
async fn fake_server_routes_and_records() {
    use k8s_openapi::api::core::v1::ConfigMap;
    use kube::Api;
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/configmaps/app", object("v1", "ConfigMap", Some("shop"), "app", json!({"data": {"a": "1"}})));
    let client = api.client();
    let cms: Api<ConfigMap> = Api::namespaced(client, "shop");
    let cm = cms.get("app").await.unwrap();
    assert_eq!(cm.data.unwrap()["a"], "1");
    let err = cms.get("missing").await.unwrap_err();
    assert!(matches!(err, kube::Error::Api(s) if s.code == 404));
    assert_eq!(api.calls().len(), 2);
}

#[test]
fn quiet_subscriber_takes_spans_and_events() {
    record_all_traces();
    let span = tracing::info_span!("outer", a = 1, b = tracing::field::Empty);
    span.record("b", "two");
    let other = tracing::info_span!("other");
    span.follows_from(&other);
    span.in_scope(|| tracing::info!(x = ?vec![1], "inside"));
    assert!(!span.is_disabled());
    let mut ran = false;
    tracing::warn!(x = %{ ran = true; 1 }, "fields run");
    assert!(ran);
}
