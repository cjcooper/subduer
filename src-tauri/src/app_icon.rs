//! The window icon follows the window's light or dark appearance on Windows,
//! where it shows in the title bar and taskbar. Elsewhere it stays light:
//! macOS takes the Dock icon from the bundle (`icons/AppIcon.icon`, which the
//! system switches itself), and Linux desktops take it from the desktop entry,
//! whose icon theme has no dark version.

use tauri::image::Image;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{AppHandle, Manager, Runtime, Theme, Window, WindowEvent};

const DARK: &[u8] = include_bytes!("../icons/dark-256x256.png");

/// Sets each window's icon as it opens, and again when its appearance changes.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("app-icon")
        .on_window_ready(|window| {
            if !cfg!(windows) {
                return;
            }
            apply(&window, window.theme().ok());
            let w = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::ThemeChanged(theme) = event {
                    apply(&w, Some(*theme));
                }
            });
        })
        .build()
}

/// Re-applies every window's icon, after Settings › Appearance changes.
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    if cfg!(windows) {
        for webview in app.webview_windows().values() {
            let window = webview.as_ref().window();
            apply(&window, window.theme().ok());
        }
    }
}

/// The icon version for a window's appearance: dark only for a dark window on
/// a system whose icons follow it; light when unknown.
fn version(follows: bool, theme: Option<Theme>) -> Theme {
    match theme {
        Some(Theme::Dark) if follows => Theme::Dark,
        _ => Theme::Light,
    }
}

fn apply<R: Runtime>(window: &Window<R>, theme: Option<Theme>) {
    let result = match version(cfg!(windows), theme) {
        Theme::Dark => Image::from_bytes(DARK).and_then(|icon| window.set_icon(icon)),
        _ => match window.app_handle().default_window_icon() {
            Some(icon) => window.set_icon(icon.clone()),
            None => Ok(()),
        },
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, "setting the window icon failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_only_where_icons_follow_the_appearance() {
        assert_eq!(version(true, Some(Theme::Dark)), Theme::Dark);
        assert_eq!(version(true, Some(Theme::Light)), Theme::Light);
        assert_eq!(version(false, Some(Theme::Dark)), Theme::Light);
    }

    #[test]
    fn light_when_the_appearance_is_unknown() {
        assert_eq!(version(true, None), Theme::Light);
    }

    #[test]
    fn dark_icon_decodes() {
        let icon = Image::from_bytes(DARK).unwrap();
        assert_eq!((icon.width(), icon.height()), (256, 256));
    }
}
