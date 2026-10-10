//! Coverage tests for the Storage section's Details: every branch of the
//! describe functions, and the related lookups against the fake API server.

use super::*;
use crate::config::describe as describe_object;
use crate::testing::{MockApi, list, object, pod};
use serde_json::json;

const PODS: &str = "/api/v1/namespaces/shop/pods";
const CLASSES: &str = "/apis/storage.k8s.io/v1/storageclasses";
const ALL_CLAIMS: &str = "/api/v1/persistentvolumeclaims";
const CSI_NODES: &str = "/apis/storage.k8s.io/v1/csinodes";

fn now() -> jiff::Timestamp {
    "2026-10-10T12:00:00Z".parse().unwrap()
}

fn row<'a>(d: &'a ObjectDetails, key: &str) -> Option<&'a str> {
    d.info.iter().find(|r| r.key == key).map(|r| r.value.as_str())
}

fn info<'a>(d: &'a ObjectDetails, key: &str) -> &'a InfoRow {
    d.info.iter().find(|r| r.key == key).unwrap_or_else(|| panic!("no {key} row"))
}

fn section<'a>(d: &'a ObjectDetails, id: &str) -> &'a crate::types::InfoSection {
    d.sections.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no {id} section"))
}

fn cells(t: &InfoTable) -> Vec<Vec<&str>> {
    t.rows.iter().map(|r| r.cells.iter().map(String::as_str).collect()).collect()
}

async fn related_details(client: &Client, key: &str, v: &Value) -> Result<ObjectDetails> {
    let mut d = describe_object(key, v, now());
    let name = d.name.clone();
    let ns = d.namespace.clone().unwrap_or_default();
    related(client, key, &ns, &name, v, &mut d).await?;
    Ok(d)
}

fn claim_value(name: &str, spec: Value, status: Value) -> Value {
    object("v1", "PersistentVolumeClaim", Some("shop"), name, json!({"spec": spec, "status": status}))
}

fn storage_class(name: &str, extra: Value) -> Value {
    let mut extra = extra;
    crate::testing::merge(&mut extra, json!({"provisioner": "ebs.csi.aws.com"}));
    object("storage.k8s.io/v1", "StorageClass", None, name, extra)
}

fn mounting(name: &str, node: &str, claim: &str, phase: &str) -> Value {
    pod("shop", name, node, json!({"spec": {"volumes": [{"name": "data", "persistentVolumeClaim": {"claimName": claim}}]},
                                   "status": {"phase": phase}}))
}

// ---- small helpers ----

#[test]
fn access_modes_reclaim_and_sizes_in_words() {
    assert_eq!(
        access_modes(&json!(["ReadWriteOnce", "ReadOnlyMany", "ReadWriteMany", "ReadWriteOncePod", "Odd"])),
        "ReadWriteOnce (one node), ReadOnlyMany (many nodes, read-only), ReadWriteMany (many nodes), ReadWriteOncePod (one pod), Odd"
    );
    assert_eq!(access_modes(&json!(null)), "—");
    assert!(reclaim_text("Retain").starts_with("Retain: the volume and its data stay"));
    assert!(reclaim_text("Recycle").starts_with("Recycle (deprecated)"));
    assert_eq!(reclaim_text("Shred"), "Shred");
    assert!(binding_text("WaitForFirstConsumer").starts_with("WaitForFirstConsumer:"));
    assert!(binding_text("").starts_with("Immediate:"));
    assert_eq!(bytes_text(512.0), "512 B");
    assert_eq!(bytes_text(1536.0), "1.5 KiB");
    assert_eq!(bytes_text(3.0 * 1024.0 * 1024.0 * 1024.0), "3.0 GiB");
    // TiB is the largest unit.
    assert_eq!(bytes_text(2048.0 * 1024f64.powi(4)), "2048.0 TiB");
}

// ---- PersistentVolumeClaim ----

#[test]
fn claim_rows_for_every_spec_field() {
    let v = claim_value(
        "restore",
        json!({"accessModes": ["ReadWriteOncePod"], "resources": {"requests": {"storage": "5Gi"}}, "storageClassName": "",
               "volumeMode": "Block", "volumeAttributesClassName": "fast",
               "dataSource": {"kind": "PersistentVolumeClaim", "name": "old"},
               "dataSourceRef": {"apiGroup": "snapshot.storage.k8s.io", "kind": "VolumeSnapshot", "name": "snap-1"},
               "selector": {"matchLabels": {"tier": "gold"}}, "volumeName": ""}),
        json!({"phase": "Bound", "capacity": {"storage": "5Gi"}}),
    );
    let d = describe_object("core/PersistentVolumeClaim", &v, now());
    assert_eq!(row(&d, "Requested"), Some("5Gi"));
    assert_eq!(row(&d, "Capacity"), Some("5Gi"));
    assert_eq!(row(&d, "Access"), Some("ReadWriteOncePod (one pod)"));
    let class = info(&d, "Storage class");
    assert_eq!((class.value.as_str(), class.link.is_none()), ("none: binds only to a volume without a class", true));
    assert_eq!(row(&d, "Volume"), None);
    assert_eq!(row(&d, "Volume mode"), Some("Block: a raw device, no filesystem"));
    // dataSourceRef wins over dataSource.
    assert_eq!(row(&d, "Created from"), Some("VolumeSnapshot snap-1"));
    assert_eq!(row(&d, "Attributes class"), Some("fast"));
    assert_eq!(row(&d, "Volume selector"), Some("tier=gold"));
    assert!(info(&d, "Volume selector").mono);
    assert_eq!(d.status.as_deref(), Some("Bound"));

    let v = claim_value("clone", json!({"storageClassName": "gp3", "dataSource": {"kind": "PersistentVolumeClaim", "name": "old"}}), json!({}));
    let d = describe_object("core/PersistentVolumeClaim", &v, now());
    assert_eq!(row(&d, "Created from"), Some("PersistentVolumeClaim old"));
    assert_eq!(row(&d, "Capacity"), None);
    let class = info(&d, "Storage class");
    assert_eq!(class.link.as_ref().map(|l| (l.kind.as_str(), l.name.as_str())), Some(("storage.k8s.io/StorageClass", "gp3")));

    // A resize condition only shows on a bound claim.
    let v = claim_value("x", json!({}), json!({"phase": "Pending", "conditions": [{"type": "Resizing", "status": "True"}]}));
    assert_eq!(describe_object("core/PersistentVolumeClaim", &v, now()).status.as_deref(), Some("Pending"));
    let v = claim_value("x", json!({}), json!({"phase": "Bound", "conditions": [{"type": "Resizing", "status": "False"}]}));
    assert_eq!(describe_object("core/PersistentVolumeClaim", &v, now()).status.as_deref(), Some("Bound"));
}

#[tokio::test]
async fn bound_claim_with_pods_and_disk_usage() {
    let api = MockApi::new();
    api.on("GET", &format!("{CLASSES}/gp3"), storage_class("gp3", json!({"allowVolumeExpansion": true})));
    let ephemeral = pod("shop", "worker", "n2", json!({"spec": {"volumes": [
        {"name": "data", "ephemeral": {"volumeClaimTemplate": {"spec": {}}}},
        {"name": "cfg", "configMap": {"name": "cfg"}}]}}));
    api.on("GET", PODS, list("PodList", vec![
        mounting("db-0", "n1", "data", "Running"),
        mounting("db-1", "n3", "other", "Running"),
        mounting("db-2", "n4", "data", "Pending"),
        pod("shop", "plain", "n1", json!({})),
        ephemeral,
    ]));
    // n1 answers without this claim's volume, n3 is not asked (not a user).
    api.on("GET", "/api/v1/nodes/n1/proxy/stats/summary", json!({"pods": [{"volume": [
        {"name": "data", "pvcRef": {"name": "data", "namespace": "other"}, "usedBytes": 1.0, "capacityBytes": 2.0}]}]}));
    api.on("GET", "/api/v1/nodes/n2/proxy/stats/summary", json!({"pods": [{"volume": [
        {"name": "data", "pvcRef": {"name": "worker-data", "namespace": "shop"}, "usedBytes": 1.0, "capacityBytes": 4.0}]}]}));
    let v = claim_value("data", json!({"storageClassName": "gp3", "volumeName": "pvc-1"}), json!({"phase": "Bound"}));
    let d = related_details(&api.client(), "core/PersistentVolumeClaim", &v).await.unwrap();

    assert_eq!(row(&d, "Can expand"), Some("yes"));
    assert_eq!(row(&d, "Why pending"), None);
    let mounted = section(&d, "mounted-by");
    assert_eq!(mounted.summary, "2 pods");
    let t = mounted.table.as_ref().unwrap();
    let names: Vec<(&str, &str)> = t.rows.iter().map(|r| (r.cells[0].as_str(), r.cells[2].as_str())).collect();
    assert_eq!(names, [("db-0", "n1"), ("db-2", "n4")]);
    let l = t.rows[0].link.as_ref().unwrap();
    assert_eq!((l.kind.as_str(), l.namespace.as_deref()), ("core/Pod", Some("shop")));
    // Only the running user's kubelet is asked, and it doesn't know the claim.
    assert_eq!(row(&d, "Disk usage"), Some("unknown: the kubelet didn't report it (e.g. on Fargate)"));
    assert_eq!(api.calls_to("GET", "/api/v1/nodes/n1/proxy/stats/summary").len(), 1);
    assert!(api.calls_to("GET", "/api/v1/nodes/n4/proxy/stats/summary").is_empty());
    assert!(d.sections.iter().all(|s| s.id != "disk"));

    // A generic ephemeral volume's claim is named <pod>-<volume>; its kubelet reports usage.
    let v = claim_value("worker-data", json!({"storageClassName": "gp3"}), json!({"phase": "Bound"}));
    let d = related_details(&api.client(), "core/PersistentVolumeClaim", &v).await.unwrap();
    assert_eq!(d.sections[0].id, "disk");
    assert_eq!(d.sections[0].summary, "25% used");
    assert_eq!(section(&d, "mounted-by").summary, "1 pod");
    assert_eq!(row(&d, "Disk usage"), None);
}

#[tokio::test]
async fn disk_usage_from_the_second_node_when_the_first_fails() {
    let api = MockApi::new();
    api.on("GET", &format!("{CLASSES}/gp3"), storage_class("gp3", json!({})));
    api.on("GET", PODS, list("PodList", vec![mounting("a", "n1", "data", "Running"), mounting("b", "n2", "data", "Running")]));
    api.on_text("GET", "/api/v1/nodes/n1/proxy/stats/summary", 503, "kubelet unavailable");
    api.on("GET", "/api/v1/nodes/n2/proxy/stats/summary", json!({"pods": [{"volume": [
        {"name": "data", "pvcRef": {"name": "data", "namespace": "shop"}, "usedBytes": 900.0, "capacityBytes": 1000.0}]}]}));
    let v = claim_value("data", json!({"storageClassName": "gp3"}), json!({"phase": "Bound"}));
    let d = related_details(&api.client(), "core/PersistentVolumeClaim", &v).await.unwrap();
    assert_eq!(row(&d, "Can expand"), Some("no: its storage class doesn't allow it"));
    let disk = &d.sections[0];
    assert_eq!((disk.id.as_str(), disk.summary.as_str()), ("disk", "90% used"));
    let r = &disk.table.as_ref().unwrap().rows[0];
    assert_eq!(r.cells, ["Space", "900 B", "100 B", "1000 B"]);
    assert!(r.warn);
    assert_eq!(r.fill, Some(0.9));
}

#[tokio::test]
async fn why_a_claim_is_pending() {
    let api = MockApi::new();
    api.on("GET", PODS, list("PodList", vec![]));
    api.on("GET", &format!("{CLASSES}/gp3"), storage_class("gp3", json!({"volumeBindingMode": "Immediate"})));
    api.on("GET", &format!("{CLASSES}/wait"), storage_class("wait", json!({"volumeBindingMode": "WaitForFirstConsumer"})));
    let client = api.client();

    let v = claim_value("data", json!({"storageClassName": "gp3"}), json!({"phase": "Pending"}));
    let d = related_details(&client, "core/PersistentVolumeClaim", &v).await.unwrap();
    assert_eq!(row(&d, "Why pending"), Some("waiting for ebs.csi.aws.com to create a volume (see events)"));
    let mounted = section(&d, "mounted-by");
    assert_eq!((mounted.summary.as_str(), mounted.empty.as_deref()), ("none", Some("No pods use it right now.")));
    assert_eq!(row(&d, "Disk usage"), None);

    let v = claim_value("data", json!({"storageClassName": "wait"}), json!({"phase": "Pending"}));
    let d = related_details(&client, "core/PersistentVolumeClaim", &v).await.unwrap();
    assert_eq!(row(&d, "Why pending"), Some("its class creates the volume when a pod first uses the claim"));

    // A class that doesn't exist.
    let v = claim_value("data", json!({"storageClassName": "gone"}), json!({"phase": "Pending"}));
    let d = related_details(&client, "core/PersistentVolumeClaim", &v).await.unwrap();
    assert_eq!(row(&d, "Can expand"), Some("no: its storage class doesn't exist"));
    assert_eq!(row(&d, "Why pending"), None);

    // An empty class: bound by hand, no class is looked up.
    let before = api.calls().len();
    let v = claim_value("data", json!({"storageClassName": ""}), json!({"phase": "Bound"}));
    let d = related_details(&client, "core/PersistentVolumeClaim", &v).await.unwrap();
    assert_eq!(row(&d, "Can expand"), Some("no: only claims provisioned by a storage class can grow"));
    assert_eq!(row(&d, "Storage class"), Some("none: binds only to a volume without a class"));
    assert!(api.calls()[before..].iter().all(|c| !c.path.starts_with(CLASSES)));
}

#[tokio::test]
async fn claim_without_a_class_uses_the_default_one() {
    let api = MockApi::new();
    api.on("GET", PODS, list("PodList", vec![]));
    api.on("GET", CLASSES, list("StorageClassList", vec![
        storage_class("slow", json!({})),
        storage_class("gp3", json!({"metadata": {"annotations": {"storageclass.kubernetes.io/is-default-class": "true"}},
                                    "allowVolumeExpansion": true, "volumeBindingMode": "WaitForFirstConsumer"})),
    ]));
    let v = claim_value("data", json!({}), json!({"phase": "Pending"}));
    let d = related_details(&api.client(), "core/PersistentVolumeClaim", &v).await.unwrap();
    assert_eq!(row(&d, "Storage class"), Some("the cluster's default"));
    assert_eq!(row(&d, "Can expand"), Some("yes"));
    assert_eq!(row(&d, "Why pending"), Some("its class creates the volume when a pod first uses the claim"));

    // No default class in the cluster.
    let api = MockApi::new();
    api.on("GET", PODS, list("PodList", vec![]));
    api.on("GET", CLASSES, list("StorageClassList", vec![storage_class("slow", json!({}))]));
    let d = related_details(&api.client(), "core/PersistentVolumeClaim", &v).await.unwrap();
    assert_eq!(row(&d, "Storage class"), Some("none (the cluster has no default class)"));
    assert_eq!(row(&d, "Can expand"), Some("no: only claims provisioned by a storage class can grow"));

    // Classes can't be listed: same as no default.
    let api = MockApi::new();
    api.on("GET", PODS, list("PodList", vec![]));
    api.on_error("GET", CLASSES, 403, "Forbidden", "storageclasses is forbidden");
    let d = related_details(&api.client(), "core/PersistentVolumeClaim", &v).await.unwrap();
    assert_eq!(row(&d, "Storage class"), Some("none (the cluster has no default class)"));
}

#[tokio::test]
async fn claim_lookup_reports_pod_list_errors() {
    let api = MockApi::new();
    api.on_error("GET", PODS, 403, "Forbidden", "pods is forbidden");
    let v = claim_value("data", json!({"storageClassName": ""}), json!({}));
    let err = related_details(&api.client(), "core/PersistentVolumeClaim", &v).await.unwrap_err();
    assert_eq!(err.user_message(), "pods is forbidden");
}

#[test]
fn volume_usage_edge_cases() {
    assert!(volume_usage(&json!({}), "shop", "data").is_none());
    let vol = |extra: Value| {
        let mut v = json!({"pvcRef": {"name": "data", "namespace": "shop"}});
        crate::testing::merge(&mut v, extra);
        json!({"pods": [{"name": "p"}, {"volume": [v]}]})
    };
    // Without a capacity there's nothing to show.
    assert!(volume_usage(&vol(json!({"usedBytes": 10})), "shop", "data").is_none());
    assert!(volume_usage(&vol(json!({"capacityBytes": 10})), "shop", "data").is_none());

    // Zero capacity: no fill, no summary; free is worked out.
    let s = volume_usage(&vol(json!({"usedBytes": 0, "capacityBytes": 0, "inodesUsed": 0, "inodes": 0})), "shop", "data").unwrap();
    assert_eq!(s.summary, "");
    let rows = &s.table.as_ref().unwrap().rows;
    assert_eq!((rows[0].fill, rows[0].warn), (None, false));
    assert_eq!(rows[1].cells, ["Files (inodes)", "0", "—", "0"]);
    assert_eq!((rows[1].fill, rows[1].warn), (None, false));

    let s = volume_usage(&vol(json!({"usedBytes": 2048, "capacityBytes": 1024, "inodesUsed": 95, "inodes": 100})), "shop", "data").unwrap();
    let t = s.table.unwrap();
    assert_eq!(t.columns, ["", "Used", "Free", "Size"]);
    // Overfull is capped at a full bar, and free at nothing.
    assert_eq!((t.rows[0].fill, t.rows[0].warn), (Some(1.0), true));
    assert_eq!(t.rows[0].cells[2], "0 B");
    assert_eq!((t.rows[1].fill, t.rows[1].warn), (Some(0.95), true));
}

// ---- PersistentVolume ----

#[test]
fn every_volume_source() {
    let cases = [
        (json!({"hostPath": {"path": "/data"}}), ("hostPath", "/data")),
        (json!({"local": {"path": "/mnt/ssd"}}), ("local", "/mnt/ssd")),
        (json!({"nfs": {"server": "nas.acme.io", "path": "/exports"}}), ("nfs", "nas.acme.io:/exports")),
        (json!({"awsElasticBlockStore": {"volumeID": "vol-1"}}), ("awsElasticBlockStore", "vol-1")),
        (json!({"gcePersistentDisk": {"pdName": "pd-1"}}), ("gcePersistentDisk", "pd-1")),
        (json!({"azureDisk": {"diskName": "disk-1"}}), ("azureDisk", "disk-1")),
        (json!({"azureFile": {"shareName": "share-1"}}), ("azureFile", "share-1")),
        (json!({"iscsi": {"iqn": "iqn.2026-10.io.acme:t"}}), ("iscsi", "iqn.2026-10.io.acme:t")),
        (json!({"fc": {"lun": 3}}), ("fc", "3")),
        (json!({"cephfs": {"path": "/vol"}}), ("cephfs", "/vol")),
        (json!({"rbd": {"image": "img"}}), ("rbd", "img")),
        (json!({"csi": {"driver": "ebs.csi.aws.com"}}), ("CSI ebs.csi.aws.com", "")),
    ];
    for (spec, (kind, id)) in cases {
        assert_eq!(volume_source(&spec), Some((kind.to_owned(), id.to_owned())), "{spec}");
    }
    assert_eq!(volume_source(&json!({"flexVolume": {}})), None);
}

#[test]
fn volume_rows() {
    let v = json!({"kind": "PersistentVolume", "metadata": {"name": "pv-1"},
        "spec": {"capacity": {"storage": "100Gi"}, "accessModes": ["ReadWriteMany"], "storageClassName": "",
                 "claimRef": {"namespace": "shop", "name": "data"}, "local": {}, "volumeMode": "Block",
                 "mountOptions": ["noatime", "nfsvers=4.1"],
                 "nodeAffinity": {"required": {"nodeSelectorTerms": [
                     {"matchExpressions": [{"key": "kubernetes.io/hostname", "operator": "In", "values": ["n1", "n2"]},
                                           {"key": "disk", "operator": "Exists"}]},
                     {"matchFields": [{"key": "metadata.name", "operator": "In", "values": ["n3"]}]}]}}},
        "status": {"phase": "Bound", "lastPhaseTransitionTime": "2026-10-09T00:00:00Z", "message": "all good"}});
    let d = describe_object("core/PersistentVolume", &v, now());
    assert_eq!(row(&d, "Capacity"), Some("100Gi"));
    assert_eq!(row(&d, "Access"), Some("ReadWriteMany (many nodes)"));
    // Unset reclaim policy reads as Retain.
    assert!(row(&d, "Reclaim policy").unwrap().starts_with("Retain:"));
    assert_eq!(row(&d, "Storage class"), None);
    let claim = info(&d, "Claim");
    assert_eq!(claim.value, "shop/data");
    let l = claim.link.as_ref().unwrap();
    assert_eq!((l.kind.as_str(), l.namespace.as_deref(), l.name.as_str()), ("core/PersistentVolumeClaim", Some("shop"), "data"));
    assert_eq!(row(&d, "Source"), Some("local"));
    assert_eq!(row(&d, "Volume ID"), None);
    assert_eq!(row(&d, "Volume mode"), Some("Block: a raw device, no filesystem"));
    let nodes: Vec<&str> = d.info.iter().filter(|r| r.key == "Nodes").map(|r| r.value.as_str()).collect();
    assert_eq!(nodes, ["kubernetes.io/hostname in (n1, n2) and disk exists ()"]);
    assert_eq!(row(&d, "Mount options"), Some("noatime, nfsvers=4.1"));
    assert!(info(&d, "Phase since").time);
    assert_eq!(row(&d, "Message"), Some("all good"));
    assert_eq!(d.status.as_deref(), Some("Bound"));

    let bare = json!({"kind": "PersistentVolume", "metadata": {"name": "pv-2"}, "spec": {"storageClassName": "gp3"}});
    let d = describe_object("core/PersistentVolume", &bare, now());
    assert_eq!(d.status.as_deref(), Some("Pending"));
    assert_eq!((row(&d, "Capacity"), row(&d, "Access")), (Some(""), Some("—")));
    assert_eq!(info(&d, "Storage class").link.as_ref().unwrap().name, "gp3");
    assert_eq!((row(&d, "Claim"), row(&d, "Source"), row(&d, "Message")), (None, None, None));
}

// ---- StorageClass ----

#[test]
fn storage_class_rows() {
    let sc = json!({"kind": "StorageClass", "metadata": {"name": "nfs"}, "provisioner": "nfs.csi.k8s.io",
        "reclaimPolicy": "Retain", "mountOptions": ["hard"],
        "allowedTopologies": [{"matchLabelExpressions": [{"key": "topology.kubernetes.io/zone", "values": ["a", "b"]}]}, {}],
        "parameters": {"server": "nas.acme.io", "weird": 3}});
    let d = describe_object("storage.k8s.io/StorageClass", &sc, now());
    assert_eq!(row(&d, "Provisioner"), Some("nfs.csi.k8s.io"));
    assert_eq!(row(&d, "Default class"), Some("no"));
    assert!(row(&d, "Reclaim policy").unwrap().starts_with("Retain:"));
    assert!(row(&d, "Binding").unwrap().starts_with("Immediate:"));
    assert_eq!(row(&d, "Expansion"), Some("not allowed"));
    assert_eq!(row(&d, "Mount options"), Some("hard"));
    assert_eq!(row(&d, "Allowed topology"), Some("topology.kubernetes.io/zone in (a, b)"));
    assert!(info(&d, "Allowed topology").mono);
    let params = section(&d, "parameters").table.as_ref().unwrap();
    assert_eq!(cells(params), [vec!["server", "nas.acme.io"], vec!["weird", ""]]);
    assert_eq!(d.status, None);

    let bare = json!({"kind": "StorageClass", "metadata": {"name": "bare"}});
    let d = describe_object("storage.k8s.io/StorageClass", &bare, now());
    assert!(row(&d, "Reclaim policy").unwrap().starts_with("Delete:"));
    assert!(d.sections.is_empty());
    assert_eq!((row(&d, "Mount options"), row(&d, "Allowed topology")), (None, None));
}

fn pvc(ns: &str, name: &str, class: Option<&str>, phase: &str, size: Option<&str>) -> Value {
    let mut status = json!({"phase": phase});
    if let Some(s) = size {
        status["capacity"] = json!({"storage": s});
    }
    let spec = match class {
        Some(c) => json!({"storageClassName": c}),
        None => json!({}),
    };
    object("v1", "PersistentVolumeClaim", Some(ns), name, json!({"spec": spec, "status": status}))
}

#[tokio::test]
async fn storage_class_lists_its_claims() {
    let api = MockApi::new();
    api.on("GET", ALL_CLAIMS, list("PersistentVolumeClaimList", vec![
        pvc("shop", "data", Some("gp3"), "Bound", Some("10Gi")),
        pvc("acme", "logs", Some("gp3"), "Pending", None),
        pvc("acme", "classless", None, "Bound", Some("1Gi")),
        pvc("acme", "other", Some("nfs"), "Bound", Some("1Gi")),
        object("v1", "PersistentVolumeClaim", Some("acme"), "bare", json!({"spec": {"storageClassName": "gp3"}})),
    ]));
    let client = api.client();
    let sc = |default: bool| {
        let ann = if default { json!({"storageclass.kubernetes.io/is-default-class": "true"}) } else { json!({}) };
        json!({"kind": "StorageClass", "metadata": {"name": "gp3", "annotations": ann}, "provisioner": "ebs.csi.aws.com"})
    };

    let d = related_details(&client, "storage.k8s.io/StorageClass", &sc(false)).await.unwrap();
    let s = section(&d, "class-claims");
    let t = s.table.as_ref().unwrap();
    // Sorted by namespace, then name.
    assert_eq!(cells(t), [vec!["acme", "bare", "", ""], vec!["acme", "logs", "Pending", ""], vec!["shop", "data", "Bound", "10Gi"]]);
    assert_eq!(t.rows.iter().map(|r| r.warn).collect::<Vec<_>>(), [true, true, false]);
    let l = t.rows[2].link.as_ref().unwrap();
    assert_eq!((l.kind.as_str(), l.namespace.as_deref(), l.name.as_str()), ("core/PersistentVolumeClaim", Some("shop"), "data"));

    // The default class also counts claims that name none.
    let d = related_details(&client, "storage.k8s.io/StorageClass", &sc(true)).await.unwrap();
    let names: Vec<&str> = section(&d, "class-claims").table.as_ref().unwrap().rows.iter().map(|r| r.cells[1].as_str()).collect();
    assert_eq!(names, ["bare", "classless", "logs", "data"]);

    let api = MockApi::new();
    api.on("GET", ALL_CLAIMS, list("PersistentVolumeClaimList", vec![]));
    let d = related_details(&api.client(), "storage.k8s.io/StorageClass", &sc(false)).await.unwrap();
    assert_eq!(section(&d, "class-claims").empty.as_deref(), Some("No claims use it."));

    let api = MockApi::new();
    api.on_error("GET", ALL_CLAIMS, 403, "Forbidden", "persistentvolumeclaims is forbidden");
    let err = related_details(&api.client(), "storage.k8s.io/StorageClass", &sc(false)).await.unwrap_err();
    assert_eq!(err.user_message(), "persistentvolumeclaims is forbidden");
}

// ---- VolumeAttachment and CSI ----

#[test]
fn volume_attachment_states() {
    let va = |status: Value, deleting: bool| {
        let mut v = json!({"kind": "VolumeAttachment", "metadata": {"name": "csi-1"},
            "spec": {"attacher": "ebs.csi.aws.com", "source": {}}, "status": status});
        if deleting {
            v["metadata"]["deletionTimestamp"] = json!("2026-10-10T11:00:00Z");
        }
        describe_object("storage.k8s.io/VolumeAttachment", &v, now())
    };
    let d = va(json!({"attached": true}), false);
    assert_eq!(d.status.as_deref(), Some("Attached"));
    assert_eq!((row(&d, "Attached"), row(&d, "Node"), row(&d, "Volume")), (Some("yes"), None, None));
    assert_eq!(va(json!({"attached": true}), true).status.as_deref(), Some("Detaching"));
    assert_eq!(va(json!({}), false).status.as_deref(), Some("Attaching"));

    let d = va(json!({"attached": true, "detachError": {"message": "still mounted"}}), true);
    assert_eq!(d.status.as_deref(), Some("Error"));
    assert_eq!(row(&d, "Detach error"), Some("still mounted"));
    assert_eq!(row(&d, "Detach error at"), None);

    let v = json!({"kind": "VolumeAttachment", "metadata": {"name": "csi-2"},
        "spec": {"attacher": "ebs.csi.aws.com", "nodeName": "n1", "source": {"persistentVolumeName": "pv-1"}},
        "status": {"attached": false, "attachError": {"message": "in use", "time": "2026-10-10T11:59:00Z"}}});
    let d = describe_object("storage.k8s.io/VolumeAttachment", &v, now());
    let node = info(&d, "Node").link.as_ref().unwrap();
    assert_eq!((node.kind.as_str(), node.namespace.as_deref(), node.name.as_str()), ("core/Node", None, "n1"));
    assert_eq!(info(&d, "Volume").link.as_ref().unwrap().kind, "core/PersistentVolume");
    assert!(info(&d, "Attach error at").time);
}

#[test]
fn csi_driver_rows() {
    let d = describe_object("storage.k8s.io/CSIDriver", &json!({"kind": "CSIDriver", "metadata": {"name": "ebs.csi.aws.com"}, "spec": {}}), now());
    assert_eq!(
        (row(&d, "Needs attaching"), row(&d, "Pod info on mount"), row(&d, "Reports capacity"), row(&d, "Volume modes")),
        (Some("yes"), Some("no"), Some("no"), Some("Persistent"))
    );
    assert_eq!((row(&d, "fsGroup policy"), row(&d, "SELinux mount"), row(&d, "Republishes"), row(&d, "Token audiences")), (None, None, None, None));

    let v = json!({"kind": "CSIDriver", "metadata": {"name": "secrets.csi.k8s.io"},
        "spec": {"attachRequired": false, "podInfoOnMount": true, "storageCapacity": true,
                 "volumeLifecycleModes": ["Persistent", "Ephemeral"], "fsGroupPolicy": "File", "seLinuxMount": true,
                 "requiresRepublish": true, "tokenRequests": [{"audience": "vault"}, {"expirationSeconds": 600}, {"audience": "sts"}]}});
    let d = describe_object("storage.k8s.io/CSIDriver", &v, now());
    assert_eq!(
        (row(&d, "Needs attaching"), row(&d, "Pod info on mount"), row(&d, "Reports capacity"), row(&d, "Volume modes")),
        (Some("no"), Some("yes"), Some("yes"), Some("Persistent, Ephemeral"))
    );
    assert_eq!(row(&d, "fsGroup policy"), Some("File"));
    assert_eq!(row(&d, "SELinux mount"), Some("yes"));
    assert_eq!(row(&d, "Republishes"), Some("yes: volumes are re-mounted periodically"));
    assert_eq!(row(&d, "Token audiences"), Some("vault, sts"));
}

#[tokio::test]
async fn csi_driver_lists_the_nodes_running_it() {
    let node = |name: &str, drivers: Value| object("storage.k8s.io/v1", "CSINode", None, name, json!({"spec": {"drivers": drivers}}));
    let api = MockApi::new();
    api.on("GET", CSI_NODES, list("CSINodeList", vec![
        node("n1", json!([{"name": "ebs.csi.aws.com", "nodeID": "i-1", "allocatable": {"count": 25}},
                          {"name": "efs.csi.aws.com", "nodeID": "i-1"}])),
        node("n2", json!([{"name": "ebs.csi.aws.com", "nodeID": "i-2"}])),
        node("n3", json!([])),
    ]));
    let v = json!({"kind": "CSIDriver", "metadata": {"name": "ebs.csi.aws.com"}, "spec": {}});
    let d = related_details(&api.client(), "storage.k8s.io/CSIDriver", &v).await.unwrap();
    let s = section(&d, "driver-nodes");
    assert_eq!(s.summary, "2 nodes");
    let t = s.table.as_ref().unwrap();
    assert_eq!(cells(t), [vec!["n1", "i-1", "25"], vec!["n2", "i-2", "—"]]);
    assert_eq!(t.rows[0].link.as_ref().unwrap().kind, "core/Node");

    let v = json!({"kind": "CSIDriver", "metadata": {"name": "nobody.csi.io"}, "spec": {}});
    let d = related_details(&api.client(), "storage.k8s.io/CSIDriver", &v).await.unwrap();
    let s = section(&d, "driver-nodes");
    assert_eq!((s.summary.as_str(), s.empty.as_deref()), ("none", Some("No nodes run it.")));

    let api = MockApi::new();
    api.on_error("GET", CSI_NODES, 403, "Forbidden", "csinodes is forbidden");
    let err = related_details(&api.client(), "storage.k8s.io/CSIDriver", &v).await.unwrap_err();
    assert_eq!(err.user_message(), "csinodes is forbidden");
}

#[test]
fn csi_node_and_capacity_rows() {
    let v = json!({"kind": "CSINode", "metadata": {"name": "n1"}, "spec": {"drivers": [
        {"name": "ebs.csi.aws.com", "nodeID": "i-1", "allocatable": {"count": 25}, "topologyKeys": ["zone", "region"]},
        {"name": "efs.csi.aws.com", "nodeID": "i-1"}]}});
    let d = describe_object("storage.k8s.io/CSINode", &v, now());
    assert_eq!(info(&d, "Node").link.as_ref().unwrap().name, "n1");
    let t = section(&d, "drivers").table.as_ref().unwrap();
    assert_eq!(cells(t), [vec!["ebs.csi.aws.com", "i-1", "25", "zone, region"], vec!["efs.csi.aws.com", "i-1", "—", ""]]);
    assert_eq!(t.rows[1].link.as_ref().unwrap().kind, "storage.k8s.io/CSIDriver");
    let d = describe_object("storage.k8s.io/CSINode", &json!({"kind": "CSINode", "metadata": {"name": "n2"}}), now());
    assert_eq!(section(&d, "drivers").empty.as_deref(), Some("No CSI drivers on this node."));

    let cap = json!({"kind": "CSIStorageCapacity", "metadata": {"name": "c", "namespace": "kube-system"},
        "storageClassName": "gp3", "capacity": "500Gi", "maximumVolumeSize": "16Ti",
        "nodeTopology": {"matchLabels": {"zone": "a"}}});
    let d = describe_object("storage.k8s.io/CSIStorageCapacity", &cap, now());
    assert_eq!(info(&d, "Storage class").link.as_ref().unwrap().name, "gp3");
    assert_eq!((row(&d, "Capacity"), row(&d, "Largest volume"), row(&d, "Nodes")), (Some("500Gi"), Some("16Ti"), Some("zone=a")));
    let d = describe_object("storage.k8s.io/CSIStorageCapacity", &json!({"kind": "CSIStorageCapacity", "metadata": {"name": "c"}}), now());
    assert_eq!(d.info.iter().map(|r| (r.key.as_str(), r.value.as_str())).collect::<Vec<_>>(), [("Capacity", "unknown")]);
}

#[tokio::test]
async fn other_kinds_are_left_alone() {
    let api = MockApi::new();
    let v = json!({"kind": "PersistentVolume", "metadata": {"name": "pv"}});
    let mut d = describe_object("core/ConfigMap", &json!({"metadata": {"name": "x"}}), now());
    let before = (d.info.len(), d.sections.len());
    describe("core/ConfigMap", &mut d, &v);
    related(&api.client(), "core/PersistentVolume", "", "pv", &v, &mut d).await.unwrap();
    assert_eq!((d.info.len(), d.sections.len()), before);
    assert!(api.calls().is_empty());
}
