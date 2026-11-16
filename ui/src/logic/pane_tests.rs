use super::*;

#[test]
fn from_saved_reads_tab_separated_lines() {
    let w = PaneWidths::from_saved("detail\t520\nreview\t610.5");
    assert_eq!(w.get("detail"), Some(520.0));
    assert_eq!(w.get("review"), Some(610.5));
    assert_eq!(w.get("other"), None);
}

#[test]
fn from_saved_skips_malformed_lines() {
    let w = PaneWidths::from_saved("\nbad\ndetail\twide\n\t400\nreview\tNaN\nreview\t500\textra");
    assert_eq!(w.get("detail"), None);
    assert_eq!(w.get(""), None);
    assert_eq!(w.get("review"), Some(500.0), "extra fields are ignored");
}

#[test]
fn saved_round_trips_as_whole_pixels() {
    let mut w = PaneWidths::default();
    w.put("review", Some(600.4));
    w.put("detail", Some(512.6));
    assert_eq!(w.saved(), "detail\t513\nreview\t600");
    assert_eq!(PaneWidths::from_saved(&w.saved()).get("detail"), Some(513.0));
}

#[test]
fn put_none_forgets_only_that_pane() {
    let mut w = PaneWidths::from_saved("detail\t520\nreview\t600");
    w.put("detail", None);
    assert_eq!(w.get("detail"), None);
    assert_eq!(w.get("review"), Some(600.0));
}

#[test]
fn dragging_left_widens() {
    assert_eq!(dragged_width(500.0, 800.0, 700.0), 600.0);
    assert_eq!(dragged_width(500.0, 800.0, 900.0), 400.0);
}

#[test]
fn clamp_keeps_both_sides_usable() {
    assert_eq!(clamp_width(100.0, 1200.0), MIN_PANE);
    assert_eq!(clamp_width(500.0, 1200.0), 500.0);
    assert_eq!(clamp_width(1100.0, 1200.0), 1200.0 - MIN_TABLE);
    // Too little room for both: the pane keeps its minimum.
    assert_eq!(clamp_width(500.0, 400.0), MIN_PANE);
}

#[test]
fn arrow_keys_step_the_width() {
    assert_eq!(key_change("ArrowLeft"), Some(KEY_STEP));
    assert_eq!(key_change("ArrowRight"), Some(-KEY_STEP));
    assert_eq!(key_change("ArrowUp"), None);
}

#[test]
fn style_only_once_set() {
    assert_eq!(pane_style(None), "");
    assert_eq!(pane_style(Some(480.4)), "width: 480px; max-width: none; flex-shrink: 1");
}
