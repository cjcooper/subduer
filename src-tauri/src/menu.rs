//! The app menu: Tauri's default, with an About item that opens our own About
//! window (the app, its license and every third-party license; the system's
//! About box can't hold them, and on Windows it's a plain message box), a
//! Settings… item (⌘, / Ctrl+,), File › New Window (⌘N / Ctrl+N), File ›
//! Manage Clusters… (the only place it is), a Trace…
//! item under Window, and our own Quit item on every system (⌘Q, Ctrl+Q on
//! Linux, Exit / Alt+F4 on Windows), so quitting can be held.

use tauri::menu::{Menu, MenuItem, MenuItemKind, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Manager, Runtime};

pub const MAKER: &str = "it-problems";
const SETTINGS_ID: &str = "settings";
const QUIT_ID: &str = "quit";
const TRACE_ID: &str = "trace";
const NEW_WINDOW_ID: &str = "new-window";
const MANAGE_CLUSTERS_ID: &str = "manage-clusters";
const ABOUT_ID: &str = "about";
/// Sent to the focused main window to open its Settings dialog.
pub const OPEN_SETTINGS: &str = "open-settings";
/// Sent to the focused main window to open the trace dialog (start, or its running one).
pub const OPEN_TRACE: &str = "open-trace";
/// Sent to the focused main window to open its Manage Clusters dialog.
pub const OPEN_MANAGE_CLUSTERS: &str = "open-manage-clusters";

pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = Menu::default(app)?;
    let about = MenuItem::with_id(app, ABOUT_ID, format!("About {}", app.package_info().name), true, None::<&str>)?;
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
    // New Window, then Manage Clusters…: first in File.
    let file_menu = menu.items()?.into_iter().find_map(|item| match item {
        MenuItemKind::Submenu(sub) if sub.text().is_ok_and(|t| t == "File") => Some(sub),
        _ => None,
    });
    if let Some(file) = file_menu {
        let new_window = MenuItem::with_id(app, NEW_WINDOW_ID, "New Window", true, Some("CmdOrCtrl+N"))?;
        let manage_clusters = MenuItem::with_id(app, MANAGE_CLUSTERS_ID, "Manage Clusters…", true, None::<&str>)?;
        file.insert(&new_window, 0)?;
        file.insert(&manage_clusters, 1)?;
        file.insert(&PredefinedMenuItem::separator(app)?, 2)?;
    }
    app.on_menu_event(|app, event| {
        if event.id() == ABOUT_ID {
            if let Err(e) = crate::about::open(app) {
                tracing::warn!(error = %e, "opening About failed");
            }
        } else if event.id() == NEW_WINDOW_ID {
            if let Err(e) = crate::new_main_window(app) {
                tracing::warn!(error = %e, "opening a new window failed");
            }
        } else if event.id() == QUIT_ID {
            // Goes through `RunEvent::ExitRequested`, which holds for unsaved
            // edits and a running trace.
            app.exit(0);
        } else if let Some(name) = [(SETTINGS_ID, OPEN_SETTINGS), (TRACE_ID, OPEN_TRACE), (MANAGE_CLUSTERS_ID, OPEN_MANAGE_CLUSTERS)]
            .iter()
            .find(|(id, _)| event.id() == *id)
            .map(|(_, name)| *name)
            && let Some(main) = focused_main(app)
        {
            let _ = main.set_focus();
            let _ = main.emit_to(main.label(), name, ());
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

/// The main window in front, or the one the window in front was opened from;
/// any main window if none is focused.
fn focused_main<R: Runtime>(app: &AppHandle<R>) -> Option<tauri::WebviewWindow<R>> {
    let state = app.state::<crate::AppState>();
    let windows = app.webview_windows();
    let focused = windows
        .iter()
        .find(|(_, w)| w.is_focused().unwrap_or(false))
        .and_then(|(label, _)| state.owner(label));
    let label = focused.or_else(|| state.main_windows().into_iter().map(|w| w.label).min())?;
    windows.get(&label).cloned()
}

/// macOS: the default menu's Quit (⌘Q) already asks the app first, so it
/// stays. Windows: the default File › Quit has no shortcut and posts a raw
/// quit message that skips the unsaved-edits hold; it becomes File › Exit,
/// labelled Alt+F4 but not bound to it, so Alt+F4 still closes only the
/// focused window (closing the main window quits). Linux: the default menu
/// has no File menu, so add one with Quit (Ctrl+Q).
#[cfg_attr(target_os = "macos", allow(unused_variables))]
fn add_quit<R: Runtime>(app: &AppHandle<R>, menu: &Menu<R>) -> tauri::Result<()> {
    // The predefined Quit sends `terminate:`, which tao ends the app on
    // without `ExitRequested`, so nothing could hold it. Ours goes through
    // `app.exit`, as on the other systems.
    #[cfg(target_os = "macos")]
    if let Some(MenuItemKind::Submenu(app_menu)) = menu.items()?.into_iter().next() {
        let items = app_menu.items()?;
        if let Some(MenuItemKind::Predefined(quit)) = items.last() {
            app_menu.remove(quit)?;
        }
        let label = format!("Quit {}", app.package_info().name);
        app_menu.append(&MenuItem::with_id(app, QUIT_ID, label, true, Some("Cmd+Q"))?)?;
    }
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
