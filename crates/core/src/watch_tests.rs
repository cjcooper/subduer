use super::*;
use crate::testing::{MockApi, kind, list, object};
use crate::types::ResourceTable;
use serde_json::{Value, json};
use std::time::Duration;

const CMS: &str = "/api/v1/namespaces/shop/configmaps";

fn cm(name: &str) -> Value {
    object("v1", "ConfigMap", Some("shop"), name, json!({}))
}

/// One watch event; the fake server sends it as the whole watch body (one line).
fn event(kind: &str, obj: Value) -> Value {
    json!({"type": kind, "object": obj})
}

/// Runs the watch until it has emitted `n` tables (or fails the test after 10s).
async fn tables(client: kube::Client, kind: KindRef, namespace: Option<&str>, n: usize) -> Vec<ResourceTable> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let watch = watch_table(client, kind, namespace.map(str::to_owned), 42, move |t| {
        let _ = tx.send(t);
    });
    let collect = async {
        let mut out = vec![];
        while out.len() < n {
            out.push(rx.recv().await.expect("watch stopped"));
        }
        out
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! {
            out = collect => out,
            () = watch => panic!("watch ended early"),
        }
    })
    .await
    .expect("no table in time")
}

fn ids(t: &ResourceTable) -> Vec<&str> {
    t.rows.iter().map(|r| r.id.as_str()).collect()
}

#[tokio::test]
async fn first_table_is_the_initial_list() {
    let api = MockApi::new();
    api.on("GET", CMS, list("ConfigMapList", vec![cm("b"), cm("a")]));
    let k = kind("", "v1", "ConfigMap", "configmaps", true);
    let t = tables(api.client(), k.clone(), Some("shop"), 1).await.remove(0);
    assert_eq!(t.watch_id, 42);
    assert_eq!(t.error, None);
    assert_eq!(t.columns, table::columns(&k));
    assert_eq!(ids(&t), ["shop/a", "shop/b"]);
    assert!(api.calls_to("GET", CMS).iter().any(|c| !c.query.contains("watch=true")));
}

#[tokio::test]
async fn added_objects_show_up_after_the_throttle() {
    let api = MockApi::new();
    api.on("GET", CMS, list("ConfigMapList", vec![cm("a")]));
    api.on_query("GET", CMS, "watch=true", event("ADDED", cm("c")));
    let k = kind("", "v1", "ConfigMap", "configmaps", true);
    let t = tables(api.client(), k, Some("shop"), 2).await;
    assert_eq!(ids(&t[0]), ["shop/a"]);
    assert_eq!(ids(&t[1]), ["shop/a", "shop/c"]);
    assert!(api.calls_to("GET", CMS).iter().any(|c| c.query.contains("watch=true")));
}

#[tokio::test]
async fn deleted_objects_disappear() {
    let api = MockApi::new();
    api.on("GET", CMS, list("ConfigMapList", vec![cm("a"), cm("b")]));
    api.on_query("GET", CMS, "watch=true", event("DELETED", cm("a")));
    let k = kind("", "v1", "ConfigMap", "configmaps", true);
    let t = tables(api.client(), k, Some("shop"), 2).await;
    assert_eq!(ids(&t[0]), ["shop/a", "shop/b"]);
    assert_eq!(ids(&t[1]), ["shop/b"]);
}

#[tokio::test]
async fn errors_are_reported_with_the_last_rows() {
    let api = MockApi::new();
    api.on_error("GET", CMS, 403, "Forbidden", "configmaps is forbidden");
    let k = kind("", "v1", "ConfigMap", "configmaps", true);
    let t = tables(api.client(), k, Some("shop"), 1).await.remove(0);
    assert!(t.rows.is_empty());
    let error = t.error.expect("an error");
    assert!(error.contains("forbidden"), "{error}");
}

#[tokio::test]
async fn events_are_sorted_most_recent_first_across_namespaces() {
    let api = MockApi::new();
    let ev = |name: &str, last: &str| {
        object("v1", "Event", Some("shop"), name, json!({"lastTimestamp": last, "message": name}))
    };
    api.on("GET", "/api/v1/events", list("EventList", vec![ev("old", "2026-10-01T00:00:00Z"), ev("new", "2026-10-02T00:00:00Z")]));
    let k = kind("", "v1", "Event", "events", true);
    let t = tables(api.client(), k, None, 1).await.remove(0);
    assert_eq!(ids(&t), ["shop/new", "shop/old"]);
}

#[tokio::test]
async fn helm_releases_use_their_own_watch() {
    let api = MockApi::new();
    let k = crate::helm::release_kind();
    let t = tables(api.client(), k, Some("shop"), 1).await.remove(0);
    // Releases come from labelled Secrets, not a `releases` resource.
    assert!(api.calls().iter().all(|c| c.path.ends_with("/secrets")), "{:?}", api.calls());
    assert_eq!(t.watch_id, 42);
}

#[test]
fn row_keys_match_row_ids() {
    let obj: DynamicObject = serde_json::from_value(cm("a")).unwrap();
    assert_eq!(row_key(&obj), "shop/a");
    let obj: DynamicObject = serde_json::from_value(json!({"apiVersion": "v1", "kind": "Node", "metadata": {}})).unwrap();
    assert_eq!(row_key(&obj), "");
}
