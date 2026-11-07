//! Find in a detail panel: where the text matches, which match is current,
//! and how far to scroll to show it.

use crate::logic::prefs::Desktop;

/// The UTF-16 ranges (as the DOM counts) where `query` occurs in `text`,
/// ignoring ASCII case. Overlapping matches aren't counted twice.
pub(crate) fn matches_in(text: &str, query: &str) -> Vec<(u32, u32)> {
    let query = query.trim();
    if query.is_empty() {
        return vec![];
    }
    let (hay, needle) = (text.to_ascii_lowercase(), query.to_ascii_lowercase());
    let utf16 = |byte: usize| text[..byte].encode_utf16().count() as u32;
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = hay[from..].find(&needle) {
        let start = from + at;
        let end = start + needle.len();
        out.push((utf16(start), utf16(end)));
        from = end;
    }
    out
}

/// "3 of 12", "No matches" or nothing before typing.
pub(crate) fn count_text(current: Option<usize>, total: usize, query: &str) -> String {
    match (query.trim().is_empty(), total, current) {
        (true, _, _) => String::new(),
        (false, 0, _) => "No matches".into(),
        (false, n, Some(i)) => format!("{} of {n}", i + 1),
        (false, n, None) => format!("{n} matches"),
    }
}

/// The next match index going forward or back, wrapping around.
pub(crate) fn step(current: Option<usize>, total: usize, back: bool) -> Option<usize> {
    if total == 0 {
        return None;
    }
    Some(match (current, back) {
        (None, false) => 0,
        (None, true) => total - 1,
        (Some(i), false) => (i + 1) % total,
        (Some(i), true) => (i + total - 1) % total,
    })
}

/// The current match after finding again: kept where it can be, else the
/// last one; none when nothing matches.
pub(crate) fn keep_current(current: Option<usize>, total: usize) -> Option<usize> {
    current.map(|i| i.min(total.saturating_sub(1))).filter(|_| total > 0)
}

/// Whether opening find (or typing) should go to the first match straight away.
pub(crate) fn goes_to_first(open: bool, query: &str) -> bool {
    open && !query.trim().is_empty()
}

/// The shortcut that opens find, as shown in tooltips.
pub(crate) fn shortcut_for(desktop: Desktop) -> &'static str {
    if desktop == Desktop::Mac { "⌘F" } else { "Ctrl+F" }
}

/// The Find button's tooltip.
pub(crate) fn button_title(shortcut: &str) -> String {
    format!("Find in this panel ({shortcut})")
}

/// Whether a key press is ⌘/Ctrl with `key` (and not Alt).
pub(crate) fn is_find_key(meta: bool, ctrl: bool, alt: bool, pressed: &str, key: &str) -> bool {
    (meta || ctrl) && !alt && pressed.eq_ignore_ascii_case(key)
}

/// Whether a computed `overflow` value scrolls.
pub(crate) fn scrolls(overflow: &str) -> bool {
    overflow == "auto" || overflow == "scroll"
}

/// How far to scroll so a match starting at `start` with size `size` sits
/// in the middle of a box starting at `box_start` with size `box_size`.
pub(crate) fn centre_offset(start: f64, size: f64, box_start: f64, box_size: f64) -> f64 {
    start - box_start - (box_size - size) / 2.0
}

/// Whether a match from `start` to `end` sticks out of a box from `box_start` to `box_end`.
pub(crate) fn outside(start: f64, end: f64, box_start: f64, box_end: f64) -> bool {
    start < box_start || end > box_end
}

/// How long to wait before scrolling to a match: a hidden section it opens
/// lays out first.
pub(crate) fn reveal_delay_ms(hidden: bool) -> u64 {
    if hidden { 50 } else { 0 }
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
