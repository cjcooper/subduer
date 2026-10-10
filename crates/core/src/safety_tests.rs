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
fn prod_tokens() {
    assert!(is_prod_like("acme-live"));
    assert!(is_prod_like("shop_prod2"));
    assert!(is_prod_like("PRD"));
    assert!(!is_prod_like(""));
    assert!(!is_prod_like("acme-dev"));
}
