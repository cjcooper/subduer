use super::*;
use crate::app::tests_unit::{context, kind, row, store, store_on};
use subduer_types::{InfoRow, InfoSection, InfoTable, InfoTableRow, ObjectDetails, Usage, WorkloadDetails};

const ALL: [Op; 11] = [
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
];

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
            "Delete"
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
    for op in ALL {
        assert!(!op.hint(&job).is_empty());
    }
}

#[test]
fn suspend_notes() {
    let cron = kind("batch", "CronJob");
    let job = kind("batch", "Job");
    assert!(suspend_note(&cron, true).starts_with("No new Jobs start"));
    assert!(suspend_note(&cron, false).starts_with("Jobs start on its schedule again"));
    assert!(suspend_note(&job, true).starts_with("Its running pods are stopped"));
    assert_eq!(suspend_note(&job, false), "New pods start for the work that's left.");
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
}

fn cron_job(name: &str, status: &str, concurrency: Option<&str>) -> WorkloadDetails {
    WorkloadDetails {
        kind: "CronJob".into(),
        name: name.into(),
        namespace: "shop".into(),
        status: status.into(),
        replicas: None,
        suspended: Some(false),
        info: concurrency.map(|c| vec![("Concurrency".to_owned(), c.to_owned())]).unwrap_or_default(),
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

#[test]
fn run_note_follows_the_concurrency_policy() {
    let s = store();
    assert_eq!(run_note(s, "nightly"), None, "no details loaded");

    s.workload.set(Some(Ok(cron_job("nightly", "Scheduled", Some("Forbid")))));
    assert_eq!(cron_state(s, "nightly"), Some(("Forbid".into(), false)));
    assert_eq!(run_note(s, "nightly"), Some("Scheduled runs are skipped while this Job runs (concurrency: Forbid)."));
    assert_eq!(run_note(s, "other"), None, "details of another object");

    s.workload.set(Some(Ok(cron_job("nightly", "Running", Some("Replace")))));
    assert_eq!(cron_state(s, "nightly"), Some(("Replace".into(), true)));
    assert!(run_note(s, "nightly").unwrap().starts_with("Its running Job is not replaced"));

    s.workload.set(Some(Ok(cron_job("nightly", "Scheduled", Some("Replace")))));
    assert!(run_note(s, "nightly").unwrap().starts_with("The next scheduled run replaces this Job"));

    s.workload.set(Some(Ok(cron_job("nightly", "Scheduled", Some("Allow")))));
    assert_eq!(run_note(s, "nightly"), None);

    s.workload.set(Some(Ok(cron_job("nightly", "Scheduled", None))));
    assert_eq!(cron_state(s, "nightly"), None, "no concurrency row");

    let mut job = cron_job("nightly", "Running", Some("Forbid"));
    job.kind = "Job".into();
    s.workload.set(Some(Ok(job)));
    assert_eq!(cron_state(s, "nightly"), None, "not a CronJob");

    s.workload.set(Some(Err("boom".into())));
    assert_eq!(cron_state(s, "nightly"), None);
}

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
    }
}

#[test]
fn history_reads_the_release_revisions() {
    let s = store();
    assert!(history(s, "shop", false).is_empty());
    s.object.set(Some(Ok(object(
        "Release",
        "shop",
        vec![],
        vec![
            section("helm-resources", vec![table_row(&["Deployment", "shop-web"])]),
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
    ))));
    let expected = vec![
        (3, "deployed".to_owned(), "shop-1.2.0".to_owned(), "2.0".to_owned(), "2026-01-03".to_owned()),
        (1, "superseded".to_owned(), "shop-1.0.0".to_owned(), "1.0".to_owned(), "2026-01-01".to_owned()),
    ];
    assert_eq!(history(s, "shop", false), expected, "malformed rows are skipped");
    assert_eq!(history(s, "shop", true), expected);
    assert!(history(s, "acme", false).is_empty(), "another release");
    assert_eq!(release_objects(s, "shop"), [("Deployment".to_owned(), "shop-web".to_owned())]);
    assert!(release_objects(s, "acme").is_empty());
}

#[test]
fn history_ignores_other_kinds_and_missing_sections() {
    let s = store();
    s.object.set(Some(Ok(object("Release", "shop", vec![], vec![]))));
    assert!(history(s, "shop", false).is_empty());
    assert!(release_objects(s, "shop").is_empty());
    s.object.set(Some(Ok(object(
        "ConfigMap",
        "shop",
        vec![],
        vec![section("helm-history", vec![table_row(&["1", "a", "b", "c", "d"])])],
    ))));
    assert!(history(s, "shop", false).is_empty());
    assert!(release_objects(s, "shop").is_empty());
}

#[test]
fn object_row_reads_an_info_row() {
    let s = store();
    assert_eq!(object_row(s, "web", "Min replicas"), None);
    s.object.set(Some(Ok(object(
        "HorizontalPodAutoscaler",
        "web",
        vec![info_row("Min replicas", "2"), info_row("Max replicas", "10")],
        vec![],
    ))));
    assert_eq!(object_row(s, "web", "Min replicas").as_deref(), Some("2"));
    assert_eq!(object_row(s, "web", "Max replicas").as_deref(), Some("10"));
    assert_eq!(object_row(s, "web", "Target"), None);
    assert_eq!(object_row(s, "api", "Min replicas"), None);
}

#[test]
fn request_opens_the_dialog_when_unlocked() {
    let s = store_on(true);
    let r = row(Some("shop"), "web");
    request(s, kind("apps", "Deployment"), &r, Op::Restart);
    let target = s.workload_prompt.get_untracked().expect("dialog open");
    assert_eq!(target.context, context("acme-dev", true));
    assert_eq!((target.namespace.as_str(), target.name.as_str(), target.op), ("shop", "web", Op::Restart));
    assert_eq!(target.save, None);
    assert_eq!(s.unlock_prompt.get_untracked(), None);
}

#[test]
fn request_asks_to_unlock_first_when_locked() {
    let s = store_on(false);
    // Cluster-scoped objects have no namespace.
    request(s, kind("", "PersistentVolume"), &row(None, "pv-1"), Op::Reclaim);
    assert_eq!(s.workload_prompt.get_untracked(), None);
    let pending = s.pending_workload.get_untracked().expect("waits for unlock");
    assert_eq!((pending.namespace.as_str(), pending.name.as_str()), ("", "pv-1"));
    assert_eq!(s.unlock_prompt.get_untracked(), Some(context("acme-dev", false)));
}

#[test]
fn request_save_carries_the_edit() {
    let s = store_on(true);
    let save = SaveData {
        resource_version: "42".into(),
        changes: vec![DataChange { key: "a".into(), value: None }],
        added: vec![],
        changed: vec![],
        removed: vec!["a".into()],
    };
    request_save(s, kind("", "ConfigMap"), "shop".into(), "cfg".into(), save.clone());
    let target = s.workload_prompt.get_untracked().unwrap();
    assert_eq!(target.op, Op::Save);
    assert_eq!(target.save, Some(save));
}

#[test]
fn request_does_nothing_without_a_context() {
    let s = store();
    request(s, kind("apps", "Deployment"), &row(Some("shop"), "web"), Op::Scale);
    assert_eq!(s.workload_prompt.get_untracked(), None);
    assert_eq!(s.pending_workload.get_untracked(), None);
}
