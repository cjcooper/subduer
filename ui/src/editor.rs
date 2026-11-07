//! The YAML edit window (opened with `?edit=<label>`): an Ace editor with YAML
//! highlighting, live syntax errors, auto-indent, folding, search and
//! Kubernetes key completion. Save validates with the server first, shows what
//! changed, then saves (gated like every other change); Cancel and closing ask
//! before throwing edits away. Also the main window's Edit button.

use js_sys::{Array, Function, Promise};
use leptos::html::Div;
use leptos::prelude::*;
use leptos::task::spawn_local;
use subduer_types::{EditSession, EditTarget, ResourceRow, edit_events};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::app::Store;
use crate::exec::{call, get, object};
use crate::logic::editor::{self as logic, After, Check, DirtyArgs, KUBERNETES_KEYS, Line, Modal, OpenArgs, SaveArgs, UnlockArgs, label_in};
use crate::tauri::{invoke, listen};

// ---- Opening from the main window ----

/// "Edit" above the YAML tab: shown when the API allows updating the kind,
/// and never for Helm releases (their YAML is a summary, not an object).
#[component]
pub fn EditButton(store: Store, row: ResourceRow) -> impl IntoView {
    move || {
        let kind = store.kind.get();
        let target = store.cluster.with(|c| logic::edit_target(kind, c.as_ref().map_or(&[], |c| &c.kinds), &row))?;
        Some(view! {
            <button class="btn small" title="Edit this YAML in a new window" on:click=move |_| open(store, target.clone())>
                "Edit"
            </button>
        })
    }
}

fn open(store: Store, target: EditTarget) {
    spawn_local(async move {
        if let Err(e) = invoke::<_, ()>("open_editor", &OpenArgs { target }).await {
            store.error.set(Some(e));
        }
    });
}

/// The window label from `?edit=<label>`, if this page is an edit window.
pub fn label_from_url() -> Option<String> {
    let search = window().location().search().ok()?;
    label_in(&search)
}

// ---- Ace ----

/// Loads a script from the app's own files once.
async fn load_script(src: &str) -> Result<(), String> {
    let document = document();
    if document.query_selector(&format!("script[src=\"{src}\"]")).ok().flatten().is_some() {
        return Ok(());
    }
    let script = document.create_element("script").map_err(|_| "can't create a script element")?;
    let _ = script.set_attribute("src", src);
    let promise = Promise::new(&mut |resolve, reject| {
        let _ = js_sys::Reflect::set(&script, &"onload".into(), &resolve);
        let _ = js_sys::Reflect::set(&script, &"onerror".into(), &reject);
    });
    let head = document.query_selector("head").ok().flatten().ok_or("no <head>")?;
    head.append_child(&script).map_err(|_| "can't add a script")?;
    wasm_bindgen_futures::JsFuture::from(promise).await.map(|_| ()).map_err(|_| format!("couldn't load {src}"))
}

fn ace_theme() -> &'static str {
    logic::ace_theme(crate::prefs::is_dark())
}

/// One Ace editor.
#[derive(Clone)]
struct Ace {
    editor: JsValue,
}

impl Ace {
    async fn new(host: web_sys::HtmlElement) -> Result<Self, String> {
        load_script("ace/ace.js").await?;
        load_script("ace/ext-language_tools.js").await?;
        let ace = get(&window().into(), "ace");
        call(&get(&ace, "config"), "set", &["basePath".into(), "ace".into()]);
        let options = object(&[
            ("mode", "ace/mode/yaml".into()),
            ("theme", ace_theme().into()),
            ("tabSize", 2.into()),
            ("useSoftTabs", true.into()),
            ("showPrintMargin", false.into()),
            ("fontSize", 13.into()),
            ("fontFamily", "\"SF Mono\", Menlo, Consolas, \"DejaVu Sans Mono\", monospace".into()),
            ("highlightActiveLine", true.into()),
            ("enableBasicAutocompletion", true.into()),
            ("enableLiveAutocompletion", true.into()),
            ("scrollPastEnd", 0.3.into()),
            ("navigateWithinSoftTabs", true.into()),
        ]);
        let editor = call(&ace, "edit", &[host.into(), options]);
        if editor.is_undefined() {
            return Err("The editor (Ace) failed to load.".into());
        }
        // Kubernetes keys alongside the document's own words.
        let keys: Array = KUBERNETES_KEYS.iter().map(|k| JsValue::from_str(k)).collect();
        let completer = Function::new_with_args(
            "keys",
            "return { getCompletions: function (editor, session, pos, prefix, callback) {
                callback(null, keys.map(function (k) { return { caption: k, value: k + ': ', meta: 'kubernetes', score: 10 }; }));
            } };",
        )
        .call1(&JsValue::NULL, &keys)
        .unwrap_or(JsValue::UNDEFINED);
        let tools = call(&ace, "require", &["ace/ext/language_tools".into()]);
        call(&tools, "addCompleter", &[completer]);
        Ok(Self { editor })
    }

    fn session(&self) -> JsValue {
        get(&self.editor, "session")
    }

    fn text(&self) -> String {
        call(&self.editor, "getValue", &[]).as_string().unwrap_or_default()
    }

    /// Replaces the text without selecting it, and clears undo history.
    fn set_text(&self, text: &str) {
        call(&self.session(), "setValue", &[text.into()]);
    }

    fn focus(&self) {
        call(&self.editor, "focus", &[]);
    }

    fn set_theme(&self) {
        call(&self.editor, "setTheme", &[ace_theme().into()]);
    }

    fn go_to(&self, row: u32) {
        call(&self.editor, "gotoLine", &[(row + 1).into(), 0.into(), true.into()]);
        self.focus();
    }

    /// Syntax errors from the YAML worker: (row, message).
    fn errors(&self) -> Vec<(u32, String)> {
        let list: Array = call(&self.session(), "getAnnotations", &[]).dyn_into().unwrap_or_default();
        list.iter()
            .filter(|a| get(a, "type").as_string().as_deref() == Some("error"))
            .map(|a| (get(&a, "row").as_f64().unwrap_or(0.0) as u32, get(&a, "text").as_string().unwrap_or_default()))
            .collect()
    }

    fn on(&self, target: &JsValue, event: &str, f: impl FnMut() + 'static) {
        let cb = Closure::<dyn FnMut()>::new(f);
        call(target, "on", &[event.into(), cb.as_ref().clone()]);
        cb.forget();
    }

    fn on_save(&self, f: impl FnMut() + 'static) {
        let cb = Closure::<dyn FnMut()>::new(f);
        let command = object(&[
            ("name", "save".into()),
            ("bindKey", object(&[("win", "Ctrl-S".into()), ("mac", "Cmd-S".into())])),
            ("exec", cb.as_ref().clone()),
        ]);
        call(&get(&self.editor, "commands"), "addCommand", &[command]);
        cb.forget();
    }
}

// ---- The window ----

#[component]
pub fn EditorApp(label: String) -> impl IntoView {
    let _ = label;
    let host = NodeRef::<Div>::new();
    let ace = StoredValue::new_local(None::<Ace>);
    let session = RwSignal::new(None::<Result<EditSession, String>>);
    let original = RwSignal::new(String::new());
    let dirty = RwSignal::new(false);
    let errors = RwSignal::new(Vec::<(u32, String)>::new());
    let message = RwSignal::new(None::<(bool, String)>);
    let busy = RwSignal::new(false);
    let modal = RwSignal::new(None::<Modal>);
    let typed = RwSignal::new(String::new());

    let report_dirty = move |now: bool| {
        if dirty.get_untracked() != now {
            dirty.set(now);
            spawn_local(async move {
                let _ = invoke::<_, ()>("editor_dirty", &DirtyArgs { dirty: now }).await;
            });
        }
    };

    let start = move || {
        spawn_local(async move {
            let result = invoke::<_, EditSession>("editor_start", &()).await;
            if let Ok(s) = &result {
                original.set(s.yaml.clone());
                ace.with_value(|a| {
                    if let Some(a) = a {
                        a.set_text(&s.yaml);
                        a.go_to(0);
                    }
                });
                report_dirty(false);
                message.set(None);
                document().set_title(&s.title);
            }
            session.set(Some(result));
        });
    };

    let close = move || {
        spawn_local(async move {
            let _ = invoke::<_, ()>("editor_close", &()).await;
        });
    };

    // Step 1 of Save: local checks, then the server validates (dry run).
    let save = move || {
        let Some(text) = ace.with_value(|a| a.as_ref().map(Ace::text)) else { return };
        let prompting = modal.with_untracked(Option::is_some);
        match errors.with_untracked(|e| logic::check(busy.get_untracked(), prompting, e, &text, &original.get_untracked())) {
            Check::Ignore => return,
            Check::Fix(row, m) => {
                message.set(Some((true, m)));
                ace.with_value(|a| a.as_ref().map(|a| a.go_to(row)));
                return;
            }
            Check::NoChanges => {
                message.set(Some((false, "No changes to save.".into())));
                return;
            }
            Check::Validate => {}
        }
        busy.set(true);
        message.set(Some((false, "Checking with the server…".into())));
        spawn_local(async move {
            let result = invoke::<_, String>("editor_save", &SaveArgs { yaml: &text, dry_run: true }).await;
            busy.set(false);
            match result {
                Err(e) => message.set(Some((true, e))),
                Ok(_) => {
                    message.set(None);
                    typed.set(String::new());
                    let unlocked = session.with_untracked(logic::unlocked);
                    modal.set(Some(logic::after_check(unlocked, &original.get_untracked(), &text)));
                }
            }
        });
    };

    // Step 2 for a read-only context: allow changes, as in the main window.
    let unlock = move || {
        let Some(Ok(s)) = session.get_untracked() else { return };
        let context = s.context.clone();
        busy.set(true);
        spawn_local(async move {
            let _ = invoke::<_, ()>("set_unlocked", &UnlockArgs { context, unlocked: true }).await;
            busy.set(false);
            session.update(|s| {
                if let Some(Ok(s)) = s {
                    s.unlocked = true;
                }
            });
            // Closed meanwhile (the window's close prompt): don't reopen as Save.
            if !matches!(modal.get_untracked(), Some(Modal::Unlock)) {
                return;
            }
            typed.set(String::new());
            let text = ace.with_value(|a| a.as_ref().map(Ace::text)).unwrap_or_default();
            modal.set(Some(logic::confirm(&original.get_untracked(), &text)));
        });
    };

    // Step 3: save for real, then close.
    let commit = move || {
        let Some(text) = ace.with_value(|a| a.as_ref().map(Ace::text)) else { return };
        busy.set(true);
        spawn_local(async move {
            match invoke::<_, String>("editor_save", &SaveArgs { yaml: &text, dry_run: false }).await {
                Ok(saved) => {
                    original.set(saved);
                    report_dirty(false);
                    modal.set(None);
                    close();
                }
                Err(e) => {
                    busy.set(false);
                    modal.set(None);
                    message.set(Some((true, e)));
                }
            }
        });
    };

    let cancel = move || match logic::leave(dirty.get_untracked(), After::Close) {
        Some(ask) => modal.set(Some(ask)),
        None => close(),
    };
    let reload = move || match logic::leave(dirty.get_untracked(), After::Reload) {
        Some(ask) => modal.set(Some(ask)),
        None => start(),
    };

    // Build the editor once its element exists, then load the YAML.
    Effect::new(move |done: Option<bool>| {
        if done == Some(true) {
            return true;
        }
        let Some(el) = host.get() else { return false };
        let el: web_sys::HtmlElement = el.into();
        spawn_local(async move {
            match Ace::new(el).await {
                Err(e) => session.set(Some(Err(e))),
                Ok(a) => {
                    let session_js = a.session();
                    a.on(&session_js, "change", move || {
                        let text = ace.with_value(|a| a.as_ref().map(Ace::text)).unwrap_or_default();
                        report_dirty(text != original.get_untracked());
                    });
                    a.on(&session_js, "changeAnnotation", move || {
                        errors.set(ace.with_value(|a| a.as_ref().map(Ace::errors)).unwrap_or_default());
                    });
                    a.on_save(save);
                    ace.set_value(Some(a));
                    start();
                }
            }
        });
        true
    });

    // Closing the window with unsaved edits: the backend holds it and asks here.
    spawn_local(async move {
        listen::<()>(edit_events::CLOSE_REQUESTED, move |()| match logic::leave(dirty.get_untracked(), After::Close) {
            Some(ask) => modal.set(Some(ask)),
            None => close(),
        })
        .await;
    });
    // Follow the Appearance setting when it changes in another window.
    let _theme = window_event_listener_untyped("storage", move |_| {
        ace.with_value(|a| a.as_ref().map(Ace::set_theme));
    });

    let heading = move || logic::heading(session.get());
    let status = move || {
        let e = errors.get();
        if let Some((true, m)) | Some((false, m)) = message.get() {
            let error = matches!(message.get(), Some((true, _)));
            return view! { <span class:warn-text=error class:muted=!error>{m}</span> }.into_any();
        }
        match logic::first_error(&e) {
            Some((row, text)) => view! {
                <button class="link warn-text" on:click=move |_| { ace.with_value(|a| a.as_ref().map(|a| a.go_to(row))); }>
                    {text}
                </button>
            }
            .into_any(),
            None => view! {
                <span class="muted">{move || logic::dirty_text(dirty.get())}</span>
            }
            .into_any(),
        }
    };

    view! {
        <div class="editor-app">
            <header class="editor-header">
                <div class="editor-title">
                    <span class="logs-heading" title=heading>{heading}</span>
                    {move || session.get().and_then(Result::ok).map(|s| view! {
                        <span class="chip icon-chip" class:prod-chip=s.prod_like>{s.context.clone()}{s.prod_like.then(|| view! { <crate::icons::ProdIcon /> })}</span>
                        {(!s.unlocked).then(|| view! { <span class="chip icon-chip"><crate::icons::LockIcon />"read-only"</span> })}
                    })}
                </div>
                <div class="editor-actions">
                    <button class="btn small" title="Load the current version from the cluster" on:click=move |_| reload()>"Reload"</button>
                    <button class="btn small" on:click=move |_| cancel()>"Cancel"</button>
                    <button
                        class="btn small primary"
                        title="Save (⌘S)"
                        prop:disabled=move || busy.get() || session.with(|s| !matches!(s, Some(Ok(_))))
                        on:click=move |_| save()
                    >{move || if busy.get() { "Saving…" } else { "Save" }}</button>
                </div>
            </header>
            {move || match session.get() {
                Some(Err(e)) => Some(view! { <p class="warn pad">{e}</p> }),
                _ => None,
            }}
            <div class="editor-host" node_ref=host></div>
            <footer class="editor-status">{status}</footer>
            {move || modal.get().map(|m| view! { <EditorModal m=m session=session typed=typed busy=busy modal=modal on_unlock=Callback::new(move |()| unlock()) on_commit=Callback::new(move |()| commit()) on_discard=Callback::new(move |after: After| {
                modal.set(None);
                match after {
                    After::Close => {
                        report_dirty(false);
                        spawn_local(async move {
                            let _ = invoke::<_, ()>("editor_close", &()).await;
                        });
                    }
                    After::Reload => start(),
                }
            }) /> })}
        </div>
    }
}

#[component]
fn EditorModal(
    m: Modal,
    session: RwSignal<Option<Result<EditSession, String>>>,
    typed: RwSignal<String>,
    busy: RwSignal<bool>,
    modal: RwSignal<Option<Modal>>,
    on_unlock: Callback<()>,
    on_commit: Callback<()>,
    on_discard: Callback<After>,
) -> impl IntoView {
    let Some(Ok(s)) = session.get_untracked() else { return ().into_any() };
    let close = move || {
        typed.set(String::new());
        modal.set(None);
    };
    let prod = s.prod_like;
    let what = view! {
        <strong class="mono">{s.target.name.clone()}</strong>
        {s.target.namespace.clone().map(|ns| view! { " in namespace "<strong>{ns}</strong> })}
        " on context "<strong>{s.context.clone()}</strong>
    };
    match m {
        Modal::Unlock => {
            let context = s.context.clone();
            let ok = move || logic::can_go(busy.get(), prod, &typed.get(), &context);
            view! {
                <crate::modal::Modal on_cancel=move |()| close() busy=busy prod=prod>
                        <h2>"Allow changes?"</h2>
                        <p>"Context "<strong>{s.context.clone()}</strong>" is read-only. Saving needs changes allowed on it, as in the main window."</p>
                        {prod.then(|| view! {
                            <p>"This context looks like "<strong>"PRODUCTION"</strong>". Type its name to allow changes:"</p>
                            <input class="confirm-input" prop:value=move || typed.get() on:input=move |ev| typed.set(event_target_value(&ev)) />
                        })}
                        <div class="modal-actions">
                            <button prop:disabled=move || busy.get() on:click=move |_| close()>"Back to editing"</button>
                            <button class="danger" prop:disabled=move || !ok() on:click=move |_| on_unlock.run(())>"Allow changes"</button>
                        </div>
                </crate::modal::Modal>
            }
            .into_any()
        }
        Modal::Confirm(lines) => {
            let (added, removed) = logic::counts(&lines);
            let ok = move || !busy.get();
            let rows = lines
                .into_iter()
                .map(|l| match l {
                    Line::Same(t) => view! { <div class="diff-line"><span class="diff-mark">" "</span>{t}</div> }.into_any(),
                    Line::Added(t) => view! { <div class="diff-line diff-add"><span class="diff-mark">"+"</span>{t}</div> }.into_any(),
                    Line::Removed(t) => view! { <div class="diff-line diff-del"><span class="diff-mark">"−"</span>{t}</div> }.into_any(),
                    Line::Skipped(n) => view! { <div class="diff-skip">{logic::skipped_text(n)}</div> }.into_any(),
                })
                .collect_view();
            view! {
                <crate::modal::Modal on_cancel=move |()| close() busy=busy prod=prod wide=true>
                        <h2>{logic::save_heading(&s.target.kind.kind)}</h2>
                        <p>"Save "{what}"? The server has checked it."</p>
                        <p class="muted">{logic::change_summary(added, removed)}</p>
                        <div class="diff">{rows}</div>
                        {prod.then(|| view! {
                            <p>"This context looks like "<strong>"PRODUCTION"</strong>"."</p>
                        })}
                        <div class="modal-actions">
                            <button prop:disabled=move || busy.get() on:click=move |_| close()>"Back to editing"</button>
                            <button class="primary" prop:disabled=move || !ok() on:click=move |_| on_commit.run(())>
                                {move || if busy.get() { "Saving…" } else { "Save" }}
                            </button>
                        </div>
                </crate::modal::Modal>
            }
            .into_any()
        }
        Modal::Discard(after) => {
            let (title, action) = logic::discard_texts(after);
            view! {
                <crate::modal::Modal on_cancel=move |()| close()>
                        <h2>{title}</h2>
                        <p>"Your changes to "<strong class="mono">{s.target.name.clone()}</strong>" haven't been saved. Nothing is saved unless you click "<strong>"Save"</strong></p>
                        <div class="modal-actions">
                            <button class="danger" on:click=move |_| on_discard.run(after)>{action}</button>
                            <button class="primary" autofocus on:click=move |_| close()>"Keep editing"</button>
                        </div>
                </crate::modal::Modal>
            }
            .into_any()
        }
    }
}

