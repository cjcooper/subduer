use std::path::PathBuf;

use super::*;
use crate::trace_file::{Batch, Bucket, ContainerKey, Event, EventKind, Owner, Resources, Source, Stat};
use crate::types::{TraceEnd, TraceScope};

pub(crate) const T0: i64 = 1_800_000_000 - 1_800_000_000 % 86_400;

pub(crate) fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("subduer-review-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn stat(v: f64) -> Option<Stat> {
    Some(Stat { min: v, avg: v, max: v })
}

fn key(kind: &str, name: &str, container: &str) -> TraceKey {
    TraceKey { namespace: "shop".into(), kind: kind.into(), name: name.into(), container: container.into() }
}

/// A trace of `web` (a Deployment with two pods, then a rollout) and `db`
/// (a StatefulSet that hits its memory limit), `minutes` long.
pub(crate) fn sample(dir: &std::path::Path, minutes: i64) -> PathBuf {
    let path = dir.join("t.trace");
    let mut file = TraceFile::create(&path, false, "acme-uat", &TraceScope::Namespaces(vec!["shop".into()]), T0 as f64).unwrap();
    let mut batch = Batch { source: Some(Source::Prometheus), ..Batch::default() };
    let web = |pod: &str| ContainerKey::new("shop", pod, "app");
    let owner = |kind: &str, name: &str, revision: Option<&str>| Owner { kind: kind.into(), name: name.into(), revision: revision.map(Into::into) };
    for (pod, rev) in [("web-a-1", "a"), ("web-a-2", "a"), ("web-b-1", "b")] {
        batch.owners.push((("shop".into(), pod.into()), owner("Deployment", "web", Some(rev))));
    }
    batch.owners.push((("shop".into(), "db-0".into()), owner("StatefulSet", "db", None)));
    let half = minutes / 2;
    for m in 0..minutes {
        let t = T0 + m * 60;
        // Two pods for the first half, one (after a rollout) for the rest; CPU 10m..109m.
        let pods: &[&str] = if m < half { &["web-a-1", "web-a-2"] } else { &["web-b-1"] };
        for pod in pods {
            let cpu = 10.0 + (m % 100) as f64;
            batch.usage.push((web(pod), Bucket { t, cpu: Some(Stat { min: cpu, avg: cpu, max: cpu + 50.0 }), memory: stat(100.0 * MIB), throttled: None }));
        }
        batch.usage.push((ContainerKey::new("shop", "db-0", "db"), Bucket { t, cpu: stat(500.0), memory: stat(950.0 * MIB), throttled: Some(0.2) }));
    }
    let r = |cpu: f64, mem: f64| Resources { cpu_request: Some(cpu), cpu_limit: None, mem_request: Some(mem * MIB), mem_limit: None };
    batch.resources.push((web("web-a-1"), T0, r(500.0, 256.0)));
    batch.resources.push((web("web-a-2"), T0, r(500.0, 256.0)));
    batch.resources.push((web("web-b-1"), T0 + half * 60, r(250.0, 256.0)));
    batch.resources.push((ContainerKey::new("shop", "db-0", "db"), T0, Resources { cpu_request: None, cpu_limit: Some(1000.0), mem_request: None, mem_limit: Some(1000.0 * MIB) }));
    let db = ContainerKey::new("shop", "db-0", "db");
    batch.events.push((db.clone(), Event { t: T0 + 600, kind: EventKind::Restart, reason: Some("OOMKilled".into()), count: 2 }));
    batch.events.push((db.clone(), Event { t: T0 + 660, kind: EventKind::Restart, reason: Some("Error".into()), count: 1 }));
    batch.events.push((db, Event { t: T0 + 600, kind: EventKind::Oom, reason: None, count: 1 }));
    // A finished pod with an owner and no usage gets no row.
    batch.owners.push((("shop".into(), "job-x".into()), owner("Job", "once", None)));
    batch.resources.push((ContainerKey::new("shop", "job-x", "run"), T0, r(100.0, 64.0)));
    file.write(&batch).unwrap();
    file.add_gap(T0 + 60, T0 + 120, "Prometheus no longer held this data").unwrap();
    file.finish((T0 + minutes * 60) as f64, TraceEnd::Stopped).unwrap();
    path
}

#[test]
fn rows_figures_flags_and_suggestions() {
    let dir = scratch("rows");
    let path = sample(&dir, 8 * 60);
    let r = review(&path, false).unwrap();
    assert_eq!(r.summary.end_reason, Some(TraceEnd::Stopped));
    let keys: Vec<&TraceKey> = r.rows.iter().map(|r| &r.key).collect();
    // Most spare CPU first; db has no CPU request, so it's last.
    assert_eq!(keys, vec![&key("Deployment", "web", "app"), &key("StatefulSet", "db", "db")]);

    let web = &r.rows[0];
    assert_eq!((web.pods, web.minutes, web.pod_minutes, web.rollouts), (3, 480, 720, 1));
    assert_eq!(web.avg_pods, 1.5);
    assert_eq!((web.cpu_request, web.mem_request), (Some(250.0), Some(256.0 * MIB)), "the latest requests");
    assert_eq!(web.cpu_max, 159.0);
    assert!(web.cpu_p50 < web.cpu_p95 && web.cpu_p95 <= web.cpu_p99 && web.cpu_p99 <= 109.0);
    assert_eq!(web.mem_max, 100.0 * MIB);
    assert_eq!(web.cpu_spare, Some((250.0 - web.cpu_p95) * 1.5));
    assert_eq!(web.mem_spare, Some(156.0 * MIB * 1.5));
    assert_eq!(web.local_share, 0.0);
    assert_eq!(web.flags, vec![ReviewFlag::RequestsChanged, ReviewFlag::ShortData { hours: 8.0 }]);
    let s = web.suggestion.as_ref().unwrap();
    assert_eq!(s.cpu_request, tidy_cpu(web.cpu_p95));
    assert_eq!(s.mem_request, 120.0 * MIB, "100Mi × 1.15 = 115Mi, up to a multiple of 8Mi");
    assert_eq!(s.basis, "p95 CPU and peak memory × 1.15 over 8h (720 pod-minutes), 1 rollout, no OOM kills");

    let db = &r.rows[1];
    assert_eq!((db.cpu_spare, db.mem_spare), (None, None));
    assert_eq!(db.flags, vec![
        ReviewFlag::Throttled { worst: 0.2 },
        ReviewFlag::NearMemoryLimit { share: 0.95 },
        ReviewFlag::OomKilled { count: 2 },
        ReviewFlag::OomEvents { count: 1 },
        ReviewFlag::Restarts { count: 1 },
        ReviewFlag::NoCpuRequest,
        ReviewFlag::NoMemoryRequest,
        ReviewFlag::ShortData { hours: 8.0 },
        ReviewFlag::AboveMemoryLimit,
    ]);
    assert!(db.suggestion.as_ref().unwrap().basis.ends_with("no rollouts, 3 OOM kills"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn little_data_means_no_suggestion() {
    let dir = scratch("little");
    let path = sample(&dir, 60);
    let r = review(&path, false).unwrap();
    assert!(r.rows.iter().all(|row| row.suggestion.is_none()));
    assert!(r.rows[0].flags.contains(&ReviewFlag::TooLittleData { hours: 1.0 }));
    // The one-minute gap is 1.7% of the hour: too small to mention.
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn series_buckets_requests_events_and_gaps() {
    let dir = scratch("series");
    let path = sample(&dir, 8 * 60);
    let web = series(&path, &key("Deployment", "web", "app"), None, None, 60).unwrap();
    assert_eq!((web.full_start, web.full_end), (T0 as f64, (T0 + 480 * 60) as f64));
    assert_eq!(web.step, 480.0, "8 minutes per bucket for 60 points over 8 hours");
    assert_eq!(web.cpu_avg.len(), 60);
    assert_eq!(web.pods[0].1, 2.0);
    assert_eq!(web.pods.last().unwrap().1, 1.0);
    assert_eq!(web.cpu_request, vec![(T0 as f64, 500.0), ((T0 + 240 * 60) as f64, 250.0)]);
    assert!(web.cpu_limit.is_empty());
    assert_eq!(web.gaps.len(), 1);
    assert!(web.events.is_empty());

    // Zoomed: one-minute buckets, clamped to the data.
    let db = series(&path, &key("StatefulSet", "db", "db"), Some((T0 + 300) as f64), Some((T0 + 900) as f64), 600).unwrap();
    assert_eq!((db.start, db.end, db.step), ((T0 + 300) as f64, (T0 + 900) as f64, 60.0));
    assert_eq!(db.cpu_max.len(), 10);
    assert_eq!(db.mem_limit, vec![(T0 as f64, 1000.0 * MIB)]);
    assert_eq!(db.events.iter().map(|e| (e.kind.as_str(), e.count)).collect::<Vec<_>>(), vec![("oom", 1), ("restart", 2), ("restart", 1)]);
    assert!(db.gaps.is_empty());
    let clamped = series(&path, &key("StatefulSet", "db", "db"), Some(0.0), Some(1e12), 10).unwrap();
    assert_eq!((clamped.start, clamped.end), (clamped.full_start, clamped.full_end));

    let missing = series(&path, &key("Deployment", "nope", "app"), None, None, 10).unwrap_err();
    assert_eq!(missing.to_string(), "Deployment nope isn't in this trace");
    let empty = series(&path, &key("Job", "once", "run"), None, None, 10).unwrap_err();
    assert_eq!(empty.to_string(), "no usage for Job once in this trace");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn daily_grid_by_local_hour() {
    let dir = scratch("daily");
    let path = sample(&dir, 26 * 60);
    let d = daily(&path, &key("StatefulSet", "db", "db"), &jiff::tz::TimeZone::UTC).unwrap();
    assert_eq!(d.days.len(), 2);
    assert_eq!(d.days[0].cpu.iter().filter(|c| c.is_some()).count(), 24);
    assert_eq!(d.days[1].cpu.iter().filter(|c| c.is_some()).count(), 2);
    assert_eq!(d.days[0].cpu[0], Some(500.0));
    assert_eq!(d.days[0].mem[0], Some(950.0 * MIB));
    assert_eq!(d.days[1].cpu[2], None);
    let label = jiff::Timestamp::from_second(T0).unwrap().to_zoned(jiff::tz::TimeZone::UTC).strftime("%a %-d %b").to_string();
    assert_eq!(d.days[0].label, label);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn rounding_and_text() {
    assert_eq!((tidy_cpu(0.4), tidy_cpu(11.8), tidy_cpu(101.0), tidy_cpu(1301.1)), (5.0, 15.0, 110.0, 1350.0));
    assert_eq!(tidy_memory(1.0), 16.0 * MIB);
    assert_eq!(tidy_memory(115.0 * MIB), 120.0 * MIB);
    assert_eq!(tidy_memory(300.0 * MIB), 304.0 * MIB);
    assert_eq!(tidy_memory(2557.0 * MIB), 2560.0 * MIB);
    assert_eq!(percentile(&[], 0.95), 0.0);
    assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0], 0.5), 2.0);
    assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0], 0.95), 4.0);
    assert_eq!((hours_text(2.1), hours_text(48.0), hours_text(98.5)), ("2h".into(), "2d".into(), "4d 2h".into()));
    assert_eq!((thousands(999), thousands(11_520), thousands(1_234_567)), ("999".into(), "11,520".into(), "1,234,567".into()));
    assert_eq!(bucket_for(3600, 600), 60);
    assert_eq!(bucket_for(7 * 86400, 600), 1020);
    assert_eq!(placeholders_from(3, 4), "?4, ?5, ?6");
}

#[test]
fn gaps_over_a_tenth_are_mentioned() {
    let dir = scratch("gaps");
    let path = sample(&dir, 8 * 60);
    {
        let file = TraceFile::open(&path).unwrap();
        drop(file);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute("INSERT INTO gaps VALUES (?1, ?2, 'x')", [T0, T0 + 3600]).unwrap();
    }
    let r = review(&path, false).unwrap();
    assert!(r.rows[0].suggestion.as_ref().unwrap().basis.ends_with(", 13% of the trace is gaps"), "{}", r.rows[0].suggestion.as_ref().unwrap().basis);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn notes_are_saved_in_the_file() {
    let dir = scratch("notes");
    let path = sample(&dir, 8 * 60);
    assert_eq!(review(&path, false).unwrap().note, None);
    crate::trace_file::set_note(&path, "", "  Peak season week  ", 1.0).unwrap();
    crate::trace_file::set_note(&path, &crate::trace_file::note_key("shop", "Deployment", "web"), "Agreed: 250m", 2.0).unwrap();
    let r = review(&path, false).unwrap();
    assert_eq!(r.note.as_deref(), Some("Peak season week"));
    assert_eq!(r.rows[0].note.as_deref(), Some("Agreed: 250m"));
    assert_eq!(r.rows[1].note, None);
    // Blank removes it.
    crate::trace_file::set_note(&path, "", " ", 3.0).unwrap();
    assert_eq!(review(&path, false).unwrap().note, None);
    let other = dir.join("other.txt");
    std::fs::write(&other, "x").unwrap();
    assert!(crate::trace_file::set_note(&other, "", "x", 1.0).is_err(), "only trace files");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn comparing_traces_lines_rows_up_oldest_first() {
    let (a, b) = (scratch("cmp-a"), scratch("cmp-b"));
    let (pa, pb) = (sample(&a, 8 * 60), sample(&b, 7 * 60));
    // A row only the second trace has.
    {
        let conn = rusqlite::Connection::open(&pb).unwrap();
        conn.execute("UPDATE pods SET workload_name = 'web2' WHERE name = 'web-b-1'", []).unwrap();
    }
    let c = compare(&[(pb.clone(), false), (pa.clone(), false)]).unwrap();
    assert_eq!(c.traces.len(), 2);
    let keys: Vec<String> = c.rows.iter().map(|r| r.key.name.clone()).collect();
    // By namespace, kind, name: the Deployments, then the StatefulSet.
    assert_eq!(keys, ["web", "web2", "db"]);
    let db = c.rows.iter().find(|r| r.key.name == "db").unwrap();
    assert!(db.cells.iter().all(Option::is_some));
    let first = db.cells[0].as_ref().unwrap();
    assert_eq!((first.ooms, first.throttled, first.cpu_request), (3, Some(0.2), None));
    let web2 = c.rows.iter().find(|r| r.key.name == "web2").unwrap();
    assert!(web2.cells.iter().filter(|c| c.is_some()).count() == 1);
    assert!(compare(&[(pa, false)]).unwrap_err().to_string().contains("at least two"));
    std::fs::remove_dir_all(&a).unwrap();
    std::fs::remove_dir_all(&b).unwrap();
}
