//! The container folder browser shared by the upload and download dialogs:
//! writable-place shortcuts, breadcrumbs, and the current folder's entries.

use leptos::prelude::*;
use subduer_types::{DirEntry, DirListing};

/// `dir` joined with `name`, without doubling the slash.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

/// The folder part of a path: itself if it ends with `/`, else its parent.
pub fn folder_of(path: &str) -> String {
    match path.trim().rsplit_once('/') {
        Some(("", _)) => "/".into(),
        Some((dir, _)) => dir.into(),
        None => "/".into(),
    }
}

/// What the browser is for, which changes what entries say and allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseMode {
    /// Folders show whether they're writable; files can be replaced.
    Upload,
    /// Files show whether they're readable; unreadable ones can't be picked.
    Download,
}

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
        let mut acc = String::new();
        let mut out = vec![("/".to_owned(), "/".to_owned())];
        for part in path.split('/').filter(|p| !p.is_empty()) {
            acc.push('/');
            acc.push_str(part);
            out.push((part.to_owned(), acc.clone()));
        }
        out.into_iter()
            .map(|(label, to)| view! { <button class="crumb mono" on:click=move |_| browse(to.clone(), false)>{label}</button> })
            .collect_view()
    };
    let entry_row = move |dir_path: String, e: DirEntry| {
        let full = join(&dir_path, &e.name);
        let icon = if e.dir { "📁" } else { "📄" };
        let (note, dim, enabled) = match (mode, e.dir) {
            (BrowseMode::Upload, true) => (if e.writable { "writable" } else { "read-only" }, !e.writable, true),
            (BrowseMode::Upload, false) => (if e.writable { "replace" } else { "read-only" }, !e.writable, true),
            (BrowseMode::Download, true) => (if e.readable { "" } else { "no access" }, !e.readable, e.readable),
            (BrowseMode::Download, false) => (if e.readable { "download" } else { "can't read" }, !e.readable, e.readable),
        };
        let title = match (mode, e.dir) {
            (_, true) => format!("Open {full}"),
            (BrowseMode::Upload, false) => format!("Upload as {full} (asks before replacing)"),
            (BrowseMode::Download, false) => format!("Download {full}"),
        };
        let click = move |_| {
            if e.dir {
                browse(full.clone(), false);
            } else {
                pick_file(full.clone());
            }
        };
        view! {
            <button class="browse-entry" class:dim=dim class:file=!e.dir title=title prop:disabled=!enabled on:click=click>
                <span aria-hidden="true">{icon}</span>
                <span class="mono name">{e.name.clone()}</span>
                <span class="muted entry-note">{note}</span>
            </button>
        }
    };

    view! {
        <div class="browser" class:stale=move || browsing.get()>
            <Show when=move || mode == BrowseMode::Upload && suggestions.with(|s| !s.is_empty())>
                <div class="browser-chips"><span class="muted">"Writable:"</span>{chips}</div>
            </Show>
            {move || match listing.get() {
                None => view! { <p class="muted browser-msg">"Reading the container's folders…"</p> }.into_any(),
                Some(Err(e)) => view! { <p class="warn browser-msg">{e}</p> }.into_any(),
                Some(Ok(l)) => {
                    let parent = folder_of(&l.path);
                    let at_root = l.path == "/";
                    let status = match (mode, l.writable) {
                        (BrowseMode::Upload, true) => "✓ writable",
                        (BrowseMode::Upload, false) => "read-only: files can't be saved here",
                        (BrowseMode::Download, _) => "",
                    };
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
