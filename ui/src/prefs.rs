//! UI preferences kept in the webview's localStorage. Failures are ignored:
//! preferences are a convenience and fall back to defaults.

use subduer_types::Category;

const SECTION_ORDER_KEY: &str = "sidebar.sectionOrder";

pub(crate) fn storage() -> Option<web_sys::Storage> {
    leptos::prelude::window().local_storage().ok().flatten()
}

/// Saved section order. Unknown names are dropped. Sections missing from the
/// saved list (e.g. added in a newer version) go before the first saved
/// section that follows them in the default order.
pub fn load_section_order() -> Vec<Category> {
    let saved = storage()
        .and_then(|s| s.get_item(SECTION_ORDER_KEY).ok().flatten())
        .unwrap_or_default();
    section_order_from(&saved)
}

/// [`load_section_order`] from the saved text.
fn section_order_from(saved: &str) -> Vec<Category> {
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

pub fn save_section_order(order: &[Category]) {
    let value = section_order_text(order);
    if let Some(s) = storage() {
        let _ = s.set_item(SECTION_ORDER_KEY, &value);
    }
}

/// The saved form read by [`section_order_from`].
fn section_order_text(order: &[Category]) -> String {
    order.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>().join(",")
}

const THEME_KEY: &str = "appearance.theme";

/// Light, dark, or following the system ("Auto").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Auto,
    Light,
    Dark,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::Auto => "auto",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
}

/// Whether the webview can tell the system's light or dark appearance.
pub fn auto_supported() -> bool {
    leptos::prelude::window()
        .match_media("(prefers-color-scheme: dark)")
        .ok()
        .flatten()
        .is_some_and(|m| m.media() != "not all")
}

/// The saved theme; Auto where supported, otherwise Light.
pub fn load_theme() -> Theme {
    let saved = storage().and_then(|s| s.get_item(THEME_KEY).ok().flatten());
    theme_from(saved.as_deref(), auto_supported)
}

/// The theme a saved value means; `auto` asks only when it matters.
fn theme_from(saved: Option<&str>, auto: impl Fn() -> bool) -> Theme {
    match saved {
        Some("light") => Theme::Light,
        Some("dark") => Theme::Dark,
        _ if auto() => Theme::Auto,
        _ => Theme::Light,
    }
}

pub fn save_theme(theme: Theme) {
    if let Some(s) = storage() {
        let _ = s.set_item(THEME_KEY, theme.as_str());
    }
}

/// Applies a theme to this window's page: `data-theme` on the root element
/// overrides the system preference in the CSS; Auto removes it.
pub fn apply_theme(theme: Theme) {
    let Some(root) = leptos::prelude::document().document_element() else { return };
    let _ = match theme {
        Theme::Auto => root.remove_attribute("data-theme"),
        other => root.set_attribute("data-theme", other.as_str()),
    };
}

/// Whether this window is showing dark colours right now.
pub fn is_dark() -> bool {
    match load_theme() {
        Theme::Dark => true,
        Theme::Light => false,
        Theme::Auto => leptos::prelude::window()
            .match_media("(prefers-color-scheme: dark)")
            .ok()
            .flatten()
            .is_some_and(|m| m.matches()),
    }
}

/// Applies the saved theme now, and again whenever another window changes it
/// (windows share localStorage, which fires `storage` in the others).
pub fn follow_theme() {
    apply_theme(load_theme());
    let handle = leptos::prelude::window_event_listener_untyped("storage", |_| apply_theme(load_theme()));
    // Lives as long as the window.
    std::mem::forget(handle);
}

/// Uses the system accent colour, asking again whenever the window gets focus
/// (the user may have changed it meanwhile). Without one, or outside the app,
/// the stylesheet's own blue stays.
pub fn follow_accent() {
    fn refresh() {
        // Not Leptos's spawn_local: this runs before the app mounts and sets
        // up its executor.
        wasm_bindgen_futures::spawn_local(async {
            let accent = crate::tauri::invoke::<_, Option<String>>("system_accent", &crate::tauri::NoArgs {}).await;
            apply_accent(accent.ok().flatten().as_deref());
        });
    }
    refresh();
    let handle = leptos::prelude::window_event_listener_untyped("focus", |_| refresh());
    // Lives as long as the window.
    std::mem::forget(handle);
}

/// `data-accent` on the root element switches the CSS to `--os-accent`.
fn apply_accent(accent: Option<&str>) {
    let Some(root) = leptos::prelude::document().document_element() else { return };
    let text = accent.and_then(accent_text);
    let _ = match (accent, text) {
        (Some(accent), Some(text)) => {
            let _ = root.set_attribute("style", &format!("--os-accent: {accent}; --accent-text: {text}"));
            root.set_attribute("data-accent", "")
        }
        _ => {
            let _ = root.remove_attribute("style");
            root.remove_attribute("data-accent")
        }
    };
}

/// The text colour for buttons filled with a `#rrggbb` accent: white, unless
/// the accent is too light for it (under 3:1, e.g. yellow), then dark.
/// `None` if it isn't a colour.
fn accent_text(hex: &str) -> Option<&'static str> {
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
pub enum Desktop {
    Mac,
    Windows,
    Linux,
}

pub fn desktop() -> Desktop {
    let agent = leptos::prelude::window().navigator().user_agent().unwrap_or_default();
    desktop_from(&agent)
}

fn desktop_from(agent: &str) -> Desktop {
    if agent.contains("Windows") {
        Desktop::Windows
    } else if agent.contains("Mac") {
        Desktop::Mac
    } else {
        Desktop::Linux
    }
}

/// The button that shows a downloaded file in the system's file manager.
pub fn reveal_label() -> &'static str {
    reveal_label_for(desktop())
}

fn reveal_label_for(desktop: Desktop) -> &'static str {
    match desktop {
        Desktop::Mac => "Show in Finder",
        Desktop::Windows => "Show in Explorer",
        Desktop::Linux => "Show folder",
    }
}

/// The capture image last used on a context.
fn capture_image_key(context: &str) -> String {
    format!("capture.image.{context}")
}

pub fn load_capture_image(context: &str) -> String {
    storage().and_then(|s| s.get_item(&capture_image_key(context)).ok().flatten()).unwrap_or_default()
}

pub fn save_capture_image(context: &str, image: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(&capture_image_key(context), image.trim());
    }
}

const PULL_EVERY_KEY: &str = "traces.pullEvery";

/// The last pull interval chosen for a trace, in seconds.
pub fn load_pull_every() -> i64 {
    pull_every_from(storage().and_then(|s| s.get_item(PULL_EVERY_KEY).ok().flatten()).as_deref())
}

/// A saved interval, if it's still one of the choices; else the default.
fn pull_every_from(saved: Option<&str>) -> i64 {
    let choices = subduer_types::TRACE_PULL_EVERY_CHOICES;
    saved.and_then(|s| s.parse().ok()).filter(|s| choices.contains(s)).unwrap_or(choices[0])
}

pub fn save_pull_every(secs: i64) {
    if let Some(s) = storage() {
        let _ = s.set_item(PULL_EVERY_KEY, &secs.to_string());
    }
}

const RECENT_TRACES_KEY: &str = "traces.recent";
/// Recent traces kept.
const RECENT_TRACES: usize = 12;

/// A trace file recorded or opened recently, for the Traces page.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RecentTrace {
    pub path: String,
    pub context: String,
    /// What was traced, e.g. "namespace shop".
    pub what: String,
    /// When it was last opened or recorded (ms).
    pub at: f64,
}

pub fn load_recent_traces() -> Vec<RecentTrace> {
    storage()
        .and_then(|s| s.get_item(RECENT_TRACES_KEY).ok().flatten())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_recent_traces(list: &[RecentTrace]) {
    if let (Some(s), Ok(text)) = (storage(), serde_json::to_string(list)) {
        let _ = s.set_item(RECENT_TRACES_KEY, &text);
    }
}

/// Puts `trace` first (replacing an older entry for the same file) and saves.
pub fn remember_trace(trace: RecentTrace) -> Vec<RecentTrace> {
    let list = with_recent(load_recent_traces(), trace);
    save_recent_traces(&list);
    list
}

pub fn forget_trace(path: &str) -> Vec<RecentTrace> {
    let list: Vec<RecentTrace> = load_recent_traces().into_iter().filter(|t| t.path != path).collect();
    save_recent_traces(&list);
    list
}

fn with_recent(mut list: Vec<RecentTrace>, trace: RecentTrace) -> Vec<RecentTrace> {
    list.retain(|t| t.path != trace.path);
    list.insert(0, trace);
    list.truncate(RECENT_TRACES);
    list
}

#[cfg(test)]
#[path = "prefs_tests.rs"]
mod tests_unit;
