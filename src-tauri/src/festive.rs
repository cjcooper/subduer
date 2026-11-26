//! A little holiday flair on macOS, from December 1 to January 6: the Dock
//! shows a festive icon (a Santa hat on a green and red helm) while the app
//! runs, and the UI adds a hat to its name and lights along the top bar.
//! The bundle's own icon stays as it is; other systems see no change.
//!
//! `SUBDUER_FESTIVE=1` turns it on outside the season and `=0` off inside it.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use jiff::civil::Date;
use tauri::Theme;

const LIGHT: &[u8] = include_bytes!("../icons/festive-512.png");
const DARK: &[u8] = include_bytes!("../icons/festive-dark-512.png");

/// Whether `date` falls in the holiday season: December, and on to Twelfth Night.
fn in_season(date: Date) -> bool {
    date.month() == 12 || (date.month() == 1 && date.day() <= 6)
}

/// `SUBDUER_FESTIVE` as on (`1`, `true`, `on`) or off (anything else); `None` unset.
fn forced(value: Option<&str>) -> Option<bool> {
    value.map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes"))
}

/// On macOS, whether it's the season (or the environment says so).
fn decide(macos: bool, forced: Option<bool>, today: impl FnOnce() -> Date) -> bool {
    macos && forced.unwrap_or_else(|| in_season(today()))
}

/// Whether the app dresses up today.
pub fn active() -> bool {
    let env = std::env::var("SUBDUER_FESTIVE").ok();
    decide(cfg!(target_os = "macos"), forced(env.as_deref()), || jiff::Zoned::now().date())
}

/// For the UI: whether to add its holiday touches.
#[tauri::command]
pub fn festive() -> bool {
    active()
}

/// The Dock icon for an appearance; light when unknown.
fn icon(theme: Option<Theme>) -> &'static [u8] {
    match theme {
        Some(Theme::Dark) => DARK,
        _ => LIGHT,
    }
}

/// In season, shows the festive icon in the Dock for `theme`, the main
/// window's appearance. Called at start and whenever that appearance changes.
pub fn dock_icon(theme: Option<Theme>) {
    if active() {
        set_dock_icon(icon(theme));
    }
}

#[cfg(target_os = "macos")]
fn set_dock_icon(png: &[u8]) {
    use objc2::AllocAnyThread;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    // AppKit only on the main thread, where setup and window events run.
    let Some(mtm) = MainThreadMarker::new() else { return };
    let data = NSData::with_bytes(png);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        tracing::warn!("the festive Dock icon didn't decode");
        return;
    };
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
}

#[cfg(not(target_os = "macos"))]
fn set_dock_icon(_png: &[u8]) {}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn the_season_runs_from_december_to_twelfth_night() {
        assert!(!in_season(date(2026, 11, 30)));
        assert!(in_season(date(2026, 12, 1)));
        assert!(in_season(date(2026, 12, 25)));
        assert!(in_season(date(2026, 12, 31)));
        assert!(in_season(date(2027, 1, 6)));
        assert!(!in_season(date(2027, 1, 7)));
        assert!(!in_season(date(2027, 7, 4)));
    }

    #[test]
    fn the_environment_overrides_the_calendar() {
        assert_eq!(forced(None), None);
        assert_eq!(forced(Some("1")), Some(true));
        assert_eq!(forced(Some(" On ")), Some(true));
        assert_eq!(forced(Some("0")), Some(false));
        assert_eq!(forced(Some("")), Some(false));
    }

    #[test]
    fn macos_only() {
        let christmas = || date(2026, 12, 25);
        let summer = || date(2026, 7, 1);
        assert!(decide(true, None, christmas));
        assert!(!decide(true, None, summer));
        assert!(decide(true, Some(true), summer));
        assert!(!decide(true, Some(false), christmas));
        assert!(!decide(false, None, christmas));
        assert!(!decide(false, Some(true), christmas));
    }

    #[test]
    fn icons_decode_and_follow_the_appearance() {
        for png in [LIGHT, DARK] {
            let image = tauri::image::Image::from_bytes(png).unwrap();
            assert_eq!((image.width(), image.height()), (512, 512));
        }
        assert!(icon(Some(Theme::Dark)) == DARK);
        assert!(icon(Some(Theme::Light)) == LIGHT);
        assert!(icon(None) == LIGHT);
    }

    #[test]
    fn setting_the_dock_icon_is_harmless_off_the_main_thread() {
        // Test threads aren't the main thread: AppKit is left alone.
        set_dock_icon(LIGHT);
        dock_icon(None);
    }
}
