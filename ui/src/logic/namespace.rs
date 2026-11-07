//! Namespace actions: when New namespace and Filter to it show, name
//! validation and the dialog's texts.

use serde::Serialize;
use subduer_types::{KindInfo, KindRef, ResourceRow};

/// `create_namespace` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct CreateArgs {
    pub(crate) context: String,
    pub(crate) name: String,
}

/// Whether the Namespace list is showing.
pub(crate) fn is_namespaces(kind: Option<&KindRef>) -> bool {
    kind.is_some_and(|k| k.key() == "core/Namespace")
}

/// Whether the API lets this user create namespaces.
pub(crate) fn may_create(kinds: &[KindInfo]) -> bool {
    kinds.iter().any(|k| k.kind.key() == "core/Namespace" && k.verbs.iter().any(|v| v == "create"))
}

/// The New namespace button's tooltip.
pub(crate) fn new_title(locked: bool) -> &'static str {
    if locked { "Create a namespace… (asks to allow changes first)" } else { "Create a namespace…" }
}

/// DNS-1123 label: lowercase letters, digits and `-`, starting and ending
/// with a letter or digit, at most 63 characters.
pub(crate) fn valid_name(name: &str) -> bool {
    let ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    !name.is_empty() && name.len() <= 63 && name.starts_with(ok) && name.ends_with(ok) && name.chars().all(|c| ok(c) || c == '-')
}

/// Whether the list already has a namespace called `name` (trimmed).
pub(crate) fn exists(rows: &[ResourceRow], name: &str) -> bool {
    rows.iter().any(|r| r.name == name.trim())
}

/// Whether Create is enabled.
pub(crate) fn can_create(name: &str, exists: bool, busy: bool) -> bool {
    valid_name(name.trim()) && !exists && !busy
}

/// What's wrong with the typed name, if anything.
pub(crate) fn hint(name: &str, exists: bool) -> Option<&'static str> {
    let n = name.trim();
    if n.is_empty() {
        None
    } else if !valid_name(n) {
        Some("Use lowercase letters, digits and '-', starting and ending with a letter or digit (63 at most).")
    } else if exists {
        Some("A namespace with that name already exists.")
    } else {
        None
    }
}

/// Filter to it's tooltip and label, `current` when the menu already shows it.
pub(crate) fn filter_button(name: &str, current: bool) -> (String, &'static str) {
    (format!("Show only objects in {name} in every list"), if current { "Filtering to it" } else { "Filter to it" })
}

#[cfg(test)]
#[path = "namespace_tests.rs"]
mod tests;
