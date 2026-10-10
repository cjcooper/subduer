use super::*;

fn usage(requests: Option<f64>, limits: Option<f64>, used: Option<f64>, node: Option<f64>) -> Usage {
    Usage { requests, limits, used, node }
}

#[test]
fn resource_labels_and_formats() {
    assert_eq!(Resource::Cpu.label(), "CPU");
    assert_eq!(Resource::Memory.label(), "Memory");
    assert_eq!(Resource::Cpu.fmt()(1500.0), "1.5 cores");
    assert_eq!(Resource::Memory.fmt()(2048.0), "2.0 KiB");
}

#[test]
fn resource_series_picks_cpu_or_memory() {
    let h = UsageHistory { cpu: vec![(1.0, 10.0)], memory: vec![(1.0, 2048.0), (2.0, 4096.0)] };
    assert_eq!(Resource::Cpu.series(&h), &[(1.0, 10.0)]);
    assert_eq!(Resource::Memory.series(&h).len(), 2);
}

#[test]
fn usage_header_against_the_limit() {
    let u = usage(Some(100.0), Some(500.0), Some(250.0), Some(4000.0));
    assert_eq!(usage_header(u, fmt_cpu), "now 250m · 50% of limit · req 100m · limit 500m");
}

#[test]
fn usage_header_warns_from_three_quarters() {
    let u = usage(None, Some(1000.0), Some(750.0), None);
    assert_eq!(usage_header(u, fmt_cpu), "⚠ now 750m · 75% of limit · req not set · limit 1 core");
    let u = usage(None, Some(1000.0), Some(749.0), None);
    assert!(!usage_header(u, fmt_cpu).starts_with('⚠'));
}

#[test]
fn usage_header_falls_back_to_the_node() {
    let u = usage(Some(100.0), None, Some(40.0), Some(4000.0));
    assert_eq!(usage_header(u, fmt_cpu), "now 40m · 1% of node · req 100m · limit unlimited (node 4 cores)");
    let u = usage(Some(100.0), None, Some(20.0), Some(4000.0));
    assert_eq!(usage_header(u, fmt_cpu), "now 20m · <1% of node · req 100m · limit unlimited (node 4 cores)");
}

#[test]
fn usage_header_without_numbers() {
    assert_eq!(usage_header(Usage::default(), fmt_cpu), "now — · req not set · limit unlimited");
    // Zero usage shows 0%, and a zero ceiling shows no share.
    let u = usage(None, Some(500.0), Some(0.0), None);
    assert_eq!(usage_header(u, fmt_cpu), "now 0m · 0% of limit · req not set · limit 500m");
    let u = usage(None, None, Some(5.0), Some(0.0));
    assert_eq!(usage_header(u, fmt_cpu), "now 5.0m · req not set · limit unlimited (node 0m)");
}
