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
    assert_eq!(quoted_names(&[]), "");
}

#[test]
fn provider_words() {
    use CloudProvider::*;
    assert_eq!([account_label(Gke), account_label(Eks), account_label(Aks)], ["Project", "Profile", "Subscription"]);
    assert_eq!([cli_name(Gke), cli_name(Eks), cli_name(Aks)], ["gcloud", "aws", "az"]);
    assert_eq!(
        [credential_command(Gke), credential_command(Eks), credential_command(Aks)],
        ["gke-gcloud-auth-plugin", "aws eks get-token", "kubelogin"]
    );
    assert_eq!(asking_text(Gke), "Asking gcloud…");
    assert_eq!(sign_in_text(Aks), "Uses your az sign-in. The context it writes runs ");
    assert_eq!(no_accounts_text(Gke), "gcloud lists no projects for this sign-in.");
    assert_eq!(where_label(Aks), "Region · group");
    assert_eq!(where_label(Gke), "Location");
    assert_eq!(AWS_REGIONS.len(), 17);
}

#[test]
fn auth_choices() {
    assert_eq!(auth_choice("cert"), "cert");
    assert_eq!(auth_choice("none"), "none");
    assert_eq!(auth_choice("bogus"), "token");
    assert_eq!(DEFAULT_AUTH, AUTHS[0].0);
}

fn typed(auth: &'static str) -> ManualInput {
    ManualInput {
        name: "lab".into(),
        server: "https://10.0.0.1:6443".into(),
        ca: "ca.pem".into(),
        insecure: true,
        proxy: "socks5://localhost:1080".into(),
        auth,
        token: "t0k".into(),
        cert: "c.pem".into(),
        key: "k.pem".into(),
        command: "kubelogin".into(),
        args: "get-token\n--x\n".into(),
        namespace: "dev".into(),
    }
}

#[test]
fn the_typed_in_cluster() {
    let c = typed("token").cluster(true);
    assert_eq!(c.name, "lab");
    assert_eq!(c.server, "https://10.0.0.1:6443");
    assert_eq!(c.certificate_authority, "ca.pem");
    assert!(c.insecure_skip_tls_verify);
    assert_eq!(c.proxy_url, "socks5://localhost:1080");
    assert_eq!(c.namespace, "dev");
    assert!(c.make_current);
    assert_eq!(c.auth, ClusterAuth::Token { token: "t0k".into() });
    assert_eq!(typed("none").cluster(false).auth, ClusterAuth::None);
    assert_eq!(typed("cert").cluster(false).auth, ClusterAuth::ClientCertificate { certificate: "c.pem".into(), key: "k.pem".into() });
    assert_eq!(
        typed("exec").cluster(false).auth,
        ClusterAuth::Exec { command: "kubelogin".into(), args: vec!["get-token".into(), "--x".into()] }
    );
    let valid = ManualInput { insecure: false, ..typed("token") };
    assert_eq!(valid.cluster(false).problem(), None);
    assert!(ManualInput::default().cluster(false).problem().is_some());
}

#[test]
fn what_stops_add() {
    assert!(touched("lab", ""));
    assert!(touched("", "https://x"));
    assert!(!touched("", ""));
    assert_eq!(problem(None, &|| Some("Give the context a name."), false), Some("Give the context a name."));
    assert_eq!(problem(Some(CloudProvider::Gke), &|| unreachable!(), false), Some("Pick a cluster."));
    assert_eq!(problem(Some(CloudProvider::Gke), &|| unreachable!(), true), None);
    assert!(shows_problem(None, true, false));
    assert!(!shows_problem(None, false, false));
    assert!(!shows_problem(None, true, true));
    assert!(!shows_problem(Some(CloudProvider::Eks), true, false));
    assert_eq!(add_label(true, Some(CloudProvider::Eks)), "Running aws…");
    assert_eq!(add_label(true, None), "Adding…");
    assert_eq!(add_label(false, None), "Add");
}

#[test]
fn picking_a_source_again() {
    let eks = Some(CloudProvider::Eks);
    assert!(same_source(None, None, false));
    assert!(same_source(eks, eks, true));
    assert!(!same_source(eks, eks, false), "a cloud whose accounts weren't asked for yet");
    assert!(!same_source(eks, None, true));
}

fn accounts(default: Option<&str>, region: Option<&str>) -> CloudAccounts {
    CloudAccounts {
        accounts: vec![
            subduer_types::CloudAccount { id: "dev".into(), label: "Dev".into(), region: Some("eu-west-1".into()) },
            subduer_types::CloudAccount { id: "prod".into(), label: "Prod".into(), region: None },
        ],
        default: default.map(str::to_owned),
        region: region.map(str::to_owned),
    }
}

#[test]
fn choosing_an_account_and_region() {
    use CloudProvider::*;
    assert_eq!(default_account(Gke, &accounts(Some("prod"), None)), "prod");
    assert_eq!(default_account(Gke, &accounts(None, None)), "dev", "the first");
    assert_eq!(default_account(Eks, &accounts(None, None)), "", "the default credentials");
    assert_eq!(default_account(Aks, &CloudAccounts { accounts: vec![], default: None, region: None }), "");
    assert_eq!(account_region(Gke, &accounts(None, Some("us-east-1")), "dev"), None);
    assert_eq!(account_region(Eks, &accounts(None, Some("us-east-1")), "dev").as_deref(), Some("eu-west-1"));
    assert_eq!(account_region(Eks, &accounts(None, Some("us-east-1")), "prod").as_deref(), Some("us-east-1"));
    assert_eq!(account_region(Eks, &accounts(None, None), ""), None);
    assert!(ready_to_list(Eks, "", "us-east-1"));
    assert!(!ready_to_list(Eks, "dev", "  "));
    assert!(ready_to_list(Gke, "proj", ""));
    assert!(!ready_to_list(Aks, "", "x"));
    assert!(no_accounts(Gke, &CloudAccounts { accounts: vec![], default: None, region: None }));
    assert!(!no_accounts(Eks, &CloudAccounts { accounts: vec![], default: None, region: None }));
    assert!(!no_accounts(Gke, &accounts(None, None)));
}

fn cloud(name: &str, group: &str) -> CloudCluster {
    CloudCluster {
        name: name.into(),
        location: "westeurope".into(),
        group: group.into(),
        version: "1.31".into(),
        status: "Running".into(),
        server: String::new(),
        entry: String::new(),
    }
}

#[test]
fn cloud_cluster_list() {
    use CloudProvider::*;
    assert_eq!(only_cluster(&Ok(vec![cloud("a", "")])), Some(cloud("a", "")));
    assert_eq!(only_cluster(&Ok(vec![cloud("a", ""), cloud("b", "")])), None);
    assert_eq!(only_cluster(&Err("no".into())), None);
    assert_eq!(place(&cloud("a", "")), "westeurope");
    assert_eq!(place(&cloud("a", "rg-1")), "westeurope · rg-1");
    assert_eq!(no_clusters_text(Eks, " "), "Choose a region.");
    assert_eq!(no_clusters_text(Eks, "us-east-1"), "No clusters here.");
    assert_eq!(no_clusters_text(Gke, ""), "No clusters here.");
    let c = cloud("web", "");
    assert_eq!(alias_placeholder(Gke, "acme", None), "as the CLI names it");
    assert_eq!(alias_placeholder(Gke, "acme", Some(&c)), "gke_acme_westeurope_web");
    assert_eq!(alias_placeholder(Eks, "", Some(&c)), "the cluster's ARN (…:cluster/web)");
    assert_eq!(alias_placeholder(Aks, "", Some(&c)), "web");
}

#[test]
fn saying_a_cluster_is_already_in() {
    let existing = [managed("web", "gke_web", "https://34.1.2.3")];
    assert_eq!(same_server(&existing, "34.1.2.3"), ["web"]);
    assert!(same_server(&existing, "  ").is_empty());
    assert_eq!(server_already_text(&["web".into()]).as_deref(), Some("This server is already in your kubeconfig as “web”."));
    assert_eq!(server_already_text(&[]), None);
    assert_eq!(cluster_already_text(&["web".into()]).as_deref(), Some("This cluster is already in your kubeconfig as context “web”. "));
    assert_eq!(
        cluster_already_text(&["a".into(), "b".into()]).as_deref(),
        Some("This cluster is already in your kubeconfig as contexts “a” and “b”. ")
    );
    assert_eq!(cluster_already_text(&[]), None);
}
