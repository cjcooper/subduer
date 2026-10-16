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
