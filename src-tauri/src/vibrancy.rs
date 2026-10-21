//! Translucent main windows, where the system has a material to show through:
//! the sidebar material on macOS, Mica on Windows 11. Elsewhere (Linux,
//! Windows 10) windows stay opaque, as there's nothing to blur with.
//!
//! A window can only be made transparent when it's created, so main windows
//! are built here; Settings › Transparency then turns the material on or off.
//! The UI draws the sidebar and top bar clear over it (`data-vibrancy`).

use std::sync::atomic::Ordering;

use tauri::utils::config::{WindowConfig, WindowEffectsConfig};
use tauri::window::{Effect, EffectState};
use tauri::{AppHandle, Manager, Runtime, State, WebviewWindow, WebviewWindowBuilder};

use crate::AppState;

/// Whether this system has a material for windows.
pub fn supported() -> bool {
    cfg!(target_os = "macos") || windows_11()
}

/// Mica needs Windows 11: build 22000 on.
#[cfg(windows)]
fn windows_11() -> bool {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;

    let build = || -> Option<u32> {
        let version = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion").ok()?;
        version.get_value::<String, _>("CurrentBuildNumber").ok()?.trim().parse().ok()
    };
    build().is_some_and(|build| build >= 22000)
}

#[cfg(not(windows))]
fn windows_11() -> bool {
    false
}

/// Each system takes the first effect it knows: macOS the sidebar material
/// (dimmed in inactive windows, like Finder), Windows Mica.
fn effects() -> WindowEffectsConfig {
    WindowEffectsConfig {
        effects: vec![Effect::Sidebar, Effect::Mica],
        state: Some(EffectState::FollowsWindowActiveState),
        ..Default::default()
    }
}

/// Builds a main window from its config: transparent where supported, with
/// the material unless Settings turned it off.
pub fn build_main<R: Runtime>(app: &AppHandle<R>, config: &WindowConfig) -> tauri::Result<WebviewWindow<R>> {
    let mut builder = WebviewWindowBuilder::from_config(app, config)?;
    if supported() {
        builder = builder.transparent(true);
        if !app.state::<AppState>().opaque.load(Ordering::SeqCst) {
            builder = builder.effects(effects());
        }
    }
    builder.build()
}

/// Settings › Transparency: turns the material on or off in every main
/// window. Returns whether the system supports it, so the UI knows whether to
/// draw clear and to offer the setting.
#[tauri::command]
pub fn set_transparency<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>, on: bool) -> bool {
    state.opaque.store(!on, Ordering::SeqCst);
    if !supported() {
        return false;
    }
    for main in state.main_windows() {
        if let Some(window) = app.get_webview_window(&main.label) {
            let _ = window.set_effects(on.then(effects));
        }
    }
    true
}
