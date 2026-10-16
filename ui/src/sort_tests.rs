use super::*;

fn sorted(items: &[&str], by: Option<(usize, bool)>, key: fn(&str) -> Key) -> Vec<String> {
    let mut v: Vec<String> = items.iter().map(|s| (*s).to_owned()).collect();
    sort(&mut v, by, |s, _| key(s));
    v
}

#[test]
fn quantities_compare_by_value() {
    assert_eq!(quantity("20Gi"), Some(20.0 * 1024f64.powi(3)));
    assert_eq!(quantity("16.9 GiB"), Some(16.9 * 1024f64.powi(3)));
    assert_eq!(quantity("250m"), Some(0.25));
    assert_eq!(quantity("1,234"), Some(1234.0));
    assert_eq!(quantity("45%"), Some(45.0));
    assert_eq!(quantity("<1%"), Some(1.0));
    assert_eq!(quantity("3 (5m ago)"), Some(3.0));
    assert_eq!(quantity("2 cores"), Some(2.0));
    assert_eq!(quantity("-1"), Some(-1.0));
    assert_eq!(quantity("none"), None);
    assert_eq!(quantity(""), None);
    let order = sorted(&["2Gi", "512Mi", "1.5 GiB", "100Ki"], Some((0, false)), Key::quantity);
    assert_eq!(order, ["100Ki", "512Mi", "1.5 GiB", "2Gi"]);
}

#[test]
fn natural_order_reads_numbers_in_names() {
    assert_eq!(natural("pod-2", "pod-10"), Ordering::Less);
    assert_eq!(natural("pod-010", "pod-10"), Ordering::Equal);
    assert_eq!(natural("a", "ab"), Ordering::Less);
    assert_eq!(natural("b", "a"), Ordering::Greater);
    assert_eq!(natural("2/3", "10/10"), Ordering::Less);
    let order = sorted(&["web-10", "Web-2", "api", "web-1"], Some((0, false)), Key::text);
    assert_eq!(order, ["api", "web-1", "Web-2", "web-10"]);
}

#[test]
fn missing_values_stay_last_both_ways() {
    let items = ["3", "", "1", "—", "2"];
    assert_eq!(sorted(&items, Some((0, false)), Key::quantity), ["1", "2", "3", "", "—"]);
    assert_eq!(sorted(&items, Some((0, true)), Key::quantity), ["3", "2", "1", "", "—"]);
    assert_eq!(sorted(&items, None, Key::quantity), items, "no sort keeps the table's order");
}

#[test]
fn numbers_come_before_text_and_ties_keep_their_order() {
    let order = sorted(&["n/a", "5", "b", "1"], Some((0, false)), Key::quantity);
    assert_eq!(order, ["1", "5", "b", "n/a"]);
    let mut pairs = vec![("x", 1), ("y", 0), ("z", 1), ("w", 0)];
    sort(&mut pairs, Some((1, false)), |p, _| Key::num(f64::from(p.1)));
    assert_eq!(pairs, [("y", 0), ("w", 0), ("x", 1), ("z", 1)]);
    sort(&mut pairs, Some((1, true)), |p, _| Key::num(f64::from(p.1)));
    assert_eq!(pairs, [("x", 1), ("z", 1), ("y", 0), ("w", 0)]);
}

#[test]
fn ages_sort_youngest_first_and_times_earliest_first() {
    let times = ["2026-10-16T10:00:00Z", "2026-10-16T12:00:00Z", "", "2025-01-01T00:00:00Z"];
    assert_eq!(
        sorted(&times, Some((0, false)), Key::age),
        ["2026-10-16T12:00:00Z", "2026-10-16T10:00:00Z", "2025-01-01T00:00:00Z", ""]
    );
    assert_eq!(
        sorted(&times, Some((0, false)), Key::time),
        ["2025-01-01T00:00:00Z", "2026-10-16T10:00:00Z", "2026-10-16T12:00:00Z", ""]
    );
    assert_eq!(js_date("1970-01-02T00:00:01Z"), 86_401_000.0);
    assert!(js_date("yesterday").is_nan());
    assert_eq!(Key::opt(None), Key::Missing);
    assert_eq!(Key::num(f64::NAN), Key::Missing);
}

#[test]
fn apply_order_handles_every_permutation_of_four() {
    let base = ['a', 'b', 'c', 'd'];
    let mut perms = vec![];
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    let p = [a, b, c, d];
                    if (0..4).all(|i| p.contains(&i)) {
                        perms.push(p);
                    }
                }
            }
        }
    }
    assert_eq!(perms.len(), 24);
    for p in perms {
        let mut v = base;
        apply_order(&mut v, &p);
        assert_eq!(v, p.map(|i| base[i]), "{p:?}");
    }
}

#[test]
fn clicks_cycle_and_are_saved() {
    let owner = Owner::new();
    owner.with(|| {
        let s = Sorts::from_saved("core/Pod\tAge\tdesc\nbroken line\nx\ty\tsideways");
        assert_eq!(s.get("core/Pod"), Some(("Age".into(), true)));
        assert_eq!(s.get("x"), None);
        assert_eq!(s.by("core/Pod", &["Name", "Age"]), Some((1, true)));
        assert_eq!(s.by("core/Pod", &["Name"]), None, "a column that's gone");
        s.click("nodes", "Pods");
        assert_eq!((s.get("nodes"), s.arrow("nodes", "Pods"), s.arrow("nodes", "Name")), (Some(("Pods".into(), false)), "↑", ""));
        s.click("nodes", "Pods");
        assert_eq!((s.get("nodes"), s.arrow("nodes", "Pods")), (Some(("Pods".into(), true)), "↓"));
        s.click("nodes", "Pods");
        assert_eq!(s.get("nodes"), None, "the third click goes back to the table's order");
        s.click("nodes", "Pods");
        s.click("nodes", "Name");
        assert_eq!(s.get("nodes"), Some(("Name".into(), false)), "another column starts ascending");
        assert_eq!(s.saved(), "core/Pod\tAge\tdesc\nnodes\tName\tasc");
        assert_eq!(Sorts::from_saved(&s.saved()).saved(), s.saved());
    });
}
