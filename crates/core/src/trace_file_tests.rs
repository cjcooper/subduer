use super::*;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("subduer-trace-file-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn key(pod: &str) -> ContainerKey {
    ContainerKey::new("shop", pod, "app")
}

fn bucket(t: i64, cpu: f64, memory: f64) -> Bucket {
    let stat = |v: f64| Some(Stat { min: v, avg: v, max: v });
    Bucket { t, cpu: stat(cpu), memory: stat(memory), throttled: None }
}

fn usage(file: &TraceFile) -> Vec<(i64, i64, i64)> {
    let mut stmt = file.conn().prepare("SELECT t, cpu_max, source FROM usage ORDER BY t").unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(rusqlite::Result::unwrap).collect()
}

fn owner(file: &TraceFile, pod: &str) -> (Option<String>, Option<String>, Option<String>) {
    file.conn()
        .query_row("SELECT workload_kind, workload_name, revision FROM pods WHERE name = ?1", [pod], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap()
}

#[test]
fn create_writes_the_header_and_open_reads_it() {
    let dir = scratch("create");
    let path = dir.join("a.trace");
    let scope = TraceScope::Namespaces(vec!["shop".into()]);
    let file = TraceFile::create(&path, false, "acme-uat", &scope, 1000.0).unwrap();
    assert_eq!(file.path(), path);
    let id: i32 = file.conn().pragma_query_value(None, "application_id", |r| r.get(0)).unwrap();
    assert_eq!(id, APPLICATION_ID);
    assert_eq!(file.meta("bucket_secs").unwrap().as_deref(), Some("60"));
    assert_eq!(file.meta("missing").unwrap(), None);
    file.set_meta("prometheus", "monitoring/prometheus").unwrap();
    file.set_meta("retention_secs", "7200").unwrap();

    let s = file.summary(true).unwrap();
    assert_eq!((s.context.as_str(), &s.scope, s.started), ("acme-uat", &scope, 1000.0));
    assert_eq!((s.ended, s.end_reason, s.unfinished), (None, None, false));
    assert_eq!((s.data_start, s.data_end), (None, None));
    assert_eq!(s.prometheus.as_deref(), Some("monitoring/prometheus"));
    assert_eq!(s.retention_secs, Some(7200.0));
    assert_eq!((s.pods, s.containers, s.prometheus_samples, s.local_samples), (0, 0, 0, 0));
    assert!(s.file_bytes > 0);
    drop(file);

    // Opened later without being recorded and without an end: unfinished.
    let s = TraceFile::open(&path).unwrap().summary(false).unwrap();
    assert!(s.unfinished);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_existing_file_is_only_replaced_when_asked() {
    let dir = scratch("exists");
    let path = dir.join("a.trace");
    std::fs::write(&path, "keep me").unwrap();
    let err = TraceFile::create(&path, false, "c", &TraceScope::Cluster, 0.0).err().unwrap();
    assert!(err.to_string().contains("already exists"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me");
    std::fs::write(dir.join("a.trace-wal"), "stale").unwrap();
    TraceFile::create(&path, true, "c", &TraceScope::Cluster, 0.0).unwrap();
    assert!(TraceFile::open(&path).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn open_refuses_other_files() {
    let dir = scratch("other");
    let text = dir.join("notes.trace");
    std::fs::write(&text, "not sqlite at all").unwrap();
    assert!(TraceFile::open(&text).err().unwrap().to_string().contains("isn't a trace file"));

    let other_db = dir.join("other.db");
    Connection::open(&other_db).unwrap().execute_batch("CREATE TABLE t (x)").unwrap();
    assert!(TraceFile::open(&other_db).err().unwrap().to_string().contains("isn't a trace file"));

    assert!(TraceFile::open(&dir.join("missing.trace")).err().unwrap().to_string().contains("can't open"));

    let newer = dir.join("newer.trace");
    let file = TraceFile::create(&newer, false, "c", &TraceScope::Cluster, 0.0).unwrap();
    file.set_meta("format", &(FORMAT + 1).to_string()).unwrap();
    drop(file);
    assert!(TraceFile::open(&newer).err().unwrap().to_string().contains("newer version"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn prometheus_usage_wins_over_the_apps_samples() {
    let dir = scratch("sources");
    let mut file = TraceFile::create(&dir.join("a.trace"), false, "c", &TraceScope::Cluster, 0.0).unwrap();
    let local = |b: Bucket| Batch { source: Some(Source::Local), usage: vec![(key("web-1"), b)], ..Batch::default() };
    let prom = |b: Bucket| Batch { source: Some(Source::Prometheus), usage: vec![(key("web-1"), b)], ..Batch::default() };

    file.write(&local(bucket(60, 1.0, 10.0))).unwrap();
    file.write(&local(bucket(120, 2.0, 10.0))).unwrap();
    // A later local sample of the same minute replaces the earlier one.
    file.write(&local(bucket(120, 2.5, 10.0))).unwrap();
    assert_eq!(usage(&file), vec![(60, 1000, 1), (120, 2500, 1)]);
    // Prometheus replaces local data…
    file.write(&prom(bucket(60, 3.0, 10.0))).unwrap();
    // …and local data never replaces Prometheus'.
    file.write(&local(bucket(60, 9.0, 10.0))).unwrap();
    assert_eq!(usage(&file), vec![(60, 3000, 0), (120, 2500, 1)]);

    let s = file.summary(true).unwrap();
    assert_eq!((s.pods, s.containers, s.prometheus_samples, s.local_samples), (1, 1, 1, 1));
    assert_eq!((s.data_start, s.data_end), (Some(60.0), Some(180.0)));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn stores_units_resources_events_and_empty_batches() {
    let dir = scratch("units");
    let mut file = TraceFile::create(&dir.join("a.trace"), false, "c", &TraceScope::Cluster, 0.0).unwrap();
    file.write(&Batch::default()).unwrap();
    let b = Bucket {
        t: 60,
        cpu: Some(Stat { min: 0.5, avg: 1.25, max: 2.0 }),
        memory: None,
        throttled: Some(0.25),
    };
    let r = Resources { cpu_request: Some(250.0), cpu_limit: None, mem_request: Some(1.5), mem_limit: Some(2e9) };
    let e = Event { t: 120, kind: EventKind::Restart, reason: Some("OOMKilled".into()), count: 1 };
    let batch = Batch {
        source: None,
        usage: vec![(key("web-1"), b)],
        resources: vec![(key("web-1"), 60, r)],
        events: vec![(key("web-1"), e.clone()), (key("web-1"), e), (key("web-1"), Event {
            t: 120,
            kind: EventKind::Oom,
            reason: None,
            count: 2,
        })],
        owners: vec![],
    };
    file.write(&batch).unwrap();
    let row: (i64, i64, i64, Option<i64>, f64, i64) = file
        .conn()
        .query_row("SELECT cpu_min, cpu_avg, cpu_max, mem_max, throttled, source FROM usage", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
        })
        .unwrap();
    assert_eq!(row, (500, 1250, 2000, None, 0.25, 0));
    let res: (f64, Option<f64>, i64, i64) = file
        .conn()
        .query_row("SELECT cpu_request, cpu_limit, mem_request, mem_limit FROM resources", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap();
    assert_eq!(res, (250.0, None, 2, 2_000_000_000));
    let events: Vec<(String, Option<String>, i64)> = file
        .conn()
        .prepare("SELECT kind, reason, count FROM events ORDER BY kind")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(rusqlite::Result::unwrap)
        .collect();
    assert_eq!(events, vec![("oom".into(), None, 2), ("restart".into(), Some("OOMKilled".into()), 1)]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn owners_from_the_apps_samples_only_fill_gaps() {
    let dir = scratch("owners");
    let mut file = TraceFile::create(&dir.join("a.trace"), false, "c", &TraceScope::Cluster, 0.0).unwrap();
    let pod = ("shop".to_owned(), "nightly-123-abcde".to_owned());
    let job = Owner { kind: "Job".into(), name: "nightly-123".into(), revision: None };
    let cron = Owner { kind: "CronJob".into(), name: "nightly".into(), revision: None };
    let with = |source, owner: &Owner| Batch { source: Some(source), owners: vec![(pod.clone(), owner.clone())], ..Batch::default() };

    file.write(&with(Source::Local, &job)).unwrap();
    assert_eq!(owner(&file, &pod.1).0.as_deref(), Some("Job"));
    file.write(&with(Source::Prometheus, &cron)).unwrap();
    file.write(&with(Source::Local, &job)).unwrap();
    assert_eq!(owner(&file, &pod.1), (Some("CronJob".into()), Some("nightly".into()), None));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn finish_leaves_one_file_with_the_end() {
    let dir = scratch("finish");
    let path = dir.join("a.trace");
    let mut file = TraceFile::create(&path, false, "c", &TraceScope::Cluster, 0.0).unwrap();
    file.write(&Batch { usage: vec![(key("web-1"), bucket(60, 1.0, 1.0))], ..Batch::default() }).unwrap();
    assert!(dir.join("a.trace-wal").exists());
    file.add_gap(100, 100, "empty, ignored").unwrap();
    file.add_gap(300, 600, "Prometheus didn't answer").unwrap();
    file.log_pull(5, 0, 60, 1, None).unwrap();
    file.log_pull(6, 60, 120, 0, Some("nope")).unwrap();
    file.finish(700.0, TraceEnd::Quit).unwrap();
    drop(file);
    assert!(!dir.join("a.trace-wal").exists());

    let s = TraceFile::open(&path).unwrap().summary(false).unwrap();
    assert_eq!((s.ended, s.end_reason, s.unfinished), (Some(700.0), Some(TraceEnd::Quit), false));
    assert_eq!(s.gaps, vec![TraceGap { start: 300.0, end: 600.0, reason: "Prometheus didn't answer".into() }]);
    let pulls: i64 = TraceFile::open(&path).unwrap().conn().query_row("SELECT COUNT(*) FROM pulls WHERE ok = 0", [], |r| r.get(0)).unwrap();
    assert_eq!(pulls, 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn summary_needs_the_header() {
    let dir = scratch("header");
    let path = dir.join("a.trace");
    let file = TraceFile::create(&path, false, "c", &TraceScope::Cluster, 0.0).unwrap();
    file.conn().execute("DELETE FROM meta WHERE key = 'context'", []).unwrap();
    assert!(file.summary(true).err().unwrap().to_string().contains("has no context"));
    file.set_meta("scope", "not json").unwrap();
    assert!(file.summary(true).err().unwrap().to_string().contains("trace scope"));
    std::fs::remove_dir_all(&dir).unwrap();
}
