//! Messages for the Tauri bridge.

use std::fmt::Display;

/// A command's answer that didn't deserialize.
pub(crate) fn bad_response(cmd: &str, error: impl Display) -> String {
    format!("{cmd}: bad response: {error}")
}

/// A rejected call's message: the backend's error text when it sent one,
/// else whatever was thrown, debug-printed.
pub(crate) fn call_error(text: Option<String>, debug: &dyn Fn() -> String) -> String {
    text.unwrap_or_else(debug)
}

/// The log line for an event payload that didn't deserialize.
pub(crate) fn bad_payload(event: &str, error: impl Display) -> String {
    format!("bad {event} payload: {error}")
}

#[cfg(test)]
#[path = "tauri_tests.rs"]
mod tests;
