use super::*;

#[test]
fn label_in_finds_the_logs_parameter() {
    assert_eq!(label_in("?logs=logs-1").as_deref(), Some("logs-1"));
    assert_eq!(label_in("?a=b&logs=logs-1").as_deref(), Some("logs-1"));
    assert_eq!(label_in("?logs="), None);
    assert_eq!(label_in("?exec=x"), None);
    assert_eq!(label_in(""), None);
}

#[test]
fn millis_cuts_nanoseconds_to_milliseconds() {
    assert_eq!(millis("2026-01-02T03:04:05.123456789Z"), "2026-01-02T03:04:05.123Z");
    assert_eq!(millis("2026-01-02T03:04:05.123Z"), "2026-01-02T03:04:05.123Z");
    assert_eq!(millis("2026-01-02T03:04:05.1Z"), "2026-01-02T03:04:05.1Z");
    assert_eq!(millis("2026-01-02T03:04:05Z"), "2026-01-02T03:04:05Z");
    assert_eq!(millis("not a time"), "not a time");
}
