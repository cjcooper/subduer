use super::*;

/// Mon 5 Oct 2026 00:00 UTC.
const MONDAY: f64 = 1_791_158_400.0;

#[test]
fn at_most_three_traces_first_and_last_two() {
    assert_eq!(picked(2), vec![0, 1]);
    assert_eq!(picked(3), vec![0, 1, 2]);
    assert_eq!(picked(5), vec![0, 3, 4]);
}

#[test]
fn weeks_start_on_monday() {
    assert_eq!(week_of(MONDAY, 0.0), week_of(MONDAY + WEEK - 1.0, 0.0));
    assert_eq!(week_of(MONDAY - 1.0, 0.0), week_of(MONDAY, 0.0) - 1.0, "Sunday night is the week before");
    // Monday 01:00 in Paris (UTC+2) is still Sunday in UTC.
    assert_eq!(week_of(MONDAY - 3600.0, 7200.0), week_of(MONDAY, 0.0));
}

#[test]
fn shifts_line_traces_up() {
    let first = MONDAY + 9.0 * 3600.0;
    // Since start: each trace from its own start.
    assert_eq!(shift(Align::SinceStart, first + 3.0 * 86400.0, 0.0, first, 0.0), first + 3.0 * 86400.0);
    // Time of week: a trace started on a Wednesday two weeks later moves back two weeks,
    // so its Wednesday sits over the first trace's Wednesday.
    let later = first + 2.0 * WEEK + 2.0 * 86400.0;
    assert_eq!(shift(Align::TimeOfWeek, later, 0.0, first, 0.0), 2.0 * WEEK);
    assert_eq!(shift(Align::TimeOfWeek, first + 3600.0, 0.0, first, 0.0), 0.0, "same week");
}

#[test]
fn end_labels_move_apart() {
    // Two lines ending on the same spot, a third close to them, and one far away.
    assert_eq!(end_offsets(&[(99.0, 80.0), (99.0, 80.0), (98.0, 84.0), (50.0, 80.0)]), vec![0, 14, 28, 0]);
    assert_eq!(end_offsets(&[(99.0, 20.0), (99.0, 80.0)]), vec![0, 0]);
}

#[test]
fn values_are_read_from_the_bucket_under_the_pointer() {
    let points = vec![(100.0, 1.0), (160.0, 2.0)];
    assert_eq!(value_at(&points, 60.0, 0.0, 130.0), Some((100.0, 1.0)));
    assert_eq!(value_at(&points, 60.0, 100.0, 70.0), Some((160.0, 2.0)), "shifted onto the shared axis");
    assert_eq!(value_at(&points, 60.0, 0.0, 230.0), None);
}
