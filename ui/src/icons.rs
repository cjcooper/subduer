//! Line icons for the sidebar: 24×24 paths drawn with `currentColor`, so they
//! follow the text colour in both themes and in the active state.

use leptos::prelude::*;
use subduer_types::{Category, KindRef};

const CLUSTER: &str = r#"<path d="M12 2.5l8.2 4.75v9.5L12 21.5l-8.2-4.75v-9.5z"/><circle cx="12" cy="12" r="3"/>"#;
const SERVER: &str = r#"<rect x="3" y="4" width="18" height="7" rx="1.5"/><rect x="3" y="13" width="18" height="7" rx="1.5"/><path d="M7 7.5h.01M7 16.5h.01"/>"#;
const PULSE: &str = r#"<path d="M3 12h4l3-7 4 14 3-7h4"/>"#;
const CUBE: &str = r#"<path d="M12 2.5l8.5 4.75v9.5L12 21.5l-8.5-4.75v-9.5z"/><path d="M3.5 7.25L12 12l8.5-4.75M12 12v9.5"/>"#;
const LAYERS: &str = r#"<path d="M12 3l9 4.5-9 4.5-9-4.5z"/><path d="M3 12l9 4.5 9-4.5"/><path d="M3 16.5l9 4.5 9-4.5"/>"#;
const DATABASE: &str = r#"<ellipse cx="12" cy="5.5" rx="8" ry="3"/><path d="M4 5.5v13c0 1.66 3.58 3 8 3s8-1.34 8-3v-13"/><path d="M4 12c0 1.66 3.58 3 8 3s8-1.34 8-3"/>"#;
const GRID: &str = r#"<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/>"#;
const COPY: &str = r#"<rect x="8" y="8" width="13" height="13" rx="2"/><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"/>"#;
const PLAY: &str = r#"<circle cx="12" cy="12" r="9"/><path d="M10 8.5l5.5 3.5-5.5 3.5z"/>"#;
const CLOCK: &str = r#"<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3.5 2"/>"#;
const FILE: &str = r#"<path d="M14 3H6.5A2.5 2.5 0 0 0 4 5.5v13A2.5 2.5 0 0 0 6.5 21h11a2.5 2.5 0 0 0 2.5-2.5V9z"/><path d="M14 3v6h6M8 13h8M8 17h5"/>"#;
const LOCK: &str = r#"<rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>"#;
const UNLOCK: &str = r#"<rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 7.75-1.4"/>"#;
const SCALE: &str = r#"<path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7"/>"#;
const SHIELD: &str = r#"<path d="M12 3l8 3v6c0 4.8-3.4 8-8 9-4.6-1-8-4.2-8-9V6z"/>"#;
const SHIELD_CHECK: &str = r#"<path d="M12 3l8 3v6c0 4.8-3.4 8-8 9-4.6-1-8-4.2-8-9V6z"/><path d="M9 12l2 2 4-4"/>"#;
const GAUGE: &str = r#"<path d="M4 17a8 8 0 1 1 16 0"/><path d="M12 17l4-5.5"/><path d="M3 20h18"/>"#;
const RULER: &str = r#"<path d="M4 4h16M4 20h16M12 7.5v9M9 10l3-3 3 3M9 14l3 3 3-3"/>"#;
const FLAG: &str = r#"<path d="M5 21V4M5 4h13l-2.5 4.5L18 13H5"/>"#;
const CHIP: &str = r#"<rect x="6" y="6" width="12" height="12" rx="1.5"/><path d="M9.5 2.5V6M14.5 2.5V6M9.5 18v3.5M14.5 18v3.5M2.5 9.5H6M2.5 14.5H6M18 9.5h3.5M18 14.5h3.5"/>"#;
const HOURGLASS: &str = r#"<path d="M6 3h12M6 21h12M7 3v2.5a5 5 0 0 0 10 0V3M7 21v-2.5a5 5 0 0 1 10 0V21"/>"#;
const PLUG: &str = r#"<path d="M9 3v5M15 3v5M6 8h12v3a6 6 0 0 1-12 0zM12 17v4"/>"#;
const SHARE: &str = r#"<circle cx="12" cy="5" r="2.5"/><circle cx="5" cy="19" r="2.5"/><circle cx="19" cy="19" r="2.5"/><path d="M12 7.5V12M12 12l-5.3 5.2M12 12l5.3 5.2"/>"#;
const ENTER: &str = r#"<path d="M14 3h4.5A2.5 2.5 0 0 1 21 5.5v13a2.5 2.5 0 0 1-2.5 2.5H14"/><path d="M3 12h12M11 8l4 4-4 4"/>"#;
const TAG: &str = r#"<path d="M3 12V4.5A1.5 1.5 0 0 1 4.5 3H12l9 9-9 9z"/><circle cx="7.5" cy="7.5" r="1.5"/>"#;
const WALL: &str = r#"<rect x="3" y="4" width="18" height="16" rx="1.5"/><path d="M3 9.33h18M3 14.67h18M9 4v5.33M15 9.33v5.34M9 14.67V20"/>"#;
const TARGET: &str = r#"<circle cx="12" cy="12" r="8.5"/><circle cx="12" cy="12" r="3.5"/>"#;
const DRIVE: &str = r#"<path d="M3 13h18v5.5a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5z"/><path d="M3 13l2.6-7.7A1.5 1.5 0 0 1 7 4.3h10a1.5 1.5 0 0 1 1.4 1L21 13"/><path d="M7 16.5h.01"/>"#;
const TRAY: &str = r#"<path d="M3 13h5l1.5 3h5L16 13h5"/><path d="M5.6 5h12.8L21 13v6a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 19v-6z"/>"#;
const LINK: &str = r#"<path d="M10 14a4 4 0 0 0 5.66 0l3-3a4 4 0 0 0-5.66-5.66l-1 1"/><path d="M14 10a4 4 0 0 0-5.66 0l-3 3a4 4 0 0 0 5.66 5.66l1-1"/>"#;
const FOLDER: &str = r#"<path d="M3 6.5A2.5 2.5 0 0 1 5.5 4H9l2 2.5h7.5A2.5 2.5 0 0 1 21 9v8.5a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5z"/>"#;
const USER: &str = r#"<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>"#;
const KEY: &str = r#"<circle cx="7.5" cy="15.5" r="4.5"/><path d="M10.7 12.3L20 3M16.5 6.5l3 3M13.5 9.5l2 2"/>"#;
const HELM: &str = r#"<circle cx="12" cy="12" r="6"/><circle cx="12" cy="12" r="1.8"/><path d="M12 2v4M12 18v4M2 12h4M18 12h4M4.9 4.9l2.8 2.8M16.3 16.3l2.8 2.8M4.9 19.1l2.8-2.8M16.3 7.7l2.8-2.8"/>"#;
const PUZZLE: &str = r#"<path d="M4 7h4a2 2 0 1 1 4 0h4v4a2 2 0 1 1 0 4v4h-4a2 2 0 1 0-4 0H4v-4a2 2 0 1 0 0-4z"/>"#;
const SLIDERS: &str = r#"<path d="M4 6h9M17 6h3M4 12h3M11 12h9M4 18h11M19 18h1"/><circle cx="15" cy="6" r="2"/><circle cx="9" cy="12" r="2"/><circle cx="17" cy="18" r="2"/>"#;
const GLOBE: &str = r#"<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a13.5 13.5 0 0 1 0 18M12 3a13.5 13.5 0 0 0 0 18"/>"#;
const BELL: &str = r#"<path d="M6 8.5a6 6 0 0 1 12 0c0 6.5 2.5 8.5 2.5 8.5h-17S6 15 6 8.5"/><path d="M10.3 20.5a2 2 0 0 0 3.4 0"/>"#;
const DASHBOARD: &str = r#"<rect x="3" y="3" width="7" height="9" rx="1.5"/><rect x="14" y="3" width="7" height="5" rx="1.5"/><rect x="14" y="12" width="7" height="9" rx="1.5"/><rect x="3" y="16" width="7" height="5" rx="1.5"/>"#;
const CHART: &str = r#"<path d="M3.5 3.5v17h17"/><path d="M7.5 15l4-4.5 3 3 5.5-6"/>"#;
const BROADCAST: &str = r#"<circle cx="12" cy="12" r="2"/><path d="M16.2 7.8a6 6 0 0 1 0 8.4M7.8 16.2a6 6 0 0 1 0-8.4M19 5a10 10 0 0 1 0 14M5 19A10 10 0 0 1 5 5"/>"#;
const VAULT: &str = r#"<rect x="3" y="3.5" width="18" height="15.5" rx="2"/><circle cx="12" cy="11.25" r="3.5"/><path d="M12 7.75V6.5M12 16v-1.25M8.5 11.25H7.25M16.75 11.25H15.5M7 19v2M17 19v2"/>"#;
/// A secret pulled into the cluster (ExternalSecret).
const SECRET_IN: &str = r#"<rect x="4" y="12" width="16" height="9" rx="2"/><path d="M12 2.5V9M9 6l3 3 3-3M12 15.25v2.5"/>"#;
/// A secret pushed out of the cluster (PushSecret).
const SECRET_OUT: &str = r#"<rect x="4" y="12" width="16" height="9" rx="2"/><path d="M12 9V2.5M9 5.5l3-3 3 3M12 15.25v2.5"/>"#;
const WAND: &str = r#"<path d="M4 20L14.5 9.5M12.5 7.5l4 4"/><path d="M18 2.5v3M16.5 4h3M20 9v2M19 10h2M9.5 3v2M8.5 4h2"/>"#;
const CLOUD: &str = r#"<path d="M7 18.5a4.5 4.5 0 0 1-.55-8.97A6 6 0 0 1 17.9 9.6a4.25 4.25 0 0 1-.4 8.9z"/>"#;
const CLIPBOARD: &str = r#"<rect x="5" y="4" width="14" height="17" rx="2"/><path d="M9 4V3h6v1M8.5 11h7M8.5 15h4.5"/>"#;
const SHIELD_OFF: &str = r#"<path d="M12 3l8 3v6c0 4.8-3.4 8-8 9-4.6-1-8-4.2-8-9V6z"/><path d="M5 5l14 14"/>"#;
const ERASER: &str = r#"<path d="M8 20.5h12"/><path d="M4.6 14.9l8.8-8.8a2 2 0 0 1 2.83 0l2.67 2.67a2 2 0 0 1 0 2.83L11 19.5H8.4z"/><path d="M9.5 10l5.5 5.5"/>"#;
const REFRESH: &str = r#"<path d="M20 11a8 8 0 0 0-14.6-4.5M4 4v4.5h4.5"/><path d="M4 13a8 8 0 0 0 14.6 4.5M20 20v-4.5h-4.5"/>"#;
const EYE: &str = r#"<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="3"/>"#;
const LIST: &str = r#"<path d="M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01"/>"#;
const ANTENNA: &str = r#"<circle cx="12" cy="9" r="2"/><path d="M12 11v10M8.5 21h7"/><path d="M8.2 5.2a5.4 5.4 0 0 0 0 7.6M15.8 5.2a5.4 5.4 0 0 1 0 7.6"/>"#;
const BOX: &str = r#"<rect x="4" y="4" width="16" height="16" rx="3"/><path d="M8.5 12h.01M12 12h.01M15.5 12h.01"/>"#;

/// Icon markup for a kind; custom resources share the puzzle piece.
fn kind_paths(kind: &KindRef) -> &'static str {
    match (kind.group.as_str(), kind.kind.as_str()) {
        ("", "Node") | ("storage.k8s.io", "CSINode") => SERVER,
        ("", "Event") => PULSE,
        ("", "Pod") => CUBE,
        ("apps", "Deployment") => LAYERS,
        ("apps", "StatefulSet") => DATABASE,
        ("apps", "DaemonSet") => GRID,
        ("apps", "ReplicaSet") | ("", "ReplicationController") => COPY,
        ("batch", "Job") => PLAY,
        ("batch", "CronJob") => CLOCK,
        ("", "ConfigMap") => FILE,
        ("", "Secret") => LOCK,
        ("autoscaling", "HorizontalPodAutoscaler") => SCALE,
        ("policy", "PodDisruptionBudget") => SHIELD,
        ("", "ResourceQuota") | ("storage.k8s.io", "CSIStorageCapacity") => GAUGE,
        ("", "LimitRange") => RULER,
        ("scheduling.k8s.io", "PriorityClass") => FLAG,
        ("node.k8s.io", "RuntimeClass") => CHIP,
        ("coordination.k8s.io", "Lease") => HOURGLASS,
        ("admissionregistration.k8s.io", k) if k.contains("Webhook") => PLUG,
        ("admissionregistration.k8s.io", _) => SHIELD_CHECK,
        ("", "Service") => SHARE,
        ("networking.k8s.io", "Ingress") => ENTER,
        ("networking.k8s.io", "IngressClass") | ("storage.k8s.io", "StorageClass") => TAG,
        ("networking.k8s.io", "NetworkPolicy") => WALL,
        ("discovery.k8s.io", "EndpointSlice") | ("", "Endpoints") => TARGET,
        ("", "PersistentVolumeClaim") => TRAY,
        ("", "PersistentVolume") => DRIVE,
        ("storage.k8s.io", "VolumeAttachment") => LINK,
        ("storage.k8s.io", "CSIDriver") => PLUG,
        ("", "Namespace") => FOLDER,
        ("", "ServiceAccount") => USER,
        ("rbac.authorization.k8s.io", "Role" | "ClusterRole") => KEY,
        ("rbac.authorization.k8s.io", _) => LINK,
        ("helm.sh", "Release") => HELM,
        ("apiextensions.k8s.io", "CustomResourceDefinition") => PUZZLE,
        _ => BOX,
    }
}

/// Icons for custom resources from well-known projects. Each draws what the
/// kind does, in the same line style as built-ins, rather than a project logo.
fn custom_paths(kind: &KindRef) -> Option<&'static str> {
    let k = kind.kind.as_str();
    let paths = match kind.group.as_str() {
        // Kyverno: policies, exceptions, cleanup, reports.
        "kyverno.io" | "policies.kyverno.io" | "reports.kyverno.io" | "wgpolicyk8s.io" => match k {
            "PolicyException" => SHIELD_OFF,
            "CleanupPolicy" | "ClusterCleanupPolicy" | "DeletingPolicy" | "NamespacedDeletingPolicy" => ERASER,
            "UpdateRequest" => REFRESH,
            "GlobalContextEntry" => GLOBE,
            _ if k.ends_with("Report") => CLIPBOARD,
            _ => SHIELD_CHECK,
        },
        "datadoghq.com" => match k {
            "DatadogMonitor" => BELL,
            "DatadogDashboard" => DASHBOARD,
            "DatadogSLO" => TARGET,
            "DatadogPodAutoscaler" => SCALE,
            "DatadogAgent" | "DatadogAgentProfile" => SERVER,
            "DatadogGenericResource" => BOX,
            _ => CHART,
        },
        // Prometheus Operator.
        "monitoring.coreos.com" => match k {
            "PrometheusAgent" => ANTENNA,
            "ServiceMonitor" | "PodMonitor" => EYE,
            "Probe" => TARGET,
            "PrometheusRule" | "ThanosRuler" => LIST,
            "Alertmanager" => BELL,
            "AlertmanagerConfig" | "ScrapeConfig" => SLIDERS,
            _ => CHART,
        },
        "keda.sh" => match k {
            "ScaledJob" => PLAY,
            "TriggerAuthentication" | "ClusterTriggerAuthentication" => KEY,
            _ => SCALE,
        },
        "eventing.keda.sh" => BROADCAST,
        "external-secrets.io" => match k {
            "SecretStore" | "ClusterSecretStore" => VAULT,
            "PushSecret" | "ClusterPushSecret" => SECRET_OUT,
            _ => SECRET_IN,
        },
        "generators.external-secrets.io" => match k {
            "SSHKey" => KEY,
            "GeneratorState" => CLIPBOARD,
            _ => WAND,
        },
        // Cloud providers' controllers (AWS, Google Cloud, Azure).
        g if Category::cloud_for_group(g).is_some() => match k {
            "TargetGroupBinding" | "PolicyEndpoint" | "ClusterPolicyEndpoint" => TARGET,
            "ManagedCertificate" => SHIELD_CHECK,
            "PodMonitoring" | "ClusterPodMonitoring" | "GlobalRules" | "Rules" => CHART,
            "ManagedCluster" | "AgentPool" | "ManagedClustersAgentPool" => SERVER,
            "ApplicationNetworkPolicy" | "ClusterNetworkPolicy" => WALL,
            "SecurityGroupPolicy" => SHIELD,
            "GlobalAccelerator" => GLOBE,
            "CNINode" => SERVER,
            "ENIConfig" => CHIP,
            _ if k.ends_with("Config") || k.ends_with("Configuration") || k.ends_with("Params") => SLIDERS,
            _ => CLOUD,
        },
        _ => return None,
    };
    Some(paths)
}

/// Icon for a custom-resource group heading, for well-known projects.
fn group_paths(group: &str) -> Option<&'static str> {
    Some(match group {
        "kyverno.io" | "policies.kyverno.io" | "reports.kyverno.io" | "wgpolicyk8s.io" => SHIELD_CHECK,
        "datadoghq.com" | "monitoring.coreos.com" => CHART,
        "keda.sh" => SCALE,
        "eventing.keda.sh" => BROADCAST,
        "external-secrets.io" => SECRET_IN,
        "generators.external-secrets.io" => WAND,
        g if Category::cloud_for_group(g).is_some() => CLOUD,
        _ => return None,
    })
}

fn category_paths(category: Category) -> &'static str {
    match category {
        Category::TopLevel => CLUSTER,
        Category::Workloads => CUBE,
        Category::Helm => HELM,
        Category::Aws | Category::GoogleCloud | Category::Azure => CLOUD,
        Category::Config => SLIDERS,
        Category::Network => GLOBE,
        Category::Storage => DATABASE,
        Category::Namespaces => FOLDER,
        Category::AccessControl => SHIELD_CHECK,
        Category::CustomResources => PUZZLE,
        Category::Other => BOX,
    }
}

fn svg(paths: &'static str) -> impl IntoView {
    view! {
        <svg
            class="nav-icon"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.8"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
            inner_html=paths
        ></svg>
    }
}

/// `custom`: the kind is a custom resource, so built-in names don't apply
/// (a CRD may be called `Service` too); unknown ones get the puzzle piece.
#[component]
pub fn KindIcon(kind: KindRef, #[prop(optional)] custom: bool) -> impl IntoView {
    svg(if custom { custom_paths(&kind).unwrap_or(PUZZLE) } else { kind_paths(&kind) })
}

/// Icon for a custom-resource API group heading; nothing for unknown groups.
#[component]
pub fn GroupIcon(group: String) -> impl IntoView {
    group_paths(&group).map(svg)
}

#[component]
pub fn CategoryIcon(category: Category) -> impl IntoView {
    svg(category_paths(category))
}

#[component]
pub fn ClusterIcon() -> impl IntoView {
    svg(CLUSTER)
}

const FORWARD: &str = r#"<path d="M4 8h13M14 4.5L17.5 8 14 11.5"/><path d="M20 16H7M10 12.5L6.5 16l3.5 3.5"/>"#;

#[component]
pub fn ForwardIcon() -> impl IntoView {
    svg(FORWARD)
}

/// Folder or file, for the file browser.
#[component]
pub fn EntryIcon(dir: bool) -> impl IntoView {
    svg(if dir { FOLDER } else { FILE })
}

#[component]
pub fn LockIcon(#[prop(optional)] open: bool) -> impl IntoView {
    svg(if open { UNLOCK } else { LOCK })
}

/// A line over time with a record dot: recorded usage.
const TRACES: &str = r#"<path d="M3 19h18"/><path d="M3 15l4-5 4 3 5-7 3 4"/><circle cx="19" cy="6" r="2"/>"#;

#[component]
pub fn TracesIcon() -> impl IntoView {
    svg(TRACES)
}

/// Filled brand marks from Simple Icons (https://simpleicons.org, CC0 1.0,
/// version 16.35.0), on a 24 unit grid.
const KUBERNETES_LOGO: &str = "M10.204 14.35l.007.01-.999 2.413a5.171 5.171 0 0 1-2.075-2.597l2.578-.437.004.005a.44.44 0 0 1 .484.606zm-.833-2.129a.44.44 0 0 0 .173-.756l.002-.011L7.585 9.7a5.143 5.143 0 0 0-.73 3.255l2.514-.725.002-.009zm1.145-1.98a.44.44 0 0 0 .699-.337l.01-.005.15-2.62a5.144 5.144 0 0 0-3.01 1.442l2.147 1.523.004-.002zm.76 2.75l.723.349.722-.347.18-.78-.5-.623h-.804l-.5.623.179.779zm1.5-3.095a.44.44 0 0 0 .7.336l.008.003 2.134-1.513a5.188 5.188 0 0 0-2.992-1.442l.148 2.615.002.001zm10.876 5.97l-5.773 7.181a1.6 1.6 0 0 1-1.248.594l-9.261.003a1.6 1.6 0 0 1-1.247-.596l-5.776-7.18a1.583 1.583 0 0 1-.307-1.34L2.1 5.573c.108-.47.425-.864.863-1.073L11.305.513a1.606 1.606 0 0 1 1.385 0l8.345 3.985c.438.209.755.604.863 1.073l2.062 8.955c.108.47-.005.963-.308 1.34zm-3.289-2.057c-.042-.01-.103-.026-.145-.034-.174-.033-.315-.025-.479-.038-.35-.037-.638-.067-.895-.148-.105-.04-.18-.165-.216-.216l-.201-.059a6.45 6.45 0 0 0-.105-2.332 6.465 6.465 0 0 0-.936-2.163c.052-.047.15-.133.177-.159.008-.09.001-.183.094-.282.197-.185.444-.338.743-.522.142-.084.273-.137.415-.242.032-.024.076-.062.11-.089.24-.191.295-.52.123-.736-.172-.216-.506-.236-.745-.045-.034.027-.08.062-.111.088-.134.116-.217.23-.33.35-.246.25-.45.458-.673.609-.097.056-.239.037-.303.033l-.19.135a6.545 6.545 0 0 0-4.146-2.003l-.012-.223c-.065-.062-.143-.115-.163-.25-.022-.268.015-.557.057-.905.023-.163.061-.298.068-.475.001-.04-.001-.099-.001-.142 0-.306-.224-.555-.5-.555-.275 0-.499.249-.499.555l.001.014c0 .041-.002.092 0 .128.006.177.044.312.067.475.042.348.078.637.056.906a.545.545 0 0 1-.162.258l-.012.211a6.424 6.424 0 0 0-4.166 2.003 8.373 8.373 0 0 1-.18-.128c-.09.012-.18.04-.297-.029-.223-.15-.427-.358-.673-.608-.113-.12-.195-.234-.329-.349-.03-.026-.077-.062-.111-.088a.594.594 0 0 0-.348-.132.481.481 0 0 0-.398.176c-.172.216-.117.546.123.737l.007.005.104.083c.142.105.272.159.414.242.299.185.546.338.743.522.076.082.09.226.1.288l.16.143a6.462 6.462 0 0 0-1.02 4.506l-.208.06c-.055.072-.133.184-.215.217-.257.081-.546.11-.895.147-.164.014-.305.006-.48.039-.037.007-.09.02-.133.03l-.004.002-.007.002c-.295.071-.484.342-.423.608.061.267.349.429.645.365l.007-.001.01-.003.129-.029c.17-.046.294-.113.448-.172.33-.118.604-.217.87-.256.112-.009.23.069.288.101l.217-.037a6.5 6.5 0 0 0 2.88 3.596l-.09.218c.033.084.069.199.044.282-.097.252-.263.517-.452.813-.091.136-.185.242-.268.399-.02.037-.045.095-.064.134-.128.275-.034.591.213.71.248.12.556-.007.69-.282v-.002c.02-.039.046-.09.062-.127.07-.162.094-.301.144-.458.132-.332.205-.68.387-.897.05-.06.13-.082.215-.105l.113-.205a6.453 6.453 0 0 0 4.609.012l.106.192c.086.028.18.042.256.155.136.232.229.507.342.84.05.156.074.295.145.457.016.037.043.09.062.129.133.276.442.402.69.282.247-.118.341-.435.213-.71-.02-.039-.045-.096-.065-.134-.083-.156-.177-.261-.268-.398-.19-.296-.346-.541-.443-.793-.04-.13.007-.21.038-.294-.018-.022-.059-.144-.083-.202a6.499 6.499 0 0 0 2.88-3.622c.064.01.176.03.213.038.075-.05.144-.114.28-.104.266.039.54.138.87.256.154.06.277.128.448.173.036.01.088.019.13.028l.009.003.007.001c.297.064.584-.098.645-.365.06-.266-.128-.537-.423-.608zM16.4 9.701l-1.95 1.746v.005a.44.44 0 0 0 .173.757l.003.01 2.526.728a5.199 5.199 0 0 0-.108-1.674A5.208 5.208 0 0 0 16.4 9.7zm-4.013 5.325a.437.437 0 0 0-.404-.232.44.44 0 0 0-.372.233h-.002l-1.268 2.292a5.164 5.164 0 0 0 3.326.003l-1.27-2.296h-.01zm1.888-1.293a.44.44 0 0 0-.27.036.44.44 0 0 0-.214.572l-.003.004 1.01 2.438a5.15 5.15 0 0 0 2.081-2.615l-2.6-.44-.004.005z";
const GOOGLE_CLOUD_LOGO: &str = "M12.19 2.38a9.344 9.344 0 0 0-9.234 6.893c.053-.02-.055.013 0 0-3.875 2.551-3.922 8.11-.247 10.941l.006-.007-.007.03a6.717 6.717 0 0 0 4.077 1.356h5.173l.03.03h5.192c6.687.053 9.376-8.605 3.835-12.35a9.365 9.365 0 0 0-2.821-4.552l-.043.043.006-.05A9.344 9.344 0 0 0 12.19 2.38zm-.358 4.146c1.244-.04 2.518.368 3.486 1.15a5.186 5.186 0 0 1 1.862 4.078v.518c3.53-.07 3.53 5.262 0 5.193h-5.193l-.008.009v-.04H6.785a2.59 2.59 0 0 1-1.067-.23h.001a2.597 2.597 0 1 1 3.437-3.437l3.013-3.012A6.747 6.747 0 0 0 8.11 8.24c.018-.01.04-.026.054-.023a5.186 5.186 0 0 1 3.67-1.69z";
/// Simple Icons has no AWS or Azure mark (their owners asked for them to be
/// removed); these are Devicon's single-colour ones (https://devicon.dev, MIT,
/// version 2.16.0), on a 128 unit grid.
const AWS_LOGO: &str = "M108.59 26.148c-1.852 0-3.622.211-5.305.715-1.684.504-3.117 1.223-4.379 2.188a10.829 10.829 0 0 0-3.031 3.453c-.757 1.348-1.137 2.906-1.137 4.676 0 2.187.716 4.25 2.106 6.105 1.386 1.895 3.66 3.324 6.734 4.293l6.106 1.895c2.062.675 3.496 1.391 4.254 2.191.757.801 1.136 1.765 1.136 2.945 0 1.726-.758 3.074-2.191 4-1.43.925-3.492 1.391-6.145 1.391-1.687 0-3.328-.168-5.011-.504a23.102 23.102 0 0 1-4.633-1.476c-.421-.168-.801-.336-1.051-.418a2.357 2.357 0 0 0-.758-.13c-.634 0-.969.423-.969 1.305v2.149a2.919 2.919 0 0 0 .254 1.18c.168.38.629.8 1.305 1.18 1.094.628 2.734 1.179 4.84 1.683 2.105.504 4.297.758 6.484.758 2.15 0 4.129-.297 6.024-.883 1.808-.551 3.367-1.309 4.672-2.36 1.304-1.01 2.316-2.273 3.074-3.707.714-1.429 1.094-3.07 1.094-4.882 0-2.188-.633-4.168-1.938-5.895-1.304-1.727-3.491-3.074-6.523-4.043l-5.98-1.895c-2.23-.713-3.79-1.516-4.634-2.316-.84-.797-1.261-1.808-1.261-2.988 0-1.726.671-2.95 1.98-3.746 1.305-.801 3.199-1.18 5.598-1.18 2.988 0 5.683.547 8.086 1.64.714.337 1.261.508 1.597.508.633 0 .969-.463.969-1.347v-1.98c0-.59-.125-1.051-.379-1.391-.25-.378-.672-.715-1.262-1.051-.422-.254-1.011-.504-1.77-.758a32.528 32.528 0 0 0-2.398-.676c-.886-.168-1.769-.336-2.738-.46a21.347 21.347 0 0 0-2.82-.169zm-86.822.082c-2.316 0-4.508.254-6.57.801-2.063.505-3.831 1.137-5.303 1.895-.59.297-.97.59-1.18.883-.211.296-.293.8-.293 1.476v2.063c0 .882.293 1.304.883 1.304.168 0 .378-.043.674-.125.293-.086.796-.254 1.472-.547a33.416 33.416 0 0 1 4.547-1.433A19.176 19.176 0 0 1 20.547 32c3.242 0 5.513.633 6.863 1.938 1.304 1.303 1.98 3.534 1.98 6.734v3.074c-1.683-.379-3.283-.715-4.843-.926-1.558-.21-3.031-.336-4.461-.336-4.34 0-7.75 1.094-10.316 3.286-2.571 2.187-3.832 5.093-3.832 8.671 0 3.368 1.05 6.063 3.113 8.086 2.066 2.02 4.887 3.032 8.422 3.032 4.97 0 9.097-1.938 12.379-5.813a34.153 34.153 0 0 0 1.304 2.484 13.28 13.28 0 0 0 1.516 1.98c.422.38.844.59 1.266.59.334 0 .714-.128 1.093-.378l2.653-1.77c.546-.42.8-.843.8-1.261a1.86 1.86 0 0 0-.293-.97 22.469 22.469 0 0 1-1.347-3.03c-.297-.925-.465-2.19-.465-3.75h-.086V40c0-4.633-1.176-8.086-3.492-10.36-2.36-2.273-6.025-3.41-11.033-3.41zm19.58 1.012c-.676 0-1.012.379-1.012 1.051 0 .297.129.844.379 1.687l9.894 32.547c.254.8.547 1.387.887 1.641.336.297.84.422 1.598.422h3.62c.759 0 1.347-.125 1.684-.422.34-.293.591-.84.801-1.684l6.485-27.117 6.527 27.16c.168.84.46 1.387.8 1.684.337.292.883.422 1.684.422h3.621c.715 0 1.262-.167 1.598-.422.34-.253.633-.8.887-1.64L90.949 30.02c.168-.46.25-.797.293-1.051.043-.254.086-.466.086-.676 0-.715-.379-1.05-1.055-1.05H86.36c-.757 0-1.308.166-1.644.421-.293.25-.59.8-.84 1.64L76.59 57.517l-6.653-28.211c-.166-.8-.464-1.39-.8-1.64-.336-.298-.884-.423-1.684-.423h-3.367c-.758 0-1.348.167-1.688.422-.335.25-.588.8-.796 1.64l-6.57 27.876-7.075-27.875c-.25-.8-.504-1.39-.84-1.64-.297-.298-.844-.423-1.644-.423h-4.125zM21.64 47.496a31.816 31.816 0 0 1 3.96.25 34.401 34.401 0 0 1 3.872.719v1.765c0 1.435-.168 2.653-.422 3.665-.25 1.01-.758 1.895-1.43 2.695-1.137 1.262-2.484 2.187-4 2.695-1.516.504-2.949.758-4.336.758-1.937 0-3.41-.508-4.422-1.559-1.054-1.01-1.558-2.484-1.558-4.464 0-2.106.675-3.704 2.062-4.84 1.391-1.137 3.454-1.684 6.274-1.684zM118 73.348c-4.432.063-9.664 1.052-13.621 3.832-1.223.883-1.012 2.062.336 1.894 4.508-.547 14.44-1.726 16.21.547 1.77 2.23-1.976 11.62-3.663 15.79-.504 1.26.59 1.769 1.726.8 7.41-6.231 9.348-19.242 7.832-21.137-.757-.925-4.388-1.79-8.82-1.726zM1.63 75.859c-.926.116-1.347 1.236-.368 2.121 16.508 14.902 38.359 23.872 62.613 23.872 17.305 0 37.43-5.43 51.281-15.66 2.273-1.689.298-4.254-2.02-3.204-15.533 6.57-32.421 9.77-47.788 9.77-22.778 0-44.8-6.273-62.653-16.633-.39-.231-.755-.304-1.064-.266z";
const AZURE_LOGO: &str = "M43.983 4.653a5.911 5.911 0 015.6 4.022l35.91 106.396a5.911 5.911 0 01-5.603 7.802h41.38a5.917 5.917 0 004.8-2.465 5.909 5.909 0 00.798-5.34L90.961 8.672a5.91 5.91 0 00-5.602-4.022zm-1.336.478a5.92 5.92 0 00-5.61 4.029L1.132 115.55a5.91 5.91 0 005.6 7.8h28.893c1.239 0 2.446-.41 3.452-1.113a5.923 5.923 0 002.157-2.916l7.019-20.71-13.411-12.857c-.246-.273-1.353-2.274-.369-4.002 1.108-1.659 2.955-1.659 2.955-1.659h17.285l9.074-26.145L48.274 8.321c-.042-.205-.914-1.365-2.281-2.28-1.37-.915-3.345-.909-3.345-.909zm-4.88 75.74a2.724 2.724 0 00-1.86 4.718l37.83 35.31c1.101 1.03 2.502 1.631 4.007 1.631 0 0 1.282.068 2.055-.033 1.817-.273 3.525-1.768 4.09-2.39 1.457-1.939.794-4.95.794-4.95l-11.45-34.28z";

/// A source in File › Add Cluster…: Kubernetes for `None`, else the cloud.
#[component]
pub fn ProviderLogo(provider: Option<subduer_types::CloudProvider>) -> impl IntoView {
    use subduer_types::CloudProvider;
    let (mark, class, view_box) = match provider {
        None => (KUBERNETES_LOGO, "provider-logo kubernetes", "0 0 24 24"),
        Some(CloudProvider::Gke) => (GOOGLE_CLOUD_LOGO, "provider-logo google-cloud", "0 0 24 24"),
        Some(CloudProvider::Eks) => (AWS_LOGO, "provider-logo aws", "0 0 128 128"),
        Some(CloudProvider::Aks) => (AZURE_LOGO, "provider-logo azure", "0 0 128 128"),
    };
    view! {
        <svg class=class viewBox=view_box fill="currentColor" aria-hidden="true"><path d=mark /></svg>
    }
}

/// A production-looking context: a filled warning sign, red with a white mark.
#[component]
pub fn ProdIcon() -> impl IntoView {
    view! {
        <svg class="prod-icon" viewBox="0 0 16 16" role="img" aria-label="Looks like production">
            <title>"Looks like production"</title>
            <path fill="currentColor" d="M7.13 1.8a1 1 0 0 1 1.74 0l6.6 11.7a1 1 0 0 1-.87 1.5H1.4a1 1 0 0 1-.87-1.5z" />
            <rect fill="#fff" x="7.2" y="5.4" width="1.6" height="4.9" rx="0.8" />
            <circle fill="#fff" cx="8" cy="12.3" r="0.95" />
        </svg>
    }
}

/// Open/closed indicator for collapsible groups; rotates when `open`.
#[component]
pub fn Chevron(#[prop(into)] open: Signal<bool>) -> impl IntoView {
    view! {
        <svg
            class="chevron"
            class:open=move || open.get()
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.6"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
        >
            <path d="M9 5.5l6.5 6.5L9 18.5" />
        </svg>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(group: &str, kind: &str) -> KindRef {
        KindRef { group: group.into(), version: "v1".into(), kind: kind.into(), plural: String::new(), namespaced: true }
    }

    #[test]
    fn well_known_custom_resources() {
        assert_eq!(custom_paths(&kind("keda.sh", "ScaledObject")), Some(SCALE));
        assert_eq!(custom_paths(&kind("external-secrets.io", "ClusterSecretStore")), Some(VAULT));
        assert_eq!(custom_paths(&kind("kyverno.io", "PolicyException")), Some(SHIELD_OFF));
        assert_eq!(custom_paths(&kind("wgpolicyk8s.io", "ClusterPolicyReport")), Some(CLIPBOARD));
        assert_eq!(custom_paths(&kind("elbv2.k8s.aws", "TargetGroupBinding")), Some(TARGET));
        assert_eq!(custom_paths(&kind("gateway.k8s.aws", "LoadBalancerConfiguration")), Some(SLIDERS));
        assert_eq!(custom_paths(&kind("s3.services.k8s.aws", "Bucket")), Some(CLOUD));
        assert_eq!(custom_paths(&kind("networking.gke.io", "ManagedCertificate")), Some(SHIELD_CHECK));
        assert_eq!(custom_paths(&kind("cloud.google.com", "BackendConfig")), Some(SLIDERS));
        assert_eq!(custom_paths(&kind("resources.azure.com", "ResourceGroup")), Some(CLOUD));
        assert_eq!(custom_paths(&kind("monitoring.coreos.com", "ServiceMonitor")), Some(EYE));
        assert_eq!(custom_paths(&kind("monitoring.coreos.com", "PrometheusRule")), Some(LIST));
        assert_eq!(custom_paths(&kind("monitoring.coreos.com", "Prometheus")), Some(CHART));
        assert_eq!(custom_paths(&kind("example.com", "Widget")), None);
        assert_eq!(custom_paths(&kind("k8s.aws.example.com", "Widget")), None);
    }
}
