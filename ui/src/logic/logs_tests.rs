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

fn line(source: Option<&str>, text: &str) -> LogLine {
    LogLine { source: source.map(str::to_owned), time: None, text: text.into() }
}

fn session(filter: Option<&str>) -> LogSession {
    LogSession {
        context: "acme".into(),
        target: LogTarget {
            namespace: "shop".into(),
            source: subduer_types::LogSource::Container { pod: "web-1".into(), container: "app".into() },
            filter: filter.map(str::to_owned),
        },
        title: "app · shop/web-1 · Logs".into(),
        event: "logs-1".into(),
    }
}

#[test]
fn lines_are_numbered_and_capped() {
    let mut lines = VecDeque::new();
    let next = push_lines(&mut lines, 7, vec![line(None, "a"), line(None, "b")]);
    assert_eq!(next, 9);
    assert_eq!(lines.iter().map(|r| r.id).collect::<Vec<_>>(), [7, 8]);
    let many = (0..MAX_LINES).map(|i| line(None, &i.to_string())).collect();
    let next = push_lines(&mut lines, next, many);
    assert_eq!(next, 9 + MAX_LINES as u64);
    assert_eq!(lines.len(), MAX_LINES);
    assert_eq!(lines.front().map(|r| r.id), Some(9), "the oldest go first");
}

#[test]
fn notices_keep_the_latest() {
    let mut notices = vec![];
    push_notices(&mut notices, vec!["a".into()]);
    assert_eq!(notices, ["a"]);
    push_notices(&mut notices, (0..MAX_NOTICES).map(|i| i.to_string()).collect());
    assert_eq!(notices.len(), MAX_NOTICES);
    assert_eq!(notices.first().map(String::as_str), Some("0"));
}

#[test]
fn filter_matches_text_or_source_in_any_case() {
    let mut lines = VecDeque::new();
    push_lines(&mut lines, 0, vec![line(None, "GET /health"), line(Some("web-1/app"), "ok"), line(None, "error")]);
    assert_eq!(visible(&lines, "").len(), 3);
    assert_eq!(visible(&lines, "get").iter().map(|r| r.id).collect::<Vec<_>>(), [0]);
    assert_eq!(visible(&lines, "WEB-1").iter().map(|r| r.id).collect::<Vec<_>>(), [1]);
    assert!(visible(&lines, "nothing").is_empty());
}

#[test]
fn the_targets_filter_applies_on_open_only() {
    assert_eq!(start_filter(&Ok(session(Some("db"))), false).as_deref(), Some("db"));
    assert_eq!(start_filter(&Ok(session(Some("db"))), true), None);
    assert_eq!(start_filter(&Ok(session(None)), false), None);
    assert_eq!(start_filter(&Err("boom".into()), false), None);
}

#[test]
fn window_texts() {
    assert!(at_bottom(1000, 570, 400));
    assert!(!at_bottom(1000, 560, 400), "40 px up is scrolled away");
    assert!(!at_bottom(1000, 500, 400));
    assert_eq!(heading(None), "Connecting…");
    assert_eq!(heading(Some(Ok(session(None)))), "app · shop/web-1 · Logs");
    assert_eq!(heading(Some(Err("gone".into()))), "gone");
    assert_eq!(count(3, 3), "3 lines");
    assert_eq!(count(1, 3), "1 of 3 lines");
    assert_eq!(clock(9, 5, 3, 7), "09:05:03.007");
}

#[test]
fn args_serialize() {
    assert_eq!(serde_json::to_value(StartArgs { previous: true }).unwrap(), serde_json::json!({ "previous": true }));
    let open = serde_json::to_value(OpenArgs { target: session(None).target }).unwrap();
    assert_eq!(open["target"]["namespace"], "shop");
}
