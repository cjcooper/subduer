//! The Details tab of the Storage section's kinds: PersistentVolumeClaims
//! (size, disk usage from the kubelet, the pods mounting them, why one is
//! pending), PersistentVolumes (source, reclaim policy, where it can attach),
//! StorageClasses (and the claims using them), VolumeAttachments and the CSI
//! kinds. Built on `config`'s generic shape.

use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod};
use k8s_openapi::api::storage::v1::{CSINode, StorageClass};
use kube::api::{Api, ListParams};
use kube::{Client, ResourceExt};
use serde_json::Value;

use crate::config::{int_at, label_selector, plural, str_at, table_section};
use crate::types::{InfoRow, InfoTable, InfoTableRow, ObjectDetails, ObjectLink};
use crate::Result;

/// Fills in what comes from the object itself.
pub(crate) fn describe(key: &str, d: &mut ObjectDetails, v: &Value) {
    match key {
        "core/PersistentVolumeClaim" => claim(d, v),
        "core/PersistentVolume" => volume(d, v),
        "storage.k8s.io/StorageClass" => storage_class(d, v),
        "storage.k8s.io/VolumeAttachment" => attachment(d, v),
        "storage.k8s.io/CSIDriver" => csi_driver(d, v),
        "storage.k8s.io/CSINode" => csi_node(d, v),
        "storage.k8s.io/CSIStorageCapacity" => capacity(d, v),
        _ => {}
    }
}

/// Adds what needs other objects.
pub(crate) async fn related(
    client: &Client,
    key: &str,
    namespace: &str,
    name: &str,
    v: &Value,
    d: &mut ObjectDetails,
) -> Result<()> {
    match key {
        "core/PersistentVolumeClaim" => claim_related(client, namespace, name, v, d).await,
        "storage.k8s.io/StorageClass" => class_claims(client, name, d).await,
        "storage.k8s.io/CSIDriver" => driver_nodes(client, name, d).await,
        _ => Ok(()),
    }
}

fn link(kind: &str, namespace: Option<&str>, name: &str) -> ObjectLink {
    ObjectLink { kind: kind.into(), namespace: namespace.map(str::to_owned), name: name.into() }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect()
}

/// Access modes with what they mean.
fn access_modes(v: &Value) -> String {
    let modes: Vec<String> = strings(v)
        .into_iter()
        .map(|m| {
            let what = match m.as_str() {
                "ReadWriteOnce" => "one node",
                "ReadOnlyMany" => "many nodes, read-only",
                "ReadWriteMany" => "many nodes",
                "ReadWriteOncePod" => "one pod",
                _ => "",
            };
            if what.is_empty() { m } else { format!("{m} ({what})") }
        })
        .collect();
    if modes.is_empty() { "—".into() } else { modes.join(", ") }
}

fn reclaim_text(policy: &str) -> String {
    match policy {
        "Retain" => "Retain: the volume and its data stay after the claim is deleted".into(),
        "Delete" => "Delete: deleting the claim deletes the volume and its data".into(),
        "Recycle" => "Recycle (deprecated): the volume is wiped and reused".into(),
        other => other.to_owned(),
    }
}

fn binding_text(mode: &str) -> String {
    match mode {
        "WaitForFirstConsumer" => "WaitForFirstConsumer: created when a pod first uses the claim, in that pod's zone".into(),
        _ => "Immediate: created as soon as the claim exists".into(),
    }
}

/// Bytes as `3.2 GiB`.
pub(crate) fn bytes_text(n: f64) -> String {
    bytes(n)
}

fn bytes(n: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut n = n;
    let mut unit = 0;
    while n >= 1024.0 && unit < UNITS.len() - 1 {
        n /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{n:.0} B") } else { format!("{n:.1} {}", UNITS[unit]) }
}

// ---- PersistentVolumeClaim ----

fn claim(d: &mut ObjectDetails, v: &Value) {
    let (spec, status) = (&v["spec"], &v["status"]);
    let phase = str_at(status, "phase").unwrap_or_else(|| "Pending".into());
    let requested = str_at(&spec["resources"]["requests"], "storage").unwrap_or_default();
    let capacity = str_at(&status["capacity"], "storage");
    d.info.push(InfoRow::new("Requested", requested.clone()));
    match &capacity {
        Some(c) if *c != requested => d.info.push(InfoRow::new("Capacity", format!("{c} (resize to {requested} in progress or pending)"))),
        Some(c) => d.info.push(InfoRow::new("Capacity", c.clone())),
        None => {}
    }
    d.info.push(InfoRow::new("Access", access_modes(&spec["accessModes"])));
    match str_at(spec, "storageClassName") {
        Some(c) if !c.is_empty() => d.info.push(InfoRow::new("Storage class", c.clone()).link(link("storage.k8s.io/StorageClass", None, &c))),
        Some(_) => d.info.push(InfoRow::new("Storage class", "none: binds only to a volume without a class")),
        None => d.info.push(InfoRow::new("Storage class", "the cluster's default")),
    }
    if let Some(pv) = str_at(spec, "volumeName").filter(|n| !n.is_empty()) {
        d.info.push(InfoRow::new("Volume", pv.clone()).link(link("core/PersistentVolume", None, &pv)));
    }
    if str_at(spec, "volumeMode").as_deref() == Some("Block") {
        d.info.push(InfoRow::new("Volume mode", "Block: a raw device, no filesystem"));
    }
    let source = if spec["dataSourceRef"].is_object() { &spec["dataSourceRef"] } else { &spec["dataSource"] };
    if let Some(kind) = str_at(source, "kind") {
        d.info.push(InfoRow::new("Created from", format!("{kind} {}", str_at(source, "name").unwrap_or_default())));
    }
    if let Some(class) = str_at(spec, "volumeAttributesClassName") {
        d.info.push(InfoRow::new("Attributes class", class));
    }
    if let Some(selector) = label_selector(&spec["selector"]) {
        d.info.push(InfoRow::new("Volume selector", selector).mono());
    }
    let resizing = d
        .conditions
        .iter()
        .any(|c| matches!(c.type_.as_str(), "Resizing" | "FileSystemResizePending") && c.status == "True");
    d.status = Some(if resizing && phase == "Bound" { "Resizing".into() } else { phase });
}

async fn claim_related(client: &Client, namespace: &str, name: &str, v: &Value, d: &mut ObjectDetails) -> Result<()> {
    let class_name = str_at(&v["spec"], "storageClassName");
    let pods = Api::<Pod>::namespaced(client.clone(), namespace);
    let classes = Api::<StorageClass>::all(client.clone());
    let all = ListParams::default();
    let class = async {
        match &class_name {
            Some(c) if !c.is_empty() => classes.get_opt(c).await.ok().flatten(),
            Some(_) => None,
            // No class named: the default one applies.
            None => classes.list(&ListParams::default()).await.ok().and_then(|l| {
                l.items.into_iter().find(|c| {
                    c.annotations().get("storageclass.kubernetes.io/is-default-class").map(String::as_str) == Some("true")
                })
            }),
        }
    };
    let (pods, class) = tokio::join!(pods.list(&all), class);
    let pods = pods?;

    // Expansion, and why a claim is still pending.
    if let Some(class) = &class {
        let expand = class.allow_volume_expansion == Some(true);
        d.info.push(InfoRow::new("Can expand", if expand { "yes" } else { "no: its storage class doesn't allow it" }));
        if d.status.as_deref() == Some("Pending") {
            let waiting = class.volume_binding_mode.as_deref() == Some("WaitForFirstConsumer");
            let why = if waiting {
                "its class creates the volume when a pod first uses the claim".to_owned()
            } else {
                format!("waiting for {} to create a volume (see events)", class.provisioner)
            };
            d.info.push(InfoRow::new("Why pending", why));
        }
    } else if class_name.as_deref().is_some_and(|c| !c.is_empty()) {
        d.info.push(InfoRow::new("Can expand", "no: its storage class doesn't exist"));
    } else {
        // Bound by hand to a volume, or no default class to fall back on.
        if class_name.is_none()
            && let Some(row) = d.info.iter_mut().find(|r| r.key == "Storage class")
        {
            row.value = "none (the cluster has no default class)".into();
        }
        d.info.push(InfoRow::new("Can expand", "no: only claims provisioned by a storage class can grow"));
    }

    // The pods mounting it.
    let users: Vec<&Pod> = pods
        .items
        .iter()
        .filter(|p| {
            p.spec.iter().flat_map(|s| s.volumes.iter().flatten()).any(|vol| {
                vol.persistent_volume_claim.as_ref().is_some_and(|c| c.claim_name == name)
                    // Generic ephemeral volumes make a claim named <pod>-<volume>.
                    || (vol.ephemeral.is_some() && format!("{}-{}", p.name_any(), vol.name) == name)
            })
        })
        .collect();
    let rows: Vec<InfoTableRow> = users
        .iter()
        .map(|p| {
            let data = serde_json::to_value(p).unwrap_or_default();
            let status = crate::table::pod_status(&data, p.metadata.deletion_timestamp.is_some());
            let node = p.spec.as_ref().and_then(|s| s.node_name.clone()).unwrap_or_else(|| "—".into());
            let mut row = InfoTableRow::new(vec![p.name_any(), status, node]);
            row.link = Some(link("core/Pod", Some(namespace), &p.name_any()));
            row
        })
        .collect();
    let table = InfoTable { columns: vec!["Pod".into(), "Status".into(), "Node".into()], rows, ..Default::default() };
    let mut section = table_section("mounted-by", "Mounted by", table, "No pods use it right now.");
    if section.table.as_ref().is_some_and(|t| !t.rows.is_empty()) {
        section.summary = plural(users.len(), "pod");
    }

    // Disk usage, from the kubelet of a node it's mounted on.
    let nodes: Vec<String> = users
        .iter()
        .filter(|p| p.status.as_ref().and_then(|s| s.phase.as_deref()) == Some("Running"))
        .filter_map(|p| p.spec.as_ref()?.node_name.clone())
        .collect();
    let mut usage = None;
    for node in nodes.iter().take(2) {
        if let Some(summary) = crate::usage::kubelet_summary(client, node).await
            && let Some(u) = volume_usage(&summary, namespace, name)
        {
            usage = Some(u);
            break;
        }
    }
    match usage {
        Some(u) => d.sections.insert(0, u),
        None if !nodes.is_empty() => {
            d.info.push(InfoRow::new("Disk usage", "unknown: the kubelet didn't report it (e.g. on Fargate)"));
        }
        None => {}
    }
    d.sections.push(section);
    Ok(())
}

/// Used, free and size of the claim's filesystem from a kubelet summary.
fn volume_usage(summary: &Value, namespace: &str, name: &str) -> Option<crate::types::InfoSection> {
    let vol = summary["pods"].as_array()?.iter().flat_map(|p| p["volume"].as_array().into_iter().flatten()).find(|vol| {
        str_at(&vol["pvcRef"], "name").as_deref() == Some(name) && str_at(&vol["pvcRef"], "namespace").as_deref() == Some(namespace)
    })?;
    let num = |k: &str| vol[k].as_f64();
    let (used, cap) = (num("usedBytes")?, num("capacityBytes")?);
    let free = num("availableBytes").unwrap_or(cap - used);
    let fill = (cap > 0.0).then(|| used / cap);
    let mut rows = vec![{
        let mut r = InfoTableRow::new(vec!["Space".into(), bytes(used), bytes(free), bytes(cap)]);
        r.fill = fill.map(|f| f.min(1.0));
        r.warn = fill.is_some_and(|f| f >= 0.85);
        r
    }];
    if let (Some(iu), Some(ic)) = (num("inodesUsed"), num("inodes")) {
        let f = (ic > 0.0).then(|| iu / ic);
        let mut r = InfoTableRow::new(vec![
            "Files (inodes)".into(),
            format!("{iu:.0}"),
            num("inodesFree").map_or_else(|| "—".into(), |x| format!("{x:.0}")),
            format!("{ic:.0}"),
        ]);
        r.fill = f.map(|f| f.min(1.0));
        r.warn = f.is_some_and(|f| f >= 0.85);
        rows.push(r);
    }
    let summary = fill.map(|f| format!("{:.0}% used", f * 100.0)).unwrap_or_default();
    let mut section = table_section(
        "disk",
        "Disk usage",
        InfoTable { columns: ["", "Used", "Free", "Size"].map(String::from).to_vec(), numeric: vec![1, 2, 3], rows, ..Default::default() },
        "",
    );
    section.summary = summary;
    Some(section)
}

// ---- PersistentVolume ----

/// The volume plugin and its identifying fields.
fn volume_source(spec: &Value) -> Option<(String, String)> {
    if spec["csi"].is_object() {
        let c = &spec["csi"];
        return Some((
            format!("CSI {}", str_at(c, "driver").unwrap_or_default()),
            str_at(c, "volumeHandle").unwrap_or_default(),
        ));
    }
    let known = [
        ("hostPath", "path"),
        ("local", "path"),
        ("nfs", "path"),
        ("awsElasticBlockStore", "volumeID"),
        ("gcePersistentDisk", "pdName"),
        ("azureDisk", "diskName"),
        ("azureFile", "shareName"),
        ("iscsi", "iqn"),
        ("fc", "lun"),
        ("cephfs", "path"),
        ("rbd", "image"),
    ];
    known.iter().find(|(k, _)| spec[*k].is_object()).map(|(k, field)| {
        let s = &spec[*k];
        let mut id = str_at(s, field).unwrap_or_default();
        if *k == "nfs" {
            id = format!("{}:{id}", str_at(s, "server").unwrap_or_default());
        }
        ((*k).to_owned(), id)
    })
}

fn volume(d: &mut ObjectDetails, v: &Value) {
    let (spec, status) = (&v["spec"], &v["status"]);
    let phase = str_at(status, "phase").unwrap_or_else(|| "Pending".into());
    d.info.push(InfoRow::new("Capacity", str_at(&spec["capacity"], "storage").unwrap_or_default()));
    d.info.push(InfoRow::new("Access", access_modes(&spec["accessModes"])));
    let policy = str_at(spec, "persistentVolumeReclaimPolicy").unwrap_or_else(|| "Retain".into());
    d.info.push(InfoRow::new("Reclaim policy", reclaim_text(&policy)));
    if let Some(c) = str_at(spec, "storageClassName").filter(|c| !c.is_empty()) {
        d.info.push(InfoRow::new("Storage class", c.clone()).link(link("storage.k8s.io/StorageClass", None, &c)));
    }
    let claim = &spec["claimRef"];
    if let (Some(ns), Some(name)) = (str_at(claim, "namespace"), str_at(claim, "name")) {
        let note = if phase == "Released" { " (deleted; the volume is kept)" } else { "" };
        d.info.push(InfoRow::new("Claim", format!("{ns}/{name}{note}")).link(link("core/PersistentVolumeClaim", Some(&ns), &name)));
    }
    if let Some((kind, id)) = volume_source(spec) {
        d.info.push(InfoRow::new("Source", kind));
        if !id.is_empty() {
            d.info.push(InfoRow::new("Volume ID", id).mono());
        }
    }
    if str_at(spec, "volumeMode").as_deref() == Some("Block") {
        d.info.push(InfoRow::new("Volume mode", "Block: a raw device, no filesystem"));
    }
    // Where it can attach, e.g. one zone.
    for term in spec["nodeAffinity"]["required"]["nodeSelectorTerms"].as_array().into_iter().flatten() {
        let parts: Vec<String> = term["matchExpressions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| {
                format!(
                    "{} {} ({})",
                    str_at(e, "key").unwrap_or_default(),
                    str_at(e, "operator").unwrap_or_default().to_lowercase(),
                    strings(&e["values"]).join(", ")
                )
            })
            .collect();
        if !parts.is_empty() {
            d.info.push(InfoRow::new("Nodes", parts.join(" and ")).mono());
        }
    }
    let options = strings(&spec["mountOptions"]);
    if !options.is_empty() {
        d.info.push(InfoRow::new("Mount options", options.join(", ")).mono());
    }
    if let Some(t) = str_at(status, "lastPhaseTransitionTime") {
        d.info.push(InfoRow::new("Phase since", t).time());
    }
    if let Some(m) = str_at(status, "message") {
        d.info.push(InfoRow::new("Message", m));
    }
    d.status = Some(phase);
}

// ---- StorageClass ----

fn storage_class(d: &mut ObjectDetails, v: &Value) {
    d.info.push(InfoRow::new("Provisioner", str_at(v, "provisioner").unwrap_or_default()).mono());
    let default = str_at(&v["metadata"]["annotations"], "storageclass.kubernetes.io/is-default-class").as_deref() == Some("true");
    d.info.push(InfoRow::new("Default class", if default { "yes: claims without a class use it" } else { "no" }));
    d.info.push(InfoRow::new("Reclaim policy", reclaim_text(&str_at(v, "reclaimPolicy").unwrap_or_else(|| "Delete".into()))));
    d.info.push(InfoRow::new("Binding", binding_text(&str_at(v, "volumeBindingMode").unwrap_or_default())));
    d.info.push(InfoRow::new("Expansion", if v["allowVolumeExpansion"] == true { "allowed" } else { "not allowed" }));
    let options = strings(&v["mountOptions"]);
    if !options.is_empty() {
        d.info.push(InfoRow::new("Mount options", options.join(", ")).mono());
    }
    for t in v["allowedTopologies"].as_array().into_iter().flatten() {
        for e in t["matchLabelExpressions"].as_array().into_iter().flatten() {
            d.info.push(InfoRow::new(
                "Allowed topology",
                format!("{} in ({})", str_at(e, "key").unwrap_or_default(), strings(&e["values"]).join(", ")),
            ).mono());
        }
    }
    let rows: Vec<InfoTableRow> = v["parameters"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, val)| InfoTableRow::new(vec![k.clone(), val.as_str().unwrap_or_default().to_owned()]))
        .collect();
    if !rows.is_empty() {
        let table = InfoTable { columns: vec!["Parameter".into(), "Value".into()], mono: vec![0, 1], rows, ..Default::default() };
        d.sections.push(table_section("parameters", "Parameters", table, ""));
    }
    if default {
        d.status = Some("Default".into());
    }
}

async fn class_claims(client: &Client, name: &str, d: &mut ObjectDetails) -> Result<()> {
    let default = d.status.as_deref() == Some("Default");
    let claims = Api::<PersistentVolumeClaim>::all(client.clone()).list(&ListParams::default()).await?;
    let mut rows: Vec<InfoTableRow> = claims
        .items
        .iter()
        .filter(|c| {
            let class = c.spec.as_ref().and_then(|s| s.storage_class_name.clone());
            class.as_deref() == Some(name) || (class.is_none() && default)
        })
        .map(|c| {
            let ns = c.namespace().unwrap_or_default();
            let phase = c.status.as_ref().and_then(|s| s.phase.clone()).unwrap_or_default();
            let size = c
                .status
                .as_ref()
                .and_then(|s| s.capacity.as_ref())
                .and_then(|m| m.get("storage"))
                .map(|q| q.0.clone())
                .unwrap_or_default();
            let mut row = InfoTableRow::new(vec![ns.clone(), c.name_any(), phase.clone(), size]);
            row.link = Some(link("core/PersistentVolumeClaim", Some(&ns), &c.name_any()));
            row.warn = phase != "Bound";
            row
        })
        .collect();
    rows.sort_by(|a, b| a.cells.cmp(&b.cells));
    let table = InfoTable {
        columns: ["Namespace", "Claim", "Status", "Size"].map(String::from).to_vec(),
        numeric: vec![3],
        rows,
        ..Default::default()
    };
    d.sections.push(table_section("class-claims", "Claims", table, "No claims use it."));
    Ok(())
}

// ---- VolumeAttachment and CSI ----

fn attachment(d: &mut ObjectDetails, v: &Value) {
    let (spec, status) = (&v["spec"], &v["status"]);
    d.info.push(InfoRow::new("Attacher", str_at(spec, "attacher").unwrap_or_default()).mono());
    if let Some(node) = str_at(spec, "nodeName") {
        d.info.push(InfoRow::new("Node", node.clone()).link(link("core/Node", None, &node)));
    }
    if let Some(pv) = str_at(&spec["source"], "persistentVolumeName") {
        d.info.push(InfoRow::new("Volume", pv.clone()).link(link("core/PersistentVolume", None, &pv)));
    }
    let attached = status["attached"] == true;
    d.info.push(InfoRow::new("Attached", if attached { "yes" } else { "no" }));
    let mut failed = false;
    for (key, label) in [("attachError", "Attach error"), ("detachError", "Detach error")] {
        if let Some(m) = str_at(&status[key], "message") {
            failed = true;
            d.info.push(InfoRow::new(label, m));
            if let Some(t) = str_at(&status[key], "time") {
                d.info.push(InfoRow::new(format!("{label} at"), t).time());
            }
        }
    }
    let deleting = v["metadata"]["deletionTimestamp"].is_string();
    d.status = Some(
        if failed {
            "Error"
        } else if deleting {
            "Detaching"
        } else if attached {
            "Attached"
        } else {
            "Attaching"
        }
        .into(),
    );
}

fn csi_driver(d: &mut ObjectDetails, v: &Value) {
    let spec = &v["spec"];
    let yes_no = |key: &str, default: bool| if spec[key].as_bool().unwrap_or(default) { "yes" } else { "no" };
    d.info.push(InfoRow::new("Needs attaching", yes_no("attachRequired", true)));
    d.info.push(InfoRow::new("Pod info on mount", yes_no("podInfoOnMount", false)));
    d.info.push(InfoRow::new("Reports capacity", yes_no("storageCapacity", false)));
    let modes = strings(&spec["volumeLifecycleModes"]);
    d.info.push(InfoRow::new("Volume modes", if modes.is_empty() { "Persistent".into() } else { modes.join(", ") }));
    if let Some(p) = str_at(spec, "fsGroupPolicy") {
        d.info.push(InfoRow::new("fsGroup policy", p));
    }
    if spec["seLinuxMount"] == true {
        d.info.push(InfoRow::new("SELinux mount", "yes"));
    }
    if spec["requiresRepublish"] == true {
        d.info.push(InfoRow::new("Republishes", "yes: volumes are re-mounted periodically"));
    }
    let tokens: Vec<String> = spec["tokenRequests"].as_array().into_iter().flatten().filter_map(|t| str_at(t, "audience")).collect();
    if !tokens.is_empty() {
        d.info.push(InfoRow::new("Token audiences", tokens.join(", ")));
    }
}

async fn driver_nodes(client: &Client, name: &str, d: &mut ObjectDetails) -> Result<()> {
    let nodes = Api::<CSINode>::all(client.clone()).list(&ListParams::default()).await?;
    let rows: Vec<InfoTableRow> = nodes
        .items
        .iter()
        .filter_map(|n| {
            let driver = n.spec.drivers.iter().find(|x| x.name == name)?;
            let count = driver.allocatable.as_ref().and_then(|a| a.count).map_or_else(|| "—".into(), |c| c.to_string());
            let mut row = InfoTableRow::new(vec![n.name_any(), driver.node_id.clone(), count]);
            row.link = Some(link("core/Node", None, &n.name_any()));
            Some(row)
        })
        .collect();
    let table = InfoTable {
        times: Vec::new(),
        columns: vec!["Node".into(), "Node ID".into(), "Max volumes".into()],
        numeric: vec![2],
        mono: vec![1],
        rows,
    };
    let mut section = table_section("driver-nodes", "Nodes", table, "No nodes run it.");
    if let Some(n) = section.table.as_ref().map(|t| t.rows.len()).filter(|n| *n > 0) {
        section.summary = plural(n, "node");
    }
    d.sections.push(section);
    Ok(())
}

fn csi_node(d: &mut ObjectDetails, v: &Value) {
    d.info.push(InfoRow::new("Node", d.name.clone()).link(link("core/Node", None, &d.name)));
    let rows: Vec<InfoTableRow> = v["spec"]["drivers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            let name = str_at(x, "name").unwrap_or_default();
            let mut row = InfoTableRow::new(vec![
                name.clone(),
                str_at(x, "nodeID").unwrap_or_default(),
                int_at(&x["allocatable"], "count").map_or_else(|| "—".into(), |c| c.to_string()),
                strings(&x["topologyKeys"]).join(", "),
            ]);
            row.link = Some(link("storage.k8s.io/CSIDriver", None, &name));
            row
        })
        .collect();
    let table = InfoTable {
        times: Vec::new(),
        columns: ["Driver", "Node ID", "Max volumes", "Topology keys"].map(String::from).to_vec(),
        numeric: vec![2],
        mono: vec![1],
        rows,
    };
    d.sections.push(table_section("drivers", "Drivers", table, "No CSI drivers on this node."));
}

fn capacity(d: &mut ObjectDetails, v: &Value) {
    if let Some(c) = str_at(v, "storageClassName") {
        d.info.push(InfoRow::new("Storage class", c.clone()).link(link("storage.k8s.io/StorageClass", None, &c)));
    }
    d.info.push(InfoRow::new("Capacity", str_at(v, "capacity").unwrap_or_else(|| "unknown".into())));
    if let Some(m) = str_at(v, "maximumVolumeSize") {
        d.info.push(InfoRow::new("Largest volume", m));
    }
    if let Some(s) = label_selector(&v["nodeTopology"]) {
        d.info.push(InfoRow::new("Nodes", s).mono());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::describe;
    use serde_json::json;

    fn now() -> jiff::Timestamp {
        "2026-10-10T12:00:00Z".parse().unwrap()
    }

    fn row<'a>(d: &'a ObjectDetails, key: &str) -> Option<&'a str> {
        d.info.iter().find(|r| r.key == key).map(|r| r.value.as_str())
    }

    #[test]
    fn claims() {
        let v = json!({"kind": "PersistentVolumeClaim", "metadata": {"name": "data", "namespace": "shop"},
            "spec": {"accessModes": ["ReadWriteOnce"], "resources": {"requests": {"storage": "20Gi"}},
                     "storageClassName": "gp3", "volumeName": "pvc-123"},
            "status": {"phase": "Bound", "capacity": {"storage": "10Gi"},
                       "conditions": [{"type": "FileSystemResizePending", "status": "True"}]}});
        let d = describe("core/PersistentVolumeClaim", &v, now());
        assert_eq!(d.status.as_deref(), Some("Resizing"));
        assert_eq!(row(&d, "Capacity"), Some("10Gi (resize to 20Gi in progress or pending)"));
        assert_eq!(row(&d, "Access"), Some("ReadWriteOnce (one node)"));
        assert_eq!(d.info.iter().find(|r| r.key == "Volume").unwrap().link.as_ref().unwrap().kind, "core/PersistentVolume");

        let pending = json!({"kind": "PersistentVolumeClaim", "metadata": {"name": "x", "namespace": "shop"}, "spec": {}, "status": {}});
        let d = describe("core/PersistentVolumeClaim", &pending, now());
        assert_eq!((d.status.as_deref(), row(&d, "Storage class")), (Some("Pending"), Some("the cluster's default")));
    }

    #[test]
    fn kubelet_volume_usage() {
        let summary = json!({"pods": [{"volume": [
            {"name": "tmp", "usedBytes": 10},
            {"name": "data", "pvcRef": {"name": "data", "namespace": "shop"},
             "usedBytes": 9.0e9, "capacityBytes": 10.0e9, "availableBytes": 1.0e9,
             "inodes": 1000, "inodesUsed": 100, "inodesFree": 900}]}]});
        let s = volume_usage(&summary, "shop", "data").unwrap();
        assert_eq!(s.summary, "90% used");
        let rows = &s.table.unwrap().rows;
        assert_eq!(rows[0].cells, ["Space", "8.4 GiB", "953.7 MiB", "9.3 GiB"]);
        assert!(rows[0].warn);
        assert_eq!((rows[1].cells[1].as_str(), rows[1].warn), ("100", false));
        assert!(volume_usage(&summary, "other", "data").is_none());
    }

    #[test]
    fn volumes_and_classes() {
        let v = json!({"kind": "PersistentVolume", "metadata": {"name": "pvc-123"},
            "spec": {"capacity": {"storage": "10Gi"}, "accessModes": ["ReadWriteOnce"], "persistentVolumeReclaimPolicy": "Delete",
                     "storageClassName": "gp3", "claimRef": {"namespace": "shop", "name": "data"},
                     "csi": {"driver": "ebs.csi.aws.com", "volumeHandle": "vol-0abc"},
                     "nodeAffinity": {"required": {"nodeSelectorTerms": [{"matchExpressions": [
                        {"key": "topology.kubernetes.io/zone", "operator": "In", "values": ["us-east-1a"]}]}]}}},
            "status": {"phase": "Released"}});
        let d = describe("core/PersistentVolume", &v, now());
        assert_eq!(d.status.as_deref(), Some("Released"));
        assert!(row(&d, "Reclaim policy").unwrap().starts_with("Delete: deleting the claim deletes"));
        assert_eq!(row(&d, "Claim"), Some("shop/data (deleted; the volume is kept)"));
        assert_eq!((row(&d, "Source"), row(&d, "Volume ID")), (Some("CSI ebs.csi.aws.com"), Some("vol-0abc")));
        assert_eq!(row(&d, "Nodes"), Some("topology.kubernetes.io/zone in (us-east-1a)"));

        let sc = json!({"kind": "StorageClass", "metadata": {"name": "gp3", "annotations": {"storageclass.kubernetes.io/is-default-class": "true"}},
            "provisioner": "ebs.csi.aws.com", "volumeBindingMode": "WaitForFirstConsumer", "allowVolumeExpansion": true,
            "parameters": {"type": "gp3", "encrypted": "true"}});
        let d = describe("storage.k8s.io/StorageClass", &sc, now());
        assert_eq!(d.status.as_deref(), Some("Default"));
        assert!(row(&d, "Binding").unwrap().starts_with("WaitForFirstConsumer"));
        assert_eq!(row(&d, "Reclaim policy").map(|r| r.starts_with("Delete")), Some(true));
        assert_eq!(d.sections[0].table.as_ref().unwrap().rows.len(), 2);

        let va = json!({"kind": "VolumeAttachment", "metadata": {"name": "csi-1"},
            "spec": {"attacher": "ebs.csi.aws.com", "nodeName": "n1", "source": {"persistentVolumeName": "pvc-123"}},
            "status": {"attached": false, "attachError": {"message": "rpc error: volume in use", "time": "2026-10-10T11:59:00Z"}}});
        let d = describe("storage.k8s.io/VolumeAttachment", &va, now());
        assert_eq!(d.status.as_deref(), Some("Error"));
        assert_eq!(row(&d, "Attach error"), Some("rpc error: volume in use"));
    }
}
