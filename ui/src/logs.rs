//! The logs window (opened with `?logs=<label>`) and the helper the main
//! window uses to open one.

use std::collections::VecDeque;

use leptos::html::Div;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{LogBatch, LogLine, LogSession, LogTarget};

use crate::tauri::{invoke, listen};

/// Lines kept per window; older ones are dropped.
const MAX_LINES: usize = 10_000;
const MAX_NOTICES: usize = 20;

/// Asks the backend to open (or focus) a logs window for `target`.
pub fn open(target: LogTarget, on_error: impl Fn(String) + 'static) {
    #[derive(Serialize)]
    struct Args {
        target: LogTarget,
    }
    spawn_local(async move {
        if let Err(e) = invoke::<_, ()>("open_logs", &Args { target }).await {
            on_error(e);
        }
    });
}

/// The window label from `?logs=<label>`, if this page is a logs window.
pub fn label_from_url() -> Option<String> {
    let search = window().location().search().ok()?;
    label_in(&search)
}

/// The `logs` parameter of a URL's query string (`?a=b&logs=x`).
fn label_in(search: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|kv| kv.strip_prefix("logs="))
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
}

#[derive(Debug, Clone, PartialEq)]
struct Row {
    id: u64,
    line: LogLine,
}

#[component]
pub fn LogsApp(label: String) -> impl IntoView {
    let session = RwSignal::new(None::<Result<LogSession, String>>);
    let lines = RwSignal::new(VecDeque::<Row>::new());
    let next_id = StoredValue::new(0u64);
    let notices = RwSignal::new(Vec::<String>::new());
    let filter = RwSignal::new(String::new());
    let follow = RwSignal::new(true);
    let wrap = RwSignal::new(false);
    let show_time = RwSignal::new(true);
    let previous = RwSignal::new(false);
    let scroller = NodeRef::<Div>::new();

    let start = move || {
        lines.set(VecDeque::new());
        notices.set(vec![]);
        follow.set(true);
        spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                previous: bool,
            }
            let result = invoke::<_, LogSession>("logs_start", &Args { previous: previous.get_untracked() }).await;
            session.set(Some(result));
        });
    };

    // Listen before starting so the first batch isn't missed.
    spawn_local(async move {
        listen::<LogBatch>(&label, move |batch| {
            if !batch.lines.is_empty() {
                lines.update(|l| {
                    for line in batch.lines {
                        let id = next_id.get_value();
                        next_id.set_value(id + 1);
                        l.push_back(Row { id, line });
                    }
                    while l.len() > MAX_LINES {
                        l.pop_front();
                    }
                });
            }
            if !batch.notices.is_empty() {
                notices.update(|n| {
                    n.extend(batch.notices);
                    let excess = n.len().saturating_sub(MAX_NOTICES);
                    n.drain(..excess);
                });
            }
        })
        .await;
        start();
    });

    let visible = Memo::new(move |_| {
        let needle = filter.get().to_lowercase();
        lines.with(|l| {
            l.iter()
                .filter(|r| {
                    needle.is_empty()
                        || r.line.text.to_lowercase().contains(&needle)
                        || r.line.source.as_ref().is_some_and(|s| s.to_lowercase().contains(&needle))
                })
                .cloned()
                .collect::<Vec<_>>()
        })
    });

    // Keep the newest line in view while following.
    Effect::new(move |_| {
        visible.track();
        if follow.get_untracked() {
            request_animation_frame(move || {
                if let Some(el) = scroller.get_untracked() {
                    el.set_scroll_top(el.scroll_height());
                }
            });
        }
    });
    // Scrolling up pauses following; scrolling back to the bottom resumes it.
    let on_scroll = move |_| {
        if let Some(el) = scroller.get_untracked() {
            let at_bottom = el.scroll_height() - el.scroll_top() - el.client_height() < 40;
            if follow.get_untracked() != at_bottom {
                follow.set(at_bottom);
            }
        }
    };
    let jump_to_end = move |_| {
        follow.set(true);
        if let Some(el) = scroller.get_untracked() {
            el.set_scroll_top(el.scroll_height());
        }
    };

    let heading = move || match session.get() {
        None => "Connecting…".to_owned(),
        Some(Ok(s)) => s.title,
        Some(Err(e)) => e,
    };
    let count = move || {
        let (shown, total) = (visible.with(Vec::len), lines.with(VecDeque::len));
        if shown == total { format!("{total} lines") } else { format!("{shown} of {total} lines") }
    };
    let toggle = |signal: RwSignal<bool>, label: &'static str, title: &'static str| {
        view! {
            <button
                class="btn small toggle"
                class:on=move || signal.get()
                aria-pressed=move || signal.get().to_string()
                title=title
                on:click=move |_| signal.update(|v| *v = !*v)
            >{label}</button>
        }
    };

    view! {
        <div class="logs-app">
            <header class="logs-header">
                <div class="logs-title">
                    <span class="logs-heading" title=heading>{heading}</span>
                </div>
                <div class="logs-tools">
                    <input
                        class="search"
                        type="search"
                        placeholder="Filter lines…"
                        prop:value=move || filter.get()
                        on:input=move |ev| filter.set(event_target_value(&ev))
                    />
                    {toggle(wrap, "Wrap", "Wrap long lines")}
                    {toggle(show_time, "Time", "Show timestamps (local time)")}
                    <button
                        class="btn small toggle"
                        class:on=move || previous.get()
                        aria-pressed=move || previous.get().to_string()
                        title="Show the previous (terminated) container's logs, e.g. after a crash"
                        on:click=move |_| {
                            previous.update(|v| *v = !*v);
                            start();
                        }
                    >"Previous"</button>
                    <button class="btn small" title="Reconnect and reload" on:click=move |_| start()>"Reload"</button>
                    <button class="btn small" on:click=move |_| lines.set(VecDeque::new())>"Clear"</button>
                </div>
            </header>
            <div class="logs-body" class:wrap=move || wrap.get() node_ref=scroller on:scroll=on_scroll>
                <For each=move || visible.get() key=|r| r.id let:row>
                    <LogRow line=row.line show_time=show_time />
                </For>
            </div>
            <footer class="logs-footer">
                <span class="muted">{count}</span>
                <span class="logs-notice">{move || notices.with(|n| n.last().cloned().unwrap_or_default())}</span>
                <span class="spacer"></span>
                <Show when=move || !follow.get()>
                    <button class="btn small" on:click=jump_to_end>"↓ Follow"</button>
                </Show>
            </footer>
        </div>
    }
}

#[component]
fn LogRow(line: LogLine, show_time: RwSignal<bool>) -> impl IntoView {
    let (local, full) = line.time.as_deref().map(local_time).unwrap_or_default();
    view! {
        <div class="log-line">
            <Show when=move || show_time.get()>
                <span class="log-time" title=full.clone()>{local.clone()}</span>
            </Show>
            {line.source.map(|s| view! { <span class="log-source" title=s.clone()>{s.clone()}</span> })}
            <span class="log-text">{line.text}</span>
        </div>
    }
}

/// An RFC 3339 UTC time cut to milliseconds, which JS dates can parse.
fn millis(ts: &str) -> String {
    match ts.find('.') {
        Some(dot) => format!("{}Z", &ts[..(dot + 4).min(ts.len() - 1)]),
        None => ts.to_owned(),
    }
}

/// `HH:MM:SS.mmm` in local time, plus the original RFC 3339 for the tooltip.
fn local_time(ts: &str) -> (String, String) {
    // JS dates take milliseconds at most; the kubelet sends nanoseconds.
    let trimmed = millis(ts);
    let d = js_sys::Date::new(&trimmed.into());
    if d.get_time().is_nan() {
        return (ts.to_owned(), ts.to_owned());
    }
    let local = format!(
        "{:02}:{:02}:{:02}.{:03}",
        d.get_hours(),
        d.get_minutes(),
        d.get_seconds(),
        d.get_milliseconds()
    );
    (local, ts.to_owned())
}

#[cfg(test)]
#[path = "logs_tests.rs"]
mod tests_unit;
