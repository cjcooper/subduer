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

fn category_paths(category: Category) -> &'static str {
    match category {
        Category::TopLevel => CLUSTER,
        Category::Workloads => CUBE,
        Category::Helm => HELM,
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

#[component]
pub fn KindIcon(kind: KindRef, #[prop(optional)] custom: bool) -> impl IntoView {
    svg(if custom { PUZZLE } else { kind_paths(&kind) })
}

#[component]
pub fn CategoryIcon(category: Category) -> impl IntoView {
    svg(category_paths(category))
}

#[component]
pub fn ClusterIcon() -> impl IntoView {
    svg(CLUSTER)
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
