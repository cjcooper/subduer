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
    /// The watch gave up after repeated failures and has ended.
    #[serde(default)]
    pub stopped: bool,
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

/// Everything shown in a selected Deployment's, StatefulSet's, Job's or
/// CronJob's Details tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkloadDetails {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    /// e.g. `Available`, `Rolling out`, `Degraded`, `Paused`, `Scaled to 0`,
    /// `Running`, `Complete`, `Failed`, `Suspended`, `Scheduled`.
    pub status: String,
    /// Deployments and StatefulSets only.
    pub replicas: Option<ReplicaCounts>,
    /// Jobs and CronJobs: whether `spec.suspend` is set.
    #[serde(default)]
    pub suspended: Option<bool>,
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
    /// A CronJob's Jobs, newest first.
    #[serde(default)]
    pub jobs: Vec<JobRun>,
    /// The workload's, its ReplicaSets' (or Jobs') and its pods' events, newest first.
    pub events: Vec<EventSummary>,
}

/// Everything shown in the Details tab of a Config-section kind (ConfigMap,
/// Secret, HPA, PDB, quotas, admission webhooks and policies, …): one generic
/// shape, rendered by one view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectDetails {
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
    /// Badge text, e.g. `Active`, `Blocking`, `Held`.
    pub status: Option<String>,
    /// Rows of the kind's own section, in display order.
    pub info: Vec<InfoRow>,
    pub created: Option<String>,
    pub labels: Vec<(String, String)>,
    /// Without `kubectl.kubernetes.io/last-applied-configuration`.
    pub annotations: Vec<(String, String)>,
    pub conditions: Vec<Condition>,
    /// A ConfigMap's or Secret's data.
    pub data: Option<DataSet>,
    /// Further sections after the kind's own one.
    pub sections: Vec<InfoSection>,
    /// Newest first; only for namespaced objects.
    pub events: Vec<EventSummary>,
}

/// Another object, opened in its own list when clicked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectLink {
    /// `group/Kind`, e.g. `apps/Deployment`, `core/Pod`.
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InfoRow {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub mono: bool,
    /// RFC 3339; shown as `5m ago` or `in 4h` beside it.
    #[serde(default)]
    pub time: bool,
    #[serde(default)]
    pub link: Option<ObjectLink>,
    /// An http(s) address the row offers to open in the browser.
    #[serde(default)]
    pub url: Option<String>,
}

impl InfoRow {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self { key: key.into(), value: value.into(), mono: false, time: false, link: None, url: None }
    }
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
    pub fn time(mut self) -> Self {
        self.time = true;
        self
    }
    pub fn link(mut self, link: ObjectLink) -> Self {
        self.link = Some(link);
        self
    }
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InfoSection {
    /// Remembers the collapsed state, e.g. `webhook:check.acme.io`.
    pub id: String,
    pub title: String,
    pub summary: String,
    pub rows: Vec<InfoRow>,
    pub table: Option<InfoTable>,
    /// Shown when there are no rows and no table rows.
    #[serde(default)]
    pub empty: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InfoTable {
    pub columns: Vec<String>,
    /// Right-aligned columns.
    #[serde(default)]
    pub numeric: Vec<usize>,
    /// Monospaced columns.
    #[serde(default)]
    pub mono: Vec<usize>,
    /// Columns holding RFC 3339 times, shown as ages (`3h`) with the time on hover.
    #[serde(default)]
    pub times: Vec<usize>,
    pub rows: Vec<InfoTableRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InfoTableRow {
    pub cells: Vec<String>,
    #[serde(default)]
    pub link: Option<ObjectLink>,
    /// 0..=1, drawn as a bar after the cells, e.g. quota used of hard.
    #[serde(default)]
    pub fill: Option<f64>,
    #[serde(default)]
    pub warn: bool,
}

impl InfoTableRow {
    pub fn new(cells: Vec<String>) -> Self {
        Self { cells, link: None, fill: None, warn: false }
    }
}

/// A ConfigMap's or Secret's keys. Secret values are never included: they're
/// fetched on Show, or all at once when editing starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataSet {
    pub secret: bool,
    /// For the save's optimistic concurrency check.
    pub resource_version: String,
    /// Why it can't be edited here (immutable, a Helm release record, …).
    pub read_only: Option<String>,
    /// Edits are allowed but may be undone, e.g. by Helm or an owning controller.
    pub warning: Option<String>,
    pub entries: Vec<DataEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataEntry {
    pub key: String,
    /// A ConfigMap's text value. Always `None` for Secrets and binary values.
    pub value: Option<String>,
    pub size: usize,
    /// Not UTF-8 (a ConfigMap's `binaryData`, or such a Secret value): kept
    /// as is when saving, but can be removed.
    pub binary: bool,
}

/// One key's new value in a save; `None` removes the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataChange {
    pub key: String,
    pub value: Option<String>,
}

/// A Secret's values for editing; `None` for binary values.
pub type SecretValues = Vec<(String, Option<String>)>;

/// Config, Network, Storage and Access Control kinds with an [`ObjectDetails`] Details tab.
pub fn has_object_details(kind: &KindRef) -> bool {
    matches!(
        kind.key().as_str(),
        "core/ConfigMap"
            | "core/Secret"
            | "autoscaling/HorizontalPodAutoscaler"
            | "policy/PodDisruptionBudget"
            | "core/ResourceQuota"
            | "core/LimitRange"
            | "scheduling.k8s.io/PriorityClass"
            | "node.k8s.io/RuntimeClass"
            | "coordination.k8s.io/Lease"
            | "admissionregistration.k8s.io/MutatingWebhookConfiguration"
            | "admissionregistration.k8s.io/ValidatingWebhookConfiguration"
            | "admissionregistration.k8s.io/ValidatingAdmissionPolicy"
            | "admissionregistration.k8s.io/ValidatingAdmissionPolicyBinding"
            | "core/Service"
            | "networking.k8s.io/Ingress"
            | "networking.k8s.io/IngressClass"
            | "networking.k8s.io/NetworkPolicy"
            | "discovery.k8s.io/EndpointSlice"
            | "core/Endpoints"
            | "core/PersistentVolumeClaim"
            | "core/PersistentVolume"
            | "storage.k8s.io/StorageClass"
            | "storage.k8s.io/VolumeAttachment"
            | "storage.k8s.io/CSIDriver"
            | "storage.k8s.io/CSINode"
            | "storage.k8s.io/CSIStorageCapacity"
            | "core/ServiceAccount"
            | "rbac.authorization.k8s.io/Role"
            | "rbac.authorization.k8s.io/RoleBinding"
            | "rbac.authorization.k8s.io/ClusterRole"
            | "rbac.authorization.k8s.io/ClusterRoleBinding"
            | "helm.sh/Release"
            | "core/Namespace"
    )
}

/// The API server's answer to "may this account do that?".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessCheck {
    pub allowed: bool,
    /// Which rule allowed it, or why not; often empty when denied.
    pub reason: String,
}

/// ConfigMaps and Secrets: their data can be edited and saved.
pub fn has_editable_data(kind: &KindRef) -> bool {
    matches!(kind.key().as_str(), "core/ConfigMap" | "core/Secret")
}

/// One Job of a CronJob, for its Jobs table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobRun {
    pub name: String,
    /// `Running`, `Complete`, `Failed`, `Suspended` or `Pending`.
    pub status: String,
    /// e.g. `1/1`.
    pub completions: String,
    /// Started by hand (Run now / `kubectl create job --from`) rather than on schedule.
    pub manual: bool,
    pub created: Option<String>,
    pub started: Option<String>,
    pub finished: Option<String>,
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
    /// Succeeded or failed: no longer running.
    #[serde(default)]
    pub finished: bool,
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

/// Kinds that get Suspend/Resume.
pub fn is_batch_workload(kind: &KindRef) -> bool {
    matches!((kind.group.as_str(), kind.kind.as_str()), ("batch", "Job" | "CronJob"))
}

/// Kinds with a Details tab built from [`WorkloadDetails`].
pub fn has_workload_details(kind: &KindRef) -> bool {
    is_scalable_workload(kind) || is_batch_workload(kind)
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

/// What an edit window edits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditTarget {
    pub kind: KindRef,
    pub namespace: Option<String>,
    pub name: String,
}

impl EditTarget {
    /// Window title, e.g. `ConfigMap shop/app · acme-uat · Edit`.
    pub fn title(&self, context: &str) -> String {
        let at = match &self.namespace {
            Some(ns) => format!("{ns}/{}", self.name),
            None => self.name.clone(),
        };
        format!("{} {at} · {context} · Edit", self.kind.kind)
    }
}

/// Returned to an edit window when it starts (and on Reload).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditSession {
    pub context: String,
    pub prod_like: bool,
    pub unlocked: bool,
    pub target: EditTarget,
    pub title: String,
    pub yaml: String,
}

/// Events for edit windows and the main window.
pub mod edit_events {
    /// To an edit window whose close was held for unsaved edits.
    pub const CLOSE_REQUESTED: &str = "edit-close-requested";
    /// To the main window after a save, with the [`super::EditTarget`].
    pub const SAVED: &str = "object-saved";
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

/// What a trace records, within one context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceScope {
    Cluster,
    Namespaces(Vec<String>),
    /// Deployments, StatefulSets, DaemonSets, Jobs or CronJobs. Pods are
    /// matched by name pattern, so ones started during the trace count.
    Workloads(Vec<TraceWorkload>),
    /// One pod; its data stops when the pod goes away.
    Pod { namespace: String, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceWorkload {
    pub kind: KindRef,
    pub namespace: String,
    pub name: String,
}

/// Why a trace ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceEnd {
    Stopped,
    Quit,
    ContextChanged,
    ConnectionLost,
}

/// A running trace, for the indicator. Times are unix seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceStatus {
    pub context: String,
    pub path: String,
    pub scope: TraceScope,
    pub started: f64,
    /// The span the file covers so far (backfill reaches before `started`).
    pub data_start: f64,
    pub data_end: f64,
    /// Whether a Prometheus was found; without one only the app's samples are kept.
    pub prometheus: bool,
    pub last_pull: Option<f64>,
    /// Why the last pull failed; cleared by the next one that works.
    pub last_error: Option<String>,
    pub gaps: u32,
    /// Running pods Prometheus misses, sampled by the app.
    pub sampled_pods: u32,
    /// Stop was asked for; the last pull and the file's close are under way.
    pub stopping: bool,
}

/// One row of a trace review: a container of a workload (or a bare pod).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TraceKey {
    pub namespace: String,
    /// Deployment, StatefulSet, DaemonSet, CronJob, Job, ReplicaSet, or Pod.
    pub kind: String,
    pub name: String,
    pub container: String,
}

/// A trace file read for review: its header and one row per container.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceReview {
    pub summary: TraceSummary,
    pub rows: Vec<ReviewRow>,
}

/// Usage of one workload container over the whole trace, against its
/// requests. CPU in millicores, memory in bytes. Percentiles are over every
/// pod-minute (each pod's average for the minute); maxima are the highest
/// any pod reached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewRow {
    pub key: TraceKey,
    /// Pods seen, and how many ran at once on average.
    pub pods: u32,
    pub avg_pods: f64,
    /// Minutes with data, and pod-minutes.
    pub minutes: u64,
    pub pod_minutes: u64,
    /// Rollouts seen (new pod-template hashes after the first).
    pub rollouts: u32,
    /// Share of pod-minutes sampled by the app rather than Prometheus.
    pub local_share: f64,
    /// The latest requests and limits.
    pub cpu_request: Option<f64>,
    pub cpu_limit: Option<f64>,
    pub mem_request: Option<f64>,
    pub mem_limit: Option<f64>,
    pub cpu_p50: f64,
    pub cpu_p95: f64,
    pub cpu_p99: f64,
    pub cpu_max: f64,
    pub mem_p95: f64,
    pub mem_max: f64,
    /// Requested but unused, over all pods: (request − p95 CPU) × average
    /// pods, and (request − peak memory) × average pods. Negative when
    /// usage is above the request. `None` without a request.
    pub cpu_spare: Option<f64>,
    pub mem_spare: Option<f64>,
    pub flags: Vec<ReviewFlag>,
    pub suggestion: Option<Suggestion>,
}

/// Something worth a look on a review row. Limits only matter when hit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ReviewFlag {
    /// The worst minute had this share of CPU periods throttled (0..1).
    Throttled { worst: f64 },
    /// Peak memory reached this share of the limit (≥ 0.9).
    NearMemoryLimit { share: f64 },
    /// Restarts whose last termination was OOMKilled.
    OomKilled { count: u32 },
    /// The kernel killed a process for memory without the container restarting.
    OomEvents { count: u32 },
    /// Other restarts.
    Restarts { count: u32 },
    NoCpuRequest,
    NoMemoryRequest,
    /// Requests changed during the trace; the figures cover both.
    RequestsChanged,
    /// Less than a day of data: daily peaks may be missing.
    ShortData { hours: f64 },
    /// Too little data to suggest anything.
    TooLittleData { hours: f64 },
    /// The suggested memory request is above the memory limit, so applying
    /// it as is would be rejected.
    AboveMemoryLimit,
}

/// Suggested requests, and what they rest on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    /// Millicores and bytes, rounded up to tidy values.
    pub cpu_request: f64,
    pub mem_request: f64,
    /// e.g. "p95 CPU and peak memory × 1.15 over 4d 2h (11,520 pod-minutes), 3 rollouts, no OOM kills".
    pub basis: String,
}

/// One container of a workload over a span of a trace, folded into buckets
/// to suit the chart. Times in unix seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceSeries {
    pub start: f64,
    pub end: f64,
    /// Bucket size in seconds.
    pub step: f64,
    /// Per bucket: the average pod's usage, and the busiest pod's peak.
    pub cpu_avg: Series,
    pub cpu_max: Series,
    pub mem_avg: Series,
    pub mem_max: Series,
    /// Pods running, on average over each bucket.
    pub pods: Series,
    /// Requests and limits as change points (the value holds until the next).
    pub cpu_request: Series,
    pub cpu_limit: Series,
    pub mem_request: Series,
    pub mem_limit: Series,
    pub events: Vec<TraceEventMark>,
    pub gaps: Vec<TraceGap>,
    /// The whole trace's span for this container, for zooming back out.
    pub full_start: f64,
    pub full_end: f64,
}

/// A restart or OOM kill, for the chart's markers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceEventMark {
    pub t: f64,
    /// "restart" or "oom".
    pub kind: String,
    pub reason: Option<String>,
    pub count: u32,
}

/// Usage by local day and hour, for spotting daily patterns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceDaily {
    pub days: Vec<TraceDay>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceDay {
    /// e.g. "Mon 12 Oct".
    pub label: String,
    /// 24 hours: p95 CPU (millicores) and peak memory (bytes); `None` without data.
    pub cpu: Vec<Option<f64>>,
    pub mem: Vec<Option<f64>>,
}

/// New requests for some containers of one workload, applied as one patch
/// (so the workload rolls out once).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RightsizeChange {
    /// Deployment, StatefulSet, DaemonSet or CronJob.
    pub kind: String,
    pub namespace: String,
    pub name: String,
    pub containers: Vec<ContainerRequests>,
}

/// CPU in millicores, memory in bytes; `None` leaves that request alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerRequests {
    pub container: String,
    pub cpu_request: Option<f64>,
    pub mem_request: Option<f64>,
}

/// What applying a change would do, checked by a server dry run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RightsizePreview {
    pub change: RightsizeChange,
    /// The Helm release that owns the workload: its next upgrade puts the
    /// old values back unless the chart's values change too.
    pub helm_release: Option<String>,
    /// For Helm-managed workloads: `resources:` blocks to put in the chart's values.
    pub helm_snippet: Option<String>,
    pub containers: Vec<ContainerPreview>,
    /// Why it can't be applied (unsupported kind, nothing to change, …).
    pub blocked: Option<String>,
    /// The server's answer to the dry run when it refused the patch.
    pub dry_run_error: Option<String>,
    /// e.g. "Rolls out new pods" or "Applies from the next scheduled run".
    pub effect: String,
}

/// One container's requests and limits before and after, as quantities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerPreview {
    pub container: String,
    /// Not in the workload's pod template: added by an admission webhook, so
    /// it can't be changed here.
    pub injected: bool,
    pub cpu_request: QuantityChange,
    pub cpu_limit: QuantityChange,
    pub mem_request: QuantityChange,
    pub mem_limit: QuantityChange,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuantityChange {
    pub before: Option<String>,
    pub after: Option<String>,
}

impl QuantityChange {
    pub fn changed(&self) -> bool {
        self.before != self.after
    }
}

/// What can be traced in one namespace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceTargets {
    pub workloads: Vec<TraceWorkload>,
    /// Running pods, by name.
    pub pods: Vec<String>,
}

/// A span with no data for the whole trace, in unix seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceGap {
    pub start: f64,
    pub end: f64,
    pub reason: String,
}

/// The header of a trace file and how much it holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceSummary {
    pub path: String,
    pub context: String,
    pub scope: TraceScope,
    /// When recording started, in unix seconds.
    pub started: f64,
    /// When recording ended; `None` while running, or if the app ended
    /// without closing it (see `unfinished`).
    pub ended: Option<f64>,
    pub end_reason: Option<TraceEnd>,
    /// The file has no end and isn't being recorded: the app crashed or was
    /// killed. Data up to `data_end` is intact.
    pub unfinished: bool,
    /// The span covered by samples (backfill reaches before `started`).
    pub data_start: Option<f64>,
    pub data_end: Option<f64>,
    /// `namespace/service` of the Prometheus used, if any.
    pub prometheus: Option<String>,
    /// Prometheus' retention in seconds, as read from its flags.
    pub retention_secs: Option<f64>,
    pub pods: u64,
    pub containers: u64,
    /// One-minute samples from Prometheus and from the app itself.
    pub prometheus_samples: u64,
    pub local_samples: u64,
    pub gaps: Vec<TraceGap>,
    pub file_bytes: u64,
}

#[cfg(test)]
mod tests;
