//! The app menu: Tauri's default, with an About item that carries the app
//! icon and says who makes the app, and a Settings… item (⌘, / Ctrl+,).
//! Without an explicit icon, macOS shows a generic one whenever the binary
//! isn't running from a bundle (`cargo tauri dev`).

use tauri::image::Image;
use tauri::menu::{AboutMetadata, Menu, MenuItem, MenuItemKind, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Manager, Runtime};

const ICON: &[u8] = include_bytes!("../icons/128x128@2x.png");
pub const MAKER: &str = "it-problems";
const SETTINGS_ID: &str = "settings";
/// Sent to the main window to open its Settings dialog.
pub const OPEN_SETTINGS: &str = "open-settings";

fn about_metadata<R: Runtime>(app: &AppHandle<R>) -> AboutMetadata<'static> {
    let info = app.package_info();
    AboutMetadata {
        name: Some(info.name.clone()),
        version: Some(info.version.to_string()),
        copyright: app.config().bundle.copyright.clone(),
        credits: Some(format!("An {MAKER} application.")),
        comments: Some(format!("An {MAKER} application.")),
        authors: Some(vec![MAKER.to_owned()]),
        icon: Image::from_bytes(ICON).ok(),
        ..Default::default()
    }
}

pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = Menu::default(app)?;
    let about = PredefinedMenuItem::about(app, None, Some(about_metadata(app)))?;
    // macOS: About is the first item of the app-name menu. Elsewhere it's the
    // only item of the last (Help) menu.
    let (submenu, position) = if cfg!(target_os = "macos") {
        (menu.items()?.into_iter().next(), 0)
    } else {
        (menu.items()?.into_iter().last(), 0)
    };
    if let Some(MenuItemKind::Submenu(submenu)) = submenu
        && submenu.remove_at(position)?.is_some()
    {
        submenu.insert(&about, position)?;
    }
    // Settings…: after About on macOS (where apps keep it), first in the
    // first (File) menu elsewhere.
    let settings = MenuItem::with_id(app, SETTINGS_ID, "Settings…", true, Some("CmdOrCtrl+,"))?;
    if let Some(MenuItemKind::Submenu(first)) = menu.items()?.into_iter().next() {
        if cfg!(target_os = "macos") {
            first.insert(&PredefinedMenuItem::separator(app)?, 1)?;
            first.insert(&settings, 2)?;
        } else {
            first.insert(&settings, 0)?;
            first.insert(&PredefinedMenuItem::separator(app)?, 1)?;
        }
    }
    app.on_menu_event(|app, event| {
        if event.id() == SETTINGS_ID
            && let Some(main) = app.get_webview_window("main")
        {
            let _ = main.set_focus();
            let _ = main.emit_to("main", OPEN_SETTINGS, ());
        }
    });
    // macOS lists open windows (main and logs) under Window once it knows
    // which submenu that is; Tauri's default menu doesn't say.
    #[cfg(target_os = "macos")]
    for item in menu.items()? {
        if let MenuItemKind::Submenu(sub) = item
            && sub.text()? == "Window"
        {
            sub.set_as_windows_menu_for_nsapp()?;
        }
    }
    app.set_menu(menu)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_icon_decodes() {
        let icon = Image::from_bytes(ICON).unwrap();
        assert_eq!((icon.width(), icon.height()), (256, 256));
    }
}
