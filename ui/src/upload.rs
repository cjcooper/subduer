//! Uploading a file into a container: the dialog (pick or drop a file, browse
//! to a writable folder, optionally rename) and the request helper, which asks
//! to unlock the context first.

use leptos::prelude::*;
use leptos::task::spawn_local;
use subduer_types::{DirListing, UploadOutcome};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::app::Store;
use crate::browser::{BrowseMode, FolderBrowser, folder_of};
use crate::logic::download::BrowseArgs;
use crate::logic::upload::{self as logic, Phase, START_DIR};
pub use crate::logic::upload::UploadPrompt;
use crate::modal::{Generation, Modal};
use crate::tauri::{invoke, invoke_bytes};

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

#[component]
pub fn UploadDialog(store: Store) -> impl IntoView {
    let file = StoredValue::new_local(None::<web_sys::File>);
    let file_label = RwSignal::new(None::<(String, f64)>);
    let dest = RwSignal::new(logic::start_dest());
    let phase = RwSignal::new(Phase::Choosing);
    let error = RwSignal::new(None::<String>);
    let dragging = RwSignal::new(false);
    let listing = RwSignal::new(None::<Result<DirListing, String>>);
    let suggestions = RwSignal::new(Vec::<String>::new());
    let browsing = RwSignal::new(false);

    // Results of requests sent before the dialog last opened or closed are dropped.
    let generation = Generation::new();

    let browse = move |path: String, suggest: bool| {
        let Some(prompt) = store.upload_prompt.get_untracked() else { return };
        browsing.set(true);
        let sent = generation.now();
        spawn_local(async move {
            let args = BrowseArgs { namespace: prompt.namespace, pod: prompt.pod, container: prompt.container, path, suggest };
            let mut result = invoke::<_, DirListing>("browse_dir", &args).await;
            // No /tmp: start at the root instead.
            if logic::retry_at_root(suggest, result.is_err()) {
                result = invoke::<_, DirListing>("browse_dir", &BrowseArgs { path: "/".into(), ..args }).await;
            }
            if !generation.still(sent) {
                return;
            }
            if let Ok(l) = &result {
                if suggest {
                    suggestions.set(l.suggestions.clone());
                }
                // Point the destination at the folder, keeping a typed file name.
                dest.set(dest.with_untracked(|d| logic::browsed_dest(&l.path, d)));
            }
            listing.set(Some(result));
            browsing.set(false);
        });
    };

    // Fresh state each time the dialog opens.
    Effect::new(move |_| {
        generation.next();
        if store.upload_prompt.with(Option::is_some) {
            file.set_value(None);
            file_label.set(None);
            dest.set(logic::start_dest());
            phase.set(Phase::Choosing);
            error.set(None);
            listing.set(None);
            suggestions.set(vec![]);
            browse(START_DIR.into(), true);
        }
    });

    let choose = move |f: web_sys::File| {
        let (name, size) = (f.name(), f.size());
        error.set(logic::size_error(size));
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
        let target = logic::target(prompt, &dest.get_untracked(), name, overwrite);
        phase.set(Phase::Uploading);
        error.set(None);
        let sent = generation.now();
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
            if !generation.still(sent) {
                return;
            }
            let (next, failed) = logic::after_upload(result);
            if failed.is_some() {
                error.set(failed);
            }
            phase.set(next);
        });
    };
    let close = move || store.upload_prompt.set(None);
    // Esc: from "already exists", back to choosing; otherwise close.
    let cancel = move || {
        if phase.with_untracked(logic::cancel_goes_back) {
            phase.set(Phase::Choosing);
        } else {
            close();
        }
    };
    let can_upload = move || file_label.with(|f| dest.with(|d| logic::can_upload(f.as_ref().map(|(_, size)| *size), d)));
    // What the destination will mean once the file is chosen.
    let hint = move || {
        let d = dest.get();
        let name = file_label.get().map(|(name, _)| name);
        listing.with(|l| logic::dest_hint(&d, l.as_ref().and_then(|l| l.as_ref().ok()), name.as_deref()))
    };

    move || {
        store.upload_prompt.get().map(|prompt| {
            let target = logic::target_line(&prompt);
            let body = move || match phase.get() {
                Phase::Done { bytes, path } => view! {
                    <p>"Uploaded "{crate::logic::download::size(bytes)}" to "<strong class="mono">{path}</strong>"."</p>
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
                                    <span class="muted">{logic::file_size(size)}</span>
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
                            <button prop:disabled=uploading on:click=move |_| close()>"Cancel"</button>
                            <button class="primary" prop:disabled=move || uploading || !can_upload() on:click=move |_| send(false)>
                                {if uploading { "Uploading…" } else { "Upload" }}
                            </button>
                        </div>
                    }
                    .into_any()
                }
            };
            view! {
                <Modal on_cancel=move |()| cancel() busy=Signal::derive(move || matches!(phase.get(), Phase::Uploading)) wide=true>
                    <h2>"Upload file"</h2>
                    <p class="mono">{target}</p>
                    {body}
                </Modal>
            }
        })
    }
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod tests_unit;
