//! UI preferences kept in the webview's localStorage. Failures are ignored:
//! preferences are a convenience and fall back to defaults. What the saved
//! values mean is in [`crate::logic::prefs`].

use subduer_types::Category;

use crate::logic::prefs as logic;
pub(crate) use crate::logic::prefs::{Desktop, RecentTrace, Theme};

const SECTION_ORDER_KEY: &str = "sidebar.sectionOrder";

pub(crate) fn storage() -> Option<web_sys::Storage> {
    leptos::prelude::window().local_storage().ok().flatten()
}

/// The value saved under `key`, if any.
fn load(key: &str) -> Option<String> {
    storage().and_then(|s| s.get_item(key).ok().flatten())
}

fn save(key: &str, value: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(key, value);
    }
}

/// Saved section order (see [`logic::section_order_from`]).
pub fn load_section_order() -> Vec<Category> {
    logic::section_order_from(&load(SECTION_ORDER_KEY).unwrap_or_default())
}

pub fn save_section_order(order: &[Category]) {
    save(SECTION_ORDER_KEY, &logic::section_order_text(order));
}

const THEME_KEY: &str = "appearance.theme";

/// The system's dark-appearance media query, where the webview has one.
fn dark_query() -> Option<web_sys::MediaQueryList> {
    leptos::prelude::window().match_media("(prefers-color-scheme: dark)").ok().flatten()
}

/// Whether the webview can tell the system's light or dark appearance.
pub fn auto_supported() -> bool {
    dark_query().is_some_and(|m| logic::media_supported(&m.media()))
}

/// The saved theme; Auto where supported, otherwise Light.
pub fn load_theme() -> Theme {
    logic::theme_from(load(THEME_KEY).as_deref(), &auto_supported)
}

pub fn save_theme(theme: Theme) {
    save(THEME_KEY, theme.as_str());
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
    logic::theme_is_dark(load_theme(), &|| dark_query().is_some_and(|m| m.matches()))
}

/// Applies the saved theme now, and again whenever another window changes it
/// (windows share localStorage, which fires `storage` in the others).
pub fn follow_theme() {
    apply_theme(load_theme());
    let handle = leptos::prelude::window_event_listener_untyped("storage", |_| apply_theme(load_theme()));
    // Lives as long as the window.
    std::mem::forget(handle);
}

const TRANSPARENCY_KEY: &str = "appearance.transparency";

/// Settings › Transparency; on unless turned off.
pub fn load_transparency() -> bool {
    logic::transparency_from(load(TRANSPARENCY_KEY).as_deref())
}

pub fn save_transparency(on: bool) {
    save(TRANSPARENCY_KEY, logic::transparency_text(on));
}

/// `data-vibrancy` on the root element draws the sidebar and top bar clear,
/// over the window's system material.
pub fn apply_vibrancy(clear: bool) {
    let Some(root) = leptos::prelude::document().document_element() else { return };
    let _ = if clear { root.set_attribute("data-vibrancy", "") } else { root.remove_attribute("data-vibrancy") };
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
    let _ = match logic::accent_style(accent) {
        Some(style) => {
            let _ = root.set_attribute("style", &style);
            root.set_attribute("data-accent", "")
        }
        None => {
            let _ = root.remove_attribute("style");
            root.remove_attribute("data-accent")
        }
    };
}

pub fn desktop() -> Desktop {
    let agent = leptos::prelude::window().navigator().user_agent().unwrap_or_default();
    logic::desktop_from(&agent)
}

/// The button that shows a downloaded file in the system's file manager.
pub fn reveal_label() -> &'static str {
    logic::reveal_label_for(desktop())
}

/// The capture image last used on a context.
pub fn load_capture_image(context: &str) -> String {
    load(&logic::capture_image_key(context)).unwrap_or_default()
}

pub fn save_capture_image(context: &str, image: &str) {
    save(&logic::capture_image_key(context), image.trim());
}

const PULL_EVERY_KEY: &str = "traces.pullEvery";

/// The last pull interval chosen for a trace, in seconds.
pub fn load_pull_every() -> i64 {
    logic::pull_every_from(load(PULL_EVERY_KEY).as_deref())
}

pub fn save_pull_every(secs: i64) {
    save(PULL_EVERY_KEY, &secs.to_string());
}

const RECENT_TRACES_KEY: &str = "traces.recent";

pub fn load_recent_traces() -> Vec<RecentTrace> {
    logic::recent_from(load(RECENT_TRACES_KEY).as_deref())
}

fn save_recent_traces(list: &[RecentTrace]) {
    save(RECENT_TRACES_KEY, &logic::recent_text(list));
}

/// Puts `trace` first (replacing an older entry for the same file) and saves.
pub fn remember_trace(trace: RecentTrace) -> Vec<RecentTrace> {
    let list = logic::with_recent(load_recent_traces(), trace);
    save_recent_traces(&list);
    list
}

pub fn forget_trace(path: &str) -> Vec<RecentTrace> {
    let list = logic::without_recent(load_recent_traces(), path);
    save_recent_traces(&list);
    list
}
