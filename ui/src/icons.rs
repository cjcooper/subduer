//! Line icons for the sidebar: 24×24 paths drawn with `currentColor`, so they
//! follow the text colour in both themes and in the active state.

use leptos::prelude::*;
use subduer_types::{Category, KindRef};

use crate::logic::icons::{self as logic, CLUSTER, FORWARD, TRACES};

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
    svg(logic::kind_icon(&kind, custom))
}

/// Icon for a custom-resource API group heading; nothing for unknown groups.
#[component]
pub fn GroupIcon(group: String) -> impl IntoView {
    logic::group_paths(&group).map(svg)
}

#[component]
pub fn CategoryIcon(category: Category) -> impl IntoView {
    svg(logic::category_paths(category))
}

#[component]
pub fn ClusterIcon() -> impl IntoView {
    svg(CLUSTER)
}

#[component]
pub fn ForwardIcon() -> impl IntoView {
    svg(FORWARD)
}

/// Folder or file, for the file browser.
#[component]
pub fn EntryIcon(dir: bool) -> impl IntoView {
    svg(logic::entry_paths(dir))
}

#[component]
pub fn LockIcon(#[prop(optional)] open: bool) -> impl IntoView {
    svg(logic::lock_paths(open))
}

#[component]
pub fn TracesIcon() -> impl IntoView {
    svg(TRACES)
}

/// A source in File › Add Cluster…: Kubernetes for `None`, else the cloud.
#[component]
pub fn ProviderLogo(provider: Option<subduer_types::CloudProvider>) -> impl IntoView {
    let (mark, class, view_box) = logic::provider_mark(provider);
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
