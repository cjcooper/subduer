//! File › Add Cluster… from a cloud: lists clusters with the provider's own
//! CLI (`gcloud`, `aws`, `az`) and lets it write the kubeconfig entries, as
//! `gcloud container clusters get-credentials`, `aws eks update-kubeconfig`
//! and `az aks get-credentials` do from a terminal. Their entries call the
//! provider's auth plugin, so they keep working in kubectl and other tools.
//!
//! The CLIs run without a shell or a terminal (a prompt fails instead of
//! waiting), with the user's sign-in; this app never sees the credentials.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;

use crate::add_cluster::{self, KubeconfigEnv};
use crate::types::{AddedCluster, CloudAccount, CloudAccounts, CloudCluster, CloudProvider};
use crate::{Error, Result};

/// Listing waits this long (a first `gcloud` run can be slow).
const LIST_TIMEOUT: Duration = Duration::from_secs(90);
/// Fetching credentials waits this long.
const ADD_TIMEOUT: Duration = Duration::from_secs(180);

/// The CLI's program name, and what to do when it's missing.
fn cli(provider: CloudProvider) -> (&'static str, &'static str) {
    match provider {
        // On Windows gcloud and az are batch files, which need their extension.
        CloudProvider::Gke => (
            if cfg!(windows) { "gcloud.cmd" } else { "gcloud" },
            "The gcloud command isn't installed or isn't on PATH. Install the Google Cloud CLI \
             (https://cloud.google.com/sdk/docs/install) and gke-gcloud-auth-plugin, then run gcloud auth login.",
        ),
        CloudProvider::Eks => (
            "aws",
            "The aws command isn't installed or isn't on PATH. Install the AWS CLI \
             (https://aws.amazon.com/cli/), then sign in with aws configure or aws sso login.",
        ),
        CloudProvider::Aks => (
            if cfg!(windows) { "az.cmd" } else { "az" },
            "The az command isn't installed or isn't on PATH. Install the Azure CLI \
             (https://learn.microsoft.com/cli/azure/install-azure-cli) and kubelogin, then run az login.",
        ),
    }
}

/// Where the CLIs and the kubeconfig are, and how long to wait for the CLIs.
/// [`Clis::system`] in the app; tests point it at fake CLIs in a scratch
/// folder.
#[derive(Clone, Debug)]
pub struct Clis {
    /// The folder the CLIs are in; empty finds them on `PATH`.
    pub dir: PathBuf,
    /// Reads an environment variable (`AWS_PROFILE`, `AWS_REGION`, …).
    pub var: fn(&str) -> Option<String>,
    /// Where the kubeconfig is.
    pub kubeconfig: KubeconfigEnv,
    /// How long listing waits.
    pub list_timeout: Duration,
    /// How long fetching credentials waits.
    pub add_timeout: Duration,
}

impl Clis {
    /// The CLIs on `PATH`, with this process's environment.
    pub fn system() -> Self {
        Self {
            dir: PathBuf::new(),
            var: |name| std::env::var(name).ok(),
            kubeconfig: KubeconfigEnv::current(),
            list_timeout: LIST_TIMEOUT,
            add_timeout: ADD_TIMEOUT,
        }
    }

    /// Runs the provider's CLI with `args` (no shell, no stdin) and returns its
    /// standard output, or its error output as the error.
    async fn run(&self, provider: CloudProvider, args: &[String]) -> Result<String> {
        self.run_both(provider, args, None, self.list_timeout).await.map(|(stdout, _)| stdout)
    }

    /// [`Clis::run`], also returning the error output of a run that
    /// succeeded (where the CLIs print their progress and notes).
    async fn run_both(&self, provider: CloudProvider, args: &[String], kubeconfig: Option<&Path>, timeout: Duration) -> Result<(String, String)> {
        let (program, missing) = cli(provider);
        let path = self.dir.join(program);
        #[allow(unused_mut)]
        let mut cmd = tokio::process::Command::new(path);
        cmd.args(args).stdin(Stdio::null()).kill_on_drop(true);
        // gcloud has no flag for the file; all three honour this.
        if let Some(path) = kubeconfig {
            cmd.env("KUBECONFIG", path);
        }
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let what = format!("{program} {}", args.iter().take(3).map(String::as_str).collect::<Vec<_>>().join(" "));
        let out = tokio::time::timeout(timeout, cmd.output())
            .await
            .map_err(|_| Error::Message(format!("{what} didn't finish within {}s", timeout.as_secs())))?
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => Error::Message(missing.into()),
                _ => Error::Message(format!("couldn't run {program}: {e}")),
            })?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        tracing::info!(%what, ok = out.status.success(), "cloud CLI");
        if out.status.success() {
            Ok((stdout, stderr))
        } else {
            Err(Error::Message(if stderr.is_empty() { format!("{what} failed ({})", out.status) } else { stderr }))
        }
    }

    /// The projects (GKE), profiles (EKS) or subscriptions (AKS) to pick from.
    pub async fn accounts(&self, provider: CloudProvider) -> Result<CloudAccounts> {
        match provider {
            CloudProvider::Gke => {
                let list = self.run(provider, &args(["projects", "list", "--format=json(projectId,name)"])).await?;
                let default = self.run(provider, &args(["config", "get-value", "project"])).await.ok();
                Ok(CloudAccounts { accounts: gke_projects(&json(&list)?), default: default.filter(|p| !p.is_empty()), region: None })
            }
            CloudProvider::Eks => {
                let list = self.run(provider, &args(["configure", "list-profiles"])).await?;
                let names: Vec<String> = list.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
                let regions = futures::future::join_all(names.iter().map(|p| async move {
                    self.run(provider, &[format!("--profile={p}"), "configure".into(), "get".into(), "region".into()])
                        .await
                        .ok()
                        .filter(|r| !r.is_empty())
                }))
                .await;
                let accounts = names
                    .into_iter()
                    .zip(regions)
                    .map(|(name, region)| CloudAccount { id: name.clone(), label: name, region })
                    .collect();
                let default = (self.var)("AWS_PROFILE").filter(|p| !p.is_empty());
                let region = match ["AWS_REGION", "AWS_DEFAULT_REGION"].iter().find_map(|v| (self.var)(v).filter(|r| !r.is_empty())) {
                    Some(region) => Some(region),
                    None => self.run(provider, &args(["configure", "get", "region"])).await.ok().filter(|r| !r.is_empty()),
                };
                Ok(CloudAccounts { accounts, default, region })
            }
            CloudProvider::Aks => {
                let list = self.run(provider, &args(["account", "list", "--output=json"])).await?;
                Ok(aks_subscriptions(&json(&list)?))
            }
        }
    }

    /// The clusters in `account` (EKS: in `region`).
    pub async fn clusters(&self, provider: CloudProvider, account: &str, region: &str) -> Result<Vec<CloudCluster>> {
        let mut list = match provider {
            CloudProvider::Gke => {
                let project = arg("project", account)?;
                let out = self.run(provider, &args(["container", "clusters", "list", &format!("--project={project}"), "--format=json"])).await?;
                let mut list = gke_clusters(&json(&out)?);
                for c in &mut list {
                    c.entry = format!("gke_{account}_{}_{}", c.location, c.name);
                }
                list
            }
            CloudProvider::Eks => {
                let region = arg("region", region)?;
                let profile = eks_profile(account)?;
                let mut a = profile.clone();
                a.extend(["eks".into(), "list-clusters".into(), format!("--region={region}"), "--output=json".into()]);
                let out = self.run(provider, &a).await?;
                let mut list = eks_clusters(&json(&out)?, &region);
                self.eks_describe(&profile, &mut list).await;
                list
            }
            CloudProvider::Aks => {
                let subscription = arg("subscription", account)?;
                let out = self.run(provider, &args(["aks", "list", &format!("--subscription={subscription}"), "--output=json"])).await?;
                aks_clusters(&json(&out)?)
            }
        };
        list.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.location.cmp(&b.location)));
        Ok(list)
    }

    /// `list-clusters` gives only names: each cluster's version and status
    /// come from `describe-cluster`, a few at a time (with `profile`, the
    /// `--profile` argument if any). One that can't be described (no
    /// `eks:DescribeCluster` permission, say) is listed without them.
    async fn eks_describe(&self, profile: &[String], list: &mut [CloudCluster]) {
        use futures::StreamExt;
        let commands: Vec<Option<Vec<String>>> = list
            .iter()
            .map(|c| {
                let mut a = profile.to_vec();
                a.extend([
                    "eks".into(),
                    "describe-cluster".into(),
                    format!("--name={}", arg("cluster", &c.name).ok()?),
                    format!("--region={}", c.location),
                    "--output=json".into(),
                ]);
                Some(a)
            })
            .collect();
        let described: Vec<Option<Value>> = futures::stream::iter(commands)
            .map(|a| async move {
                let out = self.run(CloudProvider::Eks, &a?).await.ok()?;
                json(&out).ok()
            })
            .buffered(EKS_DESCRIBE_AT_ONCE)
            .collect()
            .await;
        for (cluster, found) in list.iter_mut().zip(described) {
            if let Some(described) = found {
                eks_fill(cluster, &described);
            }
        }
    }

    /// Has the provider's CLI add `cluster` to the kubeconfig, named `alias`
    /// when given. The CLIs make it kubectl's current context; unless
    /// `make_current`, the previous one is put back.
    pub async fn add(&self, provider: CloudProvider, account: &str, cluster: &CloudCluster, alias: &str, make_current: bool) -> Result<AddedCluster> {
        let path = self.kubeconfig.path()?;
        let alias = alias.trim();
        let alias = (!alias.is_empty()).then(|| arg("context name", alias)).transpose()?;
        if let Some(alias) = &alias
            && add_cluster::TakenNames::read_in(&self.kubeconfig)?.contexts.iter().any(|c| c == alias)
        {
            return Err(Error::Message(format!("A context named {alias:?} already exists.")));
        }
        let args = credential_args(provider, account, cluster, alias.as_deref(), &path)?;
        let before = add_cluster::current_context(&path)?;
        let (stdout, stderr) = self.run_both(provider, &args, Some(&path), self.add_timeout).await?;
        let output = [stderr, stdout].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
        // All three end on the new context.
        let mut context = add_cluster::current_context(&path)?
            .ok_or_else(|| Error::Message(format!("{} didn't add a context to {}", cli(provider).0, path.display())))?;
        if provider == CloudProvider::Gke
            && let Some(alias) = alias
            && alias != context
        {
            add_cluster::rename_context(&path, &context, &alias)?;
            context = alias;
        }
        if !make_current && before.as_deref() != Some(context.as_str()) {
            add_cluster::set_current_context(&path, before.as_deref())?;
        }
        tracing::info!(?provider, %context, path = %path.display(), "added cloud cluster to kubeconfig");
        Ok(AddedCluster { context, output })
    }
}

/// A value passed to a CLI flag: not empty, not something it would read as
/// a flag of its own.
fn arg(what: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_control) {
        return Err(Error::Message(format!("not a valid {what}: {value:?}")));
    }
    Ok(value.to_owned())
}

fn json(text: &str) -> Result<Value> {
    serde_json::from_str(if text.is_empty() { "null" } else { text }).map_err(|e| Error::Message(format!("unexpected CLI output: {e}")))
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn args<const N: usize>(a: [&str; N]) -> Vec<String> {
    a.iter().map(|s| (*s).to_owned()).collect()
}

fn gke_projects(list: &Value) -> Vec<CloudAccount> {
    let mut projects: Vec<CloudAccount> = list
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let id = text(p, "projectId");
            let name = text(p, "name");
            let label = if name.is_empty() || name == id { id.clone() } else { format!("{name} ({id})") };
            CloudAccount { id, label, region: None }
        })
        .filter(|p| !p.id.is_empty())
        .collect();
    projects.sort_by_key(|p| p.label.to_lowercase());
    projects
}

fn aks_subscriptions(list: &Value) -> CloudAccounts {
    let subs: Vec<&Value> = list.as_array().into_iter().flatten().filter(|s| text(s, "state").is_empty() || text(s, "state") == "Enabled").collect();
    let default = subs.iter().find(|s| s.get("isDefault").and_then(Value::as_bool) == Some(true)).map(|s| text(s, "id"));
    let accounts = subs
        .into_iter()
        .map(|s| CloudAccount { id: text(s, "id"), label: text(s, "name"), region: None })
        .filter(|s| !s.id.is_empty())
        .collect();
    CloudAccounts { accounts, default, region: None }
}

/// `--profile=<p>`, or nothing for the default credentials.
fn eks_profile(profile: &str) -> Result<Vec<String>> {
    Ok(if profile.trim().is_empty() { vec![] } else { vec![format!("--profile={}", arg("profile", profile)?)] })
}

fn gke_clusters(list: &Value) -> Vec<CloudCluster> {
    list.as_array()
        .into_iter()
        .flatten()
        .map(|c| CloudCluster {
            name: text(c, "name"),
            location: if text(c, "location").is_empty() { text(c, "zone") } else { text(c, "location") },
            group: String::new(),
            version: text(c, "currentMasterVersion"),
            status: text(c, "status"),
            server: text(c, "endpoint"),
            // Needs the project: set by `clusters`.
            entry: String::new(),
        })
        .filter(|c| !c.name.is_empty())
        .collect()
}

fn eks_clusters(list: &Value, region: &str) -> Vec<CloudCluster> {
    list.get("clusters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|name| CloudCluster {
            name: name.to_owned(),
            location: region.to_owned(),
            group: String::new(),
            version: String::new(),
            status: String::new(),
            server: String::new(),
            entry: String::new(),
        })
        .collect()
}


/// How many `aws eks describe-cluster` run at once.
const EKS_DESCRIBE_AT_ONCE: usize = 6;

/// What `describe-cluster` adds: version, status, endpoint and ARN (the
/// name `update-kubeconfig` gives the cluster entry).
fn eks_fill(cluster: &mut CloudCluster, described: &Value) {
    let c = described.get("cluster").unwrap_or(&Value::Null);
    (cluster.version, cluster.status, cluster.server, cluster.entry) = (text(c, "version"), text(c, "status"), text(c, "endpoint"), text(c, "arn"));
}

fn aks_clusters(list: &Value) -> Vec<CloudCluster> {
    list.as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let power = c.get("powerState").map(|p| text(p, "code")).unwrap_or_default();
            let version = if text(c, "currentKubernetesVersion").is_empty() { text(c, "kubernetesVersion") } else { text(c, "currentKubernetesVersion") };
            CloudCluster {
                name: text(c, "name"),
                location: text(c, "location"),
                group: text(c, "resourceGroup"),
                version,
                status: if power.is_empty() { text(c, "provisioningState") } else { power },
                // A private cluster's API server has only its private name.
                server: if text(c, "fqdn").is_empty() { text(c, "privateFqdn") } else { text(c, "fqdn") },
                // `get-credentials` names the entry after the cluster alone: not enough to tell two apart.
                entry: String::new(),
            }
        })
        .filter(|c| !c.name.is_empty())
        .collect()
}

/// A GKE location is a zone (`us-central1-a`) or a region (`us-central1`).
fn gke_location_flag(location: &str) -> &'static str {
    let last = location.rsplit('-').next().unwrap_or_default();
    if location.matches('-').count() >= 2 && last.len() == 1 { "--zone" } else { "--region" }
}

/// The CLI arguments that write `cluster`'s kubeconfig entries to `kubeconfig`.
fn credential_args(provider: CloudProvider, account: &str, cluster: &CloudCluster, alias: Option<&str>, kubeconfig: &Path) -> Result<Vec<String>> {
    let name = arg("cluster name", &cluster.name)?;
    let location = arg("location", &cluster.location)?;
    Ok(match provider {
        CloudProvider::Gke => vec![
            "container".into(),
            "clusters".into(),
            "get-credentials".into(),
            name,
            format!("{}={location}", gke_location_flag(&location)),
            format!("--project={}", arg("project", account)?),
        ],
        CloudProvider::Eks => {
            let mut a = eks_profile(account)?;
            a.extend([
                "eks".into(),
                "update-kubeconfig".into(),
                format!("--name={name}"),
                format!("--region={location}"),
                format!("--kubeconfig={}", kubeconfig.display()),
            ]);
            if let Some(alias) = alias {
                a.push(format!("--alias={alias}"));
            }
            a
        }
        CloudProvider::Aks => {
            let mut a = vec![
                "aks".into(),
                "get-credentials".into(),
                format!("--subscription={}", arg("subscription", account)?),
                format!("--resource-group={}", arg("resource group", &cluster.group)?),
                format!("--name={name}"),
                format!("--file={}", kubeconfig.display()),
                // Without a terminal, its "overwrite?" question would fail.
                "--overwrite-existing".into(),
            ];
            if let Some(alias) = alias {
                a.push(format!("--context={alias}"));
            }
            a
        }
    })
}


#[cfg(test)]
#[path = "cloud_tests.rs"]
mod tests;
