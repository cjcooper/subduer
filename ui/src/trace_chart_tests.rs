use super::*;

#[test]
fn idle_is_what_is_requested_but_not_used() {
    assert_eq!(idle(250.0, 1000.0), Some((750.0, 0.75)));
    assert_eq!(idle(1000.0, 1000.0), None);
    assert_eq!(idle(1200.0, 1000.0), None, "over the request isn't idle");
    assert_eq!(idle(5.0, 0.0), None, "nothing requested");
    assert_eq!(share_text(0.75), "75%");
    // 7.2m used of 3 cores: not "100%".
    assert_eq!(share_text(idle(7.2, 3000.0).unwrap().1), "over 99%");
    assert_eq!(share_text(1.0), "100%");
}

#[test]
fn totals_name_their_lines() {
    assert_eq!(Names::of(true).avg, "used");
    assert_eq!(Names::of(false).count, "pods");
    assert_eq!((capitalised(Names::of(true).peak).as_str(), capitalised("").as_str()), ("Busiest minute", ""));
}

#[test]
fn step_series() {
    let changes = vec![(100.0, 500.0), (200.0, 250.0)];
    assert_eq!(step_at(&changes, 50.0), None);
    assert_eq!(step_at(&changes, 100.0), Some(500.0));
    assert_eq!(step_at(&changes, 199.0), Some(500.0));
    assert_eq!(step_at(&changes, 300.0), Some(250.0));
    assert_eq!(step_runs(&changes, 0.0, 300.0), vec![(100.0, 200.0, 500.0), (200.0, 300.0, 250.0)]);
    // Clipped to the window; runs outside it are dropped.
    assert_eq!(step_runs(&changes, 150.0, 180.0), vec![(150.0, 180.0, 500.0)]);
    assert_eq!(step_runs(&changes, 250.0, 300.0), vec![(250.0, 300.0, 250.0)]);
    assert!(step_runs(&vec![], 0.0, 10.0).is_empty());
}

#[test]
fn heat_levels() {
    assert_eq!((heat_level(0.0, 100.0), heat_level(10.0, 100.0), heat_level(50.0, 100.0), heat_level(100.0, 100.0)), (1, 1, 3, 5));
    assert_eq!(heat_level(5.0, 0.0), 1);
    assert_eq!((Measure::Cpu.label(), Measure::Memory.class()), ("CPU", "res-memory"));
    assert_eq!(Measure::Cpu.fmt(1500.0), "1.5 cores");
}

#[test]
fn paths_break_on_missing_buckets() {
    let pts = vec![(0.0, 1.0), (60.0, 2.0), (240.0, 3.0)];
    let (line, area) = paths(&pts, 60.0, |t| t, |v| 100.0 - v, 92.0);
    assert_eq!(line.matches('M').count(), 2);
    assert_eq!(area.matches('Z').count(), 2);
    assert!(line.starts_with("M30.0,99.00L90.0,98.00M270.0,97.00"), "{line}");
}

#[test]
fn reference_labels_never_overlap() {
    assert_eq!(ref_labels(Some(6.0), Some(6.0), 8.0), vec![("request = limit {}", 6.0, 0)]);
    assert_eq!(ref_labels(Some(5.8), Some(6.0), 8.0), vec![("limit {}", 6.0, 0), ("request {}", 5.8, 18)]);
    assert_eq!(ref_labels(Some(6.0), Some(5.8), 8.0), vec![("request {}", 6.0, 0), ("limit {}", 5.8, 18)]);
    assert_eq!(ref_labels(Some(2.0), Some(6.0), 8.0), vec![("request {}", 2.0, 0), ("limit {}", 6.0, 0)]);
    assert_eq!(ref_labels(None, Some(6.0), 8.0), vec![("limit {}", 6.0, 0)]);
    assert!(ref_labels(None, None, 8.0).is_empty());
}
