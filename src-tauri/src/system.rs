//! What the app asks of the desktop: native file dialogs, and helper
//! programs (the browser, Finder). Both are held in the app's state, so tests
//! can stand in for them; the app itself always uses the real ones.

use std::path::{Path, PathBuf};
use std::process::Command;

use tauri::Runtime;
use tauri_plugin_dialog::{FileDialogBuilder, FilePath};

use crate::commands::CmdResult;

/// Receives the path the user chose, or `None` if they cancelled.
pub(crate) type Reply = Box<dyn FnOnce(Option<FilePath>) + Send>;

/// Shows native Save and Open dialogs (the dialog plugin's, in the app).
pub struct Dialogs<R: Runtime> {
    pub(crate) save: fn(FileDialogBuilder<R>, Reply),
    pub(crate) open: fn(FileDialogBuilder<R>, Reply),
}

impl<R: Runtime> Default for Dialogs<R> {
    fn default() -> Self {
        Self { save: FileDialogBuilder::save_file, open: FileDialogBuilder::pick_file }
    }
}

/// `dialog`, starting in `dir` when there is one.
pub(crate) fn starting_in<R: Runtime>(dialog: FileDialogBuilder<R>, dir: Option<impl AsRef<Path>>) -> FileDialogBuilder<R> {
    match dir {
        Some(dir) => dialog.set_directory(dir),
        None => dialog,
    }
}

/// Which native dialog to show.
pub(crate) enum Ask {
    Save,
    Open,
}

/// Shows `dialog` with `dialogs` and waits for the user. `None` if they cancelled.
pub(crate) async fn ask<R: Runtime>(dialogs: &Dialogs<R>, dialog: FileDialogBuilder<R>, ask: Ask) -> CmdResult<Option<PathBuf>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let show = match ask {
        Ask::Save => dialogs.save,
        Ask::Open => dialogs.open,
    };
    show(
        dialog,
        Box::new(move |path| {
            let _ = tx.send(path);
        }),
    );
    let Some(path) = rx.await.ok().flatten() else { return Ok(None) };
    path.into_path().map(Some).map_err(crate::commands::err)
}

/// Starts a helper program without waiting for it.
pub struct Launcher(pub(crate) fn(&mut Command) -> std::io::Result<()>);

impl Default for Launcher {
    fn default() -> Self {
        Self(spawn)
    }
}

fn spawn(cmd: &mut Command) -> std::io::Result<()> {
    cmd.spawn().map(|_| ())
}

impl Launcher {
    /// Opens an http(s) address in the default browser, passed as a single
    /// argument (no shell).
    pub(crate) fn open_in_browser(&self, url: &str) -> CmdResult<()> {
        let mut cmd = browser_command(std::env::consts::OS, url);
        (self.0)(crate::shell_env::hide_console(&mut cmd)).map_err(crate::commands::err)
    }

    /// Shows a file in Finder or Explorer (selected), or opens its folder on Linux.
    pub(crate) fn reveal(&self, path: &Path) -> CmdResult<()> {
        (self.0)(&mut reveal_command(std::env::consts::OS, path)).map_err(crate::commands::err)
    }
}

/// How `os` (as in `std::env::consts::OS`) opens a URL in the browser.
fn browser_command(os: &str, url: &str) -> Command {
    let mut cmd = match os {
        "macos" => Command::new("open"),
        "windows" => {
            let mut c = Command::new("cmd");
            c.args(["/C", "start", ""]);
            c
        }
        _ => Command::new("xdg-open"),
    };
    cmd.arg(url);
    cmd
}

/// How `os` shows a file: selected in Finder or Explorer, or its folder.
fn reveal_command(os: &str, path: &Path) -> Command {
    match os {
        "macos" => {
            let mut c = Command::new("open");
            c.arg("-R").arg(path);
            c
        }
        "windows" => {
            let mut c = Command::new("explorer");
            c.arg(format!("/select,{}", path.display()));
            c
        }
        _ => {
            let mut c = Command::new("xdg-open");
            c.arg(path.parent().unwrap_or(path));
            c
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(cmd: &Command) -> (String, Vec<String>) {
        let program = cmd.get_program().to_string_lossy().into_owned();
        (program, cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect())
    }

    #[test]
    fn each_system_opens_urls_its_own_way() {
        let url = "https://acme.io/a";
        assert_eq!(args(&browser_command("macos", url)), ("open".into(), vec![url.into()]));
        assert_eq!(args(&browser_command("windows", url)), ("cmd".into(), vec!["/C".into(), "start".into(), String::new(), url.into()]));
        assert_eq!(args(&browser_command("linux", url)), ("xdg-open".into(), vec![url.into()]));
    }

    #[test]
    fn each_system_reveals_files_its_own_way() {
        let file = Path::new("/home/me/Downloads/a.txt");
        assert_eq!(args(&reveal_command("macos", file)), ("open".into(), vec!["-R".into(), file.display().to_string()]));
        assert_eq!(args(&reveal_command("windows", file)), ("explorer".into(), vec![format!("/select,{}", file.display())]));
        assert_eq!(args(&reveal_command("linux", file)), ("xdg-open".into(), vec!["/home/me/Downloads".into()]));
        // A bare root has no parent: the path itself.
        assert_eq!(args(&reveal_command("linux", Path::new("/"))).1, vec!["/".to_owned()]);
    }

    #[test]
    fn the_real_launcher_starts_programs() {
        // The test binary itself, listing its tests: harmless on every system.
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.arg("--list").stdout(std::process::Stdio::null());
        Launcher::default().0(&mut cmd).unwrap();
        // A program that can't start is an error the UI shows.
        let mut missing = Command::new("/nonexistent/subduer-opener");
        assert!(Launcher::default().0(&mut missing).is_err());
        let broken = Launcher(|_| Err(std::io::Error::other("no opener")));
        assert_eq!(broken.open_in_browser("https://acme.io").unwrap_err(), "no opener");
        assert_eq!(broken.reveal(Path::new("/tmp/a")).unwrap_err(), "no opener");
    }
}
