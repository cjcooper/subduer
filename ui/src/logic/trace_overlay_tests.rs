use super::*;

/// Mon 5 Oct 2026 00:00 UTC.
const MONDAY: f64 = 1_791_158_400.0;

fn line(n: usize, slot: usize, start: f64, avg: Vec<(f64, f64)>, request: Vec<(f64, f64)>) -> Line {
    let series = TraceSeries {
        start,
        end: start + 600.0,
        step: 60.0,
        cpu_avg: avg,
        cpu_max: vec![],
        mem_avg: vec![],
        mem_max: vec![],
        pods: vec![],
        cpu_request: request,
        cpu_limit: vec![],
        mem_request: vec![],
        mem_limit: vec![],
        events: vec![],
        gaps: vec![],
        full_start: start,
        full_end: start + 600.0,
    };
    Line { n, slot, series, shift: start }
}

#[test]
fn texts_and_colours() {
    assert_eq!(trace_var(0), "--trace: var(--trace-1)");
    assert_eq!(axis_label(Align::SinceStart, 0.0, 0.0), "start");
    assert_eq!(axis_label(Align::SinceStart, 5400.0, 0.0), "+1h 30m");
    assert_eq!(axis_label(Align::TimeOfWeek, MONDAY + 9.0 * 3600.0, 3600.0), "Mon 10:00");
    assert_eq!(summary(Measure::Cpu, 2, Align::SinceStart), "CPU of 2 traces, by time since each started");
    assert_eq!(summary(Measure::Memory, 3, Align::TimeOfWeek), "Memory of 3 traces, by time of week");
}

#[test]
fn the_overlays_frame_and_lines() {
    let a = line(1, 0, 1000.0, vec![(1000.0, 100.0), (1060.0, 200.0)], vec![(1000.0, 500.0)]);
    let b = line(2, 1, 5000.0, vec![(5000.0, 300.0)], vec![]);
    let empty = line(3, 2, 0.0, vec![], vec![]);
    let lines = drawn(&[a.clone(), empty, b.clone()], Measure::Cpu);
    assert_eq!(lines, vec![a.clone(), b.clone()], "lines without data aren't drawn");
    assert!(Frame::of(&[], Measure::Cpu).is_none());

    let f = Frame::of(&lines, Measure::Cpu).unwrap();
    // Both start at 0 on the shared axis and span 600 s; the top fits 500m.
    assert_eq!((f.lo, f.hi, f.span, f.top), (0.0, 600.0, 600.0, 600.0));
    assert_eq!((f.x(300.0), f.x(-1.0), f.xp(600.0), f.at(0.5)), (500.0, 0.0, 100.0, 300.0));
    assert_eq!((f.y(0.0), f.y(600.0), f.yp(0.0)), (100.0, 8.0, 100.0));
    assert_eq!((f.grid(), f.ticks()), ([0.0, 300.0, 600.0], [300.0, 600.0]));
    let (path, runs) = f.line_paths(&a, Measure::Cpu);
    assert!(path.starts_with("M50.0,"), "{path}");
    assert_eq!(runs, "M0.0,23.33H1000.0");
    assert_eq!(f.ends(&lines, Measure::Cpu), vec![
        (1, "--trace: var(--trace-1); left: 20.00%; top: calc(69.33% + 0px)".to_owned()),
        (2, "--trace: var(--trace-2); left: 10.00%; top: calc(54.00% + 0px)".to_owned()),
    ]);

    assert_eq!(hover_rows(&lines, Measure::Cpu, 70.0), vec![
        (0, "Trace 1: 200m".to_owned(), Some(" · request 500m".to_owned())),
    ]);
    assert_eq!(hover_rows(&lines, Measure::Cpu, 10.0), vec![
        (0, "Trace 1: 100m".to_owned(), Some(" · request 500m".to_owned())),
        (1, "Trace 2: 300m".to_owned(), None),
    ]);
    assert!(hover_rows(&lines, Measure::Cpu, 500.0).is_empty());
}

#[test]
fn at_most_three_traces_first_and_last_two() {
    assert_eq!(picked(2), vec![0, 1]);
    assert_eq!(picked(3), vec![0, 1, 2]);
    assert_eq!(picked(5), vec![0, 3, 4]);
}

#[test]
fn a_trace_keeps_its_colour_on_every_row() {
    assert_eq!((trace_slot(0, 3), trace_slot(2, 3)), (Some(0), Some(2)));
    // Of five, the first and the last two are overlaid; the others have no colour.
    assert_eq!((trace_slot(3, 5), trace_slot(4, 5), trace_slot(1, 5)), (Some(1), Some(2), None));
}

#[test]
fn weeks_start_on_monday() {
    assert_eq!(week_of(MONDAY, 0.0), week_of(MONDAY + WEEK - 1.0, 0.0));
    assert_eq!(week_of(MONDAY - 1.0, 0.0), week_of(MONDAY, 0.0) - 1.0, "Sunday night is the week before");
    // Monday 01:00 in Paris (UTC+2) is still Sunday in UTC.
    assert_eq!(week_of(MONDAY - 3600.0, 7200.0), week_of(MONDAY, 0.0));
}

#[test]
fn shifts_line_traces_up() {
    let first = MONDAY + 9.0 * 3600.0;
    // Since start: each trace from its own start.
    assert_eq!(shift(Align::SinceStart, first + 3.0 * 86400.0, 0.0, first, 0.0), first + 3.0 * 86400.0);
    // Time of week: a trace started on a Wednesday two weeks later moves back two weeks,
    // so its Wednesday sits over the first trace's Wednesday.
    let later = first + 2.0 * WEEK + 2.0 * 86400.0;
    assert_eq!(shift(Align::TimeOfWeek, later, 0.0, first, 0.0), 2.0 * WEEK);
    assert_eq!(shift(Align::TimeOfWeek, first + 3600.0, 0.0, first, 0.0), 0.0, "same week");
}

#[test]
fn end_labels_move_apart() {
    // Two lines ending on the same spot, a third close to them, and one far away.
    assert_eq!(end_offsets(&[(99.0, 80.0), (99.0, 80.0), (98.0, 84.0), (50.0, 80.0)]), vec![0, 14, 28, 0]);
    assert_eq!(end_offsets(&[(99.0, 20.0), (99.0, 80.0)]), vec![0, 0]);
}

#[test]
fn values_are_read_from_the_bucket_under_the_pointer() {
    let points = vec![(100.0, 1.0), (160.0, 2.0)];
    assert_eq!(value_at(&points, 60.0, 0.0, 130.0), Some((100.0, 1.0)));
    assert_eq!(value_at(&points, 60.0, 100.0, 70.0), Some((160.0, 2.0)), "shifted onto the shared axis");
    assert_eq!(value_at(&points, 60.0, 0.0, 230.0), None);
}
