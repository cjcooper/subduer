use super::*;

#[test]
fn nice_ceil_decimal_steps() {
    let d = Scale::Decimal;
    assert_eq!(nice_ceil(1.0, d), 1.0);
    assert_eq!(nice_ceil(1.1, d), 1.2);
    assert_eq!(nice_ceil(1.3, d), 1.5);
    assert_eq!(nice_ceil(1.7, d), 2.0);
    assert_eq!(nice_ceil(2.2, d), 2.5);
    assert_eq!(nice_ceil(2.7, d), 3.0);
    assert_eq!(nice_ceil(3.5, d), 4.0);
    assert_eq!(nice_ceil(4.5, d), 5.0);
    assert_eq!(nice_ceil(5.5, d), 6.0);
    assert_eq!(nice_ceil(7.0, d), 8.0);
    assert_eq!(nice_ceil(9.0, d), 10.0);
    assert_eq!(nice_ceil(105.0, d), 120.0);
    assert_eq!(nice_ceil(0.33, d), 0.4);
    assert_eq!(nice_ceil(2500.0, d), 2500.0);
}

#[test]
fn nice_ceil_falls_back_for_non_positive_or_non_finite() {
    for v in [0.0, -5.0, f64::NAN, f64::INFINITY] {
        assert_eq!(nice_ceil(v, Scale::Decimal), 1.0, "{v}");
        assert_eq!(nice_ceil(v, Scale::Binary), 1.0, "{v}");
    }
}

#[test]
fn nice_ceil_binary_rounds_in_binary_units() {
    let b = Scale::Binary;
    let mib = 1024.0 * 1024.0;
    assert_eq!(nice_ceil(500.0, b), 500.0, "under 1 KiB it's decimal");
    assert_eq!(nice_ceil(1000.0, b), 1000.0);
    assert_eq!(nice_ceil(1024.0, b), 1024.0);
    assert_eq!(nice_ceil(450.0 * mib, b), 500.0 * mib);
    assert_eq!(nice_ceil(1.1 * 1024.0 * mib, b), 1.2 * 1024.0 * mib);
}

#[test]
fn nice_ceil_is_never_below_the_value() {
    for scale in [Scale::Decimal, Scale::Binary] {
        let mut v = 0.003;
        while v < 1e13 {
            let top = nice_ceil(v, scale);
            assert!(top >= v * 0.9999, "{v} -> {top}");
            assert!(top <= v * 1.34, "{v} -> {top} is too loose");
            v *= 1.37;
        }
    }
}

#[test]
fn tidy_drops_trailing_zero_decimals() {
    assert_eq!(tidy("500.0 MiB".into()), "500 MiB");
    assert_eq!(tidy("250.0m".into()), "250m");
    assert_eq!(tidy("1.5 cores".into()), "1.5 cores");
    assert_eq!(tidy("10.05 GiB".into()), "10.05 GiB");
}

#[test]
fn paths_draws_one_segment_without_gaps() {
    let pts = [(0.0, 1.0), (10.0, 2.0), (20.0, 3.0)];
    let (line, area) = paths(&pts, 10.0, |t| t * 2.0, |v| v * 10.0);
    assert_eq!(line, "M0.0,10.00L20.0,20.00L40.0,30.00");
    assert_eq!(area, "M0.0,100L0.0,10.00L20.0,20.00L40.0,30.00L40.0,100Z");
}

#[test]
fn paths_breaks_at_gaps_over_two_and_a_half_steps() {
    // 10 → 35 is exactly 2.5 steps: still joined. 35 → 61 is a gap.
    let pts = [(10.0, 1.0), (35.0, 1.0), (61.0, 2.0), (70.0, 2.0)];
    let (line, area) = paths(&pts, 10.0, |t| t, |v| v);
    assert_eq!(line, "M10.0,1.00L35.0,1.00M61.0,2.00L70.0,2.00");
    assert_eq!(area, "M10.0,100L10.0,1.00L35.0,1.00L35.0,100ZM61.0,100L61.0,2.00L70.0,2.00L70.0,100Z");
}

#[test]
fn paths_of_nothing_is_empty() {
    assert_eq!(paths(&[], 10.0, |t| t, |v| v), (String::new(), String::new()));
    let (line, area) = paths(&[(5.0, 7.0)], 10.0, |t| t, |v| v);
    assert_eq!(line, "M5.0,7.00");
    assert_eq!(area, "M5.0,100L5.0,7.00L5.0,100Z");
}

#[test]
fn close_reference_labels_split_around_their_lines() {
    // Requested 1.75 and allocatable 1.93 cores on a 2.5-core chart: the lower label goes under.
    assert!(label_below(&[1750.0, 1930.0], 0, 2500.0));
    assert!(!label_below(&[1750.0, 1930.0], 1, 2500.0));
    // Far apart: both above.
    assert!(!label_below(&[1.2, 7.0], 0, 8.0) && !label_below(&[1.2, 7.0], 1, 8.0));
    // Request equal to limit: the first stays above, the second goes under.
    assert!(!label_below(&[512.0, 512.0], 0, 600.0));
    assert!(label_below(&[512.0, 512.0], 1, 600.0));
    assert!(!label_below(&[100.0], 0, 0.0));
}

fn cores(v: f64) -> String {
    format!("{v:.1} cores")
}

fn sample() -> ChartData {
    ChartData {
        start: 0.0,
        end: 100.0,
        step: 50.0,
        points: vec![(0.0, 1.0), (50.0, 2.0), (100.0, 4.0)],
        refs: vec![
            RefLine { name: "request", value: 3.9, dashed: true },
            RefLine { name: "limit", value: 4.0, dashed: false },
        ],
    }
}

#[test]
fn empty_charts_say_why() {
    assert_eq!(empty_text(None), Some("Loading…"));
    let mut d = sample();
    assert_eq!(empty_text(Some(&d)), None);
    d.points.clear();
    assert_eq!(empty_text(Some(&d)), Some("No usage data yet"));
    assert_eq!(view_box(), "0 0 1000 100");
    assert_eq!(hh_mm(9, 5), "09:05");
}

#[test]
fn frames_map_and_clamp() {
    let f = Frame::new(&sample(), Scale::Decimal);
    assert_eq!(f.top, 5.0, "4 cores with headroom rounds up to 5");
    assert_eq!(f.x(-10.0), 0.0);
    assert_eq!(f.x(50.0), 500.0);
    assert_eq!(f.x(500.0), W);
    assert_eq!(f.y(0.0), H);
    assert_eq!(f.y(10.0), 18.0, "over the top: clamped under the headroom");
    assert_eq!(f.xp(50.0), 50.0);
    // A window of no length still divides.
    let point = ChartData { start: 5.0, end: 5.0, step: 1.0, points: vec![(5.0, 0.0)], refs: vec![] };
    let f = Frame::new(&point, Scale::Decimal);
    assert_eq!((f.x(5.0), f.top), (0.0, 1.0));
}

#[test]
fn layout_draws_lines_labels_and_summary() {
    let l = layout(&sample(), "CPU", cores, Scale::Decimal);
    assert_eq!(l.line, "M0.0,83.60L500.0,67.20L1000.0,34.40");
    assert_eq!(l.area, "M0.0,100L0.0,83.60L500.0,67.20L1000.0,34.40L1000.0,100Z");
    let grid: Vec<String> = l.grid.iter().map(|y| format!("{y:.1}")).collect();
    assert_eq!(grid, ["100.0", "59.0", "18.0"]);
    assert_eq!(
        l.ticks,
        [("top: calc(59.00% - 15px)".to_owned(), "2.5 cores".to_owned()), ("top: calc(18.00% - 15px)".to_owned(), "5 cores".to_owned())]
    );
    let refs: Vec<(String, bool)> = l.refs.iter().map(|(y, d)| (format!("{y:.2}"), *d)).collect();
    assert_eq!(refs, [("36.04".to_owned(), true), ("34.40".to_owned(), false)]);
    assert_eq!(
        l.ref_labels,
        [
            ("top: calc(36.04% + 1px)".to_owned(), "request 3.9 cores".to_owned()),
            ("top: calc(34.40% - 15px)".to_owned(), "limit 4 cores".to_owned()),
        ],
        "the lower of two close lines is labelled under it"
    );
    assert_eq!(l.end_dot, "left: 100.00%; top: 34.40%");
    assert_eq!(l.summary, "CPU over the last hour: now 4.0 cores, min 1.0 cores, average 2.3 cores, max 4.0 cores");
}

#[test]
fn hover_finds_the_nearest_point_and_keeps_the_tip_inside() {
    let d = sample();
    assert_eq!(pointer_frac(150.0, 100.0, 200.0), Some(0.25));
    assert_eq!(pointer_frac(50.0, 100.0, 200.0), Some(0.0));
    assert_eq!(pointer_frac(500.0, 100.0, 200.0), Some(1.0));
    assert_eq!(pointer_frac(150.0, 100.0, 0.0), None);
    assert_eq!(nearest(&d, 0.4), Some((50.0, 2.0)));
    assert_eq!(nearest(&d, 0.9), Some((100.0, 4.0)));
    assert_eq!(nearest(&ChartData { points: vec![], ..d.clone() }, 0.5), None);
    let f = Frame::new(&d, Scale::Decimal);
    assert_eq!(
        hover(f, 50.0, 2.0),
        Hover {
            cross: "left: 50.00%".into(),
            dot: "left: 50.00%; top: 67.20%".into(),
            tip_class: "lchart-tip left".into(),
            tip_style: "left: 50.00%".into(),
        }
    );
    let right = hover(f, 100.0, 4.0);
    assert_eq!((right.tip_class.as_str(), right.tip_style.as_str()), ("lchart-tip right", "right: 0.00%"));
}
