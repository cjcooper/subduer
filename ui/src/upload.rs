//! Uploading a file into a container: the dialog (pick or drop a file, browse
//! to a writable folder, optionally rename) and the request helper, which asks
//! to unlock the context first.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{DirListing, UploadOutcome, UploadTarget};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::app::Store;
use crate::browser::{BrowseMode, FolderBrowser, folder_of, join};
use crate::overview::fmt_bytes;
use crate::tauri::{invoke, invoke_bytes};

/// Matches the backend limit; the file is held in memory.
const MAX_BYTES: f64 = 512.0 * 1024.0 * 1024.0;
const START_DIR: &str = "/tmp";

/// The container the dialog uploads into.
#[derive(Debug, Clone, PartialEq)]
pub struct UploadPrompt {
    pub context: String,
    pub namespace: String,
    pub pod: String,
    pub container: String,
}

/// Opens the dialog, or the unlock prompt first if the context is read-only.
pub fn request(store: Store, namespace: String, pod: String, container: String) {
    let Some(context) = store.current_context_untracked() else { return };
    let prompt = UploadPrompt { context: context.name.clone(), namespace, pod, container };
    if context.unlocked {
        store.upload_prompt.set(Some(prompt));
    } else {
        store.pending_upload.set(Some(prompt));
        store.unlock_prompt.set(Some(context));
    }
}

#[derive(Clone)]
enum Phase {
    Choosing,
    Uploading,
    /// The file exists; offer to replace it.
    Exists(String),
    Done { bytes: u64, path: String },
}

/// What the destination `dest` will mean for a file named `name` (if chosen),
/// given the folder on screen.
fn dest_hint(dest: &str, shown: Option<&DirListing>, name: Option<&str>) -> String {
    let d = dest.trim();
    if !d.starts_with('/') {
        return "Use an absolute path, e.g. /tmp/".to_owned();
    }
    // Warn before uploading into the folder on screen when it's read-only.
    let read_only =
        shown.is_some_and(|l| !l.writable && folder_of(d).trim_end_matches('/') == l.path.trim_end_matches('/'));
    if read_only {
        return "This folder is read-only for the container user; pick a writable one.".to_owned();
    }
    match name {
        Some(name) if d.ends_with('/') => format!("Saved as {}", join(d, name)),
        Some(name) => format!("Saved as {d}, or as {} if {d} is a folder", join(d, name)),
        None if d.ends_with('/') => "The file keeps its name in this folder.".into(),
        None => "A folder keeps the file's name; anything else is the new file's path.".into(),
    }
}

#[component]
pub fn UploadDialog(store: Store) -> impl IntoView {
    let file = StoredValue::new_local(None::<web_sys::File>);
    let file_label = RwSignal::new(None::<(String, f64)>);
    let dest = RwSignal::new(format!("{START_DIR}/"));
    let phase = RwSignal::new(Phase::Choosing);
    let error = RwSignal::new(None::<String>);
    let dragging = RwSignal::new(false);
    let listing = RwSignal::new(None::<Result<DirListing, String>>);
    let suggestions = RwSignal::new(Vec::<String>::new());
    let browsing = RwSignal::new(false);

    let browse = move |path: String, suggest: bool| {
        let Some(prompt) = store.upload_prompt.get_untracked() else { return };
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
            let args = Args { namespace: prompt.namespace, pod: prompt.pod, container: prompt.container, path, suggest };
            let mut result = invoke::<_, DirListing>("browse_dir", &args).await;
            // No /tmp: start at the root instead.
            if suggest && result.is_err() {
                result = invoke::<_, DirListing>("browse_dir", &Args { path: "/".into(), ..args }).await;
            }
            if let Ok(l) = &result {
                if suggest {
                    suggestions.set(l.suggestions.clone());
                }
                // Point the destination at the folder, keeping a typed file name.
                let name = dest.with_untracked(|d| {
                    let tail = d.rsplit('/').next().unwrap_or_default();
                    tail.to_owned()
                });
                dest.set(join(&l.path, &name));
            }
            listing.set(Some(result));
            browsing.set(false);
        });
    };

    // Fresh state each time the dialog opens.
    Effect::new(move |_| {
        if store.upload_prompt.with(Option::is_some) {
            file.set_value(None);
            file_label.set(None);
            dest.set(format!("{START_DIR}/"));
            phase.set(Phase::Choosing);
            error.set(None);
            listing.set(None);
            suggestions.set(vec![]);
            browse(START_DIR.into(), true);
        }
    });

    let choose = move |f: web_sys::File| {
        let (name, size) = (f.name(), f.size());
        error.set(if size > MAX_BYTES { Some("Files over 512 MiB aren't supported.".into()) } else { None });
        file_label.set(Some((name, size)));
        file.set_value(Some(f));
    };
    let on_input = move |ev: leptos::ev::Event| {
        let input: web_sys::HtmlInputElement = ev.target().expect("event target").unchecked_into();
        if let Some(f) = input.files().and_then(|l| l.get(0)) {
            choose(f);
        }
    };
    let on_drop = move |ev: web_sys::DragEvent| {
        ev.prevent_default();
        dragging.set(false);
        if let Some(f) = ev.data_transfer().and_then(|d| d.files()).and_then(|l| l.get(0)) {
            choose(f);
        }
    };

    let send = move |overwrite: bool| {
        let (Some(prompt), Some(f), Some((name, _))) =
            (store.upload_prompt.get_untracked(), file.get_value(), file_label.get_untracked())
        else {
            return;
        };
        let target = UploadTarget {
            context: prompt.context,
            namespace: prompt.namespace,
            pod: prompt.pod,
            container: prompt.container,
            path: dest.get_untracked().trim().to_owned(),
            file_name: name,
            overwrite,
        };
        phase.set(Phase::Uploading);
        error.set(None);
        spawn_local(async move {
            let result = async {
                let buffer = JsFuture::from(f.array_buffer()).await.map_err(|_| "Couldn't read the file.".to_owned())?;
                let bytes = js_sys::Uint8Array::new(&buffer);
                let value = serde_wasm_bindgen::to_value(&target).map_err(|e| e.to_string())?;
                let json: String = js_sys::JSON::stringify(&value).map_err(|_| "bad upload target".to_owned())?.into();
                let header: String = js_sys::encode_uri_component(&json).into();
                invoke_bytes::<UploadOutcome>("upload_file", &bytes, &[("x-upload", &header)]).await
            }
            .await;
            match result {
                Ok(UploadOutcome::Uploaded { bytes, path }) => phase.set(Phase::Done { bytes, path }),
                Ok(UploadOutcome::Exists { path }) => phase.set(Phase::Exists(path)),
                Err(e) => {
                    error.set(Some(e));
                    phase.set(Phase::Choosing);
                }
            }
        });
    };
    let close = move || store.upload_prompt.set(None);
    let dest_ok = move || dest.with(|p| p.trim().starts_with('/'));
    let can_upload = move || file_label.with(|f| f.as_ref().is_some_and(|(_, size)| *size <= MAX_BYTES)) && dest_ok();
    // What the destination will mean once the file is chosen.
    let hint = move || {
        let d = dest.get();
        let name = file_label.get().map(|(name, _)| name);
        listing.with(|l| dest_hint(&d, l.as_ref().and_then(|l| l.as_ref().ok()), name.as_deref()))
    };

    move || {
        store.upload_prompt.get().map(|prompt| {
            let target = format!("{} · {}/{} · {}", prompt.container, prompt.namespace, prompt.pod, prompt.context);
            let body = move || match phase.get() {
                Phase::Done { bytes, path } => view! {
                    <p>"Uploaded "{fmt_bytes(bytes as f64)}" to "<strong class="mono">{path}</strong>"."</p>
                    <div class="modal-actions"><button class="primary" on:click=move |_| close()>"Done"</button></div>
                }
                .into_any(),
                Phase::Exists(path) => view! {
                    <p><strong class="mono">{path}</strong>" already exists in the container."</p>
                    <div class="modal-actions">
                        <button on:click=move |_| phase.set(Phase::Choosing)>"Back"</button>
                        <button class="danger" on:click=move |_| send(true)>"Replace it"</button>
                    </div>
                }
                .into_any(),
                Phase::Choosing | Phase::Uploading => {
                    let uploading = matches!(phase.get(), Phase::Uploading);
                    view! {
                        <label
                            class="drop-zone"
                            class:dragging=move || dragging.get()
                            on:dragover=move |ev: web_sys::DragEvent| {
                                ev.prevent_default();
                                dragging.set(true);
                            }
                            on:dragleave=move |_| dragging.set(false)
                            on:drop=on_drop
                        >
                            <input type="file" class="visually-hidden" on:change=on_input />
                            {move || match file_label.get() {
                                Some((name, size)) => view! {
                                    <span class="mono">{name}</span>
                                    <span class="muted">{format!(" · {}", fmt_bytes(size))}</span>
                                }
                                .into_any(),
                                None => view! { <span class="muted">"Drop a file here, or click to choose one"</span> }.into_any(),
                            }}
                        </label>
                        <label class="local-port">
                            "Destination in the container"
                            <input
                                class="confirm-input mono"
                                placeholder="/tmp/"
                                prop:value=move || dest.get()
                                on:input=move |ev| dest.set(event_target_value(&ev))
                                on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                                    if ev.key() == "Enter" {
                                        browse(folder_of(&dest.get_untracked()), false);
                                    }
                                }
                            />
                            <span class="muted">{hint}</span>
                        </label>
                        <FolderBrowser
                            listing=listing
                            suggestions=suggestions
                            browsing=browsing
                            browse=browse
                            pick_file=move |path| dest.set(path)
                            mode=BrowseMode::Upload
                        />
                        {move || error.get().map(|e| view! { <p class="warn">{e}</p> })}
                        <div class="modal-actions">
                            <button on:click=move |_| close()>"Cancel"</button>
                            <button class="primary" prop:disabled=move || uploading || !can_upload() on:click=move |_| send(false)>
                                {if uploading { "Uploading…" } else { "Upload" }}
                            </button>
                        </div>
                    }
                    .into_any()
                }
            };
            view! {
                <div class="modal-backdrop">
                    <div class="modal wide">
                        <h2>"Upload file"</h2>
                        <p class="mono">{target}</p>
                        {body}
                    </div>
                </div>
            }
        })
    }
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod tests_unit;
