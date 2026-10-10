use super::*;
use crate::testing::{MockApi, object, pod};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const POD: &str = "/api/v1/namespaces/shop/pods/web";
const SVC: &str = "/api/v1/namespaces/shop/services/web";
const PODS: &str = "/api/v1/namespaces/shop/pods";

fn pod_req(remote_port: u16) -> ForwardRequest {
    ForwardRequest { namespace: "shop".into(), source: ForwardSource::Pod { pod: "web".into() }, remote_port, local_port: 0 }
}

fn svc_req(remote_port: u16) -> ForwardRequest {
    ForwardRequest {
        namespace: "shop".into(),
        source: ForwardSource::Service { service: "web".into() },
        remote_port,
        local_port: 0,
    }
}

fn service(spec: Value) -> Value {
    object("v1", "Service", Some("shop"), "web", json!({"spec": spec}))
}

fn ready_pod(name: &str, ready: bool) -> Value {
    pod("shop", name, "node-1", json!({
        "spec": {"containers": [{"name": "app", "image": "acme/app:1",
                                 "ports": [{"name": "http", "containerPort": 8080}]}]},
        "status": {"conditions": [{"type": "Ready", "status": if ready { "True" } else { "False" }}]}
    }))
}

#[tokio::test]
async fn pod_port_options_list_tcp_ports_of_every_container() {
    let api = MockApi::new();
    api.on("GET", POD, pod("shop", "web", "node-1", json!({"spec": {
        "initContainers": [{"name": "proxy", "image": "acme/proxy:1", "ports": [{"containerPort": 15001}]}],
        "containers": [{"name": "app", "image": "acme/app:1", "ports": [
            {"name": "http", "containerPort": 8080, "protocol": "TCP"},
            {"name": "dns", "containerPort": 53, "protocol": "UDP"}
        ]}, {"name": "side", "image": "acme/side:1"}]
    }})));
    let opts = port_options(&api.client(), "shop", &ForwardSource::Pod { pod: "web".into() }).await.unwrap();
    assert_eq!(
        opts,
        [
            PortOption { port: 15001, name: None, detail: Some("proxy".into()) },
            PortOption { port: 8080, name: Some("http".into()), detail: Some("app".into()) },
        ]
    );
}

#[tokio::test]
async fn service_port_options_show_their_targets() {
    let api = MockApi::new();
    api.on("GET", SVC, service(json!({"ports": [
        {"name": "http", "port": 80, "targetPort": 8080},
        {"name": "grpc", "port": 9090, "targetPort": "grpc", "protocol": "TCP"},
        {"port": 81},
        {"name": "dns", "port": 53, "protocol": "UDP"}
    ]})));
    let opts = port_options(&api.client(), "shop", &ForwardSource::Service { service: "web".into() }).await.unwrap();
    assert_eq!(
        opts,
        [
            PortOption { port: 80, name: Some("http".into()), detail: Some("→ 8080".into()) },
            PortOption { port: 9090, name: Some("grpc".into()), detail: Some("→ grpc".into()) },
            PortOption { port: 81, name: None, detail: None },
        ]
    );
    // No ports, and missing objects.
    api.on("GET", "/api/v1/namespaces/shop/services/empty", object("v1", "Service", Some("shop"), "empty", json!({})));
    let none = port_options(&api.client(), "shop", &ForwardSource::Service { service: "empty".into() }).await.unwrap();
    assert!(none.is_empty());
    let err = port_options(&api.client(), "shop", &ForwardSource::Pod { pod: "gone".into() }).await.unwrap_err();
    assert!(matches!(err, Error::Kube(kube::Error::Api(s)) if s.code == 404));
}

#[tokio::test]
async fn binding_a_running_pod() {
    let api = MockApi::new();
    api.on("GET", POD, pod("shop", "web", "node-1", json!({})));
    let (listener, backend) = bind(&api.client(), &pod_req(8080)).await.unwrap();
    assert_eq!(backend, ("web".to_owned(), 8080));
    let addr = listener.local_addr().unwrap();
    assert!(addr.ip().is_loopback() && addr.port() != 0);
}

#[tokio::test]
async fn binding_refuses_pods_that_are_not_running() {
    let api = MockApi::new();
    api.on("GET", POD, pod("shop", "web", "node-1", json!({"status": {"phase": "Pending"}})));
    let err = bind(&api.client(), &pod_req(8080)).await.unwrap_err();
    assert!(matches!(&err, Error::NotFound(m) if m == "pod web isn't running"), "{err}");
}

#[tokio::test]
async fn binding_a_busy_port_fails_clearly() {
    let api = MockApi::new();
    api.on("GET", POD, pod("shop", "web", "node-1", json!({})));
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut req = pod_req(8080);
    req.local_port = taken.local_addr().unwrap().port();
    let err = bind(&api.client(), &req).await.unwrap_err();
    assert!(matches!(&err, Error::NotFound(m) if m.starts_with(&format!("can't listen on 127.0.0.1:{}", req.local_port))), "{err}");
}

#[tokio::test]
async fn services_resolve_to_the_first_ready_pod() {
    let api = MockApi::new();
    api.on("GET", SVC, service(json!({"selector": {"app": "web"}, "ports": [
        {"port": 80, "targetPort": "http"}, {"port": 81, "targetPort": 9000}, {"port": 82}, {"port": 83, "targetPort": "nope"}
    ]})));
    let mut terminating = ready_pod("web-0", true);
    terminating["metadata"]["deletionTimestamp"] = json!("2026-10-10T00:00:00Z");
    api.on_query("GET", PODS, "labelSelector=app%3Dweb", crate::testing::list("PodList", vec![
        ready_pod("web-c", true), ready_pod("web-a", false), terminating, ready_pod("web-b", true),
    ]));
    let client = api.client();
    assert_eq!(resolve(&client, &svc_req(80)).await.unwrap(), ("web-b".to_owned(), 8080));
    assert_eq!(resolve(&client, &svc_req(81)).await.unwrap(), ("web-b".to_owned(), 9000));
    assert_eq!(resolve(&client, &svc_req(82)).await.unwrap(), ("web-b".to_owned(), 82));
    let err = resolve(&client, &svc_req(83)).await.unwrap_err();
    assert!(matches!(&err, Error::NotFound(m) if m == "port named nope in pod web-b"), "{err}");
    let err = resolve(&client, &svc_req(84)).await.unwrap_err();
    assert!(matches!(&err, Error::NotFound(m) if m == "port 84 on service web"), "{err}");
}

#[tokio::test]
async fn services_need_a_selector_and_a_ready_pod() {
    let api = MockApi::new();
    api.on("GET", SVC, service(json!({"selector": {}, "ports": [{"port": 80}]})));
    let err = resolve(&api.client(), &svc_req(80)).await.unwrap_err();
    assert!(matches!(&err, Error::NotFound(m) if m == "pod selector on service web"), "{err}");

    let api = MockApi::new();
    api.on("GET", SVC, service(json!({"selector": {"app": "web"}, "ports": [{"port": 80}]})));
    api.on("GET", PODS, crate::testing::list("PodList", vec![ready_pod("web-a", false)]));
    let err = resolve(&api.client(), &svc_req(80)).await.unwrap_err();
    assert!(matches!(&err, Error::NotFound(m) if m == "ready pod behind service web"), "{err}");
}

#[test]
fn readiness_needs_a_ready_condition() {
    let p: Pod = serde_json::from_value(json!({"metadata": {"name": "a"}})).unwrap();
    assert!(!is_ready(&p));
    assert_eq!(named_port(&p, "http"), None);
}

struct Served {
    stats: Arc<Mutex<ForwardStats>>,
    notified: Arc<AtomicUsize>,
    addr: std::net::SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

async fn start(api: &MockApi, req: ForwardRequest, backend: (String, u16)) -> Served {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let stats = Arc::new(Mutex::new(ForwardStats::default()));
    let notified = Arc::new(AtomicUsize::new(0));
    let n = notified.clone();
    let notify: Notify = Arc::new(move || {
        n.fetch_add(1, Ordering::SeqCst);
    });
    let task = tokio::spawn(serve(api.client(), req, listener, backend, stats.clone(), notify));
    Served { stats, notified, addr, task }
}

/// Opens one connection and waits until it has been handled.
async fn connect_once(s: &Served, total: u64) {
    let _socket = TcpStream::connect(s.addr).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            {
                let st = s.stats.lock().unwrap();
                if st.total == total && st.active == 0 && st.last_error.is_some() {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("connection not handled");
}

#[tokio::test]
async fn pod_connections_report_port_forward_failures() {
    let api = MockApi::new();
    let s = start(&api, pod_req(8080), ("web".into(), 8080)).await;
    connect_once(&s, 1).await;
    {
        let st = s.stats.lock().unwrap();
        assert_eq!(st.backend.as_deref(), Some("web:8080"));
        assert!(!st.last_error.as_deref().unwrap().is_empty());
    }
    // Once when it opened, once when it closed.
    assert_eq!(s.notified.load(Ordering::SeqCst), 2);
    let calls = api.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].path, "/api/v1/namespaces/shop/pods/web/portforward");
    assert!(calls[0].query.contains("ports=8080"), "{}", calls[0].query);
    // Pods keep their backend.
    connect_once(&s, 2).await;
    assert!(api.calls().iter().all(|c| c.path.ends_with("/portforward")));
    s.task.abort();
}

#[tokio::test]
async fn service_connections_pick_a_pod_again_after_a_failure() {
    let api = MockApi::new();
    let s = start(&api, svc_req(80), ("web-old".into(), 8080)).await;
    connect_once(&s, 1).await;
    assert_eq!(s.stats.lock().unwrap().backend.as_deref(), Some("web-old:8080"));

    // The service is gone now: re-resolving fails with the server's message.
    connect_once(&s, 2).await;
    let error = s.stats.lock().unwrap().last_error.clone().unwrap();
    assert!(error.contains("isn't faked"), "{error}");
    assert_eq!(api.calls_to("GET", SVC).len(), 1);

    // Back again with a ready pod: the next connection uses it.
    api.on("GET", SVC, service(json!({"selector": {"app": "web"}, "ports": [{"port": 80, "targetPort": 8080}]})));
    api.on("GET", PODS, crate::testing::list("PodList", vec![ready_pod("web-new", true)]));
    connect_once(&s, 3).await;
    assert_eq!(s.stats.lock().unwrap().backend.as_deref(), Some("web-new:8080"));
    assert_eq!(api.calls_to("GET", "/api/v1/namespaces/shop/pods/web-new/portforward").len(), 1);
    s.task.abort();
}
