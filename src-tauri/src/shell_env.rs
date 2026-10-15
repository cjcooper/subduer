use std::process::Command;

/// Replaces `PATH` with the one from the user's login shell, if it can be read.
/// Must run before any other threads start.
///
/// macOS and Linux desktop launchers start apps with a minimal `PATH`, so
/// kubeconfig exec plugins (`aws`, `gke-gcloud-auth-plugin`, …) and `helm`
/// wouldn't be found. Windows apps already get the user's `PATH`.
#[cfg(unix)]
pub fn inherit_login_path() {
    let default = if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/sh" };
    let shell = std::env::var("SHELL").unwrap_or_else(|_| default.into());
    let output = Command::new(shell).args(["-ilc", "printf %s \"$PATH\""]).output();
    match output {
        Ok(out) if out.status.success() && !out.stdout.is_empty() => {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            // SAFETY: called at startup before any threads are spawned.
            unsafe { std::env::set_var("PATH", path) };
        }
        Ok(out) => tracing::warn!(status = %out.status, "could not read login shell PATH"),
        Err(err) => tracing::warn!(%err, "could not run login shell to read PATH"),
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

    #[test]
    fn recognises_raspberry_pi_models() {
        assert!(is_raspberry_pi("Raspberry Pi 4 Model B Rev 1.4\0"));
        assert!(is_raspberry_pi("Raspberry Pi 5 Model B Rev 1.0\0"));
        assert!(!is_raspberry_pi("Pine64 RockPro64 v2.1\0"));
        assert!(!is_raspberry_pi(""));
    }
}
