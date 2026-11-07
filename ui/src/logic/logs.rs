//! The logs window: its URL label, the line buffer, filtering, following and
//! the texts in its header and footer.

use std::collections::VecDeque;

use serde::Serialize;
use subduer_types::{LogLine, LogSession, LogTarget};

/// Lines kept per window; older ones are dropped.
pub(crate) const MAX_LINES: usize = 10_000;
pub(crate) const MAX_NOTICES: usize = 20;
/// Within this many pixels of the bottom counts as at the bottom.
const BOTTOM_SLACK: i32 = 40;

/// `open_logs` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct OpenArgs {
    pub(crate) target: LogTarget,
}

/// `logs_start` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct StartArgs {
    pub(crate) previous: bool,
}

/// The `logs` parameter of a URL's query string (`?a=b&logs=x`).
pub(crate) fn label_in(search: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|kv| kv.strip_prefix("logs="))
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Row {
    pub(crate) id: u64,
    pub(crate) line: LogLine,
}

/// Appends `new` lines numbered from `next_id`, dropping the oldest past
/// [`MAX_LINES`]; returns the next id.
pub(crate) fn push_lines(lines: &mut VecDeque<Row>, next_id: u64, new: Vec<LogLine>) -> u64 {
    let mut id = next_id;
    for line in new {
        lines.push_back(Row { id, line });
        id += 1;
    }
    while lines.len() > MAX_LINES {
        lines.pop_front();
    }
    id
}

/// Appends notices, keeping the last [`MAX_NOTICES`].
pub(crate) fn push_notices(notices: &mut Vec<String>, new: Vec<String>) {
    notices.extend(new);
    let excess = notices.len().saturating_sub(MAX_NOTICES);
    notices.drain(..excess);
}

/// The lines matching the filter (any case, in the text or the source).
pub(crate) fn visible<'a>(lines: impl IntoIterator<Item = &'a Row>, filter: &str) -> Vec<Row> {
    let needle = filter.to_lowercase();
    lines
        .into_iter()
        .filter(|r| {
            needle.is_empty()
                || r.line.text.to_lowercase().contains(&needle)
                || r.line.source.as_ref().is_some_and(|s| s.to_lowercase().contains(&needle))
        })
        .cloned()
        .collect()
}

/// The filter a new session starts with: the target's, but only when the
/// window opens, not on every restart.
pub(crate) fn start_filter(result: &Result<LogSession, String>, had_session: bool) -> Option<String> {
    match result {
        Ok(s) if !had_session => s.target.filter.clone(),
        _ => None,
    }
}

/// Whether the scroller is at (or near) the bottom.
pub(crate) fn at_bottom(scroll_height: i32, scroll_top: i32, client_height: i32) -> bool {
    scroll_height - scroll_top - client_height < BOTTOM_SLACK
}

pub(crate) fn heading(session: Option<Result<LogSession, String>>) -> String {
    match session {
        None => "Connecting…".to_owned(),
        Some(Ok(s)) => s.title,
        Some(Err(e)) => e,
    }
}

/// "120 lines", or "12 of 120 lines" while filtering.
pub(crate) fn count(shown: usize, total: usize) -> String {
    if shown == total { format!("{total} lines") } else { format!("{shown} of {total} lines") }
}

/// An RFC 3339 UTC time cut to milliseconds, which JS dates can parse.
pub(crate) fn millis(ts: &str) -> String {
    match ts.find('.') {
        Some(dot) => format!("{}Z", &ts[..(dot + 4).min(ts.len() - 1)]),
        None => ts.to_owned(),
    }
}

/// `HH:MM:SS.mmm`.
pub(crate) fn clock(hours: u32, minutes: u32, seconds: u32, millis: u32) -> String {
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

#[cfg(test)]
#[path = "logs_tests.rs"]
mod tests;
