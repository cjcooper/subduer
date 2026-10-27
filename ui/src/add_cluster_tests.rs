use super::*;

#[test]
fn exec_arguments_are_one_per_line() {
    assert_eq!(split_args("get-token\n  --login=azurecli \n\n--server-id=abc\r\n"), ["get-token", "--login=azurecli", "--server-id=abc"]);
    assert!(split_args("  \n").is_empty());
}

#[test]
fn every_source_has_a_card() {
    let sources: Vec<Source> = SOURCES.iter().map(|(s, _, _)| *s).collect();
    assert_eq!(sources, [Some(CloudProvider::Eks), Some(CloudProvider::Gke), Some(CloudProvider::Aks), None]);
    assert_eq!(sources[0], DEFAULT_SOURCE);
}

fn managed(name: &str, cluster: &str, server: &str) -> ManagedContext {
    ManagedContext { name: name.into(), cluster: cluster.into(), user: String::new(), server: server.into(), provider: None, file: String::new() }
}

#[test]
fn servers_compare_by_host_and_port() {
    assert_eq!(server_key("https://34.1.2.3"), "34.1.2.3");
    assert_eq!(server_key("34.1.2.3"), "34.1.2.3");
    assert_eq!(server_key("https://AKS-1.hcp.westeurope.azmk8s.io:443/"), "aks-1.hcp.westeurope.azmk8s.io");
    assert_eq!(server_key(" https://10.0.0.1:6443/k8s "), "10.0.0.1:6443");
    assert_eq!(server_key(""), "");
}

#[test]
fn finds_the_contexts_a_cluster_is_under() {
    let existing = [
        managed("web", "gke_acme_us-central1_web", "https://34.1.2.3"),
        managed("web-admin", "gke_acme_us-central1_web", "https://34.1.2.3"),
        managed("api", "arn:aws:eks:eu-west-1:1:cluster/api", "https://abc.gr7.eu-west-1.eks.amazonaws.com"),
        managed("aks", "aks-1", "https://aks-1-dns.hcp.westeurope.azmk8s.io:443"),
        managed("broken", "", ""),
    ];
    assert_eq!(already_in(&existing, "34.1.2.3", "gke_acme_us-central1_web"), ["web", "web-admin"]);
    // By the entry alone (renamed context, no server reported), or the server alone.
    assert_eq!(already_in(&existing, "", "arn:aws:eks:eu-west-1:1:cluster/api"), ["api"]);
    assert_eq!(already_in(&existing, "aks-1-dns.hcp.westeurope.azmk8s.io", ""), ["aks"]);
    assert!(already_in(&existing, "", "").is_empty());
    assert!(already_in(&existing, "https://10.9.9.9", "").is_empty());
}

#[test]
fn names_in_a_sentence() {
    let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(quoted_names(&names(&["a"])), "“a”");
    assert_eq!(quoted_names(&names(&["a", "b"])), "“a” and “b”");
    assert_eq!(quoted_names(&names(&["a", "b", "c"])), "“a”, “b” and “c”");
}
