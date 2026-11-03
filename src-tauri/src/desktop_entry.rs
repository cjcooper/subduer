//! Desktop entry for runs that weren't installed from a package (Linux).
//!
//! Linux desktops find a window's icon through its desktop entry. On Wayland
//! (KDE Plasma, GNOME) the window carries no icon, only an app id: GTK uses
//! the binary name, `subduer`, and the compositor looks up
//! `subduer.desktop` for the icon. The .deb and .rpm install that entry; a
//! bare binary, an AppImage or `cargo tauri dev` don't, so those windows get a
//! generic icon. For those runs this writes a user entry and icon under
//! `$XDG_DATA_HOME`, marked so later runs can update or remove them.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The Wayland app id and X11 `WM_CLASS` GTK gives the window: the binary name.
pub const APP_ID: &str = "subduer";
const MARKER: &str = "X-Subduer-Generated=true";
const ICON: &[u8] = include_bytes!("../icons/256x256.png");

/// Writes, updates or removes the user entry. Must run before the window opens.
#[cfg(target_os = "linux")]
pub fn ensure() {
    let Some(home) = data_home() else { return };
    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
    let exec = match appimage.clone().map(Ok).unwrap_or_else(std::env::current_exe) {
        Ok(exec) => exec,
        Err(err) => return tracing::warn!(%err, "could not find the executable for the desktop entry"),
    };
    if let Err(err) = sync(&home, &data_dirs(), &exec, appimage.is_some()) {
        tracing::warn!(%err, "could not write the desktop entry");
    }
}

#[cfg(not(target_os = "linux"))]
pub fn ensure() {}

#[cfg(target_os = "linux")]
fn data_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(dir) if !dir.is_empty() => Some(dir.into()),
        _ => std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share")),
    }
}

#[cfg(target_os = "linux")]
fn data_dirs() -> Vec<PathBuf> {
    let dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    let dirs = if dirs.is_empty() { "/usr/local/share:/usr/share".into() } else { dirs };
    dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from).collect()
}

fn entry_path(data_dir: &Path) -> PathBuf {
    data_dir.join("applications").join(format!("{APP_ID}.desktop"))
}

fn icon_path(data_home: &Path) -> PathBuf {
    data_home.join("icons/hicolor/256x256/apps").join(format!("{APP_ID}.png"))
}

fn generated(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|text| text.lines().any(|l| l.trim() == MARKER))
}

/// `data_home` is `$XDG_DATA_HOME`, `data_dirs` the system data dirs, `exec`
/// what the entry launches. An AppImage's entry shows in the app menu; other
/// runs (a dev build, a bare binary) only lend the window its icon.
fn sync(data_home: &Path, data_dirs: &[PathBuf], exec: &Path, appimage: bool) -> io::Result<()> {
    let entry = entry_path(data_home);
    let ours = generated(&entry);
    // An installed package's entry wins; drop ours so it doesn't shadow it.
    if data_dirs.iter().any(|dir| entry_path(dir).is_file()) {
        if ours {
            fs::remove_file(&entry)?;
            let _ = fs::remove_file(icon_path(data_home));
        }
        return Ok(());
    }
    // The user's own entry is left alone.
    if entry.exists() && !ours {
        return Ok(());
    }
    let text = render(exec, appimage);
    if fs::read_to_string(&entry).ok().as_deref() != Some(text.as_str()) {
        fs::create_dir_all(entry.parent().unwrap())?;
        fs::write(&entry, text)?;
    }
    let icon = icon_path(data_home);
    if fs::read(&icon).ok().as_deref() != Some(ICON) {
        fs::create_dir_all(icon.parent().unwrap())?;
        fs::write(&icon, ICON)?;
    }
    Ok(())
}

fn render(exec: &Path, appimage: bool) -> String {
    let mut text = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Subduer\n\
         Comment=Browse and control Kubernetes clusters\n\
         Exec={exec}\n\
         Icon={APP_ID}\n\
         StartupWMClass={APP_ID}\n\
         Categories=Development;\n\
         Terminal=false\n",
        exec = exec_arg(&exec.to_string_lossy()),
    );
    if !appimage {
        text.push_str("NoDisplay=true\n");
    }
    text.push_str(MARKER);
    text.push('\n');
    text
}

/// Quotes a path for `Exec=` (Desktop Entry spec, "The Exec key"): reserved
/// characters are backslash-escaped inside double quotes, `%` is doubled, and
/// then backslashes are escaped again because the value is a string.
fn exec_arg(path: &str) -> String {
    let mut out = String::from("\"");
    for c in path.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out.replace('\\', "\\\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("subduer-desktop-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn exec_paths_are_quoted_and_escaped() {
        assert_eq!(exec_arg("/opt/subduer"), r#""/opt/subduer""#);
        assert_eq!(exec_arg("/home/a b/100%/x$y"), r#""/home/a b/100%%/x\\$y""#);
        assert_eq!(exec_arg(r#"/q"\"#), r#""/q\\"\\\\""#);
    }

    #[test]
    fn writes_entry_and_icon_when_nothing_is_installed() {
        let dir = scratch("fresh");
        let home = dir.join("home");
        sync(&home, &[dir.join("usr")], Path::new("/tmp/subduer"), false).unwrap();
        let text = fs::read_to_string(entry_path(&home)).unwrap();
        assert!(text.contains("Exec=\"/tmp/subduer\"\n"));
        assert!(text.contains("Icon=subduer\n"));
        assert!(text.contains("StartupWMClass=subduer\n"));
        assert!(text.contains("NoDisplay=true\n"));
        assert_eq!(fs::read(icon_path(&home)).unwrap(), ICON);
    }

    #[test]
    fn appimage_entry_shows_in_menus_and_follows_moves() {
        let dir = scratch("appimage");
        let home = dir.join("home");
        sync(&home, &[], Path::new("/a/App.AppImage"), true).unwrap();
        sync(&home, &[], Path::new("/b/App.AppImage"), true).unwrap();
        let text = fs::read_to_string(entry_path(&home)).unwrap();
        assert!(text.contains("Exec=\"/b/App.AppImage\"\n"));
        assert!(!text.contains("NoDisplay"));
    }

    #[test]
    fn installed_package_replaces_generated_entry() {
        let dir = scratch("installed");
        let (home, usr) = (dir.join("home"), dir.join("usr"));
        sync(&home, std::slice::from_ref(&usr), Path::new("/tmp/subduer"), false).unwrap();
        fs::create_dir_all(usr.join("applications")).unwrap();
        fs::write(entry_path(&usr), "[Desktop Entry]\n").unwrap();
        sync(&home, &[usr], Path::new("/usr/bin/subduer"), false).unwrap();
        assert!(!entry_path(&home).exists());
        assert!(!icon_path(&home).exists());
    }

    #[test]
    fn users_own_entry_is_left_alone() {
        let dir = scratch("own");
        let home = dir.join("home");
        fs::create_dir_all(home.join("applications")).unwrap();
        fs::write(entry_path(&home), "[Desktop Entry]\nName=Mine\n").unwrap();
        sync(&home, &[], Path::new("/tmp/subduer"), false).unwrap();
        assert_eq!(fs::read_to_string(entry_path(&home)).unwrap(), "[Desktop Entry]\nName=Mine\n");
    }
}
