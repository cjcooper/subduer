//! Editing an object's YAML: the edit is parsed, checked against the object it
//! came from, and saved by replacing the object (PUT). The YAML keeps its
//! `metadata.resourceVersion`, so a change someone else made in the meantime
//! is refused instead of overwritten. Saving is gated; a dry run (validation
//! by the server) isn't.

use kube::Client;
use kube::api::{DynamicObject, PostParams};

use crate::cluster::dynamic_api;
use crate::safety::SafetyGate;
use crate::types::KindRef;
use crate::{Error, Result};

/// Kinds whose YAML can be edited: anything real (not a Helm release, which
/// is a summary of several Secrets).
pub fn editable(kind: &KindRef) -> bool {
    !crate::helm::is_release(kind)
}

/// The YAML to start editing from: the object without `managedFields`.
pub async fn load(client: &Client, kind: &KindRef, namespace: Option<&str>, name: &str) -> Result<String> {
    if !editable(kind) {
        return Err(Error::Message(format!("{} can't be edited as YAML", kind.kind)));
    }
    crate::cluster::get_yaml(client, kind, namespace, name).await
}

/// Parses an edit and checks it still describes the same object.
pub fn parse_edit(yaml: &str, kind: &KindRef, namespace: Option<&str>, name: &str) -> Result<DynamicObject> {
    let mut obj: DynamicObject = serde_norway::from_str(yaml).map_err(|e| Error::Message(format!("Not valid YAML: {e}")))?;
    let types = obj.types.clone().unwrap_or_default();
    if types.kind != kind.kind || types.api_version != kind.api_version() {
        return Err(Error::Message(format!(
            "apiVersion and kind must stay {} {}; to change them, create a new object instead",
            kind.api_version(),
            kind.kind
        )));
    }
    if obj.metadata.name.as_deref() != Some(name) {
        return Err(Error::Message(format!(
            "metadata.name must stay {name}: renaming means creating a new object"
        )));
    }
    match (namespace, obj.metadata.namespace.as_deref()) {
        (Some(ns), None) => obj.metadata.namespace = Some(ns.to_owned()),
        (Some(ns), Some(other)) if other != ns => {
            return Err(Error::Message(format!("metadata.namespace must stay {ns}")));
        }
        (None, Some(_)) => {
            return Err(Error::Message(format!("{} isn't namespaced: remove metadata.namespace", kind.kind)));
        }
        _ => {}
    }
    if obj.metadata.resource_version.as_deref().is_none_or(str::is_empty) {
        return Err(Error::Message(
            "Keep metadata.resourceVersion: it stops a change made by someone else meanwhile from being overwritten".into(),
        ));
    }
    Ok(obj)
}

/// Saves an edit by replacing the object, and returns the saved object's
/// YAML. A `dry_run` only asks the server to validate it, so it skips the gate.
#[allow(clippy::too_many_arguments)]
pub async fn save(
    client: &Client,
    gate: &SafetyGate,
    context: &str,
    kind: &KindRef,
    namespace: Option<&str>,
    name: &str,
    yaml: &str,
    dry_run: bool,
) -> Result<String> {
    if !editable(kind) {
        return Err(Error::Message(format!("{} can't be edited as YAML", kind.kind)));
    }
    let obj = parse_edit(yaml, kind, namespace, name)?;
    if !dry_run {
        gate.ensure_writable(context)?;
    }
    let params = PostParams { dry_run, ..PostParams::default() };
    let mut saved = match dynamic_api(client, kind, namespace).replace(name, &params, &obj).await {
        Err(kube::Error::Api(status)) if status.code == 409 => {
            return Err(Error::Message(format!(
                "{} {name} was changed by someone else since you opened it. Reload to start from the current version.",
                kind.kind
            )));
        }
        other => other?,
    };
    saved.metadata.managed_fields = None;
    tracing::info!(%context, kind = %kind.key(), ?namespace, %name, dry_run, "yaml saved");
    Ok(serde_norway::to_string(&saved)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{MockApi, kind as kind_ref, object};
    use serde_json::json;

    fn cm() -> KindRef {
        kind_ref("", "v1", "ConfigMap", "configmaps", true)
    }

    const YAML: &str = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: app\n  namespace: shop\n  resourceVersion: '7'\ndata:\n  a: '1'\n";

    #[test]
    fn edits_must_describe_the_same_object() {
        let ok = parse_edit(YAML, &cm(), Some("shop"), "app").unwrap();
        assert_eq!(ok.data["data"]["a"], "1");
        // A missing namespace is filled in.
        let no_ns = YAML.replace("  namespace: shop\n", "");
        assert_eq!(parse_edit(&no_ns, &cm(), Some("shop"), "app").unwrap().metadata.namespace.as_deref(), Some("shop"));

        let err = |yaml: &str, ns: Option<&str>| parse_edit(yaml, &cm(), ns, "app").unwrap_err().to_string();
        assert!(err("a: [", Some("shop")).starts_with("Not valid YAML"));
        assert!(err(&YAML.replace("kind: ConfigMap", "kind: Secret"), Some("shop")).contains("must stay v1 ConfigMap"));
        assert!(err(&YAML.replace("apiVersion: v1", "apiVersion: v2"), Some("shop")).contains("must stay"));
        assert!(err(&YAML.replace("name: app", "name: other"), Some("shop")).contains("metadata.name must stay app"));
        assert!(err(&YAML.replace("namespace: shop", "namespace: ops"), Some("shop")).contains("namespace must stay shop"));
        assert!(err(YAML, None).contains("isn't namespaced"));
        assert!(err(&YAML.replace("  resourceVersion: '7'\n", ""), Some("shop")).contains("Keep metadata.resourceVersion"));
    }

    #[tokio::test]
    async fn saving_replaces_the_object() {
        let api = MockApi::new();
        let path = "/api/v1/namespaces/shop/configmaps/app";
        api.on("PUT", path, object("v1", "ConfigMap", Some("shop"), "app", json!({
            "metadata": {"managedFields": [{"manager": "kubectl"}]}, "data": {"a": "2"}})));
        let gate = SafetyGate::default();
        // A dry run validates without unlocking.
        let yaml = save(&api.client(), &gate, "acme-prd", &cm(), Some("shop"), "app", YAML, true).await.unwrap();
        assert!(yaml.contains("a: '2'") && !yaml.contains("managedFields"), "{yaml}");
        let call = api.calls_to("PUT", path).pop().unwrap();
        assert!(call.query.contains("dryRun=All"));
        assert_eq!(call.json()["metadata"]["resourceVersion"], "7");
        // A real save needs the context unlocked.
        let err = save(&api.client(), &gate, "acme-prd", &cm(), Some("shop"), "app", YAML, false).await.unwrap_err();
        assert!(matches!(err, Error::Locked(_)));
        gate.set_unlocked("acme-prd", true);
        save(&api.client(), &gate, "acme-prd", &cm(), Some("shop"), "app", YAML, false).await.unwrap();
        assert!(!api.calls_to("PUT", path).pop().unwrap().query.contains("dryRun"));
    }

    #[tokio::test]
    async fn conflicts_and_server_errors_are_explained() {
        let api = MockApi::new();
        let gate = SafetyGate::default();
        gate.set_unlocked("acme-uat", true);
        api.on_error("PUT", "/api/v1/namespaces/shop/configmaps/app", 409, "Conflict", "the object has been modified");
        let err = save(&api.client(), &gate, "acme-uat", &cm(), Some("shop"), "app", YAML, false).await.unwrap_err();
        assert!(err.to_string().contains("changed by someone else"), "{err}");
        let api = MockApi::new();
        api.on_error("PUT", "/api/v1/namespaces/shop/configmaps/app", 422, "Invalid", "data[a b]: Invalid value");
        let err = save(&api.client(), &gate, "acme-uat", &cm(), Some("shop"), "app", YAML, true).await.unwrap_err();
        assert_eq!(err.user_message(), "data[a b]: Invalid value");
    }

    #[tokio::test]
    async fn helm_releases_and_loading() {
        let helm = crate::helm::release_kind();
        assert!(!editable(&helm) && editable(&cm()));
        let api = MockApi::new();
        let gate = SafetyGate::default();
        assert!(load(&api.client(), &helm, Some("shop"), "web").await.is_err());
        assert!(save(&api.client(), &gate, "c", &helm, Some("shop"), "web", YAML, true).await.is_err());
        api.on("GET", "/api/v1/namespaces/shop/configmaps/app", object("v1", "ConfigMap", Some("shop"), "app", json!({
            "metadata": {"managedFields": [{"manager": "kubectl"}]}})));
        let yaml = load(&api.client(), &cm(), Some("shop"), "app").await.unwrap();
        assert!(yaml.contains("name: app") && !yaml.contains("managedFields"));
    }
}
