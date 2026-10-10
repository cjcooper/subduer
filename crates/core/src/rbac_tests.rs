//! Coverage tests for Access Control details and Check access.

use super::*;
use crate::testing::{MockApi, list, object, pod};
use crate::types::InfoSection;
use serde_json::json;

const RBAC: &str = "/apis/rbac.authorization.k8s.io/v1";

fn now() -> jiff::Timestamp {
    "2026-10-10T12:00:00Z".parse().unwrap()
}

fn row<'a>(d: &'a ObjectDetails, key: &str) -> Option<&'a str> {
    d.info.iter().find(|r| r.key == key).map(|r| r.value.as_str())
}

fn section<'a>(d: &'a ObjectDetails, id: &str) -> &'a InfoSection {
    d.sections.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no section {id}"))
}

fn cells(s: &InfoSection) -> Vec<Vec<String>> {
    s.table.as_ref().unwrap().rows.iter().map(|r| r.cells.clone()).collect()
}

fn describe(key: &str, v: &Value) -> ObjectDetails {
    crate::config::describe(key, v, now())
}

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn role_binding(ns: &str, name: &str, role_kind: &str, role: &str, subjects: Value) -> Value {
    object("rbac.authorization.k8s.io/v1", "RoleBinding", Some(ns), name, json!({
        "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": role_kind, "name": role},
        "subjects": subjects
    }))
}

fn cluster_role_binding(name: &str, role: &str, subjects: Value) -> Value {
    object("rbac.authorization.k8s.io/v1", "ClusterRoleBinding", None, name, json!({
        "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": role},
        "subjects": subjects
    }))
}

fn sa(name: &str, ns: Option<&str>) -> Value {
    let mut s = json!({"kind": "ServiceAccount", "name": name});
    if let Some(ns) = ns {
        s["namespace"] = json!(ns);
    }
    s
}

// ---- Rules ----

#[test]
fn risk_levels() {
    assert_eq!(risk(&strs(&["*"]), &strs(&[]), &strs(&["*"])), Some("full access"));
    assert_eq!(risk(&strs(&["*"]), &strs(&["*"]), &strs(&["*"])), Some("full access"));
    // Everything in one group isn't full access, but reaches nothing secret either.
    assert_eq!(risk(&strs(&["*"]), &strs(&["apps"]), &strs(&["*"])), Some("can exec into pods"));
    assert_eq!(risk(&strs(&["impersonate"]), &strs(&[""]), &strs(&["users"])), Some("can grant itself more"));
    assert_eq!(risk(&strs(&["escalate"]), &strs(&["rbac.authorization.k8s.io"]), &strs(&["roles"])), Some("can grant itself more"));
    assert_eq!(risk(&strs(&["list"]), &strs(&[]), &strs(&["secrets"])), Some("reads secrets"));
    assert_eq!(risk(&strs(&["watch"]), &strs(&["*"]), &strs(&["*"])), Some("reads secrets"));
    // Secrets in another group aren't core Secrets.
    assert_eq!(risk(&strs(&["get"]), &strs(&["acme.io"]), &strs(&["secrets"])), None);
    assert_eq!(risk(&strs(&["create"]), &strs(&[""]), &strs(&["pods/exec"])), Some("can exec into pods"));
    assert_eq!(risk(&strs(&["get"]), &strs(&[""]), &strs(&["pods/exec"])), None);
    assert_eq!(risk(&strs(&["get"]), &strs(&[""]), &strs(&["nodes/proxy"])), Some("reaches the kubelet"));
    assert_eq!(risk(&strs(&["get"]), &strs(&[""]), &strs(&["pods"])), None);
}

#[test]
fn full_access_outranks_earlier_flags() {
    let rules = [
        json!({"apiGroups": [""], "resources": ["secrets"], "verbs": ["get"]}),
        json!({"apiGroups": ["*"], "resources": ["*"], "verbs": ["*"]}),
        json!({"apiGroups": [""], "resources": ["pods/exec"], "verbs": ["create"]}),
    ];
    let (table, worst) = rules_table(&rules);
    assert_eq!(worst, Some("full access"));
    assert_eq!(table.rows.len(), 3);
    assert!(table.rows.iter().all(|r| r.warn));

    // The first flag sticks unless full access comes later.
    let rules = [
        json!({"apiGroups": [""], "resources": ["pods"], "verbs": ["get"]}),
        json!({"apiGroups": [""], "resources": ["secrets"], "verbs": ["get"]}),
        json!({"apiGroups": [""], "resources": ["pods/exec"], "verbs": ["create"]}),
    ];
    assert_eq!(rules_table(&rules).1, Some("reads secrets"));

    // A rule with nothing in it.
    let (table, worst) = rules_table(&[json!({})]);
    assert_eq!(worst, None);
    assert_eq!(table.rows[0].cells, ["—", "—", "—", "any", ""]);
}

#[test]
fn roles_without_rules_aggregation_and_built_in() {
    let v = json!({"kind": "ClusterRole", "metadata": {"name": "system:aggregate-to-view",
            "labels": {"rbac.authorization.k8s.io/aggregate-to-view": "true", "rbac.authorization.k8s.io/aggregate-to-edit": "false"}},
        "aggregationRule": {"clusterRoleSelectors": [{"matchLabels": {"acme.io/view": "true"}}, {"matchLabels": {"acme.io/read": "true"}}]}});
    let d = describe(CLUSTER_ROLE, &v);
    assert_eq!(row(&d, "Rules"), Some("0"));
    assert_eq!(row(&d, "Aggregates"), Some("rules of ClusterRoles with acme.io/view=true or acme.io/read=true"));
    assert_eq!(row(&d, "Adds its rules to"), Some("view"));
    assert_eq!(row(&d, "Built in"), Some("yes: managed by Kubernetes"));
    assert_eq!(d.status, None);
    let rules = section(&d, "rules");
    assert_eq!(rules.summary, "none");
    assert_eq!(rules.empty.as_deref(), Some("No rules: it grants nothing."));

    let plain = json!({"kind": "Role", "metadata": {"name": "reader", "namespace": "shop"},
        "rules": [{"apiGroups": ["apps"], "resources": ["deployments"], "verbs": ["get"]}]});
    let d = describe(ROLE, &plain);
    assert_eq!(row(&d, "Aggregates"), None);
    assert_eq!(row(&d, "Adds its rules to"), None);
    assert_eq!(row(&d, "Built in"), None);
    assert_eq!(cells(section(&d, "rules")), [strs(&["get", "apps", "deployments", "any", ""])]);
}

#[test]
fn unknown_kinds_are_left_alone() {
    let mut d = describe("core/Pod", &json!({"kind": "Pod", "metadata": {"name": "p"}}));
    let before = (d.info.len(), d.sections.len());
    super::describe("core/Pod", &mut d, &json!({"rules": [{"verbs": ["*"]}]}));
    assert_eq!((d.info.len(), d.sections.len()), before);
}

// ---- Bindings ----

#[test]
fn cluster_role_bindings_and_role_bindings_to_roles() {
    let crb = json!({"kind": "ClusterRoleBinding", "metadata": {"name": "ops"},
        "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": "admin"},
        "subjects": [sa("deployer", Some("ci")), {"kind": "User", "name": "jo@acme.io"}, {"kind": "Group", "name": "ops"},
                     {"kind": "Group", "name": "system:unauthenticated"}]});
    let d = describe(CLUSTER_ROLE_BINDING, &crb);
    assert_eq!(row(&d, "Grants"), Some("ClusterRole admin"));
    assert_eq!(d.info[0].link.as_ref().unwrap().namespace, None);
    assert_eq!(row(&d, "Where"), Some("the whole cluster"));
    let s = section(&d, "subjects");
    assert_eq!(s.summary, "4 subjects");
    let rows = &s.table.as_ref().unwrap().rows;
    assert_eq!(rows[0].cells, ["ServiceAccount", "deployer", "ci"]);
    assert_eq!(rows[0].link.as_ref().unwrap().namespace.as_deref(), Some("ci"));
    assert_eq!(rows[1].cells, ["User", "jo@acme.io", ""]);
    assert!(rows[1].link.is_none());
    assert!(!rows[2].warn);
    assert!(rows[3].warn);
    assert_eq!(d.status.as_deref(), Some("Everyone"));

    let rb = json!({"kind": "RoleBinding", "metadata": {"name": "read", "namespace": "shop"},
        "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "Role", "name": "reader"}});
    let d = describe(ROLE_BINDING, &rb);
    let link = d.info[0].link.as_ref().unwrap();
    assert_eq!((link.kind.as_str(), link.namespace.as_deref(), link.name.as_str()), (ROLE, Some("shop"), "reader"));
    assert_eq!(row(&d, "Where"), Some("only in namespace shop"));
    let s = section(&d, "subjects");
    assert_eq!(s.summary, "0 subjects");
    assert_eq!(s.empty.as_deref(), Some("No subjects: it grants nothing."));
    assert_eq!(d.status, None);

    // A cluster-scoped binding naming a ServiceAccount without a namespace.
    let r = subject_row(&sa("api", None), None);
    assert_eq!(r.cells, ["ServiceAccount", "api", ""]);
}

#[tokio::test]
async fn granted_rules_of_a_cluster_role() {
    let api = MockApi::new();
    api.on("GET", &format!("{RBAC}/clusterroles/edit"), object("rbac.authorization.k8s.io/v1", "ClusterRole", None, "edit", json!({
        "rules": [{"apiGroups": [""], "resources": ["secrets"], "verbs": ["get", "list"]},
                  {"apiGroups": ["apps"], "resources": ["deployments"], "verbs": ["update"]}]
    })));
    let v = role_binding("shop", "edit", "ClusterRole", "edit", json!([sa("api", None)]));
    let mut d = describe(ROLE_BINDING, &v);
    related(&api.client(), ROLE_BINDING, "shop", "edit", &v, &mut d).await.unwrap();
    let s = &d.sections[0];
    assert_eq!(s.id, "granted");
    assert_eq!(cells(s), [strs(&["get, list", "core", "secrets", "any", "reads secrets"]), strs(&["update", "apps", "deployments", "any", ""])]);
    assert_eq!(d.status.as_deref(), Some("Sensitive"));

    // Full access, and a binding to everyone keeps "Everyone".
    let api = MockApi::new();
    api.on("GET", &format!("{RBAC}/clusterroles/cluster-admin"), object("rbac.authorization.k8s.io/v1", "ClusterRole", None, "cluster-admin", json!({
        "rules": [{"apiGroups": ["*"], "resources": ["*"], "verbs": ["*"]}]
    })));
    let v = cluster_role_binding("root", "cluster-admin", json!([{"kind": "Group", "name": "ops"}]));
    let mut d = describe(CLUSTER_ROLE_BINDING, &v);
    related(&api.client(), CLUSTER_ROLE_BINDING, "", "root", &v, &mut d).await.unwrap();
    assert_eq!(d.status.as_deref(), Some("Full access"));

    let v = cluster_role_binding("open", "cluster-admin", json!([{"kind": "Group", "name": "system:authenticated"}]));
    let mut d = describe(CLUSTER_ROLE_BINDING, &v);
    related(&api.client(), CLUSTER_ROLE_BINDING, "", "open", &v, &mut d).await.unwrap();
    assert_eq!(d.status.as_deref(), Some("Everyone"));
}

#[tokio::test]
async fn granted_role_without_rules_or_missing() {
    let api = MockApi::new();
    api.on("GET", &format!("{RBAC}/namespaces/shop/roles/empty"), object("rbac.authorization.k8s.io/v1", "Role", Some("shop"), "empty", json!({})));
    let v = role_binding("shop", "b", "Role", "empty", json!([]));
    let mut d = describe(ROLE_BINDING, &v);
    related(&api.client(), ROLE_BINDING, "shop", "b", &v, &mut d).await.unwrap();
    let s = section(&d, "granted");
    assert_eq!(s.summary, "none");
    assert_eq!(s.empty.as_deref(), Some("The role has no rules."));
    assert_eq!(d.status, None);

    let v = role_binding("shop", "b", "Role", "gone", json!([]));
    let mut d = describe(ROLE_BINDING, &v);
    related(&api.client(), ROLE_BINDING, "shop", "b", &v, &mut d).await.unwrap();
    assert_eq!(d.status.as_deref(), Some("Role missing"));
    assert_eq!(row(&d, "Note"), Some("Role gone doesn't exist, so this grants nothing"));
    assert!(d.sections.iter().all(|s| s.id != "granted"));
}

#[tokio::test]
async fn granted_errors() {
    let api = MockApi::new();
    // A binding without a usable roleRef.
    let v = json!({"kind": "RoleBinding", "metadata": {"name": "b", "namespace": "shop"}});
    let mut d = describe(ROLE_BINDING, &v);
    let err = related(&api.client(), ROLE_BINDING, "shop", "b", &v, &mut d).await.unwrap_err();
    assert!(matches!(err, Error::Message(_)));

    api.on_error("GET", &format!("{RBAC}/namespaces/shop/roles/secret"), 403, "Forbidden", "roles is forbidden");
    let v = role_binding("shop", "b", "Role", "secret", json!([]));
    let mut d = describe(ROLE_BINDING, &v);
    let err = related(&api.client(), ROLE_BINDING, "shop", "b", &v, &mut d).await.unwrap_err();
    assert_eq!(err.user_message(), "roles is forbidden");
}

#[tokio::test]
async fn other_kinds_have_nothing_related() {
    let api = MockApi::new();
    let v = json!({"kind": "Pod", "metadata": {"name": "p"}});
    let mut d = describe("core/Pod", &v);
    related(&api.client(), "core/Pod", "shop", "p", &v, &mut d).await.unwrap();
    assert!(api.calls().is_empty());
}

// ---- Bound by ----

#[tokio::test]
async fn role_bound_by_bindings_in_its_namespace() {
    let api = MockApi::new();
    api.on("GET", &format!("{RBAC}/namespaces/shop/rolebindings"), list("RoleBindingList", vec![
        role_binding("shop", "read", "Role", "reader", json!([sa("api", None), {"kind": "User", "name": "jo"}])),
        role_binding("shop", "other", "Role", "writer", json!([sa("api", None)])),
        // Same name, but a ClusterRole.
        role_binding("shop", "view", "ClusterRole", "reader", json!([sa("api", None)])),
        role_binding("shop", "nobody", "Role", "reader", Value::Null),
    ]));
    let v = json!({"kind": "Role", "metadata": {"name": "reader", "namespace": "shop"}});
    let mut d = describe(ROLE, &v);
    related(&api.client(), ROLE, "shop", "reader", &v, &mut d).await.unwrap();
    let s = section(&d, "bound-by");
    assert_eq!(cells(s), [strs(&["read", "shop", "ServiceAccount api, User jo"]), strs(&["nobody", "shop", ""])]);
    assert_eq!(s.summary, "2");
    let link = s.table.as_ref().unwrap().rows[0].link.as_ref().unwrap();
    assert_eq!((link.kind.as_str(), link.namespace.as_deref()), (ROLE_BINDING, Some("shop")));
    assert!(api.calls_to("GET", &format!("{RBAC}/clusterrolebindings")).is_empty());
}

#[tokio::test]
async fn cluster_role_bound_everywhere_or_nowhere() {
    let api = MockApi::new();
    api.on("GET", &format!("{RBAC}/rolebindings"), list("RoleBindingList", vec![
        role_binding("shop", "view", "ClusterRole", "view", json!([{"kind": "Group", "name": "devs"}])),
        role_binding("shop", "local", "Role", "view", json!([])),
    ]));
    api.on("GET", &format!("{RBAC}/clusterrolebindings"), list("ClusterRoleBindingList", vec![
        cluster_role_binding("all-view", "view", json!([{"kind": "Group", "name": "system:authenticated"}])),
        cluster_role_binding("admins", "admin", json!([])),
    ]));
    let v = json!({"kind": "ClusterRole", "metadata": {"name": "view"}});
    let mut d = describe(CLUSTER_ROLE, &v);
    related(&api.client(), CLUSTER_ROLE, "", "view", &v, &mut d).await.unwrap();
    let s = section(&d, "bound-by");
    assert_eq!(cells(s), [
        strs(&["view", "shop", "Group devs"]),
        strs(&["all-view", "(cluster-wide)", "Group system:authenticated"]),
    ]);
    let link = s.table.as_ref().unwrap().rows[1].link.as_ref().unwrap();
    assert_eq!((link.kind.as_str(), link.namespace.as_deref()), (CLUSTER_ROLE_BINDING, None));

    let v = json!({"kind": "ClusterRole", "metadata": {"name": "unused"}});
    let mut d = describe(CLUSTER_ROLE, &v);
    related(&api.client(), CLUSTER_ROLE, "", "unused", &v, &mut d).await.unwrap();
    let s = section(&d, "bound-by");
    assert_eq!(s.summary, "none");
    assert_eq!(s.empty.as_deref(), Some("Not bound: it grants nothing to anyone."));
}

#[tokio::test]
async fn bound_by_list_errors_propagate() {
    let api = MockApi::new();
    api.on_error("GET", &format!("{RBAC}/namespaces/shop/rolebindings"), 403, "Forbidden", "rolebindings is forbidden");
    let v = json!({"kind": "Role", "metadata": {"name": "reader", "namespace": "shop"}});
    let mut d = describe(ROLE, &v);
    let err = related(&api.client(), ROLE, "shop", "reader", &v, &mut d).await.unwrap_err();
    assert_eq!(err.user_message(), "rolebindings is forbidden");

    api.on("GET", &format!("{RBAC}/rolebindings"), list("RoleBindingList", vec![]));
    let v = json!({"kind": "ClusterRole", "metadata": {"name": "view"}});
    let mut d = describe(CLUSTER_ROLE, &v);
    let err = related(&api.client(), CLUSTER_ROLE, "", "view", &v, &mut d).await.unwrap_err();
    assert!(err.user_message().contains("isn't faked"), "{err}");
}

// ---- ServiceAccount ----

#[test]
fn service_account_identities_and_secrets() {
    let v = json!({"kind": "ServiceAccount", "metadata": {"name": "api", "namespace": "shop",
        "annotations": {"iam.gke.io/gcp-service-account": "api@acme.iam.gserviceaccount.com",
                        "azure.workload.identity/client-id": "0000-1111"}},
        "secrets": [{"name": "api-token"}, {"noname": true}]});
    let d = describe("core/ServiceAccount", &v);
    assert_eq!(row(&d, "Google service account"), Some("api@acme.iam.gserviceaccount.com"));
    assert_eq!(row(&d, "Azure identity"), Some("0000-1111"));
    assert_eq!(row(&d, "AWS IAM role"), None);
    assert!(row(&d, "Token in pods").unwrap().starts_with("mounted automatically"));
    let token = d.info.iter().find(|r| r.key == "Token secret").unwrap();
    assert_eq!(token.value, "api-token");
    let link = token.link.as_ref().unwrap();
    assert_eq!((link.kind.as_str(), link.namespace.as_deref()), ("core/Secret", Some("shop")));
    assert_eq!(d.info.iter().filter(|r| r.key == "Token secret").count(), 1);
}

#[test]
fn which_subjects_mean_an_account() {
    let s = |kind: &str, name: &str, ns: Option<&str>| Subject {
        kind: kind.into(),
        name: name.into(),
        namespace: ns.map(str::to_owned),
        api_group: None,
    };
    // No namespace on the subject: the binding's own.
    assert_eq!(matches_account(&s("ServiceAccount", "api", None), "shop", "api"), Some("directly"));
    assert_eq!(matches_account(&s("ServiceAccount", "web", Some("shop")), "shop", "api"), None);
    assert_eq!(matches_account(&s("Group", "system:serviceaccounts", None), "shop", "api"), Some("all service accounts"));
    assert_eq!(matches_account(&s("Group", "system:serviceaccounts:acme", None), "shop", "api"), None);
    assert_eq!(matches_account(&s("User", "system:serviceaccount:shop:web", None), "shop", "api"), None);
    assert_eq!(matches_account(&s("User", "jo", None), "shop", "api"), None);
}

#[tokio::test]
async fn service_account_bindings_permissions_and_pods() {
    let api = MockApi::new();
    api.on("GET", &format!("{RBAC}/rolebindings"), list("RoleBindingList", vec![
        role_binding("shop", "api-read", "Role", "reader", json!([sa("api", None)])),
        // Same role through a second binding: the rules are read once.
        role_binding("shop", "api-read-2", "Role", "reader", json!([{"kind": "Group", "name": "system:serviceaccounts:shop"}])),
        role_binding("shop", "web", "Role", "reader", json!([sa("web", None)])),
        role_binding("shop", "gone", "Role", "missing", json!([sa("api", Some("shop"))])),
    ]));
    api.on("GET", &format!("{RBAC}/clusterrolebindings"), list("ClusterRoleBindingList", vec![
        cluster_role_binding("all-sa", "secrets", json!([{"kind": "Group", "name": "system:serviceaccounts"}])),
        cluster_role_binding("root", "cluster-admin", json!([{"kind": "User", "name": "system:serviceaccount:shop:api"}])),
        cluster_role_binding("others", "cluster-admin", json!([{"kind": "User", "name": "jo"}])),
    ]));
    api.on("GET", &format!("{RBAC}/namespaces/shop/roles/reader"), object("rbac.authorization.k8s.io/v1", "Role", Some("shop"), "reader", json!({
        "rules": [{"apiGroups": [""], "resources": ["pods"], "verbs": ["get", "list"]}]
    })));
    api.on("GET", &format!("{RBAC}/clusterroles/secrets"), object("rbac.authorization.k8s.io/v1", "ClusterRole", None, "secrets", json!({
        "rules": [{"apiGroups": [""], "resources": ["secrets"], "verbs": ["get"]},
                  {"nonResourceURLs": ["/metrics", "/healthz"], "verbs": ["get"]}]
    })));
    api.on("GET", &format!("{RBAC}/clusterroles/cluster-admin"), object("rbac.authorization.k8s.io/v1", "ClusterRole", None, "cluster-admin", json!({
        "rules": [{"apiGroups": ["*"], "resources": ["*"], "verbs": ["*"]}]
    })));
    let owned = json!({"metadata": {"ownerReferences": [
        {"apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "api-7d9f", "uid": "u1", "controller": true}]},
        "spec": {"serviceAccountName": "api"}});
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![
        pod("shop", "api-7d9f-x", "node-1", owned),
        pod("shop", "api-bare", "node-1", json!({"spec": {"serviceAccountName": "api"}})),
        pod("shop", "web-1", "node-1", json!({"spec": {"serviceAccountName": "web"}})),
        pod("shop", "plain", "node-1", json!({})),
    ]));

    let v = json!({"kind": "ServiceAccount", "metadata": {"name": "api", "namespace": "shop"}});
    let mut d = describe("core/ServiceAccount", &v);
    related(&api.client(), "core/ServiceAccount", "shop", "api", &v, &mut d).await.unwrap();

    let b = section(&d, "sa-bindings");
    assert_eq!(cells(b), [
        strs(&["api-read", "Role reader", "in shop", "directly"]),
        strs(&["api-read-2", "Role reader", "in shop", "service accounts in its namespace"]),
        strs(&["gone", "Role missing", "in shop", "directly"]),
        strs(&["all-sa", "ClusterRole secrets", "cluster-wide", "all service accounts"]),
        strs(&["root", "ClusterRole cluster-admin", "cluster-wide", "directly"]),
    ]);
    assert_eq!(b.summary, "5 bindings");
    let links: Vec<(String, Option<String>)> = b.table.as_ref().unwrap().rows.iter()
        .map(|r| (r.link.as_ref().unwrap().kind.clone(), r.link.as_ref().unwrap().namespace.clone()))
        .collect();
    assert_eq!(links[0], (ROLE_BINDING.to_owned(), Some("shop".to_owned())));
    assert_eq!(links[3], (CLUSTER_ROLE_BINDING.to_owned(), None));
    assert_eq!(api.calls_to("GET", &format!("{RBAC}/namespaces/shop/roles/reader")).len(), 1);

    let p = section(&d, "sa-permissions");
    assert_eq!(cells(p), [
        strs(&["get, list", "pods", "in shop", "Role reader", ""]),
        strs(&["get, list", "pods", "in shop", "Role reader", ""]),
        strs(&["get", "secrets", "cluster-wide", "ClusterRole secrets", "reads secrets"]),
        strs(&["get", "URLs /metrics, /healthz", "cluster-wide", "ClusterRole secrets", ""]),
        strs(&["*", "*", "cluster-wide", "ClusterRole cluster-admin", "full access"]),
    ]);
    assert_eq!(p.summary, "5 rules");
    assert!(p.table.as_ref().unwrap().rows[2].warn);
    assert_eq!(d.status.as_deref(), Some("Full access"));

    // "plain" has no service account set: it runs as "default", not "api".
    let pods = section(&d, "sa-pods");
    assert_eq!(cells(pods), [strs(&["api-7d9f-x", "ReplicaSet/api-7d9f"]), strs(&["api-bare", "—"])]);
    assert_eq!(pods.summary, "2 pods");
    assert_eq!(pods.table.as_ref().unwrap().rows[0].link.as_ref().unwrap().kind, "core/Pod");
}

#[tokio::test]
async fn service_account_with_nothing_bound() {
    let api = MockApi::new();
    api.on("GET", &format!("{RBAC}/rolebindings"), list("RoleBindingList", vec![]));
    api.on("GET", &format!("{RBAC}/clusterrolebindings"), list("ClusterRoleBindingList", vec![]));
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![pod("shop", "plain", "node-1", json!({}))]));

    let v = json!({"kind": "ServiceAccount", "metadata": {"name": "default", "namespace": "shop"}});
    let mut d = describe("core/ServiceAccount", &v);
    related(&api.client(), "core/ServiceAccount", "shop", "default", &v, &mut d).await.unwrap();
    assert_eq!(section(&d, "sa-bindings").summary, "none");
    assert_eq!(section(&d, "sa-permissions").summary, "defaults only");
    assert_eq!(d.status, None);
    let pods = section(&d, "sa-pods");
    assert_eq!(cells(pods), [strs(&["plain", "—"])]);
    assert_eq!(pods.summary, "1 pod");

    let v = json!({"kind": "ServiceAccount", "metadata": {"name": "idle", "namespace": "shop"}});
    let mut d = describe("core/ServiceAccount", &v);
    related(&api.client(), "core/ServiceAccount", "shop", "idle", &v, &mut d).await.unwrap();
    let pods = section(&d, "sa-pods");
    assert_eq!(pods.summary, "none");
    assert_eq!(pods.empty.as_deref(), Some("No pods run as it right now."));
}

#[tokio::test]
async fn service_account_list_errors_propagate() {
    let api = MockApi::new();
    let v = json!({"kind": "ServiceAccount", "metadata": {"name": "api", "namespace": "shop"}});
    // Role bindings can't be listed.
    let mut d = describe("core/ServiceAccount", &v);
    assert!(related(&api.client(), "core/ServiceAccount", "shop", "api", &v, &mut d).await.is_err());

    // Cluster role bindings can't be listed.
    api.on("GET", &format!("{RBAC}/rolebindings"), list("RoleBindingList", vec![]));
    let mut d = describe("core/ServiceAccount", &v);
    assert!(related(&api.client(), "core/ServiceAccount", "shop", "api", &v, &mut d).await.is_err());

    // Pods can't be listed.
    api.on("GET", &format!("{RBAC}/clusterrolebindings"), list("ClusterRoleBindingList", vec![]));
    api.on_error("GET", "/api/v1/namespaces/shop/pods", 403, "Forbidden", "pods is forbidden");
    let mut d = describe("core/ServiceAccount", &v);
    let err = related(&api.client(), "core/ServiceAccount", "shop", "api", &v, &mut d).await.unwrap_err();
    assert_eq!(err.user_message(), "pods is forbidden");
}

// ---- Check access ----

const SAR: &str = "/apis/authorization.k8s.io/v1/subjectaccessreviews";

fn review(status: Value) -> Value {
    json!({"apiVersion": "authorization.k8s.io/v1", "kind": "SubjectAccessReview", "metadata": {},
           "spec": {}, "status": status})
}

#[tokio::test]
async fn check_access_sends_a_review() {
    let api = MockApi::new();
    api.on("POST", SAR, review(json!({"allowed": true, "reason": "RBAC: allowed by RoleBinding \"edit/shop\""})));
    let r = check_access(&api.client(), "shop", "api", " create ", " pods/exec ", " shop ", " api-1 ").await.unwrap();
    assert!(r.allowed);
    assert_eq!(r.reason, "RBAC: allowed by RoleBinding \"edit/shop\"");
    let body = api.calls_to("POST", SAR)[0].json();
    assert_eq!(body["spec"]["user"], "system:serviceaccount:shop:api");
    assert_eq!(body["spec"]["groups"], json!(["system:serviceaccounts", "system:serviceaccounts:shop", "system:authenticated"]));
    assert_eq!(body["spec"]["resourceAttributes"], json!({
        "verb": "create", "group": "", "resource": "pods", "subresource": "exec", "namespace": "shop", "name": "api-1"
    }));
}

#[tokio::test]
async fn check_access_everywhere_and_denials() {
    let api = MockApi::new();
    api.on("POST", SAR, review(json!({"allowed": false, "evaluationError": "no RBAC policy matched"})));
    let r = check_access(&api.client(), "shop", "api", "list", "deployments.apps", "", "").await.unwrap();
    assert!(!r.allowed);
    assert_eq!(r.reason, "no RBAC policy matched");
    let attrs = &api.calls_to("POST", SAR)[0].json()["spec"]["resourceAttributes"];
    assert_eq!(attrs, &json!({"verb": "list", "group": "apps", "resource": "deployments"}));

    // No status at all.
    let api = MockApi::new();
    api.on("POST", SAR, json!({"apiVersion": "authorization.k8s.io/v1", "kind": "SubjectAccessReview", "metadata": {}, "spec": {}}));
    let r = check_access(&api.client(), "shop", "api", "get", "pods", "shop", "").await.unwrap();
    assert!(!r.allowed);
    assert_eq!(r.reason, "");
}

#[tokio::test]
async fn check_access_needs_a_verb_and_resource() {
    let api = MockApi::new();
    for (verb, resource) in [(" ", "pods"), ("get", ""), ("", "")] {
        let err = check_access(&api.client(), "shop", "api", verb, resource, "", "").await.unwrap_err();
        assert!(matches!(err, Error::Message(ref m) if m == "give a verb and a resource"), "{err}");
    }
    assert!(api.calls().is_empty());

    api.on_error("POST", SAR, 403, "Forbidden", "subjectaccessreviews is forbidden");
    let err = check_access(&api.client(), "shop", "api", "get", "pods", "", "").await.unwrap_err();
    assert_eq!(err.user_message(), "subjectaccessreviews is forbidden");
}
