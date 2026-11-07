use super::*;

use serde_json::json;

#[test]
fn system_clis_are_found_on_path() {
    let clis = Clis::system();
    assert_eq!(clis.dir, PathBuf::new());
    // Joined with nothing, the bare name: looked up on PATH.
    assert_eq!(clis.dir.join("gcloud"), Path::new("gcloud"));
    assert_eq!(clis.kubeconfig, KubeconfigEnv::current());
    assert_eq!((clis.var)("PATH"), std::env::var("PATH").ok());
    assert_eq!((clis.list_timeout, clis.add_timeout), (LIST_TIMEOUT, ADD_TIMEOUT));
}

/// A scratch folder of fake CLIs (shell scripts), removed afterwards.
#[cfg(unix)]
struct Fake(PathBuf);

#[cfg(unix)]
impl Fake {
    fn new(test: &str) -> Self {
        crate::testing::record_all_traces();
        let dir = std::env::temp_dir().join(format!("subduer-cloud-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// Writes the fake `name` CLI: `script` runs after the call's arguments
    /// are logged to `<name>.calls`.
    fn cli(&self, name: &str, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = self.0.join(name);
        std::fs::write(&path, format!("#!/bin/sh\necho \"$*\" >> \"{}/{name}.calls\"\n{script}\n", self.0.display())).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// The arguments of each call to `name`, in order.
    fn calls(&self, name: &str) -> Vec<String> {
        std::fs::read_to_string(self.0.join(format!("{name}.calls"))).unwrap_or_default().lines().map(str::to_owned).collect()
    }

    fn config(&self) -> PathBuf {
        self.0.join("config")
    }

    fn clis(&self) -> Clis {
        Clis {
            dir: self.0.clone(),
            var: |_| None,
            kubeconfig: KubeconfigEnv { kubeconfig: Some(self.config().into_os_string()), home: None },
            list_timeout: Duration::from_secs(20),
            add_timeout: Duration::from_secs(20),
        }
    }
}

#[cfg(unix)]
impl Drop for Fake {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn lists_gke_projects_and_clusters() {
    let fake = Fake::new("gke");
    fake.cli(
        "gcloud",
        r#"case "$1 $2" in
  "projects list") echo '[{"projectId":"b-proj","name":"Beta"},{"projectId":"a-proj"}]' ;;
  "config get-value") echo a-proj ;;
  "container clusters") echo '[{"name":"web","location":"us-central1"},{"name":"web","zone":"europe-west1-b"},{"name":"api","location":"us-east1","status":"RUNNING"}]' ;;
esac"#,
    );
    let clis = fake.clis();
    let accounts = clis.accounts(CloudProvider::Gke).await.unwrap();
    assert_eq!(accounts.accounts.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(), ["a-proj", "Beta (b-proj)"]);
    assert_eq!((accounts.default.as_deref(), accounts.region), (Some("a-proj"), None));

    let clusters = clis.clusters(CloudProvider::Gke, "a-proj", "").await.unwrap();
    let names: Vec<_> = clusters.iter().map(|c| (c.name.as_str(), c.location.as_str(), c.entry.as_str())).collect();
    assert_eq!(
        names,
        [
            ("api", "us-east1", "gke_a-proj_us-east1_api"),
            ("web", "europe-west1-b", "gke_a-proj_europe-west1-b_web"),
            ("web", "us-central1", "gke_a-proj_us-central1_web")
        ]
    );
    assert!(clis.clusters(CloudProvider::Gke, "--flag", "").await.unwrap_err().to_string().contains("not a valid project"));
    assert_eq!(
        fake.calls("gcloud"),
        ["projects list --format=json(projectId,name)", "config get-value project", "container clusters list --project=a-proj --format=json"]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn gke_without_a_default_project() {
    let fake = Fake::new("gke-nodefault");
    fake.cli("gcloud", r#"case "$1" in projects) echo '[]' ;; *) echo "(unset)" >&2; exit 1 ;; esac"#);
    let accounts = fake.clis().accounts(CloudProvider::Gke).await.unwrap();
    assert!(accounts.accounts.is_empty());
    assert_eq!(accounts.default, None);
}

#[cfg(unix)]
const AWS: &str = r#"case "$*" in
  "configure list-profiles") printf 'default\n\nwork\n' ;;
  "--profile=work configure get region") echo eu-west-1 ;;
  "--profile=default configure get region") exit 1 ;;
  "configure get region") echo us-east-2 ;;
  "--profile=work eks list-clusters --region=eu-west-1 --output=json") echo '{"clusters":["garbled","api","-odd","broken"]}' ;;
  *"describe-cluster --name=api "*) echo '{"cluster":{"version":"1.33","status":"ACTIVE","endpoint":"https://api.eks","arn":"arn:aws:eks:eu-west-1:1:cluster/api"}}' ;;
  *"--name=broken"*) echo "AccessDeniedException" >&2; exit 254 ;;
  *"--name=garbled"*) echo "not json" ;;
  "eks list-clusters --region=us-east-2 --output=json") echo '{"clusters":[]}' ;;
esac"#;

#[cfg(unix)]
#[tokio::test]
async fn lists_aws_profiles_with_their_regions() {
    let fake = Fake::new("eks-accounts");
    fake.cli("aws", AWS);
    let accounts = fake.clis().accounts(CloudProvider::Eks).await.unwrap();
    assert_eq!(
        accounts.accounts,
        [
            CloudAccount { id: "default".into(), label: "default".into(), region: None },
            CloudAccount { id: "work".into(), label: "work".into(), region: Some("eu-west-1".into()) }
        ]
    );
    // No AWS_PROFILE or AWS_REGION: the default credentials' region comes from the CLI.
    assert_eq!((accounts.default, accounts.region.as_deref()), (None, Some("us-east-2")));
    assert!(fake.calls("aws").contains(&"configure get region".to_owned()));

    // With them set, the environment wins (an empty AWS_REGION doesn't count).
    let fake = Fake::new("eks-env");
    fake.cli("aws", AWS);
    let mut clis = fake.clis();
    clis.var = |name| match name {
        "AWS_PROFILE" => Some("work".into()),
        "AWS_REGION" => Some(String::new()),
        "AWS_DEFAULT_REGION" => Some("ap-south-1".into()),
        _ => None,
    };
    let accounts = clis.accounts(CloudProvider::Eks).await.unwrap();
    assert_eq!((accounts.default.as_deref(), accounts.region.as_deref()), (Some("work"), Some("ap-south-1")));
    assert!(!fake.calls("aws").contains(&"configure get region".to_owned()));
}

#[cfg(unix)]
#[tokio::test]
async fn describes_each_eks_cluster_it_can() {
    let fake = Fake::new("eks-clusters");
    fake.cli("aws", AWS);
    let clis = fake.clis();
    let clusters = clis.clusters(CloudProvider::Eks, "work", "eu-west-1").await.unwrap();
    assert_eq!(clusters.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["-odd", "api", "broken", "garbled"]);
    let api = &clusters[1];
    assert_eq!((api.version.as_str(), api.status.as_str(), api.server.as_str()), ("1.33", "ACTIVE", "https://api.eks"));
    assert_eq!(api.entry, "arn:aws:eks:eu-west-1:1:cluster/api");
    // Not described (a name read as a flag), refused, or not JSON: listed as is.
    for c in [&clusters[0], &clusters[2], &clusters[3]] {
        assert_eq!((c.location.as_str(), c.version.as_str(), c.entry.as_str()), ("eu-west-1", "", ""), "{}", c.name);
    }
    let calls = fake.calls("aws");
    assert_eq!(calls[0], "--profile=work eks list-clusters --region=eu-west-1 --output=json");
    assert_eq!(calls.iter().filter(|c| c.contains("describe-cluster")).count(), 3);
    assert!(calls.iter().all(|c| !c.contains("-odd")));

    // The default credentials: no --profile.
    assert!(clis.clusters(CloudProvider::Eks, " ", "us-east-2").await.unwrap().is_empty());
    assert!(clis.clusters(CloudProvider::Eks, "work", "").await.unwrap_err().to_string().contains("not a valid region"));
    assert!(clis.clusters(CloudProvider::Eks, "-p", "us-east-2").await.unwrap_err().to_string().contains("not a valid profile"));
}

#[cfg(unix)]
#[tokio::test]
async fn lists_azure_subscriptions_and_clusters() {
    let fake = Fake::new("aks");
    fake.cli(
        "az",
        r#"case "$*" in
  "account list --output=json") echo '[{"id":"1","name":"Dev","isDefault":true}]' ;;
  "aks list --subscription=1 --output=json") echo '[{"name":"aks-1","location":"westeurope","resourceGroup":"rg"}]' ;;
  "aks list --subscription=2 --output=json") echo 'oops' ;;
esac"#,
    );
    let clis = fake.clis();
    let accounts = clis.accounts(CloudProvider::Aks).await.unwrap();
    assert_eq!((accounts.accounts.len(), accounts.default.as_deref()), (1, Some("1")));
    let clusters = clis.clusters(CloudProvider::Aks, "1", "").await.unwrap();
    assert_eq!((clusters[0].name.as_str(), clusters[0].group.as_str()), ("aks-1", "rg"));
    // Nothing printed reads as no clusters; anything else that isn't JSON is an error.
    assert!(clis.clusters(CloudProvider::Aks, "3", "").await.unwrap().is_empty());
    assert!(clis.clusters(CloudProvider::Aks, "2", "").await.unwrap_err().to_string().starts_with("unexpected CLI output"));
    assert!(clis.clusters(CloudProvider::Aks, "", "").await.unwrap_err().to_string().contains("not a valid subscription"));
}

#[cfg(unix)]
#[tokio::test]
async fn reports_cli_failures() {
    let fake = Fake::new("failures");
    let clis = fake.clis();
    // Not installed.
    let err = clis.accounts(CloudProvider::Gke).await.unwrap_err().to_string();
    assert!(err.starts_with("The gcloud command isn't installed"), "{err}");
    // Not runnable.
    std::fs::write(fake.0.join("aws"), "").unwrap();
    let err = clis.accounts(CloudProvider::Eks).await.unwrap_err().to_string();
    assert!(err.starts_with("couldn't run aws: "), "{err}");
    // Fails without saying why, then with a reason.
    fake.cli("az", "exit 3");
    let err = clis.accounts(CloudProvider::Aks).await.unwrap_err().to_string();
    assert_eq!(err, "az account list --output=json failed (exit status: 3)");
    fake.cli("az", "echo 'ERROR: Please run az login' >&2; exit 1");
    assert_eq!(clis.accounts(CloudProvider::Aks).await.unwrap_err().to_string(), "ERROR: Please run az login");
    // Takes too long.
    fake.cli("gcloud", "exec sleep 10");
    let mut slow = fake.clis();
    slow.list_timeout = Duration::from_millis(200);
    let err = slow.accounts(CloudProvider::Gke).await.unwrap_err().to_string();
    assert_eq!(err, "gcloud projects list --format=json(projectId,name) didn't finish within 0s");
}

#[cfg(unix)]
const OLD_CONFIG: &str = "apiVersion: v1
kind: Config
current-context: old
clusters:
- name: old
  cluster: {server: 'https://old'}
users:
- name: old
  user: {token: t}
contexts:
- name: old
  context: {cluster: old, user: old}
";

/// What a CLI leaves behind: `OLD_CONFIG` plus `context`, made current.
#[cfg(unix)]
fn after(context: &str) -> String {
    OLD_CONFIG.replace("current-context: old", &format!("current-context: {context}"))
        + &format!("- name: {context}\n  context: {{cluster: old, user: old}}\n")
}

#[cfg(unix)]
#[tokio::test]
async fn adds_a_gke_cluster_under_an_alias_and_keeps_the_current_context() {
    let fake = Fake::new("add-gke");
    std::fs::write(fake.config(), OLD_CONFIG).unwrap();
    std::fs::write(fake.0.join("after"), after("gke_p_us-central1_web")).unwrap();
    fake.cli(
        "gcloud",
        r#"cp "$(dirname "$0")/after" "$KUBECONFIG"
echo "Fetching cluster endpoint and auth data." >&2
echo "kubeconfig entry generated for web.""#,
    );
    let clis = fake.clis();
    let web = cluster("web", "us-central1", "");
    let added = clis.add(CloudProvider::Gke, "p", &web, " web ", false).await.unwrap();
    assert_eq!(added.context, "web");
    assert_eq!(added.output, "Fetching cluster endpoint and auth data.\nkubeconfig entry generated for web.");
    assert_eq!(add_cluster::current_context(&fake.config()).unwrap().as_deref(), Some("old"));
    let taken = add_cluster::TakenNames::read_in(&clis.kubeconfig).unwrap();
    assert_eq!(taken.contexts, ["old", "web"]);
    assert_eq!(fake.calls("gcloud"), ["container clusters get-credentials web --region=us-central1 --project=p"]);

    // Names in use, or read as flags, are refused before the CLI runs.
    let err = clis.add(CloudProvider::Gke, "p", &web, "old", false).await.unwrap_err().to_string();
    assert_eq!(err, "A context named \"old\" already exists.");
    assert!(clis.add(CloudProvider::Gke, "p", &web, "-web", false).await.unwrap_err().to_string().contains("not a valid context name"));
    assert!(clis.add(CloudProvider::Gke, "-p", &web, "", false).await.unwrap_err().to_string().contains("not a valid project"));
    assert_eq!(fake.calls("gcloud").len(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn adds_an_eks_cluster_and_makes_it_current() {
    let fake = Fake::new("add-eks");
    std::fs::write(fake.config(), OLD_CONFIG).unwrap();
    let arn = "arn:aws:eks:eu-west-1:1:cluster/api";
    std::fs::write(fake.0.join("after"), after(arn)).unwrap();
    fake.cli("aws", r#"cp "$(dirname "$0")/after" "$KUBECONFIG""#);
    let api = cluster("api", "eu-west-1", "");
    let added = fake.clis().add(CloudProvider::Eks, "work", &api, "", true).await.unwrap();
    assert_eq!((added.context.as_str(), added.output.as_str()), (arn, ""));
    assert_eq!(add_cluster::current_context(&fake.config()).unwrap().as_deref(), Some(arn));
    assert_eq!(
        fake.calls("aws"),
        [format!("--profile=work eks update-kubeconfig --name=api --region=eu-west-1 --kubeconfig={}", fake.config().display())]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn adding_fails_when_the_cli_does_or_adds_nothing() {
    let fake = Fake::new("add-fails");
    let aks = cluster("aks-1", "westeurope", "rg");
    fake.cli("az", "exit 0");
    let err = fake.clis().add(CloudProvider::Aks, "1", &aks, "", false).await.unwrap_err().to_string();
    assert_eq!(err, format!("az didn't add a context to {}", fake.config().display()));
    fake.cli("az", "echo 'ERROR: (AuthorizationFailed)' >&2; exit 1");
    assert_eq!(fake.clis().add(CloudProvider::Aks, "1", &aks, "", false).await.unwrap_err().to_string(), "ERROR: (AuthorizationFailed)");
    // Nowhere to put the kubeconfig.
    let mut homeless = fake.clis();
    homeless.kubeconfig = KubeconfigEnv::default();
    assert!(homeless.add(CloudProvider::Aks, "1", &aks, "", false).await.is_err());
    assert_eq!(fake.calls("az").len(), 2);
}

#[test]
fn reads_gke_projects_and_clusters() {
    let projects = gke_projects(&json!([
        {"projectId": "zeta-prod", "name": "Zeta"},
        {"projectId": "alpha", "name": "alpha"},
        {"name": "no id"}
    ]));
    assert_eq!(projects.iter().map(|p| p.label.as_str()).collect::<Vec<_>>(), ["alpha", "Zeta (zeta-prod)"]);
    let clusters = gke_clusters(&json!([
        {"name": "web", "location": "us-central1-a", "currentMasterVersion": "1.33.1-gke.100", "status": "RUNNING", "endpoint": "34.1.2.3"},
        {"name": "old", "zone": "europe-west1-b", "status": "STOPPING"}
    ]));
    assert_eq!(clusters[0].location, "us-central1-a");
    assert_eq!(clusters[0].version, "1.33.1-gke.100");
    assert_eq!(clusters[0].server, "34.1.2.3");
    assert_eq!(clusters[1].location, "europe-west1-b");
}

#[test]
fn gke_zones_and_regions() {
    assert_eq!(gke_location_flag("us-central1-a"), "--zone");
    assert_eq!(gke_location_flag("europe-west4-c"), "--zone");
    assert_eq!(gke_location_flag("us-central1"), "--region");
    assert_eq!(gke_location_flag("me-central2"), "--region");
}

#[test]
fn reads_eks_clusters() {
    let clusters = eks_clusters(&json!({"clusters": ["api", "batch"]}), "eu-west-1");
    assert_eq!(clusters.len(), 2);
    assert_eq!((clusters[1].name.as_str(), clusters[1].location.as_str()), ("batch", "eu-west-1"));
    assert!(eks_clusters(&json!({}), "eu-west-1").is_empty());
    let mut api = clusters[0].clone();
    let arn = "arn:aws:eks:eu-west-1:111122223333:cluster/api";
    eks_fill(&mut api, &json!({"cluster": {"name": "api", "version": "1.33", "status": "ACTIVE", "endpoint": "https://ABC.gr7.eu-west-1.eks.amazonaws.com", "arn": arn}}));
    assert_eq!((api.version.as_str(), api.status.as_str(), api.entry.as_str()), ("1.33", "ACTIVE", arn));
    assert_eq!(api.server, "https://ABC.gr7.eu-west-1.eks.amazonaws.com");
    eks_fill(&mut api, &json!({}));
    assert_eq!((api.version.as_str(), api.server.as_str()), ("", ""));
}

#[test]
fn reads_aks_subscriptions_and_clusters() {
    let subs = aks_subscriptions(&json!([
        {"id": "1111", "name": "Dev", "isDefault": false, "state": "Enabled"},
        {"id": "2222", "name": "Shared", "isDefault": true, "state": "Enabled"},
        {"id": "3333", "name": "Gone", "isDefault": false, "state": "Disabled"}
    ]));
    assert_eq!(subs.default.as_deref(), Some("2222"));
    assert_eq!(subs.accounts.len(), 2);
    let clusters = aks_clusters(&json!([
        {"name": "aks-1", "location": "westeurope", "resourceGroup": "rg-apps", "currentKubernetesVersion": "1.32.4", "powerState": {"code": "Running"}, "fqdn": "aks-1-dns.hcp.westeurope.azmk8s.io"},
        {"name": "aks-2", "location": "eastus", "resourceGroup": "rg-ops", "kubernetesVersion": "1.31", "provisioningState": "Failed", "privateFqdn": "aks-2.privatelink.eastus.azmk8s.io"}
    ]));
    assert_eq!((clusters[0].group.as_str(), clusters[0].version.as_str(), clusters[0].status.as_str()), ("rg-apps", "1.32.4", "Running"));
    assert_eq!((clusters[1].version.as_str(), clusters[1].status.as_str()), ("1.31", "Failed"));
    assert_eq!((clusters[0].server.as_str(), clusters[1].server.as_str()), ("aks-1-dns.hcp.westeurope.azmk8s.io", "aks-2.privatelink.eastus.azmk8s.io"));
}

fn cluster(name: &str, location: &str, group: &str) -> CloudCluster {
    CloudCluster { name: name.into(), location: location.into(), group: group.into(), version: String::new(), status: String::new(), server: String::new(), entry: String::new() }
}

#[test]
fn credential_arguments() {
    let path = Path::new("/home/me/.kube/config");
    let gke = credential_args(CloudProvider::Gke, "my-proj", &cluster("web", "us-central1", ""), Some("web"), path).unwrap();
    assert_eq!(gke, ["container", "clusters", "get-credentials", "web", "--region=us-central1", "--project=my-proj"]);
    let eks = credential_args(CloudProvider::Eks, "work", &cluster("api", "eu-west-1", ""), Some("api"), path).unwrap();
    assert_eq!(
        eks,
        ["--profile=work", "eks", "update-kubeconfig", "--name=api", "--region=eu-west-1", "--kubeconfig=/home/me/.kube/config", "--alias=api"]
    );
    let eks_default = credential_args(CloudProvider::Eks, "", &cluster("api", "eu-west-1", ""), None, path).unwrap();
    assert_eq!(eks_default[0], "eks");
    let aks = credential_args(CloudProvider::Aks, "2222", &cluster("aks-1", "westeurope", "rg-apps"), None, path).unwrap();
    assert_eq!(
        aks,
        [
            "aks",
            "get-credentials",
            "--subscription=2222",
            "--resource-group=rg-apps",
            "--name=aks-1",
            "--file=/home/me/.kube/config",
            "--overwrite-existing"
        ]
    );
}

#[test]
fn refuses_values_read_as_flags() {
    let path = Path::new("/c");
    assert!(credential_args(CloudProvider::Gke, "--impersonate-service-account=x", &cluster("web", "us-central1", ""), None, path).is_err());
    assert!(credential_args(CloudProvider::Gke, "p", &cluster("-web", "us-central1", ""), None, path).is_err());
    assert!(credential_args(CloudProvider::Aks, "s", &cluster("a", "eastus", ""), None, path).is_err());
    assert!(credential_args(CloudProvider::Eks, "p", &cluster("a", "eu-west-1\n", ""), None, path).is_ok());
    assert!(credential_args(CloudProvider::Eks, "p", &cluster("a", "eu\nwest", ""), None, path).is_err());
}

#[test]
fn an_aks_alias_names_the_context() {
    let aks = credential_args(CloudProvider::Aks, "1", &cluster("aks-1", "westeurope", "rg"), Some("dev"), Path::new("/c")).unwrap();
    assert_eq!(aks.last().map(String::as_str), Some("--context=dev"));
}
