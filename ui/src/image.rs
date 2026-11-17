//! A container's image metadata in the Details tab: platform, digests,
//! entrypoint, user, labels, built-in environment and build steps, read from
//! the image's registry the first time the section opens.

use leptos::prelude::*;
use leptos::task::spawn_local;
use subduer_types::ImageMetadata;

use crate::app::{Store, format_age};
use crate::logic::image::{self as logic, ImageArgs};
use crate::logic::pod_detail::{ago_at, section_open};
use crate::pod_detail::Section;
use crate::tauri::invoke;

/// Closed until opened; opening it reads the image once for the selection.
#[component]
pub(crate) fn ImageSection(store: Store, args: ImageArgs) -> impl IntoView {
    let id = format!("image:{}", args.container);
    {
        let (id, args) = (id.clone(), args.clone());
        Effect::new(move |_| {
            let open = store.detail_collapsed.with(|c| section_open(c, &id, true));
            if store.images.with(|m| logic::needs_load(open, m.get(&args))) {
                load(store, args.clone());
            }
        });
    }
    let state = {
        let args = args.clone();
        move || store.images.with(|m| m.get(&args).cloned())
    };
    let summary = Signal::derive({
        let state = state.clone();
        move || logic::summary(state().as_ref())
    });
    let container = args.container.clone();
    view! {
        <Section store=store id=id title="Image metadata".to_owned() summary=summary nested=true closed=true>
            {
                let (state, args, container) = (state.clone(), args.clone(), container.clone());
                move || match state() {
                    None | Some(None) => view! { <p class="muted">"Reading from the registry…"</p> }.into_any(),
                    Some(Some(Err(e))) => {
                        let args = args.clone();
                        view! {
                            <p class="warn-text image-error">{e}</p>
                            <button
                                class="btn small"
                                on:click=move |_| store.images.update(|m| { m.remove(&args); })
                            >"Try again"</button>
                        }
                        .into_any()
                    }
                    Some(Some(Ok(m))) => view! { <ImageDetails store=store m=m container=container.clone() /> }.into_any(),
                }
            }
        </Section>
    }
}

fn load(store: Store, args: ImageArgs) {
    store.images.update(|m| {
        m.insert(args.clone(), None);
    });
    spawn_local(async move {
        let result = invoke::<_, ImageMetadata>("image_metadata", &args).await;
        // Unless the selection changed meanwhile.
        store.images.update(|m| {
            if let Some(slot) = m.get_mut(&args) {
                *slot = Some(result);
            }
        });
    });
}

#[component]
fn ImageDetails(store: Store, m: ImageMetadata, container: String) -> impl IntoView {
    let created = m.created.clone().map(|at| {
        let shown = at.clone();
        view! { <dt>"Created"</dt><dd>{move || ago_at(&format_age(store.now.get(), &at), &shown)}</dd> }
    });
    let rows = logic::rows(&m)
        .into_iter()
        .map(|(k, v)| view! { <dt>{k}</dt><dd class="mono">{v}</dd> })
        .collect_view();
    let pairs = |list: Vec<(String, String)>| {
        let rows = list
            .into_iter()
            .map(|(k, v)| view! { <tr><td class="env-name">{k}</td><td class="env-value mono">{v}</td></tr> })
            .collect_view();
        view! { <table class="env"><tbody>{rows}</tbody></table> }
    };
    let labels = (!m.labels.is_empty()).then(|| {
        let table = pairs(m.labels.clone());
        let summary = logic::labels_summary(&m);
        view! {
            <Section store=store id=format!("image-labels:{container}") title="Labels".to_owned() summary=summary nested=true>
                {table.clone()}
            </Section>
        }
    });
    let env = (!m.env.is_empty()).then(|| {
        let table = pairs(m.env.iter().map(|e| logic::env_pair(e)).collect());
        let summary = logic::env_summary(&m);
        view! {
            <Section
                store=store
                id=format!("image-env:{container}")
                title="Environment in the image".to_owned()
                summary=summary
                nested=true
                closed=true
            >
                {table.clone()}
            </Section>
        }
    });
    let steps = logic::history_rows(&m)
        .into_iter()
        .map(|(size, step, comment)| {
            view! {
                <tr>
                    <td class="layer-size">{size}</td>
                    <td class="mono">{step}{comment.map(|c| view! { <span class="env-ref">{c}</span> })}</td>
                </tr>
            }
        })
        .collect_view();
    let history_summary = logic::history_summary(&m);
    view! {
        <dl class="kv">
            {created}
            {rows}
        </dl>
        {labels}
        {env}
        <Section
            store=store
            id=format!("image-history:{container}")
            title="Build steps".to_owned()
            summary=history_summary
            nested=true
            closed=true
        >
            <table class="env image-history"><tbody>{steps.clone()}</tbody></table>
        </Section>
    }
}
