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
