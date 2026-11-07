//! Reading a trace for review: the right-sizing table (one row per workload
//! container), a zoomable series for one row, and its usage by day and hour.
//! Works on the file alone; no cluster needed.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use rusqlite::{Connection, Row, params_from_iter};

use crate::trace_file::{BUCKET_SECS, TraceFile};
use crate::types::{
    CompareCell, CompareRow, ReviewFlag, TraceComparison, ReviewRow, Series, Suggestion, TraceDaily, TraceDay, TraceEventMark, TraceGap, TraceKey, TraceReview,
    TraceSeries,
};
use crate::{Error, Result};

/// Below this, no suggestion.
const MIN_HOURS: f64 = 6.0;
/// Below this, a suggestion with a caution: daily peaks may be missing.
const DAY_HOURS: f64 = 24.0;
/// Memory request = peak × this.
pub const MEMORY_HEADROOM: f64 = 1.15;
/// Peak memory at or above this share of the limit is flagged.
const NEAR_LIMIT: f64 = 0.9;
/// A worst minute throttled at least this much is flagged.
const THROTTLED: f64 = 0.1;
/// Most days shown by the day-and-hour grid (the latest ones).
const MAX_DAYS: usize = 31;
const MIB: f64 = 1024.0 * 1024.0;

/// The containers of each row, and what the pods say about it.
struct Group {
    containers: Vec<i64>,
    pods: BTreeSet<String>,
    revisions: BTreeSet<String>,
}

/// Every row's container ids, keyed and sorted.
fn groups(conn: &Connection) -> Result<BTreeMap<TraceKey, Group>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, p.namespace, COALESCE(p.workload_kind, 'Pod'), COALESCE(p.workload_name, p.name), c.name, p.name, p.revision
         FROM containers c JOIN pods p ON p.id = c.pod_id",
    )?;
    let mut out: BTreeMap<TraceKey, Group> = BTreeMap::new();
    let read = |r: &Row<'_>| -> rusqlite::Result<(i64, TraceKey, String, Option<String>)> {
        Ok((
            r.get(0)?,
            TraceKey { namespace: r.get(1)?, kind: r.get(2)?, name: r.get(3)?, container: r.get(4)? },
            r.get(5)?,
            r.get(6)?,
        ))
    };
    let rows = stmt.query_map([], read)?;
    for row in rows {
        let (id, key, pod, revision) = row?;
        let g = out.entry(key).or_insert_with(|| Group { containers: vec![], pods: BTreeSet::new(), revisions: BTreeSet::new() });
        g.containers.push(id);
        g.pods.insert(pod);
        g.revisions.extend(revision);
    }
    Ok(out)
}

/// Opens a trace and builds its review. `recording` says whether this app
/// is still writing it.
pub fn review(path: &Path, recording: bool) -> Result<TraceReview> {
    let file = TraceFile::open(path)?;
    let summary = file.summary(recording)?;
    let span = match (summary.data_start, summary.data_end) {
        (Some(a), Some(b)) => b - a,
        _ => 0.0,
    };
    let gap_secs: f64 = summary.gaps.iter().map(|g| g.end - g.start).sum();
    let conn = file.conn();
    let notes = file.notes()?;
    let mut rows = Vec::new();
    for (key, group) in groups(conn)? {
        if let Some(mut row) = review_row(conn, key, &group, span, gap_secs)? {
            row.note = notes.get(&crate::trace_file::note_key(&row.key.namespace, &row.key.kind, &row.key.name)).cloned();
            rows.push(row);
        }
    }
    // Most CPU requested but unused first; rows without a request after.
    rows.sort_by(|a, b| b.cpu_spare.unwrap_or(f64::MIN).total_cmp(&a.cpu_spare.unwrap_or(f64::MIN)));
    Ok(TraceReview { summary, rows, note: notes.get("").cloned() })
}

/// Per-container figures gathered from the file.
#[derive(Default)]
struct Samples {
    /// Millicores: each pod-minute's average, and the highest any pod reached.
    cpu_avg: Vec<f64>,
    cpu_max: f64,
    /// Bytes: each pod-minute's peak.
    mem_max: Vec<f64>,
    throttled: Vec<f64>,
    minutes: BTreeSet<i64>,
    local: u64,
}

fn review_row(conn: &Connection, key: TraceKey, group: &Group, span: f64, gap_secs: f64) -> Result<Option<ReviewRow>> {
    let mut s = Samples::default();
    let mut usage = conn.prepare_cached("SELECT t, cpu_avg, cpu_max, mem_max, throttled, source FROM usage WHERE container_id = ?1")?;
    type UsageRow = (i64, Option<i64>, Option<i64>, Option<i64>, Option<f64>, i64);
    let read = |r: &Row<'_>| -> rusqlite::Result<UsageRow> { Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)) };
    for id in &group.containers {
        let rows = usage.query_map([id], read)?;
        for row in rows {
            let (t, cpu_avg, cpu_max, mem_max, throttled, source) = row?;
            s.minutes.insert(t);
            if let Some(v) = cpu_avg {
                s.cpu_avg.push(v as f64 / 1000.0);
            }
            if let Some(v) = cpu_max {
                s.cpu_max = s.cpu_max.max(v as f64 / 1000.0);
            }
            if let Some(v) = mem_max {
                s.mem_max.push(v as f64);
            }
            s.throttled.extend(throttled);
            s.local += u64::from(source == 1);
        }
    }
    // Pods kube-state-metrics still lists after they finished have no usage.
    let pod_minutes = s.cpu_avg.len().max(s.mem_max.len()) as u64;
    if pod_minutes == 0 {
        return Ok(None);
    }

    // Requests and limits: the latest, and whether they changed.
    let mut latest: Option<(i64, [Option<f64>; 4])> = None;
    let mut seen: BTreeSet<[Option<u64>; 4]> = BTreeSet::new();
    let mut resources = conn.prepare_cached(
        "SELECT t, cpu_request, cpu_limit, mem_request, mem_limit FROM resources WHERE container_id = ?1",
    )?;
    let read = |r: &Row<'_>| -> rusqlite::Result<(i64, [Option<f64>; 4])> {
        Ok((r.get(0)?, [r.get(1)?, r.get(2)?, r.get::<_, Option<i64>>(3)?.map(|v| v as f64), r.get::<_, Option<i64>>(4)?.map(|v| v as f64)]))
    };
    for id in &group.containers {
        let rows = resources.query_map([id], read)?;
        for row in rows {
            let (t, values): (i64, [Option<f64>; 4]) = row?;
            seen.insert(values.map(|v| v.map(f64::to_bits)));
            if latest.is_none_or(|(lt, _)| t >= lt) {
                latest = Some((t, values));
            }
        }
    }
    let [cpu_request, cpu_limit, mem_request, mem_limit] = latest.map(|(_, v)| v).unwrap_or_default();

    // Restarts (by last termination reason) and OOM events.
    let (mut oom_killed, mut restarts, mut oom_events) = (0u32, 0u32, 0u32);
    let mut events = conn.prepare_cached("SELECT kind, reason, SUM(count) FROM events WHERE container_id = ?1 GROUP BY kind, reason")?;
    for id in &group.containers {
        let rows = events.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, i64>(2)?)))?;
        for row in rows {
            let (kind, reason, count) = row?;
            let count = count.max(0) as u32;
            match (kind.as_str(), reason.as_deref()) {
                ("restart", Some("OOMKilled")) => oom_killed += count,
                ("restart", _) => restarts += count,
                _ => oom_events += count,
            }
        }
    }

    let minutes = s.minutes.len() as u64;
    let avg_pods = pod_minutes as f64 / minutes.max(1) as f64;
    s.cpu_avg.sort_by(f64::total_cmp);
    s.mem_max.sort_by(f64::total_cmp);
    s.throttled.sort_by(f64::total_cmp);
    let cpu_p95 = percentile(&s.cpu_avg, 0.95);
    let mem_max = s.mem_max.last().copied().unwrap_or(0.0);
    let hours = minutes as f64 * BUCKET_SECS as f64 / 3600.0;
    let rollouts = group.revisions.len().saturating_sub(1) as u32;

    let mut flags = Vec::new();
    if let Some(&worst) = s.throttled.last()
        && worst >= THROTTLED
    {
        flags.push(ReviewFlag::Throttled { worst });
    }
    if let Some(limit) = mem_limit
        && limit > 0.0
        && mem_max >= NEAR_LIMIT * limit
    {
        flags.push(ReviewFlag::NearMemoryLimit { share: mem_max / limit });
    }
    for (count, flag) in [
        (oom_killed, ReviewFlag::OomKilled { count: oom_killed }),
        (oom_events, ReviewFlag::OomEvents { count: oom_events }),
        (restarts, ReviewFlag::Restarts { count: restarts }),
    ] {
        if count > 0 {
            flags.push(flag);
        }
    }
    if cpu_request.is_none() {
        flags.push(ReviewFlag::NoCpuRequest);
    }
    if mem_request.is_none() {
        flags.push(ReviewFlag::NoMemoryRequest);
    }
    if seen.len() > 1 {
        flags.push(ReviewFlag::RequestsChanged);
    }

    let suggestion = if hours < MIN_HOURS {
        flags.push(ReviewFlag::TooLittleData { hours });
        None
    } else {
        if hours < DAY_HOURS {
            flags.push(ReviewFlag::ShortData { hours });
        }
        let mem = tidy_memory(mem_max * MEMORY_HEADROOM);
        if mem_limit.is_some_and(|l| mem > l) {
            flags.push(ReviewFlag::AboveMemoryLimit);
        }
        let gaps = (span > 0.0 && gap_secs / span >= 0.1).then(|| format!(", {:.0}% of the trace is gaps", gap_secs / span * 100.0));
        let ooms = match oom_killed + oom_events {
            0 => "no OOM kills".to_owned(),
            n => format!("{n} OOM kill{}", if n == 1 { "" } else { "s" }),
        };
        let basis = format!(
            "p95 CPU and peak memory × {MEMORY_HEADROOM} over {} ({} pod-minutes), {}, {ooms}{}",
            hours_text(hours),
            thousands(pod_minutes),
            match rollouts {
                0 => "no rollouts".to_owned(),
                1 => "1 rollout".to_owned(),
                n => format!("{n} rollouts"),
            },
            gaps.unwrap_or_default(),
        );
        Some(Suggestion { cpu_request: tidy_cpu(cpu_p95), mem_request: mem, basis })
    };

    Ok(Some(ReviewRow {
        key,
        pods: group.pods.len() as u32,
        avg_pods,
        minutes,
        pod_minutes,
        rollouts,
        local_share: s.local as f64 / pod_minutes as f64,
        cpu_request,
        cpu_limit,
        mem_request,
        mem_limit,
        cpu_p50: percentile(&s.cpu_avg, 0.5),
        cpu_p95,
        cpu_p99: percentile(&s.cpu_avg, 0.99),
        cpu_max: s.cpu_max,
        mem_p95: percentile(&s.mem_max, 0.95),
        mem_max,
        cpu_spare: cpu_request.map(|r| (r - cpu_p95) * avg_pods),
        mem_spare: mem_request.map(|r| (r - mem_max) * avg_pods),
        flags,
        suggestion,
        note: None,
    }))
}

/// Several traces side by side, oldest data first, rows matched by
/// namespace, workload and container. Each is `(path, being recorded)`.
pub fn compare(files: &[(std::path::PathBuf, bool)]) -> Result<TraceComparison> {
    if files.len() < 2 {
        return Err(Error::Message("choose at least two traces to compare".into()));
    }
    let mut reviews = files.iter().map(|(p, rec)| review(p, *rec)).collect::<Result<Vec<_>>>()?;
    reviews.sort_by(|a, b| a.summary.data_start.unwrap_or(a.summary.started).total_cmp(&b.summary.data_start.unwrap_or(b.summary.started)));
    let mut rows: BTreeMap<TraceKey, Vec<Option<CompareCell>>> = BTreeMap::new();
    for (i, r) in reviews.iter().enumerate() {
        for row in &r.rows {
            let ooms = row
                .flags
                .iter()
                .map(|f| match f {
                    ReviewFlag::OomKilled { count } | ReviewFlag::OomEvents { count } => *count,
                    _ => 0,
                })
                .sum();
            let throttled = row.flags.iter().find_map(|f| match f {
                ReviewFlag::Throttled { worst } => Some(*worst),
                _ => None,
            });
            let cells = rows.entry(row.key.clone()).or_insert_with(|| vec![None; reviews.len()]);
            cells[i] = Some(CompareCell {
                cpu_request: row.cpu_request,
                cpu_p95: row.cpu_p95,
                cpu_max: row.cpu_max,
                mem_request: row.mem_request,
                mem_max: row.mem_max,
                avg_pods: row.avg_pods,
                ooms,
                throttled,
                hours: row.minutes as f64 / 60.0,
            });
        }
    }
    Ok(TraceComparison {
        traces: reviews.into_iter().map(|r| r.summary).collect(),
        rows: rows.into_iter().map(|(key, cells)| CompareRow { key, cells }).collect(),
    })
}

/// Nearest-rank percentile of sorted values; 0 when empty.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// Millicores rounded up: to 5m under 100m, 10m under 1 core, 50m above. At least 5m.
pub fn tidy_cpu(m: f64) -> f64 {
    let step = if m < 100.0 { 5.0 } else if m < 1000.0 { 10.0 } else { 50.0 };
    ((m / step).ceil() * step).max(5.0)
}

/// Bytes rounded up to whole MiB: to 8 under 256Mi, 16 under 1Gi, 64 above. At least 16Mi.
pub fn tidy_memory(bytes: f64) -> f64 {
    let mib = bytes / MIB;
    let step = if mib < 256.0 { 8.0 } else if mib < 1024.0 { 16.0 } else { 64.0 };
    ((mib / step).ceil() * step).max(16.0) * MIB
}

fn hours_text(hours: f64) -> String {
    let (d, h) = ((hours / 24.0).floor(), (hours % 24.0).floor());
    match (d as u64, h as u64) {
        (0, h) => format!("{h}h"),
        (d, 0) => format!("{d}d"),
        (d, h) => format!("{d}d {h}h"),
    }
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// One row's usage between `start` and `end` (its whole span if `None`), in
/// about `points` buckets of whole minutes.
pub fn series(path: &Path, key: &TraceKey, start: Option<f64>, end: Option<f64>, points: u32) -> Result<TraceSeries> {
    let file = TraceFile::open(path)?;
    let conn = file.conn();
    let ids = container_ids(conn, key)?;
    let list = placeholders(ids.len());
    let (full_start, full_end): (Option<i64>, Option<i64>) = conn.query_row(
        &format!("SELECT MIN(t), MAX(t) FROM usage WHERE container_id IN ({list})"),
        params_from_iter(&ids),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (Some(full_start), Some(full_end)) = (full_start, full_end.map(|t| t + BUCKET_SECS)) else {
        return Err(Error::Message(format!("no usage for {} {} in this trace", key.kind, key.name)));
    };
    let start = start.map_or(full_start, |s| s as i64).clamp(full_start, full_end);
    let end = end.map_or(full_end, |e| e as i64).clamp(start + BUCKET_SECS, full_end.max(start + BUCKET_SECS));
    let step = bucket_for(end - start, points);

    let mut out = TraceSeries {
        start: start as f64,
        end: end as f64,
        step: step as f64,
        cpu_avg: vec![],
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
        full_start: full_start as f64,
        full_end: full_end as f64,
    };
    let mut args: Vec<i64> = vec![step, start, end];
    args.extend(&ids);
    let mut stmt = conn.prepare(&format!(
        "SELECT (t / ?1) * ?1 AS b, AVG(cpu_avg), MAX(cpu_max), AVG(mem_avg), MAX(mem_max), COUNT(*)
         FROM usage WHERE t >= ?2 AND t < ?3 AND container_id IN ({}) GROUP BY b ORDER BY b",
        placeholders_from(ids.len(), 4)
    ))?;
    type BucketRow = (i64, Option<f64>, Option<i64>, Option<f64>, Option<i64>, i64);
    let read = |r: &Row<'_>| -> rusqlite::Result<BucketRow> { Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)) };
    let rows = stmt.query_map(params_from_iter(&args), read)?;
    for row in rows {
        let (b, cpu_avg, cpu_max, mem_avg, mem_max, n) = row?;
        let t = b as f64;
        if let Some(v) = cpu_avg {
            out.cpu_avg.push((t, v / 1000.0));
        }
        if let Some(v) = cpu_max {
            out.cpu_max.push((t, v as f64 / 1000.0));
        }
        if let Some(v) = mem_avg {
            out.mem_avg.push((t, v));
        }
        if let Some(v) = mem_max {
            out.mem_max.push((t, v as f64));
        }
        out.pods.push((t, n as f64 * BUCKET_SECS as f64 / step as f64));
    }

    // Requests and limits as change points across the row's pods.
    let mut stmt = conn.prepare(&format!(
        "SELECT t, cpu_request, cpu_limit, mem_request, mem_limit FROM resources WHERE container_id IN ({list}) ORDER BY t"
    ))?;
    type ResourcesRow = (i64, Option<f64>, Option<f64>, Option<i64>, Option<i64>);
    let read = |r: &Row<'_>| -> rusqlite::Result<ResourcesRow> { Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)) };
    let rows = stmt.query_map(params_from_iter(&ids), read)?;
    for row in rows {
        let (t, cr, cl, mr, ml) = row?;
        let t = t as f64;
        push_change(&mut out.cpu_request, t, cr);
        push_change(&mut out.cpu_limit, t, cl);
        push_change(&mut out.mem_request, t, mr.map(|v| v as f64));
        push_change(&mut out.mem_limit, t, ml.map(|v| v as f64));
    }
    add_marks(conn, &ids, start, end, &mut out)?;
    Ok(out)
}

/// Restarts and OOM kills of `ids`, and the trace's gaps, between `start` and `end`.
fn add_marks(conn: &Connection, ids: &[i64], start: i64, end: i64, out: &mut TraceSeries) -> Result<()> {
    let mut stmt = conn.prepare(&format!(
        "SELECT t, kind, reason, SUM(count) FROM events WHERE t >= ?1 AND t < ?2 AND container_id IN ({}) GROUP BY t, kind, reason ORDER BY t",
        placeholders_from(ids.len(), 3)
    ))?;
    let mut args: Vec<i64> = vec![start, end];
    args.extend(ids);
    let event = |r: &Row<'_>| -> rusqlite::Result<TraceEventMark> {
        Ok(TraceEventMark { t: r.get::<_, i64>(0)? as f64, kind: r.get(1)?, reason: r.get(2)?, count: r.get::<_, i64>(3)?.max(0) as u32 })
    };
    let rows = stmt.query_map(params_from_iter(&args), event)?;
    out.events = rows.collect::<rusqlite::Result<_>>()?;

    let mut stmt = conn.prepare("SELECT start, end, reason FROM gaps WHERE end > ?1 AND start < ?2 ORDER BY start")?;
    let gap = |r: &Row<'_>| -> rusqlite::Result<TraceGap> {
        Ok(TraceGap { start: r.get::<_, i64>(0)? as f64, end: r.get::<_, i64>(1)? as f64, reason: r.get(2)? })
    };
    let rows = stmt.query_map([start, end], gap)?;
    out.gaps = rows.collect::<rusqlite::Result<_>>()?;
    Ok(())
}

/// Every container of the trace (or of one namespace) added up, between
/// `start` and `end` (the whole span when absent), in about `points`
/// buckets: per bucket the average and the busiest minute of the total
/// usage, the total request of the containers running then (as change
/// points), and how many were running. Limits are left out: a total is
/// only meaningful when every container has one.
pub fn overview(path: &Path, namespace: Option<&str>, start: Option<f64>, end: Option<f64>, points: u32) -> Result<TraceSeries> {
    let file = TraceFile::open(path)?;
    let conn = file.conn();
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare(
            "SELECT c.id FROM containers c JOIN pods p ON p.id = c.pod_id WHERE ?1 IS NULL OR p.namespace = ?1 ORDER BY c.id",
        )?;
        stmt.query_map([namespace], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    let list = placeholders(ids.len());
    let (full_start, full_end): (Option<i64>, Option<i64>) = if ids.is_empty() {
        (None, None)
    } else {
        conn.query_row(
            &format!("SELECT MIN(t), MAX(t) FROM usage WHERE container_id IN ({list})"),
            params_from_iter(&ids),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
    };
    let (Some(full_start), Some(full_end)) = (full_start, full_end.map(|t| t + BUCKET_SECS)) else {
        return Err(Error::Message(match namespace {
            Some(ns) => format!("no usage in {ns} in this trace"),
            None => "no usage in this trace".into(),
        }));
    };
    let start = start.map_or(full_start, |s| s as i64).clamp(full_start, full_end);
    let end = end.map_or(full_end, |e| e as i64).clamp(start + BUCKET_SECS, full_end.max(start + BUCKET_SECS));
    let step = bucket_for(end - start, points);

    // Each container's requests as change points: (t, millicores, bytes).
    let mut requests: HashMap<i64, Vec<(i64, f64, f64)>> = HashMap::new();
    let mut stmt =
        conn.prepare(&format!("SELECT container_id, t, cpu_request, mem_request FROM resources WHERE container_id IN ({list}) ORDER BY t"))?;
    let read = |r: &Row<'_>| -> rusqlite::Result<(i64, i64, Option<f64>, Option<i64>)> { Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)) };
    let rows = stmt.query_map(params_from_iter(&ids), read)?;
    for row in rows {
        let (id, t, cpu, mem) = row?;
        requests.entry(id).or_default().push((t, cpu.unwrap_or(0.0), mem.unwrap_or(0) as f64));
    }
    let request_at = |id: i64, t: i64| -> (f64, f64) {
        let Some(changes) = requests.get(&id) else { return (0.0, 0.0) };
        match changes.partition_point(|c| c.0 <= t) {
            0 => (0.0, 0.0),
            i => (changes[i - 1].1, changes[i - 1].2),
        }
    };

    // Totals per minute, then per bucket.
    let mut minutes: BTreeMap<i64, Totals> = BTreeMap::new();
    let mut args: Vec<i64> = vec![start, end];
    args.extend(&ids);
    let mut stmt = conn.prepare(&format!(
        "SELECT container_id, t, cpu_avg, mem_avg FROM usage WHERE t >= ?1 AND t < ?2 AND container_id IN ({})",
        placeholders_from(ids.len(), 3)
    ))?;
    let read = |r: &Row<'_>| -> rusqlite::Result<(i64, i64, Option<i64>, Option<i64>)> { Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)) };
    let rows = stmt.query_map(params_from_iter(&args), read)?;
    for row in rows {
        let (id, t, cpu, mem) = row?;
        let (cpu_req, mem_req) = request_at(id, t);
        let m = minutes.entry(t).or_default();
        m.cpu += cpu.unwrap_or(0) as f64 / 1000.0;
        m.mem += mem.unwrap_or(0) as f64;
        m.cpu_req += cpu_req;
        m.mem_req += mem_req;
        m.n += 1.0;
    }
    let mut out = TraceSeries {
        start: start as f64,
        end: end as f64,
        step: step as f64,
        cpu_avg: vec![],
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
        full_start: full_start as f64,
        full_end: full_end as f64,
    };
    let mut buckets: BTreeMap<i64, Vec<&Totals>> = BTreeMap::new();
    for (t, m) in &minutes {
        buckets.entry(t / step * step).or_default().push(m);
    }
    let any_cpu_request = minutes.values().any(|m| m.cpu_req > 0.0);
    let any_mem_request = minutes.values().any(|m| m.mem_req > 0.0);
    for (b, ms) in buckets {
        let t = b as f64;
        let n = ms.len() as f64;
        let mean = |f: fn(&Totals) -> f64| ms.iter().map(|m| f(m)).sum::<f64>() / n;
        let max = |f: fn(&Totals) -> f64| ms.iter().map(|m| f(m)).fold(0.0, f64::max);
        out.cpu_avg.push((t, mean(|m| m.cpu)));
        out.cpu_max.push((t, max(|m| m.cpu)));
        out.mem_avg.push((t, mean(|m| m.mem)));
        out.mem_max.push((t, max(|m| m.mem)));
        // Running containers on average, counting minutes without data as none.
        out.pods.push((t, ms.iter().map(|m| m.n).sum::<f64>() * BUCKET_SECS as f64 / step as f64));
        if any_cpu_request {
            push_change(&mut out.cpu_request, t, Some(mean(|m| m.cpu_req)));
        }
        if any_mem_request {
            push_change(&mut out.mem_request, t, Some(mean(|m| m.mem_req)));
        }
    }
    add_marks(conn, &ids, start, end, &mut out)?;
    Ok(out)
}

/// One minute of an overview: usage and requests summed over the containers
/// with data then, and how many there were.
#[derive(Default)]
struct Totals {
    cpu: f64,
    mem: f64,
    cpu_req: f64,
    mem_req: f64,
    n: f64,
}

/// Adds a change point when a set value differs from the last one. Spans
/// where it isn't set are simply left out.
fn push_change(series: &mut Series, t: f64, value: Option<f64>) {
    if let Some(v) = value
        && series.last().is_none_or(|p| p.1 != v)
    {
        series.push((t, v));
    }
}

/// Whole minutes per bucket so `span` fits in about `points` buckets.
fn bucket_for(span: i64, points: u32) -> i64 {
    let per = span as f64 / f64::from(points.max(1));
    ((per / BUCKET_SECS as f64).ceil() as i64).max(1) * BUCKET_SECS
}

fn container_ids(conn: &Connection, key: &TraceKey) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT c.id FROM containers c JOIN pods p ON p.id = c.pod_id
         WHERE p.namespace = ?1 AND COALESCE(p.workload_kind, 'Pod') = ?2 AND COALESCE(p.workload_name, p.name) = ?3 AND c.name = ?4",
    )?;
    let ids = stmt
        .query_map([&key.namespace, &key.kind, &key.name, &key.container], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    if ids.is_empty() {
        return Err(Error::Message(format!("{} {} isn't in this trace", key.kind, key.name)));
    }
    Ok(ids)
}

fn placeholders(n: usize) -> String {
    placeholders_from(n, 1)
}

/// `?first, ?first+1, …` for `n` values.
fn placeholders_from(n: usize, first: usize) -> String {
    (first..first + n).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ")
}

/// One row's usage by local day and hour in `tz`: p95 CPU and peak memory.
pub fn daily(path: &Path, key: &TraceKey, tz: &jiff::tz::TimeZone) -> Result<TraceDaily> {
    let file = TraceFile::open(path)?;
    let conn = file.conn();
    let ids = container_ids(conn, key)?;
    let mut stmt = conn.prepare(&format!(
        "SELECT t, cpu_avg, mem_max FROM usage WHERE container_id IN ({})",
        placeholders(ids.len())
    ))?;
    // (date, hour) → CPU values and peak memory.
    let mut cells: BTreeMap<(jiff::civil::Date, i8), (Vec<f64>, Option<f64>)> = BTreeMap::new();
    let read = |r: &Row<'_>| -> rusqlite::Result<(i64, Option<i64>, Option<i64>)> { Ok((r.get(0)?, r.get(1)?, r.get(2)?)) };
    let rows = stmt.query_map(params_from_iter(&ids), read)?;
    for row in rows {
        let (t, cpu, mem) = row?;
        let Ok(ts) = jiff::Timestamp::from_second(t) else { continue };
        let local = ts.to_zoned(tz.clone());
        let cell = cells.entry((local.date(), local.hour())).or_default();
        if let Some(c) = cpu {
            cell.0.push(c as f64 / 1000.0);
        }
        if let Some(m) = mem {
            cell.1 = Some(cell.1.map_or(m as f64, |x: f64| x.max(m as f64)));
        }
    }
    let mut days: BTreeMap<jiff::civil::Date, TraceDay> = BTreeMap::new();
    for ((date, hour), (mut cpu, mem)) in cells {
        let day = days.entry(date).or_insert_with(|| TraceDay {
            label: date.strftime("%a %-d %b").to_string(),
            cpu: vec![None; 24],
            mem: vec![None; 24],
        });
        cpu.sort_by(f64::total_cmp);
        let h = hour as usize;
        day.cpu[h] = (!cpu.is_empty()).then(|| percentile(&cpu, 0.95));
        day.mem[h] = mem;
    }
    let mut days: Vec<TraceDay> = days.into_values().collect();
    if days.len() > MAX_DAYS {
        days.drain(..days.len() - MAX_DAYS);
    }
    Ok(TraceDaily { days })
}

#[cfg(test)]
#[path = "trace_review_tests.rs"]
pub(crate) mod tests;
