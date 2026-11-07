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
    build(app, config, supported(), WebviewWindow::set_effects)
}

/// [`build_main`] on a system with a material (`material`) or without. The
/// material goes on as the window opens, as Tauri does for configured effects.
fn build<R: Runtime>(app: &AppHandle<R>, config: &WindowConfig, material: bool, set: SetEffects<R>) -> tauri::Result<WebviewWindow<R>> {
    let window = WebviewWindowBuilder::from_config(app, config)?.transparent(material).build()?;
    if material && !app.state::<AppState>().opaque.load(Ordering::SeqCst) {
        let _ = set(&window, Some(effects()));
    }
    Ok(window)
}

/// Settings › Transparency: turns the material on or off in every main
/// window. Returns whether the system supports it, so the UI knows whether to
/// draw clear and to offer the setting.
#[tauri::command]
pub fn set_transparency<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>, on: bool) -> bool {
    set_material(&app, &state, on, supported(), WebviewWindow::set_effects)
}

/// Sets or clears a window's effects (Tauri's, in the app).
type SetEffects<R> = fn(&WebviewWindow<R>, Option<WindowEffectsConfig>) -> tauri::Result<()>;

/// [`set_transparency`] on a system with a material (`material`) or without.
fn set_material<R: Runtime>(app: &AppHandle<R>, state: &AppState, on: bool, material: bool, set: SetEffects<R>) -> bool {
    state.opaque.store(!on, Ordering::SeqCst);
    if !material {
        return false;
    }
    for main in state.main_windows() {
        if let Some(window) = app.get_webview_window(&main.label) {
            let _ = set(&window, on.then(effects));
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Manager;
    use tauri::test::mock_app;

    #[test]
    fn only_macos_and_windows_11_have_a_material() {
        assert_eq!(supported(), cfg!(target_os = "macos") || windows_11());
        assert!(cfg!(windows) || !windows_11());
    }

    #[test]
    fn main_windows_get_the_material_unless_settings_turned_it_off() {
        let app = mock_app();
        app.manage(AppState::default());
        let state = app.state::<AppState>();
        // The mock's windows have no native window to put a material on, so
        // the effects are recorded instead.
        static SET: std::sync::Mutex<Vec<(String, bool)>> = std::sync::Mutex::new(Vec::new());
        let record: SetEffects<tauri::test::MockRuntime> = |w, effects| {
            SET.lock().unwrap().push((w.label().to_owned(), effects.is_some()));
            Ok(())
        };
        let config = |label: &str| WindowConfig { label: label.into(), ..Default::default() };
        for (label, material, opaque) in [("main", true, false), ("main-2", true, true), ("main-3", false, false)] {
            state.opaque.store(opaque, Ordering::SeqCst);
            state.add_window(label);
            let window = build(app.handle(), &config(label), material, record).unwrap();
            assert_eq!(window.label(), label);
        }
        // Only the transparent window that isn't opaque got the material.
        assert_eq!(*SET.lock().unwrap(), [("main".to_owned(), true)]);
        SET.lock().unwrap().clear();
        // A main window whose webview never opened is skipped.
        state.add_window("main-4");
        assert!(set_material(app.handle(), &state, false, true, record));
        assert!(state.opaque.load(Ordering::SeqCst));
        assert!(set_material(app.handle(), &state, true, true, record));
        assert!(!state.opaque.load(Ordering::SeqCst));
        let mut set = SET.lock().unwrap().clone();
        set.sort();
        let expected: Vec<(String, bool)> =
            ["main", "main-2", "main-3"].iter().flat_map(|l| [((*l).to_owned(), false), ((*l).to_owned(), true)]).collect();
        assert_eq!(set, expected);
        // Without a material the setting is remembered but does nothing.
        assert!(!set_material(app.handle(), &state, false, false, record));
        assert!(state.opaque.load(Ordering::SeqCst));
        assert_eq!(SET.lock().unwrap().len(), 6);
        assert_eq!(effects().effects, vec![Effect::Sidebar, Effect::Mica]);
    }
}
