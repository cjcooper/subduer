//! Data passed between the Tauri backend and the Leptos UI.
//!
//! Everything here is plain serde data so it compiles for both the native
//! backend and `wasm32-unknown-unknown`.

use serde::{Deserialize, Serialize};

/// A kubeconfig context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextInfo {
    pub name: String,
    pub cluster: Option<String>,
    pub namespace: Option<String>,
    /// Name looks like a production cluster; unlocking needs extra confirmation.
    pub prod_like: bool,
    /// Mutations are allowed against this context.
    pub unlocked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextList {
    pub contexts: Vec<ContextInfo>,
    /// `current-context` from the kubeconfig, if set.
    pub current: Option<String>,
}

/// Identifies a resource type on the API server.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KindRef {
    /// Empty for the core API group.
    pub group: String,
    pub version: String,
    pub kind: String,
    pub plural: String,
    pub namespaced: bool,
}

impl KindRef {
    pub fn api_version(&self) -> String {
        if self.group.is_empty() {
            self.version.clone()
        } else {
            format!("{}/{}", self.group, self.version)
        }
    }

    /// Stable key, e.g. `apps/Deployment` or `core/Pod`.
    pub fn key(&self) -> String {
        let group = if self.group.is_empty() { "core" } else { &self.group };
        format!("{group}/{}", self.kind)
    }
}

/// Where a kind sits in the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Category {
    /// Listed at the top of the sidebar, outside any section.
    TopLevel,
    Workloads,
    Config,
    Network,
    Storage,
    Namespaces,
    AccessControl,
    /// Helm releases (a virtual kind read from Helm's release Secrets).
    Helm,
    /// Custom resources from AWS controllers (load balancer, VPC CNI, ACK, …).
    Aws,
    /// Custom resources from GKE and Google Cloud (Config Connector, …).
    GoogleCloud,
    /// Custom resources from AKS and Azure (Azure Service Operator, …).
    Azure,
    /// Kinds added to the cluster (CRDs and aggregated APIs).
    CustomResources,
    /// Built-in kinds that don't fit a section above.
    Other,
}

impl Category {
    /// Sidebar sections in default order (excludes `TopLevel`).
    pub const SECTIONS: [Category; 12] = [
        Self::Workloads,
        Self::Config,
        Self::Network,
        Self::Storage,
        Self::Namespaces,
        Self::AccessControl,
        Self::Helm,
        Self::Aws,
        Self::GoogleCloud,
        Self::Azure,
        Self::CustomResources,
        Self::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::TopLevel => "Top level",
            Self::Workloads => "Workloads",
            Self::Helm => "Helm",
            Self::Aws => "AWS",
            Self::GoogleCloud => "Google Cloud",
            Self::Azure => "Azure",
            Self::Config => "Config",
            Self::Network => "Network",
            Self::Storage => "Storage",
            Self::Namespaces => "Namespaces",
            Self::AccessControl => "Access Control",
            Self::CustomResources => "Custom Resources",
            Self::Other => "Other",
        }
    }
}

impl Category {
    /// The cloud section for a custom resource's API group, if it belongs to
    /// a cloud provider's controllers.
    pub fn cloud_for_group(group: &str) -> Option<Category> {
        let under = |domain: &str| group == domain || group.ends_with(&format!(".{domain}"));
        if under("k8s.aws") || under("amazonaws.com") {
            Some(Self::Aws)
        } else if under("gke.io") || under("cloud.google.com") || under("googleapis.com") {
            Some(Self::GoogleCloud)
        } else if under("azure.com") || under("azure.microsoft.com") || group == "aadpodidentity.k8s.io" {
            Some(Self::Azure)
        } else {
            None
        }
    }

    /// Sections whose kinds are listed under API group sub-headings.
    pub fn groups_by_api_group(self) -> bool {
        matches!(self, Self::CustomResources | Self::Aws | Self::GoogleCloud | Self::Azure)
    }
}

/// A listable resource type discovered on the cluster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindInfo {
    pub kind: KindRef,
    pub verbs: Vec<String>,
    pub category: Category,
}

/// Result of connecting to a context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterInfo {
    pub context: String,
    pub kinds: Vec<KindInfo>,
    pub namespaces: Vec<String>,
}

/// How the UI renders a column's cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColumnKind {
    Name,
    /// Hidden while the namespace filter selects a single namespace.
    Namespace,
    Text,
    /// Truncated to one line; the full text is shown on hover.
    LongText,
    /// Short status value styled by its content (e.g. `Warning`).
    Badge,
    /// Right-aligned.
    Number,
    /// RFC 3339 timestamp shown as a relative age (e.g. `5m`).
    Age,
    /// One icon per container; the cell is encoded with [`containers::encode`].
    Containers,
}

/// Per-container state carried in a [`ColumnKind::Containers`] cell.
pub mod containers {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum State {
        /// Running and ready.
        Running,
        /// Being created, or running but not ready yet.
        Starting,
        /// Not running, without an error (e.g. exited 0).
        Stopped,
        /// Not running because of an error (crash loop, non-zero exit, …).
        Failed,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Container {
        pub name: String,
        pub state: State,
        /// Reason shown on hover, e.g. `CrashLoopBackOff`.
        pub detail: String,
        /// An init container (listed before the app containers).
        pub init: bool,
    }

    const SEP: char = '\u{1f}';
    const FIELD: char = '\u{1e}';

    pub fn encode(containers: &[Container]) -> String {
        containers
            .iter()
            .map(|c| {
                let code = match c.state {
                    State::Running => 'R',
                    State::Starting => 'S',
                    State::Stopped => 'N',
                    State::Failed => 'E',
                };
                let init = if c.init { "i" } else { "" };
                format!("{code}{FIELD}{}{FIELD}{}{FIELD}{init}", c.name, c.detail)
            })
            .collect::<Vec<_>>()
            .join(&SEP.to_string())
    }

    pub fn decode(cell: &str) -> Vec<Container> {
        cell.split(SEP)
            .filter_map(|entry| {
                let mut fields = entry.split(FIELD);
                let state = match fields.next()? {
                    "R" => State::Running,
                    "S" => State::Starting,
                    "N" => State::Stopped,
                    "E" => State::Failed,
                    _ => return None,
                };
                Some(Container {
                    name: fields.next()?.to_owned(),
                    state,
                    detail: fields.next().unwrap_or_default().to_owned(),
                    init: fields.next() == Some("i"),
                })
            })
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn round_trip() {
            let list = vec![
                Container { name: "setup".into(), state: State::Stopped, detail: "Completed".into(), init: true },
                Container { name: "app".into(), state: State::Running, detail: String::new(), init: false },
                Container { name: "proxy".into(), state: State::Failed, detail: "CrashLoopBackOff".into(), init: false },
            ];
            assert_eq!(decode(&encode(&list)), list);
            assert!(decode("").is_empty());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Column {
    pub header: String,
    pub kind: ColumnKind,
}

/// One object rendered as a table row.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceRow {
    /// `namespace/name`, or `name` for cluster-scoped objects.
    pub id: String,
    pub name: String,
    pub namespace: Option<String>,
    /// One value per `ResourceTable::columns` entry.
    pub cells: Vec<String>,
}

/// Snapshot of a watched resource list, pushed to the UI on every change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceTable {
    pub watch_id: u64,
    pub columns: Vec<Column>,
    /// In display order.
    pub rows: Vec<ResourceRow>,
    /// Set when the watch failed; `rows` holds the last good state.
    pub error: Option<String>,
}

/// Tauri event names.
pub mod events {
    pub const RESOURCE_TABLE: &str = "resource-table";
}

/// Totals for one resource. CPU is in millicores, memory in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceTotals {
    pub capacity: f64,
    /// What pods can be scheduled against (capacity minus system reserve).
    pub allocatable: f64,
    /// Sum of requests of scheduled, non-terminated pods.
    pub requests: f64,
    pub limits: f64,
    /// Live usage from metrics-server; `None` when it isn't available.
    pub used: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSummary {
    pub name: String,
    pub ready: bool,
    pub schedulable: bool,
    pub roles: Vec<String>,
    /// e.g. `m6i.large`, or `fargate`.
    pub instance_type: Option<String>,
    pub kubelet_version: String,
    pub created: Option<String>,
    pub pods: usize,
    pub pod_capacity: u64,
    pub cpu: ResourceTotals,
    pub memory: ResourceTotals,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodCounts {
    pub running: usize,
    pub pending: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub unknown: usize,
}

impl PodCounts {
    pub fn total(&self) -> usize {
        self.running + self.pending + self.succeeded + self.failed + self.unknown
    }
}

/// Requests of non-terminated pods in one namespace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamespaceRequests {
    pub namespace: String,
    pub pods: usize,
    pub cpu: f64,
    pub memory: f64,
}

/// Cluster-wide summary for the Cluster page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClusterOverview {
    pub server_version: Option<String>,
    pub nodes: Vec<NodeSummary>,
    pub pods: PodCounts,
    pub cpu: ResourceTotals,
    pub memory: ResourceTotals,
    /// Sorted by memory requests, largest first.
    pub namespaces: Vec<NamespaceRequests>,
    /// Warning events seen in the last hour; `None` if events can't be read.
    pub warnings_last_hour: Option<usize>,
    pub metrics_available: bool,
    /// Where `used` figures came from; `None` when no source was readable.
    pub usage_source: Option<UsageSource>,
    /// Cluster-wide usage over the last hour, when Prometheus is available.
    pub history: Option<ClusterHistory>,
}

/// Where live usage came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UsageSource {
    MetricsServer,
    /// Prometheus cAdvisor metrics, with kubelet stats for nodes it doesn't cover.
    Prometheus,
    /// The kubelet's `/stats/summary` on each node, through the API server.
    Kubelet,
}

/// Sum of all containers' usage across the cluster over a time window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClusterHistory {
    /// Window bounds in unix seconds.
    pub start: f64,
    pub end: f64,
    pub step: f64,
    pub usage: UsageHistory,
}

/// Requests, limits and live usage for one resource. CPU in millicores,
/// memory in bytes. `None` means not set (or, for `used`, not available).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub requests: Option<f64>,
    pub limits: Option<f64>,
    pub used: Option<f64>,
    /// The node's allocatable amount: the real ceiling when there is no limit.
    #[serde(default)]
    pub node: Option<f64>,
}

/// Where an environment variable's value comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnvValue {
    Literal(String),
    /// Resolved from a ConfigMap; `value` is `None` if it couldn't be read.
    ConfigMap { name: String, key: String, value: Option<String> },
    /// Hidden until revealed with the `secret_value` command.
    Secret { name: String, key: String },
    /// `fieldRef`, e.g. `metadata.name`.
    Field(String),
    /// `resourceFieldRef`, e.g. `limits.memory`.
    Resource(String),
    /// An `envFrom` source whose keys couldn't be listed.
    Unreadable { source: String, error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVar {
    pub name: String,
    pub value: EnvValue,
    /// Set for variables that come from `envFrom`, e.g. `ConfigMap app-config`.
    pub from: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerDetails {
    pub name: String,
    pub init: bool,
    /// Init container with `restartPolicy: Always`: runs alongside the app.
    pub sidecar: bool,
    pub image: String,
    pub state: containers::State,
    pub detail: String,
    pub ready: bool,
    pub restarts: i64,
    pub started: Option<String>,
    /// e.g. `OOMKilled (exit 137) at 2026-10-09T12:00:00Z`.
    pub last_termination: Option<String>,
    pub ports: Vec<String>,
    pub cpu: Usage,
    pub memory: Usage,
    pub env: Vec<EnvVar>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSummary {
    pub type_: String,
    pub reason: String,
    pub message: String,
    pub count: i64,
    pub first_seen: String,
    pub last_seen: String,
    pub source: String,
    /// The object it's about, e.g. `Pod api-7d9f-x2k4q`, when a list mixes objects.
    #[serde(default)]
    pub object: Option<String>,
}

/// Everything shown in a selected pod's Details tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PodDetails {
    pub name: String,
    pub namespace: String,
    pub status: String,
    pub node: Option<String>,
    pub pod_ip: Option<String>,
    pub host_ip: Option<String>,
    pub qos_class: Option<String>,
    pub service_account: Option<String>,
    /// e.g. `ReplicaSet/api-7d9f`.
    pub owner: Option<String>,
    pub restart_policy: Option<String>,
    pub priority_class: Option<String>,
    pub created: Option<String>,
    pub labels: Vec<(String, String)>,
    pub cpu: Usage,
    pub memory: Usage,
    pub metrics_available: bool,
    /// Init containers first, then app containers, in spec order.
    pub containers: Vec<ContainerDetails>,
    /// Newest first.
    pub events: Vec<EventSummary>,
}

/// Everything shown in a selected Deployment's or StatefulSet's Details tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkloadDetails {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    /// e.g. `Available`, `Rolling out`, `Degraded`, `Paused`, `Scaled to 0`.
    pub status: String,
    pub replicas: ReplicaCounts,
    /// Kind-specific facts in display order, e.g. `("Strategy", "RollingUpdate …")`.
    pub info: Vec<(String, String)>,
    pub created: Option<String>,
    pub labels: Vec<(String, String)>,
    pub conditions: Vec<Condition>,
    /// Summed over the current pods; `node` is unused.
    pub cpu: Usage,
    pub memory: Usage,
    pub metrics_available: bool,
    pub pods: Vec<WorkloadPod>,
    /// The pod template's containers, init containers first. Usage is
    /// summed over the pods; requests and limits are per pod.
    pub containers: Vec<TemplateContainer>,
    /// The workload's, its ReplicaSets' and its pods' events, newest first.
    pub events: Vec<EventSummary>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaCounts {
    pub desired: i32,
    /// Pods that exist, any state.
    pub current: i32,
    pub ready: i32,
    /// Running the latest pod template.
    pub updated: i32,
    pub available: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    pub type_: String,
    pub status: String,
    pub reason: String,
    pub message: String,
    pub last_transition: Option<String>,
}

/// One pod of a workload, for its Pods table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkloadPod {
    pub name: String,
    pub status: String,
    /// Init containers first.
    pub containers: Vec<containers::Container>,
    /// `ready/total` app containers.
    pub ready: String,
    pub restarts: i64,
    pub node: Option<String>,
    pub created: Option<String>,
    /// Runs the latest pod template.
    pub updated: bool,
    pub cpu: Option<f64>,
    pub memory: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateContainer {
    pub name: String,
    pub init: bool,
    pub sidecar: bool,
    pub image: String,
    pub ports: Vec<String>,
    pub cpu: Usage,
    pub memory: Usage,
    pub env: Vec<EnvVar>,
}

/// Where a pod's usage history came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistorySource {
    /// Queried from the cluster's Prometheus; covers the whole window.
    Prometheus,
    /// Sampled by the app on each refresh, so it only reaches back to when
    /// the pod was first viewed.
    Local,
}

/// `(unix seconds, value)` points, oldest first. CPU in millicores, memory in bytes.
pub type Series = Vec<(f64, f64)>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageHistory {
    pub cpu: Series,
    pub memory: Series,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerHistory {
    pub name: String,
    pub usage: UsageHistory,
}

/// CPU and memory over the last hour for a pod and each of its containers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PodHistory {
    pub source: HistorySource,
    /// Window bounds in unix seconds.
    pub start: f64,
    pub end: f64,
    /// Expected spacing between points; larger gaps break the line.
    pub step: f64,
    /// Sum of all containers.
    pub pod: UsageHistory,
    pub containers: Vec<ContainerHistory>,
}

/// What a logs window shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogSource {
    /// One container of one pod.
    Container { pod: String, container: String },
    /// Every container of every pod a workload selects, merged.
    Workload { kind: KindRef, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogTarget {
    pub namespace: String,
    pub source: LogSource,
}

impl LogTarget {
    /// Window title, e.g. `api · shop/web-7d9f · Logs`.
    pub fn title(&self, context: &str) -> String {
        match &self.source {
            LogSource::Container { pod, container } => {
                format!("{container} · {}/{pod} · {context} · Logs", self.namespace)
            }
            LogSource::Workload { kind, name } => {
                format!("{} {}/{name} · {context} · Logs", kind.kind, self.namespace)
            }
        }
    }
}

/// Kinds whose pods can be found from a label selector, so they get a Logs button.
pub fn has_workload_logs(kind: &KindRef) -> bool {
    matches!(
        (kind.group.as_str(), kind.kind.as_str()),
        ("apps", "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet")
            | ("batch", "Job")
            | ("", "ReplicationController")
    )
}

/// Kinds that get Restart (a new rollout of the pod template) and Scale.
pub fn is_scalable_workload(kind: &KindRef) -> bool {
    matches!((kind.group.as_str(), kind.kind.as_str()), ("apps", "Deployment" | "StatefulSet"))
}

/// A workload's scale subresource: replicas asked for and replicas running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replicas {
    pub desired: i32,
    pub current: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    /// `pod/container` when several streams are merged.
    pub source: Option<String>,
    /// RFC 3339 from the kubelet, when present.
    pub time: Option<String>,
    pub text: String,
}

/// Lines and status messages pushed to one logs window.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogBatch {
    pub lines: Vec<LogLine>,
    /// Status for the window's footer, e.g. `streaming 3 containers` or an error.
    pub notices: Vec<String>,
}

/// Returned to a logs window when it starts streaming.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogSession {
    pub context: String,
    pub target: LogTarget,
    pub title: String,
    /// Event name this window's batches arrive on.
    pub event: String,
}

/// A container to open a shell in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecTarget {
    pub namespace: String,
    pub pod: String,
    pub container: String,
}

impl ExecTarget {
    /// Window title, e.g. `app · shop/web-7d9f · acme-uat · Shell`.
    pub fn title(&self, context: &str) -> String {
        format!("{} · {}/{} · {context} · Shell", self.container, self.namespace, self.pod)
    }
}

/// Pushed to a shell window: terminal output (base64, since output can split
/// UTF-8 sequences) or the end of the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecOutput {
    Data(String),
    Exited(String),
}

/// Returned to a shell window when it connects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecSession {
    pub context: String,
    pub target: ExecTarget,
    pub title: String,
    /// Event name this window's output arrives on.
    pub event: String,
}

/// What a port-forward connects to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForwardSource {
    Pod { pod: String },
    /// Resolved to a ready backing pod, like `kubectl port-forward svc/…`.
    Service { service: String },
}

impl ForwardSource {
    pub fn label(&self) -> String {
        match self {
            Self::Pod { pod } => format!("pod/{pod}"),
            Self::Service { service } => format!("service/{service}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardRequest {
    pub namespace: String,
    pub source: ForwardSource,
    /// Container port for pods, service port for services.
    pub remote_port: u16,
    /// 0 picks a free port.
    pub local_port: u16,
}

/// A port that can be forwarded, for the start dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortOption {
    pub port: u16,
    pub name: Option<String>,
    /// The container declaring it (pods), or the target port (services).
    pub detail: Option<String>,
}

/// A running port-forward, for the Port Forwards page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardInfo {
    pub id: u64,
    pub context: String,
    pub namespace: String,
    pub source: ForwardSource,
    pub remote_port: u16,
    pub local_port: u16,
    /// The pod and port connections currently go to (services resolve to one).
    pub backend: Option<String>,
    pub active: usize,
    pub total: u64,
    pub last_error: Option<String>,
}

/// Tauri event carrying the full forward list whenever it changes.
pub mod forward_events {
    pub const CHANGED: &str = "port-forwards";
}

/// Where an uploaded file goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadTarget {
    pub context: String,
    pub namespace: String,
    pub pod: String,
    pub container: String,
    /// Absolute path in the container. A directory (trailing `/`, or one that
    /// exists) gets `file_name` appended; otherwise it's the file to write.
    /// The directory must already exist.
    pub path: String,
    /// The original file's name, used when `path` is a directory.
    pub file_name: String,
    /// Replace an existing file instead of reporting it.
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UploadOutcome {
    /// `path` is where the file ended up.
    Uploaded { bytes: u64, path: String },
    /// Nothing was written because `path` exists and `overwrite` was false.
    Exists { path: String },
}

/// Who can browse to what: one directory in a container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirListing {
    /// Resolved absolute path.
    pub path: String,
    /// Whether the container user can create files here.
    pub writable: bool,
    /// Directories first, then files, each sorted by name.
    pub entries: Vec<DirEntry>,
    /// More entries existed than were listed.
    pub truncated: bool,
    /// Writable places worth offering: /tmp, $HOME, writable mounts.
    pub suggestions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub dir: bool,
    pub writable: bool,
    /// Whether the container user can read it (download a file, list a folder).
    pub readable: bool,
}

/// A file copied out of a container onto this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadResult {
    /// Where it was saved locally.
    pub saved_to: String,
    pub bytes: u64,
}
