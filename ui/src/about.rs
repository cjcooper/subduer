//! The About window (opened with `?about`): the app, its own license, and the
//! license of everything it includes, one collapsible section per license
//! with the software it covers.

use leptos::prelude::*;
use leptos::task::spawn_local;
use subduer_types::AboutInfo;

use crate::logic::about::{self as logic, Notice};
use crate::tauri::{NoArgs, invoke};

/// Whether this page is the About window.
pub fn from_url() -> bool {
    window().location().search().is_ok_and(|s| logic::is_about(&s))
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
    let (_, notices) = logic::parse_notices(&info.notices);
    let (crates, vendored) = logic::totals(&notices);
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
                {tab_button(Tab::ThirdParty, logic::third_party_tab(crates, vendored))}
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
    let description = logic::description_text(&info.description);
    let built_with = BUILT_WITH.iter().enumerate().map(|(i, (name, url))| {
        let sep = logic::list_separator(i, BUILT_WITH.len());
        view! { {sep}<Link href=url.to_string() error=error>{*name}</Link> }
    });
    view! {
        <div class="about-main">
            // The CSS shows the one that matches the appearance.
            <img class="about-icon light" src="128x128@2x.png" alt="" />
            <img class="about-icon dark" src="dark-256x256.png" alt="" />
            <h1>{info.name.clone()}</h1>
            <p class="muted">{logic::version_text(&info.version)}</p>
            <p>{description}</p>
            <p class="muted">{logic::maker_text(&info.maker, &info.copyright)}</p>
            <p>
                {(!info.repository.is_empty()).then(|| view! {
                    <Link href=info.repository.clone() error=error>"Source code"</Link>
                    " · "
                })}
                <a href="#" on:click=move |ev| { ev.prevent_default(); on_licenses(); }>"Third-party licenses"</a>
            </p>
            <section class="credits">
                <h3>"Built on"</h3>
                <p>"Built with " {built_with.collect_view()} {logic::built_with_tail(crates)}</p>
            </section>
        </div>
    }
}

#[component]
fn ThirdParty(name: String, notices: Vec<Notice>, crates: usize, vendored: usize) -> impl IntoView {
    let chips = logic::chips(&notices);
    let filter = RwSignal::new(String::new());
    let license = RwSignal::new(None::<String>);
    let notices = StoredValue::new(notices);
    let shown = move || {
        let f = filter.get();
        let l = license.get();
        notices.with_value(|all| logic::filtered(all, &f, l.as_deref()))
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
                {logic::third_party_text(&name, crates, vendored)}
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
                        let summary = logic::summary(&n);
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
