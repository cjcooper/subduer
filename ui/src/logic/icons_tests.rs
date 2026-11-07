use super::*;

fn kind(group: &str, kind: &str) -> KindRef {
    KindRef { group: group.into(), version: "v1".into(), kind: kind.into(), plural: String::new(), namespaced: true }
}

#[test]
fn well_known_custom_resources() {
    assert_eq!(custom_paths(&kind("keda.sh", "ScaledObject")), Some(SCALE));
    assert_eq!(custom_paths(&kind("external-secrets.io", "ClusterSecretStore")), Some(VAULT));
    assert_eq!(custom_paths(&kind("kyverno.io", "PolicyException")), Some(SHIELD_OFF));
    assert_eq!(custom_paths(&kind("wgpolicyk8s.io", "ClusterPolicyReport")), Some(CLIPBOARD));
    assert_eq!(custom_paths(&kind("elbv2.k8s.aws", "TargetGroupBinding")), Some(TARGET));
    assert_eq!(custom_paths(&kind("gateway.k8s.aws", "LoadBalancerConfiguration")), Some(SLIDERS));
    assert_eq!(custom_paths(&kind("s3.services.k8s.aws", "Bucket")), Some(CLOUD));
    assert_eq!(custom_paths(&kind("networking.gke.io", "ManagedCertificate")), Some(SHIELD_CHECK));
    assert_eq!(custom_paths(&kind("cloud.google.com", "BackendConfig")), Some(SLIDERS));
    assert_eq!(custom_paths(&kind("resources.azure.com", "ResourceGroup")), Some(CLOUD));
    assert_eq!(custom_paths(&kind("monitoring.coreos.com", "ServiceMonitor")), Some(EYE));
    assert_eq!(custom_paths(&kind("monitoring.coreos.com", "PrometheusRule")), Some(LIST));
    assert_eq!(custom_paths(&kind("monitoring.coreos.com", "Prometheus")), Some(CHART));
    assert_eq!(custom_paths(&kind("example.com", "Widget")), None);
    assert_eq!(custom_paths(&kind("k8s.aws.example.com", "Widget")), None);
}

#[test]
fn every_built_in_kind_icon() {
    let cases = [
        ("", "Node", SERVER),
        ("storage.k8s.io", "CSINode", SERVER),
        ("", "Event", PULSE),
        ("", "Pod", CUBE),
        ("apps", "Deployment", LAYERS),
        ("apps", "StatefulSet", DATABASE),
        ("apps", "DaemonSet", GRID),
        ("apps", "ReplicaSet", COPY),
        ("", "ReplicationController", COPY),
        ("batch", "Job", PLAY),
        ("batch", "CronJob", CLOCK),
        ("", "ConfigMap", FILE),
        ("", "Secret", LOCK),
        ("autoscaling", "HorizontalPodAutoscaler", SCALE),
        ("policy", "PodDisruptionBudget", SHIELD),
        ("", "ResourceQuota", GAUGE),
        ("storage.k8s.io", "CSIStorageCapacity", GAUGE),
        ("", "LimitRange", RULER),
        ("scheduling.k8s.io", "PriorityClass", FLAG),
        ("node.k8s.io", "RuntimeClass", CHIP),
        ("coordination.k8s.io", "Lease", HOURGLASS),
        ("admissionregistration.k8s.io", "ValidatingWebhookConfiguration", PLUG),
        ("admissionregistration.k8s.io", "ValidatingAdmissionPolicy", SHIELD_CHECK),
        ("", "Service", SHARE),
        ("networking.k8s.io", "Ingress", ENTER),
        ("networking.k8s.io", "IngressClass", TAG),
        ("storage.k8s.io", "StorageClass", TAG),
        ("networking.k8s.io", "NetworkPolicy", WALL),
        ("discovery.k8s.io", "EndpointSlice", TARGET),
        ("", "Endpoints", TARGET),
        ("", "PersistentVolumeClaim", TRAY),
        ("", "PersistentVolume", DRIVE),
        ("storage.k8s.io", "VolumeAttachment", LINK),
        ("storage.k8s.io", "CSIDriver", PLUG),
        ("", "Namespace", FOLDER),
        ("", "ServiceAccount", USER),
        ("rbac.authorization.k8s.io", "Role", KEY),
        ("rbac.authorization.k8s.io", "ClusterRole", KEY),
        ("rbac.authorization.k8s.io", "RoleBinding", LINK),
        ("helm.sh", "Release", HELM),
        ("apiextensions.k8s.io", "CustomResourceDefinition", PUZZLE),
        ("example.com", "Widget", BOX),
    ];
    for (group, k, want) in cases {
        assert_eq!(kind_paths(&kind(group, k)), want, "{group}/{k}");
        assert_eq!(kind_icon(&kind(group, k), false), want, "{group}/{k}");
    }
}

#[test]
fn every_custom_resource_icon() {
    let cases = [
        ("kyverno.io", "PolicyException", SHIELD_OFF),
        ("kyverno.io", "CleanupPolicy", ERASER),
        ("kyverno.io", "UpdateRequest", REFRESH),
        ("kyverno.io", "GlobalContextEntry", GLOBE),
        ("reports.kyverno.io", "EphemeralReport", CLIPBOARD),
        ("kyverno.io", "ClusterPolicy", SHIELD_CHECK),
        ("datadoghq.com", "DatadogMonitor", BELL),
        ("datadoghq.com", "DatadogDashboard", DASHBOARD),
        ("datadoghq.com", "DatadogSLO", TARGET),
        ("datadoghq.com", "DatadogPodAutoscaler", SCALE),
        ("datadoghq.com", "DatadogAgent", SERVER),
        ("datadoghq.com", "DatadogGenericResource", BOX),
        ("datadoghq.com", "DatadogMetric", CHART),
        ("monitoring.coreos.com", "PrometheusAgent", ANTENNA),
        ("monitoring.coreos.com", "PodMonitor", EYE),
        ("monitoring.coreos.com", "Probe", TARGET),
        ("monitoring.coreos.com", "ThanosRuler", LIST),
        ("monitoring.coreos.com", "Alertmanager", BELL),
        ("monitoring.coreos.com", "ScrapeConfig", SLIDERS),
        ("keda.sh", "ScaledJob", PLAY),
        ("keda.sh", "TriggerAuthentication", KEY),
        ("keda.sh", "ScaledObject", SCALE),
        ("eventing.keda.sh", "CloudEventSource", BROADCAST),
        ("external-secrets.io", "SecretStore", VAULT),
        ("external-secrets.io", "PushSecret", SECRET_OUT),
        ("external-secrets.io", "ExternalSecret", SECRET_IN),
        ("generators.external-secrets.io", "SSHKey", KEY),
        ("generators.external-secrets.io", "GeneratorState", CLIPBOARD),
        ("generators.external-secrets.io", "Password", WAND),
        ("elbv2.k8s.aws", "TargetGroupBinding", TARGET),
        ("networking.gke.io", "ManagedCertificate", SHIELD_CHECK),
        ("monitoring.googleapis.com", "PodMonitoring", CHART),
        ("containerservice.azure.com", "ManagedCluster", SERVER),
        ("networking.k8s.aws", "ApplicationNetworkPolicy", WALL),
        ("vpcresources.k8s.aws", "SecurityGroupPolicy", SHIELD),
        ("aga.k8s.aws", "GlobalAccelerator", GLOBE),
        ("vpcresources.k8s.aws", "CNINode", SERVER),
        ("crd.k8s.amazonaws.com", "ENIConfig", CHIP),
        ("gateway.k8s.aws", "TargetGroupConfiguration", SLIDERS),
        ("karpenter.k8s.aws", "EC2NodeClassParams", SLIDERS),
        ("cloud.google.com", "BackendConfig", SLIDERS),
        ("s3.services.k8s.aws", "Bucket", CLOUD),
    ];
    for (group, k, want) in cases {
        assert_eq!(custom_paths(&kind(group, k)), Some(want), "{group}/{k}");
        assert_eq!(kind_icon(&kind(group, k), true), want, "{group}/{k}");
    }
    // Unknown custom resources get the puzzle piece, even with a built-in name.
    assert_eq!(kind_icon(&kind("example.com", "Service"), true), PUZZLE);
}

#[test]
fn group_and_category_icons() {
    let groups = [
        ("wgpolicyk8s.io", Some(SHIELD_CHECK)),
        ("monitoring.coreos.com", Some(CHART)),
        ("keda.sh", Some(SCALE)),
        ("eventing.keda.sh", Some(BROADCAST)),
        ("external-secrets.io", Some(SECRET_IN)),
        ("generators.external-secrets.io", Some(WAND)),
        ("networking.gke.io", Some(CLOUD)),
        ("example.com", None),
    ];
    for (group, want) in groups {
        assert_eq!(group_paths(group), want, "{group}");
    }
    let categories = [
        (Category::TopLevel, CLUSTER),
        (Category::Workloads, CUBE),
        (Category::Helm, HELM),
        (Category::Aws, CLOUD),
        (Category::GoogleCloud, CLOUD),
        (Category::Azure, CLOUD),
        (Category::Config, SLIDERS),
        (Category::Network, GLOBE),
        (Category::Storage, DATABASE),
        (Category::Namespaces, FOLDER),
        (Category::AccessControl, SHIELD_CHECK),
        (Category::CustomResources, PUZZLE),
        (Category::Other, BOX),
    ];
    for (category, want) in categories {
        assert_eq!(category_paths(category), want, "{category:?}");
    }
}

#[test]
fn files_locks_and_providers() {
    assert_eq!(entry_paths(true), FOLDER);
    assert_eq!(entry_paths(false), FILE);
    assert_eq!(lock_paths(true), UNLOCK);
    assert_eq!(lock_paths(false), LOCK);
    assert_eq!(provider_mark(None), (KUBERNETES_LOGO, "provider-logo kubernetes", "0 0 24 24"));
    assert_eq!(provider_mark(Some(CloudProvider::Gke)), (GOOGLE_CLOUD_LOGO, "provider-logo google-cloud", "0 0 24 24"));
    assert_eq!(provider_mark(Some(CloudProvider::Eks)), (AWS_LOGO, "provider-logo aws", "0 0 128 128"));
    assert_eq!(provider_mark(Some(CloudProvider::Aks)), (AZURE_LOGO, "provider-logo azure", "0 0 128 128"));
    for paths in [CLUSTER, FORWARD, TRACES] {
        assert!(paths.starts_with('<'), "{paths}");
    }
}
