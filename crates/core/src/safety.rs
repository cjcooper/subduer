//! Read-only-by-default guard for mutating operations.
//!
//! Every context starts locked. Anything that changes cluster state must call
//! [`SafetyGate::ensure_writable`] first. Removing the guard later means making
//! `ensure_writable` always succeed.

use std::collections::HashSet;
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
