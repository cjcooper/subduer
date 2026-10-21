mod about;
mod app;
mod browser;
mod capture;
mod chart;
mod columns;
mod compare;
mod delete;
mod download;
mod editor;
mod exec;
mod find;
mod forward;
mod icons;
mod logs;
mod modal;
mod namespace;
mod overview;
mod pod_detail;
mod prefs;
mod rightsize;
mod sort;
mod tauri;
mod trace;
mod trace_chart;
mod trace_overlay;
mod trace_review;
mod upload;
mod workload;
mod object_detail;
mod workload_detail;

fn main() {
    console_error_panic_hook::set_once();
    // Every window (main, logs, shell, capture) follows the Appearance setting.
    prefs::follow_theme();
    // …and the system accent colour.
    prefs::follow_accent();
    // The same page serves logs, edit, shell, capture and About windows: `?logs=`, `?edit=`, `?exec=`, `?capture=<label>`, `?about`.
    if about::from_url() {
        leptos::mount::mount_to_body(about::AboutApp);
    } else if let Some(label) = capture::label_from_url() {
        leptos::mount::mount_to_body(move || leptos::view! { <capture::CaptureApp label=label.clone() /> });
    } else if let Some(label) = logs::label_from_url() {
        leptos::mount::mount_to_body(move || leptos::view! { <logs::LogsApp label=label.clone() /> });
    } else if let Some(label) = editor::label_from_url() {
        leptos::mount::mount_to_body(move || leptos::view! { <editor::EditorApp label=label.clone() /> });
    } else if let Some(label) = exec::label_from_url() {
        leptos::mount::mount_to_body(move || leptos::view! { <exec::ExecApp label=label.clone() /> });
    } else {
        leptos::mount::mount_to_body(app::App);
    }
}
