use super::*;

#[test]
fn arrows_home_and_end() {
    assert_eq!(step("ArrowDown", None, 3), Some(0));
    assert_eq!(step("ArrowDown", Some(1), 3), Some(2));
    assert_eq!(step("ArrowDown", Some(2), 3), Some(2));
    assert_eq!(step("ArrowUp", None, 3), Some(2));
    assert_eq!(step("ArrowUp", Some(0), 3), Some(0));
    assert_eq!(step("Home", Some(2), 3), Some(0));
    assert_eq!(step("End", Some(0), 3), Some(2));
    assert_eq!(step("a", Some(0), 3), None);
    assert_eq!(step("ArrowDown", None, 0), None);
}

#[test]
fn a_letter_jumps_to_the_next_match() {
    let names: Vec<String> = ["acme-uat", "shop-prod", "Shop-dev", "aks"].map(String::from).into();
    assert_eq!(jump(&names, Some(0), 's'), Some(1));
    assert_eq!(jump(&names, Some(1), 's'), Some(2));
    // Wraps round, and ignores case.
    assert_eq!(jump(&names, Some(2), 'S'), Some(1));
    assert_eq!(jump(&names, None, 'a'), Some(0));
    assert_eq!(jump(&names, Some(3), 'z'), None);
}

fn names() -> Vec<String> {
    ["acme-uat", "shop-prod", "Shop-dev"].map(String::from).into()
}

#[test]
fn keys_on_a_closed_menu_open_it() {
    for key in ["ArrowDown", "ArrowUp", "Enter", " "] {
        assert_eq!(key_action(key, false, None, &names()), KeyAction::Open, "{key}");
    }
    assert_eq!(key_action("a", false, None, &names()), KeyAction::Ignore);
    assert_eq!(key_action("Escape", false, None, &names()), KeyAction::Ignore);
}

#[test]
fn keys_on_an_open_menu() {
    let n = names();
    assert_eq!(key_action("Escape", true, Some(0), &n), KeyAction::Escape);
    assert_eq!(key_action("Tab", true, Some(0), &n), KeyAction::Tab);
    assert_eq!(key_action("Enter", true, Some(1), &n), KeyAction::Pick(Some(1)));
    assert_eq!(key_action(" ", true, None, &n), KeyAction::Pick(None));
    assert_eq!(key_action("Enter", true, Some(9), &n), KeyAction::Pick(None), "past the end");
    assert_eq!(key_action("End", true, Some(0), &n), KeyAction::Highlight { index: 2, prevent: true });
    assert_eq!(key_action("s", true, Some(0), &n), KeyAction::Highlight { index: 1, prevent: false });
    assert_eq!(key_action("z", true, Some(0), &n), KeyAction::Ignore);
    assert_eq!(key_action("-", true, Some(0), &n), KeyAction::Ignore, "not a letter");
    assert_eq!(key_action("Shift", true, Some(0), &n), KeyAction::Ignore, "a named key");
}

#[test]
fn list_positions_and_labels() {
    let ctx = |name: &str| ContextInfo { name: name.into(), cluster: None, namespace: None, prod_like: false, unlocked: false, provider: None };
    let list = vec![ctx("a"), ctx("b")];
    assert_eq!(selected_index(&list, Some("b")), Some(1));
    assert_eq!(selected_index(&list, Some("z")), None);
    assert_eq!(opening_index(&list, Some("b")), Some(1));
    assert_eq!(opening_index(&list, None), Some(0));
    assert_eq!(option_id(3), "context-option-3");
    assert_eq!(active_descendant(true, Some(2)).as_deref(), Some("context-option-2"));
    assert_eq!(active_descendant(true, None), None);
    assert_eq!(active_descendant(false, Some(2)), None);
    assert_eq!(empty_label(true), "No contexts");
    assert_eq!(empty_label(false), "Choose a context");
    assert!(is_switch(Some("a"), "b"));
    assert!(is_switch(None, "b"));
    assert!(!is_switch(Some("b"), "b"));
}

#[test]
fn highlighted_rows_scroll_into_view() {
    // A 200px list scrolled to 100 shows 100..300.
    assert_eq!(scroll_into_view(50, 80, 100, 200), Some(50), "above: to its top");
    assert_eq!(scroll_into_view(290, 320, 100, 200), Some(120), "below: to its bottom");
    assert_eq!(scroll_into_view(150, 180, 100, 200), None);
}
