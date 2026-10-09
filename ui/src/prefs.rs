//! UI preferences kept in the webview's localStorage. Failures are ignored:
//! preferences are a convenience and fall back to defaults.

use subduer_types::Category;

const SECTION_ORDER_KEY: &str = "sidebar.sectionOrder";

fn storage() -> Option<web_sys::Storage> {
    leptos::prelude::window().local_storage().ok().flatten()
}

/// Saved section order. Unknown names are dropped and sections missing from
/// the saved list (e.g. added in a newer version) are appended.
pub fn load_section_order() -> Vec<Category> {
    let saved = storage()
        .and_then(|s| s.get_item(SECTION_ORDER_KEY).ok().flatten())
        .unwrap_or_default();
    let mut order: Vec<Category> = vec![];
    for name in saved.split(',') {
        if let Some(c) = Category::ALL.into_iter().find(|c| format!("{c:?}") == name)
            && !order.contains(&c)
        {
            order.push(c);
        }
    }
    for c in Category::ALL {
        if !order.contains(&c) {
            order.push(c);
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
