//! Downloading a file from a container into the Downloads folder: browse to
//! the file (or type its path), then save. Read-only for the cluster, so no
//! unlock is needed.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{DirListing, DownloadResult};

use crate::app::Store;
use crate::browser::{BrowseMode, FolderBrowser, folder_of, join};
use crate::overview::fmt_bytes;
use crate::tauri::invoke;

/// The container the dialog downloads from.
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadPrompt {
    pub namespace: String,
    pub pod: String,
    pub container: String,
}

pub fn request(store: Store, namespace: String, pod: String, container: String) {
    store.download_prompt.set(Some(DownloadPrompt { namespace, pod, container }));
}

#[derive(Clone)]
enum Phase {
    Choosing,
    Downloading,
    Done(DownloadResult),
}

#[component]
pub fn DownloadDialog(store: Store) -> impl IntoView {
    let path = RwSignal::new(String::new());
    let phase = RwSignal::new(Phase::Choosing);
    let error = RwSignal::new(None::<String>);
    let listing = RwSignal::new(None::<Result<DirListing, String>>);
    let suggestions = RwSignal::new(Vec::<String>::new());
    let browsing = RwSignal::new(false);

    let browse = move |dir: String, _suggest: bool| {
        let Some(prompt) = store.download_prompt.get_untracked() else { return };
        browsing.set(true);
        spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                namespace: String,
                pod: String,
                container: String,
                path: String,
                suggest: bool,
            }
            let args = Args { namespace: prompt.namespace, pod: prompt.pod, container: prompt.container, path: dir, suggest: false };
            let result = invoke::<_, DirListing>("browse_dir", &args).await;
            if let Ok(l) = &result {
                path.set(join(&l.path, ""));
            }
            listing.set(Some(result));
            browsing.set(false);
        });
    };

    // Fresh state each time; start in the container's working directory.
    Effect::new(move |_| {
        if store.download_prompt.with(Option::is_some) {
            path.set(String::new());
            phase.set(Phase::Choosing);
            error.set(None);
            listing.set(None);
            browse(".".into(), false);
        }
    });

    let download = move || {
        let Some(prompt) = store.download_prompt.get_untracked() else { return };
        let file = path.get_untracked().trim().to_owned();
        phase.set(Phase::Downloading);
        error.set(None);
        spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                namespace: String,
                pod: String,
                container: String,
                path: String,
            }
            let args = Args { namespace: prompt.namespace, pod: prompt.pod, container: prompt.container, path: file };
            match invoke::<_, DownloadResult>("download_file", &args).await {
                Ok(r) => phase.set(Phase::Done(r)),
                Err(e) => {
                    error.set(Some(e));
                    phase.set(Phase::Choosing);
                }
            }
        });
    };
    let reveal = move |saved: String| {
        spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                path: String,
            }
            if let Err(e) = invoke::<_, ()>("reveal_download", &Args { path: saved }).await {
                error.set(Some(e));
            }
        });
    };
    let close = move || store.download_prompt.set(None);
    let is_file = move || path.with(|p| p.trim().starts_with('/') && !p.trim().ends_with('/'));

    move || {
        store.download_prompt.get().map(|prompt| {
            let source = format!("{} · {}/{}", prompt.container, prompt.namespace, prompt.pod);
            let body = move || match phase.get() {
                Phase::Done(r) => {
                    let saved = r.saved_to.clone();
                    view! {
                        <p>"Saved "{fmt_bytes(r.bytes as f64)}" to "<strong class="mono">{r.saved_to.clone()}</strong>"."</p>
                        {move || error.get().map(|e| view! { <p class="warn">{e}</p> })}
                        <div class="modal-actions">
                            <button on:click=move |_| reveal(saved.clone())>{crate::prefs::reveal_label()}</button>
                            <button class="primary" on:click=move |_| close()>"Done"</button>
                        </div>
                    }
                    .into_any()
                }
                Phase::Choosing | Phase::Downloading => {
                    let downloading = matches!(phase.get(), Phase::Downloading);
                    view! {
                        <label class="local-port">
                            "File in the container"
                            <input
                                class="confirm-input mono"
                                placeholder="/var/log/app.log"
                                prop:value=move || path.get()
                                on:input=move |ev| path.set(event_target_value(&ev))
                                on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                                    if ev.key() == "Enter" {
                                        if is_file() {
                                            download();
                                        } else {
                                            browse(folder_of(&path.get_untracked()), false);
                                        }
                                    }
                                }
                            />
                            <span class="muted">"Saved to your Downloads folder; an existing file there is kept and the new one numbered."</span>
                        </label>
                        <FolderBrowser
                            listing=listing
                            suggestions=suggestions
                            browsing=browsing
                            browse=browse
                            pick_file=move |p| path.set(p)
                            mode=BrowseMode::Download
                        />
                        {move || error.get().map(|e| view! { <p class="warn">{e}</p> })}
                        <div class="modal-actions">
                            <button on:click=move |_| close()>"Cancel"</button>
                            <button class="primary" prop:disabled=move || downloading || !is_file() on:click=move |_| download()>
                                {if downloading { "Downloading…" } else { "Download" }}
                            </button>
                        </div>
                    }
                    .into_any()
                }
            };
            view! {
                <div class="modal-backdrop">
                    <div class="modal wide">
                        <h2>"Download file"</h2>
                        <p class="mono">{source}</p>
                        {body}
                    </div>
                </div>
            }
        })
    }
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod tests_unit;
