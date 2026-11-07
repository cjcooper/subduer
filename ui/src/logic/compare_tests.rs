use super::*;
use subduer_types::{TraceKey, TraceScope};

fn summary(path: &str, started: f64, data_start: Option<f64>) -> TraceSummary {
    TraceSummary {
        path: path.into(),
        context: "acme-uat".into(),
        scope: TraceScope::Cluster,
        started,
        ended: None,
        end_reason: None,
        unfinished: false,
        data_start,
        data_end: data_start.map(|s| s + 7200.0),
        prometheus: None,
        retention_secs: None,
        pods: 0,
        containers: 0,
        prometheus_samples: 0,
        local_samples: 0,
        gaps: vec![],
        file_bytes: 0,
        pull_every: None,
        pulled_bytes: 0,
    }
}

fn series(start: f64) -> TraceSeries {
    TraceSeries {
        start,
        end: start + 60.0,
        step: 60.0,
        cpu_avg: vec![(start, 1.0)],
        cpu_max: vec![],
        mem_avg: vec![],
        mem_max: vec![],
        pods: vec![],
        cpu_request: vec![],
        cpu_limit: vec![],
        mem_request: vec![],
        mem_limit: vec![],
        events: vec![],
        gaps: vec![],
        full_start: start,
        full_end: start + 60.0,
    }
}

#[test]
fn cells_and_headings() {
    let mut c = cell(Some(1500.0), None).unwrap();
    c.avg_pods = 1.5;
    assert_eq!(cell_lines(&c), ("1.5 cores · p95 10m".into(), "— · peak 100 B".into(), "1.5 pods · 1d 0h".into()));
    assert_eq!(cell_flags(&c), (None, None));
    c.ooms = 2;
    c.throttled = Some(0.25);
    assert_eq!(cell_flags(&c), (Some("OOM ×2".into()), Some("Throttled 25%".into())));
    let (a, b) = (cell(Some(1500.0), Some(1024.0)).unwrap(), cell(Some(15.0), Some(1024.0)).unwrap());
    assert_eq!(change_lines(&a, &b), ["1.5 cores → 15m (−99%)".to_owned(), "10m".into(), "1.0 KiB".into(), "100 B".into()]);

    let r = row("web", vec![cell(None, None), None, cell(None, None)]);
    assert_eq!(only_text(&r, 3), Some("Only in trace 1, 3".into()));
    assert_eq!(only_text(&row("web", vec![cell(None, None)]), 1), None);

    let t = summary("/x/a.trace", 100.0, Some(0.0));
    assert_eq!(head_text(&t, "5/10/2026"), "5/10/2026 · 2h 0m · acme-uat");
    assert_eq!(legend_text(1, &t, "5/10/2026"), "Trace 2 · a.trace · 5/10/2026");
    assert_eq!((data_start(&t), data_start(&summary("b", 100.0, None))), (0.0, 100.0));
    assert_eq!(count_text_of(2, 5), "2 of 5 rows · select one to overlay its charts");
    assert_eq!(title(Some(&vec!["a".into(), "b".into()])), "Comparing 2 traces");
    assert_eq!(title(None), "Comparing 0 traces");
    assert_eq!(change_heading(3), "Change, 1 → 3");
}

#[test]
fn sorting_and_picking_rows() {
    let rows = vec![row("web", vec![cell(Some(500.0), None)]), row("api", vec![cell(Some(100.0), None)])];
    let names = |rows: Vec<CompareRow>| rows.into_iter().map(|r| r.key.name).collect::<Vec<_>>();
    assert_eq!(names(sorted(rows.clone(), Some((0, false)), 1)), ["api", "web"]);
    assert_eq!(names(sorted(rows.clone(), Some((1, true)), 1)), ["web", "api"]);
    assert_eq!(names(sorted(rows, None, 1)), ["web", "api"], "unsorted keeps the order");

    let mut sel = None;
    pick(&mut sel, 1);
    assert_eq!(sel, Some(1));
    pick(&mut sel, 2);
    assert_eq!(sel, Some(2));
    pick(&mut sel, 2);
    assert_eq!(sel, None, "a second click closes it");
}

#[test]
fn the_overlays_lines() {
    assert_eq!(shown_text(&[0, 1], 2), None);
    assert_eq!(shown_text(&[0, 3, 4], 5), Some("Showing traces 1, 4 and 5 of 5: an overlay holds 3 at most.".into()));
    assert_eq!(overlaid(5, &[true, true, true, false, true]), vec![0, 4], "trace 4 doesn't have the row");
    assert_eq!(overlaid(2, &[true]), vec![0]);

    let week = 7.0 * 86400.0;
    let starts = vec![(0.0, 0.0), (week, 0.0), (2.0 * week, 0.0), (3.0 * week, 0.0)];
    let loaded = vec![(3, Ok(series(3.0 * week))), (0, Ok(series(0.0))), (1, Err("gone".into())), (2, Ok(series(2.0 * week)))];
    let since = lines(&loaded, &starts, (0.0, 0.0), Align::SinceStart);
    assert_eq!(since.iter().map(|l| (l.n, l.slot, l.shift)).collect::<Vec<_>>(), [(1, 0, 0.0), (3, 1, 2.0 * week), (4, 2, 3.0 * week)]);
    let weekly = lines(&loaded, &starts, (0.0, 0.0), Align::TimeOfWeek);
    assert_eq!(weekly[2].shift, 3.0 * week);
    assert_eq!(errors(&loaded), "Trace 2: gone");
    assert_eq!(errors(&loaded[..1]), "");
}

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
