//! The app menu: Tauri's default, with an About item that carries the app
//! icon and says who makes the app, a Settings… item (⌘, / Ctrl+,), a
//! Trace… item under Window, and a Quit item on every system (⌘Q, Ctrl+Q on
//! Linux, Exit / Alt+F4 on Windows).
//! Without an explicit icon, macOS shows a generic one whenever the binary
//! isn't running from a bundle (`cargo tauri dev`).

use tauri::image::Image;
use tauri::menu::{AboutMetadata, Menu, MenuItem, MenuItemKind, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Manager, Runtime};

const ICON: &[u8] = include_bytes!("../icons/128x128@2x.png");
pub const MAKER: &str = "it-problems";
const SETTINGS_ID: &str = "settings";
const QUIT_ID: &str = "quit";
const TRACE_ID: &str = "trace";
/// Sent to the main window to open its Settings dialog.
pub const OPEN_SETTINGS: &str = "open-settings";
/// Sent to the main window to open the trace dialog (start, or the running one).
pub const OPEN_TRACE: &str = "open-trace";

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
    add_quit(app, &menu)?;
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
    // Trace…: at the end of Window (or of the first menu if there's none).
    let trace = MenuItem::with_id(app, TRACE_ID, "Trace…", true, None::<&str>)?;
    let window_menu = menu.items()?.into_iter().find_map(|item| match item {
        MenuItemKind::Submenu(sub) if sub.text().is_ok_and(|t| t == "Window") => Some(sub),
        _ => None,
    });
    let trace_menu = window_menu.or_else(|| match menu.items().ok()?.into_iter().next() {
        Some(MenuItemKind::Submenu(first)) => Some(first),
        _ => None,
    });
    if let Some(sub) = trace_menu {
        sub.append(&PredefinedMenuItem::separator(app)?)?;
        sub.append(&trace)?;
    }
    app.on_menu_event(|app, event| {
        if event.id() == QUIT_ID {
            // Goes through `RunEvent::ExitRequested`, which holds for unsaved
            // edits and a running trace.
            app.exit(0);
        } else if let Some(name) = [(SETTINGS_ID, OPEN_SETTINGS), (TRACE_ID, OPEN_TRACE)]
            .iter()
            .find(|(id, _)| event.id() == *id)
            .map(|(_, name)| *name)
            && let Some(main) = app.get_webview_window("main")
        {
            let _ = main.set_focus();
            let _ = main.emit_to("main", name, ());
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

/// macOS: the default menu's Quit (⌘Q) already asks the app first, so it
/// stays. Windows: the default File › Quit has no shortcut and posts a raw
/// quit message that skips the unsaved-edits hold; it becomes File › Exit,
/// labelled Alt+F4 but not bound to it, so Alt+F4 still closes only the
/// focused window (closing the main window quits). Linux: the default menu
/// has no File menu, so add one with Quit (Ctrl+Q).
#[cfg_attr(target_os = "macos", allow(unused_variables))]
fn add_quit<R: Runtime>(app: &AppHandle<R>, menu: &Menu<R>) -> tauri::Result<()> {
    #[cfg(windows)]
    if let Some(MenuItemKind::Submenu(file)) = menu.items()?.into_iter().next() {
        let items = file.items()?;
        if let Some(MenuItemKind::Predefined(quit)) = items.last() {
            file.remove(quit)?;
        }
        file.append(&PredefinedMenuItem::separator(app)?)?;
        file.append(&MenuItem::with_id(app, QUIT_ID, "Exit\tAlt+F4", true, None::<&str>)?)?;
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let quit = MenuItem::with_id(app, QUIT_ID, "Quit", true, Some("Ctrl+Q"))?;
        menu.insert(&tauri::menu::Submenu::with_items(app, "File", true, &[&quit])?, 0)?;
    }
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
