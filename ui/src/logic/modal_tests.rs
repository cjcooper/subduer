use super::*;

#[test]
fn only_a_plain_esc_counts() {
    assert!(is_plain_esc("Escape", false, false, false, 27));
    assert!(!is_plain_esc("Enter", false, false, false, 13));
    assert!(!is_plain_esc("Escape", true, false, false, 27), "a held key's repeat");
    assert!(!is_plain_esc("Escape", false, true, false, 27), "already handled");
    assert!(!is_plain_esc("Escape", false, false, true, 27), "composing");
    assert!(!is_plain_esc("Escape", false, false, false, 229), "WebKit's composition end");
}

#[test]
fn frame_classes_and_stacking() {
    assert_eq!(backdrop_z(1), "11");
    assert_eq!(backdrop_z(3), "13");
    assert_eq!(card_class("wide-form"), "modal wide-form");
    assert_eq!(card_class(""), "modal ");
}

#[test]
fn outside_clicks_cancel_only_when_allowed() {
    assert!(click_cancels(true, true, false));
    assert!(!click_cancels(false, true, false));
    assert!(!click_cancels(true, false, false));
    assert!(!click_cancels(true, true, true));
}

#[test]
fn generations_move_on() {
    assert_eq!(next_generation(0), 1);
    assert!(still(4, 4));
    assert!(!still(next_generation(4), 4));
}
