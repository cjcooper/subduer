use std::process::Command;

/// Replaces `PATH` with the one from the user's login shell, if it can be read.
/// Must run before any other threads start.
///
/// macOS and Linux desktop launchers start apps with a minimal `PATH`, so
/// kubeconfig exec plugins (`aws`, `gke-gcloud-auth-plugin`, …) and `helm`
/// wouldn't be found. Windows apps already get the user's `PATH`.
#[cfg(unix)]
pub fn inherit_login_path() {
    let Some(path) = login_path(&login_shell(std::env::var("SHELL").ok())) else { return };
    // SAFETY: called at startup before any threads are spawned.
    unsafe { std::env::set_var("PATH", path) };
}

/// The user's shell (`$SHELL`), else the system's default.
#[cfg(unix)]
fn login_shell(shell: Option<String>) -> String {
    shell.unwrap_or_else(|| if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/sh" }.into())
}

/// The `PATH` `shell` sets up for a login, if it can be read.
#[cfg(unix)]
fn login_path(shell: &str) -> Option<String> {
    match Command::new(shell).args(["-ilc", "printf %s \"$PATH\""]).output() {
        Ok(out) if out.status.success() && !out.stdout.is_empty() => Some(String::from_utf8_lossy(&out.stdout).trim().to_owned()),
        Ok(out) => {
            tracing::warn!(status = %out.status, "could not read login shell PATH");
            None
        }
        Err(err) => {
            tracing::warn!(%err, "could not run login shell to read PATH");
            None
        }
    }
}

#[cfg(not(unix))]
pub fn inherit_login_path() {}

/// Turns off WebKitGTK's DMA-BUF renderer on a Raspberry Pi, unless the user
/// set `WEBKIT_DISABLE_DMABUF_RENDERER` themselves. Must run before any other
/// threads start.
///
/// With it on, the Pi's GPU driver draws the first frame as random lines
/// until the window is resized.
#[cfg(target_os = "linux")]
pub fn work_around_pi_rendering() {
    const VAR: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
    if std::env::var_os(VAR).is_some() {
        return;
    }
    let Ok(model) = std::fs::read_to_string("/proc/device-tree/model") else { return };
    if is_raspberry_pi(&model) {
        tracing::info!(model = model.trim_end_matches('\0'), "Raspberry Pi: setting {VAR}=1");
        // SAFETY: called at startup before any threads are spawned.
        unsafe { std::env::set_var(VAR, "1") };
    }
}

#[cfg(not(target_os = "linux"))]
pub fn work_around_pi_rendering() {}

/// Whether a device-tree model string (`/proc/device-tree/model`) names a
/// Raspberry Pi.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn is_raspberry_pi(model: &str) -> bool {
    model.starts_with("Raspberry Pi")
}

/// Keeps a command from opening a console window on Windows (a GUI app's
/// children otherwise get one each).
pub fn hide_console(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::is_raspberry_pi;

    #[cfg(unix)]
    #[test]
    fn reads_the_login_shells_path() {
        let path = super::login_path("/bin/sh").unwrap();
        assert!(path.split(':').any(|dir| dir == "/bin" || dir == "/usr/bin"), "{path}");
        // A shell that fails, and one that isn't there.
        assert_eq!(super::login_path("false"), None);
        assert_eq!(super::login_path("/nonexistent/shell"), None);
        assert_eq!(super::login_shell(Some("/bin/fish".into())), "/bin/fish");
        assert!(super::login_shell(None).starts_with("/bin/"));
    }

    #[test]
    fn recognises_raspberry_pi_models() {
        assert!(is_raspberry_pi("Raspberry Pi 4 Model B Rev 1.4\0"));
        assert!(is_raspberry_pi("Raspberry Pi 5 Model B Rev 1.0\0"));
        assert!(!is_raspberry_pi("Pine64 RockPro64 v2.1\0"));
        assert!(!is_raspberry_pi(""));
    }
}
