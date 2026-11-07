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
    assert_eq!(theme_from(Some("light"), &yes), Theme::Light);
    assert_eq!(theme_from(Some("dark"), &yes), Theme::Dark);
    assert_eq!(theme_from(Some("auto"), &yes), Theme::Auto);
    assert_eq!(theme_from(None, &yes), Theme::Auto);
    assert_eq!(theme_from(Some("purple"), &yes), Theme::Auto);
    // Without system support, Auto falls back to Light.
    assert_eq!(theme_from(Some("auto"), &no), Theme::Light);
    assert_eq!(theme_from(None, &no), Theme::Light);
    assert_eq!(theme_from(Some("dark"), &no), Theme::Dark);
    // An explicit choice doesn't ask the system.
    assert_eq!(theme_from(Some("light"), &|| panic!("asked")), Theme::Light);
}

#[test]
fn pull_every_from_saved_value() {
    assert_eq!(pull_every_from(Some("1800")), 1800);
    assert_eq!(pull_every_from(None), 600);
    // No longer a choice, or not a number.
    assert_eq!(pull_every_from(Some("300")), 600);
    assert_eq!(pull_every_from(Some("soon")), 600);
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

#[test]
fn accent_text_keeps_contrast() {
    // macOS blue, graphite, red: white text.
    assert_eq!(accent_text("#007aff"), Some("#fff"));
    assert_eq!(accent_text("#8c8c8c"), Some("#fff"));
    assert_eq!(accent_text("#e0383e"), Some("#fff"));
    // Green, orange, yellow are too light for white.
    assert_eq!(accent_text("#62ba46"), Some("#1c2430"));
    assert_eq!(accent_text("#f7821b"), Some("#1c2430"));
    assert_eq!(accent_text("#ffc600"), Some("#1c2430"));
    for bad in ["", "#", "007aff", "#07f", "#00zzff", "#007aff00", "#ééé"] {
        assert_eq!(accent_text(bad), None, "{bad}");
    }
}

#[test]
fn transparency_is_on_unless_turned_off() {
    assert!(transparency_from(None));
    assert!(transparency_from(Some("on")));
    assert!(transparency_from(Some("bogus")));
    assert!(!transparency_from(Some("off")));
}

#[test]
fn dark_follows_the_theme_or_the_system() {
    assert!(theme_is_dark(Theme::Dark, &|| panic!("asked")));
    assert!(!theme_is_dark(Theme::Light, &|| panic!("asked")));
    assert!(theme_is_dark(Theme::Auto, &|| true));
    assert!(!theme_is_dark(Theme::Auto, &|| false));
    assert!(media_supported("(prefers-color-scheme: dark)"));
    assert!(!media_supported("not all"));
}

#[test]
fn transparency_saved_form() {
    assert_eq!(transparency_text(true), "on");
    assert_eq!(transparency_text(false), "off");
    assert!(!transparency_from(Some(transparency_text(false))));
}

#[test]
fn accent_style_needs_a_colour() {
    assert_eq!(accent_style(Some("#007aff")).as_deref(), Some("--os-accent: #007aff; --accent-text: #fff"));
    assert_eq!(accent_style(Some("blue")), None);
    assert_eq!(accent_style(None), None);
}

#[test]
fn capture_image_keys_are_per_context() {
    assert_eq!(capture_image_key("acme-uat"), "capture.image.acme-uat");
}

#[test]
fn recent_traces_round_trip_and_forget() {
    let t = |path: &str| RecentTrace { path: path.into(), context: "c".into(), what: "w".into(), at: 1.0 };
    let list = vec![t("/a"), t("/b")];
    assert_eq!(recent_from(Some(&recent_text(&list))), list);
    assert!(recent_from(Some("not json")).is_empty());
    assert!(recent_from(None).is_empty());
    assert_eq!(without_recent(list, "/a"), [t("/b")]);
}
