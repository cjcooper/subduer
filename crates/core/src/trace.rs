//! Traces: CPU and memory kept for as long as the app records, beyond the
//! hour or two the cluster's Prometheus holds. Prometheus is pulled every
//! few minutes (always well inside its retention) with an overlap, and the
//! samples are folded into one-minute buckets in a `.trace` file. Pods
//! Prometheus doesn't scrape (Fargate, Prometheus itself) are sampled by the
//! app from metrics-server or the kubelet while the trace runs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::{StreamExt, stream};
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use kube::{Client, ResourceExt};
use serde_json::Value;

use crate::history::Prometheus;
use crate::quantity::{bytes, millicores};
use crate::trace_file::{
    BUCKET_SECS, Batch, Bucket, ContainerKey, Event, EventKind, Owner, Resources, Source, Stat, TraceFile,
};
use crate::types::{KindRef, TraceEnd, TraceScope, TraceStatus, TraceSummary, TraceTargets, TraceWorkload};
use crate::{Error, Result};

/// Spacing of the raw points folded into each minute.
pub const RAW_STEP_SECS: i64 = 15;
/// Requests, limits, restarts and owners change rarely: one point a minute.
const STATE_STEP_SECS: i64 = 60;
/// How often Prometheus is pulled, at most; half its retention if shorter.
pub const PULL_EVERY_SECS: i64 = 600;
/// Each pull re-reads this much already pulled, for late scrapes.
const OVERLAP_SECS: i64 = 300;
/// Longest span asked for in one query, to keep responses small.
const CHUNK_SECS: i64 = 3600;
/// How far back the first pull reaches.
const BACKFILL_MAX_SECS: i64 = 2 * 3600;
/// Data this close to Prometheus' retention edge may already be gone.
const RETENTION_MARGIN_SECS: i64 = 300;
/// Without a readable retention flag.
const DEFAULT_RETENTION_SECS: f64 = 3600.0;
/// Pods Prometheus misses are sampled this often…
pub const LOCAL_EVERY_SECS: u64 = 30;
/// …and the list of which pods those are is refreshed this often.
const LOCAL_LIST_EVERY_SECS: i64 = 120;
/// The last pull on stop or quit gives up after this.
pub const FINAL_PULL_TIMEOUT: Duration = Duration::from_secs(10);
/// Kubelet stats requests in flight at once.
const KUBELET_CONCURRENCY: usize = 8;

const POD_KEY: [&str; 3] = ["namespace", "pod", "container"];

/// (unix seconds, value).
type Point = (i64, f64);
type Points = Vec<Point>;
/// Series per container and one more label (resource, reason).
type Labelled = Vec<((ContainerKey, String), Points)>;
/// (namespace, name) → (owner kind, owner name).
type OwnerMap = HashMap<(String, String), (String, String)>;

/// A trace being recorded.
pub struct Trace {
    client: Client,
    context: String,
    path: std::path::PathBuf,
    started: f64,
    scope: TraceScope,
    selectors: Vec<Selector>,
    prometheus: Option<Prometheus>,
    retention_secs: f64,
    file: Arc<Mutex<TraceFile>>,
    state: tokio::sync::Mutex<State>,
    /// What the indicator shows; never held across a request, so reading it
    /// doesn't wait for a pull.
    live: Mutex<Live>,
}

#[derive(Default)]
struct Live {
    data_start: i64,
    data_end: i64,
    last_pull: Option<f64>,
    last_error: Option<String>,
    gaps: u32,
    sampled_pods: u32,
}

#[derive(Default)]
struct State {
    /// Prometheus data is in the file up to here (unix seconds, on a minute).
    covered_until: i64,
    memory: Memory,
    local: Local,
}

/// Last values written, so only changes are stored.
#[derive(Default)]
struct Memory {
    resources: HashMap<ContainerKey, (i64, Resources)>,
    counters: HashMap<(ContainerKey, EventKind), (i64, f64)>,
}

#[derive(Default)]
struct Local {
    listed_at: Option<i64>,
    /// Running pods in scope that Prometheus isn't reporting.
    pods: Vec<LocalPod>,
    /// Samples for minutes not yet written.
    pending: HashMap<ContainerKey, Pending>,
}

#[derive(Debug, Clone, PartialEq)]
struct LocalPod {
    namespace: String,
    name: String,
    node: Option<String>,
    owner: Option<Owner>,
    resources: Vec<(String, Resources)>,
}

/// What one pull wrote.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct PullReport {
    pub start: i64,
    pub end: i64,
    /// Containers with usage in the pull.
    pub series: usize,
}

impl Trace {
    /// Creates the trace file and pulls whatever Prometheus still holds
    /// (up to two hours). A failed first pull isn't fatal; the next one
    /// covers the same span.
    pub async fn start(client: Client, context: &str, scope: TraceScope, path: &Path, overwrite: bool) -> Result<Trace> {
        validate(&scope)?;
        let now = unix_now();
        let prometheus = crate::history::discover_prometheus(&client).await;
        let retention_secs = match &prometheus {
            Some(p) => retention(&client, p).await.unwrap_or(DEFAULT_RETENTION_SECS),
            None => DEFAULT_RETENTION_SECS,
        };
        let (path, context, scope_copy) = (path.to_owned(), context.to_owned(), scope.clone());
        let (path_kept, context_kept) = (path.clone(), context.clone());
        let prom_name = prometheus.as_ref().map(|p| format!("{}/{}", p.namespace, p.service));
        let file = tokio::task::spawn_blocking(move || -> Result<TraceFile> {
            let file = TraceFile::create(&path, overwrite, &context, &scope_copy, now)?;
            if let Some(name) = prom_name {
                file.set_meta("prometheus", &name)?;
            }
            file.set_meta("retention_secs", &retention_secs.to_string())?;
            Ok(file)
        })
        .await
        .map_err(|e| Error::Message(e.to_string()))??;
        let backfill = (retention_secs as i64 - RETENTION_MARGIN_SECS).clamp(0, BACKFILL_MAX_SECS);
        let covered_until = align_up(now as i64 - backfill);
        let data_start = if prometheus.is_some() { covered_until } else { align_down(now as i64) };
        let trace = Trace {
            client,
            context: context_kept,
            path: path_kept,
            started: now,
            selectors: selectors(&scope),
            scope,
            prometheus,
            retention_secs,
            file: Arc::new(Mutex::new(file)),
            state: tokio::sync::Mutex::new(State { covered_until, ..State::default() }),
            live: Mutex::new(Live { data_start, data_end: data_start, ..Live::default() }),
        };
        if let Err(e) = trace.pull().await {
            tracing::warn!(error = %e, "first trace pull failed");
        }
        Ok(trace)
    }

    pub fn scope(&self) -> &TraceScope {
        &self.scope
    }

    pub fn has_prometheus(&self) -> bool {
        self.prometheus.is_some()
    }

    /// How the trace is going, from memory.
    pub fn status(&self) -> TraceStatus {
        let live = self.live.lock().unwrap();
        TraceStatus {
            context: self.context.clone(),
            path: self.path.display().to_string(),
            scope: self.scope.clone(),
            started: self.started,
            data_start: live.data_start as f64,
            data_end: live.data_end as f64,
            prometheus: self.prometheus.is_some(),
            last_pull: live.last_pull,
            last_error: live.last_error.clone(),
            gaps: live.gaps,
            sampled_pods: live.sampled_pods,
            stopping: false,
        }
    }

    fn note(&self, f: impl FnOnce(&mut Live)) {
        f(&mut self.live.lock().unwrap());
    }

    /// How often to pull: every ten minutes, or half the retention if that's shorter.
    pub fn pull_every(&self) -> Duration {
        let secs = ((self.retention_secs / 2.0) as i64).clamp(60, PULL_EVERY_SECS);
        Duration::from_secs(secs as u64)
    }

    pub async fn summary(&self, recording: bool) -> Result<TraceSummary> {
        self.with_file(move |f| f.summary(recording)).await
    }

    /// Records until `stop` finishes, then closes the file with its reason.
    pub async fn run(&self, stop: impl Future<Output = TraceEnd>) -> Result<TraceSummary> {
        use tokio::time::{Instant, MissedTickBehavior, interval_at};
        let pull_every = self.pull_every();
        let local_every = Duration::from_secs(LOCAL_EVERY_SECS);
        let mut pulls = interval_at(Instant::now() + pull_every, pull_every);
        let mut samples = interval_at(Instant::now(), local_every);
        pulls.set_missed_tick_behavior(MissedTickBehavior::Delay);
        samples.set_missed_tick_behavior(MissedTickBehavior::Delay);
        tokio::pin!(stop);
        let reason = loop {
            tokio::select! {
                reason = &mut stop => break reason,
                _ = pulls.tick() => {
                    if let Err(e) = self.pull().await {
                        tracing::warn!(error = %e, "trace pull failed");
                    }
                }
                _ = samples.tick() => {
                    if let Err(e) = self.sample_local().await {
                        tracing::warn!(error = %e, "trace sample failed");
                    }
                }
            }
        };
        self.finish(reason).await
    }

    /// A last pull (limited to [`FINAL_PULL_TIMEOUT`]; what it can't get is
    /// a gap), pending samples written, and the file closed.
    pub async fn finish(&self, reason: TraceEnd) -> Result<TraceSummary> {
        let now = unix_now();
        let pulled = match &self.prometheus {
            Some(_) => tokio::time::timeout(FINAL_PULL_TIMEOUT, self.pull()).await.map_err(|_| "timed out".to_owned()),
            None => Ok(Ok(PullReport::default())),
        };
        let mut state = self.state.lock().await;
        if self.prometheus.is_some() && !matches!(pulled, Ok(Ok(_))) {
            let (start, end) = (state.covered_until, align_up(now as i64));
            self.with_file(move |f| f.add_gap(start, end, "the last pull before stopping failed")).await?;
            self.note(|l| l.gaps += 1);
        }
        let batch = local_batch(&mut state, i64::MAX);
        drop(state);
        self.with_file(move |f| {
            f.write(&batch)?;
            f.finish(now, reason)?;
            f.summary(false)
        })
        .await
    }

    pub async fn pull(&self) -> Result<PullReport> {
        self.pull_at(unix_now() as i64).await
    }

    async fn pull_at(&self, now: i64) -> Result<PullReport> {
        let Some(prom) = &self.prometheus else {
            return Ok(PullReport::default());
        };
        let mut state = self.state.lock().await;
        let to = align_down(now);
        let floor = align_up(now - self.retention_secs as i64 + RETENTION_MARGIN_SECS);
        if state.covered_until < floor {
            let start = state.covered_until;
            self.with_file(move |f| f.add_gap(start, floor, "Prometheus no longer held this data")).await?;
            self.note(|l| l.gaps += 1);
            state.covered_until = floor;
        }
        let first = (state.covered_until - OVERLAP_SECS).max(floor);
        let mut report = PullReport { start: first, end: first, series: 0 };
        let mut from = first;
        while from < to {
            let end = (from + CHUNK_SECS).min(to);
            let Some(raw) = self.fetch(prom, from, end).await else {
                self.with_file(move |f| f.log_pull(now, from, end, 0, Some("Prometheus didn't answer"))).await?;
                self.note(|l| l.last_error = Some("Prometheus didn't answer".into()));
                return Err(Error::Message("Prometheus didn't answer".into()));
            };
            let batch = build_batch(&raw, &mut state.memory);
            let series = batch.usage.iter().map(|(k, _)| k).collect::<HashSet<_>>().len();
            self.with_file(move |f| {
                f.write(&batch)?;
                f.log_pull(now, from, end, series, None)
            })
            .await?;
            state.covered_until = state.covered_until.max(end);
            report.end = end;
            report.series = report.series.max(series);
            from = end;
        }
        let covered = state.covered_until;
        self.note(|l| {
            l.last_pull = Some(now as f64);
            l.last_error = None;
            l.data_end = l.data_end.max(covered);
        });
        Ok(report)
    }

    /// Everything for `[start, end)` from every selector, or `None` if any
    /// query failed (the next pull asks again).
    async fn fetch(&self, prom: &Prometheus, start: i64, end: i64) -> Option<Raw> {
        let mut raw = Raw::default();
        for sel in &self.selectors {
            let part = fetch_selector(&self.client, prom, sel, start, end).await?;
            raw.extend(part);
        }
        Some(raw)
    }

    /// Samples running pods in scope that Prometheus isn't reporting, and
    /// writes every finished minute.
    pub async fn sample_local(&self) -> Result<usize> {
        self.sample_local_at(unix_now() as i64).await
    }

    async fn sample_local_at(&self, now: i64) -> Result<usize> {
        let mut state = self.state.lock().await;
        if state.local.listed_at.is_none_or(|t| now - t >= LOCAL_LIST_EVERY_SECS) {
            match self.uncovered_pods(now).await {
                Ok(pods) => {
                    state.local.pods = pods;
                    state.local.listed_at = Some(now);
                }
                Err(e) => tracing::warn!(error = %e, "listing pods for the trace failed"),
            }
        }
        let pods = state.local.pods.clone();
        let usage = if pods.is_empty() { HashMap::new() } else { local_usage(&self.client, &pods).await };
        let bucket = align_down(now);
        for ((namespace, pod), containers) in &usage {
            for (container, (cpu, memory)) in containers {
                let key = ContainerKey::new(namespace, pod, container);
                state.local.pending.entry(key).or_default().entry(bucket).or_default().add(*cpu, *memory);
            }
        }
        let sampled = usage.len();
        let mut batch = local_batch(&mut state, bucket);
        // Requests, limits and owners for the pods sampled.
        for pod in &pods {
            if !usage.contains_key(&(pod.namespace.clone(), pod.name.clone())) {
                continue;
            }
            if let Some(owner) = &pod.owner {
                batch.owners.push(((pod.namespace.clone(), pod.name.clone()), owner.clone()));
            }
            for (container, r) in &pod.resources {
                let key = ContainerKey::new(&pod.namespace, &pod.name, container);
                if remember_resources(&mut state.memory, &key, bucket, *r) {
                    batch.resources.push((key, bucket, *r));
                }
            }
        }
        drop(state);
        let written_until = batch.usage.iter().map(|(_, b)| b.t + BUCKET_SECS).max();
        self.with_file(move |f| f.write(&batch)).await?;
        self.note(|l| {
            l.sampled_pods = sampled as u32;
            if let Some(t) = written_until {
                l.data_end = l.data_end.max(t);
            }
        });
        Ok(sampled)
    }

    /// Running pods in scope, less those Prometheus has reported in the last
    /// few minutes.
    async fn uncovered_pods(&self, now: i64) -> Result<Vec<LocalPod>> {
        let pods = scope_pods(&self.client, &self.scope).await?;
        let covered = match &self.prometheus {
            Some(prom) => covered_pods(&self.client, prom, &self.selectors, now).await,
            None => Some(HashSet::new()),
        };
        // If Prometheus didn't answer, sample everything; its data replaces
        // the app's when a later pull gets it.
        let covered = covered.unwrap_or_default();
        Ok(pods.into_iter().filter(|p| !covered.contains(&(p.namespace.clone(), p.name.clone()))).collect())
    }

    async fn with_file<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut TraceFile) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let file = self.file.clone();
        tokio::task::spawn_blocking(move || f(&mut file.lock().unwrap()))
            .await
            .map_err(|e| Error::Message(e.to_string()))?
    }
}

fn validate(scope: &TraceScope) -> Result<()> {
    let empty = match scope {
        TraceScope::Cluster => false,
        TraceScope::Namespaces(ns) => ns.is_empty(),
        TraceScope::Workloads(w) => w.is_empty(),
        TraceScope::Pod { namespace, name } => namespace.is_empty() || name.is_empty(),
    };
    if empty {
        return Err(Error::Message("nothing to trace: choose at least one target".into()));
    }
    if let TraceScope::Workloads(ws) = scope
        && let Some(w) = ws.iter().find(|w| !WORKLOAD_KINDS.contains(&w.kind.kind.as_str()))
    {
        return Err(Error::Message(format!("can't trace a {}", w.kind.kind)));
    }
    Ok(())
}

const WORKLOAD_KINDS: [&str; 5] = ["Deployment", "StatefulSet", "DaemonSet", "Job", "CronJob"];

// --- Prometheus ---------------------------------------------------------------

/// Label matchers for one part of the scope.
#[derive(Debug, Clone, PartialEq)]
struct Selector {
    /// Namespace and pod matchers.
    pods: String,
    /// The namespace matcher alone, for ReplicaSet and Job owners.
    namespaces: String,
}

fn selectors(scope: &TraceScope) -> Vec<Selector> {
    match scope {
        TraceScope::Cluster => vec![Selector { pods: String::new(), namespaces: String::new() }],
        TraceScope::Namespaces(ns) => {
            let m = format!(r#"namespace=~"{}""#, ns.iter().map(|n| regex_escape(n)).collect::<Vec<_>>().join("|"));
            vec![Selector { pods: m.clone(), namespaces: m }]
        }
        TraceScope::Workloads(ws) => {
            let mut by_ns: BTreeMap<&str, Vec<&TraceWorkload>> = BTreeMap::new();
            for w in ws {
                by_ns.entry(&w.namespace).or_default().push(w);
            }
            by_ns
                .into_iter()
                .map(|(ns, ws)| {
                    let names: Vec<String> =
                        ws.iter().map(|w| format!("({})", crate::workload::pod_name_regex(&w.kind, &w.name))).collect();
                    let namespaces = format!(r#"namespace="{ns}""#);
                    Selector { pods: format!(r#"{namespaces},pod=~"{}""#, names.join("|")), namespaces }
                })
                .collect()
        }
        TraceScope::Pod { namespace, name } => {
            let namespaces = format!(r#"namespace="{namespace}""#);
            vec![Selector { pods: format!(r#"{namespaces},pod="{name}""#), namespaces }]
        }
    }
}

/// Kubernetes names only contain `.` that regexes care about; PromQL
/// strings need the backslash doubled.
fn regex_escape(name: &str) -> String {
    name.replace('.', r"\\.")
}

fn matchers(parts: &[&str]) -> String {
    parts.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(",")
}

/// Raw query results for one span.
#[derive(Default, Debug)]
struct Raw {
    /// Millicores, every [`RAW_STEP_SECS`].
    cpu: Vec<(ContainerKey, Points)>,
    memory: Vec<(ContainerKey, Points)>,
    /// Share of CFS periods throttled.
    throttled: Vec<(ContainerKey, Points)>,
    /// (key, resource) → cores or bytes, every minute.
    requests: Labelled,
    limits: Labelled,
    restarts: Vec<(ContainerKey, Points)>,
    /// (key, reason) → 1 while it's the last termination reason.
    reasons: Labelled,
    oom: Vec<(ContainerKey, Points)>,
    /// (namespace, pod) → (owner kind, owner name).
    pod_owners: OwnerMap,
    /// (namespace, ReplicaSet) → its owner, and the same for Jobs.
    rs_owners: OwnerMap,
    job_owners: OwnerMap,
}

impl Raw {
    fn extend(&mut self, other: Raw) {
        self.cpu.extend(other.cpu);
        self.memory.extend(other.memory);
        self.throttled.extend(other.throttled);
        self.requests.extend(other.requests);
        self.limits.extend(other.limits);
        self.restarts.extend(other.restarts);
        self.reasons.extend(other.reasons);
        self.oom.extend(other.oom);
        self.pod_owners.extend(other.pod_owners);
        self.rs_owners.extend(other.rs_owners);
        self.job_owners.extend(other.job_owners);
    }
}

async fn fetch_selector(client: &Client, prom: &Prometheus, sel: &Selector, start: i64, end: i64) -> Option<Raw> {
    let c = matchers(&[&sel.pods, r#"container!="",container!="POD""#]);
    let by = "namespace, pod, container";
    let cpu = format!("max by ({by}) (rate(container_cpu_usage_seconds_total{{{c}}}[2m])) * 1000");
    let memory = format!("max by ({by}) (container_memory_working_set_bytes{{{c}}})");
    let throttled = format!(
        "max by ({by}) (rate(container_cpu_cfs_throttled_periods_total{{{c}}}[2m])) \
         / max by ({by}) (rate(container_cpu_cfs_periods_total{{{c}}}[2m]))"
    );
    let resources = |metric: &str| {
        let m = matchers(&[&c, r#"resource=~"cpu|memory""#]);
        format!("max by ({by}, resource) ({metric}{{{m}}})")
    };
    let requests = resources("kube_pod_container_resource_requests");
    let limits = resources("kube_pod_container_resource_limits");
    let restarts = format!("max by ({by}) (kube_pod_container_status_restarts_total{{{c}}})");
    let reasons = format!("max by ({by}, reason) (kube_pod_container_status_last_terminated_reason{{{c}}})");
    let oom = format!("max by ({by}) (container_oom_events_total{{{c}}})");
    // Owners of every pod seen in the span, not just the ones alive at its end.
    let span = end - start + STATE_STEP_SECS;
    let owners = |metric: &str, sel: &str, name: &str| {
        format!("max by (namespace, {name}, owner_kind, owner_name) (max_over_time({metric}{{{sel}}}[{span}s]))")
    };
    let pod_owners = owners("kube_pod_owner", &sel.pods, "pod");
    let rs_owners = owners("kube_replicaset_owner", &sel.namespaces, "replicaset");
    let job_owners = owners("kube_job_owner", &sel.namespaces, "job_name");

    let raw_end = end - RAW_STEP_SECS;
    let state_end = end - STATE_STEP_SECS;
    let range = |q: &String, step: i64, end: i64| query_range(client, prom, q.clone(), start, end, step);
    let instant = |q: &String| query_instant(client, prom, q.clone(), end - STATE_STEP_SECS);
    let (cpu, memory, throttled, requests, limits, restarts, reasons, oom) = tokio::join!(
        range(&cpu, RAW_STEP_SECS, raw_end),
        range(&memory, RAW_STEP_SECS, raw_end),
        range(&throttled, RAW_STEP_SECS, raw_end),
        range(&requests, STATE_STEP_SECS, state_end),
        range(&limits, STATE_STEP_SECS, state_end),
        range(&restarts, STATE_STEP_SECS, state_end),
        range(&reasons, STATE_STEP_SECS, state_end),
        range(&oom, STATE_STEP_SECS, state_end),
    );
    let (pod_owners, rs_owners, job_owners) = tokio::join!(instant(&pod_owners), instant(&rs_owners), instant(&job_owners));
    let keyed = |data: &Value| -> Vec<(ContainerKey, Points)> {
        series(data, &POD_KEY).into_iter().map(|(l, p)| (ContainerKey::new(&l[0], &l[1], &l[2]), p)).collect()
    };
    let keyed_by = |data: &Value, extra: &str| -> Labelled {
        series(data, &["namespace", "pod", "container", extra])
            .into_iter()
            .map(|(l, p)| ((ContainerKey::new(&l[0], &l[1], &l[2]), l[3].clone()), p))
            .collect()
    };
    let owner_map = |data: &Value, name: &str| -> OwnerMap {
        series(data, &["namespace", name, "owner_kind", "owner_name"])
            .into_iter()
            .map(|(l, _)| ((l[0].clone(), l[1].clone()), (l[2].clone(), l[3].clone())))
            .collect()
    };
    Some(Raw {
        cpu: keyed(&cpu?),
        memory: keyed(&memory?),
        throttled: keyed(&throttled?),
        requests: keyed_by(&requests?, "resource"),
        limits: keyed_by(&limits?, "resource"),
        restarts: keyed(&restarts?),
        reasons: keyed_by(&reasons?, "reason"),
        oom: keyed(&oom?),
        pod_owners: owner_map(&pod_owners?, "pod"),
        rs_owners: owner_map(&rs_owners?, "replicaset"),
        job_owners: owner_map(&job_owners?, "job_name"),
    })
}

async fn query_range(client: &Client, prom: &Prometheus, query: String, start: i64, end: i64, step: i64) -> Option<Value> {
    if end < start {
        return Some(serde_json::json!({"result": []}));
    }
    let (start, end, step) = (start.to_string(), end.to_string(), step.to_string());
    let params = [("query", query.as_str()), ("start", &start), ("end", &end), ("step", &step)];
    prom.get(client, "query_range", &params).await
}

async fn query_instant(client: &Client, prom: &Prometheus, query: String, time: i64) -> Option<Value> {
    let time = time.to_string();
    prom.get(client, "query", &[("query", query.as_str()), ("time", &time)]).await
}

/// Each result's `labels` values and its points; instant vectors give one
/// point. Points that aren't finite numbers are dropped.
fn series(data: &Value, labels: &[&str]) -> Vec<(Vec<String>, Points)> {
    let point = |p: &Value| {
        let t = p[0].as_f64()?.round() as i64;
        let v = p[1].as_str()?.parse::<f64>().ok().filter(|v| v.is_finite())?;
        Some((t, v))
    };
    data["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let names = labels.iter().map(|l| r["metric"][l].as_str().map(str::to_owned)).collect::<Option<Vec<_>>>()?;
            let points = match r["values"].as_array() {
                Some(values) => values.iter().filter_map(point).collect(),
                None => point(&r["value"]).into_iter().collect(),
            };
            Some((names, points))
        })
        .collect()
}

/// Prometheus' `storage.tsdb.retention.time`, in seconds.
async fn retention(client: &Client, prom: &Prometheus) -> Option<f64> {
    let flags = prom.get(client, "status/flags", &[]).await?;
    parse_duration(flags["storage.tsdb.retention.time"].as_str()?).filter(|s| *s > 0.0)
}

/// A Prometheus duration such as `2h`, `15d` or `1h30m`.
fn parse_duration(s: &str) -> Option<f64> {
    let mut total = 0.0;
    let mut rest = s.trim();
    if rest.is_empty() {
        return None;
    }
    while !rest.is_empty() {
        let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        let n: f64 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let unit = rest.find(|c: char| c.is_ascii_digit()).unwrap_or(rest.len());
        let secs = match &rest[..unit] {
            "ms" => 0.001,
            "s" => 1.0,
            "m" => 60.0,
            "h" => 3600.0,
            "d" => 86400.0,
            "w" => 7.0 * 86400.0,
            "y" => 365.0 * 86400.0,
            _ => return None,
        };
        total += n * secs;
        rest = &rest[unit..];
    }
    Some(total)
}

/// Pods with cAdvisor memory data in the last five minutes.
async fn covered_pods(client: &Client, prom: &Prometheus, selectors: &[Selector], now: i64) -> Option<HashSet<(String, String)>> {
    let mut covered = HashSet::new();
    for sel in selectors {
        let c = matchers(&[&sel.pods, r#"container!="",container!="POD""#]);
        let q = format!("count by (namespace, pod) (container_memory_working_set_bytes{{{c}}})");
        let data = query_instant(client, prom, q, now).await?;
        covered.extend(series(&data, &["namespace", "pod"]).into_iter().map(|(l, _)| (l[0].clone(), l[1].clone())));
    }
    Some(covered)
}

// --- Turning results into rows ------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Acc {
    min: f64,
    max: f64,
    sum: f64,
    n: u32,
}

impl Default for Acc {
    fn default() -> Self {
        Self { min: f64::INFINITY, max: f64::NEG_INFINITY, sum: 0.0, n: 0 }
    }
}

impl Acc {
    fn push(&mut self, v: f64) {
        self.min = self.min.min(v);
        self.max = self.max.max(v);
        self.sum += v;
        self.n += 1;
    }

    fn stat(&self) -> Option<Stat> {
        (self.n > 0).then(|| Stat { min: self.min, avg: self.sum / f64::from(self.n), max: self.max })
    }
}

/// Local samples: CPU and memory accumulate side by side.
#[derive(Debug, Default, Clone, Copy)]
struct PairAcc {
    cpu: Acc,
    memory: Acc,
}

type Pending = BTreeMap<i64, PairAcc>;

impl PairAcc {
    fn add(&mut self, cpu: f64, memory: f64) {
        self.cpu.push(cpu);
        self.memory.push(memory);
    }
}

fn align_down(t: i64) -> i64 {
    t.div_euclid(BUCKET_SECS) * BUCKET_SECS
}

fn align_up(t: i64) -> i64 {
    align_down(t + BUCKET_SECS - 1)
}

/// Raw points folded into minutes.
fn buckets(points: &[(i64, f64)]) -> BTreeMap<i64, Acc> {
    let mut out: BTreeMap<i64, Acc> = BTreeMap::new();
    for &(t, v) in points {
        out.entry(align_down(t)).or_default().push(v);
    }
    out
}

fn build_batch(raw: &Raw, memory: &mut Memory) -> Batch {
    let mut batch = Batch { source: Some(Source::Prometheus), ..Batch::default() };

    // Usage: CPU, memory and throttling per container per minute.
    let mut minutes: BTreeMap<ContainerKey, BTreeMap<i64, Bucket>> = BTreeMap::new();
    let mut fold = |series: &[(ContainerKey, Points)], set: fn(&mut Bucket, &Acc)| {
        for (key, points) in series {
            for (t, acc) in buckets(points) {
                let b = minutes
                    .entry(key.clone())
                    .or_default()
                    .entry(t)
                    .or_insert(Bucket { t, cpu: None, memory: None, throttled: None });
                set(b, &acc);
            }
        }
    };
    fold(&raw.cpu, |b, a| b.cpu = a.stat());
    fold(&raw.memory, |b, a| b.memory = a.stat());
    fold(&raw.throttled, |b, a| b.throttled = a.stat().map(|s| s.max));
    for (key, by_t) in minutes {
        // Throttling alone (no usage) isn't worth a row.
        for b in by_t.into_values().filter(|b| b.cpu.is_some() || b.memory.is_some()) {
            batch.usage.push((key.clone(), b));
        }
    }

    // Requests and limits, as change points.
    let mut resources: BTreeMap<ContainerKey, BTreeMap<i64, Resources>> = BTreeMap::new();
    for (list, is_limit) in [(&raw.requests, false), (&raw.limits, true)] {
        for ((key, resource), points) in list {
            for &(t, v) in points {
                let r = resources.entry(key.clone()).or_default().entry(t).or_default();
                match (resource.as_str(), is_limit) {
                    ("cpu", false) => r.cpu_request = Some(v * 1000.0),
                    ("cpu", true) => r.cpu_limit = Some(v * 1000.0),
                    ("memory", false) => r.mem_request = Some(v),
                    ("memory", true) => r.mem_limit = Some(v),
                    _ => {}
                }
            }
        }
    }
    for (key, by_t) in resources {
        for (t, r) in by_t {
            if remember_resources(memory, &key, t, r) {
                batch.resources.push((key.clone(), t, r));
            }
        }
    }

    // Restarts (with the termination reason) and OOM kills.
    for (list, kind) in [(&raw.restarts, EventKind::Restart), (&raw.oom, EventKind::Oom)] {
        for (key, points) in list {
            let last = memory.counters.get(&(key.clone(), kind)).copied();
            let (events, newest) = increases(points, last);
            if let Some(newest) = newest {
                memory.counters.insert((key.clone(), kind), newest);
            }
            for (t, count) in events {
                let reason = (kind == EventKind::Restart).then(|| reason_at(&raw.reasons, key, t)).flatten();
                batch.events.push((key.clone(), Event { t, kind, reason, count }));
            }
        }
    }

    // Owners, resolved through ReplicaSets and Jobs.
    for ((namespace, pod), (kind, name)) in &raw.pod_owners {
        let owner = resolve_owner(namespace, pod, kind, name, &raw.rs_owners, &raw.job_owners);
        batch.owners.push(((namespace.clone(), pod.clone()), owner));
    }
    batch.owners.sort_by(|a, b| a.0.cmp(&b.0));
    batch
}

/// Records `r` as the container's latest requests and limits at `t`, and
/// says whether it's a change worth writing. Points older than the last one
/// seen (an overlapping pull) are ignored.
fn remember_resources(memory: &mut Memory, key: &ContainerKey, t: i64, r: Resources) -> bool {
    match memory.resources.get(key) {
        Some((last_t, _)) if t <= *last_t => false,
        Some((_, last)) if *last == r => {
            memory.resources.insert(key.clone(), (t, r));
            false
        }
        _ => {
            memory.resources.insert(key.clone(), (t, r));
            true
        }
    }
}

/// Rises in a counter after `last` (time, value): (time, amount) each, and
/// the newest point. The first point ever seen sets the baseline; a drop is
/// a reset, not an event.
fn increases(points: &[Point], last: Option<Point>) -> (Vec<(i64, i64)>, Option<Point>) {
    let mut prev = last;
    let mut events = Vec::new();
    for &(t, v) in points {
        match prev {
            Some((pt, _)) if t <= pt => continue,
            Some((_, pv)) if v > pv => events.push((t, (v - pv).round() as i64)),
            _ => {}
        }
        prev = Some((t, v));
    }
    (events, prev)
}

/// The container's last termination reason at `t` (or within a minute after).
fn reason_at(reasons: &Labelled, key: &ContainerKey, t: i64) -> Option<String> {
    reasons
        .iter()
        .filter(|((k, _), _)| k == key)
        .find(|(_, points)| points.iter().any(|&(pt, v)| v > 0.0 && pt >= t && pt <= t + STATE_STEP_SECS))
        .map(|((_, reason), _)| reason.clone())
}

fn resolve_owner(
    namespace: &str,
    pod: &str,
    kind: &str,
    name: &str,
    rs_owners: &OwnerMap,
    job_owners: &OwnerMap,
) -> Owner {
    let key = (namespace.to_owned(), name.to_owned());
    let found = |map: &OwnerMap| {
        map.get(&key).filter(|(k, n)| k != "<none>" && !n.is_empty() && n != "<none>").cloned()
    };
    match kind {
        "ReplicaSet" => match found(rs_owners) {
            Some((k, n)) => {
                let revision = name.strip_prefix(&format!("{n}-")).map(str::to_owned);
                Owner { kind: k, name: n, revision }
            }
            None => Owner { kind: kind.into(), name: name.into(), revision: None },
        },
        "Job" => match found(job_owners) {
            Some((k, n)) => Owner { kind: k, name: n, revision: None },
            None => Owner { kind: kind.into(), name: name.into(), revision: None },
        },
        "<none>" | "" => Owner { kind: "Pod".into(), name: pod.into(), revision: None },
        _ => Owner { kind: kind.into(), name: name.into(), revision: None },
    }
}

// --- The app's own samples ----------------------------------------------------

/// Finished minutes (before `before`) of local samples, as a batch.
fn local_batch(state: &mut State, before: i64) -> Batch {
    let mut batch = Batch { source: Some(Source::Local), ..Batch::default() };
    for (key, pending) in state.local.pending.iter_mut() {
        let done: Vec<i64> = pending.range(..before).map(|(t, _)| *t).collect();
        for t in done {
            let acc = pending.remove(&t).unwrap_or_default();
            batch.usage.push((key.clone(), Bucket { t, cpu: acc.cpu.stat(), memory: acc.memory.stat(), throttled: None }));
        }
    }
    state.local.pending.retain(|_, p| !p.is_empty());
    batch.usage.sort_by(|a, b| (&a.0, a.1.t).cmp(&(&b.0, b.1.t)));
    batch
}

/// Running pods in scope, from the API.
async fn scope_pods(client: &Client, scope: &TraceScope) -> Result<Vec<LocalPod>> {
    let lp = ListParams::default().fields("status.phase=Running");
    let pods: Vec<Pod> = match scope {
        TraceScope::Cluster => Api::<Pod>::all(client.clone()).list(&lp).await?.items,
        TraceScope::Namespaces(namespaces) => {
            let mut all = Vec::new();
            for ns in namespaces {
                all.extend(Api::<Pod>::namespaced(client.clone(), ns).list(&lp).await?.items);
            }
            all
        }
        TraceScope::Workloads(workloads) => {
            let mut namespaces: Vec<&str> = workloads.iter().map(|w| w.namespace.as_str()).collect();
            namespaces.sort_unstable();
            namespaces.dedup();
            let mut all = Vec::new();
            for ns in namespaces {
                let pods = Api::<Pod>::namespaced(client.clone(), ns).list(&lp).await?.items;
                all.extend(pods.into_iter().filter(|p| {
                    let owner = pod_owner(p);
                    workloads.iter().any(|w| w.namespace == ns && owned_by(&owner, w))
                }));
            }
            all
        }
        TraceScope::Pod { namespace, name } => {
            match Api::<Pod>::namespaced(client.clone(), namespace).get_opt(name).await? {
                Some(p) if p.status.as_ref().and_then(|s| s.phase.as_deref()) == Some("Running") => vec![p],
                _ => Vec::new(),
            }
        }
    };
    Ok(pods.iter().map(local_pod).collect())
}

fn local_pod(p: &Pod) -> LocalPod {
    let spec = p.spec.as_ref();
    let resources = spec
        .map(|s| s.containers.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|c| {
            let r = c.resources.as_ref();
            let get = |limits: bool, key: &str| {
                let map = if limits { r.and_then(|r| r.limits.as_ref()) } else { r.and_then(|r| r.requests.as_ref()) };
                map.and_then(|m| m.get(key)).map(|q| q.0.clone())
            };
            let res = Resources {
                cpu_request: get(false, "cpu").map(|q| millicores(&q)),
                cpu_limit: get(true, "cpu").map(|q| millicores(&q)),
                mem_request: get(false, "memory").map(|q| bytes(&q)),
                mem_limit: get(true, "memory").map(|q| bytes(&q)),
            };
            (c.name.clone(), res)
        })
        .collect();
    LocalPod {
        namespace: p.namespace().unwrap_or_default(),
        name: p.name_any(),
        node: spec.and_then(|s| s.node_name.clone()),
        owner: Some(pod_owner(p)),
        resources,
    }
}

/// The pod's workload as far as the pod alone tells: a ReplicaSet with the
/// pod's template hash belongs to a Deployment of the same prefix.
fn pod_owner(p: &Pod) -> Owner {
    let Some(o) = p.owner_references().iter().find(|o| o.controller == Some(true)) else {
        return Owner { kind: "Pod".into(), name: p.name_any(), revision: None };
    };
    if o.kind == "ReplicaSet"
        && let Some(hash) = p.labels().get("pod-template-hash")
        && let Some(deployment) = o.name.strip_suffix(&format!("-{hash}"))
    {
        return Owner { kind: "Deployment".into(), name: deployment.into(), revision: Some(hash.clone()) };
    }
    Owner { kind: o.kind.clone(), name: o.name.clone(), revision: None }
}

fn owned_by(owner: &Owner, w: &TraceWorkload) -> bool {
    match w.kind.kind.as_str() {
        // The pod's owner is a Job named <cronjob>-<schedule> or <cronjob>-manual-<x>.
        "CronJob" => owner.kind == "Job" && owner.name.starts_with(&format!("{}-", w.name)),
        kind => owner.kind == kind && owner.name == w.name,
    }
}

/// (namespace, pod) → container → (cpu millicores, memory bytes), from
/// metrics-server or, without it, each pod's kubelet. Pods neither reports
/// are left out.
async fn local_usage(client: &Client, pods: &[LocalPod]) -> HashMap<(String, String), HashMap<String, (f64, f64)>> {
    let wanted: HashSet<(String, String)> = pods.iter().map(|p| (p.namespace.clone(), p.name.clone())).collect();
    let mut namespaces: Vec<&str> = pods.iter().map(|p| p.namespace.as_str()).collect();
    namespaces.sort_unstable();
    namespaces.dedup();
    let mut out = HashMap::new();
    let mut metrics_server = true;
    for ns in &namespaces {
        match crate::pod_details::pod_metrics_api(client, ns).list(&ListParams::default()).await {
            Ok(list) => {
                for m in &list.items {
                    let key = (ns.to_string(), m.name_any());
                    if wanted.contains(&key)
                        && let Some(c) = crate::pod_details::metrics_containers(m)
                    {
                        out.insert(key, c);
                    }
                }
            }
            Err(_) => {
                metrics_server = false;
                break;
            }
        }
    }
    if metrics_server {
        return out;
    }
    let mut nodes: Vec<String> = pods.iter().filter_map(|p| p.node.clone()).collect();
    nodes.sort_unstable();
    nodes.dedup();
    let summaries: Vec<(String, Option<Value>)> = stream::iter(nodes)
        .map(|n| async move {
            let summary = crate::usage::kubelet_summary(client, &n).await;
            (n, summary)
        })
        .buffer_unordered(KUBELET_CONCURRENCY)
        .collect()
        .await;
    let summaries: HashMap<String, Value> = summaries.into_iter().filter_map(|(n, s)| Some((n, s?))).collect();
    pods.iter()
        .filter_map(|p| {
            let summary = summaries.get(p.node.as_deref()?)?;
            let usage = crate::usage::pod_usage(summary, &p.namespace, &p.name)?;
            Some(((p.namespace.clone(), p.name.clone()), usage))
        })
        .collect()
}

/// What can be traced in a namespace: its workloads (Jobs a CronJob made
/// are left out; trace the CronJob) and its running pods, each sorted by name.
pub async fn targets(client: &Client, namespace: &str) -> Result<TraceTargets> {
    use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
    use k8s_openapi::api::batch::v1::{CronJob, Job};
    let (deployments, statefulsets, daemonsets, cronjobs, jobs, pods) = (
        Api::<Deployment>::namespaced(client.clone(), namespace),
        Api::<StatefulSet>::namespaced(client.clone(), namespace),
        Api::<DaemonSet>::namespaced(client.clone(), namespace),
        Api::<CronJob>::namespaced(client.clone(), namespace),
        Api::<Job>::namespaced(client.clone(), namespace),
        Api::<Pod>::namespaced(client.clone(), namespace),
    );
    let (lp, running) = (ListParams::default(), ListParams::default().fields("status.phase=Running"));
    let (deployments, statefulsets, daemonsets, cronjobs, jobs, pods) = tokio::join!(
        deployments.list_metadata(&lp),
        statefulsets.list_metadata(&lp),
        daemonsets.list_metadata(&lp),
        cronjobs.list_metadata(&lp),
        jobs.list_metadata(&lp),
        pods.list_metadata(&running),
    );
    let names = |items: Vec<kube::core::ObjectMeta>| -> Vec<String> { items.into_iter().filter_map(|m| m.name).collect() };
    let mut workloads = Vec::new();
    let mut add = |kind: KindRef, names: Vec<String>| {
        workloads.extend(names.into_iter().map(|name| TraceWorkload { kind: kind.clone(), namespace: namespace.to_owned(), name }));
    };
    add(workload_kind("apps", "Deployment", "deployments"), names(deployments?.items.into_iter().map(|o| o.metadata).collect()));
    add(workload_kind("apps", "StatefulSet", "statefulsets"), names(statefulsets?.items.into_iter().map(|o| o.metadata).collect()));
    add(workload_kind("apps", "DaemonSet", "daemonsets"), names(daemonsets?.items.into_iter().map(|o| o.metadata).collect()));
    add(workload_kind("batch", "CronJob", "cronjobs"), names(cronjobs?.items.into_iter().map(|o| o.metadata).collect()));
    let standalone = jobs?
        .items
        .into_iter()
        .map(|o| o.metadata)
        .filter(|m| !m.owner_references.iter().flatten().any(|o| o.kind == "CronJob"))
        .collect();
    add(workload_kind("batch", "Job", "jobs"), names(standalone));
    workloads.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.kind.kind.cmp(&b.kind.kind)));
    let mut pods = names(pods?.items.into_iter().map(|o| o.metadata).collect());
    pods.sort();
    Ok(TraceTargets { workloads, pods })
}

fn workload_kind(group: &str, kind: &str, plural: &str) -> KindRef {
    KindRef { group: group.into(), version: "v1".into(), kind: kind.into(), plural: plural.into(), namespaced: true }
}

/// Opens a trace file and reads its header.
pub async fn read_summary(path: &Path) -> Result<TraceSummary> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || TraceFile::open(&path)?.summary(false))
        .await
        .map_err(|e| Error::Message(e.to_string()))?
}

fn unix_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

#[cfg(test)]
#[path = "trace_tests.rs"]
mod tests;
