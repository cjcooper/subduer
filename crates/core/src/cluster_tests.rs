use serde_json::{Value, json};

use super::*;
use crate::testing::{MockApi, kind, list, object};
use crate::types::Category;

fn resource(name: &str, kind: &str, namespaced: bool, verbs: &[&str]) -> Value {
    json!({"name": name, "singularName": "", "namespaced": namespaced, "kind": kind, "verbs": verbs})
}

fn resource_list(group_version: &str, resources: Vec<Value>) -> Value {
    json!({"kind": "APIResourceList", "apiVersion": "v1", "groupVersion": group_version, "resources": resources})
}

fn group(name: &str, versions: &[&str]) -> Value {
    let versions: Vec<Value> = versions.iter().map(|v| json!({"groupVersion": format!("{name}/{v}"), "version": v})).collect();
    json!({"name": name, "versions": versions.clone(), "preferredVersion": versions[0].clone()})
}

const LIST: &[&str] = &["get", "list", "watch"];

/// A small cluster: core v1, apps v1, events.k8s.io v1 and a custom group
/// with a kind only at v1alpha1.
fn fake_discovery(api: &MockApi) {
    api.on("GET", "/api", json!({"kind": "APIVersions", "versions": ["v1"], "serverAddressByClientCIDRs": []}));
    api.on("GET", "/api/v1", resource_list("v1", vec![
        resource("pods", "Pod", true, LIST),
        resource("pods/log", "Pod", true, &["get"]),
        resource("nodes", "Node", false, LIST),
        resource("events", "Event", true, LIST),
        resource("namespaces", "Namespace", false, LIST),
        // Not listable: left out.
        resource("bindings", "Binding", true, &["create"]),
    ]));
    api.on("GET", "/apis", json!({"kind": "APIGroupList", "apiVersion": "v1", "groups": [
        group("apps", &["v1"]),
        group("events.k8s.io", &["v1"]),
        group("acme.io", &["v1", "v1alpha1"]),
    ]}));
    api.on("GET", "/apis/apps/v1", resource_list("apps/v1", vec![
        resource("deployments", "Deployment", true, LIST),
        resource("deployments/status", "Deployment", true, &["get"]),
        resource("statefulsets", "StatefulSet", true, LIST),
    ]));
    api.on("GET", "/apis/events.k8s.io/v1", resource_list("events.k8s.io/v1", vec![resource("events", "Event", true, LIST)]));
    api.on("GET", "/apis/acme.io/v1", resource_list("acme.io/v1", vec![resource("widgets", "Widget", true, LIST)]));
    api.on("GET", "/apis/acme.io/v1alpha1", resource_list("acme.io/v1alpha1", vec![
        resource("widgets", "Widget", true, LIST),
        resource("gadgets", "Gadget", false, LIST),
    ]));
}

fn metadata_list(names: &[&str]) -> Value {
    let items = names.iter().map(|n| json!({"metadata": {"name": n}})).collect();
    let mut l = list("PartialObjectMetadataList", items);
    l["apiVersion"] = json!("meta.k8s.io/v1");
    l
}

#[tokio::test]
async fn discover_kinds_orders_filters_and_adds_helm() {
    let api = MockApi::new();
    fake_discovery(&api);
    api.on("GET", "/apis/apiextensions.k8s.io/v1/customresourcedefinitions", metadata_list(&["widgets.acme.io", "gadgets.acme.io"]));
    api.on("GET", "/api/v1/secrets", metadata_list(&["sh.helm.release.v1.shop.v1"]));
    let kinds = discover_kinds(&api.client()).await.unwrap();
    let keys: Vec<String> = kinds.iter().map(|k| format!("{}@{}", k.kind.key(), k.kind.version)).collect();
    assert_eq!(
        keys,
        [
            "core/Node@v1",
            "core/Event@v1",
            "core/Pod@v1",
            "apps/Deployment@v1",
            "apps/StatefulSet@v1",
            "core/Namespace@v1",
            "acme.io/Gadget@v1alpha1",
            "acme.io/Widget@v1",
            "helm.sh/Release@v3",
        ]
    );
    let pod = kinds.iter().find(|k| k.kind.kind == "Pod").unwrap();
    assert!(pod.kind.namespaced);
    assert_eq!(pod.kind.plural, "pods");
    assert_eq!(pod.verbs, ["get", "list", "watch"]);
    assert_eq!(pod.category, Category::Workloads);
    let gadget = kinds.iter().find(|k| k.kind.kind == "Gadget").unwrap();
    assert!(!gadget.kind.namespaced);
    assert_eq!(gadget.category, Category::CustomResources);
    assert_eq!(kinds.last().unwrap().category, Category::Helm);
}

#[tokio::test]
async fn discover_kinds_without_crds_or_helm() {
    // CRDs and Helm secrets can't be listed (404): still works, no Helm entry.
    let api = MockApi::new();
    fake_discovery(&api);
    let kinds = discover_kinds(&api.client()).await.unwrap();
    assert!(kinds.iter().all(|k| k.category != Category::Helm));
    // Non-built-in groups are custom even without the CRD list.
    assert_eq!(kinds.iter().find(|k| k.kind.kind == "Widget").unwrap().category, Category::CustomResources);
    assert_eq!(api.calls_to("GET", "/apis/apiextensions.k8s.io/v1/customresourcedefinitions").len(), 1);
}

#[tokio::test]
async fn discover_kinds_fails_when_discovery_fails() {
    let api = MockApi::new();
    api.on_error("GET", "/apis", 403, "Forbidden", "discovery is forbidden");
    let err = discover_kinds(&api.client()).await.unwrap_err();
    assert!(err.to_string().contains("forbidden"), "{err}");
}

#[tokio::test]
async fn namespaces_are_sorted() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces", metadata_list(&["shop", "acme-uat", "default"]));
    assert_eq!(list_namespaces(&api.client()).await.unwrap(), ["acme-uat", "default", "shop"]);
}

#[tokio::test]
async fn namespaces_error_is_returned() {
    let api = MockApi::new();
    api.on_error("GET", "/api/v1/namespaces", 403, "Forbidden", "namespaces is forbidden");
    let err = list_namespaces(&api.client()).await.unwrap_err();
    assert_eq!(err.user_message(), "namespaces is forbidden");
}

#[test]
fn api_resource_from_kind() {
    let ar = api_resource(&kind("apps", "v1", "Deployment", "deployments", true));
    assert_eq!((ar.group.as_str(), ar.version.as_str(), ar.api_version.as_str()), ("apps", "v1", "apps/v1"));
    assert_eq!((ar.kind.as_str(), ar.plural.as_str()), ("Deployment", "deployments"));
    let core = api_resource(&kind("", "v1", "Pod", "pods", true));
    assert_eq!(core.api_version, "v1");
}

#[tokio::test]
async fn dynamic_api_scopes() {
    let api = MockApi::new();
    let client = api.client();
    let cm = kind("", "v1", "ConfigMap", "configmaps", true);
    let node = kind("", "v1", "Node", "nodes", false);
    let _ = dynamic_api(&client, &cm, Some("shop")).get("app").await;
    let _ = dynamic_api(&client, &cm, None).get("app").await;
    // The namespace is ignored for cluster-scoped kinds.
    let _ = dynamic_api(&client, &node, Some("shop")).get("n1").await;
    let paths: Vec<String> = api.calls().into_iter().map(|c| c.path).collect();
    assert_eq!(paths, ["/api/v1/namespaces/shop/configmaps/app", "/api/v1/configmaps/app", "/api/v1/nodes/n1"]);
}

#[tokio::test]
async fn get_yaml_drops_managed_fields() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/configmaps/app", object("v1", "ConfigMap", Some("shop"), "app", json!({
        "metadata": {"managedFields": [{"manager": "kubectl", "operation": "Apply"}]},
        "data": {"greeting": "hello"}
    })));
    let yaml = get_yaml(&api.client(), &kind("", "v1", "ConfigMap", "configmaps", true), Some("shop"), "app").await.unwrap();
    assert!(yaml.contains("greeting: hello"), "{yaml}");
    assert!(yaml.contains("name: app"));
    assert!(!yaml.contains("managedFields"));
    assert!(!yaml.contains("kubectl"));
}

#[tokio::test]
async fn get_yaml_errors() {
    let api = MockApi::new();
    let client = api.client();
    let err = get_yaml(&client, &kind("", "v1", "ConfigMap", "configmaps", true), Some("shop"), "missing").await.unwrap_err();
    assert!(matches!(err, crate::Error::Kube(kube::Error::Api(ref s)) if s.code == 404), "{err}");
    // Helm releases are read from their Secrets, not a resource endpoint.
    assert!(get_yaml(&client, &crate::helm::release_kind(), Some("shop"), "web").await.is_err());
    assert!(api.calls_to("GET", "/api/v1/namespaces/shop/secrets").len() == 1);
}

#[tokio::test(start_paused = true)]
async fn connection_check_gives_up_on_a_silent_cluster() {
    // Like a connection a dropped VPN left open: the request is never answered.
    let service = tower::service_fn(|_req: http::Request<kube::client::Body>| {
        std::future::pending::<std::result::Result<http::Response<kube::client::Body>, std::convert::Infallible>>()
    });
    let client = Client::new(service, "default");
    let start = tokio::time::Instant::now();
    let err = check_connection(&client).await.unwrap_err();
    assert_eq!(start.elapsed(), CHECK_TIMEOUT);
    assert!(err.to_string().contains("no answer from the cluster"), "{err}");
}

#[tokio::test]
async fn connection_check_reads_the_version() {
    let api = MockApi::new();
    api.on("GET", "/version", json!({"major": "1", "minor": "36", "gitVersion": "v1.36.0"}));
    check_connection(&api.client()).await.unwrap();
    api.on_error("GET", "/version", 403, "Forbidden", "forbidden");
    check_connection(&api.client()).await.unwrap();
    api.on_error("GET", "/version", 401, "Unauthorized", "Unauthorized");
    let err = check_connection(&api.client()).await.unwrap_err();
    assert!(err.to_string().contains("Unauthorized"), "{err}");
}
