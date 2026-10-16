//! Coverage tests for External Secrets Details and Force sync: the describe
//! functions, the related lookups and the annotation patch, against the fake
//! API server.

use super::*;
use crate::config::{describe as describe_object, object_details};
use crate::testing::{MockApi, kind, list, object};
use crate::types::InfoTable;
use serde_json::json;

const ALL_ES: &str = "/apis/external-secrets.io/v1/externalsecrets";
const SHOP_ES: &str = "/apis/external-secrets.io/v1/namespaces/shop/externalsecrets";
const DEPLOYMENTS: &str = "/apis/apps/v1/deployments";
const ALL_EVENTS: &str = "/api/v1/events";

fn now() -> jiff::Timestamp {
    "2026-10-16T12:00:00Z".parse().unwrap()
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

fn cells(t: &InfoTable) -> Vec<Vec<&str>> {
    t.rows.iter().map(|r| r.cells.iter().map(String::as_str).collect()).collect()
}

fn ready(status: &str, reason: &str) -> Value {
    json!({"conditions": [{"type": "Ready", "status": status, "reason": reason, "message": "m"}]})
}

fn es(ns: &str, name: &str, spec: Value, status: Value) -> Value {
    object("external-secrets.io/v1", "ExternalSecret", Some(ns), name, json!({"spec": spec, "status": status}))
}

fn es_kind_ref() -> KindRef {
    kind("external-secrets.io", "v1", "ExternalSecret", "externalsecrets", true)
}

fn ces_kind_ref() -> KindRef {
    kind("external-secrets.io", "v1", "ClusterExternalSecret", "clusterexternalsecrets", false)
}

fn controller_deployment() -> Value {
    json!({"apiVersion": "meta.k8s.io/v1", "kind": "PartialObjectMetadataList", "metadata": {},
           "items": [{"apiVersion": "meta.k8s.io/v1", "kind": "PartialObjectMetadata",
                      "metadata": {"name": "external-secrets", "namespace": "external-secrets"}}]})
}

fn toast() -> Value {
    let mut v = es(
        "shop",
        "toast",
        json!({
            "secretStoreRef": {"kind": "SecretStore", "name": "default-store"},
            "target": {"creationPolicy": "Owner", "deletionPolicy": "Retain"},
            "refreshInterval": "1h",
            "data": [{"secretKey": "DB_USER", "remoteRef": {"key": "eg/db", "property": "user"}}],
            "dataFrom": [
                {"extract": {"key": "eg/api-key"}},
                {"find": {"path": "eg/", "name": {"regexp": "^db"}, "tags": {"env": "uat"}}, "rewrite": [{"regexp": {}}]},
                {"sourceRef": {"generatorRef": {"apiVersion": "generators.external-secrets.io/v1alpha1", "kind": "Password", "name": "pw"}}}
            ]
        }),
        json!({"refreshTime": "2026-10-16T11:30:00Z", "conditions": [{"type": "Ready", "status": "True", "reason": "SecretSynced"}]}),
    );
    v["metadata"]["annotations"] = json!({"force-sync": "1792152000"});
    v
}

// ---- small helpers ----

#[test]
fn go_durations_in_seconds() {
    assert_eq!(go_duration("1h"), Some(3600));
    assert_eq!(go_duration("1h30m"), Some(5400));
    assert_eq!(go_duration("15m0s"), Some(900));
    assert_eq!(go_duration("1.5h"), Some(5400));
    assert_eq!(go_duration("500ms"), Some(0));
    assert_eq!(go_duration("0"), Some(0));
    assert_eq!(go_duration("0s"), Some(0));
    assert_eq!(go_duration(""), None);
    assert_eq!(go_duration("1d"), None);
    assert_eq!(go_duration("h"), None);
}

#[test]
fn sync_status_from_the_ready_condition() {
    let with = |status: Value| es("shop", "x", json!({}), status);
    assert_eq!(sync_status(&with(ready("True", "SecretSynced"))), "Synced");
    assert_eq!(sync_status(&with(ready("False", "SecretSyncedError"))), "Sync failed");
    assert_eq!(sync_status(&with(ready("False", "SecretDeleted"))), "Deleted");
    assert_eq!(sync_status(&with(ready("False", "SecretMissing"))), "Secret missing");
    assert_eq!(sync_status(&with(ready("False", "SecretOwnedByOther"))), "Owned by other");
    assert_eq!(sync_status(&with(ready("Unknown", ""))), "Pending");
    assert_eq!(sync_status(&with(ready("False", "Other"))), "Not ready");
    assert_eq!(sync_status(&with(json!({}))), "Pending");
}

// ---- ExternalSecret ----

#[test]
fn external_secret_says_what_it_syncs() {
    let d = describe_object("external-secrets.io/ExternalSecret", &toast(), now());
    assert_eq!(d.status.as_deref(), Some("Synced"));
    let store = info(&d, "Store");
    assert_eq!(store.value, "SecretStore/default-store");
    assert_eq!(store.link.as_ref().unwrap().kind, "external-secrets.io/SecretStore");
    assert_eq!(store.link.as_ref().unwrap().namespace.as_deref(), Some("shop"));
    let target = info(&d, "Target secret");
    assert_eq!(target.value, "toast");
    assert_eq!(target.link.as_ref().unwrap().kind, "core/Secret");
    assert!(row(&d, "Creation").unwrap().starts_with("Owner: creates the Secret"));
    assert!(row(&d, "Deletion").unwrap().starts_with("Retain:"));
    assert_eq!(row(&d, "Refresh interval"), Some("1h"));
    assert!(info(&d, "Last refresh").time);
    assert_eq!(row(&d, "Next refresh"), Some("2026-10-16T12:30:00Z"));
    let forced = info(&d, "Forced sync");
    assert!(forced.time);
    assert_eq!(forced.value, "2026-10-16T12:00:00Z");
    let data = section(&d, "es-data").table.as_ref().unwrap();
    assert_eq!(data.columns, ["Secret key", "Remote key", "Property"], "unused Version and From are left out");
    assert_eq!(cells(data), vec![vec!["DB_USER", "eg/db", "user"]]);
    let from = section(&d, "es-data-from").table.as_ref().unwrap();
    assert_eq!(
        cells(from),
        vec![
            vec!["Extract", "eg/api-key", "", ""],
            vec!["Find", "path eg/, name ~ ^db, tag env=uat", "1 rewrite", ""],
            vec!["Generate", "", "", "Password/pw"],
        ]
    );
    let generator = from.rows[2].link.as_ref().unwrap();
    assert_eq!((generator.kind.as_str(), generator.namespace.as_deref()), ("generators.external-secrets.io/Password", Some("shop")));
}

#[test]
fn external_secret_refresh_policies_and_defaults() {
    let created_once = es("shop", "a", json!({"refreshPolicy": "CreatedOnce", "secretStoreRef": {"kind": "ClusterSecretStore", "name": "aws"},
        "target": {"name": "app", "creationPolicy": "Merge", "immutable": true,
                   "template": {"type": "kubernetes.io/tls", "data": {"tls.crt": "{{ .crt }}"}, "templateFrom": [{}]}}}), json!({}));
    let d = describe_object("external-secrets.io/ExternalSecret", &created_once, now());
    assert_eq!(d.status.as_deref(), Some("Pending"));
    assert!(row(&d, "Refresh policy").unwrap().starts_with("CreatedOnce"));
    assert_eq!(row(&d, "Refresh interval"), None);
    assert_eq!(info(&d, "Store").link.as_ref().unwrap().namespace, None);
    assert_eq!(row(&d, "Target secret"), Some("app"));
    assert!(row(&d, "Creation").unwrap().starts_with("Merge"));
    assert!(row(&d, "Immutable").is_some());
    assert_eq!(row(&d, "Template"), Some("engine v2 · type kubernetes.io/tls · 1 key · 1 from ConfigMaps/Secrets"));
    assert_eq!(section(&d, "es-data").empty.as_deref(), Some("No single keys: everything comes from Data from."));
    assert!(d.sections.iter().all(|s| s.id != "es-data-from"));

    let never = es("shop", "b", json!({"refreshInterval": "0", "refreshPolicy": "Periodic"}), json!({"refreshTime": "2026-10-16T11:30:00Z"}));
    let d = describe_object("external-secrets.io/ExternalSecret", &never, now());
    assert!(row(&d, "Refresh interval").unwrap().contains("fetched once, then never refreshed"));
    assert_eq!(row(&d, "Next refresh"), None);
    assert_eq!(row(&d, "Refresh policy"), Some("Periodic"));

    let on_change = es("shop", "c", json!({"refreshPolicy": "OnChange"}), json!({}));
    let d = describe_object("external-secrets.io/ExternalSecret", &on_change, now());
    assert!(row(&d, "Refresh policy").unwrap().starts_with("OnChange"));
    assert_eq!(row(&d, "Creation").map(|c| c.starts_with("Owner")), Some(true));
}

#[tokio::test]
async fn external_secret_details_with_events_controller_and_missing_secret() {
    let api = MockApi::new();
    api.on("GET", "/apis/external-secrets.io/v1/namespaces/shop/externalsecrets/toast", toast());
    api.on_error("GET", "/api/v1/namespaces/shop/secrets/toast", 404, "NotFound", "secrets \"toast\" not found");
    api.on("GET", DEPLOYMENTS, controller_deployment());
    api.on("GET", "/api/v1/namespaces/shop/events", list("EventList", vec![json!({
        "metadata": {"name": "e1", "namespace": "shop", "creationTimestamp": "2026-10-16T11:00:00Z"},
        "involvedObject": {"kind": "ExternalSecret", "name": "toast", "namespace": "shop"},
        "type": "Normal", "reason": "Updated", "message": "secret updated", "count": 1,
        "lastTimestamp": "2026-10-16T11:00:00Z"
    })]));
    let d = object_details(&api.client(), &es_kind_ref(), Some("shop"), "toast").await.unwrap();
    let target = info(&d, "Target secret");
    assert_eq!(target.value, "toast (missing)");
    assert!(target.link.is_none());
    let controller = info(&d, "Controller");
    assert_eq!(controller.value, "external-secrets/external-secrets");
    assert_eq!(controller.link.as_ref().unwrap().kind, "apps/Deployment");
    let logs = d.logs.as_ref().unwrap();
    assert_eq!(logs.namespace, "external-secrets");
    assert_eq!(logs.filter.as_deref(), Some("toast"));
    assert!(matches!(&logs.source, LogSource::Workload { kind, name } if kind.kind == "Deployment" && name == "external-secrets"));
    assert_eq!(d.events.len(), 1);
    let deploys = api.calls_to("GET", DEPLOYMENTS);
    assert!(deploys[0].query.contains("labelSelector=app.kubernetes.io%2Fname%3Dexternal-secrets"), "{}", deploys[0].query);
}

#[tokio::test]
async fn an_existing_target_and_no_controller() {
    let api = MockApi::new();
    let v = toast();
    api.on("GET", "/api/v1/namespaces/shop/secrets/toast", object("v1", "Secret", Some("shop"), "toast", json!({})));
    api.on_error("GET", DEPLOYMENTS, 403, "Forbidden", "deployments is forbidden");
    let mut d = describe_object("external-secrets.io/ExternalSecret", &v, now());
    related(&api.client(), &es_kind_ref(), "shop", "toast", &v, &mut d).await.unwrap();
    assert_eq!(row(&d, "Target secret"), Some("toast"));
    assert_eq!(row(&d, "Controller"), None);
    assert!(d.logs.is_none());

    // No Secret is written, so none is looked for.
    let none = es("shop", "n", json!({"target": {"creationPolicy": "None"}}), json!({}));
    let mut d = describe_object("external-secrets.io/ExternalSecret", &none, now());
    related(&api.client(), &es_kind_ref(), "shop", "n", &none, &mut d).await.unwrap();
    assert!(api.calls_to("GET", "/api/v1/namespaces/shop/secrets/n").is_empty());
}

// ---- ClusterExternalSecret ----

fn ces(status: Value) -> Value {
    object("external-secrets.io/v1", "ClusterExternalSecret", None, "shared", json!({
        "spec": {
            "externalSecretName": "shared-es",
            "namespaceSelectors": [{"matchLabels": {"team": "shop"}}],
            "namespaces": ["extra"],
            "refreshTime": "1m",
            "externalSecretSpec": {"secretStoreRef": {"kind": "ClusterSecretStore", "name": "aws"}, "refreshInterval": "30m",
                                   "target": {"name": "shared"}, "dataFrom": [{"extract": {"key": "eg/shared"}}]}
        },
        "status": status
    }))
}

#[test]
fn cluster_external_secret_rows_and_status() {
    let good = ces(json!({"provisionedNamespaces": ["shop"], "conditions": [{"type": "Ready", "status": "True"}]}));
    let d = describe_object("external-secrets.io/ClusterExternalSecret", &good, now());
    assert_eq!(d.status.as_deref(), Some("Ready"));
    assert_eq!(row(&d, "ExternalSecret name"), Some("shared-es"));
    assert_eq!(row(&d, "Namespaces"), Some("extra; labelled team=shop"));
    assert_eq!(row(&d, "Store"), Some("ClusterSecretStore/aws"));
    assert_eq!(row(&d, "Target secret"), Some("shared"));
    assert_eq!(row(&d, "Refresh interval"), Some("30m (each ExternalSecret)"));
    assert_eq!(row(&d, "Namespaces checked"), Some("every 1m"));
    assert!(d.sections.iter().any(|s| s.id == "es-data-from"));

    let partly = ces(json!({"provisionedNamespaces": ["shop"], "failedNamespaces": [{"namespace": "x", "reason": "denied"}]}));
    assert_eq!(describe_object("external-secrets.io/ClusterExternalSecret", &partly, now()).status.as_deref(), Some("Partly failed"));
    let failed = ces(json!({"failedNamespaces": [{"namespace": "x", "reason": "denied"}]}));
    assert_eq!(describe_object("external-secrets.io/ClusterExternalSecret", &failed, now()).status.as_deref(), Some("Failed"));
    let not_ready = ces(json!({"conditions": [{"type": "Ready", "status": "False"}]}));
    assert_eq!(describe_object("external-secrets.io/ClusterExternalSecret", &not_ready, now()).status.as_deref(), Some("Not ready"));
    assert_eq!(describe_object("external-secrets.io/ClusterExternalSecret", &ces(json!({})), now()).status.as_deref(), Some("Pending"));

    let mut everywhere = ces(json!({}));
    everywhere["spec"]["namespaceSelectors"] = json!([]);
    everywhere["spec"]["namespaces"] = json!([]);
    everywhere["spec"]["namespaceSelector"] = json!({});
    assert_eq!(row(&describe_object("external-secrets.io/ClusterExternalSecret", &everywhere, now()), "Namespaces"), Some("every namespace"));
    everywhere["spec"]["namespaceSelector"] = Value::Null;
    assert_eq!(row(&describe_object("external-secrets.io/ClusterExternalSecret", &everywhere, now()), "Namespaces"), Some("none selected"));
}

#[tokio::test]
async fn cluster_external_secret_lists_its_external_secrets_and_cluster_events() {
    let api = MockApi::new();
    let v = ces(json!({"provisionedNamespaces": ["shop", "web"], "failedNamespaces": [{"namespace": "x", "reason": "denied"}]}));
    let owned = |ns: &str, status: Value| {
        let mut o = es(ns, "shared-es", json!({}), status);
        o["metadata"]["ownerReferences"] = json!([{"kind": "ClusterExternalSecret", "name": "shared", "apiVersion": "external-secrets.io/v1", "uid": "u"}]);
        o
    };
    api.on("GET", ALL_ES, list("ExternalSecretList", vec![
        owned("web", ready("False", "SecretSyncedError")),
        owned("shop", json!({"refreshTime": "2026-10-16T11:00:00Z", "conditions": [{"type": "Ready", "status": "True"}]})),
        es("shop", "unrelated", json!({}), json!({})),
    ]));
    api.on("GET", ALL_EVENTS, list("EventList", vec![]));
    let mut d = describe_object("external-secrets.io/ClusterExternalSecret", &v, now());
    related(&api.client(), &ces_kind_ref(), "", "shared", &v, &mut d).await.unwrap();
    let s = section(&d, "externalsecret-children");
    assert_eq!(s.title, "ExternalSecrets");
    let t = s.table.as_ref().unwrap();
    assert_eq!(
        cells(t),
        vec![
            vec!["shop", "shared-es", "Synced", "2026-10-16T11:00:00Z"],
            vec!["web", "shared-es", "Sync failed", ""],
            vec!["x", "", "Failed: denied", ""],
        ]
    );
    assert_eq!(t.rows.iter().map(|r| r.warn).collect::<Vec<_>>(), [false, true, true]);
    assert_eq!(t.rows[0].link.as_ref().unwrap().namespace.as_deref(), Some("shop"));
    let events = api.calls_to("GET", ALL_EVENTS);
    assert!(events[0].query.contains("involvedObject.kind%3DClusterExternalSecret"), "{}", events[0].query);
}

#[tokio::test]
async fn lookups_fall_back_to_v1beta1_and_note_failures() {
    let api = MockApi::new();
    api.on("GET", "/apis/external-secrets.io/v1beta1/externalsecrets", list("ExternalSecretList", vec![]));
    assert!(external_secrets(&api.client(), None).await.unwrap().is_empty());
    assert_eq!(api.calls_to("GET", ALL_ES).len(), 1);

    let api = MockApi::new();
    api.on_error("GET", SHOP_ES, 403, "Forbidden", "externalsecrets is forbidden");
    assert!(external_secrets(&api.client(), Some("shop")).await.is_err());
    let v = store_value("SecretStore", Some("shop"));
    let mut d = describe_object("external-secrets.io/SecretStore", &v, now());
    let store_kind = kind("external-secrets.io", "v1", "SecretStore", "secretstores", true);
    related(&api.client(), &store_kind, "shop", "default-store", &v, &mut d).await.unwrap();
    assert!(row(&d, "Note").unwrap().starts_with("couldn't list ExternalSecrets"));
    assert_eq!(section(&d, "store-users").empty.as_deref(), Some("No ExternalSecrets use it."));

    // Neither version served.
    let api = MockApi::new();
    assert!(external_secrets(&api.client(), None).await.is_err());
}

// ---- Stores ----

fn store_value(kind_name: &str, ns: Option<&str>) -> Value {
    object("external-secrets.io/v1", kind_name, ns, "default-store", json!({
        "spec": {
            "provider": {"aws": {"service": "SecretsManager", "region": "us-east-1",
                                 "auth": {"jwt": {"serviceAccountRef": {"name": "store-sa"}},
                                          "secretRef": {"accessKeyIDSecretRef": {"name": "aws-creds", "key": "id"}}}}},
            "controller": "dev",
            "refreshInterval": 60,
            "retrySettings": {"maxRetries": 3, "retryInterval": "10s"},
            "conditions": [{"namespaces": ["shop"], "namespaceRegexes": ["^team-"], "namespaceSelector": {"matchLabels": {"es": "on"}}}]
        },
        "status": {"capabilities": "ReadWrite", "conditions": [{"type": "Ready", "status": "True", "reason": "Valid"}]}
    }))
}

#[test]
fn stores_describe_their_provider_without_secrets() {
    let d = describe_object("external-secrets.io/SecretStore", &store_value("SecretStore", Some("shop")), now());
    assert_eq!(d.status.as_deref(), Some("Valid"));
    assert_eq!(row(&d, "Provider"), Some("AWS Secrets Manager (us-east-1)"));
    assert!(row(&d, "Capabilities").unwrap().starts_with("ReadWrite"));
    assert_eq!(row(&d, "Controller class"), Some("dev"));
    assert_eq!(row(&d, "Checked every"), Some("60s"));
    assert_eq!(row(&d, "Retries"), Some("3, every 10s"));
    assert_eq!(row(&d, "Usable from"), None);
    let s = section(&d, "store-provider");
    assert_eq!(s.title, "Provider: aws");
    let sa = s.rows.iter().find(|r| r.key == "auth.jwt.serviceAccountRef.name").unwrap();
    assert_eq!(sa.link.as_ref().unwrap().kind, "core/ServiceAccount");
    let secret = s.rows.iter().find(|r| r.key == "auth.secretRef.accessKeyIDSecretRef.name").unwrap();
    assert_eq!(secret.link.as_ref().unwrap().kind, "core/Secret");
    assert!(s.rows.iter().any(|r| r.key == "region" && r.value == "us-east-1"));

    let cluster = describe_object("external-secrets.io/ClusterSecretStore", &store_value("ClusterSecretStore", None), now());
    assert_eq!(row(&cluster, "Usable from"), Some("shop; matching ^team-; labelled es=on"));
    // No namespace to link a cluster store's references to.
    let s = section(&cluster, "store-provider");
    assert!(s.rows.iter().all(|r| r.link.is_none()));

    let mut bad = store_value("SecretStore", Some("shop"));
    bad["status"] = json!({"capabilities": "ReadOnly", "conditions": [{"type": "Ready", "status": "False", "reason": "InvalidProviderConfig"}]});
    bad["spec"]["provider"] = json!({"vault": {"server": "https://vault", "namespaces": ["a", "b"], "list": [{"x": 1}]}});
    let d = describe_object("external-secrets.io/SecretStore", &bad, now());
    assert_eq!(d.status.as_deref(), Some("Invalid"));
    assert_eq!(row(&d, "Provider"), Some("HashiCorp Vault"));
    assert!(row(&d, "Capabilities").unwrap().starts_with("ReadOnly"));
    let s = section(&d, "store-provider");
    assert!(s.rows.iter().any(|r| r.key == "namespaces" && r.value == "a, b"));
    assert!(s.rows.iter().any(|r| r.key == "list[0].x" && r.value == "1"));

    for (p, text) in [("gcpsm", "Google Secret Manager"), ("azurekv", "Azure Key Vault"), ("other", "other"), ("", "—")] {
        assert_eq!(provider_name(p, &Value::Null), text);
    }
    assert_eq!(provider_name("aws", &json!({"service": "ParameterStore"})), "AWS Parameter Store");
    let unknown = object("external-secrets.io/v1", "SecretStore", Some("shop"), "s", json!({"spec": {}}));
    assert_eq!(describe_object("external-secrets.io/SecretStore", &unknown, now()).status.as_deref(), Some("Unknown"));
}

#[tokio::test]
async fn stores_list_the_external_secrets_using_them() {
    let api = MockApi::new();
    api.on("GET", SHOP_ES, list("ExternalSecretList", vec![
        es("shop", "direct", json!({"secretStoreRef": {"name": "default-store"}}), ready("True", "SecretSynced")),
        es("shop", "per-key", json!({"secretStoreRef": {"name": "other"},
            "data": [{"secretKey": "k", "remoteRef": {"key": "r"}, "sourceRef": {"storeRef": {"name": "default-store", "kind": "SecretStore"}}}]}), json!({})),
        es("shop", "cluster-one", json!({"secretStoreRef": {"name": "default-store", "kind": "ClusterSecretStore"}}), json!({})),
    ]));
    let v = store_value("SecretStore", Some("shop"));
    let mut d = describe_object("external-secrets.io/SecretStore", &v, now());
    let store_kind = kind("external-secrets.io", "v1", "SecretStore", "secretstores", true);
    related(&api.client(), &store_kind, "shop", "default-store", &v, &mut d).await.unwrap();
    let t = section(&d, "store-users").table.as_ref().unwrap();
    assert_eq!(cells(t), vec![vec!["shop", "direct", "Synced", ""], vec!["shop", "per-key", "Pending", ""]]);
    assert!(t.rows[1].warn);
    // Namespaced: no events from other namespaces are read here.
    assert!(api.calls_to("GET", ALL_EVENTS).is_empty());
}

// ---- PushSecret and ClusterPushSecret ----

fn push_spec() -> Value {
    json!({
        "secretStoreRefs": [{"name": "default-store"}, {"kind": "ClusterSecretStore", "labelSelector": {"matchLabels": {"push": "yes"}}}],
        "selector": {"secret": {"name": "local"}},
        "updatePolicy": "IfNotExists",
        "deletionPolicy": "Delete",
        "data": [{"match": {"secretKey": "token", "remoteRef": {"remoteKey": "eg/token", "property": "value"}}}, {"match": {"remoteRef": {"remoteKey": "eg/all"}}}]
    })
}

#[test]
fn push_secrets_say_what_they_push_and_where() {
    let v = object("external-secrets.io/v1alpha1", "PushSecret", Some("shop"), "push", json!({
        "spec": push_spec(),
        "status": {"refreshTime": "2026-10-16T11:00:00Z", "syncedPushSecrets": {"default-store": {"eg/token": {}}},
                   "conditions": [{"type": "Ready", "status": "True"}]}
    }));
    let d = describe_object("external-secrets.io/PushSecret", &v, now());
    assert_eq!(d.status.as_deref(), Some("Synced"));
    assert_eq!(info(&d, "Source secret").link.as_ref().unwrap().kind, "core/Secret");
    assert_eq!(row(&d, "Store"), Some("SecretStore/default-store"));
    assert_eq!(row(&d, "Stores"), Some("ClusterSecretStore labelled push=yes"));
    assert!(row(&d, "Update").unwrap().starts_with("IfNotExists"));
    assert!(row(&d, "Deletion").unwrap().starts_with("Delete"));
    assert_eq!(row(&d, "Refresh interval"), Some("1h"));
    assert!(info(&d, "Last refresh").time);
    assert_eq!(cells(section(&d, "push-data").table.as_ref().unwrap()), vec![vec!["token", "eg/token", "value"], vec!["(whole secret)", "eg/all", ""]]);
    assert_eq!(cells(section(&d, "push-synced").table.as_ref().unwrap()), vec![vec!["default-store", "eg/token"]]);

    let mut spec = push_spec();
    spec["selector"] = json!({"secret": {"selector": {"matchLabels": {"a": "b"}}}});
    spec["updatePolicy"] = Value::Null;
    spec["deletionPolicy"] = Value::Null;
    let failed = object("external-secrets.io/v1alpha1", "PushSecret", Some("shop"), "p2", json!({"spec": spec, "status": ready("False", "Errored")}));
    let d = describe_object("external-secrets.io/PushSecret", &failed, now());
    assert_eq!(d.status.as_deref(), Some("Sync failed"));
    assert_eq!(row(&d, "Source secrets"), Some("labelled a=b"));
    assert!(row(&d, "Update").unwrap().starts_with("Replace"));
    assert!(row(&d, "Deletion").unwrap().starts_with("None"));

    let mut spec = push_spec();
    spec["selector"] = json!({"generatorRef": {"kind": "Password", "name": "pw"}});
    let generated = object("external-secrets.io/v1alpha1", "PushSecret", Some("shop"), "p3", json!({"spec": spec}));
    let d = describe_object("external-secrets.io/PushSecret", &generated, now());
    assert_eq!(d.status.as_deref(), Some("Pending"));
    assert_eq!(row(&d, "Source"), Some("Password/pw"));
}

#[tokio::test]
async fn cluster_push_secret_lists_its_push_secrets() {
    let api = MockApi::new();
    let v = object("external-secrets.io/v1alpha1", "ClusterPushSecret", None, "cps", json!({
        "spec": {"pushSecretName": "pushed", "namespaceSelectors": [{"matchLabels": {"a": "b"}}], "refreshTime": "5m", "pushSecretSpec": push_spec()},
        "status": {"provisionedNamespaces": ["shop"], "conditions": [{"type": "Ready", "status": "True"}]}
    }));
    let mut child = object("external-secrets.io/v1alpha1", "PushSecret", Some("shop"), "pushed", json!({"status": ready("True", "Synced")}));
    child["metadata"]["ownerReferences"] = json!([{"kind": "ClusterPushSecret", "name": "cps"}]);
    api.on("GET", "/apis/external-secrets.io/v1alpha1/pushsecrets", list("PushSecretList", vec![child]));
    let mut d = describe_object("external-secrets.io/ClusterPushSecret", &v, now());
    assert_eq!(d.status.as_deref(), Some("Ready"));
    assert_eq!(row(&d, "PushSecret name"), Some("pushed"));
    assert_eq!(row(&d, "Source secret"), Some("local"));
    assert!(info(&d, "Source secret").link.is_none());
    assert_eq!(row(&d, "Namespaces checked"), Some("every 5m"));
    let k = kind("external-secrets.io", "v1alpha1", "ClusterPushSecret", "clusterpushsecrets", false);
    related(&api.client(), &k, "", "cps", &v, &mut d).await.unwrap();
    let t = section(&d, "pushsecret-children").table.as_ref().unwrap();
    assert_eq!(cells(t), vec![vec!["shop", "pushed", "Synced", ""]]);
}

// ---- Generators ----

#[test]
fn generators_show_settings_but_not_literal_values() {
    let pw = object("generators.external-secrets.io/v1alpha1", "Password", Some("shop"), "pw", json!({
        "spec": {"length": 32, "symbols": 4, "allowRepeat": true, "symbolCharacters": "-_"}
    }));
    let d = describe_object("generators.external-secrets.io/Password", &pw, now());
    assert_eq!(row(&d, "length"), Some("32"));
    assert_eq!(row(&d, "allowRepeat"), Some("true"));

    let fake = object("generators.external-secrets.io/v1alpha1", "Fake", Some("shop"), "f", json!({
        "spec": {"controller": "dev", "data": {"password": "hunter2", "user": "admin"}}
    }));
    let d = describe_object("generators.external-secrets.io/Fake", &fake, now());
    assert_eq!(row(&d, "data"), Some("password, user (values not shown)"));
    assert!(d.info.iter().all(|r| !r.value.contains("hunter2")));

    let cluster = object("generators.external-secrets.io/v1alpha1", "ClusterGenerator", None, "cg", json!({
        "spec": {"kind": "Fake", "generator": {"fakeSpec": {"data": {"k": "secret-value"}}}}
    }));
    let d = describe_object("generators.external-secrets.io/ClusterGenerator", &cluster, now());
    assert_eq!(row(&d, "Generates"), Some("Fake"));
    assert!(d.info.iter().all(|r| !r.value.contains("secret-value")));

    let ecr = object("generators.external-secrets.io/v1alpha1", "ECRAuthorizationToken", Some("shop"), "ecr", json!({
        "spec": {"region": "us-east-1", "auth": {"jwt": {"serviceAccountRef": {"name": "puller", "namespace": "infra"}}}}
    }));
    let d = describe_object("generators.external-secrets.io/ECRAuthorizationToken", &ecr, now());
    let sa = info(&d, "auth.jwt.serviceAccountRef.name");
    assert_eq!(sa.link.as_ref().unwrap().namespace.as_deref(), Some("infra"));
}

#[test]
fn generator_state_shows_key_names_only() {
    let v = object("generators.external-secrets.io/v1alpha1", "GeneratorState", Some("shop"), "gs", json!({
        "spec": {"resource": {"kind": "Password", "metadata": {"name": "pw"}}, "garbageCollectionDeadline": "2026-10-17T00:00:00Z",
                 "state": {"password": "s3cret", "issued": "x"}}
    }));
    let d = describe_object("generators.external-secrets.io/GeneratorState", &v, now());
    assert_eq!(row(&d, "Generator"), Some("Password/pw"));
    assert!(info(&d, "Removed after").time);
    assert_eq!(row(&d, "State keys"), Some("issued, password"));
    assert!(d.info.iter().all(|r| !r.value.contains("s3cret")));
}

#[tokio::test]
async fn generators_list_their_users() {
    let api = MockApi::new();
    api.on("GET", SHOP_ES, list("ExternalSecretList", vec![
        es("shop", "uses", json!({"dataFrom": [{"sourceRef": {"generatorRef": {"kind": "Password", "name": "pw"}}}]}), ready("True", "SecretSynced")),
        es("shop", "other", json!({"dataFrom": [{"sourceRef": {"generatorRef": {"kind": "Password", "name": "else"}}}]}), json!({})),
    ]));
    let v = object("generators.external-secrets.io/v1alpha1", "Password", Some("shop"), "pw", json!({"spec": {"length": 8}}));
    let mut d = describe_object("generators.external-secrets.io/Password", &v, now());
    let k = kind("generators.external-secrets.io", "v1alpha1", "Password", "passwords", true);
    related(&api.client(), &k, "shop", "pw", &v, &mut d).await.unwrap();
    assert_eq!(cells(section(&d, "generator-users").table.as_ref().unwrap()), vec![vec!["shop", "uses", "Synced", ""]]);

    // A GeneratorState isn't used by anything.
    let gs = object("generators.external-secrets.io/v1alpha1", "GeneratorState", Some("shop"), "gs", json!({"spec": {}}));
    let mut d = describe_object("generators.external-secrets.io/GeneratorState", &gs, now());
    let k = kind("generators.external-secrets.io", "v1alpha1", "GeneratorState", "generatorstates", true);
    related(&api.client(), &k, "shop", "gs", &gs, &mut d).await.unwrap();
    assert!(d.sections.is_empty());
}

#[test]
fn other_external_secrets_kinds_get_a_ready_badge() {
    let v = object("external-secrets.io/v1alpha1", "SomethingNew", Some("shop"), "x", json!({"status": ready("True", "")}));
    assert_eq!(describe_object("external-secrets.io/SomethingNew", &v, now()).status.as_deref(), Some("Ready"));
    let v = object("external-secrets.io/v1alpha1", "SomethingNew", Some("shop"), "x", json!({"status": ready("False", "")}));
    assert_eq!(describe_object("external-secrets.io/SomethingNew", &v, now()).status.as_deref(), Some("Not ready"));
}

// ---- Force sync ----

fn unlocked() -> SafetyGate {
    let gate = SafetyGate::default();
    gate.set_unlocked("acme-uat", true);
    gate
}

#[tokio::test]
async fn force_sync_annotates_an_external_secret() {
    let api = MockApi::new();
    let path = "/apis/external-secrets.io/v1/namespaces/shop/externalsecrets/toast";
    api.on("PATCH", path, toast());
    let before = jiff::Timestamp::now().as_second();
    let n = force_sync(&api.client(), &unlocked(), "acme-uat", &es_kind_ref(), Some("shop"), "toast", false).await.unwrap();
    assert_eq!(n, 1);
    let call = &api.calls_to("PATCH", path)[0];
    let stamp: i64 = call.json()["metadata"]["annotations"]["force-sync"].as_str().unwrap().parse().unwrap();
    assert!(stamp >= before);
    assert!(!call.query.contains("dryRun"));
}

#[tokio::test]
async fn force_sync_is_gated_except_for_dry_runs() {
    let api = MockApi::new();
    let path = "/apis/external-secrets.io/v1/namespaces/shop/externalsecrets/toast";
    api.on("PATCH", path, toast());
    let locked = SafetyGate::default();
    let err = force_sync(&api.client(), &locked, "acme-uat", &es_kind_ref(), Some("shop"), "toast", false).await.unwrap_err();
    assert!(matches!(err, Error::Locked(_)));
    assert!(api.calls_to("PATCH", path).is_empty());
    force_sync(&api.client(), &locked, "acme-uat", &es_kind_ref(), Some("shop"), "toast", true).await.unwrap();
    assert!(api.calls_to("PATCH", path)[0].query.contains("dryRun=All"));

    let store = kind("external-secrets.io", "v1", "SecretStore", "secretstores", true);
    let err = force_sync(&api.client(), &unlocked(), "acme-uat", &store, Some("shop"), "s", false).await.unwrap_err();
    assert!(err.to_string().contains("can't be force-synced"));
}

#[tokio::test]
async fn force_sync_of_a_cluster_external_secret_reaches_its_external_secrets() {
    let api = MockApi::new();
    let ces_path = "/apis/external-secrets.io/v1/clusterexternalsecrets/shared";
    api.on("PATCH", ces_path, ces(json!({"provisionedNamespaces": ["shop", "web"]})));
    api.on("PATCH", "/apis/external-secrets.io/v1/namespaces/shop/externalsecrets/shared-es", json!({}));
    api.on("PATCH", "/apis/external-secrets.io/v1/namespaces/web/externalsecrets/shared-es", json!({}));
    let n = force_sync(&api.client(), &unlocked(), "acme-uat", &ces_kind_ref(), None, "shared", false).await.unwrap();
    assert_eq!(n, 2);
    let stamp = api.calls_to("PATCH", ces_path)[0].json()["metadata"]["annotations"]["force-sync"].clone();
    let child = api.calls_to("PATCH", "/apis/external-secrets.io/v1/namespaces/web/externalsecrets/shared-es");
    assert_eq!(child[0].json()["metadata"]["annotations"]["force-sync"], stamp);

    // A namespace it can't annotate is reported; the rest still happened.
    let api = MockApi::new();
    api.on("PATCH", ces_path, ces(json!({"provisionedNamespaces": ["shop", "web"]})));
    api.on("PATCH", "/apis/external-secrets.io/v1/namespaces/shop/externalsecrets/shared-es", json!({}));
    api.on_error("PATCH", "/apis/external-secrets.io/v1/namespaces/web/externalsecrets/shared-es", 403, "Forbidden", "no");
    let err = force_sync(&api.client(), &unlocked(), "acme-uat", &ces_kind_ref(), None, "shared", false).await.unwrap_err();
    assert!(err.to_string().contains("1 of them couldn't be annotated directly: web:"), "{err}");
    assert_eq!(api.calls_to("PATCH", "/apis/external-secrets.io/v1/namespaces/shop/externalsecrets/shared-es").len(), 1);
}

#[test]
fn only_unused_optional_columns_are_dropped() {
    let row = |c: &[&str]| InfoTableRow::new(c.iter().map(|s| (*s).to_owned()).collect());
    let table = |rows| InfoTable { columns: ["A", "B", "C", "D"].map(String::from).to_vec(), mono: vec![0, 2, 3], rows, ..Default::default() };
    let t = drop_empty_columns(table(vec![row(&["a", "", "c", ""]), row(&["", "", "", "d"])]), &[1, 2]);
    assert_eq!(t.columns, ["A", "C", "D"]);
    assert_eq!(t.mono, [0, 1, 2]);
    assert_eq!(cells(&t), vec![vec!["a", "c", ""], vec!["", "", "d"]]);
    let t = drop_empty_columns(table(vec![row(&["a", "", "", ""])]), &[1, 2, 3]);
    assert_eq!((t.columns.len(), t.mono.clone()), (1, vec![0]));
    assert_eq!(drop_empty_columns(table(vec![]), &[1, 2, 3]).columns.len(), 4, "an empty table keeps its headings");
}
