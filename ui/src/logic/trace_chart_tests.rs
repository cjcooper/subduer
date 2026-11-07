use super::*;
use subduer_types::TraceDay;

/// Mon 5 Oct 2026 00:00 UTC.
const MONDAY: f64 = 1_791_158_400.0;

fn series() -> TraceSeries {
    TraceSeries {
        start: 0.0,
        end: 3600.0,
        step: 60.0,
        cpu_avg: vec![(0.0, 100.0), (60.0, 200.0)],
        cpu_max: vec![(0.0, 150.0), (60.0, 300.0)],
        mem_avg: vec![],
        mem_max: vec![],
        pods: vec![(0.0, 2.0)],
        cpu_request: vec![(0.0, 500.0)],
        cpu_limit: vec![(0.0, 1000.0)],
        mem_request: vec![],
        mem_limit: vec![],
        events: vec![],
        gaps: vec![],
        full_start: 0.0,
        full_end: 3600.0,
    }
}

fn mark(kind: &str, reason: Option<&str>) -> TraceEventMark {
    TraceEventMark { t: MONDAY, kind: kind.into(), reason: reason.map(Into::into), count: 2 }
}

#[test]
fn measures() {
    assert_eq!((Measure::Memory.label(), Measure::Cpu.class()), ("Memory", "res-cpu"));
    assert_eq!(Measure::Memory.fmt(1536.0), "1.5 KiB");
    assert!(matches!((Measure::Cpu.scale(), Measure::Memory.scale()), (Scale::Decimal, Scale::Binary)));
    let s = series();
    assert_eq!(Measure::Cpu.pick(&s).3, &s.cpu_limit);
    assert_eq!(Measure::Memory.pick(&s).0, &s.mem_avg);
    assert_eq!((Measure::Cpu.opt(None), Measure::Cpu.opt(Some(15.0))), ("—".to_owned(), "15m".to_owned()));
}

#[test]
fn local_times() {
    let t = local_time(MONDAY + 14.0 * 3600.0 + 5.0 * 60.0 + 30.0, 0.0);
    assert_eq!(t, LocalTime { year: 2026, month: 10, date: 5, weekday: 1, hours: 14, minutes: 5 });
    assert_eq!(local_time(MONDAY, 7200.0).hours, 2, "two hours east");
    // The epoch was a Thursday; a second before it, Wednesday night.
    assert_eq!(local_time(0.0, 0.0), LocalTime { year: 1970, month: 1, date: 1, weekday: 4, hours: 0, minutes: 0 });
    assert_eq!(local_time(-1.0, 0.0), LocalTime { year: 1969, month: 12, date: 31, weekday: 3, hours: 23, minutes: 59 });
    // 29 Feb 2024 12:00 UTC.
    let leap = local_time(1_709_208_000.0, 0.0);
    assert_eq!((leap.year, leap.month, leap.date, leap.weekday), (2024, 2, 29, 4));
    assert_eq!(when(MONDAY + 9.0 * 3600.0, false, 0.0), "09:00");
    assert_eq!(when(MONDAY + 9.0 * 3600.0, true, -3600.0), "Mon 5 08:00");
}

#[test]
fn small_chart_texts() {
    assert_eq!(step_path(&[(0.0, 10.0, 5.0)], |t| t * 2.0, |v| 100.0 - v), "M0.0,95.00H20.0");
    assert_eq!(idle_text(Measure::Cpu, Some(250.0), Some(1000.0)), Some("750m (75% of requested)".into()));
    assert_eq!(idle_text(Measure::Cpu, None, Some(1000.0)), None);
    assert_eq!(idle_text(Measure::Cpu, Some(250.0), None), None);
    assert_eq!(idle_text(Measure::Cpu, Some(2000.0), Some(1000.0)), None);
    assert_eq!((count_text(2.0), count_text(1.54)), ("2".to_owned(), "1.5".to_owned()));
    assert_eq!((tip_side(70.0), tip_side(20.0)), ("right: 30.00%".to_owned(), "left: 20.00%".to_owned()));
    assert_eq!(left_style(5.0), "left: 5.00%");
    assert_eq!(top_style(5.0, 15), "top: calc(5.00% - 15px)");
    assert_eq!(dot_style(1.0, 2.0), "left: 1.00%; top: 2.00%");
    assert_eq!(span_style(1.0, 2.0), "left: 1.00%; width: 2.00%");
    assert_eq!(hour_label(6), Some("06".into()));
    assert_eq!(hour_label(7), None);
}

#[test]
fn pointer_drags_and_zooms() {
    assert_eq!(fraction(150.0, 100.0, 200.0), Some(0.25));
    assert_eq!(fraction(500.0, 100.0, 200.0), Some(1.0), "clamped");
    assert_eq!(fraction(150.0, 100.0, 0.0), None, "not laid out");
    assert_eq!(zoom_span(0.5, 0.2), Some((0.2, 0.5)));
    assert_eq!(zoom_span(0.5, 0.51), None, "a click, not a drag");
    assert_eq!(selection(Some((0.5, 0.25))), Some((25.0, 25.0)));
    assert_eq!(selection(Some((0.5, 0.501))), None);
    assert_eq!(selection(None), None);
    assert_eq!(time_at(&series(), 0.5), 1800.0);
}

#[test]
fn the_line_charts_frame() {
    let s = series();
    let c = chart(&s, Measure::Cpu).unwrap();
    let f = c.frame;
    assert_eq!((f.start, f.end, f.span, f.top), (0.0, 3600.0, 3600.0, 1200.0));
    assert_eq!((f.x(1800.0), f.x(-5.0), f.xp(3600.0)), (500.0, 0.0, 100.0));
    assert_eq!((f.y(0.0), f.y(1200.0), f.y(5000.0)), (92.0, 12.0, 12.0));
    assert_eq!(f.yp(0.0), 92.0);
    assert!(!f.long());
    assert!(Frame { span: 21.0 * 3600.0, ..f }.long());
    assert_eq!((f.grid(), f.ticks()), ([0.0, 600.0, 1200.0], [600.0, 1200.0]));
    assert_eq!((c.request_runs.clone(), c.limit_runs.clone()), (vec![(0.0, 3600.0, 500.0)], vec![(0.0, 3600.0, 1000.0)]));
    assert!(c.avg_line.starts_with("M8.3,"), "{}", c.avg_line);
    assert!(c.avg_area.ends_with('Z') && !c.peak_line.is_empty());
    assert_eq!(c.request_path, "M0.0,58.67H1000.0");
    assert_eq!(c.limit_path, "M0.0,25.33H1000.0");
    assert_eq!(ref_label_texts(&c, Names::of(true), Measure::Cpu), vec![("requested 500m".into(), 500.0, 0), ("limit 1 core".into(), 1000.0, 0)]);
    assert!(chart(&s, Measure::Memory).is_none(), "no memory data");
}

#[test]
fn markers_tooltips_and_summary() {
    let (restart, oom, killed, plain) = (mark("restart", Some("Error")), mark("oom", None), mark("restart", Some("OOMKilled")), mark("restart", None));
    assert!(shows_mark(Measure::Cpu, &restart) && !shows_mark(Measure::Cpu, &oom) && shows_mark(Measure::Memory, &oom));
    assert!(is_oom(&oom) && is_oom(&killed) && !is_oom(&restart) && !is_oom(&plain));
    assert_eq!(mark_text(&oom, "Mon 5 09:00"), "OOM kill ×2 at Mon 5 09:00");
    assert_eq!(mark_text(&restart, "x"), "Restart ×2 (Error) at x");
    assert_eq!(mark_text(&plain, "x"), "Restart ×2 at x");

    let s = series();
    assert_eq!(hover_bucket(&s.cpu_avg, &s.cpu_max, 90.0, 60.0), Some(60.0));
    assert_eq!(hover_bucket(&s.cpu_avg, &s.cpu_max, 500.0, 60.0), None, "past the last bucket");
    assert_eq!(hover_bucket(&s.cpu_avg, &s.cpu_max, -1.0, 60.0), None, "before the first");
    assert_eq!((value_at(&s.cpu_avg, 60.0), value_at(&s.cpu_avg, 30.0)), (Some(200.0), None));
    assert_eq!((highest(&s.cpu_max), highest(&vec![])), (300.0, 0.0));
    assert_eq!(
        summary(Measure::Cpu, Names::of(false), "a", "b", &s.cpu_avg, &s.cpu_max),
        "CPU from a to b: average pod up to 200m, busiest pod up to 300m"
    );
}

#[test]
fn the_day_and_hour_grid() {
    let day = |label: &str, cpu: f64, mem: Option<f64>| TraceDay { label: label.into(), cpu: vec![Some(cpu), None], mem: vec![mem, None] };
    let days = vec![day("Mon 12 Oct", 50.0, Some(1024.0)), day("Tue 13 Oct", 80.0, None)];
    assert_eq!(day_values(&days[0], Measure::Cpu), vec![Some(50.0), None]);
    assert_eq!(day_values(&days[0], Measure::Memory), vec![Some(1024.0), None]);
    assert_eq!((heat_max(&days, Measure::Cpu), heat_max(&days, Measure::Memory)), (80.0, 1024.0));
    assert_eq!((heat_what(Measure::Cpu), heat_what(Measure::Memory)), ("p95 CPU", "peak memory"));
    assert_eq!(heat_texts(Measure::Cpu, 80.0), ("p95 CPU by day and hour".into(), "High (80m)".into()));
    assert_eq!(heat_title("Mon 12 Oct", 9, Measure::Cpu, Some(50.0)), "Mon 12 Oct 09:00 · p95 CPU 50m");
    assert_eq!(heat_title("Mon 12 Oct", 10, Measure::Memory, None), "Mon 12 Oct 10:00 · no data");
}

#[test]
fn idle_is_what_is_requested_but_not_used() {
    assert_eq!(idle(250.0, 1000.0), Some((750.0, 0.75)));
    assert_eq!(idle(1000.0, 1000.0), None);
    assert_eq!(idle(1200.0, 1000.0), None, "over the request isn't idle");
    assert_eq!(idle(5.0, 0.0), None, "nothing requested");
    assert_eq!(share_text(0.75), "75%");
    // 7.2m used of 3 cores: not "100%".
    assert_eq!(share_text(idle(7.2, 3000.0).unwrap().1), "over 99%");
    assert_eq!(share_text(1.0), "100%");
}

#[test]
fn totals_name_their_lines() {
    assert_eq!(Names::of(true).avg, "used");
    assert_eq!(Names::of(false).count, "pods");
    assert_eq!((capitalised(Names::of(true).peak).as_str(), capitalised("").as_str()), ("Busiest minute", ""));
}

#[test]
fn step_series() {
    let changes = vec![(100.0, 500.0), (200.0, 250.0)];
    assert_eq!(step_at(&changes, 50.0), None);
    assert_eq!(step_at(&changes, 100.0), Some(500.0));
    assert_eq!(step_at(&changes, 199.0), Some(500.0));
    assert_eq!(step_at(&changes, 300.0), Some(250.0));
    assert_eq!(step_runs(&changes, 0.0, 300.0), vec![(100.0, 200.0, 500.0), (200.0, 300.0, 250.0)]);
    // Clipped to the window; runs outside it are dropped.
    assert_eq!(step_runs(&changes, 150.0, 180.0), vec![(150.0, 180.0, 500.0)]);
    assert_eq!(step_runs(&changes, 250.0, 300.0), vec![(250.0, 300.0, 250.0)]);
    assert!(step_runs(&vec![], 0.0, 10.0).is_empty());
}

#[test]
fn heat_levels() {
    assert_eq!((heat_level(0.0, 100.0), heat_level(10.0, 100.0), heat_level(50.0, 100.0), heat_level(100.0, 100.0)), (1, 1, 3, 5));
    assert_eq!(heat_level(5.0, 0.0), 1);
    assert_eq!((Measure::Cpu.label(), Measure::Memory.class()), ("CPU", "res-memory"));
    assert_eq!(Measure::Cpu.fmt(1500.0), "1.5 cores");
}

#[test]
fn paths_break_on_missing_buckets() {
    let pts = vec![(0.0, 1.0), (60.0, 2.0), (240.0, 3.0)];
    let (x, y) = (|t: f64| t, |v: f64| 100.0 - v);
    let (line, area) = paths(&pts, 60.0, x, y, 92.0);
    assert_eq!(line.matches('M').count(), 2);
    assert_eq!(area.matches('Z').count(), 2);
    assert!(line.starts_with("M30.0,99.00L90.0,98.00M270.0,97.00"), "{line}");
    assert_eq!(paths(&vec![], 60.0, x, y, 92.0), (String::new(), String::new()));
}

#[test]
fn reference_labels_never_overlap() {
    assert_eq!(ref_labels(Some(6.0), Some(6.0), 8.0), vec![("request = limit {}", 6.0, 0)]);
    assert_eq!(ref_labels(Some(5.8), Some(6.0), 8.0), vec![("limit {}", 6.0, 0), ("request {}", 5.8, 18)]);
    assert_eq!(ref_labels(Some(6.0), Some(5.8), 8.0), vec![("request {}", 6.0, 0), ("limit {}", 5.8, 18)]);
    assert_eq!(ref_labels(Some(2.0), Some(6.0), 8.0), vec![("request {}", 2.0, 0), ("limit {}", 6.0, 0)]);
    assert_eq!(ref_labels(None, Some(6.0), 8.0), vec![("limit {}", 6.0, 0)]);
    assert!(ref_labels(None, None, 8.0).is_empty());
}
