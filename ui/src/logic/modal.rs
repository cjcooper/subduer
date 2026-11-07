//! Dialog frame logic: Esc handling, stacking and request generations.

/// One Esc press for us to act on: not a held key's repeat, not already
/// handled, not ending an input-method composition (that Esc belongs to the
/// text field; WebKit reports some of those as not composing, with key code 229).
pub(crate) fn is_plain_esc(key: &str, repeat: bool, handled: bool, composing: bool, key_code: u32) -> bool {
    key == "Escape" && !repeat && !handled && !composing && key_code != 229
}

/// The backdrop's `z-index` for a dialog `depth` deep: newer dialogs draw
/// over older ones (the stylesheet's base is 10).
pub(crate) fn backdrop_z(depth: usize) -> String {
    (10 + depth).to_string()
}

/// The card's classes, with a dialog's own.
pub(crate) fn card_class(class: &str) -> String {
    format!("modal {class}")
}

/// Whether a click cancels the dialog: only on the backdrop itself, for a
/// dialog that allows it, and not while its action runs.
pub(crate) fn click_cancels(dismiss_outside: bool, on_backdrop: bool, busy: bool) -> bool {
    dismiss_outside && on_backdrop && !busy
}

/// The generation after `now`: a dialog moves on each time it opens or closes.
pub(crate) fn next_generation(now: u64) -> u64 {
    now + 1
}

/// Whether a request started at generation `then` still belongs to the dialog.
pub(crate) fn still(now: u64, then: u64) -> bool {
    now == then
}

#[cfg(test)]
#[path = "modal_tests.rs"]
mod tests;
