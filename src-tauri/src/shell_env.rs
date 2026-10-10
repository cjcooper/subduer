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
