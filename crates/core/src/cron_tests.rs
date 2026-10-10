use super::*;

fn next(schedule: &str, after: &str) -> Option<String> {
    next_run(schedule, None, after.parse().unwrap()).map(|t| t.to_string())
}

#[test]
fn shorthands() {
    let at = "2026-10-10T12:00:00Z";
    assert_eq!(next("@yearly", at).unwrap(), "2027-01-01T00:00:00Z");
    assert_eq!(next("@annually", at).unwrap(), "2027-01-01T00:00:00Z");
    assert_eq!(next("@monthly", at).unwrap(), "2026-11-01T00:00:00Z");
    // 2026-10-10 is a Saturday.
    assert_eq!(next("@weekly", at).unwrap(), "2026-10-11T00:00:00Z");
    assert_eq!(next("@midnight", at).unwrap(), "2026-10-11T00:00:00Z");
    assert_eq!(next("  @hourly  ", at).unwrap(), "2026-10-10T13:00:00Z");
}

#[test]
fn names_question_marks_and_lists() {
    let at = "2026-10-10T12:00:00Z";
    assert_eq!(next("0 0 1 JAN,jul ?", at).unwrap(), "2027-01-01T00:00:00Z");
    assert_eq!(next("0 0 ? * sun", at).unwrap(), "2026-10-11T00:00:00Z");
    assert_eq!(next("0 12-14/2 * * *", at).unwrap(), "2026-10-10T14:00:00Z");
}

#[test]
fn rejects_bad_fields() {
    let at = "2026-10-10T12:00:00Z";
    for bad in [
        "*/0 * * * *", "*/x * * * *", "5-1 * * * *", "0 24 * * *", "0 0 0 * *", "0 0 * 13 *",
        "0 0 * * 8", "0 0 * foo *", "a * * * *", "0 0 * * mon-x", "* * * * * *",
    ] {
        assert_eq!(next(bad, at), None, "{bad}");
    }
}

#[test]
fn bits() {
    assert!(bit(0b10, 1));
    assert!(!bit(0b10, 0));
    assert_eq!(field("*", 0, 3, &[], 0), Some(0b1111));
    assert_eq!(field("1,3", 0, 3, &[], 0), Some(0b1010));
    assert_eq!(field("feb", 1, 12, &MONTHS, 1), Some(1 << 2));
}
