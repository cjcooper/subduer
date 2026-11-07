//! The Details tab of KEDA kinds (`keda.sh`, `eventing.keda.sh`):
//! ScaledObjects and ScaledJobs (what they scale, between what bounds, on
//! which triggers, and what those triggers read now through the HPA KEDA
//! made), TriggerAuthentications (where each parameter comes from, and who
//! uses it) and CloudEventSources. Built on `config`'s generic shape, with
//! the operator's controller, whose logs explain failures. Also Pause/Resume
//! (KEDA's pause annotations) and the replica range of ScaledObjects and
//! ScaledJobs.
//!
//! Nothing here reads secret values: authentications name Secrets, env vars
//! and vault paths, a Vault token written into the object is only said to be
//! there, and trigger settings whose names say they hold a secret
//! (`password`, `connectionString`, …) are shown without their value.

use k8s_openapi::api::batch::v1::Job;
use kube::Client;
use kube::api::{Api, ListParams, Patch, PatchParams};
use serde_json::{Value, json};

use crate::cluster::dynamic_api;
use crate::config::{int_at, metric_body, metric_value, plural, str_at, table_section};
use crate::external_secrets::{controller, drop_empty_columns, link, list_or_note, strings};
use crate::safety::SafetyGate;
use crate::types::{InfoRow, InfoSection, InfoTable, InfoTableRow, KindRef, ObjectDetails, ObjectLink, is_keda_scaler};
use crate::{Error, Result};

pub(crate) const GROUP: &str = "keda.sh";
const VERSION: &str = "v1alpha1";
/// The operator Deployment's label in the official Helm chart (its metrics
/// server and webhooks have their own names).
const CONTROLLER_LABEL: &str = "app.kubernetes.io/name=keda-operator";
/// Stops scaling where it is (ScaledObjects and ScaledJobs).
const PAUSED: &str = "autoscaling.keda.sh/paused";
/// Scales a ScaledObject to this many replicas and stops there.
const PAUSED_REPLICAS: &str = "autoscaling.keda.sh/paused-replicas";
/// The label on the Jobs a ScaledJob makes.
const JOB_LABEL: &str = "scaledjob.keda.sh/name";

/// Fills in what comes from the object itself.
pub(crate) fn describe(key: &str, d: &mut ObjectDetails, v: &Value) {
    match key {
        "keda.sh/ScaledObject" => scaled_object(d, v),
        "keda.sh/ScaledJob" => scaled_job(d, v),
        "keda.sh/TriggerAuthentication" | "keda.sh/ClusterTriggerAuthentication" => authentication(d, v),
        "eventing.keda.sh/CloudEventSource" | "eventing.keda.sh/ClusterCloudEventSource" => cloud_event_source(d, v),
        k if k.starts_with("keda.sh/") || k.starts_with("eventing.keda.sh/") => {
            d.status = Some(match condition(v, "Ready").and_then(|c| str_at(c, "status")).as_deref() {
                Some("True") => "Ready".into(),
                Some("False") => "Not ready".into(),
                _ => "Pending".into(),
            });
        }
        _ => {}
    }
}

/// Adds what needs other objects: a ScaledObject's HPA (replicas, and each
/// trigger's value now), a ScaledJob's Jobs, who uses an authentication, the
/// controller (for Logs) and, for cluster-scoped kinds, events from any
/// namespace.
pub(crate) async fn related(
    client: &Client,
    kind: &KindRef,
    namespace: &str,
    name: &str,
    v: &Value,
    d: &mut ObjectDetails,
) -> Result<()> {
    match kind.key().as_str() {
        "keda.sh/ScaledObject" => scaled_object_hpa(client, namespace, name, v, d).await,
        "keda.sh/ScaledJob" => {
            let lp = ListParams::default().labels(&format!("{JOB_LABEL}={name}"));
            match Api::<Job>::namespaced(client.clone(), namespace).list(&lp).await {
                Ok(list) => d.sections.push(jobs_section(&list.items)),
                Err(e) => d.info.push(InfoRow::new("Note", format!("couldn't list Jobs: {}", Error::from(e).user_message()))),
            }
        }
        "keda.sh/TriggerAuthentication" | "keda.sh/ClusterTriggerAuthentication" => {
            let ns = kind.namespaced.then_some(namespace);
            let mut items = list_or_note(list(client, "ScaledObject", "scaledobjects", ns).await, "ScaledObjects", d);
            items.extend(list_or_note(list(client, "ScaledJob", "scaledjobs", ns).await, "ScaledJobs", d));
            d.sections.push(auth_users(&items, &kind.kind, name));
        }
        _ => {}
    }
    let keda_namespace = controller(client, CONTROLLER_LABEL, name, d).await;
    if kind.kind == "ClusterTriggerAuthentication" {
        cluster_auth_links(d, keda_namespace.as_deref());
    }
    if !kind.namespaced {
        let fields = format!("involvedObject.kind={},involvedObject.name={name}", kind.kind);
        d.events = crate::pod_details::list_events(client, "", Some(&fields), |_| Some(None)).await;
    }
    Ok(())
}

// ---- ScaledObject ----

fn scaled_object(d: &mut ObjectDetails, v: &Value) {
    let (spec, status) = (&v["spec"], &v["status"]);
    d.status = Some(scaling_status(v));
    let target = &spec["scaleTargetRef"];
    if let Some(name) = str_at(target, "name") {
        let kind = str_at(target, "kind").unwrap_or_else(|| "Deployment".into());
        let group = str_at(target, "apiVersion")
            .map_or_else(|| "apps".to_owned(), |a| a.split_once('/').map_or("core", |(g, _)| g).to_owned());
        let row = InfoRow::new("Scales", format!("{kind} {name}"));
        d.info.push(row.link(link(&format!("{group}/{kind}"), d.namespace.as_deref(), &name)));
    }
    let (min, max) = (int_at(spec, "minReplicaCount").unwrap_or(0), int_at(spec, "maxReplicaCount").unwrap_or(100));
    d.info.push(InfoRow::new("Range", format!("{min} to {max} replicas")));
    idle_row(d, spec, min);
    d.info.push(InfoRow::new("Polling", format!("every {}s", int_at(spec, "pollingInterval").unwrap_or(30))));
    paused_rows(d, v);
    let fallback = &spec["fallback"];
    if let (Some(failures), Some(replicas)) = (int_at(fallback, "failureThreshold"), int_at(fallback, "replicas")) {
        let mut text = format!("{} after {} failed reads in a row", plural(replicas as usize, "replica"), failures);
        if let Some(b) = str_at(fallback, "behavior").filter(|b| b != "static") {
            text.push_str(&format!(" ({b})"));
        }
        d.info.push(InfoRow::new("Fallback", text));
    }
    let advanced = &spec["advanced"];
    let modifiers = &advanced["scalingModifiers"];
    if let Some(formula) = str_at(modifiers, "formula") {
        let mut text = formula;
        if let Some(t) = str_at(modifiers, "target") {
            text.push_str(&format!(" · target {t}"));
        }
        if let Some(a) = str_at(modifiers, "activationTarget") {
            text.push_str(&format!(" · active above {a}"));
        }
        d.info.push(InfoRow::new("Formula", text).mono());
    }
    if advanced["horizontalPodAutoscalerConfig"]["behavior"].is_object() {
        d.info.push(InfoRow::new("HPA behavior", "custom (see the HPA)"));
    }
    let original = int_at(status, "originalReplicaCount");
    d.info.push(InfoRow::new(
        "When deleted",
        match (advanced["restoreToOriginalReplicaCount"] == true, original) {
            (true, Some(n)) => format!("puts back the {} it had before", plural(n as usize, "replica")),
            (true, None) => "puts back the replicas it had before".into(),
            (false, _) => "leaves the replicas as they are".into(),
        },
    ));
    if let Some(t) = str_at(status, "lastActiveTime") {
        d.info.push(InfoRow::new("Last active", t).time());
    }
    d.sections.extend(trigger_sections(v, None));
}

/// What happens when no trigger is active: KEDA itself scales to zero (or
/// the idle count) once the cooldown has passed; above that it's the HPA's.
fn idle_row(d: &mut ObjectDetails, spec: &Value, min: i64) {
    let idle = int_at(spec, "idleReplicaCount");
    if idle.is_none() && min > 0 {
        return;
    }
    let to = idle.unwrap_or(0);
    let mut text = format!("{} after {}s without activity", plural(to as usize, "replica"), int_at(spec, "cooldownPeriod").unwrap_or(300));
    if let Some(n) = int_at(spec, "initialCooldownPeriod").filter(|n| *n > 0) {
        text.push_str(&format!(" (not in its first {n}s)"));
    }
    d.info.push(InfoRow::new("When idle", text));
}

/// `Paused`, `Not ready`, `Fallback`, `Active` or `Idle`, from the
/// conditions and the pause annotations.
fn scaling_status(v: &Value) -> String {
    let is = |type_: &str, status: &str| condition(v, type_).is_some_and(|c| c["status"] == status);
    if is("Paused", "True") || paused(v).is_some() {
        "Paused"
    } else if is("Ready", "False") {
        "Not ready"
    } else if is("Fallback", "True") {
        "Fallback"
    } else if is("Active", "True") {
        "Active"
    } else if is("Active", "False") {
        "Idle"
    } else {
        "Pending"
    }
    .into()
}

fn condition<'a>(v: &'a Value, type_: &str) -> Option<&'a Value> {
    v["status"]["conditions"].as_array()?.iter().find(|c| c["type"] == type_)
}

/// How it's paused, from the annotations: `Some(Some(n))` at n replicas,
/// `Some(None)` where it is.
fn paused(v: &Value) -> Option<Option<String>> {
    let a = &v["metadata"]["annotations"];
    if let Some(n) = str_at(a, PAUSED_REPLICAS) {
        return Some(Some(n));
    }
    (str_at(a, PAUSED).as_deref() == Some("true")).then_some(None)
}

fn paused_rows(d: &mut ObjectDetails, v: &Value) {
    match paused(v) {
        Some(Some(n)) => d.info.push(InfoRow::new("Paused", format!("at {n} replicas ({PAUSED_REPLICAS})"))),
        Some(None) => d.info.push(InfoRow::new("Paused", format!("yes: scaling stopped where it is ({PAUSED})"))),
        None => {}
    }
}

/// The HPA KEDA made for it: replicas now, and each trigger's value now
/// against its target. Missing while paused, since KEDA removes it then.
async fn scaled_object_hpa(client: &Client, namespace: &str, name: &str, v: &Value, d: &mut ObjectDetails) {
    let hpa_name = str_at(&v["status"], "hpaName")
        .or_else(|| str_at(&v["spec"]["advanced"]["horizontalPodAutoscalerConfig"], "name"))
        .unwrap_or_else(|| format!("keda-hpa-{name}"));
    let kind = KindRef {
        group: "autoscaling".into(),
        version: "v2".into(),
        kind: "HorizontalPodAutoscaler".into(),
        plural: "horizontalpodautoscalers".into(),
        namespaced: true,
    };
    let at = d.info.iter().position(|r| r.key == "Scales").map_or(0, |i| i + 1);
    match dynamic_api(client, &kind, Some(namespace)).get(&hpa_name).await {
        Ok(object) => {
            let Ok(hpa) = serde_json::to_value(&object) else { return };
            let status = &hpa["status"];
            let (current, desired) = (int_at(status, "currentReplicas").unwrap_or(0), int_at(status, "desiredReplicas").unwrap_or(0));
            d.info.insert(at, InfoRow::new("Replicas", format!("{current} now · {desired} wanted")));
            let row = InfoRow::new("HPA", hpa_name.clone()).mono();
            d.info.insert(at + 1, row.link(link("autoscaling/HorizontalPodAutoscaler", Some(namespace), &hpa_name)));
            if let Some(s) = d.sections.iter_mut().find(|s| s.id == "keda-triggers") {
                *s = trigger_sections(v, Some(&hpa)).swap_remove(0);
            }
            if let Some(composite) = str_at(&v["status"], "compositeScalerName")
                && let Some(row) = d.info.iter_mut().find(|r| r.key == "Formula")
            {
                let (now, _) = hpa_metric(&hpa, "External", &composite);
                if !now.is_empty() {
                    row.value.push_str(&format!(" · now {now}"));
                }
            }
        }
        Err(kube::Error::Api(s)) if s.code == 404 => {
            let why = if paused(v).is_some() { "none while paused" } else { "missing" };
            d.info.insert(at, InfoRow::new("HPA", format!("{hpa_name} ({why})")).mono());
        }
        Err(e) => d.info.push(InfoRow::new("Note", format!("couldn't read the HPA: {}", Error::from(e).user_message()))),
    }
}

// ---- Triggers ----

/// One row per trigger. With the HPA, its value now and target; with
/// KEDA's health record, whether reading it has been failing. Each trigger's
/// settings get a section of their own after it, since queries and addresses
/// are too long for a table cell.
fn trigger_sections(v: &Value, hpa: Option<&Value>) -> Vec<InfoSection> {
    let (spec, status) = (&v["spec"], &v["status"]);
    let namespace = str_at(&v["metadata"], "namespace");
    let external = strings(&status["externalMetricNames"]);
    let rows = spec["triggers"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, t)| {
            let type_ = str_at(t, "type").unwrap_or_default();
            let resource = matches!(type_.as_str(), "cpu" | "memory");
            // KEDA names a trigger's metric `s<index>-<type>…`; cpu and memory
            // are the HPA's own resource metrics.
            let metric = if resource {
                type_.clone()
            } else {
                let prefix = format!("s{i}-");
                external.iter().find(|n| n.starts_with(&prefix)).cloned().unwrap_or_default()
            };
            let (now, target) = match hpa {
                Some(h) if !metric.is_empty() => hpa_metric(h, if resource { "Resource" } else { "External" }, &metric),
                _ => Default::default(),
            };
            let health = &status["health"][&metric];
            let failing = health["status"] == "Failing";
            let health_text = match str_at(health, "status").as_deref() {
                Some("Happy") => "OK".into(),
                Some("Failing") => format!("failing ({} in a row)", int_at(health, "numberOfFailures").unwrap_or(0)),
                Some(other) => other.into(),
                None => String::new(),
            };
            let auth = &t["authenticationRef"];
            let auth_ref = str_at(auth, "name").map(|n| (str_at(auth, "kind").unwrap_or_else(|| "TriggerAuthentication".into()), n));
            let mut row = InfoTableRow::new(vec![
                type_,
                str_at(t, "name").unwrap_or_default(),
                now,
                target,
                health_text,
                auth_ref.as_ref().map(|(k, n)| format!("{k}/{n}")).unwrap_or_default(),
            ]);
            row.link = auth_ref.map(|(k, n)| {
                let ns = if k == "ClusterTriggerAuthentication" { None } else { namespace.as_deref() };
                link(&format!("{GROUP}/{k}"), ns, &n)
            });
            row.warn = failing;
            row
        })
        .collect();
    let table = InfoTable {
        columns: ["Trigger", "Name", "Now", "Target", "Health", "Authentication"].map(String::from).to_vec(),
        rows,
        ..Default::default()
    };
    let mut sections = vec![table_section("keda-triggers", "Triggers", drop_empty_columns(table, &[1, 2, 3, 4, 5]), "No triggers: it won't scale.")];
    for (i, t) in spec["triggers"].as_array().into_iter().flatten().enumerate() {
        let rows = settings(t);
        if rows.is_empty() {
            continue;
        }
        let type_ = str_at(t, "type").unwrap_or_default();
        let title = match str_at(t, "name") {
            Some(name) => format!("Trigger: {type_} ({name})"),
            None => format!("Trigger: {type_}"),
        };
        sections.push(InfoSection {
            id: format!("keda-trigger-{i}"),
            title,
            summary: plural(rows.len(), "setting"),
            rows,
            table: None,
            empty: None,
        });
    }
    sections
}

/// A trigger's metadata, one row per key, with values left out where the key
/// says it holds a secret (`…FromEnv` names an env var, so it's shown) and
/// passwords cut out of addresses.
fn settings(trigger: &Value) -> Vec<InfoRow> {
    let mut rows: Vec<InfoRow> = trigger["metadata"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| {
            let value = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
            if is_sensitive(k) { InfoRow::new(k, "(not shown)") } else { InfoRow::new(k, without_password(&value)).mono() }
        })
        .collect();
    if let Some(m) = str_at(trigger, "metricType") {
        rows.push(InfoRow::new("metricType", m).mono());
    }
    if trigger["useCachedMetrics"] == true {
        rows.push(InfoRow::new("useCachedMetrics", "true").mono());
    }
    rows
}

fn is_sensitive(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    !k.ends_with("fromenv")
        && ["password", "secret", "token", "connection", "apikey", "accesskey", "credential"]
            .iter()
            .any(|s| k.contains(s))
}

/// An address with its password replaced: `amqp://user:(not shown)@mq`.
fn without_password(value: &str) -> String {
    let Some(scheme) = value.find("://").map(|i| i + 3) else { return value.to_owned() };
    let authority = value[scheme..].find('/').map_or(value.len(), |i| scheme + i);
    let Some(at) = value[scheme..authority].rfind('@').map(|i| scheme + i) else { return value.to_owned() };
    match value[scheme..at].find(':').map(|i| scheme + i) {
        Some(colon) => format!("{}(not shown){}", &value[..=colon], &value[at..]),
        None => value.to_owned(),
    }
}

/// (value now, target) of the HPA metric `name`: a `Resource` (or
/// `ContainerResource`) by resource name, or an `External` metric by its
/// metric name.
fn hpa_metric(hpa: &Value, type_: &str, name: &str) -> (String, String) {
    let types: &[&str] = if type_ == "Resource" { &["Resource", "ContainerResource"] } else { &["External"] };
    let find = |list: &Value| -> Option<(Value, String)> {
        list.as_array().into_iter().flatten().find_map(|m| {
            let t = str_at(m, "type")?;
            if !types.contains(&t.as_str()) {
                return None;
            }
            let body = metric_body(m, &t);
            let id = if t == "External" { str_at(&body["metric"], "name") } else { str_at(body, "name") };
            (id.as_deref() == Some(name)).then(|| (body.clone(), t))
        })
    };
    let target = find(&hpa["spec"]["metrics"]).map(|(body, _)| target_text(&body["target"])).unwrap_or_default();
    let now = find(&hpa["status"]["currentMetrics"]).map(|(body, _)| metric_value(&body["current"])).unwrap_or_default();
    (now, target)
}

fn target_text(target: &Value) -> String {
    match str_at(target, "type").as_deref() {
        Some("Utilization") => format!("{}% of request", int_at(target, "averageUtilization").unwrap_or(0)),
        Some("AverageValue") => format!("{} average", str_at(target, "averageValue").unwrap_or_default()),
        Some("Value") => str_at(target, "value").unwrap_or_default(),
        _ => String::new(),
    }
}

// ---- ScaledJob ----

fn scaled_job(d: &mut ObjectDetails, v: &Value) {
    let (spec, status) = (&v["spec"], &v["status"]);
    d.status = Some(scaling_status(v));
    let job = &spec["jobTargetRef"];
    let containers: Vec<String> = job["template"]["spec"]["containers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| format!("{} ({})", str_at(c, "name").unwrap_or_default(), str_at(c, "image").unwrap_or_default()))
        .collect();
    if !containers.is_empty() {
        d.info.push(InfoRow::new("Runs", containers.join(", ")).mono());
    }
    let mut each = Vec::new();
    for (key, label) in [("parallelism", "parallelism"), ("completions", "completions"), ("backoffLimit", "retries")] {
        if let Some(n) = int_at(job, key) {
            each.push(format!("{label} {n}"));
        }
    }
    if let Some(s) = int_at(job, "activeDeadlineSeconds") {
        each.push(format!("deadline {s}s"));
    }
    if !each.is_empty() {
        d.info.push(InfoRow::new("Each job", each.join(" · ")));
    }
    let (min, max) = (int_at(spec, "minReplicaCount").unwrap_or(0), int_at(spec, "maxReplicaCount").unwrap_or(100));
    d.info.push(InfoRow::new("Jobs at once", format!("{min} to {max}")));
    d.info.push(InfoRow::new("Polling", format!("every {}s", int_at(spec, "pollingInterval").unwrap_or(30))));
    let scaling = &spec["scalingStrategy"];
    let strategy = str_at(scaling, "strategy").unwrap_or_else(|| "default".into());
    d.info.push(InfoRow::new(
        "Strategy",
        match strategy.as_str() {
            "default" => "default: starts as many jobs as there are waiting items, less those running".into(),
            "accurate" => "accurate: for queues that drop items once they're taken, so running jobs aren't subtracted twice".into(),
            "eager" => "eager: starts up to the maximum as soon as there's work".into(),
            "custom" => format!(
                "custom: waiting items less {} and {}% of running jobs",
                str_at(scaling, "customScalingQueueLengthDeduction").unwrap_or_else(|| "0".into()),
                str_at(scaling, "customScalingRunningJobPercentage").unwrap_or_else(|| "100".into()),
            ),
            _ => strategy,
        },
    ));
    let pending = strings(&scaling["pendingPodConditions"]);
    if !pending.is_empty() {
        d.info.push(InfoRow::new("Pending until", pending.join(", ")).mono());
    }
    if spec["triggers"].as_array().is_some_and(|t| t.len() > 1)
        && let Some(calc) = str_at(scaling, "multipleScalersCalculation")
    {
        d.info.push(InfoRow::new("Several triggers", format!("{calc} of them")));
    }
    let rollout = str_at(&spec["rollout"], "strategy").or_else(|| str_at(spec, "rolloutStrategy")).unwrap_or_else(|| "default".into());
    d.info.push(InfoRow::new(
        "On change",
        match rollout.as_str() {
            "gradual" => "gradual: running jobs finish; new ones get the change".into(),
            "default" => "default: running jobs are deleted".into(),
            _ => rollout,
        },
    ));
    let kept = (int_at(spec, "successfulJobsHistoryLimit").unwrap_or(100), int_at(spec, "failedJobsHistoryLimit").unwrap_or(100));
    d.info.push(InfoRow::new("Keeps", format!("{} succeeded · {} failed jobs", kept.0, kept.1)));
    paused_rows(d, v);
    if let Some(t) = str_at(status, "lastActiveTime") {
        d.info.push(InfoRow::new("Last active", t).time());
    }
    d.sections.extend(trigger_sections(v, None));
}

/// The Jobs it made, newest first.
fn jobs_section(jobs: &[Job]) -> InfoSection {
    let mut jobs: Vec<&Job> = jobs.iter().collect();
    jobs.sort_by(|a, b| b.metadata.creation_timestamp.cmp(&a.metadata.creation_timestamp));
    let mut running = 0;
    let rows: Vec<InfoTableRow> = jobs
        .iter()
        .map(|j| {
            let name = j.metadata.name.clone().unwrap_or_default();
            let s = j.status.clone().unwrap_or_default();
            let failed = s.conditions.iter().flatten().any(|c| c.type_ == "Failed" && c.status == "True");
            let state = if failed {
                "Failed"
            } else if s.succeeded.unwrap_or(0) > 0 && s.active.unwrap_or(0) == 0 {
                "Succeeded"
            } else if s.active.unwrap_or(0) > 0 {
                running += 1;
                "Running"
            } else {
                "Pending"
            };
            let started = s.start_time.map(|t| t.0.to_string()).unwrap_or_default();
            let mut row = InfoTableRow::new(vec![name.clone(), state.into(), started]);
            row.link = Some(link("batch/Job", j.metadata.namespace.as_deref(), &name));
            row.warn = failed;
            row
        })
        .collect();
    let table = InfoTable { columns: ["Job", "Status", "Started"].map(String::from).to_vec(), mono: vec![0], times: vec![2], rows, ..Default::default() };
    let mut section = table_section("scaledjob-jobs", "Jobs", table, "No jobs: none started, or all cleaned up.");
    if running > 0 {
        section.summary = format!("{} · {running} running", section.summary);
    }
    section
}

// ---- TriggerAuthentication ----

fn authentication(d: &mut ObjectDetails, v: &Value) {
    let spec = &v["spec"];
    let ns = d.namespace.clone();
    let ns = ns.as_deref();
    identity_rows(d, &spec["podIdentity"], "Pod identity");
    let mut rows = Vec::new();
    let mut add = |parameter: Option<String>, from: &str, what: String, key: String, link: Option<ObjectLink>| {
        let mut row = InfoTableRow::new(vec![parameter.unwrap_or_default(), from.into(), what, key]);
        row.link = link;
        rows.push(row);
    };
    for e in items(&spec["secretTargetRef"]) {
        let name = str_at(e, "name").unwrap_or_default();
        add(str_at(e, "parameter"), "Secret", name.clone(), str_at(e, "key").unwrap_or_default(), Some(link("core/Secret", ns, &name)));
    }
    for e in items(&spec["configMapTargetRef"]) {
        let name = str_at(e, "name").unwrap_or_default();
        add(str_at(e, "parameter"), "ConfigMap", name.clone(), str_at(e, "key").unwrap_or_default(), Some(link("core/ConfigMap", ns, &name)));
    }
    for e in items(&spec["env"]) {
        let container = str_at(e, "containerName").map(|c| format!("container {c}")).unwrap_or_default();
        add(str_at(e, "parameter"), "Env var", str_at(e, "name").unwrap_or_default(), container, None);
    }
    for e in items(&spec["boundServiceAccountToken"]) {
        let name = str_at(e, "serviceAccountName").unwrap_or_default();
        add(str_at(e, "parameter"), "ServiceAccount token", name.clone(), String::new(), Some(link("core/ServiceAccount", ns, &name)));
    }
    let vault = &spec["hashiCorpVault"];
    for e in items(&vault["secrets"]) {
        add(str_at(e, "parameter"), "Vault", str_at(e, "path").unwrap_or_default(), str_at(e, "key").unwrap_or_default(), None);
    }
    let azure = &spec["azureKeyVault"];
    for e in items(&azure["secrets"]) {
        add(str_at(e, "parameter"), "Azure Key Vault", str_at(e, "name").unwrap_or_default(), str_at(e, "version").unwrap_or_default(), None);
    }
    let aws = &spec["awsSecretManager"];
    for e in items(&aws["secrets"]) {
        let key = str_at(e, "secretKey").or_else(|| str_at(e, "versionStage")).or_else(|| str_at(e, "versionId")).unwrap_or_default();
        add(str_at(e, "parameter"), "AWS Secrets Manager", str_at(e, "name").unwrap_or_default(), key, None);
    }
    let gcp = &spec["gcpSecretManager"];
    for e in items(&gcp["secrets"]) {
        add(str_at(e, "parameter"), "Google Secret Manager", str_at(e, "id").unwrap_or_default(), str_at(e, "version").unwrap_or_default(), None);
    }

    if vault.is_object() {
        let mut text = str_at(vault, "address").unwrap_or_default();
        if let Some(n) = str_at(vault, "namespace") {
            text.push_str(&format!(" · namespace {n}"));
        }
        d.info.push(InfoRow::new("Vault", text).mono());
        let mut auth = vec![str_at(vault, "authentication").unwrap_or_default()];
        auth.extend(str_at(vault, "role").map(|r| format!("role {r}")));
        auth.extend(str_at(vault, "mount").map(|m| format!("mount {m}")));
        d.info.push(InfoRow::new("Vault auth", auth.join(" · ")));
        if str_at(&vault["credential"], "token").is_some() {
            d.info.push(InfoRow::new("Vault token", "written in this object (not shown)"));
        }
        if let Some(path) = str_at(&vault["credential"], "serviceAccount") {
            d.info.push(InfoRow::new("Vault token file", path).mono());
        }
    }
    if azure.is_object() {
        d.info.push(InfoRow::new("Azure Key Vault", str_at(azure, "vaultUri").unwrap_or_default()).mono());
        provider_credentials(d, &azure["credentials"], &[("clientId", "Client ID"), ("tenantId", "Tenant ID")], &[("clientSecret", "Client secret")], ns);
        identity_rows(d, &azure["podIdentity"], "Key Vault identity");
    }
    if aws.is_object() {
        if let Some(r) = str_at(aws, "region") {
            d.info.push(InfoRow::new("AWS region", r));
        }
        provider_credentials(d, &aws["credentials"], &[], &[("accessKey", "Access key"), ("accessSecretKey", "Secret key"), ("accessToken", "Session token")], ns);
        identity_rows(d, &aws["podIdentity"], "Secrets Manager identity");
    }
    if gcp.is_object() {
        provider_credentials(d, &gcp["credentials"], &[], &[("clientSecret", "Credentials")], ns);
        identity_rows(d, &gcp["podIdentity"], "Secret Manager identity");
    }
    let table = InfoTable { columns: ["Parameter", "From", "Name", "Key"].map(String::from).to_vec(), mono: vec![0, 2, 3], rows, ..Default::default() };
    d.sections.push(table_section("keda-auth-params", "Parameters", drop_empty_columns(table, &[3]), "No parameters: only an identity."));
}

fn items(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}

/// The pod identity a scaler signs in with, in words.
fn identity_rows(d: &mut ObjectDetails, identity: &Value, label: &str) {
    let Some(provider) = str_at(identity, "provider").filter(|p| p != "none") else { return };
    let mut text = match provider.as_str() {
        "azure-workload" => "Azure Workload Identity".into(),
        "aws" => "AWS (EKS Pod Identity or IRSA)".into(),
        "aws-eks" => "AWS IRSA (deprecated)".into(),
        "gcp" => "Google Workload Identity".into(),
        _ => provider,
    };
    match str_at(identity, "identityOwner").as_deref() {
        Some("workload") => text.push_str(" · the scaled workload's"),
        Some("keda") => text.push_str(" · KEDA's own"),
        _ => {}
    }
    d.info.push(InfoRow::new(label, text));
    for (key, row) in [("identityId", "Identity ID"), ("roleArn", "Role ARN"), ("identityTenantId", "Tenant ID")] {
        if let Some(value) = str_at(identity, key) {
            d.info.push(InfoRow::new(row, value).mono());
        }
    }
}

/// A provider's credentials: plain IDs as they are, secrets as the Secret
/// and key they're read from.
fn provider_credentials(d: &mut ObjectDetails, creds: &Value, plain: &[(&str, &str)], secrets: &[(&str, &str)], ns: Option<&str>) {
    for (key, label) in plain {
        if let Some(value) = str_at(creds, key) {
            d.info.push(InfoRow::new(*label, value).mono());
        }
    }
    for (key, label) in secrets {
        let r = &creds[*key]["valueFrom"]["secretKeyRef"];
        if let Some(name) = str_at(r, "name") {
            let text = format!("Secret {name} · key {}", str_at(r, "key").unwrap_or_default());
            d.info.push(InfoRow::new(*label, text).mono().link(link("core/Secret", ns, &name)));
        }
    }
}

/// A ClusterTriggerAuthentication's Secrets and ConfigMaps are read from
/// KEDA's own namespace: links point there, or go if it isn't known.
fn cluster_auth_links(d: &mut ObjectDetails, keda_namespace: Option<&str>) {
    let fix = |l: &mut Option<ObjectLink>| {
        if l.as_ref().is_some_and(|l| l.namespace.is_none() && l.kind.starts_with("core/")) {
            match keda_namespace {
                Some(ns) => l.as_mut().unwrap().namespace = Some(ns.to_owned()),
                None => *l = None,
            }
        }
    };
    for r in d.info.iter_mut().chain(d.sections.iter_mut().flat_map(|s| s.rows.iter_mut())) {
        fix(&mut r.link);
    }
    for r in d.sections.iter_mut().flat_map(|s| s.table.iter_mut()).flat_map(|t| t.rows.iter_mut()) {
        fix(&mut r.link);
    }
    if let Some(ns) = keda_namespace {
        d.info.push(InfoRow::new("Secrets from", format!("{ns} (KEDA's namespace)")));
    }
}

/// ScaledObjects and ScaledJobs with a trigger using this authentication.
fn auth_users(items: &[Value], auth_kind: &str, name: &str) -> InfoSection {
    let mut rows: Vec<InfoTableRow> = items
        .iter()
        .filter(|o| {
            o["spec"]["triggers"].as_array().into_iter().flatten().any(|t| {
                let r = &t["authenticationRef"];
                str_at(r, "name").as_deref() == Some(name) && str_at(r, "kind").as_deref().unwrap_or("TriggerAuthentication") == auth_kind
            })
        })
        .map(|o| {
            let (ns, n) = (str_at(&o["metadata"], "namespace").unwrap_or_default(), str_at(&o["metadata"], "name").unwrap_or_default());
            let kind = str_at(o, "kind").unwrap_or_default();
            let status = scaling_status(o);
            let mut row = InfoTableRow::new(vec![ns.clone(), kind.clone(), n.clone(), status.clone()]);
            row.link = Some(link(&format!("{GROUP}/{kind}"), Some(&ns), &n));
            row.warn = status == "Not ready";
            row
        })
        .collect();
    rows.sort_by(|a, b| a.cells.cmp(&b.cells));
    let table = InfoTable { columns: ["Namespace", "Kind", "Name", "Status"].map(String::from).to_vec(), mono: vec![2], rows, ..Default::default() };
    table_section("keda-auth-users", "Used by", table, "No ScaledObjects or ScaledJobs use it.")
}

// ---- CloudEventSource ----

fn cloud_event_source(d: &mut ObjectDetails, v: &Value) {
    let spec = &v["spec"];
    d.status = Some(match condition(v, "Active").and_then(|c| str_at(c, "status")).as_deref() {
        Some("True") => "Active".into(),
        Some("False") => "Inactive".into(),
        _ => "Pending".into(),
    });
    let dest = &spec["destination"];
    if let Some(uri) = str_at(&dest["http"], "uri") {
        d.info.push(InfoRow::new("Sends to", uri).mono());
    } else if let Some(endpoint) = str_at(&dest["azureEventGridTopic"], "endpoint") {
        d.info.push(InfoRow::new("Sends to", format!("Azure Event Grid {endpoint}")).mono());
    }
    if let Some(c) = str_at(spec, "clusterName") {
        d.info.push(InfoRow::new("Cluster name", c));
    }
    let auth = &spec["authenticationRef"];
    if let Some(name) = str_at(auth, "name") {
        let kind = str_at(auth, "kind").unwrap_or_else(|| "TriggerAuthentication".into());
        let ns = if kind == "ClusterTriggerAuthentication" { None } else { d.namespace.clone() };
        let row = InfoRow::new("Authentication", format!("{kind}/{name}"));
        d.info.push(row.link(link(&format!("{GROUP}/{kind}"), ns.as_deref(), &name)));
    }
    let sub = &spec["eventSubscription"];
    let (included, excluded) = (strings(&sub["includedEventTypes"]), strings(&sub["excludedEventTypes"]));
    let events = if !included.is_empty() {
        format!("only {}", included.join(", "))
    } else if !excluded.is_empty() {
        format!("all but {}", excluded.join(", "))
    } else {
        "all".into()
    };
    d.info.push(InfoRow::new("Events", events).mono());
}

// ---- lookups ----

/// ScaledObjects or ScaledJobs in `namespace` (or everywhere), as JSON with
/// their `kind`, which list items leave out.
async fn list(client: &Client, kind: &str, plural: &str, namespace: Option<&str>) -> Result<Vec<Value>> {
    let k = KindRef { group: GROUP.into(), version: VERSION.into(), kind: kind.into(), plural: plural.into(), namespaced: true };
    let list = dynamic_api(client, &k, namespace).list(&ListParams::default()).await?;
    Ok(list
        .items
        .iter()
        .filter_map(|o| serde_json::to_value(o).ok())
        .map(|mut v| {
            v["kind"] = kind.into();
            v
        })
        .collect())
}

// ---- Pause/Resume and the replica range ----

/// Pauses or resumes a ScaledObject or ScaledJob with KEDA's annotations.
/// Pausing leaves the replicas where they are, or for a ScaledObject with
/// `hold`, scales it to that many first (`paused-replicas`, which KEDA
/// prefers over `paused`). Resuming removes both.
#[allow(clippy::too_many_arguments)]
pub async fn set_paused(
    client: &Client,
    gate: &SafetyGate,
    context: &str,
    kind: &KindRef,
    namespace: &str,
    name: &str,
    paused: bool,
    hold: Option<i32>,
    dry_run: bool,
) -> Result<()> {
    if !is_keda_scaler(kind) {
        return Err(Error::Message(format!("{} can't be paused", kind.kind)));
    }
    if hold.is_some() && (kind.kind != "ScaledObject" || !paused) {
        return Err(Error::Message("only a ScaledObject can be paused at a replica count".into()));
    }
    if hold.is_some_and(|n| n < 0) {
        return Err(Error::Message("replicas can't be negative".into()));
    }
    if !dry_run {
        gate.ensure_writable(context)?;
    }
    let annotations = match (paused, hold) {
        (true, Some(n)) => json!({ PAUSED_REPLICAS: n.to_string(), PAUSED: null }),
        (true, None) => json!({ PAUSED: "true", PAUSED_REPLICAS: null }),
        (false, _) => json!({ PAUSED: null, PAUSED_REPLICAS: null }),
    };
    let patch = json!({ "metadata": { "annotations": annotations } });
    let params = PatchParams { dry_run, ..PatchParams::default() };
    dynamic_api(client, kind, Some(namespace)).patch(name, &params, &Patch::Merge(&patch)).await?;
    tracing::info!(%context, kind = %kind.key(), %namespace, %name, paused, ?hold, dry_run, "keda pause set");
    Ok(())
}

/// Sets a ScaledObject's or ScaledJob's `minReplicaCount` and
/// `maxReplicaCount`. Min may be 0 (scale to zero); a ScaledObject's
/// `idleReplicaCount` must stay below min, as KEDA requires.
#[allow(clippy::too_many_arguments)]
pub async fn set_replica_range(
    client: &Client,
    gate: &SafetyGate,
    context: &str,
    kind: &KindRef,
    namespace: &str,
    name: &str,
    min: i32,
    max: i32,
    dry_run: bool,
) -> Result<()> {
    if !is_keda_scaler(kind) {
        return Err(Error::Message(format!("{} has no replica range", kind.kind)));
    }
    if min < 0 || max < 1 || max < min {
        return Err(Error::Message("the range needs 0 ≤ min ≤ max, and max at least 1".into()));
    }
    if !dry_run {
        gate.ensure_writable(context)?;
    }
    let api = dynamic_api(client, kind, Some(namespace));
    if kind.kind == "ScaledObject" {
        let current = api.get(name).await?.data;
        if let Some(idle) = int_at(&current["spec"], "idleReplicaCount")
            && i64::from(min) <= idle
        {
            return Err(Error::Message(format!("min must be above its idle replica count ({idle})")));
        }
    }
    let patch = json!({ "spec": { "minReplicaCount": min, "maxReplicaCount": max } });
    let params = PatchParams { dry_run, ..PatchParams::default() };
    api.patch(name, &params, &Patch::Merge(&patch)).await?;
    tracing::info!(%context, kind = %kind.key(), %namespace, %name, min, max, dry_run, "keda range set");
    Ok(())
}

#[cfg(test)]
#[path = "keda_tests.rs"]
mod coverage_tests;
