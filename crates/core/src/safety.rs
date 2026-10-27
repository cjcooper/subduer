//! Read-only-by-default guard for mutating operations.
//!
//! Every context starts locked. Anything that changes cluster state must call
//! [`SafetyGate::ensure_writable`] first. Removing the guard later means making
//! `ensure_writable` always succeed.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::{Error, Result};

#[derive(Debug, Default)]
pub struct SafetyGate {
    unlocked: Mutex<HashSet<String>>,
}

impl SafetyGate {
    pub fn is_unlocked(&self, context: &str) -> bool {
        self.unlocked.lock().unwrap().contains(context)
    }

    pub fn set_unlocked(&self, context: &str, unlocked: bool) {
        let mut set = self.unlocked.lock().unwrap();
        if unlocked {
            set.insert(context.to_owned());
        } else {
            set.remove(context);
        }
    }

    pub fn ensure_writable(&self, context: &str) -> Result<()> {
        if self.is_unlocked(context) {
            Ok(())
        } else {
            Err(Error::Locked(context.to_owned()))
        }
    }
}

/// Contexts the user marked as production, or as not, overriding
/// [`is_prod_like`]. Kept in a file of the app's own (`{"context": true}`):
/// the mark is this app's, so the kubeconfig isn't touched.
#[derive(Debug, Default)]
pub struct ProdMarks {
    path: Option<PathBuf>,
    marks: BTreeMap<String, bool>,
}

impl ProdMarks {
    /// The marks saved at `path`; none if it's missing or unreadable.
    pub fn load(path: &Path) -> Self {
        let marks = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { path: Some(path.to_owned()), marks }
    }

    /// Whether `context` (on `cluster`) counts as production: its mark, else its names.
    pub fn is_prod(&self, context: &str, cluster: Option<&str>) -> bool {
        self.marks.get(context).copied().unwrap_or_else(|| is_prod_like(context) || cluster.is_some_and(is_prod_like))
    }

    /// Marks `context` as production or not, and saves. A mark that agrees
    /// with its names is dropped, so a later rename is judged afresh.
    pub fn set(&mut self, context: &str, cluster: Option<&str>, prod: bool) -> Result<()> {
        if prod == (is_prod_like(context) || cluster.is_some_and(is_prod_like)) {
            self.marks.remove(context);
        } else {
            self.marks.insert(context.to_owned(), prod);
        }
        self.save()
    }

    /// Forgets `context`'s mark (it was removed from the kubeconfig), and saves.
    pub fn forget(&mut self, context: &str) -> Result<()> {
        if self.marks.remove(context).is_some() { self.save() } else { Ok(()) }
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::Message(format!("{}: {e}", dir.display())))?;
        }
        let text = serde_json::to_string_pretty(&self.marks).map_err(|e| Error::Message(e.to_string()))?;
        std::fs::write(path, text).map_err(|e| Error::Message(format!("{}: {e}", path.display())))
    }
}

/// Heuristic: does this context/cluster name look like production?
pub fn is_prod_like(name: &str) -> bool {
    name.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| matches!(token, "prd" | "prod" | "production" | "live") || token.starts_with("prod"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prod_detection() {
        assert!(is_prod_like("acme-prd-prd-v2"));
        assert!(is_prod_like("arn:aws:eks:us-east-1:1:cluster/prod-main"));
        assert!(is_prod_like("Production"));
        assert!(!is_prod_like("acme-uat-uat-v2"));
        assert!(!is_prod_like("docker-desktop"));
        assert!(!is_prod_like("reproduce"));
    }

    #[test]
    fn marks_override_the_names_and_are_saved() {
        let dir = std::env::temp_dir().join(format!("subduer-prod-marks-{}", std::process::id()));
        let path = dir.join("production.json");
        let _ = std::fs::remove_dir_all(&dir);
        let mut marks = ProdMarks::load(&path);
        assert!(marks.is_prod("shop-prod", None) && !marks.is_prod("lab", None));
        assert!(marks.is_prod("ops", Some("eks-prod-1")));
        marks.set("lab", None, true).unwrap();
        marks.set("shop-prod", None, false).unwrap();
        // Agreeing with the names: nothing to keep.
        marks.set("acme-uat", None, false).unwrap();
        let again = ProdMarks::load(&path);
        assert!(again.is_prod("lab", None) && !again.is_prod("shop-prod", None));
        assert_eq!(again.marks.len(), 2);
        marks.set("lab", None, false).unwrap();
        marks.forget("shop-prod").unwrap();
        assert!(ProdMarks::load(&path).marks.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gate_starts_locked() {
        let gate = SafetyGate::default();
        assert!(gate.ensure_writable("docker-desktop").is_err());
        gate.set_unlocked("docker-desktop", true);
        assert!(gate.ensure_writable("docker-desktop").is_ok());
        gate.set_unlocked("docker-desktop", false);
        assert!(gate.ensure_writable("docker-desktop").is_err());
    }
}

#[cfg(test)]
#[path = "safety_tests.rs"]
mod coverage_tests;
