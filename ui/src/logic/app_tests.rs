use super::*;
use subduer_types::containers::encode;
use subduer_types::{ContextRemoved, ObjectDetails};

fn kind(group: &str, kind: &str) -> KindRef {
    KindRef { group: group.into(), version: "v1".into(), kind: kind.into(), plural: format!("{}s", kind.to_lowercase()), namespaced: true }
}

fn cluster_kind(group: &str, k: &str, namespaced: bool) -> KindRef {
    KindRef { namespaced, ..kind(group, k) }
}

fn info(k: KindRef, category: Category) -> KindInfo {
    KindInfo { kind: k, verbs: vec![], category }
}

fn context(name: &str, namespace: Option<&str>) -> ContextInfo {
    ContextInfo { name: name.into(), cluster: None, namespace: namespace.map(str::to_owned), prod_like: false, unlocked: false, provider: None }
}

fn row(namespace: Option<&str>, name: &str) -> ResourceRow {
    ResourceRow {
        id: namespace.map_or_else(|| name.to_owned(), |ns| format!("{ns}/{name}")),
        name: name.into(),
        namespace: namespace.map(str::to_owned),
        cells: vec![],
    }
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
    let kinds = vec![info(kind("", "Pod"), Category::Workloads), info(kind("apps", "Deployment"), Category::Workloads)];
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
    assert_eq!(visible_headers(&cols, true), ["Name", "Age"]);
    assert_eq!(column_headers(&cols), ["Name", "Namespace", "Age"]);
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
    assert_eq!(column_title(ColumnKind::Containers), CONTAINER_LEGEND);
    assert_eq!(column_title(ColumnKind::Name), "");
}

#[test]
fn sidebar_sections_get_a_colour_class() {
    assert_eq!(category_class(Category::Workloads), "cat-workloads");
    assert_eq!(category_class(Category::AccessControl), "cat-access-control");
    assert_eq!(category_class(Category::GoogleCloud), "cat-google-cloud");
    assert_eq!(category_class(Category::CustomResources), "cat-custom-resources");
    assert_eq!(kind_section_class(Category::Config), "kind-section cat-config");
    // Every section has a tint defined in the stylesheet, for both themes.
    let css = include_str!("../../styles/app.css");
    for c in Category::SECTIONS {
        let class = category_class(c);
        assert!(css.contains(&format!(".{class} {{ --cat: var(--{class})")), "{class}");
        let token = format!("--{class}:");
        assert_eq!(css.matches(&token).count(), 3, "{token} in light and both dark blocks");
    }
}

#[test]
fn resource_cells_sort_by_their_kind() {
    use crate::logic::sort::parse_utc_ms;
    use subduer_types::containers::Container;
    let c = |state| Container { name: "c".into(), state, detail: String::new(), init: false };
    assert_eq!(cell_key(ColumnKind::Number, "3 (5m ago)", parse_utc_ms), Key::Num(3.0));
    assert_eq!(cell_key(ColumnKind::Text, "Web-2", parse_utc_ms), Key::Text("web-2".into()));
    assert_eq!(cell_key(ColumnKind::Age, "", parse_utc_ms), Key::Missing);
    assert!(matches!(cell_key(ColumnKind::Age, "2026-10-16T12:00:00Z", parse_utc_ms), Key::Num(n) if n < 0.0), "younger is smaller");
    let ready = cell_key(ColumnKind::Containers, &encode(&[c(State::Running), c(State::Running)]), parse_utc_ms);
    let half = cell_key(ColumnKind::Containers, &encode(&[c(State::Running), c(State::Starting)]), parse_utc_ms);
    let failed = cell_key(ColumnKind::Containers, &encode(&[c(State::Running), c(State::Failed)]), parse_utc_ms);
    assert_eq!((ready, half), (Key::Num(1.0), Key::Num(0.5)));
    assert_eq!(failed, Key::Num(-0.5), "failures sort first");
    assert_eq!(cell_key(ColumnKind::Containers, "", parse_utc_ms), Key::Missing);
}

#[test]
fn table_rows_filter_and_sort() {
    let table = || ResourceTable {
        watch_id: 1,
        columns: vec![Column { header: "Name".into(), kind: ColumnKind::Name }, Column { header: "Restarts".into(), kind: ColumnKind::Number }],
        rows: [("web-10", "3"), ("Web-2", "1"), ("api", "2")]
            .map(|(name, restarts)| ResourceRow { cells: vec![name.into(), restarts.into()], ..row(Some("shop"), name) })
            .into(),
        error: None,
        stopped: false,
    };
    let ids = |rows: Vec<ResourceRow>| rows.into_iter().map(|r| r.name).collect::<Vec<_>>();
    let parse = crate::logic::sort::parse_utc_ms;
    assert_eq!(ids(table_rows(table(), "", None, parse)), ["web-10", "Web-2", "api"], "the table's own order");
    assert_eq!(ids(table_rows(table(), "WEB", Some((0, false)), parse)), ["Web-2", "web-10"]);
    assert_eq!(ids(table_rows(table(), "", Some((1, true)), parse)), ["web-10", "api", "Web-2"]);
}

#[test]
fn table_header_and_watch_problems() {
    let cluster_wide = cluster_kind("", "Node", false);
    assert_eq!(scope_text(Some(&cluster_wide), Some("shop")), "cluster-scoped");
    assert_eq!(scope_text(Some(&kind("", "Pod")), Some("shop")), "in shop");
    assert_eq!(scope_text(None, None), "in all namespaces");
    let mut t = ResourceTable { watch_id: 1, columns: vec![], rows: vec![], error: None, stopped: false };
    assert_eq!(watch_problem(&t), None);
    t.error = Some("timed out".into());
    assert_eq!(watch_problem(&t), Some(("timed out".into(), false)));
    t.stopped = true;
    assert_eq!(watch_problem(&t), Some(("timed out".into(), true)));
}

#[test]
fn kind_predicates() {
    let pod = kind("", "Pod");
    assert!(is_kind(Some(&pod), "core/Pod"));
    assert!(!is_kind(Some(&pod), "core/Node"));
    assert!(!is_kind(None, "core/Pod"));
    assert!(is_node_page(false, Some(&kind("", "Node"))));
    assert!(!is_node_page(true, Some(&kind("", "Node"))));
    assert!(!is_node_page(false, Some(&pod)));
    assert!(has_details(Some(&pod)));
    assert!(has_details(Some(&kind("apps", "Deployment"))));
    assert!(has_details(Some(&kind("", "ConfigMap"))));
    assert!(!has_details(Some(&kind("", "Event"))));
    assert!(!has_details(None));
    assert!(is_object(Some(&kind("", "ConfigMap"))));
    assert!(!is_object(Some(&pod)));
    assert!(!is_object(None));
}

#[test]
fn contexts_by_name() {
    let list = [context("a", None), context("b", Some("shop"))];
    assert_eq!(context_named(&list, Some("b")), Some(context("b", Some("shop"))));
    assert_eq!(context_named(&list, Some("z")), None);
    assert_eq!(context_named(&list, None), None);
}

#[test]
fn the_context_to_connect_to_when_one_appears() {
    let list = |current: Option<&str>| ContextList { contexts: vec![context("a", None), context("b", None)], current: current.map(str::to_owned) };
    assert_eq!(first_context(&list(Some("b"))).as_deref(), Some("b"));
    assert_eq!(first_context(&list(Some("gone"))).as_deref(), Some("a"));
    assert_eq!(first_context(&list(None)).as_deref(), Some("a"));
    assert_eq!(first_context(&ContextList { contexts: vec![], current: None }), None);
}

#[test]
fn connecting_keeps_or_picks_the_namespace_and_kind() {
    assert!(resumes(true, Some("a"), "a"));
    assert!(!resumes(true, Some("a"), "b"));
    assert!(!resumes(false, Some("a"), "a"));
    let contexts = [context("a", Some("shop")), context("b", None)];
    let ns: Vec<String> = ["shop", "web"].map(String::from).into();
    // Resuming keeps the namespace the cluster still has, or all namespaces.
    assert_eq!(namespace_after_connect(true, Some("web".into()), &contexts, "a", &ns).as_deref(), Some("web"));
    assert_eq!(namespace_after_connect(true, None, &contexts, "a", &ns), None);
    // Otherwise (or if it's gone): the context's default, if the cluster has it.
    assert_eq!(namespace_after_connect(true, Some("gone".into()), &contexts, "a", &ns).as_deref(), Some("shop"));
    assert_eq!(namespace_after_connect(false, Some("web".into()), &contexts, "a", &ns).as_deref(), Some("shop"));
    assert_eq!(namespace_after_connect(false, None, &contexts, "b", &ns), None);
    assert_eq!(namespace_after_connect(false, None, &contexts, "a", &["web".into()]), None);

    let kinds = [info(kind("", "Pod"), Category::Workloads), info(kind("apps", "Deployment"), Category::Workloads)];
    assert_eq!(kind_after_connect(Some(&kind("apps", "Deployment")), &kinds), Some(kind("apps", "Deployment")));
    assert_eq!(kind_after_connect(Some(&kind("batch", "Job")), &kinds), Some(kind("", "Pod")));
    assert_eq!(kind_after_connect(None, &kinds), Some(kind("", "Pod")));
    assert_eq!(kind_after_connect(None, &[]), None);
}

#[test]
fn connection_checks() {
    assert!(should_probe(false, false, true));
    assert!(!should_probe(true, false, true));
    assert!(!should_probe(false, true, true));
    assert!(!should_probe(false, false, false));
    assert!(probe_gives_up(MAX_PROBE_FAILURES, true));
    assert!(!probe_gives_up(MAX_PROBE_FAILURES - 1, true));
    assert!(!probe_gives_up(MAX_PROBE_FAILURES, false));
}

#[test]
fn backend_events() {
    let moved = ContextRemoved { removed: "old".into(), switch_to: "new".into() };
    assert_eq!(removed_notice(&moved), "“old” was removed from your kubeconfig, so this window moved to “new”.");
    assert!(NO_CONTEXTS_NOTICE.contains("no contexts left"));
    assert!(takes_table(5, 5));
    assert!(!takes_table(4, 5));
    let table = ResourceTable { watch_id: 1, columns: vec![], rows: vec![row(Some("shop"), "web")], error: None, stopped: false };
    assert_eq!(pending_row(&table, Some("shop/web")), Some(row(Some("shop"), "web")));
    assert_eq!(pending_row(&table, Some("shop/api")), None);
    assert_eq!(pending_row(&table, None), None);

    let target = EditTarget { kind: kind("", "ConfigMap"), namespace: Some("shop".into()), name: "cfg".into() };
    let cm = kind("", "ConfigMap");
    assert!(edit_shows(Some(&row(Some("shop"), "cfg")), Some(&cm), &target));
    assert!(!edit_shows(Some(&row(Some("web"), "cfg")), Some(&cm), &target));
    assert!(!edit_shows(Some(&row(Some("shop"), "other")), Some(&cm), &target));
    assert!(!edit_shows(Some(&row(Some("shop"), "cfg")), Some(&kind("", "Secret")), &target));
    assert!(!edit_shows(None, Some(&cm), &target));
    assert!(refreshes_details(true, true, true, true));
    assert!(!refreshes_details(false, true, true, true));
    assert!(!refreshes_details(true, true, true, false));
}

#[test]
fn the_main_area_page() {
    assert_eq!(page(true, true, true, true, true), Page::Traces);
    assert_eq!(page(false, true, true, true, true), Page::Forwards);
    assert_eq!(page(false, false, true, true, true), Page::NoClusters);
    assert_eq!(page(false, false, false, true, true), Page::Disconnected);
    assert_eq!(page(false, false, false, false, true), Page::Overview);
    assert_eq!(page(false, false, false, false, false), Page::Resources);
    assert_eq!(not_connected_title(Some("acme")), "Not connected to acme");
    assert_eq!(not_connected_title(None), "Not connected to ");
    assert_eq!(connect_label(true), "Connecting…");
    assert_eq!(connect_label(false), "Connect");
}

#[test]
fn top_bar_namespace_and_lock() {
    assert_eq!(namespace_choice(String::new()), None);
    assert_eq!(namespace_choice("shop".into()).as_deref(), Some("shop"));
    assert_eq!(lock_look(false, false), ("lock".into(), "Read-only", "Click to allow changes to this context."));
    assert_eq!(lock_look(true, false), ("lock unlocked".into(), "Writable", "Changes are allowed. Click to make read-only."));
    assert_eq!(lock_look(true, true).0, "lock unlocked prod");
    assert!(unlock_ok(false, "", "acme-prod"));
    assert!(unlock_ok(true, " acme-prod ", "acme-prod"));
    assert!(!unlock_ok(true, "acme", "acme-prod"));
}

#[test]
fn sidebar_kinds_sections_and_drag() {
    let cluster = ClusterInfo {
        context: "c".into(),
        kinds: vec![
            info(kind("", "Pod"), Category::Workloads),
            info(kind("apps", "Deployment"), Category::Workloads),
            info(kind("", "ConfigMap"), Category::Config),
        ],
        namespaces: vec![],
    };
    assert_eq!(kinds_in(Some(&cluster), Category::Workloads, ""), [kind("", "Pod"), kind("apps", "Deployment")]);
    assert_eq!(kinds_in(Some(&cluster), Category::Workloads, "DEPLOY"), [kind("apps", "Deployment")]);
    assert!(kinds_in(None, Category::Workloads, "").is_empty());

    let mut expanded = HashSet::new();
    assert!(!section_open("", &expanded, Category::Config));
    assert!(section_open("cfg", &expanded, Category::Config), "searching opens every section");
    toggle_section(&mut expanded, Category::Config);
    assert!(section_open("", &expanded, Category::Config));
    toggle_section(&mut expanded, Category::Config);
    assert!(expanded.is_empty());

    assert!(is_drop_target(Some(Category::Config), Some(Category::Helm), Category::Config));
    assert!(!is_drop_target(Some(Category::Config), Some(Category::Config), Category::Config));
    assert!(!is_drop_target(None, Some(Category::Helm), Category::Config));
}

#[test]
fn sidebar_active_entries() {
    assert!(cluster_active(true, false, false));
    assert!(!cluster_active(true, true, false));
    assert!(!cluster_active(true, false, true));
    assert!(!cluster_active(false, false, false));
    assert!(forwards_active(true, false));
    assert!(!forwards_active(true, true));
    let pod = kind("", "Pod");
    assert!(kind_active(false, false, false, Some(&pod), "core/Pod"));
    assert!(!kind_active(true, false, false, Some(&pod), "core/Pod"));
    assert!(!kind_active(false, true, false, Some(&pod), "core/Pod"));
    assert!(!kind_active(false, false, true, Some(&pod), "core/Pod"));
    assert!(!kind_active(false, false, false, Some(&pod), "core/Node"));
}

#[test]
fn group_headings_start_each_api_group() {
    let kinds = [kind("a.io", "X"), kind("a.io", "Y"), kind("b.io", "Z"), kind("a.io", "W")];
    assert_eq!(group_headings(true, &kinds), [true, false, true, true]);
    assert_eq!(group_headings(false, &kinds), [false, false, false, false]);
    assert_eq!(kind_title(&kind("apps", "Deployment")), "deployments (apps/v1)");
}

#[test]
fn container_icons_say_their_state() {
    let c = |state, init| Container { name: "web".into(), state, detail: "OOMKilled".into(), init };
    assert_eq!(container_look(&c(State::Running, false)), ("ctr running", "web: OOMKilled".into(), "web running".into()));
    assert_eq!(container_look(&c(State::Starting, false)).0, "ctr starting");
    assert_eq!(container_look(&c(State::Stopped, false)).0, "ctr stopped");
    assert_eq!(
        container_look(&c(State::Failed, true)),
        ("ctr failed", "init container web: OOMKilled".into(), "init container web failed".into())
    );
}

#[test]
fn settings_discard_and_detail_panel() {
    assert_eq!(theme_note(Theme::Auto), "Follows your system's light or dark appearance.");
    assert_eq!(theme_note(Theme::Light), "Always light.");
    assert_eq!(theme_note(Theme::Dark), "Always dark.");
    assert_eq!(dirty_what(Some("ConfigMap cfg".into())), "ConfigMap cfg");
    assert_eq!(dirty_what(None), "this object");
    assert!(loads_yaml(true, false));
    assert!(!loads_yaml(true, true));
    assert!(!loads_yaml(false, false));
    assert!(esc_closes_panel(true, false, true, false));
    assert!(!esc_closes_panel(false, false, true, false));
    assert!(!esc_closes_panel(true, true, true, false));
    assert!(!esc_closes_panel(true, false, false, false));
    assert!(!esc_closes_panel(true, false, true, true));
}

#[test]
fn opening_objects_from_links() {
    let kinds = [info(kind("", "Pod"), Category::Workloads), info(cluster_kind("", "Node", false), Category::TopLevel)];
    let pod = kind("", "Pod");
    let plan = |key: &str, ns: Option<&str>, name: &str, current: Option<&KindRef>, filter: Option<&str>| {
        open_plan(&kinds, key, ns.map(str::to_owned), name.to_owned(), current, filter.map(str::to_owned))
    };
    assert_eq!(plan("batch/Job", None, "x", None, None), OpenPlan::Missing("batch/Job isn't listed on this cluster".into()));
    assert_eq!(plan("core/Pod", Some("shop"), "", None, None), OpenPlan::List { kind: pod.clone(), namespace: Some("shop".into()) });
    // A cluster-scoped kind drops the namespace a link gave.
    assert_eq!(
        plan("core/Node", Some("shop"), "n1", None, Some("shop")),
        OpenPlan::Select { kind: cluster_kind("", "Node", false), id: "n1".into(), filter: Some("shop".into()), same_list: false }
    );
    // Same list, filter widened from another namespace.
    assert_eq!(
        plan("core/Pod", Some("shop"), "web", Some(&pod), Some("other")),
        OpenPlan::Select { kind: pod.clone(), id: "shop/web".into(), filter: Some("shop".into()), same_list: true }
    );
    assert_eq!(
        plan("core/Pod", Some("shop"), "web", Some(&pod), None),
        OpenPlan::Select { kind: pod.clone(), id: "shop/web".into(), filter: None, same_list: true }
    );
    assert_eq!(
        plan("core/Pod", Some("shop"), "web", Some(&kind("apps", "Deployment")), Some("shop")),
        OpenPlan::Select { kind: pod.clone(), id: "shop/web".into(), filter: Some("shop".into()), same_list: false }
    );
}

fn details(name: &str, logs: Option<LogTarget>) -> ObjectDetails {
    ObjectDetails {
        kind: "ScaledObject".into(),
        name: name.into(),
        namespace: Some("shop".into()),
        status: None,
        info: vec![],
        created: None,
        labels: vec![],
        annotations: vec![],
        conditions: vec![],
        data: None,
        sections: vec![],
        events: vec![],
        node: None,
        logs,
    }
}

#[test]
fn log_buttons() {
    let target = LogTarget {
        namespace: "keda".into(),
        source: LogSource::Workload { kind: kind("apps", "Deployment"), name: "keda-operator".into() },
        filter: Some("web-scaler".into()),
    };
    let r = row(Some("shop"), "web");
    assert_eq!(object_logs(Some(&Ok(details("web", Some(target.clone())))), &r), Some(target.clone()));
    assert_eq!(object_logs(Some(&Ok(details("other", Some(target.clone())))), &r), None);
    assert_eq!(object_logs(Some(&Err("no".into())), &r), None);
    assert_eq!(object_logs(None, &r), None);
    assert_eq!(object_logs_title(&target, "web"), "Open the logs of keda-operator in a new window, filtered to web-scaler");
    let pod_logs = LogTarget {
        namespace: "keda".into(),
        source: LogSource::Container { pod: "keda-0".into(), container: "c".into() },
        filter: None,
    };
    assert_eq!(object_logs_title(&pod_logs, "web"), "Open the logs of keda-0 in a new window, filtered to web");
    assert_eq!(workload_logs_title(&kind("apps", "Deployment"), "web"), "Open logs from every pod of Deployment web in a new window");
}
