use super::*;
use subduer_types::TraceKey;

fn cell(cpu_req: Option<f64>, mem_req: Option<f64>) -> Option<CompareCell> {
    Some(CompareCell { cpu_request: cpu_req, cpu_p95: 10.0, cpu_max: 20.0, mem_request: mem_req, mem_max: 100.0, avg_pods: 2.0, ooms: 0, throttled: None, hours: 24.0 })
}

fn row(name: &str, cells: Vec<Option<CompareCell>>) -> CompareRow {
    CompareRow { key: TraceKey { namespace: "shop".into(), kind: "Deployment".into(), name: name.into(), container: "app".into() }, cells }
}

#[test]
fn changes_and_filters() {
    let rows = vec![
        row("web", vec![cell(Some(1500.0), Some(6e9)), None, cell(Some(15.0), Some(6e9))]),
        row("db", vec![cell(None, None), cell(None, None)]),
        row("api", vec![None, cell(Some(100.0), None)]),
    ];
    assert!(requests_changed(&rows[0]));
    assert!(!requests_changed(&rows[1]));
    assert!(!requests_changed(&rows[2]), "one trace: nothing to compare");
    assert_eq!(filter(&rows, "", true).iter().map(|r| r.key.name.as_str()).collect::<Vec<_>>(), ["web"]);
    assert_eq!(filter(&rows, " API", false).len(), 1);
    let (a, b) = ends(&rows[0]).unwrap();
    assert_eq!((a.cpu_request, b.cpu_request), (Some(1500.0), Some(15.0)));
    let (a, b) = ends(&rows[2]).unwrap();
    assert_eq!(a, b);
    assert!(ends(&row("x", vec![None, None])).is_none());
}

#[test]
fn from_to_texts() {
    assert_eq!(from_to(Some(1500.0), Some(15.0), Measure::Cpu), "1.5 cores → 15m (−99%)");
    assert_eq!(from_to(Some(15.0), Some(15.0), Measure::Cpu), "15m");
    assert_eq!(from_to(None, Some(15.0), Measure::Cpu), "not set → 15m");
    assert_eq!(from_to(Some(15.0), None, Measure::Cpu), "15m → not set");
    assert_eq!(from_to(None, None, Measure::Cpu), "not set");
}

#[test]
fn compare_rows_sort_by_trace_and_change() {
    use crate::sort::Key;
    assert_eq!(headers(2), ["Workload", "Trace 1", "Trace 2", "Change"]);
    let r = row("web", vec![cell(Some(500.0), None), None, cell(Some(250.0), None)]);
    assert_eq!(row_key(&r, 0, 3), Key::Text("web app shop".into()));
    assert_eq!(row_key(&r, 1, 3), Key::Num(500.0));
    assert_eq!(row_key(&r, 2, 3), Key::Missing, "not in that trace");
    assert_eq!(row_key(&r, 4, 3), Key::Num(-0.5), "request halved");
    let mem = row("db", vec![cell(None, Some(100.0)), cell(None, Some(150.0))]);
    assert_eq!(row_key(&mem, 1, 2), Key::Num(10.0), "no request: its p95");
    assert_eq!(row_key(&mem, 3, 2), Key::Num(0.5), "memory when CPU has no request");
    assert_eq!(row_key(&row("x", vec![None, None]), 3, 2), Key::Missing);
}
