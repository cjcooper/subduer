//! The app menu: Tauri's default, with an About item that carries the app
//! icon and says who makes the app. Without an explicit icon, macOS shows a
//! generic one whenever the binary isn't running from a bundle (`cargo tauri dev`).

use tauri::image::Image;
use tauri::menu::{AboutMetadata, Menu, MenuItemKind, PredefinedMenuItem};
use tauri::{AppHandle, Runtime};

const ICON: &[u8] = include_bytes!("../icons/128x128@2x.png");
pub const MAKER: &str = "it-problems";

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
