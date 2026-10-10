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
    let value = order.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>().join(",");
    if let Some(s) = storage() {
        let _ = s.set_item(SECTION_ORDER_KEY, &value);
    }
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
    match saved.as_deref() {
        Some("light") => Theme::Light,
        Some("dark") => Theme::Dark,
        Some("auto") if auto_supported() => Theme::Auto,
        _ if auto_supported() => Theme::Auto,
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
