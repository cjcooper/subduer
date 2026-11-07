//! The Context menu's keyboard and list logic.

use subduer_types::ContextInfo;

/// The index after `from` for `key` in a list of `len` (`None` if the key
/// doesn't move the highlight).
pub(crate) fn step(key: &str, from: Option<usize>, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let last = len - 1;
    match key {
        "ArrowDown" => Some(from.map_or(0, |i| (i + 1).min(last))),
        "ArrowUp" => Some(from.map_or(last, |i| i.saturating_sub(1))),
        "Home" | "PageUp" => Some(0),
        "End" | "PageDown" => Some(last),
        _ => None,
    }
}

/// The next context after `from` whose name starts with `letter`, wrapping round.
pub(crate) fn jump(names: &[String], from: Option<usize>, letter: char) -> Option<usize> {
    let letter = letter.to_lowercase().to_string();
    let start = from.map_or(0, |i| i + 1);
    (0..names.len()).map(|n| (start + n) % names.len()).find(|&i| names[i].to_lowercase().starts_with(&letter))
}

/// What a key press does to the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyAction {
    /// Nothing; the key goes on as usual.
    Ignore,
    /// Opens the closed menu (the key's default is prevented).
    Open,
    /// Esc: closes it, and nothing else gets this Esc.
    Escape,
    /// Tab: closes it and moves focus on as usual.
    Tab,
    /// Enter or Space: picks the context at this index, or closes without one.
    Pick(Option<usize>),
    /// Moves the highlight; `prevent` stops the key's default (scrolling).
    Highlight { index: usize, prevent: bool },
}

/// What `key` does with the menu `open` or not, the highlight at `active`,
/// and these context `names`.
pub(crate) fn key_action(key: &str, open: bool, active: Option<usize>, names: &[String]) -> KeyAction {
    if !open {
        return if matches!(key, "ArrowDown" | "ArrowUp" | "Enter" | " ") { KeyAction::Open } else { KeyAction::Ignore };
    }
    match key {
        "Escape" => KeyAction::Escape,
        "Tab" => KeyAction::Tab,
        "Enter" | " " => KeyAction::Pick(active.filter(|&i| i < names.len())),
        _ => {
            if let Some(index) = step(key, active, names.len()) {
                return KeyAction::Highlight { index, prevent: true };
            }
            let letter = key.chars().next().filter(|c| key.chars().count() == 1 && c.is_alphanumeric());
            match letter.and_then(|letter| jump(names, active, letter)) {
                Some(index) => KeyAction::Highlight { index, prevent: false },
                None => KeyAction::Ignore,
            }
        }
    }
}

/// Where the context named `current` is in the list.
pub(crate) fn selected_index(contexts: &[ContextInfo], current: Option<&str>) -> Option<usize> {
    contexts.iter().position(|c| Some(c.name.as_str()) == current)
}

/// The row highlighted when the menu opens: the current context, else the first.
pub(crate) fn opening_index(contexts: &[ContextInfo], current: Option<&str>) -> Option<usize> {
    selected_index(contexts, current).or(Some(0))
}

/// The id of option `i`, for `aria-activedescendant`.
pub(crate) fn option_id(i: usize) -> String {
    format!("context-option-{i}")
}

/// The button's `aria-activedescendant`: the highlighted option while open.
pub(crate) fn active_descendant(open: bool, active: Option<usize>) -> Option<String> {
    open.then_some(active).flatten().map(option_id)
}

/// The list's new `scrollTop` so a row from `top` to `bottom` is in view,
/// or `None` if it already is.
pub(crate) fn scroll_into_view(top: i32, bottom: i32, scroll_top: i32, height: i32) -> Option<i32> {
    if top < scroll_top {
        Some(top)
    } else if bottom > scroll_top + height {
        Some(bottom - height)
    } else {
        None
    }
}

/// The button's text with no context chosen.
pub(crate) fn empty_label(no_contexts: bool) -> &'static str {
    if no_contexts { "No contexts" } else { "Choose a context" }
}

/// Whether picking `ctx` changes anything: the context shown is left alone,
/// as with a native menu.
pub(crate) fn is_switch(current: Option<&str>, ctx: &str) -> bool {
    current != Some(ctx)
}

#[cfg(test)]
#[path = "context_picker_tests.rs"]
mod tests;
