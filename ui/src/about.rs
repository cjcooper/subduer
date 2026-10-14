//! The About window (opened with `?about`): the app, its own license, and the
//! license of everything it includes, one collapsible section per license
//! with the software it covers.

use leptos::prelude::*;
use leptos::task::spawn_local;
use subduer_types::AboutInfo;

use crate::tauri::{NoArgs, invoke};

/// Whether this page is the About window.
pub fn from_url() -> bool {
    window().location().search().is_ok_and(|s| is_about(&s))
}

fn is_about(search: &str) -> bool {
    search.trim_start_matches('?').split('&').any(|kv| kv == "about")
}

/// One license from `THIRD-PARTY-NOTICES.txt` and the software under it.
#[derive(Debug, Clone, PartialEq)]
struct Notice {
    title: String,
    /// `name version (repository)`, one per crate; empty for vendored code,
    /// whose title names it.
    used_by: Vec<String>,
    text: String,
}

const RULE: &str = "================================================================================";

/// Splits the generated notices into the intro (the counts) and the
/// licenses. See `about.hbs` and `tools/third-party-notices.sh`.
fn parse_notices(all: &str) -> (String, Vec<Notice>) {
    let separator = format!("{RULE}\n");
    let mut blocks = all.split(&separator);
    let intro = blocks.next().unwrap_or_default().trim().to_owned();
    let notices = blocks
        .filter_map(|block| {
            let (title, rest) = block.split_once('\n')?;
            let (used_by, text) = match rest.trim_start_matches('\n').strip_prefix("Used by:\n") {
                Some(rest) => {
                    let end = rest.find("\n\n").unwrap_or(rest.len());
                    let crates = rest[..end].lines().map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()).collect();
                    (crates, &rest[end..])
                }
                None => (vec![], rest),
            };
            Some(Notice { title: title.trim().to_owned(), used_by, text: text.trim_matches('\n').to_owned() })
        })
        .collect();
    (intro, notices)
}

/// Whether `notice` matches the filter (case-insensitive, on its title or a
/// crate name).
fn matches(notice: &Notice, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty()
        || notice.title.to_lowercase().contains(&filter)
        || notice.used_by.iter().any(|c| c.to_lowercase().contains(&filter))
}

/// A license's heading: its name and the first few crates under it (the same
/// license appears once per distinct copyright text).
fn summary(notice: &Notice) -> String {
    const SHOWN: usize = 3;
    let names: Vec<&str> = notice.used_by.iter().map(|c| c.split(' ').next().unwrap_or(c)).collect();
    match names.len() {
        0 => notice.title.clone(),
        n if n <= SHOWN => format!("{} · {}", notice.title, names.join(", ")),
        n => format!("{} · {} and {} more", notice.title, names[..SHOWN].join(", "), n - SHOWN),
    }
}

/// The per-license crate counts from the notices' intro.
fn counts(intro: &str) -> String {
    let lines: Vec<&str> = intro.lines().map(str::trim).filter(|l| l.starts_with(|c: char| c.is_ascii_digit())).collect();
    lines.join("\n")
}

#[component]
pub fn AboutApp() -> impl IntoView {
    let info = RwSignal::new(None::<Result<AboutInfo, String>>);
    spawn_local(async move { info.set(Some(invoke::<_, AboutInfo>("about_info", &NoArgs {}).await)) });
    view! {
        <div class="about-app">
            {move || match info.get() {
                None => view! { <p class="muted pad">"Loading…"</p> }.into_any(),
                Some(Err(e)) => view! { <div class="warn">{e}</div> }.into_any(),
                Some(Ok(info)) => view! { <About info=info /> }.into_any(),
            }}
        </div>
    }
}

#[component]
fn About(info: AboutInfo) -> impl IntoView {
    let (intro, notices) = parse_notices(&info.notices);
    let crates: usize = notices.iter().map(|n| n.used_by.len()).sum();
    // Vendored code has a section of its own and no crate list.
    let vendored = notices.iter().filter(|n| n.used_by.is_empty()).count();
    let filter = RwSignal::new(String::new());
    let notices = StoredValue::new(notices);
    let shown = move || {
        let f = filter.get();
        notices.with_value(|all| all.iter().filter(|n| matches(n, &f)).cloned().collect::<Vec<_>>())
    };
    view! {
        <header class="about-head">
            <img class="about-icon" src="128x128@2x.png" alt="" />
            <div>
                <h1>{info.name.clone()}</h1>
                <div class="muted">{format!("Version {}", info.version)}</div>
                <div class="muted">{format!("An {} application. {}", info.maker, info.copyright)}</div>
            </div>
        </header>
        <section class="about-section">
            <h2>"License"</h2>
            <p>{format!("{} is free software under the MIT License:", info.name)}</p>
            <pre class="about-text">{info.license.trim().to_owned()}</pre>
        </section>
        <section class="about-section">
            <h2>"Third-party software"</h2>
            <p>
                {format!(
                    "{} includes {crates} open-source Rust crates and {vendored} JavaScript libraries, used under the \
                     licenses below. Each section lists the software it covers and the full license text.",
                    info.name,
                )}
            </p>
            <pre class="about-counts muted">{counts(&intro)}</pre>
            <input
                class="search"
                type="search"
                placeholder="Filter by name or license"
                prop:value=move || filter.get()
                on:input=move |ev| filter.set(event_target_value(&ev))
            />
            {move || {
                let list = shown();
                if list.is_empty() {
                    return view! { <p class="muted">"Nothing matches."</p> }.into_any();
                }
                list.into_iter()
                    .map(|n| {
                        let summary = summary(&n);
                        view! {
                            <details class="about-notice">
                                <summary>{summary}</summary>
                                {(!n.used_by.is_empty()).then(|| view! {
                                    <ul class="about-crates mono">
                                        {n.used_by.iter().map(|c| view! { <li>{c.clone()}</li> }).collect_view()}
                                    </ul>
                                })}
                                <pre class="about-text">{n.text.clone()}</pre>
                            </details>
                        }
                    })
                    .collect_view()
                    .into_any()
            }}
        </section>
    }
}

#[cfg(test)]
#[path = "about_tests.rs"]
mod tests_unit;
