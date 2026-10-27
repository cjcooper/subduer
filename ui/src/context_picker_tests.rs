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
