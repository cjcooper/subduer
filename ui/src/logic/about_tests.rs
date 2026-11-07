use super::*;

const SAMPLE: &str = "Third-party notices for Subduer

Subduer includes the software listed below. Each license is reproduced
with the crates it covers.

  2 crate(s) under MIT License
  1 crate(s) under ISC License

================================================================================
MIT License

Used by:
  serde 1.0.0 (https://github.com/serde-rs/serde)
  serde_json 1.0.0

Permission is hereby granted, free of charge,
to any person.

================================================================================
ISC License

Used by:
  a 1.0.0
  b 1.0.0
  c 1.0.0
  d 1.0.0

ISC text.

================================================================================
Vendored JavaScript: ace

Copyright (c) 2010, Ajax.org B.V.
";

#[test]
fn parses_licenses_with_their_crates() {
    let (intro, notices) = parse_notices(SAMPLE);
    assert!(intro.starts_with("Third-party notices"));
    assert_eq!(notices.len(), 3);
    assert_eq!(notices[0].title, "MIT License");
    assert_eq!(notices[0].used_by, ["serde 1.0.0 (https://github.com/serde-rs/serde)", "serde_json 1.0.0"]);
    assert_eq!(notices[0].text, "Permission is hereby granted, free of charge,\nto any person.");
    assert_eq!(notices[2].title, "Vendored JavaScript: ace");
    assert!(notices[2].used_by.is_empty());
    assert_eq!(notices[2].text, "Copyright (c) 2010, Ajax.org B.V.");
}

#[test]
fn summaries_name_the_first_crates() {
    let (_, notices) = parse_notices(SAMPLE);
    assert_eq!(summary(&notices[0]), "MIT License · serde, serde_json");
    assert_eq!(summary(&notices[1]), "ISC License · a, b, c and 1 more");
    assert_eq!(summary(&notices[2]), "Vendored JavaScript: ace");
}

#[test]
fn chips_count_the_software_under_each_license() {
    let (_, notices) = parse_notices(SAMPLE);
    assert_eq!(chips(&notices), [("ISC License".to_owned(), 4), ("MIT License".to_owned(), 2), ("JavaScript".to_owned(), 1)]);
    assert_eq!(kind(&notices[2]), "JavaScript");
}

#[test]
fn filter_matches_title_or_crate() {
    let (_, notices) = parse_notices(SAMPLE);
    assert!(matches(&notices[0], ""));
    assert!(matches(&notices[0], "SERDE_json"));
    assert!(matches(&notices[1], "isc"));
    assert!(!matches(&notices[1], "serde"));
}

#[test]
fn about_parameter() {
    assert!(is_about("?about"));
    assert!(is_about("?x=1&about"));
    assert!(!is_about("?logs=logs-1"));
    assert!(!is_about(""));
}

#[test]
fn the_real_notices_parse() {
    let (intro, notices) = parse_notices(include_str!("../../../THIRD-PARTY-NOTICES.txt"));
    assert!(intro.contains("crate(s) under MIT License"));
    assert_eq!(chips(&notices)[0].0, "MIT License");
    assert!(notices.iter().all(|n| !n.title.is_empty() && !n.text.is_empty()));
    assert!(notices.iter().map(|n| n.used_by.len()).sum::<usize>() > 100);
    assert!(notices.iter().any(|n| n.title == "Vendored JavaScript: xterm"));
}

#[test]
fn filtering_and_totals() {
    let (_, notices) = parse_notices(SAMPLE);
    let titles = |list: Vec<Notice>| list.into_iter().map(|n| n.title).collect::<Vec<_>>();
    assert_eq!(titles(filtered(&notices, "", None)).len(), 3);
    assert_eq!(titles(filtered(&notices, "ace", None)), ["Vendored JavaScript: ace"]);
    assert!(filtered(&notices, "nothing like it", None).is_empty());
    assert_eq!(titles(filtered(&notices, "", Some("JavaScript"))), ["Vendored JavaScript: ace"]);
    assert_eq!(titles(filtered(&notices, "serde", Some("MIT License"))), ["MIT License"]);
    assert!(filtered(&notices, "serde", Some("ISC License")).is_empty());
    assert_eq!(totals(&notices), (6, 1));
}

#[test]
fn sentences() {
    assert_eq!(version_text("0.1.0"), "Version 0.1.0");
    assert_eq!(description_text("Kubernetes, subdued."), "Kubernetes, subdued.");
    assert_eq!(description_text("Kubernetes, subdued"), "Kubernetes, subdued.");
    assert_eq!(maker_text("Acme", "© 2026 Acme"), "An Acme application. © 2026 Acme. Released under the MIT License.");
    assert_eq!(built_with_tail(300), ", and 300 open-source Rust crates.");
    assert_eq!(third_party_tab(300, 2), "Third-party software (302)");
    assert_eq!(
        third_party_text("Subduer", 300, 2),
        "Subduer includes 300 open-source Rust crates and 2 JavaScript libraries, used under the licenses below. \
         Each section lists the software it covers and the full license text."
    );
}

#[test]
fn names_are_joined_with_commas_and_a_final_and() {
    let seps: Vec<_> = (0..4).map(|i| list_separator(i, 4)).collect();
    assert_eq!(seps, ["", ", ", ", ", " and "]);
}

#[test]
fn a_block_without_a_title_line_is_skipped() {
    let (_, notices) = parse_notices("intro\n================================================================================\nno newline");
    assert!(notices.is_empty());
}
