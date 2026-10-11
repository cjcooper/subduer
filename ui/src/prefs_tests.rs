use super::*;
use Category::*;

#[test]
fn section_order_defaults_when_nothing_is_saved() {
    assert_eq!(section_order_from(""), Category::SECTIONS.to_vec());
}

#[test]
fn section_order_round_trips() {
    let mut order = Category::SECTIONS.to_vec();
    order.reverse();
    assert_eq!(section_order_from(&section_order_text(&order)), order);
    assert_eq!(section_order_text(&[Workloads, Config, AccessControl]), "Workloads,Config,AccessControl");
}

#[test]
fn section_order_drops_unknown_and_repeated_names() {
    let order = section_order_from("Helm,Bogus,Helm,TopLevel,Workloads");
    assert_eq!(order.iter().filter(|c| **c == Helm).count(), 1);
    assert!(!order.contains(&TopLevel), "not a section");
    assert_eq!(order.len(), Category::SECTIONS.len());
}

#[test]
fn section_order_slots_in_missing_sections() {
    // Each missing section goes before the first saved one (in saved order)
    // that comes after it by default; with none, at the end.
    assert_eq!(
        section_order_from("Other,Storage,Workloads"),
        [Config, Network, Namespaces, AccessControl, Helm, Aws, GoogleCloud, Azure, CustomResources, Other, Storage, Workloads]
    );
    assert_eq!(
        section_order_from("Helm,Workloads"),
        [Config, Network, Storage, Namespaces, AccessControl, Helm, Workloads, Aws, GoogleCloud, Azure, CustomResources, Other]
    );
}

#[test]
fn theme_strings() {
    assert_eq!(Theme::Auto.as_str(), "auto");
    assert_eq!(Theme::Light.as_str(), "light");
    assert_eq!(Theme::Dark.as_str(), "dark");
}

#[test]
fn theme_from_saved_value() {
    let yes = || true;
    let no = || false;
    assert_eq!(theme_from(Some("light"), yes), Theme::Light);
    assert_eq!(theme_from(Some("dark"), yes), Theme::Dark);
    assert_eq!(theme_from(Some("auto"), yes), Theme::Auto);
    assert_eq!(theme_from(None, yes), Theme::Auto);
    assert_eq!(theme_from(Some("purple"), yes), Theme::Auto);
    // Without system support, Auto falls back to Light.
    assert_eq!(theme_from(Some("auto"), no), Theme::Light);
    assert_eq!(theme_from(None, no), Theme::Light);
    assert_eq!(theme_from(Some("dark"), no), Theme::Dark);
    // An explicit choice doesn't ask the system.
    assert_eq!(theme_from(Some("light"), || panic!("asked")), Theme::Light);
}

#[test]
fn desktop_from_user_agent() {
    assert_eq!(desktop_from("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15"), Desktop::Mac);
    assert_eq!(desktop_from("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Edg/120.0"), Desktop::Windows);
    assert_eq!(desktop_from("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15"), Desktop::Linux);
    assert_eq!(desktop_from(""), Desktop::Linux);
}

#[test]
fn reveal_labels() {
    assert_eq!(reveal_label_for(Desktop::Mac), "Show in Finder");
    assert_eq!(reveal_label_for(Desktop::Windows), "Show in Explorer");
    assert_eq!(reveal_label_for(Desktop::Linux), "Show folder");
}

#[test]
fn recent_traces_newest_first_without_repeats() {
    let t = |path: &str, at: f64| RecentTrace { path: path.into(), context: "acme-uat".into(), what: "namespace shop".into(), at };
    let list = with_recent(vec![t("/a", 1.0), t("/b", 2.0)], t("/b", 3.0));
    assert_eq!(list.iter().map(|t| (t.path.as_str(), t.at)).collect::<Vec<_>>(), vec![("/b", 3.0), ("/a", 1.0)]);
    let many = (0..20).fold(vec![], |list, i| with_recent(list, t(&format!("/{i}"), i as f64)));
    assert_eq!(many.len(), 12);
    assert_eq!(many[0].path, "/19");
}
