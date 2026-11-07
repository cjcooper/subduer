//! The app menu: Tauri's default, with an About item that opens our own About
//! window (the app, its license and every third-party license; the system's
//! About box can't hold them, and on Windows it's a plain message box), a
//! Settings… item (⌘, / Ctrl+,), File › New Window (⌘N / Ctrl+N), File ›
//! Manage Clusters… (the only place it is), a Trace…
//! item under Window, and our own Quit item on every system (⌘Q, Ctrl+Q on
//! Linux, Exit / Alt+F4 on Windows), so quitting can be held.
//!
//! Where the systems differ, the choice is made by small functions that take
//! the system as an argument, so each system's layout is checked everywhere.

use tauri::menu::{IsMenuItem, Menu, MenuEvent, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu};
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

const MAC: bool = cfg!(target_os = "macos");

pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = Menu::default(app)?;
    add_about(app, &menu)?;
    add_quit(app, &menu)?;
    add_settings(app, &menu)?;
    add_trace(app, &menu)?;
    add_file_items(app, &menu)?;
    app.on_menu_event(on_menu_event);
    // macOS lists open windows (main and logs) under Window once it knows
    // which submenu that is; Tauri's default menu doesn't say.
    #[cfg(target_os = "macos")]
    for (title, sub) in submenus(&menu)? {
        if title == "Window" {
            sub.set_as_windows_menu_for_nsapp()?;
        }
    }
    app.set_menu(menu)?;
    Ok(())
}

/// The menu's submenus, with their titles.
fn submenus<R: Runtime>(menu: &Menu<R>) -> tauri::Result<Vec<(String, Submenu<R>)>> {
    let subs = menu.items()?.into_iter().filter_map(|item| item.as_submenu().cloned());
    Ok(subs.map(|sub| (sub.text().unwrap_or_default(), sub)).collect())
}

/// Which of `count` submenus About goes in: the first (the app-name menu) on
/// macOS, the last (Help) elsewhere. It replaces the system's About there.
fn about_menu(mac: bool, count: usize) -> usize {
    if mac { 0 } else { count.saturating_sub(1) }
}

fn add_about<R: Runtime>(app: &AppHandle<R>, menu: &Menu<R>) -> tauri::Result<()> {
    let about = MenuItem::with_id(app, ABOUT_ID, format!("About {}", app.package_info().name), true, None::<&str>)?;
    let subs = submenus(menu)?;
    let index = about_menu(MAC, subs.len());
    let Some((_, submenu)) = subs.into_iter().nth(index) else { return Ok(()) };
    let Some(_) = submenu.remove_at(0)? else { return Ok(()) };
    submenu.insert(&about, 0)?;
    Ok(())
}

/// Where Settings… and the separator next to it go in the first menu:
/// after About on macOS (where apps keep it), first in File elsewhere.
fn settings_positions(mac: bool) -> (usize, usize) {
    if mac { (2, 1) } else { (0, 1) }
}

fn add_settings<R: Runtime>(app: &AppHandle<R>, menu: &Menu<R>) -> tauri::Result<()> {
    let settings = MenuItem::with_id(app, SETTINGS_ID, "Settings…", true, Some("CmdOrCtrl+,"))?;
    let separator = PredefinedMenuItem::separator(app)?;
    let Some((_, first)) = submenus(menu)?.into_iter().next() else { return Ok(()) };
    let (settings_at, separator_at) = settings_positions(MAC);
    let mut items: [(usize, &dyn IsMenuItem<R>); 2] = [(settings_at, &settings), (separator_at, &separator)];
    items.sort_by_key(|(position, _)| *position);
    for (position, item) in items {
        first.insert(item, position)?;
    }
    Ok(())
}

/// Which submenu Trace… ends: Window, or the first if there's none.
fn trace_menu(titles: &[&str]) -> Option<usize> {
    titles.iter().position(|t| *t == "Window").or((!titles.is_empty()).then_some(0))
}

fn add_trace<R: Runtime>(app: &AppHandle<R>, menu: &Menu<R>) -> tauri::Result<()> {
    let trace = MenuItem::with_id(app, TRACE_ID, "Trace…", true, None::<&str>)?;
    let subs = submenus(menu)?;
    let titles: Vec<&str> = subs.iter().map(|(title, _)| title.as_str()).collect();
    let Some(index) = trace_menu(&titles) else { return Ok(()) };
    let sub = &subs[index].1;
    sub.append(&PredefinedMenuItem::separator(app)?)?;
    sub.append(&trace)
}

/// New Window, then Manage Clusters…: first in File.
fn add_file_items<R: Runtime>(app: &AppHandle<R>, menu: &Menu<R>) -> tauri::Result<()> {
    let Some((_, file)) = submenus(menu)?.into_iter().find(|(title, _)| title == "File") else { return Ok(()) };
    let new_window = MenuItem::with_id(app, NEW_WINDOW_ID, "New Window", true, Some("CmdOrCtrl+N"))?;
    let manage_clusters = MenuItem::with_id(app, MANAGE_CLUSTERS_ID, "Manage Clusters…", true, None::<&str>)?;
    file.insert(&new_window, 0)?;
    file.insert(&manage_clusters, 1)?;
    file.insert(&PredefinedMenuItem::separator(app)?, 2)
}

/// The event a menu item sends to the focused main window, for items that
/// open one of its dialogs.
fn event_for(id: &str) -> Option<&'static str> {
    [(SETTINGS_ID, OPEN_SETTINGS), (TRACE_ID, OPEN_TRACE), (MANAGE_CLUSTERS_ID, OPEN_MANAGE_CLUSTERS)]
        .iter()
        .find(|(item, _)| *item == id)
        .map(|(_, name)| *name)
}

pub(crate) fn on_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id = event.id().as_ref();
    if id == ABOUT_ID {
        crate::log_err(crate::about::open(app), "opening About failed");
    } else if id == NEW_WINDOW_ID {
        crate::log_err(crate::new_main_window(app), "opening a new window failed");
    } else if id == QUIT_ID {
        // Goes through `RunEvent::ExitRequested`, which holds for unsaved
        // edits and a running trace.
        app.exit(0);
    } else if let Some(name) = event_for(id)
        && let Some(main) = focused_main(app)
    {
        let _ = main.set_focus();
        let _ = main.emit_to(main.label(), name, ());
    }
}

/// The main window in front, or the one the window in front was opened from;
/// any main window if none is focused.
fn focused_main<R: Runtime>(app: &AppHandle<R>) -> Option<tauri::WebviewWindow<R>> {
    let windows = app.webview_windows();
    let focused = windows.iter().find_map(|(label, w)| w.is_focused().unwrap_or(false).then_some(label.as_str()));
    let label = main_for(&app.state::<crate::AppState>(), focused)?;
    windows.get(&label).cloned()
}

/// The main window `focused` is or was opened from; else the first main window.
fn main_for(state: &crate::AppState, focused: Option<&str>) -> Option<String> {
    focused.and_then(|label| state.owner(label)).or_else(|| state.main_windows().into_iter().map(|w| w.label).min())
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
    {
        let Some((_, app_menu)) = submenus(menu)?.into_iter().next() else { return Ok(()) };
        let items = app_menu.items()?;
        items.last().and_then(MenuItemKind::as_predefined_menuitem).map(|quit| app_menu.remove(quit)).transpose()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_system_puts_about_and_settings_in_its_own_place() {
        // macOS: the app-name menu, after About. Elsewhere: Help, and first in File.
        assert_eq!(about_menu(true, 6), 0);
        assert_eq!(about_menu(false, 5), 4);
        assert_eq!(about_menu(false, 0), 0);
        assert_eq!(settings_positions(true), (2, 1));
        assert_eq!(settings_positions(false), (0, 1));
    }

    #[test]
    fn trace_goes_under_window() {
        assert_eq!(trace_menu(&["Subduer", "File", "Edit", "Window", "Help"]), Some(3));
        assert_eq!(trace_menu(&["File", "Edit"]), Some(0));
        assert_eq!(trace_menu(&[]), None);
    }

    #[test]
    fn dialog_items_send_their_event() {
        assert_eq!(event_for(SETTINGS_ID), Some(OPEN_SETTINGS));
        assert_eq!(event_for(TRACE_ID), Some(OPEN_TRACE));
        assert_eq!(event_for(MANAGE_CLUSTERS_ID), Some(OPEN_MANAGE_CLUSTERS));
        assert_eq!(event_for(ABOUT_ID), None);
    }

    #[test]
    fn menu_items_go_to_the_main_window_in_front() {
        let state = crate::AppState::default();
        assert_eq!(main_for(&state, None), None);
        state.add_window("main-2");
        state.add_window("main");
        state.adopt("logs-1", "main-2");
        assert_eq!(main_for(&state, Some("logs-1")).as_deref(), Some("main-2"));
        assert_eq!(main_for(&state, Some("main-2")).as_deref(), Some("main-2"));
        // Nothing of ours in front (the About window): the first main window.
        assert_eq!(main_for(&state, Some("about")).as_deref(), Some("main"));
        assert_eq!(main_for(&state, None).as_deref(), Some("main"));
    }
}
