use std::process::Command;

/// Replaces `PATH` with the one from the user's login shell, if it can be read.
/// Must run before any other threads start.
pub fn inherit_login_path() {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
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
