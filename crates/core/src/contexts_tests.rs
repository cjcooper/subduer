use super::*;

const KUBECONFIG: &str = r#"
apiVersion: v1
kind: Config
current-context: acme-uat
clusters:
- name: uat-cluster
  cluster:
    server: https://127.0.0.1:6443
- name: eks-prod-1
  cluster:
    server: https://127.0.0.1:6444
contexts:
- name: acme-uat
  context:
    cluster: uat-cluster
    user: dev
    namespace: shop
- name: acme-prd
  context:
    cluster: uat-cluster
    user: dev
- name: ops
  context:
    cluster: eks-prod-1
    user: dev
- name: dangling
  context:
    cluster: no-such-cluster
    user: dev
- name: bare
users:
- name: dev
  user:
    token: not-a-real-token
"#;

fn kubeconfig() -> Kubeconfig {
    Kubeconfig::from_yaml(KUBECONFIG).unwrap()
}

#[test]
fn lists_contexts_with_prod_and_unlock_flags() {
    let gate = SafetyGate::default();
    gate.set_unlocked("acme-prd", true);
    let list = list_contexts(&kubeconfig(), &gate);
    assert_eq!(list.current.as_deref(), Some("acme-uat"));
    let names: Vec<&str> = list.contexts.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["acme-uat", "acme-prd", "ops", "dangling", "bare"]);

    let uat = &list.contexts[0];
    assert_eq!(uat.cluster.as_deref(), Some("uat-cluster"));
    assert_eq!(uat.namespace.as_deref(), Some("shop"));
    assert!(!uat.prod_like && !uat.unlocked);

    // Prod-like by context name.
    let prd = &list.contexts[1];
    assert!(prd.prod_like && prd.unlocked);
    assert_eq!(prd.namespace, None);

    // Prod-like by cluster name only.
    let ops = &list.contexts[2];
    assert!(ops.prod_like && !ops.unlocked);

    // A context entry with no body.
    let bare = &list.contexts[4];
    assert_eq!((bare.cluster.as_deref(), bare.namespace.as_deref(), bare.prod_like), (None, None, false));
}

#[test]
fn empty_kubeconfig_lists_nothing() {
    let list = list_contexts(&Kubeconfig::default(), &SafetyGate::default());
    assert!(list.contexts.is_empty());
    assert_eq!(list.current, None);
}

#[tokio::test]
async fn client_for_known_context() {
    // Builds a client without contacting the server.
    let client = client_for(&kubeconfig(), "acme-uat").await.unwrap();
    assert_eq!(CONNECT_TIMEOUT, std::time::Duration::from_secs(10));
    assert_eq!(client.default_namespace(), "shop");
}

#[tokio::test]
async fn client_for_unknown_context() {
    let Err(err) = client_for(&kubeconfig(), "acme-dev").await else { panic!("expected an error") };
    assert!(matches!(err, Error::UnknownContext(ref c) if c == "acme-dev"));
    assert_eq!(err.to_string(), r#"context "acme-dev" not found in kubeconfig"#);
}

#[tokio::test]
async fn client_for_context_with_missing_cluster() {
    let Err(err) = client_for(&kubeconfig(), "dangling").await else { panic!("expected an error") };
    assert!(matches!(err, Error::Kubeconfig(_)), "{err}");
    assert!(err.to_string().starts_with("kubeconfig: "), "{err}");
}

#[test]
fn load_kubeconfig_honours_env() {
    let dir = std::env::temp_dir().join(format!("subduer-contexts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config");
    std::fs::write(&path, KUBECONFIG).unwrap();
    let previous = std::env::var_os("KUBECONFIG");
    // SAFETY: restored below; no other test in this crate reads $KUBECONFIG.
    unsafe { std::env::set_var("KUBECONFIG", &path) };
    let loaded = load_kubeconfig();
    let missing = {
        // SAFETY: as above.
        unsafe { std::env::set_var("KUBECONFIG", dir.join("absent")) };
        load_kubeconfig()
    };
    match previous {
        // SAFETY: as above.
        Some(v) => unsafe { std::env::set_var("KUBECONFIG", v) },
        None => unsafe { std::env::remove_var("KUBECONFIG") },
    }
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&dir).unwrap();
    let loaded = loaded.unwrap();
    assert_eq!(loaded.current_context.as_deref(), Some("acme-uat"));
    assert_eq!(loaded.contexts.len(), 5);
    assert!(matches!(missing, Err(Error::Kubeconfig(_))));
}
