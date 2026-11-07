//! What the saved preferences mean: parsing and writing their saved forms,
//! and choices that follow from them.

use subduer_types::Category;

/// Saved section order. Unknown names are dropped. Sections missing from the
/// saved list (e.g. added in a newer version) go before the first saved
/// section that follows them in the default order.
pub(crate) fn section_order_from(saved: &str) -> Vec<Category> {
    let mut order: Vec<Category> = vec![];
    for name in saved.split(',') {
        if let Some(c) = Category::SECTIONS.into_iter().find(|c| format!("{c:?}") == name)
            && !order.contains(&c)
        {
            order.push(c);
        }
    }
    let rank = |c: Category| Category::SECTIONS.iter().position(|x| *x == c);
    for c in Category::SECTIONS {
        if !order.contains(&c) {
            let at = order.iter().position(|x| rank(*x) > rank(c)).unwrap_or(order.len());
            order.insert(at, c);
        }
    }
    order
}

/// The saved form read by [`section_order_from`].
pub(crate) fn section_order_text(order: &[Category]) -> String {
    order.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>().join(",")
}

/// Light, dark, or following the system ("Auto").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Theme {
    Auto,
    Light,
    Dark,
}

impl Theme {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Theme::Auto => "auto",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
}

/// The theme a saved value means; `auto` asks only when it matters.
pub(crate) fn theme_from(saved: Option<&str>, auto: &dyn Fn() -> bool) -> Theme {
    match saved {
        Some("light") => Theme::Light,
        Some("dark") => Theme::Dark,
        _ if auto() => Theme::Auto,
        _ => Theme::Light,
    }
}

/// Whether a theme shows dark colours; Auto asks the system.
pub(crate) fn theme_is_dark(theme: Theme, system_dark: &dyn Fn() -> bool) -> bool {
    match theme {
        Theme::Dark => true,
        Theme::Light => false,
        Theme::Auto => system_dark(),
    }
}

/// Whether a `prefers-color-scheme` media query's text says the webview
/// understood it.
pub(crate) fn media_supported(media: &str) -> bool {
    media != "not all"
}

/// Settings › Transparency from its saved value; on unless turned off.
pub(crate) fn transparency_from(saved: Option<&str>) -> bool {
    saved != Some("off")
}

/// The saved form of Settings › Transparency.
pub(crate) fn transparency_text(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// The root element's `style` for a system accent colour, with the text
/// colour that goes on it; `None` if it isn't a usable colour.
pub(crate) fn accent_style(accent: Option<&str>) -> Option<String> {
    let accent = accent?;
    let text = accent_text(accent)?;
    Some(format!("--os-accent: {accent}; --accent-text: {text}"))
}

/// The text colour for buttons filled with a `#rrggbb` accent: white, unless
/// the accent is too light for it (under 3:1, e.g. yellow), then dark.
/// `None` if it isn't a colour.
pub(crate) fn accent_text(hex: &str) -> Option<&'static str> {
    let hex = hex.strip_prefix('#').filter(|h| h.len() == 6)?;
    let channel = |i: usize| -> Option<f64> {
        let c = f64::from(u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()?) / 255.0;
        Some(if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) })
    };
    let luminance = 0.2126 * channel(0)? + 0.7152 * channel(2)? + 0.0722 * channel(4)?;
    Some(if 1.05 / (luminance + 0.05) >= 3.0 { "#fff" } else { "#1c2430" })
}

/// The desktop the app runs on, from the webview's user agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Desktop {
    Mac,
    Windows,
    Linux,
}

pub(crate) fn desktop_from(agent: &str) -> Desktop {
    if agent.contains("Windows") {
        Desktop::Windows
    } else if agent.contains("Mac") {
        Desktop::Mac
    } else {
        Desktop::Linux
    }
}

/// The button that shows a downloaded file in the system's file manager.
pub(crate) fn reveal_label_for(desktop: Desktop) -> &'static str {
    match desktop {
        Desktop::Mac => "Show in Finder",
        Desktop::Windows => "Show in Explorer",
        Desktop::Linux => "Show folder",
    }
}

/// Where the capture image last used on a context is kept.
pub(crate) fn capture_image_key(context: &str) -> String {
    format!("capture.image.{context}")
}

/// A saved trace pull interval, if it's still one of the choices; else the default.
pub(crate) fn pull_every_from(saved: Option<&str>) -> i64 {
    let choices = subduer_types::TRACE_PULL_EVERY_CHOICES;
    saved.and_then(|s| s.parse().ok()).filter(|s| choices.contains(s)).unwrap_or(choices[0])
}

/// Recent traces kept.
const RECENT_TRACES: usize = 12;

/// A trace file recorded or opened recently, for the Traces page.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct RecentTrace {
    pub(crate) path: String,
    pub(crate) context: String,
    /// What was traced, e.g. "namespace shop".
    pub(crate) what: String,
    /// When it was last opened or recorded (ms).
    pub(crate) at: f64,
}

/// The recent traces from their saved JSON; none if it doesn't parse.
pub(crate) fn recent_from(saved: Option<&str>) -> Vec<RecentTrace> {
    saved.and_then(|text| serde_json::from_str(text).ok()).unwrap_or_default()
}

/// The saved form read by [`recent_from`].
pub(crate) fn recent_text(list: &[RecentTrace]) -> String {
    serde_json::to_string(list).unwrap_or_default()
}

/// Puts `trace` first (replacing an older entry for the same file), keeping
/// the newest [`RECENT_TRACES`].
pub(crate) fn with_recent(mut list: Vec<RecentTrace>, trace: RecentTrace) -> Vec<RecentTrace> {
    list.retain(|t| t.path != trace.path);
    list.insert(0, trace);
    list.truncate(RECENT_TRACES);
    list
}

/// The list without `path`'s entry.
pub(crate) fn without_recent(list: Vec<RecentTrace>, path: &str) -> Vec<RecentTrace> {
    list.into_iter().filter(|t| t.path != path).collect()
}

#[cfg(test)]
#[path = "prefs_tests.rs"]
mod tests;
