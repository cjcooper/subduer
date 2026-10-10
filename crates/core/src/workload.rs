//! Everything about one Deployment, StatefulSet, Job or CronJob for the
//! Details tab: status, its pods, the pod template's containers, usage summed
//! over the pods, and the events of the workload, its ReplicaSets or Jobs, and
//! its pods.

use std::collections::{HashMap, HashSet};

use k8s_openapi::api::apps::v1::{Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Container, Pod, PodTemplateSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta, OwnerReference, Time};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{Api, ListParams};
use kube::{Client, ResourceExt};

use crate::pod_details::{container_resources, env_vars, list_events, pods_usage, port_names, read_config_maps, read_secret_keys};
use crate::table::{container_states, pod_status};
use crate::types::{
    Condition, JobRun, KindRef, ReplicaCounts, TemplateContainer, Usage, WorkloadDetails, WorkloadPod,
    has_workload_details, is_batch_workload,
};
use crate::{Error, Result};

/// Pod → container → (cpu millicores, memory bytes).
type PodsUsage = HashMap<String, HashMap<String, (f64, f64)>>;

/// The parts of a workload that don't depend on the kind.
struct Workload {
    kind: String,
    meta: ObjectMeta,
    /// Label selector for its pods in kubectl syntax; empty when it has none,
    /// e.g. a CronJob without Jobs.
    pod_selector: String,
    template: PodTemplateSpec,
    status: String,
    replicas: Option<ReplicaCounts>,
    suspended: Option<bool>,
    info: Vec<(String, String)>,
    conditions: Vec<Condition>,
    jobs: Vec<JobRun>,
    /// Whether a pod runs the latest template.
    is_updated: Box<dyn Fn(&Pod) -> bool + Send + Sync>,
    /// Owners the pods must be controlled by: the StatefulSet or Job itself,
    /// the Deployment's ReplicaSets, or the CronJob's Jobs.
    owners: Vec<(String, String)>,
}

pub async fn workload_details(client: &Client, kind: &KindRef, namespace: &str, name: &str) -> Result<WorkloadDetails> {
    let w = load(client, kind, namespace, name).await?;
    let selector = w.pod_selector.clone();
    let mut pods = owned_pods(client, namespace, &selector, &w.owners).await?;
    if is_batch_workload(kind) {
        pods.sort_by_key(|p| std::cmp::Reverse(p.creation_timestamp().map(|t| t.0)));
    }

    let template_spec = w.template.spec.clone().unwrap_or_default();
    let all: Vec<&Container> = template_spec.init_containers.iter().flatten().chain(&template_spec.containers).collect();
    let placements: Vec<(String, Option<String>)> =
        pods.iter().map(|p| (p.name_any(), p.spec.as_ref().and_then(|s| s.node_name.clone()))).collect();

    let object_names: HashMap<(String, String), Option<String>> = w
        .owners
        .iter()
        .filter(|(k, _)| k != &w.kind)
        .map(|(k, n)| ((k.clone(), n.clone()), Some(format!("{k} {n}"))))
        .chain(placements.iter().map(|(p, _)| (("Pod".to_owned(), p.clone()), Some(format!("Pod {p}")))))
        .chain([((w.kind.clone(), name.to_owned()), None)])
        .collect();
    let (usage, config_maps, secret_keys, events) = tokio::join!(
        pods_usage(client, namespace, &selector, &placements),
        read_config_maps(client, namespace, &all),
        read_secret_keys(client, namespace, &all),
        list_events(client, namespace, None, |e| {
            let key = (e.involved_object.kind.clone()?, e.involved_object.name.clone()?);
            object_names.get(&key).cloned()
        }),
    );

    let workload_pods: Vec<WorkloadPod> = pods.iter().map(|p| workload_pod(p, &w, usage.as_ref())).collect();
    let containers: Vec<TemplateContainer> = template_spec
        .init_containers
        .iter()
        .flatten()
        .map(|c| (c, true))
        .chain(template_spec.containers.iter().map(|c| (c, false)))
        .map(|(c, init)| {
            let (cpu, memory) = container_resources(c);
            let used = usage.as_ref().map(|u| summed(u.values().filter_map(|pod| pod.get(&c.name).copied())));
            TemplateContainer {
                name: c.name.clone(),
                init,
                sidecar: init && c.restart_policy.as_deref() == Some("Always"),
                image: c.image.clone().unwrap_or_default(),
                ports: port_names(c),
                cpu: Usage { used: used.map(|u| u.0), ..cpu },
                memory: Usage { used: used.map(|u| u.1), ..memory },
                env: env_vars(c, &config_maps, &secret_keys),
            }
        })
        .collect();

    // Totals over the pods still running; limits only if every long-running
    // container has one (the same rule as a pod's).
    let live: Vec<&Pod> = pods.iter().filter(|p| !finished(p)).collect();
    // A Job or CronJob with nothing running shows what one pod would get.
    let per_pod = live.is_empty() && is_batch_workload(kind);
    let specs: Vec<&k8s_openapi::api::core::v1::PodSpec> = if per_pod {
        vec![&template_spec]
    } else {
        live.iter().filter_map(|p| p.spec.as_ref()).collect()
    };
    let resources = specs
        .into_iter()
        .map(crate::overview::pod_resources)
        .fold((0.0, 0.0, 0.0, 0.0), |a, r| {
            (a.0 + r.cpu_requests, a.1 + r.cpu_limits, a.2 + r.memory_requests, a.3 + r.memory_limits)
        });
    let long_running: Vec<&TemplateContainer> = containers.iter().filter(|c| !c.init || c.sidecar).collect();
    let all_limited = |pick: fn(&TemplateContainer) -> Option<f64>| long_running.iter().all(|c| pick(c).is_some());
    let used = usage.as_ref().map(|u| summed(u.values().flat_map(|pod| pod.values().copied())));

    Ok(WorkloadDetails {
        kind: w.kind.clone(),
        name: name.to_owned(),
        namespace: namespace.to_owned(),
        status: w.status.clone(),
        replicas: w.replicas,
        suspended: w.suspended,
        info: w.info.clone(),
        created: w.meta.creation_timestamp.as_ref().map(|t| t.0.to_string()),
        labels: w.meta.labels.clone().unwrap_or_default().into_iter().collect(),
        conditions: w.conditions.clone(),
        cpu: Usage {
            requests: positive(resources.0),
            limits: all_limited(|c| c.cpu.limits).then_some(resources.1).filter(|_| !live.is_empty() || per_pod),
            used: used.map(|u| u.0),
            node: None,
        },
        memory: Usage {
            requests: positive(resources.2),
            limits: all_limited(|c| c.memory.limits).then_some(resources.3).filter(|_| !live.is_empty() || per_pod),
            used: used.map(|u| u.1),
            node: None,
        },
        metrics_available: usage.is_some(),
        pods: workload_pods,
        containers,
        jobs: w.jobs.clone(),
        events,
    })
}

/// The workload's current pods with their nodes, for usage history.
pub async fn current_pods(
    client: &Client,
    kind: &KindRef,
    namespace: &str,
    name: &str,
) -> Result<(String, Vec<(String, Option<String>)>)> {
    let w = load(client, kind, namespace, name).await?;
    let selector = w.pod_selector.clone();
    let pods = owned_pods(client, namespace, &selector, &w.owners).await?;
    let placements = pods
        .iter()
        .filter(|p| !finished(p))
        .map(|p| (p.name_any(), p.spec.as_ref().and_then(|s| s.node_name.clone())))
        .collect();
    Ok((selector, placements))
}

/// A Prometheus regex matching every pod the workload has ever created,
/// including ones replaced during the last hour.
pub fn pod_name_regex(kind: &KindRef, name: &str) -> String {
    let name = name.replace('.', r"\\.");
    match kind.kind.as_str() {
        "StatefulSet" => format!("{name}-[0-9]+"),
        // <job>[-<completion index>]-<5 random characters>
        "Job" => format!("{name}(-[0-9]+)?-[a-z0-9]{{5}}"),
        // Jobs are <cronjob>-<scheduled minute>, or <cronjob>-manual-<suffix> from Run now.
        "CronJob" => format!("{name}-([0-9]+|manual-[a-z0-9]+)(-[0-9]+)?-[a-z0-9]{{5}}"),
        // <deployment>-<pod-template-hash>-<5 random characters>
        _ => format!("{name}-[a-z0-9]{{1,10}}-[a-z0-9]{{5}}"),
    }
}

async fn load(client: &Client, kind: &KindRef, namespace: &str, name: &str) -> Result<Workload> {
    if !has_workload_details(kind) {
        return Err(Error::Message(format!("no details view for {}", kind.kind)));
    }
    match kind.kind.as_str() {
        "StatefulSet" => {
            let sts = Api::<StatefulSet>::namespaced(client.clone(), namespace).get(name).await?;
            Ok(statefulset(sts))
        }
        "Job" => {
            let job = Api::<Job>::namespaced(client.clone(), namespace).get(name).await?;
            Ok(job_workload(job, jiff::Timestamp::now()))
        }
        "CronJob" => {
            let (cron_jobs, jobs) =
                (Api::<CronJob>::namespaced(client.clone(), namespace), Api::<Job>::namespaced(client.clone(), namespace));
            let params = ListParams::default();
            let (cron_job, jobs) = tokio::join!(cron_jobs.get(name), jobs.list(&params));
            let jobs = jobs?
                .items
                .into_iter()
                .filter(|j| controlled_by(j.owner_references(), "CronJob", name))
                .collect();
            Ok(cron_job_workload(cron_job?, jobs, jiff::Timestamp::now()))
        }
        _ => {
            let deployment = Api::<Deployment>::namespaced(client.clone(), namespace).get(name).await?;
            let selector = selector_string(&deployment.spec.as_ref().map(|s| s.selector.clone()).unwrap_or_default());
            let replica_sets = Api::<ReplicaSet>::namespaced(client.clone(), namespace)
                .list(&ListParams::default().labels(&selector))
                .await?
                .items
                .into_iter()
                .filter(|rs| controlled_by(rs.owner_references(), "Deployment", name))
                .collect();
            Ok(deployment_workload(deployment, replica_sets))
        }
    }
}

fn deployment_workload(d: Deployment, replica_sets: Vec<ReplicaSet>) -> Workload {
    let spec = d.spec.clone().unwrap_or_default();
    let status = d.status.clone().unwrap_or_default();
    let revision = d.annotations().get("deployment.kubernetes.io/revision").cloned();
    let replicas = ReplicaCounts {
        desired: spec.replicas.unwrap_or(1),
        current: status.replicas.unwrap_or(0),
        ready: status.ready_replicas.unwrap_or(0),
        updated: status.updated_replicas.unwrap_or(0),
        available: status.available_replicas.unwrap_or(0),
    };
    let conditions = conditions(status.conditions.as_deref());
    let failed = conditions
        .iter()
        .any(|c| c.type_ == "Progressing" && c.reason == "ProgressDeadlineExceeded");
    let stale = status.observed_generation.unwrap_or(0) < d.metadata.generation.unwrap_or(0);
    let state = if spec.paused == Some(true) {
        "Paused"
    } else if failed {
        "Failed"
    } else if replicas.desired == 0 && replicas.current == 0 {
        "Scaled to 0"
    } else if stale || replicas.updated < replicas.desired || replicas.current > replicas.updated {
        "Rolling out"
    } else if replicas.available < replicas.desired {
        "Degraded"
    } else {
        "Available"
    };

    let strategy = spec.strategy.clone().unwrap_or_default();
    let strategy = match strategy.type_.as_deref() {
        Some("Recreate") => "Recreate".to_owned(),
        _ => {
            let ru = strategy.rolling_update.unwrap_or_default();
            format!(
                "RollingUpdate · max surge {} · max unavailable {}",
                int_or_string(ru.max_surge.as_ref(), "25%"),
                int_or_string(ru.max_unavailable.as_ref(), "25%"),
            )
        }
    };
    // The newest ReplicaSet carries the Deployment's revision.
    let newest_hash = replica_sets
        .iter()
        .find(|rs| rs.annotations().get("deployment.kubernetes.io/revision") == revision.as_ref())
        .and_then(|rs| rs.labels().get("pod-template-hash").cloned());
    let mut info = vec![("Strategy".to_owned(), strategy)];
    push(&mut info, "Revision", revision);
    push(&mut info, "Min ready", spec.min_ready_seconds.filter(|s| *s > 0).map(|s| format!("{s}s")));
    push(&mut info, "Progress deadline", spec.progress_deadline_seconds.filter(|s| *s != 600).map(|s| format!("{s}s")));
    with_common_rows(Workload {
        kind: "Deployment".to_owned(),
        meta: d.metadata,
        pod_selector: selector_string(&spec.selector),
        template: spec.template,
        status: state.to_owned(),
        replicas: Some(replicas),
        suspended: None,
        info,
        conditions,
        jobs: Vec::new(),
        is_updated: Box::new(move |pod: &Pod| {
            newest_hash.is_some() && pod.labels().get("pod-template-hash") == newest_hash.as_ref()
        }),
        owners: replica_sets.iter().map(|rs| ("ReplicaSet".to_owned(), rs.name_any())).collect(),
    })
}

fn statefulset(sts: StatefulSet) -> Workload {
    let spec = sts.spec.clone().unwrap_or_default();
    let status = sts.status.clone().unwrap_or_default();
    let replicas = ReplicaCounts {
        desired: spec.replicas.unwrap_or(1),
        current: status.replicas,
        ready: status.ready_replicas.unwrap_or(0),
        updated: status.updated_replicas.unwrap_or(0),
        available: status.available_replicas.unwrap_or(0),
    };
    let conditions = conditions(status.conditions.as_deref());
    let (current_rev, update_rev) = (status.current_revision.clone(), status.update_revision.clone());
    let stale = status.observed_generation.unwrap_or(0) < sts.metadata.generation.unwrap_or(0);
    let state = if replicas.desired == 0 && replicas.current == 0 {
        "Scaled to 0"
    } else if stale || (update_rev.is_some() && current_rev != update_rev) || replicas.updated < replicas.desired {
        "Rolling out"
    } else if replicas.ready < replicas.desired {
        "Degraded"
    } else {
        "Ready"
    };

    let strategy = spec.update_strategy.clone().unwrap_or_default();
    let strategy = match strategy.type_.as_deref() {
        Some("OnDelete") => "OnDelete (pods update only when deleted)".to_owned(),
        _ => {
            let ru = strategy.rolling_update.unwrap_or_default();
            let mut s = "RollingUpdate".to_owned();
            if let Some(p) = ru.partition.filter(|p| *p > 0) {
                s += &format!(" · partition {p}");
            }
            if let Some(m) = ru.max_unavailable.as_ref() {
                s += &format!(" · max unavailable {}", int_or_string(Some(m), "1"));
            }
            s
        }
    };
    let claims: Vec<String> = spec
        .volume_claim_templates
        .iter()
        .flatten()
        .map(|t| {
            let pvc = t.spec.clone().unwrap_or_default();
            let size = pvc
                .resources
                .and_then(|r| r.requests)
                .and_then(|r| r.get("storage").map(|q| q.0.clone()));
            let details: Vec<String> = size.into_iter().chain(pvc.storage_class_name).collect();
            if details.is_empty() {
                t.name_any()
            } else {
                format!("{} ({})", t.name_any(), details.join(", "))
            }
        })
        .collect();
    let retention = spec.persistent_volume_claim_retention_policy.as_ref().map(|r| {
        format!(
            "when deleted: {} · when scaled: {}",
            r.when_deleted.as_deref().unwrap_or("Retain"),
            r.when_scaled.as_deref().unwrap_or("Retain"),
        )
    });
    let revision = match (&current_rev, &update_rev) {
        (Some(c), Some(u)) if c != u => Some(format!("{c} → {u}")),
        (Some(c), _) => Some(c.clone()),
        (None, u) => u.clone(),
    };

    let mut info = vec![("Strategy".to_owned(), strategy)];
    push(&mut info, "Pod management", spec.pod_management_policy.clone());
    push(&mut info, "Service", spec.service_name.clone().filter(|s| !s.is_empty()));
    push(&mut info, "Volume claims", (!claims.is_empty()).then(|| claims.join(", ")));
    push(&mut info, "Claim retention", retention);
    push(&mut info, "Revision", revision);
    push(&mut info, "Min ready", spec.min_ready_seconds.filter(|s| *s > 0).map(|s| format!("{s}s")));
    let name = sts.name_any();
    with_common_rows(Workload {
        kind: "StatefulSet".to_owned(),
        meta: sts.metadata,
        pod_selector: selector_string(&spec.selector),
        template: spec.template,
        status: state.to_owned(),
        replicas: Some(replicas),
        suspended: None,
        info,
        conditions,
        jobs: Vec::new(),
        is_updated: Box::new(move |pod: &Pod| {
            update_rev.is_some() && pod.labels().get("controller-revision-hash") == update_rev.as_ref()
        }),
        owners: vec![("StatefulSet".to_owned(), name)],
    })
}

/// `Complete`, `Failed`, `Suspended`, `Running` or `Pending`.
fn job_state(job: &Job) -> &'static str {
    let status = job.status.clone().unwrap_or_default();
    let holds = |t: &str| status.conditions.iter().flatten().any(|c| c.type_ == t && c.status == "True");
    if holds("Complete") || holds("SuccessCriteriaMet") {
        "Complete"
    } else if holds("Failed") || holds("FailureTarget") {
        "Failed"
    } else if job.spec.as_ref().and_then(|s| s.suspend) == Some(true) {
        "Suspended"
    } else if status.active.unwrap_or(0) > 0 {
        "Running"
    } else {
        "Pending"
    }
}

/// When it completed, or failed: a failed Job has no completion time, so
/// it's when its Failed condition was set.
fn job_finished(job: &Job) -> Option<jiff::Timestamp> {
    let status = job.status.as_ref()?;
    status.completion_time.as_ref().map(|t| t.0).or_else(|| {
        status
            .conditions
            .iter()
            .flatten()
            .find(|c| c.type_ == "Failed" && c.status == "True")
            .and_then(|c| c.last_transition_time.as_ref().map(|t| t.0))
    })
}

/// `succeeded/completions`, or just the successes for a work queue.
fn job_completions(job: &Job) -> String {
    let succeeded = job.status.as_ref().and_then(|s| s.succeeded).unwrap_or(0);
    match job.spec.as_ref().and_then(|s| s.completions) {
        Some(c) => format!("{succeeded}/{c}"),
        None => format!("{succeeded}/1+"),
    }
}

fn job_workload(job: Job, now: jiff::Timestamp) -> Workload {
    let spec = job.spec.clone().unwrap_or_default();
    let status = job.status.clone().unwrap_or_default();
    let state = job_state(&job);
    let mut info = vec![("Completions".to_owned(), job_completions(&job) + " succeeded")];
    let (active, ready) = (status.active.unwrap_or(0), status.ready.unwrap_or(0));
    let pods = if active == 1 { "pod" } else { "pods" };
    push(&mut info, "Active", (active > 0).then(|| format!("{active} {pods} · {ready} ready")));
    let failed = status.failed.unwrap_or(0);
    let failed = match spec.backoff_limit_per_index {
        Some(per_index) => format!("{failed} · retries allowed per index: {per_index}"),
        None => format!("{failed} · retries allowed: {}", spec.backoff_limit.unwrap_or(6)),
    };
    info.push(("Failed pods".to_owned(), failed));
    push(&mut info, "Parallelism", spec.parallelism.filter(|p| *p != 1).map(|p| p.to_string()));
    if spec.completion_mode.as_deref() == Some("Indexed") {
        let done = status.completed_indexes.clone().filter(|i| !i.is_empty());
        info.push(("Completion mode".to_owned(), format!("Indexed · done: {}", done.as_deref().unwrap_or("none"))));
        push(&mut info, "Failed indexes", status.failed_indexes.clone());
    }
    push(&mut info, "Started", time(&status.start_time));
    let finished = job_finished(&job);
    push(&mut info, "Finished", finished.map(|t| t.to_string()));
    if let Some(start) = &status.start_time {
        let verb = if finished.is_some() { "took" } else { "running for" };
        push(&mut info, "Duration", Some(format!("{verb} {}", duration(start.0, finished.unwrap_or(now)))));
    }
    push(&mut info, "Active deadline", spec.active_deadline_seconds.map(|s| format!("{s}s")));
    push(
        &mut info,
        "Clean up",
        spec.ttl_seconds_after_finished.map(|s| format!("deleted {s}s after it finishes")),
    );
    let name = job.name_any();
    with_common_rows(Workload {
        kind: "Job".to_owned(),
        meta: job.metadata,
        pod_selector: spec.selector.as_ref().map(selector_string).unwrap_or_default(),
        template: spec.template,
        status: state.to_owned(),
        replicas: None,
        suspended: Some(spec.suspend == Some(true)),
        info,
        conditions: conditions(status.conditions.as_deref()),
        jobs: Vec::new(),
        is_updated: Box::new(|_| true),
        owners: vec![("Job".to_owned(), name)],
    })
}

fn cron_job_workload(cj: CronJob, mut jobs: Vec<Job>, now: jiff::Timestamp) -> Workload {
    let spec = cj.spec.clone();
    let status = cj.status.clone().unwrap_or_default();
    jobs.sort_by_key(|j| std::cmp::Reverse(j.creation_timestamp().map(|t| t.0)));
    let runs: Vec<JobRun> = jobs
        .iter()
        .map(|j| {
            let status = j.status.clone().unwrap_or_default();
            JobRun {
                name: j.name_any(),
                status: job_state(j).to_owned(),
                completions: job_completions(j),
                manual: j.annotations().get("cronjob.kubernetes.io/instantiate").map(String::as_str) == Some("manual"),
                created: j.creation_timestamp().map(|t| t.0.to_string()),
                started: time(&status.start_time),
                finished: job_finished(j).map(|t| t.to_string()),
            }
        })
        .collect();
    let suspended = spec.suspend == Some(true);
    let running = runs.iter().filter(|r| r.status == "Running").count();
    let state = if suspended {
        "Suspended"
    } else if running > 0 || status.active.as_ref().is_some_and(|a| !a.is_empty()) {
        "Running"
    } else {
        "Scheduled"
    };

    let zone = spec.time_zone.as_deref();
    let schedule = format!("{} ({})", spec.schedule, zone.unwrap_or("controller's time zone, usually UTC"));
    let mut info = vec![("Schedule".to_owned(), schedule)];
    if !suspended {
        let next = crate::cron::next_run(&spec.schedule, zone, now);
        info.push(("Next run".to_owned(), next.map_or_else(|| "can't work out".to_owned(), |t| t.to_string())));
    }
    push(&mut info, "Last scheduled", time(&status.last_schedule_time));
    push(&mut info, "Last successful", time(&status.last_successful_time));
    info.push(("Concurrency".to_owned(), spec.concurrency_policy.clone().unwrap_or_else(|| "Allow".to_owned())));
    push(&mut info, "Starting deadline", spec.starting_deadline_seconds.map(|s| format!("{s}s")));
    info.push((
        "History kept".to_owned(),
        format!(
            "{} successful · {} failed",
            spec.successful_jobs_history_limit.unwrap_or(3),
            spec.failed_jobs_history_limit.unwrap_or(1)
        ),
    ));
    let template = spec.job_template.spec.clone().unwrap_or_default().template;
    let names: Vec<String> = runs.iter().map(|r| r.name.clone()).collect();
    with_common_rows(Workload {
        kind: "CronJob".to_owned(),
        meta: cj.metadata,
        // Every Job labels its pods with its name.
        pod_selector: if names.is_empty() { String::new() } else { format!("job-name in ({})", names.join(",")) },
        template,
        status: state.to_owned(),
        replicas: None,
        suspended: Some(suspended),
        info,
        conditions: Vec::new(),
        jobs: runs,
        is_updated: Box::new(|_| true),
        owners: names.into_iter().map(|n| ("Job".to_owned(), n)).collect(),
    })
}

/// Adds the rows every workload has, after the kind-specific ones.
fn with_common_rows(mut w: Workload) -> Workload {
    let info = &mut w.info;
    let annotations = w.template.metadata.as_ref().and_then(|m| m.annotations.as_ref());
    push(info, "Last restart", annotations.and_then(|a| a.get(crate::actions::RESTARTED_AT).cloned()));
    // A Job's selector is a generated uid, and a CronJob's is made up here.
    if !matches!(w.kind.as_str(), "Job" | "CronJob") {
        push(info, "Selector", Some(w.pod_selector.clone()));
    }
    let spec = w.template.spec.as_ref();
    push(info, "Service account", spec.and_then(|s| s.service_account_name.clone()));
    push(info, "Priority class", spec.and_then(|s| s.priority_class_name.clone()));
    if w.kind == "Job" || w.kind == "CronJob" {
        push(info, "Restart policy", spec.and_then(|s| s.restart_policy.clone()));
    }
    let owner = w.meta.owner_references.iter().flatten().find(|o| o.controller == Some(true));
    push(info, "Controlled by", owner.map(|o| format!("{}/{}", o.kind, o.name)));
    w
}

/// Any kind's status conditions; they share these fields.
pub(crate) fn conditions<T: serde::Serialize>(list: Option<&[T]>) -> Vec<Condition> {
    let text = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or_default().to_owned();
    list.unwrap_or_default()
        .iter()
        .filter_map(|c| serde_json::to_value(c).ok())
        .map(|c| Condition {
            type_: text(&c, "type"),
            status: text(&c, "status"),
            reason: text(&c, "reason"),
            message: text(&c, "message"),
            last_transition: c["lastTransitionTime"].as_str().map(str::to_owned),
        })
        .collect()
}

fn time(t: &Option<Time>) -> Option<String> {
    t.as_ref().map(|t| t.0.to_string())
}

/// e.g. `45s`, `3m 12s`, `2h 5m`, `1d 3h`.
fn duration(from: jiff::Timestamp, to: jiff::Timestamp) -> String {
    let s = to.as_second().saturating_sub(from.as_second()).max(0);
    match s {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m {}s", s / 60, s % 60),
        s if s < 86400 => format!("{}h {}m", s / 3600, s % 3600 / 60),
        s => format!("{}d {}h", s / 86400, s % 86400 / 3600),
    }
}

/// Pods matching the selector that one of `owners` controls.
async fn owned_pods(client: &Client, namespace: &str, selector: &str, owners: &[(String, String)]) -> Result<Vec<Pod>> {
    // An empty selector would list every pod in the namespace.
    if selector.is_empty() || owners.is_empty() {
        return Ok(Vec::new());
    }
    let owners: HashSet<(&str, &str)> = owners.iter().map(|(k, n)| (k.as_str(), n.as_str())).collect();
    let mut pods: Vec<Pod> = Api::<Pod>::namespaced(client.clone(), namespace)
        .list(&ListParams::default().labels(selector))
        .await?
        .items
        .into_iter()
        .filter(|p| {
            p.owner_references()
                .iter()
                .any(|o| o.controller == Some(true) && owners.contains(&(o.kind.as_str(), o.name.as_str())))
        })
        .collect();
    // StatefulSet pods by ordinal; others by name.
    pods.sort_by_key(|p| {
        let name = p.name_any();
        let ordinal = name.rsplit('-').next().and_then(|n| n.parse::<u64>().ok());
        (ordinal, name)
    });
    Ok(pods)
}

fn workload_pod(pod: &Pod, w: &Workload, usage: Option<&PodsUsage>) -> WorkloadPod {
    let data = serde_json::to_value(pod).unwrap_or_default();
    let containers = container_states(&data);
    let statuses = pod.status.as_ref().and_then(|s| s.container_statuses.as_ref());
    let ready = statuses.map_or(0, |s| s.iter().filter(|c| c.ready).count());
    let total = pod.spec.as_ref().map_or(0, |s| s.containers.len());
    let name = pod.name_any();
    let used = usage.and_then(|u| u.get(&name)).map(|c| summed(c.values().copied()));
    WorkloadPod {
        status: pod_status(&data, pod.metadata.deletion_timestamp.is_some()),
        containers,
        ready: format!("{ready}/{total}"),
        restarts: statuses.map_or(0, |s| s.iter().map(|c| i64::from(c.restart_count)).sum()),
        node: pod.spec.as_ref().and_then(|s| s.node_name.clone()),
        created: pod.creation_timestamp().map(|t| t.0.to_string()),
        updated: (w.is_updated)(pod),
        finished: finished(pod),
        cpu: used.map(|u| u.0),
        memory: used.map(|u| u.1),
        name,
    }
}

fn controlled_by(owners: &[OwnerReference], kind: &str, name: &str) -> bool {
    owners.iter().any(|o| o.controller == Some(true) && o.kind == kind && o.name == name)
}

fn finished(pod: &Pod) -> bool {
    matches!(pod.status.as_ref().and_then(|s| s.phase.as_deref()), Some("Succeeded" | "Failed"))
}

/// kubectl's selector syntax: `app=api,tier in (web,edge),!canary`.
pub(crate) fn selector_string(selector: &LabelSelector) -> String {
    let labels = selector.match_labels.iter().flatten().map(|(k, v)| format!("{k}={v}"));
    let expressions = selector.match_expressions.iter().flatten().map(|e| {
        let values = e.values.clone().unwrap_or_default().join(",");
        match e.operator.as_str() {
            "In" => format!("{} in ({values})", e.key),
            "NotIn" => format!("{} notin ({values})", e.key),
            "Exists" => e.key.clone(),
            "DoesNotExist" => format!("!{}", e.key),
            op => format!("{} {op} ({values})", e.key),
        }
    });
    labels.chain(expressions).collect::<Vec<_>>().join(",")
}

fn int_or_string(v: Option<&IntOrString>, default: &str) -> String {
    match v {
        Some(IntOrString::Int(i)) => i.to_string(),
        Some(IntOrString::String(s)) => s.clone(),
        None => default.to_owned(),
    }
}

fn push(info: &mut Vec<(String, String)>, key: &str, value: Option<String>) {
    if let Some(v) = value.filter(|v| !v.is_empty()) {
        info.push((key.to_owned(), v));
    }
}

fn summed(values: impl Iterator<Item = (f64, f64)>) -> (f64, f64) {
    values.fold((0.0, 0.0), |(c, m), (dc, dm)| (c + dc, m + dm))
}

fn positive(v: f64) -> Option<f64> {
    (v > 0.0).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selector_syntax() {
        let selector: LabelSelector = serde_json::from_value(json!({
            "matchLabels": {"app": "api"},
            "matchExpressions": [
                {"key": "tier", "operator": "In", "values": ["web", "edge"]},
                {"key": "canary", "operator": "DoesNotExist"}
            ]
        }))
        .unwrap();
        assert_eq!(selector_string(&selector), "app=api,tier in (web,edge),!canary");
    }

    #[test]
    fn deployment_status() {
        let d = |spec: serde_json::Value, status: serde_json::Value| -> String {
            let d: Deployment = serde_json::from_value(json!({
                "metadata": {"name": "api", "generation": 2},
                "spec": spec,
                "status": status,
            }))
            .unwrap();
            deployment_workload(d, vec![]).status
        };
        let sel = json!({"matchLabels": {"app": "api"}});
        let steady = json!({"observedGeneration": 2, "replicas": 3, "readyReplicas": 3, "updatedReplicas": 3, "availableReplicas": 3});
        assert_eq!(d(json!({"replicas": 3, "selector": sel, "template": {}}), steady.clone()), "Available");
        assert_eq!(d(json!({"replicas": 3, "selector": sel, "template": {}, "paused": true}), steady.clone()), "Paused");
        let rolling = json!({"observedGeneration": 2, "replicas": 4, "readyReplicas": 3, "updatedReplicas": 1, "availableReplicas": 3});
        assert_eq!(d(json!({"replicas": 3, "selector": sel, "template": {}}), rolling), "Rolling out");
        let degraded = json!({"observedGeneration": 2, "replicas": 3, "readyReplicas": 2, "updatedReplicas": 3, "availableReplicas": 2});
        assert_eq!(d(json!({"replicas": 3, "selector": sel, "template": {}}), degraded), "Degraded");
        assert_eq!(d(json!({"replicas": 0, "selector": sel, "template": {}}), json!({"observedGeneration": 2})), "Scaled to 0");
    }

    #[test]
    fn pod_name_patterns() {
        let kind = |k: &str| KindRef {
            group: "apps".into(),
            version: "v1".into(),
            kind: k.into(),
            plural: String::new(),
            namespaced: true,
        };
        assert_eq!(pod_name_regex(&kind("StatefulSet"), "db"), "db-[0-9]+");
        assert_eq!(pod_name_regex(&kind("Deployment"), "a.b"), r"a\\.b-[a-z0-9]{1,10}-[a-z0-9]{5}");
        assert_eq!(pod_name_regex(&kind("Job"), "migrate"), "migrate(-[0-9]+)?-[a-z0-9]{5}");
        assert_eq!(
            pod_name_regex(&kind("CronJob"), "nightly"),
            "nightly-([0-9]+|manual-[a-z0-9]+)(-[0-9]+)?-[a-z0-9]{5}"
        );
    }

    fn now() -> jiff::Timestamp {
        "2026-10-10T12:00:00Z".parse().unwrap()
    }

    fn job(spec: serde_json::Value, status: serde_json::Value) -> Job {
        serde_json::from_value(json!({
            "metadata": {"name": "nightly-29330000", "creationTimestamp": "2026-10-10T11:00:00Z",
                         "ownerReferences": [{"apiVersion": "batch/v1", "kind": "CronJob", "name": "nightly", "uid": "u", "controller": true}]},
            "spec": spec,
            "status": status,
        }))
        .unwrap()
    }

    #[test]
    fn job_status_and_info() {
        let tpl = json!({"spec": {"containers": [], "restartPolicy": "Never"}});
        let done = job(
            json!({"template": tpl, "completions": 1, "selector": {"matchLabels": {"batch.kubernetes.io/controller-uid": "u"}}}),
            json!({"succeeded": 1, "startTime": "2026-10-10T11:00:00Z", "completionTime": "2026-10-10T11:03:12Z",
                   "conditions": [{"type": "Complete", "status": "True"}]}),
        );
        let w = job_workload(done, now());
        assert_eq!(w.status, "Complete");
        assert_eq!(w.pod_selector, "batch.kubernetes.io/controller-uid=u");
        let get = |w: &Workload, k: &str| w.info.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        assert_eq!(get(&w, "Completions").unwrap(), "1/1 succeeded");
        assert_eq!(get(&w, "Duration").unwrap(), "took 3m 12s");
        assert_eq!(get(&w, "Restart policy").unwrap(), "Never");
        assert_eq!(get(&w, "Controlled by").unwrap(), "CronJob/nightly");
        assert_eq!(get(&w, "Selector"), None);

        let running = job(json!({"template": tpl}), json!({"active": 2, "ready": 1, "startTime": "2026-10-10T11:58:00Z"}));
        let w = job_workload(running, now());
        assert_eq!(w.status, "Running");
        assert_eq!(get(&w, "Duration").unwrap(), "running for 2m 0s");
        assert_eq!(get(&w, "Active").unwrap(), "2 pods · 1 ready");
        assert_eq!(w.suspended, Some(false));
        assert_eq!(job_workload(job(json!({"template": tpl, "suspend": true}), json!({})), now()).status, "Suspended");
        let failed = job(
            json!({"template": tpl}),
            json!({"startTime": "2026-10-10T11:00:00Z",
                   "conditions": [{"type": "Failed", "status": "True", "lastTransitionTime": "2026-10-10T11:20:00Z"}]}),
        );
        let w = job_workload(failed, now());
        assert_eq!(w.status, "Failed");
        assert_eq!(get(&w, "Finished").unwrap(), "2026-10-10T11:20:00Z");
        assert_eq!(get(&w, "Duration").unwrap(), "took 20m 0s");
    }

    #[test]
    fn cron_job_status_and_jobs() {
        let cron = |suspend: bool| -> CronJob {
            serde_json::from_value(json!({
                "metadata": {"name": "nightly"},
                "spec": {"schedule": "0 3 * * *", "suspend": suspend,
                         "jobTemplate": {"spec": {"template": {"spec": {"containers": []}}}}},
            }))
            .unwrap()
        };
        let tpl = json!({"spec": {"containers": []}});
        let mut manual = job(json!({"template": tpl}), json!({"active": 1}));
        manual.metadata.name = Some("nightly-manual-abcde".into());
        manual.metadata.creation_timestamp = Some(Time(now()));
        manual.metadata.annotations = Some([("cronjob.kubernetes.io/instantiate".to_owned(), "manual".to_owned())].into());
        let done = job(json!({"template": tpl, "completions": 1}), json!({"succeeded": 1, "conditions": [{"type": "Complete", "status": "True"}]}));

        let w = cron_job_workload(cron(false), vec![done, manual], now());
        assert_eq!(w.status, "Running");
        assert_eq!(w.jobs.iter().map(|j| (j.name.as_str(), j.status.as_str(), j.manual)).collect::<Vec<_>>(), [
            ("nightly-manual-abcde", "Running", true),
            ("nightly-29330000", "Complete", false)
        ]);
        assert_eq!(w.pod_selector, "job-name in (nightly-manual-abcde,nightly-29330000)");
        let next = w.info.iter().find(|(k, _)| k == "Next run").unwrap();
        assert_eq!(next.1, "2026-10-11T03:00:00Z");

        let w = cron_job_workload(cron(true), vec![], now());
        assert_eq!(w.status, "Suspended");
        assert!(w.info.iter().all(|(k, _)| k != "Next run"));
        assert_eq!(w.pod_selector, "");
    }
}
