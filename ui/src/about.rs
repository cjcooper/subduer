//! The About window (opened with `?about`): the app, its own license, and the
//! license of everything it includes, one collapsible section per license
//! with the software it covers.

use leptos::prelude::*;
use leptos::task::spawn_local;
use subduer_types::AboutInfo;

use crate::logic::about as logic;
use crate::tauri::{NoArgs, invoke};

/// Whether this page is the About window.
pub fn from_url() -> bool {
    window().location().search().is_ok_and(|s| logic::is_about(&s))
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
    let (intro, notices) = logic::parse_notices(&info.notices);
    let (crates, vendored) = logic::totals(&notices);
    let filter = RwSignal::new(String::new());
    let notices = StoredValue::new(notices);
    let shown = move || {
        let f = filter.get();
        notices.with_value(|all| logic::filtered(all, &f))
    };
    view! {
        <header class="about-head">
            // The CSS shows the one that matches the appearance.
            <img class="about-icon light" src="128x128@2x.png" alt="" />
            <img class="about-icon dark" src="dark-256x256.png" alt="" />
            <div>
                <h1>{info.name.clone()}</h1>
                <div class="muted">{logic::version_text(&info.version)}</div>
                <div class="muted">{logic::maker_text(&info.maker, &info.copyright)}</div>
            </div>
        </header>
        <section class="about-section">
            <h2>"License"</h2>
            <p>{logic::license_intro(&info.name)}</p>
            <pre class="about-text">{info.license.trim().to_owned()}</pre>
        </section>
        <section class="about-section">
            <h2>"Third-party software"</h2>
            <p>{logic::third_party_text(&info.name, crates, vendored)}</p>
            <pre class="about-counts muted">{logic::counts(&intro)}</pre>
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
        </section>
    }
}
