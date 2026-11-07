use super::*;
use crate::app::tests_unit::{context, kind, row};
use subduer_types::{Category, InfoRow, InfoSection, InfoTable, InfoTableRow, KindInfo, ObjectLink, Usage};

const ALL: [Op; 14] = [
    Op::Restart,
    Op::Scale,
    Op::Run,
    Op::Suspend,
    Op::Resume,
    Op::Range,
    Op::Save,
    Op::Expand,
    Op::Reclaim,
    Op::Rollback,
    Op::Uninstall,
    Op::ForceSync,
    Op::Pause,
    Op::Unpause,
];

fn info_row(key: &str, value: &str) -> InfoRow {
    InfoRow { key: key.into(), value: value.into(), mono: false, time: false, link: None, url: None }
}

fn table_row(cells: &[&str]) -> InfoTableRow {
    InfoTableRow { cells: cells.iter().map(|c| (*c).to_owned()).collect(), link: None, fill: None, warn: false }
}

fn section(id: &str, rows: Vec<InfoTableRow>) -> InfoSection {
    InfoSection {
        id: id.into(),
        title: id.into(),
        summary: String::new(),
        rows: vec![],
        table: Some(InfoTable { rows, ..InfoTable::default() }),
        empty: None,
    }
}

fn object(kind: &str, name: &str, info: Vec<InfoRow>, sections: Vec<InfoSection>) -> ObjectDetails {
    ObjectDetails {
        kind: kind.into(),
        name: name.into(),
        namespace: Some("shop".into()),
        status: None,
        info,
        created: None,
        labels: vec![],
        annotations: vec![],
        conditions: vec![],
        data: None,
        sections,
        events: vec![],
        node: None,
        logs: None,
    }
}

fn loaded(d: ObjectDetails) -> Loaded<ObjectDetails> {
    Some(Ok(d))
}

fn workload(kind: &str, name: &str, status: &str, suspended: Option<bool>, info: Vec<(String, String)>) -> WorkloadDetails {
    WorkloadDetails {
        kind: kind.into(),
        name: name.into(),
        namespace: "shop".into(),
        status: status.into(),
        replicas: None,
        suspended,
        info,
        created: None,
        labels: vec![],
        conditions: vec![],
        cpu: Usage::default(),
        memory: Usage::default(),
        metrics_available: false,
        pods: vec![],
        containers: vec![],
        jobs: vec![],
        events: vec![],
    }
}

fn cron_job(name: &str, status: &str, concurrency: Option<&str>) -> Loaded<WorkloadDetails> {
    let info = concurrency.map(|c| vec![("Concurrency".to_owned(), c.to_owned())]).unwrap_or_default();
    Some(Ok(workload("CronJob", name, status, Some(false), info)))
}

fn cluster(perms: &[(&str, &str, &str)]) -> ClusterInfo {
    ClusterInfo {
        context: "acme-dev".into(),
        kinds: perms
            .iter()
            .map(|(g, k, verb)| KindInfo { kind: kind(g, k), verbs: vec![(*verb).to_owned()], category: Category::Workloads })
            .collect(),
        namespaces: vec![],
    }
}

fn revision(rev: i64, status: &str, chart: &str, app: &str) -> Revision {
    (rev, status.into(), chart.into(), app.into(), "2026-01-01".into())
}

fn release(revisions: &[Revision]) -> Loaded<ObjectDetails> {
    let rows = revisions.iter().map(|r| table_row(&[&r.0.to_string(), &r.1, &r.2, &r.3, &r.4])).collect();
    loaded(object("Release", "shop", vec![], vec![section("helm-history", rows)]))
}

fn target(k: KindRef, op: Op) -> WorkloadTarget {
    WorkloadTarget { context: context("acme-dev", true), kind: k, namespace: "shop".into(), name: "web".into(), op, save: None }
}

// ---- Ops ----

#[test]
fn op_labels() {
    let labels: Vec<&str> = ALL.iter().map(|o| o.label()).collect();
    assert_eq!(
        labels,
        [
            "Restart",
            "Scale",
            "Run now",
            "Suspend",
            "Resume",
            "Edit range",
            "Save",
            "Expand",
            "Reclaim policy",
            "Rollback",
            "Delete",
            "Force sync",
            "Pause",
            "Resume"
        ]
    );
}

#[test]
fn op_hints_depend_on_kind_for_suspend_and_resume() {
    let cron = kind("batch", "CronJob");
    let job = kind("batch", "Job");
    assert_eq!(Op::Suspend.hint(&cron), "Stop starting new Jobs");
    assert_eq!(Op::Suspend.hint(&job), "Stop its running pods until resumed");
    assert_eq!(Op::Resume.hint(&cron), "Start Jobs on its schedule again");
    assert_eq!(Op::Resume.hint(&job), "Start its pods again");
    assert_eq!(Op::Restart.hint(&job), "Rollout restart");
    assert_eq!(Op::Scale.hint(&job), "Scale");
    assert_eq!(Op::Run.hint(&cron), "Start a Job from its template now");
    assert_eq!(Op::Range.hint(&job), "Change its min and max replicas");
    assert_eq!(Op::Save.hint(&job), "Save the edited data");
    assert_eq!(Op::Expand.hint(&job), "Make the claim bigger");
    assert_eq!(Op::Reclaim.hint(&job), "Keep or delete the data when its claim is deleted");
    assert_eq!(Op::Rollback.hint(&job), "Go back to an earlier revision with helm rollback");
    assert!(Op::Uninstall.hint(&job).contains("helm uninstall"));
    assert!(Op::ForceSync.hint(&kind("external-secrets.io", "ClusterExternalSecret")).contains("every namespace"));
    assert!(Op::ForceSync.hint(&kind("external-secrets.io", "ExternalSecret")).contains("update the Secret"));
    let (so, sj) = (kind("keda.sh", "ScaledObject"), kind("keda.sh", "ScaledJob"));
    assert_eq!(Op::Range.hint(&sj), "Change how many Jobs it may run at once");
    assert_eq!(Op::Range.hint(&so), "Change its min and max replicas");
    assert_eq!(Op::Pause.hint(&sj), "Stop KEDA starting new Jobs");
    assert!(Op::Pause.hint(&so).contains("replica count"));
    assert!(Op::Unpause.hint(&so).contains("again"));
    for op in ALL {
        assert!(!op.hint(&job).is_empty());
    }
}

#[test]
fn button_titles_mention_the_lock() {
    assert_eq!(button_title("Scale", false), "Scale…");
    assert_eq!(button_title("Scale", true), "Scale… (asks to allow changes first: the context is read-only)");
}

#[test]
fn has_ops_for_workloads_and_changeable_kinds() {
    for (g, k) in [
        ("apps", "Deployment"),
        ("batch", "CronJob"),
        ("external-secrets.io", "ExternalSecret"),
        ("keda.sh", "ScaledJob"),
        ("autoscaling", "HorizontalPodAutoscaler"),
        ("", "PersistentVolumeClaim"),
        ("", "PersistentVolume"),
        ("helm.sh", "Release"),
    ] {
        assert!(has_ops(&kind(g, k)), "{g}/{k}");
    }
    assert!(!has_ops(&kind("", "ConfigMap")));
}

#[test]
fn allowed_reads_the_listed_verbs() {
    let c = cluster(&[("apps", "Deployment", "patch")]);
    assert!(allowed(Some(&c), "apps/Deployment", "patch"));
    assert!(!allowed(Some(&c), "apps/Deployment", "delete"));
    assert!(!allowed(Some(&c), "batch/Job", "patch"));
    assert!(!allowed(None, "apps/Deployment", "patch"));
}

#[test]
fn ops_for_helm_releases_follow_the_history() {
    let k = kind("helm.sh", "Release");
    let r = row(Some("shop"), "shop");
    assert!(ops_for(&k, &r, None, &None, &None).is_empty());
    let one = release(&[revision(1, "deployed", "shop-1.0.0", "")]);
    assert_eq!(ops_for(&k, &r, None, &one, &None), [Op::Uninstall]);
    let two = release(&[revision(2, "deployed", "c", ""), revision(1, "superseded", "c", "")]);
    assert_eq!(ops_for(&k, &r, None, &two, &None), [Op::Rollback, Op::Uninstall]);
}

#[test]
fn ops_for_hpas_claims_and_volumes_need_patch() {
    let r = row(Some("shop"), "web");
    let hpa = kind("autoscaling", "HorizontalPodAutoscaler");
    assert!(ops_for(&hpa, &r, None, &None, &None).is_empty());
    let c = cluster(&[("autoscaling", "HorizontalPodAutoscaler", "patch")]);
    assert_eq!(ops_for(&hpa, &r, Some(&c), &None, &None), [Op::Range]);

    let pvc = kind("", "PersistentVolumeClaim");
    let c = cluster(&[("", "PersistentVolumeClaim", "patch")]);
    assert!(ops_for(&pvc, &r, Some(&c), &None, &None).is_empty());
    let can = loaded(object("PersistentVolumeClaim", "web", vec![info_row("Can expand", "yes")], vec![]));
    assert_eq!(ops_for(&pvc, &r, Some(&c), &can, &None), [Op::Expand]);
    assert!(ops_for(&pvc, &r, None, &can, &None).is_empty());

    let pv = kind("", "PersistentVolume");
    let c = cluster(&[("", "PersistentVolume", "patch")]);
    let r = row(None, "pv-1");
    for (policy, offered) in [("Retain", true), ("Delete (default)", true), ("Recycle", false)] {
        let o = loaded(object("PersistentVolume", "pv-1", vec![info_row("Reclaim policy", policy)], vec![]));
        assert_eq!(ops_for(&pv, &r, Some(&c), &o, &None) == [Op::Reclaim], offered, "{policy}");
    }
    assert!(ops_for(&pv, &r, Some(&c), &None, &None).is_empty());
}

#[test]
fn ops_for_external_secrets_and_keda() {
    let r = row(Some("shop"), "web");
    let es = kind("external-secrets.io", "ExternalSecret");
    assert!(ops_for(&es, &r, None, &None, &None).is_empty());
    let c = cluster(&[("external-secrets.io", "ExternalSecret", "patch")]);
    assert_eq!(ops_for(&es, &r, Some(&c), &None, &None), [Op::ForceSync]);

    let so = kind("keda.sh", "ScaledObject");
    let c = cluster(&[("keda.sh", "ScaledObject", "patch")]);
    assert!(ops_for(&so, &r, Some(&c), &None, &None).is_empty(), "waits for the details");
    let running = loaded(object("ScaledObject", "web", vec![], vec![]));
    assert_eq!(ops_for(&so, &r, Some(&c), &running, &None), [Op::Range, Op::Pause]);
    let paused = loaded(object("ScaledObject", "web", vec![info_row("Paused", "yes")], vec![]));
    assert_eq!(ops_for(&so, &r, Some(&c), &paused, &None), [Op::Range, Op::Unpause]);
    assert!(ops_for(&so, &r, None, &paused, &None).is_empty());
}

#[test]
fn ops_for_scalable_workloads() {
    let r = row(Some("shop"), "web");
    let d = kind("apps", "Deployment");
    assert!(ops_for(&d, &r, None, &None, &None).is_empty());
    let c = cluster(&[("apps", "Deployment", "patch")]);
    assert_eq!(ops_for(&d, &r, Some(&c), &None, &None), [Op::Restart, Op::Scale]);
}

#[test]
fn ops_for_jobs_and_cron_jobs() {
    let r = row(Some("shop"), "nightly");
    let cron = kind("batch", "CronJob");
    let job = kind("batch", "Job");
    let create = cluster(&[("batch", "Job", "create")]);
    assert_eq!(ops_for(&cron, &r, Some(&create), &None, &None), [Op::Run]);
    let patch = cluster(&[("batch", "CronJob", "patch"), ("batch", "Job", "patch")]);
    assert!(ops_for(&cron, &r, Some(&patch), &None, &None).is_empty(), "waits for the details");
    let active = Some(Ok(workload("CronJob", "nightly", "Scheduled", Some(false), vec![])));
    assert_eq!(ops_for(&cron, &r, Some(&patch), &None, &active), [Op::Suspend]);
    assert!(ops_for(&cron, &r, None, &None, &active).is_empty(), "no patch");
    let suspended = Some(Ok(workload("CronJob", "nightly", "Suspended", Some(true), vec![])));
    assert_eq!(ops_for(&cron, &r, Some(&patch), &None, &suspended), [Op::Resume]);
    // Details of another object, namespace or kind don't count.
    assert!(ops_for(&cron, &row(Some("shop"), "other"), Some(&patch), &None, &active).is_empty());
    assert!(ops_for(&cron, &row(Some("acme"), "nightly"), Some(&patch), &None, &active).is_empty());
    assert!(ops_for(&job, &r, Some(&patch), &None, &active).is_empty());
    let unknown = Some(Ok(workload("CronJob", "nightly", "Scheduled", None, vec![])));
    assert!(ops_for(&cron, &r, Some(&patch), &None, &unknown).is_empty());
    // A finished Job has nothing left to suspend.
    for (status, offered) in [("Running", true), ("Complete", false), ("Failed", false)] {
        let w = Some(Ok(workload("Job", "nightly", status, Some(false), vec![])));
        assert_eq!(ops_for(&job, &r, Some(&patch), &None, &w) == [Op::Suspend], offered, "{status}");
    }
}

// ---- The loaded details ----

#[test]
fn bytes_parses_plain_decimal_and_binary_sizes() {
    assert_eq!(bytes("0"), Some(0.0));
    assert_eq!(bytes("1024"), Some(1024.0));
    assert_eq!(bytes("1.5"), Some(1.5));
    assert_eq!(bytes("5k"), Some(5e3));
    assert_eq!(bytes("500M"), Some(5e8));
    assert_eq!(bytes("2G"), Some(2e9));
    assert_eq!(bytes("1T"), Some(1e12));
    assert_eq!(bytes("1Ki"), Some(1024.0));
    assert_eq!(bytes("3Mi"), Some(3.0 * 1024.0 * 1024.0));
    assert_eq!(bytes("20Gi"), Some(20.0 * 1024f64.powi(3)));
    assert_eq!(bytes("1.5Ti"), Some(1.5 * 1024f64.powi(4)));
    assert_eq!(bytes("1Pi"), Some(1024f64.powi(5)));
}

#[test]
fn bytes_handles_exponents() {
    assert_eq!(bytes("1e9"), Some(1e9));
    assert_eq!(bytes("2.5e3"), Some(2500.0));
    assert_eq!(bytes("1E6"), Some(1e6));
}

#[test]
fn bytes_rejects_bad_input() {
    assert_eq!(bytes(""), None);
    assert_eq!(bytes("Gi"), None);
    assert_eq!(bytes("10gi"), None, "suffixes are case-sensitive");
    assert_eq!(bytes("10 Gi"), None);
    assert_eq!(bytes("10Gb"), None);
    assert_eq!(bytes("1.2.3"), None);
    assert_eq!(bytes("-5Gi"), None);
    assert_eq!(bytes("1e"), None);
}

#[test]
fn keda_range_and_replicas_come_from_the_loaded_details() {
    assert_eq!(keda_range(&None, "web"), None);
    let o = loaded(object("ScaledObject", "web", vec![info_row("Range", "0 to 10 replicas"), info_row("Replicas", "3 now · 4 wanted")], vec![]));
    assert_eq!(keda_range(&o, "web"), Some((0, 10)));
    assert_eq!(replicas_now(&o, "web"), Some(3));
    assert_eq!(keda_range(&o, "other"), None, "another object's details");
    let o = loaded(object("ScaledJob", "worker", vec![info_row("Jobs at once", "2 to 20")], vec![]));
    assert_eq!(keda_range(&o, "worker"), Some((2, 20)));
    assert_eq!(replicas_now(&o, "worker"), None);
    let o = loaded(object("ScaledObject", "odd", vec![info_row("Range", "many")], vec![]));
    assert_eq!(keda_range(&o, "odd"), None);
}

#[test]
fn helm_release_comes_from_the_loaded_labels() {
    let mut d = object("ScaledObject", "java", vec![], vec![]);
    assert_eq!(helm_release(&loaded(d.clone()), "java"), None);
    d.labels = vec![("app.kubernetes.io/managed-by".into(), "Helm".into())];
    assert_eq!(helm_release(&loaded(d.clone()), "java").as_deref(), Some(""));
    d.annotations = vec![("meta.helm.sh/release-name".into(), "java".into())];
    let o = loaded(d);
    assert_eq!(helm_release(&o, "java").as_deref(), Some("java"));
    assert_eq!(helm_release(&o, "other"), None);
}

#[test]
fn run_note_follows_the_concurrency_policy() {
    assert_eq!(run_note(&None, "nightly"), None, "no details loaded");

    let w = cron_job("nightly", "Scheduled", Some("Forbid"));
    assert_eq!(cron_state(&w, "nightly"), Some(("Forbid".into(), false)));
    assert_eq!(run_note(&w, "nightly"), Some("Scheduled runs are skipped while this Job runs (concurrency: Forbid)."));
    assert_eq!(run_note(&w, "other"), None, "details of another object");

    let w = cron_job("nightly", "Running", Some("Replace"));
    assert_eq!(cron_state(&w, "nightly"), Some(("Replace".into(), true)));
    assert!(run_note(&w, "nightly").unwrap().starts_with("Its running Job is not replaced"));

    let w = cron_job("nightly", "Scheduled", Some("Replace"));
    assert!(run_note(&w, "nightly").unwrap().starts_with("The next scheduled run replaces this Job"));

    assert_eq!(run_note(&cron_job("nightly", "Scheduled", Some("Allow")), "nightly"), None);
    assert_eq!(cron_state(&cron_job("nightly", "Scheduled", None), "nightly"), None, "no concurrency row");

    let job = Some(Ok(workload("Job", "nightly", "Running", None, vec![("Concurrency".into(), "Forbid".into())])));
    assert_eq!(cron_state(&job, "nightly"), None, "not a CronJob");
    assert_eq!(cron_state(&Some(Err("boom".into())), "nightly"), None);
}

#[test]
fn history_reads_the_release_revisions() {
    assert!(history(&None, "shop").is_empty());
    let o = loaded(object(
        "Release",
        "shop",
        vec![],
        vec![
            section("helm-resources", vec![table_row(&["Deployment", "shop-web"]), table_row(&["Service"])]),
            section(
                "helm-history",
                vec![
                    table_row(&["3", "deployed", "shop-1.2.0", "2.0", "2026-01-03"]),
                    table_row(&["x", "bad revision", "c", "a", "u"]),
                    table_row(&["2", "superseded", "shop-1.1.0"]),
                    table_row(&["1", "superseded", "shop-1.0.0", "1.0", "2026-01-01"]),
                ],
            ),
        ],
    ));
    let expected = vec![
        (3, "deployed".to_owned(), "shop-1.2.0".to_owned(), "2.0".to_owned(), "2026-01-03".to_owned()),
        (1, "superseded".to_owned(), "shop-1.0.0".to_owned(), "1.0".to_owned(), "2026-01-01".to_owned()),
    ];
    assert_eq!(history(&o, "shop"), expected, "malformed rows are skipped");
    assert!(history(&o, "acme").is_empty(), "another release");
    assert_eq!(release_objects(&o, "shop"), [("Deployment".to_owned(), "shop-web".to_owned())]);
    assert!(release_objects(&o, "acme").is_empty());
}

#[test]
fn history_ignores_other_kinds_and_missing_sections() {
    let o = loaded(object("Release", "shop", vec![], vec![]));
    assert!(history(&o, "shop").is_empty());
    assert!(release_objects(&o, "shop").is_empty());
    let o = loaded(object("ConfigMap", "shop", vec![], vec![section("helm-history", vec![table_row(&["1", "a", "b", "c", "d"])])]));
    assert!(history(&o, "shop").is_empty());
    assert!(release_objects(&o, "shop").is_empty());
}

#[test]
fn object_row_reads_an_info_row() {
    assert_eq!(object_row(&None, "web", "Min replicas"), None);
    let o = loaded(object("HorizontalPodAutoscaler", "web", vec![info_row("Min replicas", "2"), info_row("Max replicas", "10")], vec![]));
    assert_eq!(object_row(&o, "web", "Min replicas").as_deref(), Some("2"));
    assert_eq!(object_row(&o, "web", "Max replicas").as_deref(), Some("10"));
    assert_eq!(object_row(&o, "web", "Target"), None);
    assert_eq!(object_row(&o, "api", "Min replicas"), None);
}

#[test]
fn force_sync_notes_and_counts_come_from_the_loaded_details() {
    assert_eq!(force_sync_note(&None, "db"), None);
    assert_eq!(provisioned_count(&None, "shared"), 0);
    let o = loaded(object("ExternalSecret", "db", vec![info_row("Refresh interval", "1h")], vec![]));
    assert_eq!(force_sync_note(&o, "db"), None);
    let o = loaded(object("ExternalSecret", "db", vec![info_row("Refresh policy", "CreatedOnce: fetched once, never refreshed")], vec![]));
    assert!(force_sync_note(&o, "db").unwrap().contains("CreatedOnce"));
    let o = loaded(object("ExternalSecret", "db", vec![info_row("Refresh interval", "0s: fetched once, then never refreshed")], vec![]));
    assert!(force_sync_note(&o, "db").unwrap().contains("interval is 0"));
    assert_eq!(force_sync_note(&o, "other"), None, "another object's details");

    let mut linked = table_row(&["shop", "shared", "Synced", ""]);
    linked.link = Some(ObjectLink { kind: "external-secrets.io/ExternalSecret".into(), namespace: Some("shop".into()), name: "shared".into() });
    let failed = table_row(&["x", "", "Failed: denied", ""]);
    let o = loaded(object("ClusterExternalSecret", "shared", vec![], vec![section("externalsecret-children", vec![linked.clone(), linked, failed])]));
    assert_eq!(provisioned_count(&o, "shared"), 2, "failed namespaces have no ExternalSecret");
    let o = loaded(object("ClusterExternalSecret", "shared", vec![], vec![]));
    assert_eq!(provisioned_count(&o, "shared"), 0);
}

#[test]
fn new_policy_switches_to_the_other_one() {
    let pv = |p: &str| loaded(object("PersistentVolume", "pv-1", vec![info_row("Reclaim policy", p)], vec![]));
    assert_eq!(new_policy(&pv("Retain"), "pv-1"), "Delete");
    assert_eq!(new_policy(&pv("Delete"), "pv-1"), "Retain");
    assert_eq!(new_policy(&None, "pv-1"), "Retain");
}

#[test]
fn range_now_reads_keda_or_hpa_rows() {
    let hpa = loaded(object("HorizontalPodAutoscaler", "web", vec![info_row("Min replicas", "2"), info_row("Max replicas", "10")], vec![]));
    assert_eq!(range_now(&hpa, "web", false), Some((2, 10)));
    assert_eq!(range_now(&hpa, "web", true), None);
    let so = loaded(object("ScaledObject", "web", vec![info_row("Range", "1 to 5 replicas")], vec![]));
    assert_eq!(range_now(&so, "web", true), Some((1, 5)));
    let bad = loaded(object("HorizontalPodAutoscaler", "web", vec![info_row("Min replicas", "x"), info_row("Max replicas", "10")], vec![]));
    assert_eq!(range_now(&bad, "web", false), None);
}

#[test]
fn prefill_per_action() {
    let so = loaded(object("ScaledObject", "web", vec![info_row("Range", "0 to 10 replicas"), info_row("Replicas", "3 now")], vec![]));
    let keda = target(kind("keda.sh", "ScaledObject"), Op::Range);
    assert_eq!(prefill(&keda, &so), Prefill::Range("0".into(), "10".into()));
    assert_eq!(prefill(&keda, &None), Prefill::Range(String::new(), String::new()));
    let hpa = loaded(object("HorizontalPodAutoscaler", "web", vec![info_row("Min replicas", "2"), info_row("Max replicas", "10")], vec![]));
    assert_eq!(prefill(&target(kind("autoscaling", "HorizontalPodAutoscaler"), Op::Range), &hpa), Prefill::Range("2".into(), "10".into()));
    assert_eq!(prefill(&target(kind("autoscaling", "HorizontalPodAutoscaler"), Op::Range), &None), Prefill::Range(String::new(), String::new()));
    assert_eq!(prefill(&target(kind("keda.sh", "ScaledObject"), Op::Pause), &so), Prefill::Hold("3".into()));
    assert_eq!(prefill(&target(kind("keda.sh", "ScaledObject"), Op::Pause), &None), Prefill::Hold("0".into()));
    let pvc = loaded(object("PersistentVolumeClaim", "web", vec![info_row("Requested", "10Gi")], vec![]));
    assert_eq!(prefill(&target(kind("", "PersistentVolumeClaim"), Op::Expand), &pvc), Prefill::Size("10Gi".into()));
    assert_eq!(prefill(&target(kind("", "PersistentVolumeClaim"), Op::Expand), &None), Prefill::Size(String::new()));
    assert_eq!(prefill(&target(kind("apps", "Deployment"), Op::Scale), &None), Prefill::FetchReplicas);
    assert_eq!(prefill(&target(kind("apps", "Deployment"), Op::Restart), &None), Prefill::Nothing);
}

#[test]
fn prefill_rolls_back_to_the_latest_earlier_revision_that_worked() {
    let mut t = target(kind("helm.sh", "Release"), Op::Rollback);
    t.name = "shop".into();
    let o = release(&[revision(4, "deployed", "c", ""), revision(3, "failed", "c", ""), revision(2, "superseded", "c", "")]);
    assert_eq!(prefill(&t, &o), Prefill::Rollback("2".into()));
    let o = release(&[revision(4, "deployed", "c", ""), revision(3, "failed", "c", "")]);
    assert_eq!(prefill(&t, &o), Prefill::Rollback("3".into()), "only failed ones: the previous");
    let o = release(&[revision(1, "deployed", "c", "")]);
    assert_eq!(prefill(&t, &o), Prefill::Rollback(String::new()));
}

// ---- The dialog's entries ----

#[test]
fn counts_and_ranges_parse() {
    assert_eq!(parse_count(" 3 "), Some(3));
    assert_eq!(parse_count("0"), Some(0));
    assert_eq!(parse_count("-1"), None);
    assert_eq!(parse_count("x"), None);
    assert_eq!((range_floor(true), range_floor(false)), (0, 1));
    assert_eq!(parse_range(" 1", "5 ", 1), Some((1, 5)));
    assert_eq!(parse_range("0", "5", 1), None, "an HPA can't go to zero");
    assert_eq!(parse_range("0", "5", 0), Some((0, 5)));
    assert_eq!(parse_range("0", "0", 0), None, "max at least 1");
    assert_eq!(parse_range("6", "5", 0), None, "max at least min");
    assert_eq!(parse_range("x", "5", 0), None);
    assert_eq!(pause_hold(false, "x"), None);
    assert_eq!(pause_hold(true, "2"), Some(Some(2)));
    assert_eq!(pause_hold(true, "x"), Some(None));
    assert!(grows(" 20Gi ", "10Gi"));
    assert!(!grows("10Gi", "10Gi"));
    assert!(!grows("lots", "10Gi"));
    assert!(!grows("20Gi", ""));
}

#[test]
fn confirm_needs_the_name_on_production_and_for_uninstall() {
    assert!(needs_name(true, Op::Restart));
    assert!(needs_name(false, Op::Uninstall));
    assert!(!needs_name(false, Op::Restart));
    let f = Form::default();
    assert!(can_confirm(Op::Restart, false, "web", &f));
    assert!(!can_confirm(Op::Restart, true, "web", &f));
    assert!(can_confirm(Op::Restart, true, "web", &Form { typed: " web ".into(), ..Form::default() }));
    assert!(!can_confirm(Op::Uninstall, false, "web", &f));
    assert!(!can_confirm(Op::Restart, false, "web", &Form { busy: true, ..Form::default() }));
}

#[test]
fn confirm_checks_each_actions_entry() {
    let f = Form::default();
    let scale = |desired, wanted| Form { current: Some(Replicas { desired, current: desired }), replicas: wanted, ..Form::default() };
    assert!(can_confirm(Op::Scale, false, "web", &scale(3, Some(4))));
    assert!(!can_confirm(Op::Scale, false, "web", &scale(3, Some(3))), "no change");
    assert!(!can_confirm(Op::Scale, false, "web", &scale(3, None)));
    assert!(!can_confirm(Op::Scale, false, "web", &f), "replicas not read yet");
    let range = |range, range_now| Form { range, range_now, ..Form::default() };
    assert!(can_confirm(Op::Range, false, "web", &range(Some((1, 5)), Some((1, 4)))));
    assert!(can_confirm(Op::Range, false, "web", &range(Some((1, 5)), None)));
    assert!(!can_confirm(Op::Range, false, "web", &range(Some((1, 5)), Some((1, 5)))));
    assert!(!can_confirm(Op::Range, false, "web", &range(None, None)));
    assert!(can_confirm(Op::Expand, false, "web", &Form { grows: true, ..Form::default() }));
    assert!(!can_confirm(Op::Expand, false, "web", &f));
    assert!(can_confirm(Op::Rollback, false, "web", &Form { rollback_to: "2".into(), ..Form::default() }));
    assert!(!can_confirm(Op::Rollback, false, "web", &f));
    assert!(can_confirm(Op::Pause, false, "web", &f));
    assert!(can_confirm(Op::Pause, false, "web", &Form { held: Some(Some(2)), ..Form::default() }));
    assert!(!can_confirm(Op::Pause, false, "web", &Form { held: Some(None), ..Form::default() }));
}

#[test]
fn titles_per_action() {
    let d = kind("apps", "Deployment");
    let cases = [
        (Op::Restart, &d, "Restart Deployment?", "Restarting…"),
        (Op::Scale, &d, "Scale Deployment?", "Scaling…"),
        (Op::Run, &d, "Run Deployment now?", "Starting…"),
        (Op::Suspend, &d, "Suspend Deployment?", "Suspending…"),
        (Op::Resume, &d, "Resume Deployment?", "Resuming…"),
        (Op::Pause, &d, "Pause Deployment?", "Pausing…"),
        (Op::Unpause, &d, "Resume Deployment?", "Resuming…"),
        (Op::Expand, &d, "Expand claim?", "Expanding…"),
        (Op::Reclaim, &d, "Change reclaim policy?", "Saving…"),
        (Op::Rollback, &d, "Roll back Helm release?", "Rolling back…"),
        (Op::Uninstall, &d, "Delete Helm release?", "Deleting…"),
        (Op::Save, &d, "Save Deployment?", "Saving…"),
        (Op::ForceSync, &d, "Force sync Deployment?", "Syncing…"),
        (Op::Range, &d, "Edit HPA range?", "Saving…"),
    ];
    for (op, k, t, doing) in cases {
        assert_eq!(title(op, k), (t.to_owned(), doing), "{op:?}");
    }
    assert_eq!(title(Op::Range, &kind("keda.sh", "ScaledJob")).0, "Edit Jobs at once?");
    assert_eq!(title(Op::Range, &kind("keda.sh", "ScaledObject")).0, "Edit replica range?");
}

#[test]
fn confirm_labels_show_what_will_happen() {
    let f = Form::default();
    assert_eq!(confirm_label(Op::Scale, "Scaling…", &Form { busy: true, ..Form::default() }), "Scaling…");
    assert_eq!(confirm_label(Op::Scale, "", &f), "Scale");
    assert_eq!(confirm_label(Op::Scale, "", &Form { replicas: Some(4), ..Form::default() }), "Scale to 4");
    assert_eq!(confirm_label(Op::Uninstall, "", &f), "Delete release");
    assert_eq!(confirm_label(Op::Rollback, "", &f), "Rollback");
    assert_eq!(confirm_label(Op::Rollback, "", &Form { rollback_to: "2".into(), ..Form::default() }), "Roll back to 2");
    assert_eq!(confirm_label(Op::Reclaim, "", &Form { policy: "Delete", ..Form::default() }), "Set Delete");
    assert_eq!(confirm_label(Op::Expand, "", &Form { size: "  ".into(), ..Form::default() }), "Expand");
    assert_eq!(confirm_label(Op::Expand, "", &Form { size: " 20Gi ".into(), ..Form::default() }), "Expand to 20Gi");
    assert_eq!(confirm_label(Op::Pause, "", &f), "Pause");
    assert_eq!(confirm_label(Op::Pause, "", &Form { held: Some(Some(2)), ..Form::default() }), "Pause at 2");
    assert_eq!(confirm_label(Op::Range, "", &f), "Save");
    assert_eq!(confirm_label(Op::Range, "", &Form { range: Some((1, 5)), ..Form::default() }), "Set 1–5");
    assert_eq!(confirm_label(Op::Run, "", &f), "Run now");
    assert_eq!((confirm_class(Op::Uninstall), confirm_class(Op::Scale)), ("danger", "primary"));
    assert_eq!((cancel_label(Op::Save), cancel_label(Op::Scale)), ("Back to editing", "Cancel"));
}

// ---- Requests ----

fn json(t: &WorkloadTarget, c: &Choices) -> (&'static str, serde_json::Value) {
    let (command, args) = change(t, c);
    (command, serde_json::to_value(&args).unwrap())
}

#[test]
fn change_sends_each_actions_command_and_fields() {
    let d = kind("apps", "Deployment");
    let c = Choices {
        replicas: Some(4),
        range: Some((1, 5)),
        held: Some(2),
        size: "20Gi".into(),
        revision: Some(3),
        policy: "Delete",
        wait: true,
        keep_history: true,
    };
    let (cmd, v) = json(&target(d.clone(), Op::Restart), &c);
    assert_eq!(cmd, "restart_workload");
    assert_eq!(v["context"], "acme-dev");
    assert_eq!((v["namespace"].as_str(), v["name"].as_str()), (Some("shop"), Some("web")));
    assert_eq!(v["kind"]["kind"], "Deployment");
    // Every action sends the range when it's set; only its own fields otherwise.
    assert_eq!((v["min"].as_i64(), v["max"].as_i64()), (Some(1), Some(5)));
    for key in ["replicas", "suspend", "paused", "resource_version", "changes", "size", "policy", "revision", "wait", "keep_history"] {
        assert!(v.get(key).is_none(), "{key}");
    }

    let (cmd, v) = json(&target(d.clone(), Op::Scale), &c);
    assert_eq!((cmd, v["replicas"].as_i64()), ("scale_workload", Some(4)));
    assert_eq!(json(&target(d.clone(), Op::Run), &c).0, "run_cron_job");
    let (cmd, v) = json(&target(d.clone(), Op::Suspend), &c);
    assert_eq!((cmd, v["suspend"].as_bool()), ("suspend_workload", Some(true)));
    let (cmd, v) = json(&target(d.clone(), Op::Resume), &c);
    assert_eq!((cmd, v["suspend"].as_bool()), ("suspend_workload", Some(false)));
    assert_eq!(json(&target(kind("keda.sh", "ScaledObject"), Op::Range), &c).0, "set_scaling_range");
    assert_eq!(json(&target(kind("autoscaling", "HorizontalPodAutoscaler"), Op::Range), &c).0, "set_hpa_range");
    let (cmd, v) = json(&target(d.clone(), Op::Expand), &c);
    assert_eq!((cmd, v["size"].as_str()), ("expand_claim", Some("20Gi")));
    let (cmd, v) = json(&target(d.clone(), Op::Reclaim), &c);
    assert_eq!((cmd, v["policy"].as_str()), ("set_reclaim_policy", Some("Delete")));
    let (cmd, v) = json(&target(d.clone(), Op::Rollback), &c);
    assert_eq!((cmd, v["revision"].as_i64(), v["wait"].as_bool()), ("helm_rollback", Some(3), Some(true)));
    assert!(v.get("keep_history").is_none());
    let (cmd, v) = json(&target(d.clone(), Op::Uninstall), &c);
    assert_eq!((cmd, v["wait"].as_bool(), v["keep_history"].as_bool()), ("helm_uninstall", Some(true), Some(true)));
    assert_eq!(json(&target(d.clone(), Op::ForceSync), &c).0, "force_sync");
    let (cmd, v) = json(&target(d.clone(), Op::Pause), &c);
    assert_eq!((cmd, v["paused"].as_bool(), v["replicas"].as_i64()), ("pause_scaling", Some(true), Some(2)));
    let (cmd, v) = json(&target(d.clone(), Op::Unpause), &c);
    assert_eq!((cmd, v["paused"].as_bool()), ("pause_scaling", Some(false)));
    assert!(v.get("replicas").is_none());

    let mut save = target(kind("", "ConfigMap"), Op::Save);
    save.save = Some(SaveData {
        resource_version: "42".into(),
        changes: vec![DataChange { key: "a".into(), value: Some("1".into()) }],
        ..SaveData::default()
    });
    let (cmd, v) = json(&save, &Choices::default());
    assert_eq!(cmd, "save_config_data");
    assert_eq!(v["resource_version"], "42");
    assert_eq!(v["changes"][0]["key"], "a");
    assert!(v.get("min").is_none());
}

#[test]
fn targets_and_what_follows_success() {
    let t = target_for(context("acme-dev", true), kind("", "PersistentVolume"), &row(None, "pv-1"), Op::Reclaim);
    assert_eq!((t.namespace.as_str(), t.name.as_str(), t.op, t.save), ("", "pv-1", Op::Reclaim, None));
    let t = target_for(context("acme-dev", true), kind("apps", "Deployment"), &row(Some("shop"), "web"), Op::Scale);
    assert_eq!(t.namespace, "shop");
    let v = serde_json::to_value(target_args(&t)).unwrap();
    assert_eq!((v["namespace"].as_str(), v["name"].as_str()), (Some("shop"), Some("web")));
    assert_eq!(after_success(Op::Save), (true, false));
    assert_eq!(after_success(Op::Uninstall), (false, true));
    assert_eq!(after_success(Op::Scale), (false, false));
}

// ---- Texts ----

#[test]
fn pause_notes() {
    let (so, sj) = (kind("keda.sh", "ScaledObject"), kind("keda.sh", "ScaledJob"));
    assert!(pause_note(&sj, true).contains("Jobs already running carry on"));
    assert!(pause_note(&so, true).contains("stops scaling"));
    assert!(pause_note(&sj, false).contains("starts Jobs"));
    assert!(pause_note(&so, false).contains("within its range"));
    assert_eq!(pause_detail(Some(2), Some(3)), "Sets autoscaling.keda.sh/paused-replicas to 2.");
    assert_eq!(pause_detail(None, Some(3)), "Sets autoscaling.keda.sh/paused; it stays at 3 replicas.");
    assert_eq!(pause_detail(None, None), "Sets autoscaling.keda.sh/paused; it stays at the replicas it has.");
}

#[test]
fn suspend_notes() {
    let cron = kind("batch", "CronJob");
    let job = kind("batch", "Job");
    assert!(suspend_note(&cron, true).starts_with("No new Jobs start"));
    assert!(suspend_note(&cron, false).starts_with("Jobs start on its schedule again"));
    assert!(suspend_note(&job, true).starts_with("Its running pods are stopped"));
    assert_eq!(suspend_note(&job, false), "New pods start for the work that's left.");
    assert_eq!(will_be(Op::Suspend), " will be suspended.");
    assert_eq!(will_be(Op::Pause), " will be paused.");
    assert_eq!(will_be(Op::Resume), " will be resumed.");
    assert_eq!(will_be(Op::Unpause), " will be resumed.");
}

#[test]
fn restart_notes() {
    assert_eq!(restart_note(&kind("apps", "StatefulSet")), "Its pods are replaced one at a time, highest ordinal first.");
    assert_eq!(
        restart_note(&kind("apps", "Deployment")),
        "New pods replace the current ones following its rollout strategy."
    );
}

#[test]
fn scale_notes() {
    let sts = kind("apps", "StatefulSet");
    let deploy = kind("apps", "Deployment");
    assert_eq!(scale_note(&deploy, 3, 0), Some("Every pod is stopped."));
    assert_eq!(scale_note(&sts, 3, 0), Some("Every pod is stopped."));
    assert_eq!(scale_note(&deploy, 0, 0), None);
    assert_eq!(scale_note(&deploy, 3, 1), None);
    assert_eq!(scale_note(&deploy, 1, 3), None);
    assert_eq!(
        scale_note(&sts, 3, 1),
        Some("Pods with the highest ordinals are removed; their volume claims are kept.")
    );
    assert_eq!(scale_note(&sts, 1, 3), None);
    assert_eq!(replicas_text(Replicas { desired: 3, current: 2 }), "Now 3 desired, 2 running.");
}

#[test]
fn range_texts() {
    assert_eq!(range_intro(true), "Change how many Jobs at once ");
    assert_eq!(range_intro(false), "Change the replica range of ");
    assert!(range_rule(true).starts_with("Min can't be below 0"));
    assert!(range_rule(false).starts_with("Min must be at least 1"));
    assert!(zero_note(true, false, Some((0, 5))));
    assert!(!zero_note(true, true, Some((0, 5))), "not for ScaledJobs");
    assert!(!zero_note(false, false, Some((0, 5))));
    assert!(!zero_note(true, false, Some((1, 5))));
    assert!(!zero_note(true, false, None));
    assert!(helm_note("").starts_with("Managed by Helm: the next"));
    assert!(helm_note("java").starts_with("Managed by Helm (release java): "));
    assert_eq!(
        owner_note("KEDA"),
        "It's controlled by KEDA, which sets the range itself: change it there, or this edit is undone."
    );
}

#[test]
fn rollback_and_uninstall_texts() {
    assert_eq!(listed(3), (3, 0));
    assert_eq!(listed(11), (8, 3));
    let revisions = [revision(3, "deployed", "shop-1.2.0", "2.0"), revision(2, "failed", "shop-1.1.0", "")];
    assert_eq!(revision_label(&revisions[0]), "3 · shop-1.2.0 (app 2.0) · deployed");
    assert_eq!(revision_label(&revisions[1]), "2 · shop-1.1.0 · failed");
    assert_eq!(current_revision(&revisions[0]), "Now at revision 3 (shop-1.2.0, deployed).");
    assert!(rollback_note(&revisions).contains("as a new revision 4 "));
    assert!(rollback_note(&[]).contains("as a new revision 0 "));
    assert!(chose_failed(&revisions, "2"));
    assert!(!chose_failed(&revisions, "3"));
    assert!(!chose_failed(&revisions, "9"));
    assert!(!chose_failed(&revisions, ""));
    assert_eq!(manual_job("nightly"), "nightly-manual-…");
}

#[test]
fn force_sync_reclaim_and_save_texts() {
    assert!(force_sync_text(true, 0).contains("will each ExternalSecret it makes"));
    assert!(force_sync_text(true, 1).contains("does its ExternalSecret."));
    assert!(force_sync_text(true, 3).contains("do its 3 ExternalSecrets"));
    assert!(force_sync_text(false, 0).ends_with("updates the Secret."));
    assert_eq!(reclaim_note("Delete").1, "warn-text");
    assert!(reclaim_note("Delete").0.contains("deleted too"));
    assert_eq!(reclaim_note("Retain").1, "muted");
    assert_eq!(key_list(&[]), None);
    assert_eq!(key_list(&["a".into(), "b".into()]).as_deref(), Some("a, b"));
}
