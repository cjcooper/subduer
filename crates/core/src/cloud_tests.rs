use super::*;

use serde_json::json;

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
