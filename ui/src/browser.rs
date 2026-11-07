//! The container folder browser shared by the upload and download dialogs:
//! writable-place shortcuts, breadcrumbs, and the current folder's entries.

use leptos::prelude::*;
use subduer_types::{DirEntry, DirListing};

use crate::logic::browser::{at_root, crumb_paths, entry_state, entry_title, folder_status, show_chips};
pub(crate) use crate::logic::browser::{BrowseMode, folder_of, join};

/// Writable-place shortcuts, breadcrumbs and the current folder's entries.
/// Clicking a folder opens it; clicking a file passes it to `pick_file`.
#[component]
pub fn FolderBrowser(
    listing: RwSignal<Option<Result<DirListing, String>>>,
    suggestions: RwSignal<Vec<String>>,
    browsing: RwSignal<bool>,
    browse: impl Fn(String, bool) + Copy + Send + Sync + 'static,
    /// Called with a file's full path when it's clicked.
    pick_file: impl Fn(String) + Copy + Send + Sync + 'static,
    mode: BrowseMode,
) -> impl IntoView {
    let chips = move || {
        suggestions
            .get()
            .into_iter()
            .map(|s| {
                let to = s.clone();
                view! { <button class="chip-button mono" title="Writable" on:click=move |_| browse(to.clone(), false)>{s}</button> }
            })
            .collect_view()
    };
    let crumbs = move |path: &str| {
        crumb_paths(path)
            .into_iter()
            .map(|(label, to)| view! { <button class="crumb mono" on:click=move |_| browse(to.clone(), false)>{label}</button> })
            .collect_view()
    };
    let entry_row = move |dir_path: String, e: DirEntry| {
        let full = join(&dir_path, &e.name);
        let (note, dim, enabled) = entry_state(mode, &e);
        let title = entry_title(mode, e.dir, &full);
        let click = move |_| {
            if e.dir {
                browse(full.clone(), false);
            } else {
                pick_file(full.clone());
            }
        };
        view! {
            <button class="browse-entry" class:dim=dim class:file=!e.dir title=title prop:disabled=!enabled on:click=click>
                <crate::icons::EntryIcon dir=e.dir />
                <span class="mono name">{e.name.clone()}</span>
                <span class="muted entry-note">{note}</span>
            </button>
        }
    };

    view! {
        <div class="browser" class:stale=move || browsing.get()>
            <Show when=move || suggestions.with(|s| show_chips(mode, s))>
                <div class="browser-chips"><span class="muted">"Writable:"</span>{chips}</div>
            </Show>
            {move || match listing.get() {
                None => view! { <p class="muted browser-msg">"Reading the container's folders…"</p> }.into_any(),
                Some(Err(e)) => view! { <p class="warn browser-msg">{e}</p> }.into_any(),
                Some(Ok(l)) => {
                    let parent = folder_of(&l.path);
                    let at_root = at_root(&l.path);
                    let status = folder_status(mode, l.writable);
                    let rows = l.entries.iter().cloned().map(|e| entry_row(l.path.clone(), e)).collect_view();
                    view! {
                        <div class="browser-bar">
                            <button class="btn small" prop:disabled=at_root title="Up one folder" on:click=move |_| browse(parent.clone(), false)>"↑"</button>
                            <div class="crumbs">{crumbs(&l.path)}</div>
                            <span class="browser-status" class:ok=l.writable>{status}</span>
                        </div>
                        <div class="browser-list">
                            {rows}
                            {l.entries.is_empty().then(|| view! { <p class="muted browser-msg">"Empty folder"</p> })}
                            {l.truncated.then(|| view! { <p class="muted browser-msg">"Showing the first 1000 entries."</p> })}
                        </div>
                    }
                    .into_any()
                }
            }}
        </div>
    }
}
