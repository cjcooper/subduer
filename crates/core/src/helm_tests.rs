//! Coverage tests for Helm releases: decoding, the release watch, details and
//! the summary text against the fake API server, and the checks that run
//! before `helm` would be started. The `helm` binary itself is never run.

use super::*;
use crate::testing::{MockApi, list, object};
use std::io::Write;
use std::sync::{Arc, Mutex};

const SHOP_SECRETS: &str = "/api/v1/namespaces/shop/secrets";
const ALL_SECRETS: &str = "/api/v1/secrets";

/// What Helm stores: base64 of gzipped JSON (the API adds its own base64).
fn helm_b64(release: &Value) -> String {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(release.to_string().as_bytes()).unwrap();
    base64::engine::general_purpose::STANDARD.encode(gz.finish().unwrap())
}

fn b64(s: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(s)
}

fn secret_name(release: &str, revision: i64) -> String {
    format!("sh.helm.release.v1.{release}.v{revision}")
}

/// A release Secret's metadata, as the list returns it.
fn release_meta(ns: &str, release: &str, revision: i64, status: &str, created: &str) -> Value {
    object("v1", "Secret", Some(ns), &secret_name(release, revision), json!({
        "type": "helm.sh/release.v1",
        "metadata": {"creationTimestamp": created, "resourceVersion": revision.to_string(),
                     "labels": {"owner": "helm", "name": release, "version": revision.to_string(), "status": status}}}))
}

/// The full Secret, with the release payload.
fn release_secret(ns: &str, release: &str, revision: i64, status: &str, payload: &Value) -> Value {
    let mut v = release_meta(ns, release, revision, status, "2026-10-01T00:00:00Z");
    v["data"] = json!({"release": b64(&helm_b64(payload))});
    v
}

fn payload(chart: &str, version: &str, app: &str, info: Value) -> Value {
    json!({"name": chart, "chart": {"metadata": {"name": chart, "version": version, "appVersion": app,
                                                 "description": "The acme web app", "home": "https://charts.acme.io/web"},
                                    "templates": [{"name": "t.yaml", "data": "c2VjcmV0"}], "values": {"password": "hunter2"}},
           "config": {"password": "hunter2"}, "hooks": [{"name": "h"}],
           "manifest": "---\napiVersion: v1\nkind: Secret\nmetadata:\n  name: web-creds\ndata:\n  password: aHVudGVyMg==\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: shop\n",
           "info": info})
}

fn ok_info(description: &str) -> Value {
    json!({"description": description, "status": "deployed", "first_deployed": "2026-09-01T00:00:00Z",
           "last_deployed": "2026-10-01T00:00:00Z", "notes": "  Visit https://shop.acme.io   \n\nThanks!  \n\n"})
}

// ---- the virtual kind and discovery ----

#[tokio::test]
async fn release_kind_and_discovery() {
    let kind = release_kind();
    assert!(is_release(&kind));
    assert_eq!((kind.plural.as_str(), kind.namespaced), ("releases", true));
    assert!(!is_release(&crate::testing::kind("", "v1", "Secret", "secrets", true)));
    assert!(!is_release(&crate::testing::kind("helm.sh", "v3", "Chart", "charts", true)));

    let api = MockApi::new();
    api.on("GET", ALL_SECRETS, list("SecretList", vec![release_meta("shop", "web", 1, "deployed", "2026-10-01T00:00:00Z")]));
    let found = discover(&api.client()).await.unwrap();
    assert!(is_release(&found.kind));
    assert_eq!(found.verbs, ["list", "watch", "get"]);
    assert!(matches!(found.category, Category::Helm));
    let q = &api.calls_to("GET", ALL_SECRETS)[0].query;
    assert!(q.contains("labelSelector=owner%3Dhelm") && q.contains("limit=1"), "{q}");

    let api = MockApi::new();
    api.on("GET", ALL_SECRETS, list("SecretList", vec![]));
    assert!(discover(&api.client()).await.is_none());

    let api = MockApi::new();
    api.on_error("GET", ALL_SECRETS, 403, "Forbidden", "secrets is forbidden");
    assert!(discover(&api.client()).await.is_none());
}

// ---- decoding ----

#[test]
fn decode_plain_and_broken_payloads() {
    let plain = base64::engine::general_purpose::STANDARD.encode(br#"{"chart":{"metadata":{"name":"web"}}}"#);
    let v = decode(plain.as_bytes()).unwrap();
    // No version: just the name.
    assert_eq!(chart_info(&v), ChartInfo { chart: "web".into(), app_version: String::new() });
    assert!(decode(b"%%% not base64").is_none());
    let bad_gzip = base64::engine::general_purpose::STANDARD.encode([0x1f, 0x8b, 0x00, 0x01]);
    assert!(decode(bad_gzip.as_bytes()).is_none());
    let not_json = base64::engine::general_purpose::STANDARD.encode("not json");
    assert!(decode(not_json.as_bytes()).is_none());
    assert_eq!(chart_info(&Value::Null), ChartInfo::default());
}

#[test]
fn manifest_documents_without_kind_or_name_are_skipped() {
    let manifest = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: first\n---\nkind: Service\nmetadata:\n  name: no-version\n---\napiVersion: v1\nkind: Service\n---\n: : not yaml\n";
    let objects = manifest_objects(manifest);
    assert_eq!(
        objects,
        [ManifestObject { api_version: "v1".into(), kind: "ConfigMap".into(), name: "first".into(), namespace: None }]
    );
    assert!(manifest_objects("").is_empty());
}

#[test]
fn revisions_from_secret_metadata() {
    let meta = |v: Value| -> PartialObjectMeta<Secret> { serde_json::from_value(v).unwrap() };
    let r = Revision::from_meta(&meta(release_meta("shop", "web", 4, "deployed", "2026-10-02T03:04:05Z"))).unwrap();
    assert_eq!(
        r,
        Revision {
            namespace: "shop".into(),
            secret: "sh.helm.release.v1.web.v4".into(),
            release: "web".into(),
            revision: 4,
            status: "deployed".into(),
            updated: "2026-10-02T03:04:05Z".into(),
        }
    );
    // No status label: empty; no creation time: empty.
    let r = Revision::from_meta(&meta(json!({"metadata": {"name": "s", "namespace": "shop", "labels": {"name": "web", "version": "1"}}}))).unwrap();
    assert_eq!((r.status.as_str(), r.updated.as_str()), ("", ""));
    // Not a usable revision.
    for bad in [
        json!({"metadata": {"name": "s", "labels": {"name": "web", "version": "1"}}}),
        json!({"metadata": {"name": "s", "namespace": "shop", "labels": {"version": "1"}}}),
        json!({"metadata": {"name": "s", "namespace": "shop", "labels": {"name": "web"}}}),
        json!({"metadata": {"name": "s", "namespace": "shop", "labels": {"name": "web", "version": "v1"}}}),
    ] {
        assert!(Revision::from_meta(&meta(bad.clone())).is_none(), "{bad}");
    }
}

#[test]
fn table_columns_and_rows_without_chart_info() {
    let cols = columns();
    let headers: Vec<&str> = cols.iter().map(|c| c.header.as_str()).collect();
    assert_eq!(headers, ["Name", "Namespace", "Chart", "App Version", "Revision", "Status", "Updated"]);
    assert!(matches!(cols[4].kind, ColumnKind::Number));
    assert!(matches!(cols[6].kind, ColumnKind::Age));

    let r = Revision {
        namespace: "shop".into(),
        secret: secret_name("web", 1),
        release: "web".into(),
        revision: 1,
        status: "pending-install".into(),
        updated: "2026-10-01T00:00:00Z".into(),
    };
    let rows = rows(&BTreeMap::from([(secret_key("shop", &r.secret), r)]), &HashMap::new());
    assert_eq!(rows[0].cells, ["web", "shop", "", "", "1", "pending-install", "2026-10-01T00:00:00Z"]);
    assert_eq!((rows[0].name.as_str(), rows[0].namespace.as_deref()), ("web", Some("shop")));
}

// ---- summary text ----

fn history_api(release: &str, revisions: &[(i64, &str)]) -> MockApi {
    let api = MockApi::new();
    let items = revisions
        .iter()
        .map(|(n, s)| release_meta("shop", release, *n, s, &format!("2026-10-{:02}T00:00:00Z", n)))
        .collect();
    api.on("GET", SHOP_SECRETS, list("SecretList", items));
    api
}

#[tokio::test]
async fn release_summary_yaml() {
    let api = history_api("web", &[(1, "superseded"), (3, "deployed"), (2, "superseded")]);
    api.on("GET", &format!("{SHOP_SECRETS}/{}", secret_name("web", 3)),
        release_secret("shop", "web", 3, "deployed", &payload("web", "1.3.0", "2.0", ok_info("Upgrade complete"))));
    let text = release_yaml(&api.client(), "shop", "web").await.unwrap();

    let q = &api.calls_to("GET", SHOP_SECRETS)[0].query;
    assert!(q.contains("labelSelector=owner%3Dhelm%2Cname%3Dweb"), "{q}");
    // Only the latest revision's payload is read.
    assert_eq!(api.calls().len(), 2);
    assert!(text.starts_with("# Helm release summary. Values and manifest are not shown.\n"));
    let keys: Vec<&str> = text.lines().filter(|l| !l.is_empty() && !l.starts_with([' ', '-', '#'])).filter_map(|l| l.split(':').next()).collect();
    assert_eq!(
        keys,
        ["release", "namespace", "revision", "status", "chart", "chartVersion", "appVersion", "description", "firstDeployed", "lastDeployed", "history", "notes"]
    );
    let doc: serde_norway::Value = serde_norway::from_str(&text).unwrap();
    assert_eq!(doc["revision"].as_i64(), Some(3));
    assert_eq!(doc["chartVersion"].as_str(), Some("1.3.0"));
    assert_eq!(doc["notes"].as_str(), Some("  Visit https://shop.acme.io\n\nThanks!"));
    let history: Vec<i64> = doc["history"].as_sequence().unwrap().iter().map(|h| h["revision"].as_i64().unwrap()).collect();
    assert_eq!(history, [3, 2, 1]);
    assert_eq!(doc["history"][1]["updated"].as_str(), Some("2026-10-02T00:00:00Z"));
    // Values, manifest and hooks never make it out.
    for secret in ["hunter2", "aHVudGVy", "manifest:", "config", "hooks", "web-creds", "templates"] {
        assert!(!text.contains(secret), "{secret} leaked:\n{text}");
    }
}

#[tokio::test]
async fn release_summary_without_a_readable_payload() {
    // The latest Secret can't be read: the labels still give a summary.
    let api = history_api("web", &[(1, "failed")]);
    let text = release_yaml(&api.client(), "shop", "web").await.unwrap();
    let doc: serde_norway::Value = serde_norway::from_str(&text).unwrap();
    assert_eq!(doc["status"].as_str(), Some("failed"));
    assert!(doc.get("chart").is_none() && doc.get("notes").is_none() && doc.get("description").is_none());

    // A Secret without the release key, and one with an empty description.
    let api = history_api("web", &[(1, "deployed")]);
    let mut s = release_meta("shop", "web", 1, "deployed", "2026-10-01T00:00:00Z");
    s["data"] = json!({"other": b64("x")});
    api.on("GET", &format!("{SHOP_SECRETS}/{}", secret_name("web", 1)), s);
    let doc: serde_norway::Value = serde_norway::from_str(&release_yaml(&api.client(), "shop", "web").await.unwrap()).unwrap();
    assert!(doc.get("chart").is_none());

    let api = history_api("web", &[(1, "deployed")]);
    api.on("GET", &format!("{SHOP_SECRETS}/{}", secret_name("web", 1)),
        release_secret("shop", "web", 1, "deployed", &json!({"chart": {"metadata": {"name": "web", "version": ""}}, "info": {"description": ""}})));
    let doc: serde_norway::Value = serde_norway::from_str(&release_yaml(&api.client(), "shop", "web").await.unwrap()).unwrap();
    assert_eq!(doc["chart"].as_str(), Some("web"));
    assert!(doc.get("chartVersion").is_none() && doc.get("description").is_none());

    // A payload without a chart at all.
    let api = history_api("web", &[(1, "deployed")]);
    api.on("GET", &format!("{SHOP_SECRETS}/{}", secret_name("web", 1)),
        release_secret("shop", "web", 1, "deployed", &json!({"info": {"description": "Install complete"}, "manifest": ""})));
    let doc: serde_norway::Value = serde_norway::from_str(&release_yaml(&api.client(), "shop", "web").await.unwrap()).unwrap();
    assert_eq!((doc.get("chart"), doc["description"].as_str()), (None, Some("Install complete")));
}

#[tokio::test]
async fn unknown_release_and_api_errors() {
    let api = history_api("web", &[]);
    let err = release_yaml(&api.client(), "shop", "web").await.unwrap_err();
    assert!(matches!(&err, Error::NotFound(m) if m == "Helm release shop/web"), "{err}");
    let err = release_details(&api.client(), "shop", "web").await.unwrap_err();
    assert!(matches!(&err, Error::NotFound(m) if m == "Helm release shop/web"), "{err}");

    let api = MockApi::new();
    api.on_error("GET", SHOP_SECRETS, 403, "Forbidden", "secrets is forbidden");
    assert_eq!(release_yaml(&api.client(), "shop", "web").await.unwrap_err().user_message(), "secrets is forbidden");
    assert_eq!(release_details(&api.client(), "shop", "web").await.unwrap_err().user_message(), "secrets is forbidden");
}

// ---- details ----

#[tokio::test]
async fn release_details_from_the_api() {
    let api = history_api("web", &[(1, "failed"), (2, "superseded"), (3, "deployed")]);
    api.on("GET", &format!("{SHOP_SECRETS}/{}", secret_name("web", 3)),
        release_secret("shop", "web", 3, "deployed", &payload("web", "1.3.0", "2.0", ok_info("Upgrade complete"))));
    api.on("GET", &format!("{SHOP_SECRETS}/{}", secret_name("web", 2)),
        release_secret("shop", "web", 2, "superseded", &payload("web", "1.2.0", "1.9", json!({"description": "Install complete"}))));
    // Revision 1's Secret is gone: its row has no chart.
    let d = release_details(&api.client(), "shop", "web").await.unwrap();

    assert_eq!((d.kind.as_str(), d.name.as_str(), d.namespace.as_deref()), ("Release", "web", Some("shop")));
    assert_eq!(d.status.as_deref(), Some("deployed"));
    assert_eq!(d.created.as_deref(), Some("2026-10-01T00:00:00Z"));
    let rows: Vec<(&str, &str)> = d.info.iter().map(|r| (r.key.as_str(), r.value.as_str())).collect();
    assert_eq!(
        rows,
        [
            ("Chart", "web-1.3.0"),
            ("App version", "2.0"),
            ("Revision", "3"),
            ("Last change", "Upgrade complete"),
            ("First deployed", "2026-09-01T00:00:00Z"),
            ("Last deployed", "2026-10-01T00:00:00Z"),
            ("About the chart", "The acme web app"),
            ("Chart home", "https://charts.acme.io/web"),
            ("Values", "not shown: they can hold secrets"),
        ]
    );
    assert!(d.info[0].mono && d.info[4].time && d.info[5].time);
    assert_eq!(d.info[7].url.as_deref(), Some("https://charts.acme.io/web"));

    let history = d.sections[0].table.as_ref().unwrap();
    assert_eq!(d.sections[0].summary, "3 revisions");
    let h: Vec<Vec<&str>> = history.rows.iter().map(|r| r.cells.iter().map(String::as_str).collect()).collect();
    assert_eq!(
        h,
        [
            vec!["3", "deployed", "web-1.3.0", "2.0", "2026-10-03T00:00:00Z", "Upgrade complete"],
            vec!["2", "superseded", "web-1.2.0", "1.9", "2026-10-02T00:00:00Z", "Install complete"],
            vec!["1", "failed", "", "", "2026-10-01T00:00:00Z", ""],
        ]
    );
    assert_eq!(history.rows.iter().map(|r| r.warn).collect::<Vec<_>>(), [false, false, true]);

    let res = &d.sections[1];
    assert_eq!((res.id.as_str(), res.summary.as_str()), ("helm-resources", "2 objects"));
    let t = res.table.as_ref().unwrap();
    assert_eq!(t.rows[0].cells, ["Secret", "web-creds", ""]);
    let l = t.rows[0].link.as_ref().unwrap();
    assert_eq!((l.kind.as_str(), l.namespace.as_deref(), l.name.as_str()), ("core/Secret", Some("shop"), "web-creds"));
    assert_eq!(t.rows[1].link.as_ref().unwrap().kind, "apps/Deployment");

    let notes = &d.sections[2];
    assert_eq!((notes.id.as_str(), notes.rows[0].value.as_str()), ("helm-notes", "  Visit https://shop.acme.io\n\nThanks!"));
    assert!(notes.rows[0].mono);
    assert!(!format!("{d:?}").contains("hunter2"));
}

#[tokio::test]
async fn release_details_reads_at_most_twenty_revisions() {
    let revisions: Vec<(i64, &str)> = (1..=25).map(|n| (n, if n == 25 { "deployed" } else { "superseded" })).collect();
    let api = history_api("web", &revisions);
    let d = release_details(&api.client(), "shop", "web").await.unwrap();
    let gets = api.calls().iter().filter(|c| c.path.starts_with(&format!("{SHOP_SECRETS}/"))).count();
    assert_eq!(gets, 20);
    assert_eq!(d.sections[0].table.as_ref().unwrap().rows.len(), 25);
    assert_eq!(d.sections[0].summary, "25 revisions");
}

#[test]
fn release_details_with_nothing_decoded() {
    let r = Revision {
        namespace: "shop".into(),
        secret: secret_name("web", 1),
        release: "web".into(),
        revision: 1,
        status: "deployed".into(),
        updated: "2026-10-01T00:00:00Z".into(),
    };
    let d = describe_release("shop", "web", &[r], &HashMap::new(), &[]);
    let keys: Vec<&str> = d.info.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(keys, ["Revision", "Values"]);
    assert_eq!(d.sections.len(), 2);
    let res = &d.sections[1];
    assert_eq!((res.summary.as_str(), res.empty.as_deref()), ("none", Some("Its manifest lists no objects.")));
    assert_eq!(d.sections[0].summary, "1 revision");

    // Cluster-scoped objects from a release in "shop" link into it too.
    let objects = [ManifestObject { api_version: "rbac.authorization.k8s.io/v1".into(), kind: "ClusterRole".into(), name: "web".into(), namespace: None }];
    let d = describe_release("shop", "web", &[d_rev()], &HashMap::new(), &objects);
    assert_eq!(d.sections[1].summary, "1 object");
    assert_eq!(d.sections[1].table.as_ref().unwrap().rows[0].link.as_ref().unwrap().kind, "rbac.authorization.k8s.io/ClusterRole");
}

fn d_rev() -> Revision {
    Revision {
        namespace: "shop".into(),
        secret: secret_name("web", 1),
        release: "web".into(),
        revision: 1,
        status: "deployed".into(),
        updated: String::new(),
    }
}

// ---- the release watch ----

type Seen = Arc<Mutex<Vec<ResourceTable>>>;

async fn watch_for(client: Client, namespace: Option<&str>, millis: u64) -> Vec<ResourceTable> {
    let seen: Seen = Arc::default();
    let sink = seen.clone();
    let _ = tokio::time::timeout(
        Duration::from_millis(millis),
        watch_releases(client, namespace.map(str::to_owned), 42, move |t| sink.lock().unwrap().push(t)),
    )
    .await;
    seen.lock().unwrap().clone()
}

fn by_release(t: &ResourceTable) -> Vec<(String, String, String)> {
    t.rows.iter().map(|r| (r.id.clone(), r.cells[2].clone(), r.cells[4].clone())).collect()
}

fn watch_event(kind: &str, object: Value) -> Value {
    json!({"type": kind, "object": object})
}

#[tokio::test]
async fn release_watch_lists_fills_charts_and_follows_changes() {
    let api = MockApi::new();
    let mut listed = list("SecretList", vec![
        release_meta("shop", "web", 1, "superseded", "2026-10-01T00:00:00Z"),
        release_meta("shop", "web", 2, "deployed", "2026-10-02T00:00:00Z"),
        release_meta("shop", "db", 1, "deployed", "2026-10-01T00:00:00Z"),
        // Not a release revision: ignored.
        object("v1", "Secret", Some("shop"), "stray", json!({"metadata": {"labels": {"owner": "helm"}}})),
    ]);
    listed["metadata"]["resourceVersion"] = json!("1");
    api.on("GET", SHOP_SECRETS, listed);
    // Revision 3 of web arrives, then is deleted again (a failed upgrade cleaned up).
    let mut v3 = release_meta("shop", "web", 3, "failed", "2026-10-03T00:00:00Z");
    v3["metadata"]["resourceVersion"] = json!("8");
    api.on_query("GET", SHOP_SECRETS, "resourceVersion=1", watch_event("ADDED", v3.clone()));
    v3["metadata"]["resourceVersion"] = json!("9");
    api.on_query("GET", SHOP_SECRETS, "resourceVersion=8", watch_event("DELETED", v3));
    // Afterwards the watch fails: the error is reported with the last rows.
    for (name, rev, chart, version) in [("web", 2, "web", "1.2.0"), ("web", 3, "web", "1.3.0"), ("db", 1, "postgres", "15.0.0")] {
        api.on("GET", &format!("{SHOP_SECRETS}/{}", secret_name(name, rev)),
            release_secret("shop", name, rev, "deployed", &payload(chart, version, "1", json!({}))));
    }

    let seen = watch_for(api.client(), Some("shop"), 2500).await;
    assert!(seen.iter().all(|t| t.watch_id == 42 && t.columns.len() == 7));

    let snapshots: Vec<Vec<(String, String, String)>> = seen.iter().filter(|t| t.error.is_none()).map(by_release).collect();
    let s = |id: &str, chart: &str, rev: &str| (id.to_owned(), chart.to_owned(), rev.to_owned());
    // The list shows straight away, before any chart is decoded; charts fill in after.
    assert!(snapshots[0].iter().all(|(_, chart, _)| chart.is_empty()), "{snapshots:?}");
    assert!(snapshots.iter().any(|t| t.contains(&s("shop/db", "postgres-15.0.0", "1"))), "{snapshots:?}");
    // Revision 3 was added (the watch went on from its version) and deleted again.
    assert_eq!(api.calls().iter().filter(|c| c.query.contains("resourceVersion=8")).count(), 1);
    assert!(api.calls().iter().any(|c| c.query.contains("resourceVersion=9")));
    // Then the watch fails: errors carry the last rows; web is back at revision 2.
    let errors: Vec<&ResourceTable> = seen.iter().filter(|t| t.error.is_some()).collect();
    assert!(!errors.is_empty(), "{seen:?}");
    assert!(errors.iter().all(|t| t.rows.len() == 2 && t.rows[1].cells[4] == "2"), "{errors:?}");
    assert_eq!(seen.last().map(by_release).unwrap(), [s("shop/db", "postgres-15.0.0", "1"), s("shop/web", "web-1.2.0", "2")]);
    // Each chart is decoded once.
    assert_eq!(api.calls_to("GET", &format!("{SHOP_SECRETS}/{}", secret_name("db", 1))).len(), 1);
    // The watch asked only for Helm's Secrets.
    let watches: Vec<_> = api.calls_to("GET", SHOP_SECRETS).into_iter().filter(|c| c.query.contains("watch=true")).collect();
    assert!(watches.iter().all(|c| c.query.contains("labelSelector=owner%3Dhelm")));
    assert!(watches.len() >= 3);
}

#[tokio::test]
async fn release_watch_reports_a_failing_list() {
    let api = MockApi::new();
    api.on_error("GET", ALL_SECRETS, 403, "Forbidden", "secrets is forbidden");
    let seen = watch_for(api.client(), None, 400).await;
    let first = seen.first().expect("an error table");
    assert!(first.rows.is_empty());
    assert!(first.error.as_deref().unwrap().contains("secrets is forbidden"), "{:?}", first.error);
    assert!(api.calls_to("GET", ALL_SECRETS)[0].query.contains("labelSelector=owner%3Dhelm"));
}

#[tokio::test]
async fn release_watch_with_unreadable_payloads_still_lists() {
    let api = MockApi::new();
    let meta = release_meta("acme", "web", 1, "deployed", "2026-10-01T00:00:00Z");
    api.on("GET", ALL_SECRETS, list("SecretList", vec![meta.clone()]));
    // A label change on a revision already shown: no payload is read again.
    let mut changed = meta;
    changed["metadata"]["resourceVersion"] = json!("2");
    changed["metadata"]["labels"]["status"] = json!("superseded");
    // It arrives once the first charts are decoded: the watch fails until then.
    let later = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        api.on_query("GET", ALL_SECRETS, "resourceVersion=1", watch_event("MODIFIED", changed));
    };
    let (seen, ()) = tokio::join!(watch_for(api.client(), None, 2000), later);
    let ok: Vec<&ResourceTable> = seen.iter().filter(|t| t.error.is_none()).collect();
    assert!(ok.len() >= 3, "{seen:?}");
    assert_eq!(ok[1].rows[0].cells[5], "deployed");
    assert_eq!(ok.last().unwrap().rows[0].cells, ["web", "acme", "", "", "1", "superseded", "2026-10-01T00:00:00Z"]);
    assert_eq!(api.calls_to("GET", &format!("/api/v1/namespaces/acme/secrets/{}", secret_name("web", 1))).len(), 1);
}

// ---- rollback and uninstall: checked before helm would run ----

#[test]
fn helm_names() {
    for good in ["web", "0web", "web-1.2", "a"] {
        assert!(valid_name(good), "{good}");
    }
    for bad in ["", "-web", ".web", "Web", "web_1", "web 1", "web;rm", "--all"] {
        assert!(!valid_name(bad), "{bad}");
    }
    assert!(valid_name(&"a".repeat(253)));
    assert!(!valid_name(&"a".repeat(254)));
}

#[test]
fn rollback_and_uninstall_flag_combinations() {
    assert_eq!(rollback_args("acme-uat", "shop", "web", 7, false, false, false), ["rollback", "web", "7", "--namespace=shop", "--kube-context=acme-uat"]);
    assert_eq!(
        rollback_args("acme-uat", "shop", "web", 7, true, false, false),
        ["rollback", "web", "7", "--namespace=shop", "--kube-context=acme-uat", "--wait", "--timeout", "5m0s"]
    );
    assert_eq!(uninstall_args("acme-uat", "shop", "web", false, true, false)[4..], ["--wait", "--timeout", "5m0s"]);
    assert_eq!(uninstall_args("acme-uat", "shop", "web", true, false, false)[4..], ["--keep-history"]);
    assert_eq!(uninstall_args("acme-uat", "shop", "web", false, false, true)[4..], ["--dry-run"]);
}

#[tokio::test]
async fn rollback_and_uninstall_refuse_before_running_helm() {
    let gate = SafetyGate::default();
    let invalid = "not a valid release or namespace name";
    // Bad namespaces are refused like bad release names, dry run or not.
    for (ns, release) in [("-n", "web"), ("Shop", "web"), ("shop", "")] {
        let err = rollback(&gate, "acme-uat", ns, release, 1, false, true).await.unwrap_err();
        assert_eq!(err.to_string(), invalid);
        let err = uninstall(&gate, "acme-uat", ns, release, false, false, true).await.unwrap_err();
        assert_eq!(err.to_string(), invalid);
    }
    let err = rollback(&gate, "acme-uat", "shop", "web", -1, false, false).await.unwrap_err();
    assert_eq!(err.to_string(), "pick a revision to roll back to");

    // Locked contexts refuse real changes; unlocking another context doesn't help.
    gate.set_unlocked("acme-uat", true);
    let err = rollback(&gate, "acme-prd", "shop", "web", 1, true, false).await.unwrap_err();
    assert!(matches!(&err, Error::Locked(c) if c == "acme-prd"));
    let err = uninstall(&gate, "acme-prd", "shop", "web", true, true, false).await.unwrap_err();
    assert!(matches!(&err, Error::Locked(c) if c == "acme-prd"));
}
