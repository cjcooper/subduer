//! A container's image metadata, read from its registry as `docker inspect`
//! and `docker history` show it: the config (entrypoint, env, labels, …) and
//! build history of the exact digest the container runs.
//!
//! Read-only and small: one manifest (two for a multi-platform image) and the
//! config blob, never the layers. Credentials are tried in turn until the
//! registry accepts one: the pod's image pull secrets, the user's Docker
//! credentials (`~/.docker/config.json` and its credential helpers), the AWS
//! CLI for ECR, then none. They're never logged or returned.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use base64::Engine;
use k8s_openapi::api::core::v1::{Node, Pod, Secret};
use kube::api::Api;
use kube::Client;
use oci_client::client::{ClientConfig, ClientProtocol};
use oci_client::errors::{OciDistributionError, OciErrorCode};
use oci_client::manifest::{ImageIndexEntry, OciImageManifest};
use oci_client::secrets::RegistryAuth;
use oci_client::Reference;
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::types::{ImageLayer, ImageMetadata};
use crate::{Error, Result};

/// One registry attempt (token, manifests, config) waits this long.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// A credential helper or the AWS CLI waits this long.
const TOOL_TIMEOUT: Duration = Duration::from_secs(60);

/// Where credentials come from outside the cluster. [`RegistryEnv::system`]
/// in the app; tests point it at scratch files and a plain-HTTP registry.
#[derive(Clone, Debug)]
pub struct RegistryEnv {
    /// The Docker client config (`$DOCKER_CONFIG/config.json`, else
    /// `~/.docker/config.json`).
    pub docker_config: Option<PathBuf>,
    /// The folder credential helpers and `aws` are in; empty finds them on `PATH`.
    pub tools_dir: PathBuf,
    /// The AWS profile for ECR, e.g. the one the context's `aws eks get-token` uses.
    pub aws_profile: Option<String>,
    /// Registries reached over plain HTTP (tests).
    pub plain_http: Vec<String>,
    /// How long one registry attempt waits.
    pub timeout: Duration,
    /// How long a credential helper or the AWS CLI waits.
    pub tool_timeout: Duration,
}

impl RegistryEnv {
    pub fn system(aws_profile: Option<String>) -> Self {
        let docker_config = std::env::var_os("DOCKER_CONFIG")
            .map(PathBuf::from)
            .or_else(|| std::env::home_dir().map(|h| h.join(".docker")))
            .map(|dir| dir.join("config.json"));
        Self { docker_config, tools_dir: PathBuf::new(), aws_profile, plain_http: vec![], timeout: FETCH_TIMEOUT, tool_timeout: TOOL_TIMEOUT }
    }
}

/// The AWS profile `context`'s user signs in with (`AWS_PROFILE` in its exec
/// env, or `--profile`), so ECR is read as the same account.
pub fn aws_profile(kubeconfig: &kube::config::Kubeconfig, context: &str) -> Option<String> {
    let user = kubeconfig.contexts.iter().find(|c| c.name == context)?.context.as_ref()?.user.clone()?;
    let exec = kubeconfig.auth_infos.iter().find(|u| u.name == user)?.auth_info.as_ref()?.exec.as_ref()?;
    let from_env = exec.env.iter().flatten().find(|e| e.get("name").map(String::as_str) == Some("AWS_PROFILE"));
    if let Some(value) = from_env.and_then(|e| e.get("value")) {
        return Some(value.clone());
    }
    let args = exec.args.as_deref().unwrap_or_default();
    args.iter().position(|a| a == "--profile").and_then(|i| args.get(i + 1)).cloned()
}

/// The metadata of `container`'s image in `pod`.
pub async fn image_metadata(client: &Client, namespace: &str, pod: &str, container: &str, env: &RegistryEnv) -> Result<ImageMetadata> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let data = serde_json::to_value(pods.get(pod).await?).unwrap_or_default();
    let (image, image_id) = container_image(&data, container).ok_or_else(|| Error::NotFound(format!("container {container} in pod {pod}")))?;
    let reference = pinned_reference(&image, image_id.as_deref())?;
    let platform = node_platform(client, data["spec"]["nodeName"].as_str()).await;
    let mut sources = pull_secret_sources(client, namespace, &data, reference.registry()).await;
    sources.extend(local_sources(env, reference.registry()));
    fetch(&reference, &platform, sources, env).await
}

/// The container's image and, once it started, the image ID it runs.
fn container_image(pod: &Value, name: &str) -> Option<(String, Option<String>)> {
    let found = ["initContainers", "containers", "ephemeralContainers"]
        .iter()
        .flat_map(|k| pod["spec"][k].as_array().into_iter().flatten())
        .find(|c| c["name"] == name)?;
    let image_id = ["initContainerStatuses", "containerStatuses", "ephemeralContainerStatuses"]
        .iter()
        .flat_map(|k| pod["status"][k].as_array().into_iter().flatten())
        .find(|s| s["name"] == name)
        .and_then(|s| s["imageID"].as_str())
        .filter(|id| !id.is_empty())
        .map(str::to_owned);
    Some((found["image"].as_str()?.to_owned(), image_id))
}

/// `image` pinned to the digest the container runs, if its image ID has one
/// (`docker.io/library/nginx@sha256:…`, `docker-pullable://nginx@sha256:…`).
/// A bare `sha256:…` is a local config ID, not a registry digest, so the
/// image's own tag or digest is used then.
pub(crate) fn pinned_reference(image: &str, image_id: Option<&str>) -> Result<Reference> {
    let spec: Reference = image.parse().map_err(|e| Error::Message(format!("can't read image name {image:?}: {e}")))?;
    let digest = image_id.and_then(|id| id.rsplit_once('@')).map(|(_, d)| d).filter(|d| d.contains(':'));
    Ok(match (spec.digest(), digest) {
        (None, Some(d)) => Reference::with_digest(spec.registry().to_owned(), spec.repository().to_owned(), d.to_owned()),
        _ => spec,
    })
}

/// The node's `(os, architecture)`, which picks the image from a
/// multi-platform index as the kubelet did. `linux/amd64` when unknown.
async fn node_platform(client: &Client, node: Option<&str>) -> (String, String) {
    let info = match node {
        Some(n) => Api::<Node>::all(client.clone()).get(n).await.ok().and_then(|n| n.status?.node_info),
        None => None,
    };
    info.map(|i| (i.operating_system, i.architecture)).unwrap_or_else(|| ("linux".into(), "amd64".into()))
}

/// Where to try credentials from, in order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Source {
    Auth { label: String, auth: RegistryAuth },
    /// `docker-credential-<helper> get`.
    Helper { helper: String, server: String },
    /// `aws ecr get-login-password`.
    Ecr { region: String },
    Anonymous,
}

impl Source {
    fn label(&self) -> String {
        match self {
            Self::Auth { label, .. } => label.clone(),
            Self::Helper { helper, .. } => format!("Docker credential helper {helper}"),
            Self::Ecr { .. } => "AWS CLI".into(),
            Self::Anonymous => "no credentials".into(),
        }
    }

    /// The credentials, `Ok(None)` when the source has none for this registry.
    async fn auth(&self, env: &RegistryEnv) -> std::result::Result<Option<RegistryAuth>, String> {
        match self {
            Self::Auth { auth, .. } => Ok(Some(auth.clone())),
            Self::Anonymous => Ok(Some(RegistryAuth::Anonymous)),
            Self::Helper { helper, server } => {
                let out = run_tool(env, &format!("docker-credential-{helper}"), &["get"], Some(server)).await;
                // Helpers fail with "credentials not found" for registries they don't know.
                Ok(out.ok().and_then(|o| helper_auth(&o)))
            }
            Self::Ecr { region } => {
                // The profile comes in as AWS_PROFILE.
                let password = run_tool(env, "aws", &["ecr", "get-login-password", "--region", region], None).await?;
                Ok(Some(RegistryAuth::Basic("AWS".into(), password)))
            }
        }
    }
}

/// Credentials from the pod's image pull secrets that name this registry.
async fn pull_secret_sources(client: &Client, namespace: &str, pod: &Value, registry: &str) -> Vec<Source> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), namespace);
    let mut found = vec![];
    for name in pod["spec"]["imagePullSecrets"].as_array().into_iter().flatten().filter_map(|s| s["name"].as_str()) {
        // A missing or unreadable secret is skipped; the kubelet would skip it too.
        let Ok(secret) = secrets.get(name).await else { continue };
        let data = secret.data.unwrap_or_default();
        let auths = match (data.get(".dockerconfigjson"), data.get(".dockercfg")) {
            (Some(json), _) => serde_json::from_slice::<Value>(&json.0).ok().map(|v| v["auths"].clone()),
            (None, Some(cfg)) => serde_json::from_slice::<Value>(&cfg.0).ok(),
            _ => None,
        };
        if let Some(auth) = auths.and_then(|a| config_auth(&a, registry)) {
            found.push(Source::Auth { label: format!("pull secret {name}"), auth });
        }
    }
    found
}

/// Credentials from this machine: the Docker config, the AWS CLI for ECR,
/// then none.
pub(crate) fn local_sources(env: &RegistryEnv, registry: &str) -> Vec<Source> {
    let mut found = vec![];
    let config = env.docker_config.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    if let Some(config) = config {
        if let Some(auth) = config_auth(&config["auths"], registry) {
            found.push(Source::Auth { label: "Docker credentials".into(), auth });
        }
        let helper = config["credHelpers"]
            .as_object()
            .and_then(|h| h.iter().find(|(k, _)| same_registry(k, registry)))
            .map(|(_, v)| v)
            .unwrap_or(&config["credsStore"]);
        if let Some(helper) = helper.as_str().filter(|h| !h.is_empty()) {
            let server = if normal_host(registry) == "docker.io" { "https://index.docker.io/v1/".to_owned() } else { registry.to_owned() };
            found.push(Source::Helper { helper: helper.to_owned(), server });
        }
    }
    if let Some(region) = ecr_region(registry) {
        found.push(Source::Ecr { region });
    }
    found.push(Source::Anonymous);
    found
}

/// The `auths` entry for `registry` in a Docker config: `auth` (base64
/// `user:password`) or `username` and `password`.
pub(crate) fn config_auth(auths: &Value, registry: &str) -> Option<RegistryAuth> {
    let (_, entry) = auths.as_object()?.iter().find(|(k, _)| same_registry(k, registry))?;
    if let Some(encoded) = entry["auth"].as_str().filter(|a| !a.is_empty()) {
        let decoded = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).ok()?;
        let (user, password) = String::from_utf8(decoded).ok()?.split_once(':').map(|(u, p)| (u.to_owned(), p.to_owned()))?;
        return Some(RegistryAuth::Basic(user, password));
    }
    match (entry["username"].as_str(), entry["password"].as_str()) {
        (Some(u), Some(p)) => Some(RegistryAuth::Basic(u.into(), p.into())),
        _ => None,
    }
}

/// A credential helper's answer, `{"Username": …, "Secret": …}`. A
/// `<token>` user is an identity token, which needs an OAuth exchange
/// this doesn't do.
fn helper_auth(out: &str) -> Option<RegistryAuth> {
    let v: Value = serde_json::from_str(out).ok()?;
    let (user, secret) = (v["Username"].as_str()?, v["Secret"].as_str()?);
    (user != "<token>" && !secret.is_empty()).then(|| RegistryAuth::Basic(user.into(), secret.into()))
}

/// A Docker config key (`https://index.docker.io/v1/`, `ghcr.io`, …) as a host.
fn normal_host(key: &str) -> &str {
    let host = key.trim_start_matches("https://").trim_start_matches("http://");
    let host = host.split('/').next().unwrap_or(host);
    match host {
        "index.docker.io" | "registry-1.docker.io" | "registry.hub.docker.com" => "docker.io",
        h => h,
    }
}

fn same_registry(key: &str, registry: &str) -> bool {
    normal_host(key).eq_ignore_ascii_case(normal_host(registry))
}

/// The region of a private ECR registry (`<account>.dkr.ecr.<region>.amazonaws.com`).
pub(crate) fn ecr_region(registry: &str) -> Option<String> {
    let parts: Vec<&str> = registry.split('.').collect();
    let i = parts.iter().position(|p| *p == "dkr")?;
    (parts.get(i + 1)?.starts_with("ecr") && parts.get(i + 3) == Some(&"amazonaws")).then(|| parts[i + 2].to_owned())
}

/// Runs a helper or the AWS CLI (no shell), with `input` on its stdin, and
/// returns its output; its error output is the error.
async fn run_tool(env: &RegistryEnv, program: &str, args: &[&str], input: Option<&str>) -> std::result::Result<String, String> {
    let mut cmd = tokio::process::Command::new(env.tools_dir.join(program));
    cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    // The context's AWS account for ECR helpers too; and no EC2 metadata
    // probe, which only times out (~10 s) off EC2.
    if let Some(profile) = &env.aws_profile {
        cmd.env("AWS_PROFILE", profile);
    }
    cmd.env("AWS_EC2_METADATA_DISABLED", "true");
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let run = async {
        let mut child = cmd.spawn()?;
        let mut stdin = child.stdin.take();
        if let (Some(stdin), Some(input)) = (stdin.as_mut(), input) {
            let _ = stdin.write_all(input.as_bytes()).await;
        }
        drop(stdin);
        child.wait_with_output().await
    };
    let out = match tokio::time::timeout(env.tool_timeout, run).await {
        Err(_) => return Err(format!("{program} didn't finish within {}s", env.tool_timeout.as_secs())),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => return Err(format!("{program} isn't installed or isn't on PATH")),
        Ok(Err(e)) => return Err(format!("couldn't run {program}: {e}")),
        Ok(Ok(out)) => out,
    };
    tracing::info!(%program, ok = out.status.success(), "registry credentials");
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        Err(if err.is_empty() { format!("{program} failed ({})", out.status) } else { err })
    }
}

/// Tries each source until the registry lets one read the image.
async fn fetch(reference: &Reference, platform: &(String, String), sources: Vec<Source>, env: &RegistryEnv) -> Result<ImageMetadata> {
    let mut refused = vec![];
    for source in sources {
        let auth = match source.auth(env).await {
            Ok(Some(auth)) => auth,
            Ok(None) => continue,
            Err(e) => {
                refused.push(format!("{} ({e})", source.label()));
                continue;
            }
        };
        match tokio::time::timeout(env.timeout, pull(reference, &auth, platform, env)).await {
            Err(_) => return Err(Error::Message(format!("{} didn't answer within {}s", reference.registry(), env.timeout.as_secs()))),
            Ok(Ok((manifest, digest, config, index))) => {
                tracing::info!(image = %reference.whole(), via = %source.label(), "image metadata read");
                return metadata(reference, &manifest, digest, &config, index, source.label());
            }
            Ok(Err(e)) if denied(&e) => refused.push(source.label()),
            Ok(Err(e)) => return Err(Error::Message(fetch_error(reference, platform, &e))),
        }
    }
    Err(Error::Message(format!(
        "{} refused to show {} with each of: {}. Add an image pull secret to the pod, or sign this machine in (docker login, or aws sso login for ECR).",
        reference.registry(),
        reference.repository(),
        refused.join(", ")
    )))
}

async fn pull(
    reference: &Reference,
    auth: &RegistryAuth,
    (os, arch): &(String, String),
    env: &RegistryEnv,
) -> std::result::Result<(OciImageManifest, String, String, Option<String>), OciDistributionError> {
    let (os, arch) = (os.clone(), arch.clone());
    let config = ClientConfig {
        protocol: if env.plain_http.is_empty() { ClientProtocol::Https } else { ClientProtocol::HttpsExcept(env.plain_http.clone()) },
        platform_resolver: Some(Box::new(move |entries: &[ImageIndexEntry]| pick_platform(entries, &os, &arch))),
        connect_timeout: Some(Duration::from_secs(10)),
        user_agent: concat!("subduer/", env!("CARGO_PKG_VERSION")),
        ..Default::default()
    };
    // reqwest uses rustls' process-wide provider: the same one (ring) kube
    // does. Installing fails harmlessly once one is in place.
    let _ = rustls::crypto::ring::default_provider().install_default();
    // A new client each time: it keeps the first credentials it's given.
    oci_client::Client::new(config).pull_manifest_and_config_and_list_digest(reference, auth).await
}

/// The index entry for the node's platform.
pub(crate) fn pick_platform(entries: &[ImageIndexEntry], os: &str, arch: &str) -> Option<String> {
    entries
        .iter()
        .find(|e| e.platform.as_ref().is_some_and(|p| p.os.to_string() == os && p.architecture.to_string() == arch))
        .map(|e| e.digest.clone())
}

/// The registry wants other credentials.
pub(crate) fn denied(e: &OciDistributionError) -> bool {
    match e {
        OciDistributionError::UnauthorizedError { .. } | OciDistributionError::AuthenticationFailure(_) => true,
        OciDistributionError::ServerError { code, .. } => matches!(code, 401 | 403),
        OciDistributionError::RegistryError { envelope, .. } => {
            envelope.errors.iter().any(|e| matches!(e.code, OciErrorCode::Unauthorized | OciErrorCode::Denied))
        }
        _ => false,
    }
}

fn fetch_error(reference: &Reference, (os, arch): &(String, String), e: &OciDistributionError) -> String {
    let codes = |want: &[OciErrorCode]| match e {
        OciDistributionError::RegistryError { envelope, .. } => envelope.errors.iter().any(|x| want.contains(&x.code)),
        _ => false,
    };
    let image = reference.whole();
    match e {
        OciDistributionError::ImageManifestNotFoundError(_) => format!("{image} isn't in the registry, or has no {os}/{arch} image"),
        _ if codes(&[OciErrorCode::ManifestUnknown, OciErrorCode::NameUnknown, OciErrorCode::NotFound]) => {
            format!("{image} isn't in the registry (it may have been deleted since the node pulled it)")
        }
        _ if codes(&[OciErrorCode::Toomanyrequests]) => format!("{} is limiting requests; try again later", reference.registry()),
        OciDistributionError::RequestError(err) => format!("couldn't reach {}: {err}", reference.registry()),
        OciDistributionError::GenericError(Some(m)) if m.contains("platform") => format!("{image} has no {os}/{arch} image"),
        other => format!("couldn't read {image}: {other}"),
    }
}

/// The metadata from a manifest and its config JSON.
pub(crate) fn metadata(
    reference: &Reference,
    manifest: &OciImageManifest,
    digest: String,
    config: &str,
    index_digest: Option<String>,
    credentials: String,
) -> Result<ImageMetadata> {
    let v: Value = serde_json::from_str(config).map_err(|e| Error::Message(format!("the image config isn't JSON: {e}")))?;
    let c = &v["config"];
    let string = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    let strings = |v: &Value| v.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_owned)).collect::<Vec<_>>();
    let keys = |v: &Value| v.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
    let labels: BTreeMap<String, String> =
        c["Labels"].as_object().into_iter().flatten().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned())).collect();
    let platform = [&v["os"], &v["architecture"], &v["variant"]].into_iter().filter_map(string).collect::<Vec<_>>().join("/");

    Ok(ImageMetadata {
        reference: reference.whole(),
        digest,
        index_digest,
        id: manifest.config.digest.clone(),
        platform,
        created: string(&v["created"]),
        author: string(&v["author"]),
        docker_version: string(&v["docker_version"]),
        size: manifest.layers.iter().map(|l| l.size.max(0) as u64).sum(),
        entrypoint: strings(&c["Entrypoint"]),
        cmd: strings(&c["Cmd"]),
        working_dir: string(&c["WorkingDir"]),
        user: string(&c["User"]),
        env: strings(&c["Env"]),
        exposed_ports: keys(&c["ExposedPorts"]),
        volumes: keys(&c["Volumes"]),
        stop_signal: string(&c["StopSignal"]),
        labels: labels.into_iter().collect(),
        history: history(&v["history"], manifest),
        credentials,
    })
}

/// The build steps, each layer-adding step given its layer's size. Without a
/// history, just the layers.
fn history(history: &Value, manifest: &OciImageManifest) -> Vec<ImageLayer> {
    let steps = history.as_array().map(Vec::as_slice).unwrap_or_default();
    if steps.is_empty() {
        return manifest
            .layers
            .iter()
            .map(|l| ImageLayer { created_by: l.digest.clone(), size: Some(l.size.max(0) as u64), ..Default::default() })
            .collect();
    }
    let mut layers = manifest.layers.iter();
    steps
        .iter()
        .map(|h| {
            let empty = h["empty_layer"].as_bool().unwrap_or(false);
            let text = |k: &str| h[k].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
            ImageLayer {
                created_by: tidy_step(&text("created_by").unwrap_or_default()),
                created: text("created"),
                size: if empty { None } else { layers.next().map(|l| l.size.max(0) as u64) },
                comment: text("comment"),
            }
        })
        .collect()
}

/// A history step as a Dockerfile line: Docker writes
/// `/bin/sh -c #(nop)  CMD ["nginx"]` for metadata-only steps.
fn tidy_step(step: &str) -> String {
    match step.split_once("#(nop)") {
        Some((_, rest)) => rest.trim().to_owned(),
        None => step.trim().to_owned(),
    }
}

#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
