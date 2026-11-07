use super::*;

#[test]
fn matches_ignore_case_and_count_utf16() {
    assert_eq!(matches_in("Pod web-1 on WEB node", "web"), [(4, 7), (13, 16)]);
    assert_eq!(matches_in("aaaa", "aa"), [(0, 2), (2, 4)], "no overlaps");
    assert_eq!(matches_in("anything", "  "), [], "blank finds nothing");
    assert_eq!(matches_in("x", "xy"), []);
    // "é" is one UTF-16 unit, "😀" two: offsets follow the DOM's counting.
    assert_eq!(matches_in("é😀 api", "api"), [(4, 7)]);
    assert_eq!(matches_in("  padded ", " padded "), [(2, 8)], "the query is trimmed");
}

#[test]
fn counts_and_steps_wrap() {
    assert_eq!(count_text(None, 0, ""), "");
    assert_eq!(count_text(None, 0, "x"), "No matches");
    assert_eq!(count_text(Some(2), 12, "x"), "3 of 12");
    assert_eq!(count_text(None, 4, "x"), "4 matches");
    assert_eq!(step(None, 0, false), None);
    assert_eq!(step(None, 3, false), Some(0));
    assert_eq!(step(None, 3, true), Some(2));
    assert_eq!(step(Some(2), 3, false), Some(0));
    assert_eq!(step(Some(0), 3, true), Some(2));
    assert_eq!(step(Some(1), 3, false), Some(2));
}

#[test]
fn finding_again_keeps_the_place() {
    assert_eq!(keep_current(Some(2), 5), Some(2));
    assert_eq!(keep_current(Some(7), 5), Some(4), "fewer matches now");
    assert_eq!(keep_current(Some(0), 0), None);
    assert_eq!(keep_current(None, 3), None);
    assert!(goes_to_first(true, "web"));
    assert!(!goes_to_first(true, "  "));
    assert!(!goes_to_first(false, "web"));
}

#[test]
fn shortcuts_and_keys() {
    assert_eq!(shortcut_for(Desktop::Mac), "⌘F");
    assert_eq!(shortcut_for(Desktop::Windows), "Ctrl+F");
    assert_eq!(shortcut_for(Desktop::Linux), "Ctrl+F");
    assert_eq!(button_title("⌘F"), "Find in this panel (⌘F)");
    assert!(is_find_key(true, false, false, "f", "f"));
    assert!(is_find_key(false, true, false, "F", "f"));
    assert!(!is_find_key(false, false, false, "f", "f"));
    assert!(!is_find_key(true, false, true, "f", "f"));
    assert!(!is_find_key(true, false, false, "g", "f"));
}

#[test]
fn scrolling_to_a_match() {
    assert!(scrolls("auto"));
    assert!(scrolls("scroll"));
    assert!(!scrolls("visible"));
    // A 20px match at 300 in a 200px box at 100: 110px down centres it.
    assert_eq!(centre_offset(300.0, 20.0, 100.0, 200.0), 110.0);
    assert!(outside(90.0, 120.0, 100.0, 300.0));
    assert!(outside(250.0, 320.0, 100.0, 300.0));
    assert!(!outside(150.0, 200.0, 100.0, 300.0));
    assert_eq!(reveal_delay_ms(true), 50);
    assert_eq!(reveal_delay_ms(false), 0);
}
