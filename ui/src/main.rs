mod app;
mod browser;
mod chart;
mod columns;
mod delete;
mod download;
mod exec;
mod forward;
mod icons;
mod logs;
mod overview;
mod pod_detail;
mod prefs;
mod tauri;
mod upload;

fn main() {
    console_error_panic_hook::set_once();
    // The same page serves logs and shell windows: `?logs=<label>`, `?exec=<label>`.
    if let Some(label) = logs::label_from_url() {
        leptos::mount::mount_to_body(move || leptos::view! { <logs::LogsApp label=label.clone() /> });
    } else if let Some(label) = exec::label_from_url() {
        leptos::mount::mount_to_body(move || leptos::view! { <exec::ExecApp label=label.clone() /> });
    } else {
        leptos::mount::mount_to_body(app::App);
    }
}
