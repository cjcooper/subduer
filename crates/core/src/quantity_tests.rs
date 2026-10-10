use super::*;

#[test]
fn every_suffix() {
    let cases = [
        ("1n", 1e-9), ("1u", 1e-6), ("1m", 1e-3), ("1k", 1e3), ("1M", 1e6), ("1G", 1e9),
        ("1T", 1e12), ("1P", 1e15), ("1E", 1e18), ("1Ki", 1024.0), ("1Mi", 1024f64.powi(2)),
        ("1Gi", 1024f64.powi(3)), ("1Ti", 1024f64.powi(4)), ("1Pi", 1024f64.powi(5)), ("1Ei", 1024f64.powi(6)),
        ("2e2", 200.0), ("5E-1", 0.5), (" 3 ", 3.0), ("-1", -1.0), ("+2", 2.0),
    ];
    for (s, want) in cases {
        let got = parse(s).unwrap_or_else(|| panic!("{s} didn't parse"));
        assert!((got - want).abs() <= want.abs() * 1e-12, "{s}: {got} != {want}");
    }
}

#[test]
fn bad_quantities() {
    assert_eq!(parse(""), None);
    assert_eq!(parse("Gi"), None);
    assert_eq!(parse("1ex"), None);
    assert_eq!(parse("1.2.3"), None);
    assert_eq!(millicores("lots"), 0.0);
    assert_eq!(bytes("lots"), 0.0);
}
