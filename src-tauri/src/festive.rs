//! A little holiday flair on macOS. The bundle's icon is the holiday version
//! (`icons/festive`, set in `tauri.macos.conf.json`), and the UI adds a hat to
//! its name and lights along the top bar. Other systems see no change.

/// For the UI: whether to add its holiday touches.
#[tauri::command]
pub fn festive() -> bool {
    cfg!(target_os = "macos")
}

#[cfg(test)]
mod tests {
    #[test]
    fn macos_only() {
        assert_eq!(super::festive(), cfg!(target_os = "macos"));
    }
}
