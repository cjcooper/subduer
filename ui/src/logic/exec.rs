//! The shell window: its URL label, the terminal's colours, the requests and
//! the texts it shows.

use serde::Serialize;
use subduer_types::{ExecSession, ExecTarget};

/// `open_exec` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct OpenArgs {
    pub(crate) target: ExecTarget,
}

/// `exec_start` and `exec_resize` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct SizeArgs {
    pub(crate) cols: u16,
    pub(crate) rows: u16,
}

/// `exec_input` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct InputArgs {
    pub(crate) data: String,
}

/// The `exec` parameter of a URL's query string (`?a=b&exec=x`).
pub(crate) fn label_in(search: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|kv| kv.strip_prefix("exec="))
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
}

/// The xterm.js theme: matches the app's --panel / --text / --accent tokens.
pub(crate) fn theme(dark: bool) -> Vec<(&'static str, &'static str)> {
    let (bg, fg, cursor, selection) =
        if dark { ("#1d2128", "#e3e7ed", "#5b9bff", "#2f4a7a") } else { ("#ffffff", "#1c2430", "#2f6fe4", "#cfe0fb") };
    let mut theme = vec![("background", bg), ("foreground", fg), ("cursor", cursor), ("selectionBackground", selection)];
    // xterm's default ANSI colours are tuned for dark backgrounds; on white
    // the bright green/yellow/cyan prompts are unreadable, so darken them.
    if !dark {
        theme.extend([
            ("black", "#1c2430"),
            ("red", "#c6362c"),
            ("green", "#1d7a3f"),
            ("yellow", "#8a5300"),
            ("blue", "#2457c5"),
            ("magenta", "#9b3fb5"),
            ("cyan", "#0f7285"),
            ("white", "#6b7686"),
            ("brightBlack", "#4b5563"),
            ("brightRed", "#a8261d"),
            ("brightGreen", "#166534"),
            ("brightYellow", "#734500"),
            ("brightBlue", "#1d4ab0"),
            ("brightMagenta", "#7e2f96"),
            ("brightCyan", "#0b5d6c"),
            ("brightWhite", "#1c2430"),
        ]);
    }
    theme
}

/// A terminal dimension read from JS, or `default` when missing.
pub(crate) fn dimension(v: Option<f64>, default: f64) -> u16 {
    v.unwrap_or(default) as u16
}

/// The dimmed "[message]" line written when the session ends.
pub(crate) fn exited_line(message: &str) -> String {
    format!("\r\n\x1b[2m[{message}]\x1b[0m\r\n")
}

pub(crate) fn heading(session: Option<Result<ExecSession, String>>) -> String {
    match session {
        None => "Connecting…".to_owned(),
        Some(Ok(s)) => s.title,
        Some(Err(_)) => "Shell".to_owned(),
    }
}

#[cfg(test)]
#[path = "exec_tests.rs"]
mod tests;
