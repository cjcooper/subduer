use super::*;

use std::fs;

use kube::config::Kubeconfig;

/// A scratch folder that's removed afterwards.
struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Self {
        crate::testing::record_all_traces();
        let dir = std::env::temp_dir().join(format!("subduer-manage-clusters-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const MAIN: &str = "apiVersion: v1
kind: Config
current-context: lab
clusters:
- name: lab
  cluster: {server: 'https://10.0.0.1:6443'}
- name: shared
  cluster: {server: 'https://ABC.gr7.eu-west-1.eks.amazonaws.com'}
users:
- name: lab
  user: {token: t}
- name: shared
  user: {token: s}
contexts:
- name: lab
  context: {cluster: lab, user: lab}
- name: a
  context: {cluster: shared, user: shared}
- name: b
  context: {cluster: shared, user: shared, namespace: x}
x-custom: kept
";

const OTHER: &str = "apiVersion: v1
kind: Config
clusters:
- name: aks
  cluster: {server: 'https://aks-dns-1234.hcp.westeurope.azmk8s.io:443'}
users:
- name: gke
  user:
    exec: {command: /usr/local/bin/gke-gcloud-auth-plugin, apiVersion: client.authentication.k8s.io/v1beta1}
contexts:
- name: aks
  context: {cluster: aks, user: lab}
- name: gke-web
  context: {cluster: lab, user: gke}
- name: lab
  context: {cluster: aks}
";

fn names(list: &ManagedContexts) -> Vec<&str> {
    list.contexts.iter().map(|c| c.name.as_str()).collect()
}

#[test]
fn lists_file_by_file_first_name_wins() {
    let dir = Scratch::new("list");
    let files = [dir.file("main", MAIN), dir.file("other", OTHER), dir.0.join("missing")];
    let list = list(&files).unwrap();
    assert_eq!(names(&list), ["lab", "a", "b", "aks", "gke-web"]);
    assert_eq!(list.current.as_deref(), Some("lab"));
    assert_eq!(list.files.len(), 3);
    let by = |n: &str| list.contexts.iter().find(|c| c.name == n).unwrap();
    assert_eq!((by("lab").server.as_str(), by("lab").provider), ("https://10.0.0.1:6443", None));
    assert_eq!(by("a").provider, Some(CloudProvider::Eks));
    assert_eq!(by("aks").provider, Some(CloudProvider::Aks));
    // The user (from the other file) runs GKE's plugin; the cluster comes from the first file.
    assert_eq!((by("gke-web").provider, by("gke-web").server.as_str()), (Some(CloudProvider::Gke), "https://10.0.0.1:6443"));
    assert!(by("aks").file.ends_with("other"));
}

#[test]
fn the_cloud_a_context_is_in() {
    assert_eq!(provider("x", "https://ABC.yl4.us-east-2.eks.amazonaws.com", ""), Some(CloudProvider::Eks));
    assert_eq!(provider("x", "https://10.1.2.3", "aws"), Some(CloudProvider::Eks));
    assert_eq!(provider("x", "https://10.1.2.3", "C:\\bin\\kubelogin.exe"), Some(CloudProvider::Aks));
    assert_eq!(provider("x", "https://a.privatelink.eastus.azmk8s.io:443", ""), Some(CloudProvider::Aks));
    assert_eq!(provider("gke_p_us-central1_web", "https://34.1.2.3", ""), Some(CloudProvider::Gke));
    assert_eq!(provider("kind-kind", "https://127.0.0.1:6443", ""), None);
    assert_eq!(provider("x", "https://eks.amazonaws.com.example.org", ""), None);
}

#[test]
fn removes_the_context_and_what_only_it_used() {
    let dir = Scratch::new("remove");
    let main = dir.file("main", MAIN);
    let files = [main.clone()];
    // `a` shares its cluster and user with `b`: only the context goes.
    let removed = remove(&files, "a").unwrap();
    assert_eq!(removed, RemovedContext { context: "a".into(), cluster: None, user: None });
    // Now `b` is the last to use them.
    let removed = remove(&files, "b").unwrap();
    assert_eq!((removed.cluster.as_deref(), removed.user.as_deref()), (Some("shared"), Some("shared")));
    let config = Kubeconfig::read_from(&main).unwrap();
    assert_eq!(config.contexts.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["lab"]);
    assert_eq!(config.clusters.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["lab"]);
    assert_eq!(config.current_context.as_deref(), Some("lab"));
    assert!(fs::read_to_string(&main).unwrap().contains("x-custom: kept"));

    // The current context: current-context goes with it.
    remove(&files, "lab").unwrap();
    assert_eq!(Kubeconfig::read_from(&main).unwrap().current_context, None);
    assert!(remove(&files, "nope").unwrap_err().to_string().contains("nope"));
    assert!(!dir.0.join("main.lock").exists());
}

#[test]
fn waits_for_no_one_holding_the_lock() {
    let dir = Scratch::new("locked");
    let main = dir.file("main", MAIN);
    let lock = dir.file("main.lock", "");
    let files = [main.clone()];
    assert!(remove(&files, "a").unwrap_err().to_string().contains("another program"));
    assert!(reorder(&files, &["b".into(), "a".into()]).unwrap_err().to_string().contains("another program"));
    assert_eq!(fs::read_to_string(&main).unwrap(), MAIN);
    assert!(lock.exists());
}

#[test]
fn a_context_gone_by_the_time_the_file_is_locked_is_not_found() {
    let file = Path::new("/k/config");
    let none = (HashSet::new(), HashSet::new());
    let mut no_list: Mapping = serde_norway::from_str("apiVersion: v1\n").unwrap();
    let err = remove_from(&mut no_list, file, "a", &none).unwrap_err().to_string();
    assert_eq!(err, "not found: context \"a\" in /k/config");
    let mut other: Mapping = serde_norway::from_str("contexts:\n- name: b\n  context: {cluster: b}\n").unwrap();
    assert!(remove_from(&mut other, file, "a", &none).is_err());
    assert_eq!(entries(&other, "contexts").count(), 1);
}

#[test]
fn a_cluster_or_user_the_file_does_not_define_is_not_reported() {
    let file = Path::new("/k/config");
    let none = (HashSet::new(), HashSet::new());
    let mut doc: Mapping = serde_norway::from_str("users: []\ncontexts:\n- name: a\n  context: {cluster: elsewhere, user: someone}\n").unwrap();
    let removed = remove_from(&mut doc, file, "a", &none).unwrap();
    assert_eq!(removed, RemovedContext { context: "a".into(), cluster: None, user: None });
    assert_eq!(entries(&doc, "contexts").count(), 0);
}

#[test]
fn keeps_what_another_file_still_uses() {
    let dir = Scratch::new("remove-across");
    let (main, other) = (dir.file("main", MAIN), dir.file("other", OTHER));
    let files = [main.clone(), other.clone()];
    // `lab` is in both files: the first goes, and the other file's `lab` shows instead.
    // Its cluster and user are still used by `gke-web` and `aks` in the other file.
    let removed = remove(&files, "lab").unwrap();
    assert_eq!((removed.cluster, removed.user), (None, None));
    assert_eq!(fs::read_to_string(&other).unwrap(), OTHER);
    let list = list(&files).unwrap();
    assert_eq!(names(&list), ["a", "b", "aks", "gke-web", "lab"]);
    assert!(list.contexts[4].file.ends_with("other"));
}

#[test]
fn reorders_each_file_and_leaves_others_alone() {
    let dir = Scratch::new("reorder");
    let (main, other) = (dir.file("main", MAIN), dir.file("other", OTHER));
    let files = [main.clone(), other.clone()];
    let order: Vec<String> = ["b", "lab", "a", "gke-web", "aks"].map(String::from).into();
    reorder(&files, &order).unwrap();
    let list = list(&files).unwrap();
    assert_eq!(names(&list), ["b", "lab", "a", "gke-web", "aks"]);
    // The shadowed `lab` in the other file has a place too; unknown names go last.
    let config = Kubeconfig::read_from(&other).unwrap();
    assert_eq!(config.contexts.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["lab", "gke-web", "aks"]);
    let text = fs::read_to_string(&main).unwrap();
    assert!(text.contains("namespace: x") && text.contains("x-custom: kept"), "{text}");

    // Unchanged files aren't written.
    let before = fs::metadata(&main).unwrap().modified().unwrap();
    fs::write(&other, OTHER).unwrap();
    reorder(&files, &order).unwrap();
    assert_eq!(fs::metadata(&main).unwrap().modified().unwrap(), before);
    // The rewritten file was put back in order; names left out keep theirs, after the rest.
    let partial: Vec<String> = ["aks"].map(String::from).into();
    reorder(&files, &partial).unwrap();
    let config = Kubeconfig::read_from(&other).unwrap();
    assert_eq!(config.contexts.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["aks", "lab", "gke-web"]);
}
