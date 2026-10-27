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
fn format_span_picks_units_at_boundaries() {
    assert_eq!(format_span(0), "0s");
    assert_eq!(format_span(45), "45s");
    assert_eq!(format_span(119), "119s");
    assert_eq!(format_span(120), "2m");
    assert_eq!(format_span(2 * 3600 - 1), "119m");
    assert_eq!(format_span(2 * 3600), "2h");
    assert_eq!(format_span(2 * 86400 - 1), "47h");
    assert_eq!(format_span(2 * 86400), "2d");
    assert_eq!(format_span(365 * 86400 - 1), "364d");
    assert_eq!(format_span(365 * 86400), "1y");
    assert_eq!(format_span(3 * 365 * 86400 + 5), "3y");
}

#[test]
fn age_since_formats_elapsed_time() {
    assert_eq!(age_since(90_000.0, 0.0), "90s");
    assert_eq!(age_since(3_600_000.0, 0.0), "60m");
    // Clock skew: a creation time in the future reads as zero.
    assert_eq!(age_since(0.0, 5_000.0), "0s");
    // Unparseable times show nothing.
    assert_eq!(age_since(1_000.0, f64::NAN), "");
}

#[test]
fn find_kind_matches_by_key() {
    let kinds = vec![
        KindInfo { kind: kind("", "Pod"), verbs: vec![], category: Category::Workloads },
        KindInfo { kind: kind("apps", "Deployment"), verbs: vec![], category: Category::Workloads },
    ];
    assert_eq!(find_kind(&kinds, "core/Pod"), Some(kind("", "Pod")));
    assert_eq!(find_kind(&kinds, "apps/Deployment"), Some(kind("apps", "Deployment")));
    assert_eq!(find_kind(&kinds, "Pod"), None);
    assert_eq!(find_kind(&[], "core/Pod"), None);
}

#[test]
fn kind_filters_match_name_plural_or_group() {
    assert!(kind_matches_text("Port Forwards", ""));
    assert!(kind_matches_text("Port Forwards", "forw"));
    assert!(!kind_matches_text("Port Forwards", "Forw"), "the filter is already lowercased");
    assert!(!kind_matches_text("Cluster", "pod"));

    let deploy = kind("apps", "Deployment");
    assert!(kind_matches(&deploy, ""));
    assert!(kind_matches(&deploy, "deploy"));
    assert!(kind_matches(&deploy, "deployments"));
    assert!(kind_matches(&deploy, "apps"));
    assert!(!kind_matches(&deploy, "batch"));
}

#[test]
fn reorder_moves_a_section_to_the_target() {
    use Category::*;
    let mut order = vec![Workloads, Config, Network, Storage];
    reorder(&mut order, Storage, Config);
    assert_eq!(order, [Workloads, Storage, Config, Network]);
    reorder(&mut order, Workloads, Network);
    assert_eq!(order, [Storage, Config, Network, Workloads]);
    // Unknown sections change nothing.
    reorder(&mut order, Helm, Config);
    reorder(&mut order, Config, Helm);
    assert_eq!(order, [Storage, Config, Network, Workloads]);
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
fn visible_hides_namespace_column_for_one_namespace() {
    let cols = vec![
        Column { header: "Name".into(), kind: ColumnKind::Name },
        Column { header: "Namespace".into(), kind: ColumnKind::Namespace },
        Column { header: "Age".into(), kind: ColumnKind::Age },
    ];
    let all: Vec<usize> = visible(&cols, false).map(|(i, _)| i).collect();
    assert_eq!(all, [0, 1, 2]);
    let one: Vec<(usize, &str)> = visible(&cols, true).map(|(i, c)| (i, c.header.as_str())).collect();
    assert_eq!(one, [(0, "Name"), (2, "Age")], "cell indexes are kept");
}

#[test]
fn column_class_per_kind() {
    assert_eq!(column_class(ColumnKind::Name), "name");
    assert_eq!(column_class(ColumnKind::LongText), "long");
    assert_eq!(column_class(ColumnKind::Number), "num");
    assert_eq!(column_class(ColumnKind::Age), "age");
    for k in [ColumnKind::Namespace, ColumnKind::Text, ColumnKind::Badge, ColumnKind::Containers] {
        assert_eq!(column_class(k), "");
    }
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

#[test]
fn sidebar_sections_get_a_colour_class() {
    assert_eq!(category_class(Category::Workloads), "cat-workloads");
    assert_eq!(category_class(Category::AccessControl), "cat-access-control");
    assert_eq!(category_class(Category::GoogleCloud), "cat-google-cloud");
    assert_eq!(category_class(Category::CustomResources), "cat-custom-resources");
    // Every section has a tint defined in the stylesheet, for both themes.
    let css = include_str!("../styles/app.css");
    for c in Category::SECTIONS {
        let class = category_class(c);
        assert!(css.contains(&format!(".{class} {{ --cat: var(--{class})")), "{class}");
        let token = format!("--{class}:");
        assert_eq!(css.matches(&token).count(), 3, "{token} in light and both dark blocks");
    }
}

#[test]
fn resource_cells_sort_by_their_kind() {
    use crate::sort::Key;
    use subduer_types::containers::{Container, State, encode};
    let c = |state| Container { name: "c".into(), state, detail: String::new(), init: false };
    assert_eq!(cell_key(ColumnKind::Number, "3 (5m ago)"), Key::Num(3.0));
    assert_eq!(cell_key(ColumnKind::Text, "Web-2"), Key::Text("web-2".into()));
    assert_eq!(cell_key(ColumnKind::Age, ""), Key::Missing);
    assert!(matches!(cell_key(ColumnKind::Age, "2026-10-16T12:00:00Z"), Key::Num(n) if n < 0.0), "younger is smaller");
    let ready = cell_key(ColumnKind::Containers, &encode(&[c(State::Running), c(State::Running)]));
    let half = cell_key(ColumnKind::Containers, &encode(&[c(State::Running), c(State::Starting)]));
    let failed = cell_key(ColumnKind::Containers, &encode(&[c(State::Running), c(State::Failed)]));
    assert_eq!((ready, half), (Key::Num(1.0), Key::Num(0.5)));
    assert_eq!(failed, Key::Num(-0.5), "failures sort first");
    assert_eq!(cell_key(ColumnKind::Containers, ""), Key::Missing);
}
