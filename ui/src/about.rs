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

/// What a license chip filters on: the license's name, or `JavaScript` for
/// vendored code.
fn kind(notice: &Notice) -> &str {
    if notice.used_by.is_empty() && notice.title.starts_with("Vendored JavaScript") { "JavaScript" } else { &notice.title }
}

/// The license chips: each license (or `JavaScript`) and how much software it
/// covers, most first. A license appears once per distinct copyright text in
/// the notices, so the counts add those up.
fn chips(notices: &[Notice]) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = vec![];
    for n in notices {
        let count = n.used_by.len().max(1);
        match out.iter_mut().find(|(k, _)| k == kind(n)) {
            Some((_, c)) => *c += count,
            None => out.push((kind(n).to_owned(), count)),
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    About,
    License,
    ThirdParty,
}

/// What the app is built on, for the About tab: name and home page.
const BUILT_WITH: [(&str, &str); 5] = [
    ("Tauri", "https://tauri.app"),
    ("Leptos", "https://leptos.dev"),
    ("kube-rs", "https://kube.rs"),
    ("Ace", "https://ace.c9.io"),
    ("xterm.js", "https://xtermjs.org"),
];

#[derive(serde::Serialize)]
struct UrlArgs {
    url: String,
}

/// A link that opens in the browser rather than in the window.
#[component]
fn Link(href: String, children: Children, error: RwSignal<Option<String>>) -> impl IntoView {
    let url = href.clone();
    let open = move |ev: leptos::ev::MouseEvent| {
        ev.prevent_default();
        let args = UrlArgs { url: url.clone() };
        spawn_local(async move {
            if let Err(e) = invoke::<_, ()>("open_url", &args).await {
                error.set(Some(e));
            }
        });
    };
    view! { <a href=href on:click=open>{children()}</a> }
}

#[component]
pub fn AboutApp() -> impl IntoView {
    let info = RwSignal::new(None::<Result<AboutInfo, String>>);
    spawn_local(async move { info.set(Some(invoke::<_, AboutInfo>("about_info", &NoArgs {}).await)) });
    // Esc closes the window, as it does a dialog.
    let _esc = window_event_listener(leptos::ev::keydown, |ev| {
        if crate::modal::is_plain_esc(&ev) {
            spawn_local(async { _ = invoke::<_, ()>("close_about", &NoArgs {}).await });
        }
    });
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
    let (_, notices) = parse_notices(&info.notices);
    let crates: usize = notices.iter().map(|n| n.used_by.len()).sum();
    // Vendored code has a section of its own and no crate list.
    let vendored = notices.iter().filter(|n| n.used_by.is_empty()).count();
    let tab = RwSignal::new(Tab::About);
    let error = RwSignal::new(None::<String>);
    let tab_button = move |t: Tab, label: String| {
        view! {
            <button class="tab" class:active=move || tab.get() == t on:click=move |_| tab.set(t)>{label}</button>
        }
    };
    let license = info.license.trim().to_owned();
    let notices = StoredValue::new(notices);
    let info = StoredValue::new(info);
    view! {
        <header class="about-head">
            <nav class="tabs">
                {tab_button(Tab::About, "About".into())}
                {tab_button(Tab::License, "License".into())}
                {tab_button(Tab::ThirdParty, format!("Third-party software ({})", crates + vendored))}
            </nav>
        </header>
        <div class="about-body">
            {move || error.get().map(|e| view! { <div class="warn">{e}</div> })}
            {move || match tab.get() {
                Tab::About => info.with_value(|info| view! {
                    <AboutTab info=info.clone() crates=crates error=error on_licenses=move || tab.set(Tab::ThirdParty) />
                }).into_any(),
                Tab::License => view! { <pre class="about-text license">{license.clone()}</pre> }.into_any(),
                Tab::ThirdParty => view! {
                    <ThirdParty name=info.with_value(|i| i.name.clone()) notices=notices.get_value() crates=crates vendored=vendored />
                }.into_any(),
            }}
        </div>
    }
}

#[component]
fn AboutTab(
    info: AboutInfo,
    crates: usize,
    error: RwSignal<Option<String>>,
    on_licenses: impl Fn() + Send + Sync + 'static,
) -> impl IntoView {
    let description = format!("{}.", info.description.trim_end_matches('.'));
    let built_with = BUILT_WITH.iter().enumerate().map(|(i, (name, url))| {
        let sep = match i {
            0 => "",
            i if i == BUILT_WITH.len() - 1 => " and ",
            _ => ", ",
        };
        view! { {sep}<Link href=url.to_string() error=error>{*name}</Link> }
    });
    view! {
        <div class="about-main">
            // The CSS shows the one that matches the appearance.
            <img class="about-icon light" src="128x128@2x.png" alt="" />
            <img class="about-icon dark" src="dark-256x256.png" alt="" />
            <h1>{info.name.clone()}</h1>
            <p class="muted">{format!("Version {}", info.version)}</p>
            <p>{description}</p>
            <p class="muted">{format!("An {} application. {}. Released under the MIT License.", info.maker, info.copyright)}</p>
            <p>
                {(!info.repository.is_empty()).then(|| view! {
                    <Link href=info.repository.clone() error=error>"Source code"</Link>
                    " · "
                })}
                <a href="#" on:click=move |ev| { ev.prevent_default(); on_licenses(); }>"Third-party licenses"</a>
            </p>
            <section class="credits">
                <h3>"Built on"</h3>
                <p>"Built with " {built_with.collect_view()} {format!(", and {crates} open-source Rust crates.")}</p>
            </section>
        </div>
    }
}

#[component]
fn ThirdParty(name: String, notices: Vec<Notice>, crates: usize, vendored: usize) -> impl IntoView {
    let chips = chips(&notices);
    let filter = RwSignal::new(String::new());
    let license = RwSignal::new(None::<String>);
    let notices = StoredValue::new(notices);
    let shown = move || {
        let f = filter.get();
        let l = license.get();
        notices.with_value(|all| {
            all.iter().filter(|n| matches(n, &f) && l.as_deref().is_none_or(|l| kind(n) == l)).cloned().collect::<Vec<_>>()
        })
    };
    let chip = move |key: Option<String>, label: String| {
        let on = key.clone();
        view! {
            <button
                class="chip-button"
                class:on=move || license.get() == on
                on:click=move |_| license.set(if license.get() == key { None } else { key.clone() })
            >{label}</button>
        }
    };
    view! {
        <div class="third-party">
            <p class="muted">
                {format!(
                    "{name} includes {crates} open-source Rust crates and {vendored} JavaScript libraries, used under \
                     the licenses below. Each section lists the software it covers and the full license text.",
                )}
            </p>
            <input
                class="search"
                type="search"
                placeholder="Filter by name or license"
                prop:value=move || filter.get()
                on:input=move |ev| filter.set(event_target_value(&ev))
            />
            <div class="chips about-chips">
                {chip(None, format!("All {}", crates + vendored))}
                {chips.into_iter().map(|(k, n)| chip(Some(k.clone()), format!("{k} {n}"))).collect_view()}
            </div>
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
        </div>
    }
}

#[cfg(test)]
#[path = "about_tests.rs"]
mod tests_unit;
