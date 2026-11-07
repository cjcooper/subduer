use super::*;

#[test]
fn from_saved_reads_tab_separated_lines() {
    let w = WidthState::from_saved("core/Pod\tName\t240\ncore/Pod\tAge\t60\nnodes\tName\t180.5");
    assert_eq!(w.get("core/Pod", "Name"), Some(240.0));
    assert_eq!(w.get("core/Pod", "Age"), Some(60.0));
    assert_eq!(w.get("nodes", "Name"), Some(180.5));
    assert_eq!(w.get("core/Pod", "Status"), None);
    assert!(w.has_table("core/Pod"));
    assert!(w.has_table("nodes"));
    assert!(!w.has_table("apps/Deployment"));
}

#[test]
fn from_saved_skips_malformed_lines() {
    let w = WidthState::from_saved("\nbad\ncore/Pod\tName\nx\ty\twide\ncore/Pod\tAge\t60\t\n");
    assert_eq!(w.get("core/Pod", "Age"), Some(60.0), "extra fields are ignored");
    assert_eq!(w.get("core/Pod", "Name"), None);
    assert_eq!(w.get("x", "y"), None);
    assert!(!WidthState::from_saved("").has_table("core/Pod"));
}

#[test]
fn saved_round_trips() {
    let mut w = WidthState::from_saved("");
    w.set("core/Pod", "Name", 241.6);
    w.set("nodes", "CPU", 90.0);
    let back = WidthState::from_saved(&w.saved());
    assert_eq!(back.get("core/Pod", "Name"), Some(242.0), "saved as whole pixels");
    assert_eq!(back.get("nodes", "CPU"), Some(90.0));
}

#[test]
fn set_enforces_a_minimum_width() {
    let mut w = WidthState::from_saved("");
    w.set("core/Pod", "Name", 5.0);
    assert_eq!(w.get("core/Pod", "Name"), Some(MIN_WIDTH));
    w.set("core/Pod", "Name", 300.0);
    assert_eq!(w.get("core/Pod", "Name"), Some(300.0));
}

#[test]
fn forget_drops_only_that_table() {
    let mut w = WidthState::from_saved("core/Pod\tName\t240\nnodes\tName\t180");
    w.forget("core/Pod");
    assert!(!w.has_table("core/Pod"));
    assert_eq!(w.get("nodes", "Name"), Some(180.0));
}

#[test]
fn table_style_sums_widths_once_customised() {
    let headers: Vec<String> = ["Name", "Status", "Age"].map(String::from).to_vec();
    let mut w = WidthState::from_saved("");
    assert_eq!(w.table_style("core/Pod", &headers), "", "automatic layout");
    w.set("core/Pod", "Name", 200.0);
    w.set("core/Pod", "Age", 60.4);
    // Status has no width yet and gets the fallback.
    assert_eq!(w.table_style("core/Pod", &headers), "table-layout: fixed; width: max(100%, 380px)");
    assert_eq!(w.table_style("nodes", &headers), "");
}

#[test]
fn cell_styles_and_dragging() {
    let mut w = WidthState::default();
    assert_eq!(w.cell_style("t", "Name"), "");
    w.set("t", "Name", 150.4);
    assert_eq!(w.cell_style("t", "Name"), "width: 150px");
    assert_eq!(dragged_width(150.0, 300.0, 340.0), 190.0);
    assert_eq!(dragged_width(150.0, 300.0, 250.0), 100.0);
}

#[test]
fn heading_aria_sort() {
    let sorted = || Some(("Age".to_owned(), true));
    assert_eq!(column_aria(true, sorted(), "Age"), Some("descending"));
    assert_eq!(column_aria(true, Some(("Age".to_owned(), false)), "Age"), Some("ascending"));
    assert_eq!(column_aria(true, sorted(), "Name"), Some("none"));
    assert_eq!(column_aria(true, None, "Name"), Some("none"));
    assert_eq!(column_aria(false, None, "Name"), None);
}
