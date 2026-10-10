mod app;
mod chart;
mod columns;
mod delete;
mod overview;
mod pod_detail;
mod prefs;
mod tauri;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}
