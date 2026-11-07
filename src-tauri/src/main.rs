// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    subduer_lib::prepare();
    subduer_lib::start(tauri::Builder::default(), tauri::generate_context!());
}
