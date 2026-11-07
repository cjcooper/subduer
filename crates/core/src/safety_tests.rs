use super::*;

#[test]
fn contexts_unlock_independently() {
    let gate = SafetyGate::default();
    gate.set_unlocked("acme-uat", true);
    assert!(gate.is_unlocked("acme-uat"));
    assert!(!gate.is_unlocked("acme-prd"));
    assert!(matches!(gate.ensure_writable("acme-prd"), Err(Error::Locked(c)) if c == "acme-prd"));
    // Locking an already locked context is harmless.
    gate.set_unlocked("acme-prd", false);
    assert!(!gate.is_unlocked("acme-prd"));
}

#[test]
fn saved_marks_are_pretty_json() {
    let dir = std::env::temp_dir().join(format!("subduer-prod-pretty-{}", std::process::id()));
    let path = dir.join("production.json");
    let _ = std::fs::remove_dir_all(&dir);
    let mut marks = ProdMarks::load(&path);
    marks.set("lab", None, true).unwrap();
    marks.set("acme", None, true).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\n  \"acme\": true,\n  \"lab\": true\n}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saving_reports_the_path_it_couldnt_write() {
    let dir = std::env::temp_dir().join(format!("subduer-prod-fail-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // The folder for the file is a file: it can't be created.
    let blocker = dir.join("blocker");
    std::fs::write(&blocker, "").unwrap();
    let mut under_file = ProdMarks::load(&blocker.join("production.json"));
    let err = under_file.set("lab", None, true).unwrap_err().to_string();
    assert!(err.starts_with(&blocker.display().to_string()), "{err}");
    // The file is a folder: it can't be written.
    let folder = dir.join("taken");
    std::fs::create_dir_all(&folder).unwrap();
    let mut on_folder = ProdMarks::load(&folder);
    let err = on_folder.set("lab", None, true).unwrap_err().to_string();
    assert!(err.starts_with(&folder.display().to_string()), "{err}");
    // An empty path has no folder to make, and can't be written.
    assert!(ProdMarks::load(std::path::Path::new("")).set("lab", None, true).is_err());
    // No path: nothing to save, and that's fine.
    assert!(ProdMarks::default().set("lab", None, true).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prod_tokens() {
    assert!(is_prod_like("acme-live"));
    assert!(is_prod_like("shop_prod2"));
    assert!(is_prod_like("PRD"));
    assert!(!is_prod_like(""));
    assert!(!is_prod_like("acme-dev"));
}
