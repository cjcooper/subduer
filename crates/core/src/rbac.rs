//! The Details tab of the Access Control section: ServiceAccounts (cloud
//! identity, bindings, what they can do, the pods using them), Roles and
//! ClusterRoles (rules with risky ones flagged, who they're bound to), and
//! RoleBindings and ClusterRoleBindings (subjects and what they're granted).
//! Plus "Check access": a SubjectAccessReview for a ServiceAccount, which
//! asks the API server and changes nothing.

use std::collections::HashMap;

use k8s_openapi::api::authorization::v1::{ResourceAttributes, SubjectAccessReview, SubjectAccessReviewSpec};
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding, PolicyRule, Role, RoleBinding, RoleRef, Subject};
use kube::api::{Api, ListParams, PostParams};
use kube::{Client, ResourceExt};
use serde_json::Value;

use crate::config::{label_selector, plural, str_at, table_section};
use crate::types::{AccessCheck, InfoRow, InfoTable, InfoTableRow, ObjectDetails, ObjectLink};
use crate::{Error, Result};

const ROLE: &str = "rbac.authorization.k8s.io/Role";
const CLUSTER_ROLE: &str = "rbac.authorization.k8s.io/ClusterRole";
const ROLE_BINDING: &str = "rbac.authorization.k8s.io/RoleBinding";
const CLUSTER_ROLE_BINDING: &str = "rbac.authorization.k8s.io/ClusterRoleBinding";

/// Fills in what comes from the object itself.
pub(crate) fn describe(key: &str, d: &mut ObjectDetails, v: &Value) {
    match key {
        "core/ServiceAccount" => service_account(d, v),
        ROLE | CLUSTER_ROLE => role(d, v),
        ROLE_BINDING | CLUSTER_ROLE_BINDING => binding(d, v),
        _ => {}
    }
}

/// Adds what needs other objects.
pub(crate) async fn related(
    client: &Client,
    key: &str,
    namespace: &str,
    name: &str,
    v: &Value,
    d: &mut ObjectDetails,
) -> Result<()> {
    match key {
        "core/ServiceAccount" => account_related(client, namespace, name, d).await,
        ROLE | CLUSTER_ROLE => role_bindings(client, key == CLUSTER_ROLE, namespace, name, d).await,
        ROLE_BINDING | CLUSTER_ROLE_BINDING => granted(client, namespace, v, d).await,
        _ => Ok(()),
    }
}

fn link(kind: &str, namespace: Option<&str>, name: &str) -> ObjectLink {
    ObjectLink { kind: kind.into(), namespace: namespace.map(str::to_owned), name: name.into() }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect()
}

fn join(items: &[String], empty: &str) -> String {
    if items.is_empty() { empty.to_owned() } else { items.join(", ") }
}

// ---- Rules ----

/// Why a rule deserves a second look, if it does.
fn risk(verbs: &[String], groups: &[String], resources: &[String]) -> Option<&'static str> {
    let all = |v: &[String]| v.iter().any(|x| x == "*");
    let has = |v: &[String], x: &str| v.iter().any(|y| y == x);
    if all(verbs) && all(resources) && (all(groups) || groups.is_empty()) {
        return Some("full access");
    }
    if ["escalate", "bind", "impersonate"].iter().any(|x| has(verbs, x)) {
        return Some("can grant itself more");
    }
    let reads = all(verbs) || ["get", "list", "watch"].iter().any(|x| has(verbs, x));
    if reads && (has(resources, "secrets") || all(resources)) && (groups.is_empty() || has(groups, "") || all(groups)) {
        return Some("reads secrets");
    }
    if (all(verbs) || has(verbs, "create")) && (has(resources, "pods/exec") || all(resources)) {
        return Some("can exec into pods");
    }
    // The kubelet API through the API server: includes running commands in pods.
    if has(resources, "nodes/proxy") {
        return Some("reaches the kubelet");
    }
    None
}

/// Rules from JSON, one row each: verbs, groups, resources, names; risky ones flagged.
fn rules_table(rules: &[Value]) -> (InfoTable, Option<&'static str>) {
    let mut worst = None;
    let rows = rules
        .iter()
        .map(|r| {
            let verbs = strings(&r["verbs"]);
            let urls = strings(&r["nonResourceURLs"]);
            let groups: Vec<String> = strings(&r["apiGroups"]).into_iter().map(|g| if g.is_empty() { "core".into() } else { g }).collect();
            let resources = strings(&r["resources"]);
            let what = if urls.is_empty() { join(&resources, "—") } else { format!("URLs {}", urls.join(", ")) };
            let flag = risk(&verbs, &strings(&r["apiGroups"]), &resources);
            if flag == Some("full access") || worst.is_none() {
                worst = flag.or(worst);
            }
            let mut row = InfoTableRow::new(vec![
                join(&verbs, "—"),
                join(&groups, "—"),
                what,
                join(&strings(&r["resourceNames"]), "any"),
                flag.unwrap_or_default().to_owned(),
            ]);
            row.warn = flag.is_some();
            row
        })
        .collect();
    let table = InfoTable {
        columns: ["Verbs", "API groups", "Resources", "Names", "Note"].map(String::from).to_vec(),
        mono: vec![0, 2],
        rows,
        ..Default::default()
    };
    (table, worst)
}

fn role(d: &mut ObjectDetails, v: &Value) {
    let rules: Vec<Value> = v["rules"].as_array().cloned().unwrap_or_default();
    let (table, worst) = rules_table(&rules);
    d.info.push(InfoRow::new("Rules", rules.len().to_string()));
    let aggregation = &v["aggregationRule"]["clusterRoleSelectors"];
    if aggregation.is_array() {
        let selectors: Vec<String> = aggregation.as_array().into_iter().flatten().filter_map(label_selector).collect();
        d.info.push(InfoRow::new("Aggregates", format!("rules of ClusterRoles with {}", selectors.join(" or "))).mono());
    }
    let labels = &v["metadata"]["labels"];
    let into: Vec<&str> = ["admin", "edit", "view"]
        .into_iter()
        .filter(|r| str_at(labels, &format!("rbac.authorization.k8s.io/aggregate-to-{r}")).as_deref() == Some("true"))
        .collect();
    if !into.is_empty() {
        d.info.push(InfoRow::new("Adds its rules to", into.join(", ")));
    }
    if d.name.starts_with("system:") {
        d.info.push(InfoRow::new("Built in", "yes: managed by Kubernetes"));
    }
    d.sections.push(table_section("rules", "Rules", table, "No rules: it grants nothing."));
    d.status = worst.map(|w| match w {
        "full access" => "Full access".to_owned(),
        _ => "Sensitive".to_owned(),
    });
}

// ---- Bindings ----

fn role_link(role_ref: &Value, namespace: Option<&str>) -> (String, ObjectLink) {
    let kind = str_at(role_ref, "kind").unwrap_or_default();
    let name = str_at(role_ref, "name").unwrap_or_default();
    let key = if kind == "ClusterRole" { CLUSTER_ROLE } else { ROLE };
    let ns = if kind == "ClusterRole" { None } else { namespace };
    (format!("{kind} {name}"), link(key, ns, &name))
}

fn subject_row(s: &Value, default_ns: Option<&str>) -> InfoTableRow {
    let kind = str_at(s, "kind").unwrap_or_default();
    let name = str_at(s, "name").unwrap_or_default();
    // Only ServiceAccounts live in a namespace; users and groups don't.
    let ns = (kind == "ServiceAccount").then(|| str_at(s, "namespace").unwrap_or_else(|| default_ns.unwrap_or_default().to_owned()));
    let mut row = InfoTableRow::new(vec![kind.clone(), name.clone(), ns.clone().unwrap_or_default()]);
    if kind == "ServiceAccount" {
        row.link = Some(link("core/ServiceAccount", ns.as_deref(), &name));
    }
    // Everyone, or everyone signed in.
    row.warn = kind == "Group" && matches!(name.as_str(), "system:authenticated" | "system:unauthenticated");
    row
}

fn binding(d: &mut ObjectDetails, v: &Value) {
    let ns = d.namespace.clone();
    let (text, role) = role_link(&v["roleRef"], ns.as_deref());
    let cluster_role = role.kind == CLUSTER_ROLE;
    d.info.push(InfoRow::new("Grants", text).link(role));
    let scope = match &ns {
        Some(n) if cluster_role => format!("only in namespace {n} (the ClusterRole's rules, limited to here)"),
        Some(n) => format!("only in namespace {n}"),
        None => "the whole cluster".into(),
    };
    d.info.push(InfoRow::new("Where", scope));
    let rows: Vec<InfoTableRow> = v["subjects"].as_array().into_iter().flatten().map(|s| subject_row(s, ns.as_deref())).collect();
    let open = rows.iter().any(|r| r.warn);
    let mut section = table_section(
        "subjects",
        "Subjects",
        InfoTable { columns: vec!["Kind".into(), "Name".into(), "Namespace".into()], rows, ..Default::default() },
        "No subjects: it grants nothing.",
    );
    section.summary = section.table.as_ref().map_or(String::new(), |t| plural(t.rows.len(), "subject"));
    d.sections.push(section);
    if open {
        d.status = Some("Everyone".into());
    }
}

/// The bound role's rules, or a note that the role doesn't exist.
async fn granted(client: &Client, namespace: &str, v: &Value, d: &mut ObjectDetails) -> Result<()> {
    let role_ref: RoleRef = serde_json::from_value(v["roleRef"].clone()).map_err(|e| Error::Message(e.to_string()))?;
    let rules = role_rules(client, &role_ref, namespace).await?;
    match rules {
        None => {
            d.status = Some("Role missing".into());
            d.info.push(InfoRow::new("Note", format!("{} {} doesn't exist, so this grants nothing", role_ref.kind, role_ref.name)));
        }
        Some(rules) => {
            let rules: Vec<Value> = rules.iter().filter_map(|r| serde_json::to_value(r).ok()).collect();
            let (table, worst) = rules_table(&rules);
            d.sections.insert(0, table_section("granted", "What it grants", table, "The role has no rules."));
            if d.status.is_none() {
                d.status = worst.map(|w| if w == "full access" { "Full access".into() } else { "Sensitive".into() });
            }
        }
    }
    Ok(())
}

async fn role_rules(client: &Client, role_ref: &RoleRef, namespace: &str) -> Result<Option<Vec<PolicyRule>>> {
    Ok(if role_ref.kind == "ClusterRole" {
        Api::<ClusterRole>::all(client.clone()).get_opt(&role_ref.name).await?.map(|r| r.rules.unwrap_or_default())
    } else {
        Api::<Role>::namespaced(client.clone(), namespace).get_opt(&role_ref.name).await?.map(|r| r.rules.unwrap_or_default())
    })
}

/// Bindings that use this role, with their subjects.
async fn role_bindings(client: &Client, cluster: bool, namespace: &str, name: &str, d: &mut ObjectDetails) -> Result<()> {
    let wanted = if cluster { "ClusterRole" } else { "Role" };
    let mut rows = Vec::new();
    let subjects = |s: &Option<Vec<Subject>>| {
        s.iter().flatten().map(|s| format!("{} {}", s.kind, s.name)).collect::<Vec<_>>().join(", ")
    };
    let role_bindings = if cluster {
        Api::<RoleBinding>::all(client.clone()).list(&ListParams::default()).await?
    } else {
        Api::<RoleBinding>::namespaced(client.clone(), namespace).list(&ListParams::default()).await?
    };
    for b in role_bindings.items.iter().filter(|b| b.role_ref.kind == wanted && b.role_ref.name == name) {
        let ns = b.namespace().unwrap_or_default();
        let mut row = InfoTableRow::new(vec![b.name_any(), ns.clone(), subjects(&b.subjects)]);
        row.link = Some(link(ROLE_BINDING, Some(&ns), &b.name_any()));
        rows.push(row);
    }
    if cluster {
        let list = Api::<ClusterRoleBinding>::all(client.clone()).list(&ListParams::default()).await?;
        for b in list.items.iter().filter(|b| b.role_ref.name == name) {
            let mut row = InfoTableRow::new(vec![b.name_any(), "(cluster-wide)".into(), subjects(&b.subjects)]);
            row.link = Some(link(CLUSTER_ROLE_BINDING, None, &b.name_any()));
            rows.push(row);
        }
    }
    let table = InfoTable { columns: vec!["Binding".into(), "Where".into(), "Subjects".into()], rows, ..Default::default() };
    let mut section = table_section("bound-by", "Bound by", table, "Not bound: it grants nothing to anyone.");
    if section.table.as_ref().is_some_and(|t| t.rows.is_empty()) {
        section.summary = "none".into();
    }
    d.sections.push(section);
    Ok(())
}

// ---- ServiceAccount ----

fn service_account(d: &mut ObjectDetails, v: &Value) {
    let ns = d.namespace.clone().unwrap_or_default();
    let annotations = &v["metadata"]["annotations"];
    for (key, label) in [
        ("eks.amazonaws.com/role-arn", "AWS IAM role"),
        ("iam.gke.io/gcp-service-account", "Google service account"),
        ("azure.workload.identity/client-id", "Azure identity"),
    ] {
        if let Some(id) = str_at(annotations, key) {
            d.info.push(InfoRow::new(label, id).mono());
        }
    }
    let automount = v["automountServiceAccountToken"].as_bool().unwrap_or(true);
    d.info.push(InfoRow::new(
        "Token in pods",
        if automount { "mounted automatically (unless a pod opts out)" } else { "not mounted unless a pod asks" },
    ));
    let user = format!("system:serviceaccount:{ns}:{}", d.name);
    d.info.push(InfoRow::new("Acts as", user).mono());
    for (key, label) in [("secrets", "Token secret"), ("imagePullSecrets", "Image pull secret")] {
        for s in v[key].as_array().into_iter().flatten() {
            if let Some(name) = str_at(s, "name") {
                d.info.push(InfoRow::new(label, name.clone()).link(link("core/Secret", Some(&ns), &name)));
            }
        }
    }
}

/// Whether a binding subject means this ServiceAccount, and how.
fn matches_account(s: &Subject, namespace: &str, name: &str) -> Option<&'static str> {
    match s.kind.as_str() {
        "ServiceAccount" if s.name == name && s.namespace.as_deref().unwrap_or(namespace) == namespace => Some("directly"),
        "User" if s.name == format!("system:serviceaccount:{namespace}:{name}") => Some("directly"),
        "Group" if s.name == "system:serviceaccounts" => Some("all service accounts"),
        "Group" if s.name == format!("system:serviceaccounts:{namespace}") => Some("service accounts in its namespace"),
        _ => None,
    }
}

async fn account_related(client: &Client, namespace: &str, name: &str, d: &mut ObjectDetails) -> Result<()> {
    let all = ListParams::default();
    let rbs = Api::<RoleBinding>::all(client.clone());
    let crbs = Api::<ClusterRoleBinding>::all(client.clone());
    let pods = Api::<Pod>::namespaced(client.clone(), namespace);
    let (rbs, crbs, pods) = tokio::join!(rbs.list(&all), crbs.list(&all), pods.list(&all));

    // Bindings that name it, and the roles they grant.
    struct Grant {
        binding: (String, Option<String>),
        role: RoleRef,
        how: &'static str,
    }
    let mut grants = Vec::new();
    for b in rbs?.items {
        if let Some(how) = b.subjects.iter().flatten().find_map(|s| matches_account(s, namespace, name)) {
            grants.push(Grant { binding: (b.name_any(), b.namespace()), role: b.role_ref.clone(), how });
        }
    }
    for b in crbs?.items {
        if let Some(how) = b.subjects.iter().flatten().find_map(|s| matches_account(s, namespace, name)) {
            grants.push(Grant { binding: (b.name_any(), None), role: b.role_ref.clone(), how });
        }
    }
    let mut binding_rows = Vec::new();
    let mut permission_rows = Vec::new();
    let mut cache: HashMap<(String, String, String), Option<Vec<PolicyRule>>> = HashMap::new();
    let mut worst: Option<&'static str> = None;
    for g in &grants {
        let ns = g.binding.1.clone();
        let where_ = ns.clone().map_or_else(|| "cluster-wide".to_owned(), |n| format!("in {n}"));
        let mut row = InfoTableRow::new(vec![
            g.binding.0.clone(),
            format!("{} {}", g.role.kind, g.role.name),
            where_.clone(),
            g.how.to_owned(),
        ]);
        row.link = Some(match &ns {
            Some(n) => link(ROLE_BINDING, Some(n), &g.binding.0),
            None => link(CLUSTER_ROLE_BINDING, None, &g.binding.0),
        });
        binding_rows.push(row);
        let key = (g.role.kind.clone(), g.role.name.clone(), ns.clone().unwrap_or_default());
        if !cache.contains_key(&key) {
            let rules = role_rules(client, &g.role, ns.as_deref().unwrap_or_default()).await.ok().flatten();
            cache.insert(key.clone(), rules);
        }
        for r in cache[&key].iter().flatten() {
            let verbs = r.verbs.clone();
            let groups = r.api_groups.clone().unwrap_or_default();
            let resources = r.resources.clone().unwrap_or_default();
            let urls = r.non_resource_urls.clone().unwrap_or_default();
            let flag = risk(&verbs, &groups, &resources);
            if flag == Some("full access") || worst.is_none() {
                worst = flag.or(worst);
            }
            let what = if urls.is_empty() { join(&resources, "—") } else { format!("URLs {}", urls.join(", ")) };
            let mut row = InfoTableRow::new(vec![
                join(&verbs, "—"),
                what,
                where_.clone(),
                format!("{} {}", g.role.kind, g.role.name),
                flag.unwrap_or_default().to_owned(),
            ]);
            row.warn = flag.is_some();
            permission_rows.push(row);
        }
    }
    let n = binding_rows.len();
    let mut bindings = table_section(
        "sa-bindings",
        "Bindings",
        InfoTable {
            columns: ["Binding", "Role", "Where", "Applies"].map(String::from).to_vec(),
            rows: binding_rows,
            ..Default::default()
        },
        "No bindings name it: it can only do what every signed-in user can.",
    );
    bindings.summary = if n == 0 { "none".into() } else { plural(n, "binding") };
    d.sections.push(bindings);
    let p = permission_rows.len();
    let mut permissions = table_section(
        "sa-permissions",
        "What it can do",
        InfoTable {
            columns: ["Verbs", "Resources", "Where", "From", "Note"].map(String::from).to_vec(),
            mono: vec![0, 1],
            rows: permission_rows,
            ..Default::default()
        },
        "Nothing beyond the defaults for signed-in users.",
    );
    permissions.summary = if p == 0 { "defaults only".into() } else { plural(p, "rule") };
    d.sections.push(permissions);
    d.status = worst.map(|w| if w == "full access" { "Full access".into() } else { "Sensitive".into() });

    // Pods running as it.
    let rows: Vec<InfoTableRow> = pods?
        .items
        .iter()
        .filter(|p| p.spec.as_ref().and_then(|s| s.service_account_name.as_deref()).unwrap_or("default") == name)
        .map(|p| {
            let owner = p
                .owner_references()
                .iter()
                .find(|o| o.controller == Some(true))
                .map_or_else(|| "—".to_owned(), |o| format!("{}/{}", o.kind, o.name));
            let mut row = InfoTableRow::new(vec![p.name_any(), owner]);
            row.link = Some(link("core/Pod", Some(namespace), &p.name_any()));
            row
        })
        .collect();
    let count = rows.len();
    let mut section = table_section(
        "sa-pods",
        "Pods running as it",
        InfoTable { columns: vec!["Pod".into(), "Controlled by".into()], rows, ..Default::default() },
        "No pods run as it right now.",
    );
    if count > 0 {
        section.summary = plural(count, "pod");
    }
    d.sections.push(section);
    Ok(())
}

// ---- Check access ----

/// `deployments.apps` → (`apps`, `deployments`, None); `pods/log` → (``, `pods`, `log`).
fn split_resource(resource: &str) -> (String, String, Option<String>) {
    let (main, sub) = match resource.split_once('/') {
        Some((m, s)) => (m, Some(s.to_owned())),
        None => (resource, None),
    };
    match main.split_once('.') {
        Some((r, g)) => (g.to_owned(), r.to_owned(), sub),
        None => (String::new(), main.to_owned(), sub),
    }
}

/// Asks the API server whether a ServiceAccount may do `verb` on `resource`
/// (e.g. `pods`, `deployments.apps`, `pods/exec`) in `namespace` (all
/// namespaces when empty). Nothing is changed or stored.
pub async fn check_access(
    client: &Client,
    account_namespace: &str,
    account: &str,
    verb: &str,
    resource: &str,
    namespace: &str,
    name: &str,
) -> Result<AccessCheck> {
    if verb.trim().is_empty() || resource.trim().is_empty() {
        return Err(Error::Message("give a verb and a resource".into()));
    }
    let (group, resource, subresource) = split_resource(resource.trim());
    let review = SubjectAccessReview {
        spec: SubjectAccessReviewSpec {
            user: Some(format!("system:serviceaccount:{account_namespace}:{account}")),
            groups: Some(vec![
                "system:serviceaccounts".into(),
                format!("system:serviceaccounts:{account_namespace}"),
                "system:authenticated".into(),
            ]),
            resource_attributes: Some(ResourceAttributes {
                verb: Some(verb.trim().to_owned()),
                group: Some(group),
                resource: Some(resource),
                subresource,
                namespace: Some(namespace.trim().to_owned()).filter(|n| !n.is_empty()),
                name: Some(name.trim().to_owned()).filter(|n| !n.is_empty()),
                ..ResourceAttributes::default()
            }),
            ..SubjectAccessReviewSpec::default()
        },
        ..SubjectAccessReview::default()
    };
    let result = Api::<SubjectAccessReview>::all(client.clone()).create(&PostParams::default(), &review).await?;
    let status = result.status.unwrap_or_default();
    Ok(AccessCheck {
        allowed: status.allowed,
        reason: status.reason.or(status.evaluation_error).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::describe;
    use serde_json::json;

    fn now() -> jiff::Timestamp {
        "2026-10-10T12:00:00Z".parse().unwrap()
    }

    fn row<'a>(d: &'a ObjectDetails, key: &str) -> Option<&'a str> {
        d.info.iter().find(|r| r.key == key).map(|r| r.value.as_str())
    }

    #[test]
    fn roles_flag_risky_rules() {
        let v = json!({"kind": "ClusterRole", "metadata": {"name": "ops", "labels": {"rbac.authorization.k8s.io/aggregate-to-admin": "true"}},
            "rules": [
                {"apiGroups": [""], "resources": ["pods", "pods/log"], "verbs": ["get", "list"]},
                {"apiGroups": [""], "resources": ["secrets"], "verbs": ["get"], "resourceNames": ["tls"]},
                {"apiGroups": ["rbac.authorization.k8s.io"], "resources": ["clusterroles"], "verbs": ["bind"]},
                {"nonResourceURLs": ["/healthz"], "verbs": ["get"]}]});
        let d = describe(CLUSTER_ROLE, &v, now());
        assert_eq!(d.status.as_deref(), Some("Sensitive"));
        assert_eq!(row(&d, "Adds its rules to"), Some("admin"));
        let rows = &d.sections[0].table.as_ref().unwrap().rows;
        assert_eq!(rows[0].cells, ["get, list", "core", "pods, pods/log", "any", ""]);
        assert_eq!((rows[1].cells[3].as_str(), rows[1].cells[4].as_str()), ("tls", "reads secrets"));
        assert_eq!(rows[2].cells[4], "can grant itself more");
        assert_eq!(rows[3].cells[2], "URLs /healthz");
        assert_eq!(risk(&["get".into()], &["".into()], &["nodes/proxy".into()]), Some("reaches the kubelet"));

        let admin = json!({"kind": "ClusterRole", "metadata": {"name": "cluster-admin"},
            "rules": [{"apiGroups": ["*"], "resources": ["*"], "verbs": ["*"]}]});
        assert_eq!(describe(CLUSTER_ROLE, &admin, now()).status.as_deref(), Some("Full access"));
    }

    #[test]
    fn bindings_scope_and_subjects() {
        let v = json!({"kind": "RoleBinding", "metadata": {"name": "view", "namespace": "shop"},
            "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": "view"},
            "subjects": [{"kind": "ServiceAccount", "name": "api"}, {"kind": "Group", "name": "system:authenticated", "apiGroup": "rbac.authorization.k8s.io"}]});
        let d = describe(ROLE_BINDING, &v, now());
        assert_eq!(d.info[0].link.as_ref().unwrap().kind, CLUSTER_ROLE);
        assert!(row(&d, "Where").unwrap().contains("limited to here"));
        let rows = &d.sections[0].table.as_ref().unwrap().rows;
        assert_eq!(rows[0].cells, ["ServiceAccount", "api", "shop"]);
        assert_eq!(rows[0].link.as_ref().unwrap().namespace.as_deref(), Some("shop"));
        assert!(rows[1].warn);
        assert_eq!(d.status.as_deref(), Some("Everyone"));
    }

    #[test]
    fn service_accounts() {
        let v = json!({"kind": "ServiceAccount", "metadata": {"name": "api", "namespace": "shop",
            "annotations": {"eks.amazonaws.com/role-arn": "arn:aws:iam::123456789012:role/api"}},
            "automountServiceAccountToken": false, "imagePullSecrets": [{"name": "regcred"}]});
        let d = describe("core/ServiceAccount", &v, now());
        assert_eq!(row(&d, "AWS IAM role"), Some("arn:aws:iam::123456789012:role/api"));
        assert_eq!(row(&d, "Acts as"), Some("system:serviceaccount:shop:api"));
        assert!(row(&d, "Token in pods").unwrap().starts_with("not mounted"));
        assert_eq!(d.info.iter().find(|r| r.key == "Image pull secret").unwrap().link.as_ref().unwrap().name, "regcred");

        let s = |kind: &str, name: &str, ns: Option<&str>| Subject {
            kind: kind.into(),
            name: name.into(),
            namespace: ns.map(str::to_owned),
            api_group: None,
        };
        assert_eq!(matches_account(&s("ServiceAccount", "api", Some("shop")), "shop", "api"), Some("directly"));
        assert_eq!(matches_account(&s("ServiceAccount", "api", Some("other")), "shop", "api"), None);
        assert_eq!(matches_account(&s("Group", "system:serviceaccounts:shop", None), "shop", "api"), Some("service accounts in its namespace"));
        assert_eq!(matches_account(&s("User", "system:serviceaccount:shop:api", None), "shop", "api"), Some("directly"));
    }

    #[test]
    fn resource_names() {
        assert_eq!(split_resource("pods"), ("".into(), "pods".into(), None));
        assert_eq!(split_resource("deployments.apps"), ("apps".into(), "deployments".into(), None));
        assert_eq!(split_resource("pods/exec"), ("".into(), "pods".into(), Some("exec".into())));
        assert_eq!(
            split_resource("ingresses.networking.k8s.io"),
            ("networking.k8s.io".into(), "ingresses".into(), None)
        );
    }
}

#[cfg(test)]
#[path = "rbac_tests.rs"]
mod coverage_tests;
