//! Add Cluster: matching clusters already in the kubeconfig, building the
//! typed-in cluster, choosing a cloud account, and the dialog's wording.

use subduer_types::{CloudAccounts, CloudCluster, CloudProvider, ClusterAuth, ManagedContext, NewCluster};

/// An API server's host and port, lowercase, without `https://`, a path or
/// the default port: clouds report a bare host or a URL, kubeconfigs a URL.
pub(crate) fn server_key(server: &str) -> String {
    let rest = server.trim().split_once("://").map_or(server.trim(), |(_, r)| r);
    let host = rest.split('/').next().unwrap_or("").to_ascii_lowercase();
    host.strip_suffix(":443").map(str::to_owned).unwrap_or(host)
}

/// The contexts already pointing at a cluster: same API server, or (from a
/// cloud's CLI) the cluster entry it would write. Renamed contexts count too.
pub(crate) fn already_in(existing: &[ManagedContext], server: &str, entry: &str) -> Vec<String> {
    let key = server_key(server);
    existing
        .iter()
        .filter(|c| (!key.is_empty() && server_key(&c.server) == key) || (!entry.is_empty() && c.cluster == entry))
        .map(|c| c.name.clone())
        .collect()
}

/// "“a”", "“a” and “b”", "“a”, “b” and “c”".
pub(crate) fn quoted_names(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("“{n}”")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// Where a new cluster comes from: typed in (`None`) or a cloud.
pub(crate) type Source = Option<CloudProvider>;

pub(crate) const SOURCES: [(Source, &str, &str); 4] = [
    (Some(CloudProvider::Eks), "AWS", "EKS, with the aws CLI"),
    (Some(CloudProvider::Gke), "Google Cloud", "GKE, with gcloud"),
    (Some(CloudProvider::Aks), "Azure", "AKS, with the az CLI"),
    (None, "Kubernetes", "Server and credentials"),
];

/// The card the dialog opens on.
pub(crate) const DEFAULT_SOURCE: Source = Some(CloudProvider::Eks);

/// Regions offered for EKS; any other can be typed.
pub(crate) const AWS_REGIONS: [&str; 17] = [
    "us-east-1", "us-east-2", "us-west-1", "us-west-2", "ca-central-1", "sa-east-1", "eu-west-1", "eu-west-2",
    "eu-west-3", "eu-central-1", "eu-north-1", "ap-south-1", "ap-southeast-1", "ap-southeast-2", "ap-northeast-1",
    "ap-northeast-2", "me-central-1",
];

/// What the cloud's account picker is called.
pub(crate) fn account_label(provider: CloudProvider) -> &'static str {
    match provider {
        CloudProvider::Gke => "Project",
        CloudProvider::Eks => "Profile",
        CloudProvider::Aks => "Subscription",
    }
}

pub(crate) fn cli_name(provider: CloudProvider) -> &'static str {
    match provider {
        CloudProvider::Gke => "gcloud",
        CloudProvider::Eks => "aws",
        CloudProvider::Aks => "az",
    }
}

/// What the context a cloud's CLI writes runs for credentials.
pub(crate) fn credential_command(provider: CloudProvider) -> &'static str {
    match provider {
        CloudProvider::Gke => "gke-gcloud-auth-plugin",
        CloudProvider::Eks => "aws eks get-token",
        CloudProvider::Aks => "kubelogin",
    }
}

/// Exec plugin arguments, one per line.
pub(crate) fn split_args(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

/// The authentication choices: (value, label).
pub(crate) const AUTHS: [(&str, &str); 4] = [("token", "Bearer token"), ("cert", "Client certificate"), ("exec", "Exec plugin"), ("none", "None")];

/// The authentication the form starts on.
pub(crate) const DEFAULT_AUTH: &str = "token";

/// The authentication choice for a `<select>` value; unknown ones are a token.
pub(crate) fn auth_choice(value: &str) -> &'static str {
    AUTHS.iter().find(|(k, _)| *k == value).map_or(DEFAULT_AUTH, |(k, _)| *k)
}

/// The typed-in form's values.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ManualInput {
    pub(crate) name: String,
    pub(crate) server: String,
    pub(crate) ca: String,
    pub(crate) insecure: bool,
    pub(crate) proxy: String,
    /// `none`, `token`, `cert` or `exec`.
    pub(crate) auth: &'static str,
    pub(crate) token: String,
    pub(crate) cert: String,
    pub(crate) key: String,
    pub(crate) command: String,
    pub(crate) args: String,
    pub(crate) namespace: String,
}

impl ManualInput {
    /// The cluster to add.
    pub(crate) fn cluster(self, make_current: bool) -> NewCluster {
        let auth = match self.auth {
            "none" => ClusterAuth::None,
            "cert" => ClusterAuth::ClientCertificate { certificate: self.cert, key: self.key },
            "exec" => ClusterAuth::Exec { command: self.command, args: split_args(&self.args) },
            _ => ClusterAuth::Token { token: self.token },
        };
        NewCluster {
            name: self.name,
            server: self.server,
            certificate_authority: self.ca,
            insecure_skip_tls_verify: self.insecure,
            proxy_url: self.proxy,
            auth,
            namespace: self.namespace,
            make_current,
        }
    }
}

/// Something was typed: only then is what's missing pointed out.
pub(crate) fn touched(name: &str, server: &str) -> bool {
    !name.is_empty() || !server.is_empty()
}

/// What stops Add: the typed-in form's problem, or no cloud cluster picked.
pub(crate) fn problem(source: Source, manual: &dyn Fn() -> Option<&'static str>, picked: bool) -> Option<&'static str> {
    match source {
        None => manual(),
        Some(_) if !picked => Some("Pick a cluster."),
        Some(_) => None,
    }
}

/// Whether the typed-in form's problem shows: only once something's typed,
/// and not while adding.
pub(crate) fn shows_problem(source: Source, touched: bool, busy: bool) -> bool {
    source.is_none() && touched && !busy
}

/// Whether picking source `s` changes nothing: it's already the one shown
/// (and a cloud's accounts have been asked for).
pub(crate) fn same_source(current: Source, s: Source, accounts_asked: bool) -> bool {
    current == s && (s.is_none() || accounts_asked)
}

/// The Add button's text.
pub(crate) fn add_label(busy: bool, source: Source) -> String {
    match (busy, source) {
        (true, Some(p)) => format!("Running {}…", cli_name(p)),
        (true, None) => "Adding…".to_owned(),
        (false, _) => "Add".to_owned(),
    }
}

/// The account to list clusters for once the CLI answers: its default, else
/// (except EKS, which can list with the default credentials, without a
/// profile) the first one.
pub(crate) fn default_account(provider: CloudProvider, found: &CloudAccounts) -> String {
    let first = found.accounts.first().map(|a| a.id.clone());
    found.default.clone().or(if provider == CloudProvider::Eks { None } else { first }).unwrap_or_default()
}

/// The region to list EKS clusters in for `account`: the profile's own,
/// else the default credentials' one. `None` keeps the region typed, and
/// for other clouds.
pub(crate) fn account_region(provider: CloudProvider, found: &CloudAccounts, account: &str) -> Option<String> {
    if provider != CloudProvider::Eks {
        return None;
    }
    found.accounts.iter().find(|a| a.id == account).and_then(|a| a.region.clone()).or_else(|| found.region.clone())
}

/// Whether there's enough to list clusters: a region for EKS, an account for the others.
pub(crate) fn ready_to_list(provider: CloudProvider, account: &str, region: &str) -> bool {
    match provider {
        CloudProvider::Eks => !region.trim().is_empty(),
        _ => !account.is_empty(),
    }
}

/// The cluster to pick straight away: the only one listed.
pub(crate) fn only_cluster(result: &Result<Vec<CloudCluster>, String>) -> Option<CloudCluster> {
    match result.as_deref() {
        Ok([only]) => Some(only.clone()),
        _ => None,
    }
}

/// Whether the account picker is empty: a cloud other than EKS (which can
/// use the default credentials) listed no accounts.
pub(crate) fn no_accounts(provider: CloudProvider, found: &CloudAccounts) -> bool {
    found.accounts.is_empty() && provider != CloudProvider::Eks
}

/// Shown when [`no_accounts`].
pub(crate) fn no_accounts_text(provider: CloudProvider) -> String {
    format!("{} lists no {}s for this sign-in.", cli_name(provider), account_label(provider).to_lowercase())
}

pub(crate) fn asking_text(provider: CloudProvider) -> String {
    format!("Asking {}…", cli_name(provider))
}

pub(crate) fn sign_in_text(provider: CloudProvider) -> String {
    format!("Uses your {} sign-in. The context it writes runs ", cli_name(provider))
}

/// The context name field's placeholder: what the CLI will call it.
pub(crate) fn alias_placeholder(provider: CloudProvider, account: &str, picked: Option<&CloudCluster>) -> String {
    match (provider, picked) {
        (_, None) => "as the CLI names it".to_owned(),
        (CloudProvider::Gke, Some(c)) => format!("gke_{account}_{}_{}", c.location, c.name),
        (CloudProvider::Eks, Some(c)) => format!("the cluster's ARN (…:cluster/{})", c.name),
        (CloudProvider::Aks, Some(c)) => c.name.clone(),
    }
}

/// The cluster table's location heading.
pub(crate) fn where_label(provider: CloudProvider) -> &'static str {
    if provider == CloudProvider::Aks { "Region · group" } else { "Location" }
}

/// Why no clusters are listed.
pub(crate) fn no_clusters_text(provider: CloudProvider, region: &str) -> &'static str {
    if provider == CloudProvider::Eks && region.trim().is_empty() { "Choose a region." } else { "No clusters here." }
}

/// Where a cloud cluster is: its location, and (Azure) resource group.
pub(crate) fn place(c: &CloudCluster) -> String {
    if c.group.is_empty() { c.location.clone() } else { format!("{} · {}", c.location, c.group) }
}

/// The contexts already using the typed-in server (none until it's a server).
pub(crate) fn same_server(existing: &[ManagedContext], server: &str) -> Vec<String> {
    if server_key(server).is_empty() {
        return vec![];
    }
    already_in(existing, server, "")
}

/// Says the typed-in server is in the kubeconfig already.
pub(crate) fn server_already_text(names: &[String]) -> Option<String> {
    (!names.is_empty()).then(|| format!("This server is already in your kubeconfig as {}.", quoted_names(names)))
}

/// Says the picked cloud cluster is in the kubeconfig already.
pub(crate) fn cluster_already_text(names: &[String]) -> Option<String> {
    (!names.is_empty()).then(|| {
        let what = if names.len() == 1 { "context" } else { "contexts" };
        format!("This cluster is already in your kubeconfig as {what} {}. ", quoted_names(names))
    })
}

#[cfg(test)]
#[path = "add_cluster_tests.rs"]
mod tests;
