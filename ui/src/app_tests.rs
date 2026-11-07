use super::*;
use crate::prefs::Theme;
use subduer_types::{ContextInfo, ResourceRow};

pub(crate) fn kind(group: &str, kind: &str) -> KindRef {
    KindRef {
        group: group.into(),
        version: "v1".into(),
        kind: kind.into(),
        plural: format!("{}s", kind.to_lowercase()),
        namespaced: true,
    }
}

pub(crate) fn context(name: &str, unlocked: bool) -> ContextInfo {
    ContextInfo { name: name.into(), cluster: None, namespace: None, prod_like: false, unlocked, provider: None }
}

pub(crate) fn row(namespace: Option<&str>, name: &str) -> ResourceRow {
    ResourceRow {
        id: namespace.map_or_else(|| name.to_owned(), |ns| format!("{ns}/{name}")),
        name: name.into(),
        namespace: namespace.map(str::to_owned),
        cells: vec![],
    }
}

/// A store with default preferences, at time zero.
pub(crate) fn store() -> Store {
    Store::with(Category::SECTIONS.to_vec(), Theme::Light, 0.0, ColumnWidths::from_saved(""))
}

/// A store connected to `acme-dev` (locked or not).
pub(crate) fn store_on(unlocked: bool) -> Store {
    let s = store();
    s.contexts.set(vec![context("acme-prod", false), context("acme-dev", unlocked)]);
    s.context.set(Some("acme-dev".into()));
    s
}

#[test]
fn move_section_to_itself_is_a_no_op() {
    let s = store();
    move_section(s, Category::Config, Category::Config);
    assert_eq!(s.section_order.get_untracked(), Category::SECTIONS.to_vec());
}

#[test]
fn end_drag_clears_drag_state() {
    let s = store();
    s.dragging.set(Some(Category::Config));
    s.drop_target.set(Some(Category::Helm));
    end_drag(s);
    assert_eq!(s.dragging.get_untracked(), None);
    assert_eq!(s.drop_target.get_untracked(), None);
}

#[test]
fn new_store_starts_on_the_cluster_page() {
    let s = store();
    assert!(s.overview_open.get_untracked());
    assert!(s.detail_tab.get_untracked() == DetailTab::Yaml);
    assert_eq!(s.theme.get_untracked(), Theme::Light);
    assert_eq!(s.now.get_untracked(), 0.0);
    assert!(s.needs_overview());
    assert!(!s.node_page());
}

#[test]
fn current_context_follows_the_selected_name() {
    let s = store();
    assert_eq!(s.current_context(), None);
    s.contexts.set(vec![context("acme-dev", false), context("shop-prod", true)]);
    assert_eq!(s.current_context(), None);
    s.context.set(Some("shop-prod".into()));
    assert_eq!(s.current_context(), Some(context("shop-prod", true)));
    assert_eq!(s.current_context_untracked(), Some(context("shop-prod", true)));
    s.context.set(Some("gone".into()));
    assert_eq!(s.current_context_untracked(), None);
}

#[test]
fn node_page_needs_the_node_kind_off_the_cluster_page() {
    let s = store();
    s.kind.set(Some(kind("", "Node")));
    assert!(!s.node_page(), "the cluster page is still open");
    s.overview_open.set(false);
    assert!(s.node_page());
    assert!(s.needs_overview());
    s.kind.set(Some(kind("", "Pod")));
    assert!(!s.node_page());
    assert!(!s.needs_overview());
}

#[test]
fn selected_kind_predicates() {
    let s = store();
    assert!(!selected_is_pod(s));
    assert!(!selected_has_details(s));
    assert!(!selected_is_object(s));

    s.kind.set(Some(kind("", "Pod")));
    assert!(selected_is_pod(s));
    assert!(selected_has_details(s));
    assert!(!selected_is_object(s));

    s.kind.set(Some(kind("apps", "Deployment")));
    assert!(!selected_is_pod(s));
    assert!(selected_has_details(s));

    s.kind.set(Some(kind("", "ConfigMap")));
    assert!(selected_has_details(s));
    assert!(selected_is_object(s));
}

#[test]
fn guard_runs_at_once_without_unsaved_edits() {
    let s = store();
    s.editing.set(true);
    let ran = RwSignal::new(false);
    guard(s, move || ran.set(true));
    assert!(ran.get_untracked());
    assert!(!s.editing.get_untracked(), "a clean edit just ends");
    assert!(!s.discard_prompt.get_untracked());
}

#[test]
fn guard_asks_before_discarding_edits() {
    let s = store();
    s.editing.set(true);
    s.dirty.set(Some("ConfigMap shop-config".into()));
    let ran = RwSignal::new(false);
    guard(s, move || ran.set(true));
    assert!(!ran.get_untracked());
    assert!(s.discard_prompt.get_untracked());
    assert!(s.editing.get_untracked());
    // Discarding runs what was held.
    let then = s.discard_then.try_update_value(Option::take).flatten().expect("held action");
    then();
    assert!(ran.get_untracked());
}
