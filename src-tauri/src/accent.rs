//! The system accent colour, so buttons, selections and focus rings match the
//! desktop. Where the system doesn't say (macOS "Multicolor", a Linux desktop
//! without the appearance portal), the UI keeps its own blue.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// The accent colour as `#rrggbb`, or `None` to keep the app's own.
/// Linux asks the desktop portal over D-Bus, so it runs off the main thread.
#[cfg_attr(target_os = "linux", tauri::command(async))]
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub fn system_accent() -> Option<String> {
    let accent = read();
    tracing::debug!(?accent, "system accent colour");
    accent
}

/// `#rrggbb` from sRGB components in 0..=1; `None` if any is outside that
/// range (the portal's "no preference").
fn hex(r: f64, g: f64, b: f64) -> Option<String> {
    let byte = |c: f64| (0.0..=1.0).contains(&c).then(|| (c * 255.0).round() as u8);
    Some(format!("#{:02x}{:02x}{:02x}", byte(r)?, byte(g)?, byte(b)?))
}

/// Windows keeps the accent as `0xAABBGGRR`.
fn from_abgr(abgr: u32) -> String {
    let [r, g, b, _] = abgr.to_le_bytes();
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The colour in `gdbus call` output for the portal's `accent-color`, either
/// `(<(r, g, b)>,)` from `ReadOne` or `(<<(r, g, b)>>,)` from `Read`.
fn from_portal(output: &str) -> Option<String> {
    let inner = output.rsplit('(').next()?.split(')').next()?;
    let parts: Vec<f64> = inner.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    match parts[..] {
        [r, g, b] => hex(r, g, b),
        _ => None,
    }
}

/// The accent the user picked; `None` for Multicolor, where apps use their own.
#[cfg(target_os = "macos")]
fn read() -> Option<String> {
    use objc2_app_kit::{NSColor, NSColorSpace};
    use objc2_foundation::{NSString, NSUserDefaults};

    let defaults = NSUserDefaults::standardUserDefaults();
    defaults.objectForKey(&NSString::from_str("AppleAccentColor"))?;
    let color = NSColor::controlAccentColor().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    hex(color.redComponent(), color.greenComponent(), color.blueComponent())
}

#[cfg(windows)]
fn read() -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let dwm = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Microsoft\Windows\DWM").ok()?;
    dwm.get_value::<u32, _>("AccentColor").ok().map(from_abgr)
}

/// The freedesktop appearance portal (GNOME 47+, KDE Plasma 6).
#[cfg(target_os = "linux")]
fn read() -> Option<String> {
    let portal = |method: &str| {
        std::process::Command::new("gdbus")
            .args(["call", "--session", "--timeout", "2"])
            .args(["--dest", "org.freedesktop.portal.Desktop", "--object-path", "/org/freedesktop/portal/desktop"])
            .args(["--method", method, "org.freedesktop.appearance", "accent-color"])
            .output()
            .ok()
            .filter(|out| out.status.success())
            .and_then(|out| from_portal(&String::from_utf8_lossy(&out.stdout)))
    };
    portal("org.freedesktop.portal.Settings.ReadOne").or_else(|| portal("org.freedesktop.portal.Settings.Read"))
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn read() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_from_components() {
        assert_eq!(hex(0.0, 0.5, 1.0).as_deref(), Some("#0080ff"));
        assert_eq!(hex(-1.0, 0.5, 1.0), None);
        assert_eq!(hex(0.2, 1.5, 0.0), None);
    }

    #[test]
    fn windows_accent_is_abgr() {
        assert_eq!(from_abgr(0xff_d7_78_00), "#0078d7");
    }

    #[test]
    fn portal_output() {
        let read_one = "(<(0.20784313725490197, 0.51764705882352946, 0.89411764705882357)>,)\n";
        assert_eq!(from_portal(read_one).as_deref(), Some("#3584e4"));
        let read = "(<<(0.2, 0.6, 0.29)>>,)\n";
        assert_eq!(from_portal(read).as_deref(), Some("#33994a"));
        // Unset: components out of range.
        assert_eq!(from_portal("(<(-1.0, -1.0, -1.0)>,)"), None);
        assert_eq!(from_portal("(<uint32 0>,)"), None);
        assert_eq!(from_portal(""), None);
    }
}
