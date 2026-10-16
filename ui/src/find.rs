//! Find in a detail panel: ⌘F / Ctrl+F (or the panel's Find button) opens a
//! bar that highlights every match of the text typed, case-insensitively,
//! and scrolls to one at a time (Enter / Shift+Enter, ⌘G / ⇧⌘G).
//!
//! Matches are found in the panel's text nodes, collapsed sections included:
//! while a search is on, sections render their bodies hidden, and going to a
//! match inside one opens it. Never searched: anything inside
//! `[data-find-skip]` (revealed secret values, Secret data being edited, a
//! Secret's YAML), charts, buttons and form fields.
//!
//! Highlights use the CSS Custom Highlight API, so the page's nodes are never
//! changed (Leptos owns them); where it's missing, the current match is
//! selected instead. A MutationObserver finds matches again when the panel
//! refreshes.

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;

use crate::app::Store;

/// Elements whose text is never searched.
const SKIP: &str = "[data-find-skip], svg, script, style, button.btn, button.close, textarea, input, select, .find-bar, .secret-mask";
const ALL: &str = "subduer-find";
const CURRENT: &str = "subduer-find-current";

/// The UTF-16 ranges (as the DOM counts) where `query` occurs in `text`,
/// ignoring ASCII case. Overlapping matches aren't counted twice.
pub(crate) fn matches_in(text: &str, query: &str) -> Vec<(u32, u32)> {
    let query = query.trim();
    if query.is_empty() {
        return vec![];
    }
    let (hay, needle) = (text.to_ascii_lowercase(), query.to_ascii_lowercase());
    let utf16 = |byte: usize| text[..byte].encode_utf16().count() as u32;
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = hay[from..].find(&needle) {
        let start = from + at;
        let end = start + needle.len();
        out.push((utf16(start), utf16(end)));
        from = end;
    }
    out
}

/// "3 of 12", "No matches" or nothing before typing.
pub(crate) fn count_text(current: Option<usize>, total: usize, query: &str) -> String {
    match (query.trim().is_empty(), total, current) {
        (true, _, _) => String::new(),
        (false, 0, _) => "No matches".into(),
        (false, n, Some(i)) => format!("{} of {n}", i + 1),
        (false, n, None) => format!("{n} matches"),
    }
}

/// The next match index going forward or back, wrapping around.
pub(crate) fn step(current: Option<usize>, total: usize, back: bool) -> Option<usize> {
    if total == 0 {
        return None;
    }
    Some(match (current, back) {
        (None, false) => 0,
        (None, true) => total - 1,
        (Some(i), false) => (i + 1) % total,
        (Some(i), true) => (i + total - 1) % total,
    })
}

/// Whether ⌘F (Mac) or Ctrl+F opens find: the shortcut shown in tooltips.
pub(crate) fn shortcut_label() -> &'static str {
    if crate::prefs::desktop() == crate::prefs::Desktop::Mac { "⌘F" } else { "Ctrl+F" }
}

fn is_find_key(ev: &web_sys::KeyboardEvent, key: &str) -> bool {
    (ev.meta_key() || ev.ctrl_key()) && !ev.alt_key() && ev.key().eq_ignore_ascii_case(key)
}

/// Every match under `root` as DOM ranges, in document order.
fn find_ranges(root: &web_sys::Element, query: &str) -> Vec<web_sys::Range> {
    let Some(doc) = root.owner_document() else { return vec![] };
    // 4 = NodeFilter.SHOW_TEXT
    let Ok(walker) = doc.create_tree_walker_with_what_to_show(root, 4) else { return vec![] };
    let mut out = Vec::new();
    while let Ok(Some(node)) = walker.next_node() {
        let Some(parent) = node.parent_element() else { continue };
        if parent.closest(SKIP).ok().flatten().is_some() {
            continue;
        }
        let text = node.text_content().unwrap_or_default();
        for (start, end) in matches_in(&text, query) {
            if let Ok(range) = doc.create_range()
                && range.set_start(&node, start).is_ok()
                && range.set_end(&node, end).is_ok()
            {
                out.push(range);
            }
        }
    }
    out
}

/// `CSS.highlights`, where the Custom Highlight API exists.
fn highlights() -> Option<js_sys::Object> {
    let css = js_sys::Reflect::get(&js_sys::global(), &"CSS".into()).ok()?;
    let registry = js_sys::Reflect::get(&css, &"highlights".into()).ok()?;
    let has_ctor = js_sys::Reflect::get(&js_sys::global(), &"Highlight".into()).ok()?.is_function();
    (has_ctor && registry.is_object()).then(|| registry.unchecked_into())
}

/// Sets highlight `name` to `ranges` (none clears it).
fn paint(name: &str, ranges: &[web_sys::Range]) {
    let Some(registry) = highlights() else { return };
    let call = |method: &str, args: &js_sys::Array| {
        if let Ok(f) = js_sys::Reflect::get(&registry, &method.into()) {
            let _ = f.unchecked_ref::<js_sys::Function>().apply(&registry, args);
        }
    };
    if ranges.is_empty() {
        call("delete", &js_sys::Array::of1(&name.into()));
        return;
    }
    let Ok(ctor) = js_sys::Reflect::get(&js_sys::global(), &"Highlight".into()) else { return };
    let args: js_sys::Array = ranges.iter().cloned().collect();
    if let Ok(h) = js_sys::Reflect::construct(ctor.unchecked_ref::<js_sys::Function>(), &args) {
        call("set", &js_sys::Array::of2(&name.into(), &h));
    }
}

/// Opens every collapsed section around `node` (their bodies are rendered but
/// hidden while finding), like clicking their titles.
fn open_sections_around(node: &web_sys::Node) {
    let mut el = node.parent_element();
    while let Some(e) = el {
        if e.class_list().contains("find-hidden")
            && let Some(title) = e.parent_element().and_then(|s| s.query_selector(":scope > .dsection-head > .dsection-title").ok().flatten())
            && let Ok(title) = title.dyn_into::<web_sys::HtmlElement>()
        {
            title.click();
        }
        el = e.parent_element();
    }
}

/// Scrolls the nearest scrolling ancestors so `range` is in view, centred.
fn reveal(range: &web_sys::Range) {
    let Ok(node) = range.start_container() else { return };
    let mut el = node.parent_element();
    while let Some(e) = el {
        let style = leptos::prelude::window().get_computed_style(&e).ok().flatten();
        let scrolls = |prop: &str| {
            style.as_ref().and_then(|s| s.get_property_value(prop).ok()).is_some_and(|v| v == "auto" || v == "scroll")
        };
        let r = range.get_bounding_client_rect();
        let b = e.get_bounding_client_rect();
        if scrolls("overflow-y") && e.scroll_height() > e.client_height() {
            let dy = r.top() - b.top() - (b.height() - r.height()) / 2.0;
            e.set_scroll_top(e.scroll_top() + dy as i32);
        }
        if scrolls("overflow-x") && e.scroll_width() > e.client_width() && (r.left() < b.left() || r.right() > b.right()) {
            let dx = r.left() - b.left() - (b.width() - r.width()) / 2.0;
            e.set_scroll_left(e.scroll_left() + dx as i32);
        }
        el = e.parent_element();
    }
}

/// Shows the current match: highlighted, or selected where highlights
/// aren't available.
fn mark_current(range: Option<&web_sys::Range>) {
    match range {
        Some(r) if highlights().is_some() => paint(CURRENT, std::slice::from_ref(r)),
        Some(r) => {
            if let Ok(Some(sel)) = leptos::prelude::window().get_selection() {
                let _ = sel.remove_all_ranges();
                let _ = sel.add_range(r);
            }
        }
        None => paint(CURRENT, &[]),
    }
}

/// The find bar for one panel. `root` selects the panel's element.
#[component]
pub fn FindBar(store: Store, root: &'static str) -> impl IntoView {
    let input = NodeRef::<leptos::html::Input>::new();
    let ranges = StoredValue::new_local(Vec::<web_sys::Range>::new());
    let total = RwSignal::new(0usize);
    let current = RwSignal::new(None::<usize>);
    let root_el = move || leptos::prelude::document().query_selector(root).ok().flatten();

    // Finds the matches again; keeps the position where it can.
    let rescan = move || {
        let query = store.find_query.get_untracked();
        let found = if store.find_open.get_untracked() { root_el().map(|r| find_ranges(&r, &query)).unwrap_or_default() } else { vec![] };
        paint(ALL, &found);
        let n = found.len();
        let at = current.get_untracked().map(|i| i.min(n.saturating_sub(1))).filter(|_| n > 0);
        mark_current(at.and_then(|i| found.get(i)));
        ranges.set_value(found);
        total.set(n);
        current.set(at);
    };
    let go = move |back: bool| {
        rescan();
        let Some(i) = step(current.get_untracked(), total.get_untracked(), back) else { return };
        current.set(Some(i));
        let range = ranges.with_value(|r| r.get(i).cloned());
        if let Some(range) = range {
            let hidden = range.start_container().ok().is_some_and(|n| {
                n.parent_element().and_then(|e| e.closest(".find-hidden").ok().flatten()).is_some()
            });
            if let Ok(n) = range.start_container() {
                open_sections_around(&n);
            }
            mark_current(Some(&range));
            // An opened section lays out first.
            let delay = if hidden { 50 } else { 0 };
            set_timeout(move || reveal(&range), std::time::Duration::from_millis(delay));
        }
    };

    // Typing: the first match from the top.
    Effect::new(move |_| {
        let _ = store.find_query.get();
        let _ = store.find_open.get();
        current.set(None);
        // After hidden section bodies render.
        set_timeout(move || if store.find_open.get_untracked() && !store.find_query.with_untracked(|q| q.trim().is_empty()) { go(false) } else { rescan() }, std::time::Duration::from_millis(0));
    });

    // The panel changing (refreshes, tabs, sections) finds again, a little later.
    let pending = StoredValue::new(false);
    let observer = {
        let on_change = Closure::<dyn FnMut()>::new(move || {
            if pending.get_value() || !store.find_open.get_untracked() {
                return;
            }
            pending.set_value(true);
            set_timeout(move || {
                pending.set_value(false);
                rescan();
            }, std::time::Duration::from_millis(150));
        });
        let observer = web_sys::MutationObserver::new(on_change.as_ref().unchecked_ref()).ok();
        on_change.forget();
        observer
    };
    if let Some(obs) = observer.clone() {
        Effect::new(move |_| {
            obs.disconnect();
            if store.find_open.get()
                && let Some(r) = root_el()
            {
                let opts = web_sys::MutationObserverInit::new();
                opts.set_child_list(true);
                opts.set_subtree(true);
                opts.set_character_data(true);
                let _ = obs.observe_with_options(&r, &opts);
            }
        });
    }

    // ⌘F / Ctrl+F opens (or refocuses) the bar; ⌘G / ⇧⌘G step through.
    let keys = window_event_listener(leptos::ev::keydown, move |ev| {
        if crate::modal::any_open() || root_el().is_none() {
            return;
        }
        if is_find_key(&ev, "f") && !ev.shift_key() {
            ev.prevent_default();
            store.find_open.set(true);
            set_timeout(move || if let Some(i) = input.get_untracked() { let _ = i.focus(); i.select(); }, std::time::Duration::from_millis(0));
        } else if is_find_key(&ev, "g") && store.find_open.get_untracked() {
            ev.prevent_default();
            go(ev.shift_key());
        }
    });
    let close = move || {
        store.find_open.set(false);
        paint(ALL, &[]);
        paint(CURRENT, &[]);
    };
    on_cleanup(move || {
        keys.remove();
        if let Some(obs) = observer {
            obs.disconnect();
        }
        paint(ALL, &[]);
        paint(CURRENT, &[]);
    });

    move || {
        store.find_open.get().then(|| view! {
            <div class="find-bar" role="search">
                <input
                    node_ref=input
                    class="search find-input"
                    type="search"
                    placeholder="Find in this panel"
                    aria-label="Find in this panel"
                    autofocus
                    spellcheck="false"
                    prop:value=move || store.find_query.get()
                    on:input=move |ev| store.find_query.set(event_target_value(&ev))
                    on:keydown=move |ev| match ev.key().as_str() {
                        "Enter" => { ev.prevent_default(); go(ev.shift_key()); }
                        "Escape" => { ev.prevent_default(); ev.stop_propagation(); close(); }
                        _ => {}
                    }
                />
                <span class="find-count muted small" aria-live="polite">
                    {move || count_text(current.get(), total.get(), &store.find_query.get())}
                </span>
                <button class="btn small find-step" title="Previous match (Shift+Enter)" aria-label="Previous match" prop:disabled=move || total.get() == 0 on:click=move |_| go(true)>"↑"</button>
                <button class="btn small find-step" title="Next match (Enter)" aria-label="Next match" prop:disabled=move || total.get() == 0 on:click=move |_| go(false)>"↓"</button>
                <button class="close" title="Close find (Esc)" aria-label="Close find" on:click=move |_| close()>"×"</button>
            </div>
        })
    }
}

/// The header button that opens find.
#[component]
pub fn FindButton(store: Store) -> impl IntoView {
    let title = format!("Find in this panel ({})", shortcut_label());
    view! {
        <button
            class="btn small find-open"
            class:on=move || store.find_open.get()
            title=title
            aria-label="Find in this panel"
            on:click=move |_| store.find_open.update(|o| *o = !*o)
        >
            <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
                <circle cx="11" cy="11" r="6.5" /><path d="M16 16l4.5 4.5" />
            </svg>
        </button>
    }
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests_unit;
