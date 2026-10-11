//! The YAML edit window (opened with `?edit=<label>`): an Ace editor with YAML
//! highlighting, live syntax errors, auto-indent, folding, search and
//! Kubernetes key completion. Save validates with the server first, shows what
//! changed, then saves (gated like every other change); Cancel and closing ask
//! before throwing edits away. Also the main window's Edit button.

use js_sys::{Array, Function, Promise};
use leptos::html::Div;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{EditSession, EditTarget, ResourceRow, edit_events};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::app::Store;
use crate::exec::{call, get, object};
use crate::tauri::{invoke, listen};

// ---- Opening from the main window ----

/// "Edit" above the YAML tab: shown when the API allows updating the kind,
/// and never for Helm releases (their YAML is a summary, not an object).
#[component]
pub fn EditButton(store: Store, row: ResourceRow) -> impl IntoView {
    move || {
        let kind = store.kind.get().filter(|k| !(k.group == "helm.sh" && k.kind == "Release"))?;
        let allowed = store.cluster.with(|c| {
            c.iter().flat_map(|c| &c.kinds).any(|k| k.kind == kind && k.verbs.iter().any(|v| v == "update"))
        });
        allowed.then(|| {
            let target = EditTarget { kind: kind.clone(), namespace: row.namespace.clone(), name: row.name.clone() };
            view! {
                <button class="btn small" title="Edit this YAML in a new window" on:click=move |_| open(store, target.clone())>
                    "Edit"
                </button>
            }
        })
    }
}

fn open(store: Store, target: EditTarget) {
    #[derive(Serialize)]
    struct Args {
        target: EditTarget,
    }
    spawn_local(async move {
        if let Err(e) = invoke::<_, ()>("open_editor", &Args { target }).await {
            store.error.set(Some(e));
        }
    });
}

/// The window label from `?edit=<label>`, if this page is an edit window.
pub fn label_from_url() -> Option<String> {
    let search = window().location().search().ok()?;
    label_in(&search)
}

fn label_in(search: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|kv| kv.strip_prefix("edit="))
        // Only window labels (`edit-3`): other pages may use `edit` for something else.
        .filter(|l| l.strip_prefix("edit-").is_some_and(|id| !id.is_empty()))
        .map(str::to_owned)
}

// ---- Diff ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Line {
    Same(String),
    Added(String),
    Removed(String),
    /// A run of unchanged lines left out of the summary.
    Skipped(usize),
}

/// Line diff of `old` → `new`: common prefix and suffix are trimmed, the rest
/// compared with an LCS (or shown as replaced when it's too big to compare).
pub(crate) fn line_diff(old: &str, new: &str) -> Vec<Line> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (ma, mb) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let mut out: Vec<Line> = a[..prefix].iter().map(|l| Line::Same((*l).to_owned())).collect();
    if ma.len() * mb.len() > 4_000_000 {
        out.extend(ma.iter().map(|l| Line::Removed((*l).to_owned())));
        out.extend(mb.iter().map(|l| Line::Added((*l).to_owned())));
    } else {
        // lcs[i][j]: longest common subsequence of ma[i..] and mb[j..].
        let mut lcs = vec![vec![0u32; mb.len() + 1]; ma.len() + 1];
        for i in (0..ma.len()).rev() {
            for j in (0..mb.len()).rev() {
                lcs[i][j] = if ma[i] == mb[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < ma.len() || j < mb.len() {
            if i < ma.len() && j < mb.len() && ma[i] == mb[j] {
                out.push(Line::Same(ma[i].to_owned()));
                i += 1;
                j += 1;
            } else if i < ma.len() && (j == mb.len() || lcs[i + 1][j] >= lcs[i][j + 1]) {
                // Removals before additions, as diffs usually read.
                out.push(Line::Removed(ma[i].to_owned()));
                i += 1;
            } else {
                out.push(Line::Added(mb[j].to_owned()));
                j += 1;
            }
        }
    }
    out.extend(a[a.len() - suffix..].iter().map(|l| Line::Same((*l).to_owned())));
    out
}

/// Keeps `context` unchanged lines around each change and folds the rest.
pub(crate) fn summarize(lines: Vec<Line>, context: usize) -> Vec<Line> {
    let changed: Vec<usize> =
        lines.iter().enumerate().filter(|(_, l)| !matches!(l, Line::Same(_))).map(|(i, _)| i).collect();
    let near = |i: usize| changed.iter().any(|c| c.abs_diff(i) <= context);
    let mut out = Vec::new();
    let mut skipped = 0;
    for (i, line) in lines.into_iter().enumerate() {
        if matches!(line, Line::Same(_)) && !near(i) {
            skipped += 1;
            continue;
        }
        if skipped > 0 {
            out.push(Line::Skipped(skipped));
            skipped = 0;
        }
        out.push(line);
    }
    if skipped > 0 {
        out.push(Line::Skipped(skipped));
    }
    out
}

/// (added, removed) line counts.
pub(crate) fn counts(lines: &[Line]) -> (usize, usize) {
    lines.iter().fold((0, 0), |(a, r), l| match l {
        Line::Added(_) => (a + 1, r),
        Line::Removed(_) => (a, r + 1),
        _ => (a, r),
    })
}

/// "1 line added, 2 removed."
pub(crate) fn change_summary(added: usize, removed: usize) -> String {
    let lines = |n: usize| if n == 1 { "line" } else { "lines" };
    match (added, removed) {
        (a, 0) => format!("{a} {} added.", lines(a)),
        (0, r) => format!("{r} {} removed.", lines(r)),
        (a, r) => format!("{a} {} added, {r} removed.", lines(a)),
    }
}

/// Keys offered by completion, besides the words already in the document.
pub(crate) const KUBERNETES_KEYS: &[&str] = &[
    "apiVersion", "kind", "metadata", "name", "namespace", "labels", "annotations", "spec", "status", "replicas",
    "selector", "matchLabels", "matchExpressions", "template", "containers", "initContainers", "image",
    "imagePullPolicy", "command", "args", "ports", "containerPort", "protocol", "env", "envFrom", "value",
    "valueFrom", "configMapKeyRef", "secretKeyRef", "configMapRef", "secretRef", "resources", "requests", "limits",
    "cpu", "memory", "volumeMounts", "mountPath", "subPath", "readOnly", "volumes", "configMap", "secret",
    "secretName", "persistentVolumeClaim", "claimName", "emptyDir", "livenessProbe", "readinessProbe",
    "startupProbe", "httpGet", "path", "port", "tcpSocket", "exec", "initialDelaySeconds", "periodSeconds",
    "timeoutSeconds", "failureThreshold", "serviceAccountName", "nodeSelector", "tolerations", "affinity",
    "securityContext", "runAsNonRoot", "runAsUser", "fsGroup", "restartPolicy", "strategy", "rollingUpdate",
    "maxSurge", "maxUnavailable", "type", "targetPort", "data", "stringData", "rules", "host", "http", "paths",
    "pathType", "backend", "service", "number", "tls", "hosts", "schedule", "suspend", "jobTemplate",
    "concurrencyPolicy", "backoffLimit", "accessModes", "storageClassName", "storage", "minReplicas",
    "maxReplicas", "metrics", "target", "averageUtilization",
];

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
    if crate::prefs::is_dark() { "ace/theme/cloud_editor_dark" } else { "ace/theme/cloud_editor" }
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

#[derive(Debug, Clone, PartialEq)]
enum Modal {
    /// The context is read-only: allow changes first.
    Unlock,
    /// What will change, and the final go-ahead.
    Confirm(Vec<Line>),
    /// Throw the edits away (Cancel, Reload or closing the window).
    Discard(After),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    Close,
    Reload,
}

#[derive(Serialize)]
struct SaveArgs<'a> {
    yaml: &'a str,
    dry_run: bool,
}

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
            #[derive(Serialize)]
            struct Args {
                dirty: bool,
            }
            spawn_local(async move {
                let _ = invoke::<_, ()>("editor_dirty", &Args { dirty: now }).await;
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
        if busy.get_untracked() || modal.get_untracked().is_some() {
            return;
        }
        let Some(text) = ace.with_value(|a| a.as_ref().map(Ace::text)) else { return };
        if let Some((row, text)) = errors.get_untracked().first().cloned() {
            message.set(Some((true, format!("Fix the YAML first: line {}: {text}", row + 1))));
            ace.with_value(|a| a.as_ref().map(|a| a.go_to(row)));
            return;
        }
        if text == original.get_untracked() {
            message.set(Some((false, "No changes to save.".into())));
            return;
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
                    let unlocked = session.with_untracked(|s| matches!(s, Some(Ok(s)) if s.unlocked));
                    modal.set(Some(if unlocked {
                        Modal::Confirm(summarize(line_diff(&original.get_untracked(), &text), 3))
                    } else {
                        Modal::Unlock
                    }));
                }
            }
        });
    };

    // Step 2 for a read-only context: allow changes, as in the main window.
    let unlock = move || {
        let Some(Ok(s)) = session.get_untracked() else { return };
        #[derive(Serialize)]
        struct Args {
            context: String,
            unlocked: bool,
        }
        let context = s.context.clone();
        busy.set(true);
        spawn_local(async move {
            let _ = invoke::<_, ()>("set_unlocked", &Args { context, unlocked: true }).await;
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
            modal.set(Some(Modal::Confirm(summarize(line_diff(&original.get_untracked(), &text), 3))));
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

    let cancel = move || {
        if dirty.get_untracked() {
            modal.set(Some(Modal::Discard(After::Close)));
        } else {
            close();
        }
    };
    let reload = move || {
        if dirty.get_untracked() {
            modal.set(Some(Modal::Discard(After::Reload)));
        } else {
            start();
        }
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
        listen::<()>(edit_events::CLOSE_REQUESTED, move |()| {
            if dirty.get_untracked() {
                modal.set(Some(Modal::Discard(After::Close)));
            } else {
                close();
            }
        })
        .await;
    });
    // Follow the Appearance setting when it changes in another window.
    let _theme = window_event_listener_untyped("storage", move |_| {
        ace.with_value(|a| a.as_ref().map(Ace::set_theme));
    });

    let heading = move || match session.get() {
        None => "Loading…".to_owned(),
        Some(Ok(s)) => s.title,
        Some(Err(_)) => "Edit".to_owned(),
    };
    let status = move || {
        let e = errors.get();
        if let Some((true, m)) | Some((false, m)) = message.get() {
            let error = matches!(message.get(), Some((true, _)));
            return view! { <span class:warn-text=error class:muted=!error>{m}</span> }.into_any();
        }
        match e.first() {
            Some((row, text)) => {
                let row = *row;
                let more = if e.len() > 1 { format!(" (+{} more)", e.len() - 1) } else { String::new() };
                view! {
                    <button class="link warn-text" on:click=move |_| { ace.with_value(|a| a.as_ref().map(|a| a.go_to(row))); }>
                        {format!("Line {}: {text}{more}", row + 1)}
                    </button>
                }
                .into_any()
            }
            None => view! {
                <span class="muted">{move || if dirty.get() { "Edited · not saved" } else { "No changes" }}</span>
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
                        <span class="chip" class:prod-chip=s.prod_like>{if s.prod_like { format!("{} ⚠", s.context) } else { s.context.clone() }}</span>
                        {(!s.unlocked).then(|| view! { <span class="chip">"🔒 read-only"</span> })}
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
            let ok = move || !busy.get() && (!prod || typed.get().trim() == context);
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
            let (added, removed) = counts(&lines);
            let name = s.target.name.clone();
            let ok = move || !busy.get() && (!prod || typed.get().trim() == name);
            let rows = lines
                .into_iter()
                .map(|l| match l {
                    Line::Same(t) => view! { <div class="diff-line"><span class="diff-mark">" "</span>{t}</div> }.into_any(),
                    Line::Added(t) => view! { <div class="diff-line diff-add"><span class="diff-mark">"+"</span>{t}</div> }.into_any(),
                    Line::Removed(t) => view! { <div class="diff-line diff-del"><span class="diff-mark">"−"</span>{t}</div> }.into_any(),
                    Line::Skipped(n) => view! { <div class="diff-skip">{format!("… {n} unchanged lines")}</div> }.into_any(),
                })
                .collect_view();
            view! {
                <crate::modal::Modal on_cancel=move |()| close() busy=busy prod=prod wide=true>
                        <h2>{format!("Save {}?", s.target.kind.kind)}</h2>
                        <p>"Save "{what}"? The server has checked it."</p>
                        <p class="muted">{change_summary(added, removed)}</p>
                        <div class="diff">{rows}</div>
                        {prod.then(|| view! {
                            <p>"This context looks like "<strong>"PRODUCTION"</strong>"."</p>
                            <p>"Type the name to confirm:"</p>
                            <input class="confirm-input" prop:value=move || typed.get() on:input=move |ev| typed.set(event_target_value(&ev)) />
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
            let (title, action) = match after {
                After::Close => ("Discard your edits?", "Discard and close"),
                After::Reload => ("Reload and discard your edits?", "Discard and reload"),
            };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn d(old: &str, new: &str) -> Vec<Line> {
        line_diff(old, new)
    }

    #[test]
    fn diffs_lines() {
        use Line::*;
        assert_eq!(d("a\nb\nc", "a\nb\nc"), [Same("a".into()), Same("b".into()), Same("c".into())]);
        assert_eq!(d("a\nb\nc", "a\nx\nc"), [Same("a".into()), Removed("b".into()), Added("x".into()), Same("c".into())]);
        assert_eq!(d("a\nc", "a\nb\nc"), [Same("a".into()), Added("b".into()), Same("c".into())]);
        assert_eq!(d("a\nb\nc", "a\nc"), [Same("a".into()), Removed("b".into()), Same("c".into())]);
        assert_eq!(d("", "a"), [Added("a".into())]);
        assert_eq!(counts(&d("a\nb\nc\nd", "a\nB\nc\nd\ne")), (2, 1));
    }

    #[test]
    fn summaries_fold_unchanged_runs() {
        let old: String = (0..20).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l10\n", "L10\n");
        let s = summarize(line_diff(&old, &new), 2);
        assert_eq!(s.first(), Some(&Line::Skipped(8)));
        assert_eq!(s.last(), Some(&Line::Skipped(7)));
        assert_eq!(counts(&s), (1, 1));
        assert_eq!(change_summary(1, 1), "1 line added, 1 removed.");
        assert_eq!(change_summary(2, 0), "2 lines added.");
        assert_eq!(change_summary(0, 1), "1 line removed.");
        assert!(s.contains(&Line::Same("l8".into())) && !s.contains(&Line::Same("l7".into())));
        assert_eq!(summarize(line_diff("a", "a"), 3), [Line::Skipped(1)]);
    }

    #[test]
    fn edit_labels_and_keys() {
        assert_eq!(label_in("?edit=edit-3"), Some("edit-3".into()));
        assert_eq!(label_in("?a=1&edit=edit-9"), Some("edit-9".into()));
        assert_eq!(label_in("?logs=x"), None);
        assert_eq!(label_in("?edit="), None);
        assert_eq!(label_in("?kind=cm&edit=1"), None);
        assert_eq!(label_in("?edit=edit-"), None);
        let mut keys = KUBERNETES_KEYS.to_vec();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), KUBERNETES_KEYS.len(), "no duplicate completion keys");
    }

}
