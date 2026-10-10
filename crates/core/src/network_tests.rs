//! Coverage tests for the Network section's Details: every branch of the
//! describe functions, and the related lookups against the fake API server.

use super::*;
use crate::config::describe as describe_object;
use crate::testing::{MockApi, list, object, pod};
use serde_json::json;

const SLICES: &str = "/apis/discovery.k8s.io/v1/namespaces/shop/endpointslices";
const INGRESSES: &str = "/apis/networking.k8s.io/v1/namespaces/shop/ingresses";
const SERVICES: &str = "/api/v1/namespaces/shop/services";
const SECRETS: &str = "/api/v1/namespaces/shop/secrets";
const PODS: &str = "/api/v1/namespaces/shop/pods";

fn now() -> jiff::Timestamp {
    "2026-10-10T12:00:00Z".parse().unwrap()
}

fn row<'a>(d: &'a ObjectDetails, key: &str) -> Option<&'a str> {
    d.info.iter().find(|r| r.key == key).map(|r| r.value.as_str())
}

fn info<'a>(d: &'a ObjectDetails, key: &str) -> &'a InfoRow {
    d.info.iter().find(|r| r.key == key).unwrap_or_else(|| panic!("no {key} row"))
}

fn section<'a>(d: &'a ObjectDetails, id: &str) -> &'a InfoSection {
    d.sections.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no {id} section"))
}

fn table<'a>(d: &'a ObjectDetails, id: &str) -> &'a InfoTable {
    section(d, id).table.as_ref().unwrap()
}

fn cells(t: &InfoTable) -> Vec<Vec<&str>> {
    t.rows.iter().map(|r| r.cells.iter().map(String::as_str).collect()).collect()
}

fn slice(name: &str, service: Option<&str>, endpoints: Value) -> Value {
    let mut extra = json!({"addressType": "IPv4", "endpoints": endpoints});
    if let Some(s) = service {
        extra["metadata"] = json!({"labels": {"kubernetes.io/service-name": s}});
    }
    object("discovery.k8s.io/v1", "EndpointSlice", Some("shop"), name, extra)
}

fn endpoint(ip: &str, ready: bool, pod_name: &str) -> Value {
    json!({"addresses": [ip], "conditions": {"ready": ready}, "nodeName": "n1",
           "targetRef": {"kind": "Pod", "name": pod_name, "namespace": "shop"}})
}

fn service_value(name: &str, spec: Value) -> Value {
    object("v1", "Service", Some("shop"), name, json!({"spec": spec}))
}

async fn related_details(client: &Client, key: &str, v: &Value) -> Result<ObjectDetails> {
    let mut d = describe_object(key, v, now());
    let name = d.name.clone();
    let ns = d.namespace.clone().unwrap_or_default();
    related(client, key, &ns, &name, v, &mut d).await?;
    Ok(d)
}

// ---- Service ----

#[test]
fn load_balancer_service_with_every_option() {
    let mut v = service_value("edge", json!({
        "type": "LoadBalancer", "clusterIP": "10.0.0.5", "clusterIPs": ["10.0.0.5", "fd00::5"],
        "loadBalancerClass": "acme.io/lb", "externalIPs": ["192.0.2.10", "192.0.2.11"],
        "sessionAffinity": "ClientIP", "sessionAffinityConfig": {"clientIP": {"timeoutSeconds": 600}},
        "internalTrafficPolicy": "Local", "publishNotReadyAddresses": true, "ipFamilies": ["IPv4", "IPv6"],
        "ports": [{"port": 443},
                  {"name": "dns", "port": 53, "targetPort": 5353, "nodePort": 30053, "protocol": "UDP", "appProtocol": "dns"}]}));
    v["status"] = json!({"loadBalancer": {"ingress": [{"hostname": "edge.elb.acme.io"}, {"ip": "198.51.100.7"}, {}]}});
    let d = describe_object("core/Service", &v, now());
    assert_eq!(row(&d, "Type"), Some("LoadBalancer"));
    assert_eq!(row(&d, "Cluster IP"), Some("10.0.0.5, fd00::5"));
    assert!(info(&d, "Cluster IP").mono);
    assert_eq!(row(&d, "Load balancer"), Some("edge.elb.acme.io, 198.51.100.7"));
    assert!(info(&d, "Load balancer").mono);
    assert_eq!(row(&d, "Load balancer class"), Some("acme.io/lb"));
    assert_eq!(row(&d, "External IPs"), Some("192.0.2.10, 192.0.2.11"));
    assert_eq!(row(&d, "Selector"), Some("none: its endpoints are managed by hand or by a controller"));
    assert_eq!(row(&d, "Session affinity"), Some("by client IP, for 600s"));
    assert_eq!(row(&d, "External traffic"), Some("Cluster: any node"));
    assert_eq!(row(&d, "Internal traffic"), Some("Local: only pods on the caller's node"));
    assert_eq!(row(&d, "Not-ready pods"), Some("published too"));
    assert_eq!(row(&d, "IP families"), Some("IPv4, IPv6"));
    // With an address the status is left to the endpoints.
    assert_eq!(d.status, None);
    let ports = table(&d, "ports");
    assert_eq!(
        cells(ports),
        [vec!["—", "443", "443", "—", "TCP", ""], vec!["dns", "53", "5353", "30053", "UDP", "dns"]]
    );
    assert_eq!((ports.numeric.clone(), ports.mono.clone()), (vec![1, 3], vec![2]));
    assert_eq!(section(&d, "ports").summary, "2");
}

#[test]
fn node_port_and_plain_services() {
    let v = service_value("np", json!({"type": "NodePort", "clusterIP": "10.0.0.9", "sessionAffinity": "ClientIP",
        "selector": {"app": "np", "tier": "web"}, "ipFamilies": ["IPv4"]}));
    let d = describe_object("core/Service", &v, now());
    // No timeout given: the API's default of three hours.
    assert_eq!(row(&d, "Session affinity"), Some("by client IP, for 10800s"));
    assert_eq!(row(&d, "External traffic"), Some("Cluster: any node"));
    assert_eq!(row(&d, "Selector"), Some("app=np,tier=web"));
    assert!(info(&d, "Selector").mono);
    assert_eq!(row(&d, "IP families"), None);
    assert_eq!(row(&d, "Load balancer"), None);
    assert_eq!(row(&d, "Internal traffic"), None);
    assert_eq!(row(&d, "Not-ready pods"), None);
    let ports = section(&d, "ports");
    assert_eq!((ports.summary.as_str(), ports.empty.as_deref()), ("none", Some("No ports.")));

    // No type means ClusterIP; no clusterIPs falls back to clusterIP.
    let v = service_value("plain", json!({"clusterIP": "10.0.0.10", "sessionAffinity": "None"}));
    let d = describe_object("core/Service", &v, now());
    assert_eq!(row(&d, "Type"), Some("ClusterIP"));
    assert_eq!(row(&d, "Cluster IP"), Some("10.0.0.10"));
    assert_eq!(row(&d, "Session affinity"), None);
    assert_eq!(row(&d, "External traffic"), None);
}

#[tokio::test]
async fn external_name_service_has_no_ports_or_endpoints() {
    let v = service_value("mail", json!({"type": "ExternalName", "externalName": "mail.acme.io"}));
    let d = describe_object("core/Service", &v, now());
    assert_eq!(row(&d, "Points to"), Some("mail.acme.io"));
    assert!(info(&d, "Points to").mono);
    assert_eq!(row(&d, "Cluster IP"), None);
    assert_eq!(row(&d, "Selector"), None);
    assert_eq!(d.status.as_deref(), Some("External name"));
    assert!(d.sections.is_empty());

    // Nothing to look up for it.
    let api = MockApi::new();
    let d = related_details(&api.client(), "core/Service", &v).await.unwrap();
    assert!(d.sections.is_empty());
    assert!(api.calls().is_empty());
}

#[tokio::test]
async fn service_endpoints_and_the_ingresses_routing_to_it() {
    let api = MockApi::new();
    api.on("GET", SLICES, list("EndpointSliceList", vec![
        slice("web-v4", Some("web"), json!([endpoint("10.1.0.4", true, "web-1"), endpoint("10.1.0.5", false, "web-2")])),
        // The same pod listed again by a second slice.
        slice("web-v4b", Some("web"), json!([endpoint("10.1.0.4", true, "web-1")])),
    ]));
    let backend = |svc: &str| json!({"service": {"name": svc, "port": {"number": 80}}});
    api.on("GET", INGRESSES, list("IngressList", vec![
        object("networking.k8s.io/v1", "Ingress", Some("shop"), "front", json!({"spec": {
            "defaultBackend": backend("web"),
            "rules": [
                {"host": "shop.acme.io", "http": {"paths": [
                    {"path": "/", "pathType": "Prefix", "backend": backend("web")},
                    {"path": "/api", "pathType": "Prefix", "backend": backend("api")}]}},
                {"http": {"paths": [{"pathType": "Prefix", "backend": backend("web")}]}},
                {"host": "no-http.acme.io"}]}})),
        object("networking.k8s.io/v1", "Ingress", Some("shop"), "other", json!({"spec": {
            "defaultBackend": {"resource": {"apiGroup": "acme.io", "kind": "Bucket", "name": "assets"}}}})),
        object("networking.k8s.io/v1", "Ingress", Some("shop"), "bare", json!({})),
    ]));
    let v = service_value("web", json!({"clusterIP": "10.0.0.5", "selector": {"app": "web"}, "ports": [{"port": 80}]}));
    let d = related_details(&api.client(), "core/Service", &v).await.unwrap();

    let calls = api.calls_to("GET", SLICES);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].query.contains("service-name"), "{}", calls[0].query);
    assert!(calls[0].query.contains("web"), "{}", calls[0].query);

    assert_eq!(d.status.as_deref(), Some("Partly ready"));
    let eps = section(&d, "endpoints");
    assert_eq!(eps.summary, "1 of 2 ready");
    let t = eps.table.as_ref().unwrap();
    assert_eq!(cells(t), [vec!["10.1.0.4", "ready", "Pod web-1", "n1", ""], vec!["10.1.0.5", "not ready", "Pod web-2", "n1", ""]]);
    assert!(!t.rows[0].warn && t.rows[1].warn);
    assert_eq!(t.rows[1].link.as_ref().unwrap().name, "web-2");

    let routes = table(&d, "routed-from");
    assert_eq!(
        cells(routes),
        [vec!["front", "(default backend)", ""], vec!["front", "shop.acme.io", "/"], vec!["front", "*", "/"]]
    );
    for r in &routes.rows {
        let l = r.link.as_ref().unwrap();
        assert_eq!((l.kind.as_str(), l.namespace.as_deref(), l.name.as_str()), ("networking.k8s.io/Ingress", Some("shop"), "front"));
    }
}

#[tokio::test]
async fn service_status_follows_its_endpoints() {
    async fn status_for(spec: Value, endpoints: Option<Value>) -> ObjectDetails {
        let api = MockApi::new();
        let items = endpoints.map(|e| vec![slice("s", Some("web"), e)]).unwrap_or_default();
        api.on("GET", SLICES, list("EndpointSliceList", items));
        api.on("GET", INGRESSES, list("IngressList", vec![]));
        related_details(&api.client(), "core/Service", &service_value("web", spec)).await.unwrap()
    }

    // No selector and no endpoints.
    let d = status_for(json!({"clusterIP": "10.0.0.5"}), None).await;
    assert_eq!(d.status.as_deref(), Some("No endpoints"));
    let eps = section(&d, "endpoints");
    assert_eq!((eps.summary.as_str(), eps.empty.as_deref()), ("none", Some("No endpoints.")));
    assert!(d.sections.iter().all(|s| s.id != "routed-from"));

    // A selector matching nothing.
    let d = status_for(json!({"selector": {"app": "web"}}), None).await;
    assert_eq!(d.status.as_deref(), Some("No ready endpoints"));
    assert_eq!(section(&d, "endpoints").empty.as_deref(), Some("No pods match its selector."));

    // Only not-ready pods.
    let d = status_for(json!({"selector": {"app": "web"}}), Some(json!([endpoint("10.1.0.4", false, "web-1")]))).await;
    assert_eq!(d.status.as_deref(), Some("No ready endpoints"));
    assert_eq!(section(&d, "endpoints").summary, "0 of 1 ready");

    // All ready.
    let d = status_for(json!({"selector": {"app": "web"}}), Some(json!([endpoint("10.1.0.4", true, "web-1")]))).await;
    assert_eq!(d.status.as_deref(), Some("Ready"));
    assert_eq!(section(&d, "endpoints").summary, "1 of 1 ready");

    // A load balancer still waiting for an address keeps saying so.
    let d = status_for(json!({"type": "LoadBalancer", "selector": {"app": "web"}}), Some(json!([endpoint("10.1.0.4", true, "web-1")]))).await;
    assert_eq!(d.status.as_deref(), Some("Pending address"));
}

#[tokio::test]
async fn service_lookups_report_api_errors() {
    let v = service_value("web", json!({"selector": {"app": "web"}}));

    let api = MockApi::new();
    api.on_error("GET", SLICES, 403, "Forbidden", "endpointslices is forbidden");
    api.on("GET", INGRESSES, list("IngressList", vec![]));
    let err = related_details(&api.client(), "core/Service", &v).await.unwrap_err();
    assert_eq!(err.user_message(), "endpointslices is forbidden");

    let api = MockApi::new();
    api.on("GET", SLICES, list("EndpointSliceList", vec![]));
    api.on_error("GET", INGRESSES, 403, "Forbidden", "ingresses is forbidden");
    let err = related_details(&api.client(), "core/Service", &v).await.unwrap_err();
    assert_eq!(err.user_message(), "ingresses is forbidden");
}

// ---- EndpointSlice and Endpoints ----

#[test]
fn endpoint_slice_without_service_and_every_endpoint_state() {
    let v = json!({"kind": "EndpointSlice", "addressType": "IPv6",
        "metadata": {"name": "manual", "namespace": "shop", "labels": {"endpointslice.kubernetes.io/managed-by": "acme-controller"}},
        "ports": [{"port": 80}, {"name": "", "port": 81, "protocol": "UDP"}, {}],
        "endpoints": [
            {"addresses": ["fd00::1", "fd00::2"], "zone": "a"},
            {"addresses": ["fd00::3"], "conditions": {"ready": false, "serving": true}},
            {"addresses": ["fd00::4"], "conditions": {"ready": false}, "targetRef": {"kind": "Node", "name": "n1"}},
            {"addresses": ["fd00::5"], "conditions": {"ready": true}, "targetRef": {"kind": "Pod", "name": "web-5"}}]});
    let d = describe_object("discovery.k8s.io/EndpointSlice", &v, now());
    assert_eq!(row(&d, "Service"), None);
    assert_eq!(row(&d, "Address type"), Some("IPv6"));
    assert_eq!(row(&d, "Managed by"), Some("acme-controller"));
    assert_eq!(row(&d, "Ports"), Some("80/TCP, 81/UDP, 0/TCP"));
    assert_eq!(d.status.as_deref(), Some("Partly ready"));
    let s = section(&d, "slice-endpoints");
    assert_eq!(s.summary, "2 of 4 ready");
    let t = s.table.as_ref().unwrap();
    assert_eq!(
        cells(t),
        [
            vec!["fd00::1, fd00::2", "ready", "—", "—", "a"],
            vec!["fd00::3", "serving, not ready", "—", "—", ""],
            vec!["fd00::4", "not ready", "Node n1", "—", ""],
            vec!["fd00::5", "ready", "Pod web-5", "—", ""],
        ]
    );
    assert_eq!(t.rows.iter().map(|r| r.warn).collect::<Vec<_>>(), [false, true, true, false]);
    // Only pods link; a pod without a namespace is in the slice's.
    assert!(t.rows[2].link.is_none());
    let l = t.rows[3].link.as_ref().unwrap();
    assert_eq!((l.kind.as_str(), l.namespace.as_deref(), l.name.as_str()), ("core/Pod", Some("shop"), "web-5"));

    let empty = json!({"kind": "EndpointSlice", "metadata": {"name": "e", "namespace": "shop"}});
    let d = describe_object("discovery.k8s.io/EndpointSlice", &empty, now());
    assert_eq!((row(&d, "Ports"), row(&d, "Address type"), row(&d, "Managed by")), (Some("—"), Some(""), None));
    assert_eq!(d.status.as_deref(), Some("No endpoints"));
    assert_eq!(section(&d, "slice-endpoints").summary, "0 of 0 ready");

    let down = json!({"kind": "EndpointSlice", "metadata": {"name": "d", "namespace": "shop"},
        "endpoints": [{"addresses": ["10.0.0.1"], "conditions": {"ready": false}}]});
    assert_eq!(describe_object("discovery.k8s.io/EndpointSlice", &down, now()).status.as_deref(), Some("No ready endpoints"));
}

#[test]
fn legacy_endpoints() {
    let v = json!({"kind": "Endpoints", "metadata": {"name": "web", "namespace": "shop"},
        "subsets": [
            {"addresses": [{"ip": "10.1.0.4", "nodeName": "n1", "targetRef": {"kind": "Pod", "name": "web-1", "namespace": "shop"}}],
             "notReadyAddresses": [{"ip": "10.1.0.5"}],
             "ports": [{"port": 80}, {"port": 53, "protocol": "UDP"}]},
            {"addresses": [{"ip": "10.1.0.6"}], "ports": [{"port": 80, "protocol": "TCP"}]}]});
    let d = describe_object("core/Endpoints", &v, now());
    let svc = info(&d, "Service");
    assert_eq!(svc.value, "web");
    let l = svc.link.as_ref().unwrap();
    assert_eq!((l.kind.as_str(), l.namespace.as_deref()), ("core/Service", Some("shop")));
    assert!(row(&d, "Note").unwrap().contains("deprecated"));
    assert_eq!(row(&d, "Ports"), Some("53/UDP, 80/TCP"));
    assert_eq!(d.status.as_deref(), Some("Partly ready"));
    let t = table(&d, "addresses");
    assert_eq!(
        cells(t),
        [
            vec!["10.1.0.4", "ready", "Pod web-1", "n1", ""],
            vec!["10.1.0.5", "not ready", "—", "—", ""],
            vec!["10.1.0.6", "ready", "—", "—", ""],
        ]
    );
    assert_eq!(t.rows.iter().map(|r| r.warn).collect::<Vec<_>>(), [false, true, false]);
    assert_eq!(t.rows[0].link.as_ref().unwrap().name, "web-1");

    let empty = json!({"kind": "Endpoints", "metadata": {"name": "web", "namespace": "shop"}});
    let d = describe_object("core/Endpoints", &empty, now());
    assert_eq!((row(&d, "Ports"), d.status.as_deref()), (Some("—"), Some("No endpoints")));
    assert_eq!(section(&d, "addresses").empty.as_deref(), Some("No addresses."));

    let ready = json!({"kind": "Endpoints", "metadata": {"name": "web", "namespace": "shop"},
        "subsets": [{"addresses": [{"ip": "10.1.0.4"}]}]});
    assert_eq!(describe_object("core/Endpoints", &ready, now()).status.as_deref(), Some("Ready"));
}

// ---- Ingress ----

#[test]
fn ingress_backends_in_words() {
    assert_eq!(backend(&json!({"service": {"name": "web", "port": {"number": 8080}}})), ("web:8080".into(), Some("web".into())));
    assert_eq!(backend(&json!({"service": {"name": "web", "port": {"name": "http"}}})), ("web:http".into(), Some("web".into())));
    assert_eq!(backend(&json!({"service": {"name": "web"}})), ("web:".into(), Some("web".into())));
    assert_eq!(backend(&json!({"resource": {"kind": "Bucket", "name": "assets"}})), ("Bucket assets".into(), None));
    assert_eq!(backend(&json!({"resource": {"kind": "Bucket"}})), ("Bucket ".into(), None));
    assert_eq!(backend(&json!({})), ("—".into(), None));
}

#[test]
fn ingress_without_class_or_address() {
    let v = json!({"kind": "Ingress", "metadata": {"name": "web", "namespace": "shop"},
        "spec": {"defaultBackend": {"resource": {"kind": "Bucket", "name": "assets"}},
            "tls": [{"hosts": ["a.acme.io", "b.acme.io"]}],
            "rules": [
                {"http": {"paths": [{"backend": {"service": {"name": "web", "port": {"number": 80}}}}]}},
                {"host": "*.acme.io", "http": {"paths": [{"path": "/x", "pathType": "Prefix", "backend": {}}]}}]}});
    let d = describe_object("networking.k8s.io/Ingress", &v, now());
    let class = info(&d, "Class");
    assert_eq!((class.value.as_str(), class.link.is_none()), ("none: the cluster's default class, if any", true));
    let address = info(&d, "Address");
    assert_eq!((address.value.as_str(), address.mono), ("none yet", false));
    let default = info(&d, "Default backend");
    assert_eq!((default.value.as_str(), default.link.is_none(), default.mono), ("Bucket assets", true, true));
    assert_eq!(d.status.as_deref(), Some("No address"));

    let rules = section(&d, "rules");
    assert_eq!(rules.summary, "2 paths");
    let t = rules.table.as_ref().unwrap();
    assert_eq!(cells(t), [vec!["*", "/", "", "web:80", ""], vec!["*.acme.io", "/x", "Prefix", "—", ""]]);
    assert_eq!(t.rows[0].link.as_ref().unwrap().kind, "core/Service");
    assert!(t.rows[1].link.is_none());
    // No real host and no address: nothing a browser could open.
    assert!(d.sections.iter().all(|s| s.id != "urls"));

    let tls = table(&d, "tls");
    assert_eq!(cells(tls), [vec!["a.acme.io, b.acme.io", "(controller's default certificate)"]]);
    assert!(tls.rows[0].link.is_none());
}

#[test]
fn ingress_class_from_annotation_and_urls() {
    let v = json!({"kind": "Ingress",
        "metadata": {"name": "web", "namespace": "shop", "annotations": {"kubernetes.io/ingress.class": "nginx"}},
        "spec": {"defaultBackend": {"service": {"name": "web", "port": {"number": 80}}},
            "rules": [
                {"http": {"paths": [{"path": "/", "pathType": "Prefix", "backend": {"service": {"name": "web", "port": {"number": 80}}}},
                                    {"path": "/", "pathType": "Exact", "backend": {"service": {"name": "web", "port": {"number": 80}}}}]}}]},
        "status": {"loadBalancer": {"ingress": [{"ip": "198.51.100.7"}]}}});
    let d = describe_object("networking.k8s.io/Ingress", &v, now());
    let class = info(&d, "Class");
    assert_eq!(class.value, "nginx");
    let l = class.link.as_ref().unwrap();
    assert_eq!((l.kind.as_str(), l.namespace.as_deref()), ("networking.k8s.io/IngressClass", None));
    let default = info(&d, "Default backend");
    assert_eq!(default.value, "web:80");
    assert_eq!(default.link.as_ref().unwrap().namespace.as_deref(), Some("shop"));
    assert_eq!(info(&d, "Address").value, "198.51.100.7");
    assert!(info(&d, "Address").mono);
    assert_eq!(d.status.as_deref(), Some("Ready"));
    // A host-less rule falls back to the address; the same URL appears once.
    let urls = section(&d, "urls");
    assert_eq!(urls.rows.len(), 1);
    assert_eq!(urls.rows[0].value, "http://198.51.100.7/");
    assert_eq!(urls.rows[0].url.as_deref(), Some("http://198.51.100.7/"));
    assert!(d.sections.iter().all(|s| s.id != "tls"));

    // Nothing at all.
    let bare = json!({"kind": "Ingress", "metadata": {"name": "bare", "namespace": "shop"}});
    let d = describe_object("networking.k8s.io/Ingress", &bare, now());
    assert_eq!(row(&d, "Default backend"), None);
    let rules = section(&d, "rules");
    assert_eq!((rules.summary.as_str(), rules.empty.as_deref()), ("0 paths", Some("No rules: everything goes to the default backend.")));
}

fn rule_ingress(backends: &[&str], tls: Value) -> Value {
    let paths: Vec<Value> = backends
        .iter()
        .map(|b| json!({"path": format!("/{b}"), "pathType": "Prefix", "backend": {"service": {"name": b, "port": {"number": 80}}}}))
        .collect();
    let mut paths = paths;
    paths.push(json!({"path": "/files", "pathType": "Prefix", "backend": {"resource": {"kind": "Bucket", "name": "files"}}}));
    json!({"kind": "Ingress", "metadata": {"name": "front", "namespace": "shop"},
        "spec": {"tls": tls, "rules": [{"host": "shop.acme.io", "http": {"paths": paths}}]},
        "status": {"loadBalancer": {"ingress": [{"hostname": "lb.acme.io"}]}}})
}

fn ingress_api(services: &[&str], slices: Vec<Value>, secrets: Option<&[&str]>) -> MockApi {
    let api = MockApi::new();
    let svc = services.iter().map(|s| object("v1", "Service", Some("shop"), s, json!({}))).collect();
    api.on("GET", SERVICES, list("ServiceList", svc));
    api.on("GET", SLICES, list("EndpointSliceList", slices));
    match secrets {
        Some(names) => {
            let items = names.iter().map(|s| object("v1", "Secret", Some("shop"), s, json!({}))).collect();
            api.on("GET", SECRETS, list("SecretList", items));
        }
        None => {
            api.on_error("GET", SECRETS, 403, "Forbidden", "secrets is forbidden");
        }
    }
    api
}

#[tokio::test]
async fn ingress_backend_health_and_missing_objects() {
    let api = ingress_api(
        &["web", "api"],
        vec![
            slice("web-a", Some("web"), json!([endpoint("10.1.0.4", true, "web-1"), endpoint("10.1.0.5", false, "web-2")])),
            slice("web-b", Some("web"), json!([{"addresses": ["10.1.0.6"]}])),
            slice("api-a", Some("api"), json!([endpoint("10.1.0.7", false, "api-1")])),
            slice("loose", None, json!([endpoint("10.1.0.8", true, "x")])),
        ],
        Some(&["shop-tls"]),
    );
    let v = rule_ingress(
        &["web", "api", "gone"],
        json!([{"hosts": ["shop.acme.io"], "secretName": "shop-tls"}, {"hosts": ["old.acme.io"], "secretName": "old-tls"},
               {"hosts": ["x.acme.io"]}]),
    );
    let d = related_details(&api.client(), "networking.k8s.io/Ingress", &v).await.unwrap();

    // Secrets are read as metadata only.
    assert_eq!(api.calls_to("GET", SECRETS).len(), 1);
    let rules = table(&d, "rules");
    let ready: Vec<(&str, bool)> = rules.rows.iter().map(|r| (r.cells[4].as_str(), r.warn)).collect();
    assert_eq!(ready, [("2/3", false), ("0/1", true), ("no such service", true), ("", false)]);
    assert_eq!(d.status.as_deref(), Some("Backend missing"));

    let tls = table(&d, "tls");
    assert_eq!(cells(tls)[0], ["shop.acme.io", "shop-tls"]);
    assert!(!tls.rows[0].warn);
    assert_eq!((tls.rows[1].cells[1].as_str(), tls.rows[1].warn), ("old-tls (missing)", true));
    assert_eq!((tls.rows[2].cells[1].as_str(), tls.rows[2].warn), ("(controller's default certificate)", false));
}

#[tokio::test]
async fn ingress_status_when_backends_are_not_ready_or_fine() {
    // Services exist, one has no ready endpoints.
    let api = ingress_api(&["web", "api"], vec![slice("web-a", Some("web"), json!([endpoint("10.1.0.4", true, "web-1")]))], Some(&[]));
    let v = rule_ingress(&["web", "api"], json!([{"hosts": ["shop.acme.io"], "secretName": "shop-tls"}]));
    let d = related_details(&api.client(), "networking.k8s.io/Ingress", &v).await.unwrap();
    assert_eq!(d.status.as_deref(), Some("Backend not ready"));
    let rules = table(&d, "rules");
    assert_eq!((rules.rows[0].cells[4].as_str(), rules.rows[1].cells[4].as_str()), ("1/1", "0/0"));
    // The namespace has no Secrets at all: the certificate Secret is missing.
    let tls = table(&d, "tls");
    assert!(tls.rows[0].warn);
    assert_eq!(tls.rows[0].cells[1], "shop-tls (missing)");

    // Everything healthy; Secrets can't be listed, so certificates aren't checked.
    let api = ingress_api(&["web"], vec![slice("web-a", Some("web"), json!([endpoint("10.1.0.4", true, "web-1")]))], None);
    let v = rule_ingress(&["web"], json!([{"hosts": ["shop.acme.io"], "secretName": "elsewhere-tls"}]));
    let d = related_details(&api.client(), "networking.k8s.io/Ingress", &v).await.unwrap();
    assert_eq!(d.status.as_deref(), Some("Ready"));
    let tls = table(&d, "tls");
    assert_eq!((tls.rows[0].cells[1].as_str(), tls.rows[0].warn), ("elsewhere-tls", false));
}

#[tokio::test]
async fn ingress_lookups_report_api_errors() {
    let v = rule_ingress(&["web"], json!([]));
    let api = MockApi::new();
    api.on_error("GET", SERVICES, 403, "Forbidden", "services is forbidden");
    api.on("GET", SLICES, list("EndpointSliceList", vec![]));
    api.on("GET", SECRETS, list("SecretList", vec![]));
    let err = related_details(&api.client(), "networking.k8s.io/Ingress", &v).await.unwrap_err();
    assert_eq!(err.user_message(), "services is forbidden");

    let api = MockApi::new();
    api.on("GET", SERVICES, list("ServiceList", vec![]));
    api.on_error("GET", SLICES, 403, "Forbidden", "endpointslices is forbidden");
    api.on("GET", SECRETS, list("SecretList", vec![]));
    let err = related_details(&api.client(), "networking.k8s.io/Ingress", &v).await.unwrap_err();
    assert_eq!(err.user_message(), "endpointslices is forbidden");
}

// ---- IngressClass ----

fn ingress_class_value(default: bool, parameters: Value) -> Value {
    let annotations = if default { json!({"ingressclass.kubernetes.io/is-default-class": "true"}) } else { json!({}) };
    json!({"kind": "IngressClass", "metadata": {"name": "nginx", "annotations": annotations},
           "spec": {"controller": "k8s.io/ingress-nginx", "parameters": parameters}})
}

#[test]
fn ingress_class_rows() {
    let d = describe_object(
        "networking.k8s.io/IngressClass",
        &ingress_class_value(true, json!({"apiGroup": "acme.io", "kind": "Params", "name": "p", "namespace": "ops"})),
        now(),
    );
    assert_eq!(row(&d, "Controller"), Some("k8s.io/ingress-nginx"));
    assert!(info(&d, "Controller").mono);
    assert_eq!(row(&d, "Default class"), Some("yes: Ingresses without a class use it"));
    assert_eq!(row(&d, "Parameters"), Some("Params p in ops"));
    assert_eq!(d.status.as_deref(), Some("Default"));

    let d = describe_object("networking.k8s.io/IngressClass", &ingress_class_value(false, json!({"kind": "Params"})), now());
    assert_eq!(row(&d, "Default class"), Some("no"));
    assert_eq!(row(&d, "Parameters"), Some("Params "));
    assert_eq!(d.status, None);

    let d = describe_object("networking.k8s.io/IngressClass", &ingress_class_value(false, json!(null)), now());
    assert_eq!(row(&d, "Parameters"), None);
}

#[tokio::test]
async fn ingress_class_lists_the_ingresses_using_it() {
    const ALL: &str = "/apis/networking.k8s.io/v1/ingresses";
    let ing = |ns: &str, name: &str, extra: Value| object("networking.k8s.io/v1", "Ingress", Some(ns), name, extra);
    let api = MockApi::new();
    api.on("GET", ALL, list("IngressList", vec![
        ing("shop", "by-spec", json!({"spec": {"ingressClassName": "nginx",
            "rules": [{"host": "shop.acme.io"}, {"host": "www.acme.io"}, {"http": {"paths": []}}]}})),
        ing("ops", "by-annotation", json!({"metadata": {"annotations": {"kubernetes.io/ingress.class": "nginx"}}})),
        ing("ops", "classless", json!({"spec": {}})),
        ing("ops", "other", json!({"spec": {"ingressClassName": "alb"}})),
    ]));
    let client = api.client();

    let d = related_details(&client, "networking.k8s.io/IngressClass", &ingress_class_value(false, json!(null))).await.unwrap();
    let t = table(&d, "class-ingresses");
    assert_eq!(cells(t), [vec!["shop", "by-spec", "shop.acme.io, www.acme.io"], vec!["ops", "by-annotation", ""]]);
    let l = t.rows[1].link.as_ref().unwrap();
    assert_eq!((l.kind.as_str(), l.namespace.as_deref(), l.name.as_str()), ("networking.k8s.io/Ingress", Some("ops"), "by-annotation"));

    // The default class also takes Ingresses that name none.
    let d = related_details(&client, "networking.k8s.io/IngressClass", &ingress_class_value(true, json!(null))).await.unwrap();
    let names: Vec<&str> = table(&d, "class-ingresses").rows.iter().map(|r| r.cells[1].as_str()).collect();
    assert_eq!(names, ["by-spec", "by-annotation", "classless"]);

    let api = MockApi::new();
    api.on("GET", ALL, list("IngressList", vec![]));
    let d = related_details(&api.client(), "networking.k8s.io/IngressClass", &ingress_class_value(false, json!(null))).await.unwrap();
    let s = section(&d, "class-ingresses");
    assert_eq!((s.summary.as_str(), s.empty.as_deref()), ("none", Some("No Ingresses use it.")));

    let api = MockApi::new();
    api.on_error("GET", ALL, 403, "Forbidden", "ingresses is forbidden at the cluster scope");
    let err = related_details(&api.client(), "networking.k8s.io/IngressClass", &ingress_class_value(false, json!(null))).await.unwrap_err();
    assert_eq!(err.user_message(), "ingresses is forbidden at the cluster scope");
}

// ---- NetworkPolicy ----

#[test]
fn every_kind_of_peer_in_words() {
    let some = json!({"matchLabels": {"app": "web"}});
    let ns = json!({"matchLabels": {"team": "ops"}});
    let cases = [
        (json!({"podSelector": {}}), "all pods in this namespace"),
        (json!({"podSelector": some}), "pods app=web in this namespace"),
        (json!({"namespaceSelector": {}}), "all pods in all namespaces"),
        (json!({"namespaceSelector": ns}), "all pods in namespaces team=ops"),
        (json!({"podSelector": {}, "namespaceSelector": {}}), "all pods in all namespaces"),
        (json!({"podSelector": some, "namespaceSelector": {}}), "pods app=web in any namespace"),
        (json!({"podSelector": {}, "namespaceSelector": ns}), "all pods in namespaces team=ops"),
        (json!({"podSelector": some, "namespaceSelector": ns}), "pods app=web in namespaces team=ops"),
        (json!({}), "—"),
        (json!({"ipBlock": {"cidr": "10.0.0.0/8"}}), "addresses 10.0.0.0/8"),
        (json!({"ipBlock": {"cidr": "10.0.0.0/8", "except": ["10.1.0.0/16", "10.2.0.0/16"]}}), "addresses 10.0.0.0/8 except 10.1.0.0/16, 10.2.0.0/16"),
    ];
    for (p, want) in cases {
        assert_eq!(peer(&p), want, "{p}");
    }
}

#[test]
fn rule_ports_in_words() {
    assert_eq!(ports(&json!({})), "all ports");
    assert_eq!(ports(&json!({"ports": []})), "all ports");
    assert_eq!(ports(&json!({"ports": [{"protocol": "UDP"}, {"port": "http"}, {"port": 80, "endPort": 90}]})), "all UDP, TCP http, TCP 80–90");
}

#[test]
fn network_policy_types_and_statuses() {
    // Types unset, egress rules given: both directions; no ingress rules blocks incoming.
    let v = json!({"kind": "NetworkPolicy", "metadata": {"name": "p", "namespace": "shop"},
        "spec": {"podSelector": {}, "egress": [{"to": [], "ports": []}, {"to": [{"ipBlock": {"cidr": "0.0.0.0/0"}}], "ports": [{"port": 443}]}]}});
    let d = describe_object("networking.k8s.io/NetworkPolicy", &v, now());
    assert_eq!(row(&d, "Incoming"), Some("all blocked (unless another policy allows it)"));
    assert_eq!(row(&d, "Outgoing"), Some("only what the 2 rules below allow"));
    assert_eq!(d.status.as_deref(), Some("Denies incoming"));
    let egress = section(&d, "egress");
    assert_eq!(egress.title, "Outgoing");
    let t = egress.table.as_ref().unwrap();
    assert_eq!(t.columns, ["To", "Ports"]);
    assert_eq!(cells(t), [vec!["anywhere", "all ports"], vec!["addresses 0.0.0.0/0", "TCP 443"]]);
    assert!(d.sections.iter().all(|s| s.id != "ingress"));

    let both = json!({"kind": "NetworkPolicy", "metadata": {"name": "deny-all", "namespace": "shop"},
        "spec": {"podSelector": {}, "policyTypes": ["Ingress", "Egress"]}});
    let d = describe_object("networking.k8s.io/NetworkPolicy", &both, now());
    assert_eq!(d.status.as_deref(), Some("Denies all"));
    assert!(d.sections.is_empty());

    let allow = json!({"kind": "NetworkPolicy", "metadata": {"name": "allow", "namespace": "shop"},
        "spec": {"podSelector": {"matchLabels": {"app": "api"}}, "policyTypes": ["Ingress"], "ingress": [{}]}});
    let d = describe_object("networking.k8s.io/NetworkPolicy", &allow, now());
    assert_eq!(d.status.as_deref(), Some("Allows listed"));
    assert_eq!(row(&d, "Outgoing"), Some("not restricted by this policy"));
    let t = table(&d, "ingress");
    assert_eq!(t.columns, ["From", "Ports"]);
    assert_eq!(cells(t), [vec!["anywhere", "all ports"]]);

    let egress_only = json!({"kind": "NetworkPolicy", "metadata": {"name": "e", "namespace": "shop"},
        "spec": {"podSelector": {}, "policyTypes": ["Egress"]}});
    let d = describe_object("networking.k8s.io/NetworkPolicy", &egress_only, now());
    assert_eq!(d.status.as_deref(), Some("Denies outgoing"));
    assert_eq!(row(&d, "Incoming"), Some("not restricted by this policy"));
}

#[tokio::test]
async fn network_policy_lists_the_pods_it_selects() {
    let api = MockApi::new();
    let owned = pod("shop", "api-1", "n1", json!({"metadata": {"ownerReferences": [
        {"apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "api-7d9", "uid": "u1", "controller": true}]}}));
    let loose = pod("shop", "api-debug", "n1", json!({"metadata": {"ownerReferences": [
        {"apiVersion": "v1", "kind": "ConfigMap", "name": "x", "uid": "u2"}]}}));
    api.on("GET", PODS, list("PodList", vec![owned, loose]));
    let v = json!({"kind": "NetworkPolicy", "metadata": {"name": "api", "namespace": "shop"},
        "spec": {"podSelector": {"matchLabels": {"app": "api"}}, "policyTypes": ["Ingress"]}});
    let d = related_details(&api.client(), "networking.k8s.io/NetworkPolicy", &v).await.unwrap();

    let calls = api.calls_to("GET", PODS);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].query.contains("labelSelector=app%3Dapi"), "{}", calls[0].query);
    let s = section(&d, "policy-pods");
    assert_eq!(s.summary, "2 pods");
    let t = s.table.as_ref().unwrap();
    assert_eq!(cells(t), [vec!["api-1", "ReplicaSet/api-7d9"], vec!["api-debug", "—"]]);
    let l = t.rows[0].link.as_ref().unwrap();
    assert_eq!((l.kind.as_str(), l.namespace.as_deref(), l.name.as_str()), ("core/Pod", Some("shop"), "api-1"));

    // An empty selector lists every pod, unfiltered.
    let api = MockApi::new();
    api.on("GET", PODS, list("PodList", vec![]));
    let v = json!({"kind": "NetworkPolicy", "metadata": {"name": "all", "namespace": "shop"}, "spec": {"podSelector": {}}});
    let d = related_details(&api.client(), "networking.k8s.io/NetworkPolicy", &v).await.unwrap();
    assert!(!api.calls_to("GET", PODS)[0].query.contains("labelSelector"));
    let s = section(&d, "policy-pods");
    assert_eq!((s.summary.as_str(), s.empty.as_deref()), ("none", Some("No pods match right now.")));

    let api = MockApi::new();
    api.on_error("GET", PODS, 403, "Forbidden", "pods is forbidden");
    let err = related_details(&api.client(), "networking.k8s.io/NetworkPolicy", &v).await.unwrap_err();
    assert_eq!(err.user_message(), "pods is forbidden");
}

#[tokio::test]
async fn other_kinds_are_left_alone() {
    let api = MockApi::new();
    let v = json!({"kind": "ConfigMap", "metadata": {"name": "x", "namespace": "shop"}});
    let mut d = describe_object("core/Endpoints", &json!({"metadata": {"name": "x"}}), now());
    let before = (d.info.len(), d.sections.len());
    describe("core/ConfigMap", &mut d, &v);
    related(&api.client(), "discovery.k8s.io/EndpointSlice", "shop", "x", &v, &mut d).await.unwrap();
    related(&api.client(), "core/Endpoints", "shop", "x", &v, &mut d).await.unwrap();
    assert_eq!((d.info.len(), d.sections.len()), before);
    assert!(api.calls().is_empty());
}

#[tokio::test]
async fn service_details_through_object_details() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/services/web", service_value("web", json!({"clusterIP": "10.0.0.5", "selector": {"app": "web"}})));
    api.on("GET", SLICES, list("EndpointSliceList", vec![slice("web-a", Some("web"), json!([endpoint("10.1.0.4", true, "web-1")]))]));
    api.on("GET", INGRESSES, list("IngressList", vec![]));
    api.on("GET", "/api/v1/namespaces/shop/events", list("EventList", vec![]));
    let kind = crate::testing::kind("", "v1", "Service", "services", true);
    let d = crate::config::object_details(&api.client(), &kind, Some("shop"), "web").await.unwrap();
    assert_eq!(d.status.as_deref(), Some("Ready"));
    assert_eq!(section(&d, "endpoints").summary, "1 of 1 ready");
}

#[tokio::test]
async fn ingress_health_leaves_other_tables_alone() {
    let api = ingress_api(&[], vec![], Some(&[]));
    let v = rule_ingress(&[], json!([]));
    let mut d = describe_object("networking.k8s.io/Ingress", &v, now());
    let extra = InfoTable { columns: vec!["A".into()], rows: vec![InfoTableRow::new(vec!["x".into()])], ..Default::default() };
    d.sections.push(table_section("extra", "Extra", extra, ""));
    related(&api.client(), "networking.k8s.io/Ingress", "shop", "front", &v, &mut d).await.unwrap();
    let t = table(&d, "extra");
    assert_eq!((t.rows[0].cells.as_slice(), t.rows[0].warn), (["x".to_owned()].as_slice(), false));
    assert_eq!(d.status.as_deref(), Some("Ready"));
}
