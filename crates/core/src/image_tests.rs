//! Tests for image metadata.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::*;
use crate::testing::{MockApi, object, pod};
use serde_json::json;
use sha2::Digest as _;
use tokio::io::AsyncReadExt;

fn b64(s: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(s)
}

fn sha(bytes: &[u8]) -> String {
    format!("sha256:{}", sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>())
}

fn env() -> RegistryEnv {
    RegistryEnv { docker_config: None, tools_dir: PathBuf::new(), aws_profile: None, plain_http: vec![], timeout: Duration::from_secs(10) }
}

// ---- references and containers ----

#[test]
fn reference_is_pinned_to_the_digest_the_container_runs() {
    let digest = "sha256:aaaa";
    let r = pinned_reference("nginx:1.27", Some(&format!("docker.io/library/nginx@{digest}"))).unwrap();
    assert_eq!(r.whole(), format!("docker.io/library/nginx@{digest}"));
    // Docker's runtime prefix.
    let r = pinned_reference("ghcr.io/acme/app:2", Some(&format!("docker-pullable://ghcr.io/acme/app@{digest}"))).unwrap();
    assert_eq!(r.whole(), format!("ghcr.io/acme/app@{digest}"));
    // Repo digest under another name (mirror): the spec's repository is kept.
    let r = pinned_reference("acme/app:2", Some(&format!("mirror.local/acme/app@{digest}"))).unwrap();
    assert_eq!(r.whole(), format!("docker.io/acme/app@{digest}"));
}

#[test]
fn reference_falls_back_to_the_spec_image() {
    // Not started yet, or a bare local config ID.
    assert_eq!(pinned_reference("nginx", None).unwrap().whole(), "docker.io/library/nginx:latest");
    assert_eq!(pinned_reference("nginx:1", Some("sha256:bbbb")).unwrap().whole(), "docker.io/library/nginx:1");
    // A spec digest is what runs already.
    let (c, d) = (format!("sha256:{}", "c".repeat(64)), format!("sha256:{}", "d".repeat(64)));
    let r = pinned_reference(&format!("nginx@{c}"), Some(&format!("docker.io/library/nginx@{d}"))).unwrap();
    assert_eq!(r.digest(), Some(c.as_str()));
    assert!(pinned_reference("UPPER/Case:x", None).is_err());
}

#[test]
fn container_image_finds_every_kind_of_container() {
    let p = json!({
        "spec": {"initContainers": [{"name": "init", "image": "busybox"}],
                 "containers": [{"name": "app", "image": "acme/app:1"}],
                 "ephemeralContainers": [{"name": "debug", "image": "netshoot"}]},
        "status": {"containerStatuses": [{"name": "app", "imageID": "docker.io/acme/app@sha256:1"}],
                   "initContainerStatuses": [{"name": "init", "imageID": ""}]}
    });
    assert_eq!(container_image(&p, "app"), Some(("acme/app:1".into(), Some("docker.io/acme/app@sha256:1".into()))));
    assert_eq!(container_image(&p, "init"), Some(("busybox".into(), None)));
    assert_eq!(container_image(&p, "debug"), Some(("netshoot".into(), None)));
    assert_eq!(container_image(&p, "nope"), None);
}

// ---- credentials ----

#[test]
fn docker_config_auths_match_the_registry() {
    let auths = json!({
        "https://index.docker.io/v1/": {"auth": b64("hub:pw")},
        "ghcr.io": {"username": "gh", "password": "tok"},
        "quay.io": {"auth": ""},
    });
    assert_eq!(config_auth(&auths, "docker.io"), Some(RegistryAuth::Basic("hub".into(), "pw".into())));
    assert_eq!(config_auth(&auths, "GHCR.io"), Some(RegistryAuth::Basic("gh".into(), "tok".into())));
    // An empty entry (credentials kept by a helper) and unknown registries.
    assert_eq!(config_auth(&auths, "quay.io"), None);
    assert_eq!(config_auth(&auths, "example.com"), None);
    assert_eq!(config_auth(&json!({"x.io": {"auth": "not base64!"}}), "x.io"), None);
}

#[test]
fn ecr_regions() {
    assert_eq!(ecr_region("123456789012.dkr.ecr.us-east-1.amazonaws.com").as_deref(), Some("us-east-1"));
    assert_eq!(ecr_region("123456789012.dkr.ecr-fips.us-gov-west-1.amazonaws.com").as_deref(), Some("us-gov-west-1"));
    assert_eq!(ecr_region("123456789012.dkr.ecr.cn-north-1.amazonaws.com.cn").as_deref(), Some("cn-north-1"));
    assert_eq!(ecr_region("public.ecr.aws"), None);
    assert_eq!(ecr_region("docker.io"), None);
}

#[test]
fn local_sources_in_order() {
    let dir = std::env::temp_dir().join(format!("subduer-image-local-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");
    std::fs::write(&path, json!({
        "auths": {"ghcr.io": {"auth": b64("u:p")}},
        "credHelpers": {"123.dkr.ecr.eu-west-1.amazonaws.com": "ecr-login"},
        "credsStore": "desktop",
    }).to_string()).unwrap();
    let env = RegistryEnv { docker_config: Some(path), ..env() };

    assert_eq!(local_sources(&env, "ghcr.io"), vec![
        Source::Auth { label: "Docker credentials".into(), auth: RegistryAuth::Basic("u".into(), "p".into()) },
        Source::Helper { helper: "desktop".into(), server: "ghcr.io".into() },
        Source::Anonymous,
    ]);
    assert_eq!(local_sources(&env, "123.dkr.ecr.eu-west-1.amazonaws.com"), vec![
        Source::Helper { helper: "ecr-login".into(), server: "123.dkr.ecr.eu-west-1.amazonaws.com".into() },
        Source::Ecr { region: "eu-west-1".into() },
        Source::Anonymous,
    ]);
    assert_eq!(local_sources(&env, "docker.io")[0], Source::Helper { helper: "desktop".into(), server: "https://index.docker.io/v1/".into() });
    // Credentials but no helper.
    std::fs::write(dir.join("plain.json"), json!({"auths": {"ghcr.io": {"auth": b64("u:p")}}}).to_string()).unwrap();
    let plain = RegistryEnv { docker_config: Some(dir.join("plain.json")), ..self::env() };
    assert_eq!(local_sources(&plain, "ghcr.io").len(), 2);
    // No Docker config.
    assert_eq!(local_sources(&RegistryEnv { docker_config: Some(dir.join("missing.json")), ..self::env() }, "ghcr.io"), vec![Source::Anonymous]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn aws_profile_comes_from_the_contexts_exec() {
    let config: kube::config::Kubeconfig = serde_norway::from_str(r#"
contexts:
- name: by-env
  context: {cluster: c, user: u-env}
- name: by-arg
  context: {cluster: c, user: u-arg}
- name: plain
  context: {cluster: c, user: u-token}
users:
- name: u-env
  user:
    exec: {apiVersion: client.authentication.k8s.io/v1beta1, command: aws, args: [eks, get-token, --profile, ignored], env: [{name: AWS_PROFILE, value: uat}]}
- name: u-arg
  user:
    exec: {apiVersion: client.authentication.k8s.io/v1beta1, command: aws, args: [eks, get-token, --profile, prod]}
- name: u-token
  user: {token: abc}
"#).unwrap();
    assert_eq!(aws_profile(&config, "by-env").as_deref(), Some("uat"));
    assert_eq!(aws_profile(&config, "by-arg").as_deref(), Some("prod"));
    assert_eq!(aws_profile(&config, "plain"), None);
    assert_eq!(aws_profile(&config, "missing"), None);
}

#[test]
fn helper_answers() {
    assert_eq!(helper_auth(r#"{"ServerURL":"x","Username":"u","Secret":"s"}"#), Some(RegistryAuth::Basic("u".into(), "s".into())));
    assert_eq!(helper_auth(r#"{"Username":"<token>","Secret":"refresh"}"#), None);
    assert_eq!(helper_auth("credentials not found in native keychain"), None);
}

#[test]
fn denied_errors_move_on_to_the_next_credentials() {
    use oci_client::errors::{OciEnvelope, OciError};
    let envelope = |code| OciDistributionError::RegistryError {
        envelope: OciEnvelope { errors: vec![OciError { code, message: String::new(), detail: Value::Null }] },
        url: "u".into(),
    };
    assert!(denied(&OciDistributionError::UnauthorizedError { url: "u".into() }));
    assert!(denied(&envelope(OciErrorCode::Denied)));
    assert!(denied(&OciDistributionError::ServerError { code: 403, url: "u".into(), message: String::new() }));
    assert!(!denied(&envelope(OciErrorCode::ManifestUnknown)));
    assert!(!denied(&OciDistributionError::ServerError { code: 500, url: "u".into(), message: String::new() }));
    assert!(!denied(&OciDistributionError::GenericError(None)));
    let r = pinned_reference("acme/app:1", None).unwrap();
    let p = ("linux".to_owned(), "arm64".to_owned());
    let not_found = fetch_error(&r, &p, &OciDistributionError::ImageManifestNotFoundError("x".into()));
    assert_eq!(not_found, "docker.io/acme/app:1 isn't in the registry, or has no linux/arm64 image");
    let platform = fetch_error(&r, &p, &OciDistributionError::GenericError(Some("no matching platform".into())));
    assert_eq!(platform, "docker.io/acme/app:1 has no linux/arm64 image");
    let other = fetch_error(&r, &p, &OciDistributionError::GenericError(None));
    assert_eq!(other, "couldn't read docker.io/acme/app:1: Generic error: None");
    assert!(fetch_error(&r, &p, &envelope(OciErrorCode::Toomanyrequests)).contains("limiting requests"));
    assert!(fetch_error(&r, &p, &envelope(OciErrorCode::ManifestUnknown)).contains("isn't in the registry"));
}

// ---- metadata ----

fn sample_config() -> Value {
    json!({
        "architecture": "arm64", "os": "linux", "variant": "v8",
        "created": "2026-11-01T10:00:00Z", "author": "ACME", "docker_version": "27.0.1",
        "config": {
            "Entrypoint": ["/docker-entrypoint.sh"], "Cmd": ["nginx", "-g", "daemon off;"],
            "Env": ["PATH=/usr/bin", "NGINX_VERSION=1.27"], "WorkingDir": "/app", "User": "101",
            "ExposedPorts": {"80/tcp": {}, "443/tcp": {}}, "Volumes": {"/data": {}},
            "StopSignal": "SIGQUIT",
            "Labels": {"org.opencontainers.image.source": "https://github.com/acme/app", "maintainer": "ops"}
        },
        "history": [
            {"created": "2026-10-01T00:00:00Z", "created_by": "/bin/sh -c #(nop) ADD file:abc in / "},
            {"created": "2026-10-01T00:00:01Z", "created_by": "/bin/sh -c #(nop)  CMD [\"bash\"]", "empty_layer": true},
            {"created": "2026-11-01T10:00:00Z", "created_by": "RUN /bin/sh -c apt-get update # buildkit", "comment": "buildkit.dockerfile.v0"}
        ],
        "rootfs": {"type": "layers", "diff_ids": ["sha256:d1", "sha256:d2"]}
    })
}

fn manifest(config: &[u8], layers: &[(&str, i64)]) -> Value {
    json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {"mediaType": "application/vnd.oci.image.config.v1+json", "digest": sha(config), "size": config.len()},
        "layers": layers.iter().map(|(d, s)| json!({"mediaType": "application/vnd.oci.image.layer.v1.tar+gzip", "digest": d, "size": s})).collect::<Vec<_>>()
    })
}

#[test]
fn metadata_from_config() {
    let config = sample_config().to_string();
    let m: OciImageManifest = serde_json::from_value(manifest(config.as_bytes(), &[("sha256:l1", 3000), ("sha256:l2", 500)])).unwrap();
    let digest = format!("sha256:{}", "a".repeat(64));
    let r = pinned_reference(&format!("nginx@{digest}"), None).unwrap();
    let md = metadata(&r, &m, digest.clone(), &config, Some("sha256:idx".into()), "no credentials".into()).unwrap();
    assert_eq!(md.reference, format!("docker.io/library/nginx@{digest}"));
    assert_eq!(md.id, sha(config.as_bytes()));
    assert_eq!(md.index_digest.as_deref(), Some("sha256:idx"));
    assert_eq!(md.platform, "linux/arm64/v8");
    assert_eq!((md.created.as_deref(), md.author.as_deref(), md.docker_version.as_deref()), (Some("2026-11-01T10:00:00Z"), Some("ACME"), Some("27.0.1")));
    assert_eq!(md.size, 3500);
    assert_eq!(md.entrypoint, vec!["/docker-entrypoint.sh"]);
    assert_eq!(md.cmd, vec!["nginx", "-g", "daemon off;"]);
    assert_eq!((md.working_dir.as_deref(), md.user.as_deref(), md.stop_signal.as_deref()), (Some("/app"), Some("101"), Some("SIGQUIT")));
    assert_eq!(md.env, vec!["PATH=/usr/bin", "NGINX_VERSION=1.27"]);
    assert_eq!(md.exposed_ports, vec!["443/tcp", "80/tcp"]);
    assert_eq!(md.volumes, vec!["/data"]);
    assert_eq!(md.labels, vec![
        ("maintainer".into(), "ops".into()),
        ("org.opencontainers.image.source".into(), "https://github.com/acme/app".into()),
    ]);
    let steps: Vec<(&str, Option<u64>)> = md.history.iter().map(|h| (h.created_by.as_str(), h.size)).collect();
    assert_eq!(steps, vec![("ADD file:abc in /", Some(3000)), ("CMD [\"bash\"]", None), ("RUN /bin/sh -c apt-get update # buildkit", Some(500))]);
    assert_eq!(md.history[2].comment.as_deref(), Some("buildkit.dockerfile.v0"));
    assert_eq!(md.credentials, "no credentials");
}

#[test]
fn metadata_without_history_lists_layers() {
    let config = json!({"os": "linux", "architecture": "amd64", "config": {}}).to_string();
    let m: OciImageManifest = serde_json::from_value(manifest(config.as_bytes(), &[("sha256:l1", 10)])).unwrap();
    let md = metadata(&pinned_reference("x/y:1", None).unwrap(), &m, "sha256:m".into(), &config, None, String::new()).unwrap();
    assert_eq!(md.platform, "linux/amd64");
    assert_eq!(md.history, vec![ImageLayer { created_by: "sha256:l1".into(), size: Some(10), ..Default::default() }]);
    assert_eq!((md.created, md.user, md.env.len()), (None, None, 0));
    assert!(metadata(&pinned_reference("x/y:1", None).unwrap(), &m, String::new(), "not json", None, String::new()).is_err());
}

// ---- end to end, against a fake registry ----

/// A plain-HTTP registry serving fixed blobs and manifests, wanting Basic
/// `user:password` when given.
struct FakeRegistry {
    host: String,
    /// `<path> <authorization>` per request.
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakeRegistry {
    async fn start(login: Option<&str>, routes: HashMap<String, (String, Vec<u8>)>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = listener.local_addr().unwrap().to_string();
        let calls = Arc::new(Mutex::new(vec![]));
        let want = login.map(|l| format!("Basic {}", b64(l)));
        let (log, routes) = (calls.clone(), Arc::new(routes));
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let (log, routes, want) = (log.clone(), routes.clone(), want.clone());
                tokio::spawn(async move {
                    let mut buf = vec![];
                    let mut chunk = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let req = String::from_utf8_lossy(&buf).to_string();
                    let path = req.split_whitespace().nth(1).unwrap_or_default().split('?').next().unwrap_or_default().to_owned();
                    let auth = req
                        .lines()
                        .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("authorization")).map(|(_, v)| v.trim().to_owned()))
                        .unwrap_or_default();
                    log.lock().unwrap().push(format!("{path} {auth}"));
                    let allowed = want.as_ref().is_none_or(|w| *w == auth);
                    let (status, ctype, body): (&str, String, Vec<u8>) = if !allowed {
                        ("401 Unauthorized", "application/json".into(), br#"{"errors":[{"code":"UNAUTHORIZED","message":"login"}]}"#.to_vec())
                    } else if path == "/v2/" {
                        ("200 OK", "application/json".into(), b"{}".to_vec())
                    } else if let Some((ctype, body)) = routes.get(&path) {
                        ("200 OK", ctype.clone(), body.clone())
                    } else {
                        ("404 Not Found", "application/json".into(), br#"{"errors":[{"code":"MANIFEST_UNKNOWN","message":"unknown"}]}"#.to_vec())
                    };
                    let challenge = if want.is_some() { "WWW-Authenticate: Basic realm=\"fake\"\r\n" } else { "" };
                    let digest = format!("Docker-Content-Digest: {}\r\n", sha(&body));
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n{challenge}{digest}Connection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    if !req.starts_with("HEAD") {
                        let _ = sock.write_all(&body).await;
                    }
                });
            }
        });
        Self { host, calls }
    }

    fn env(&self) -> RegistryEnv {
        RegistryEnv { plain_http: vec![self.host.clone()], ..env() }
    }
}

/// An `acme/app` index with amd64 and arm64 images; returns the routes and
/// the index's digest.
fn multi_arch_image() -> (HashMap<String, (String, Vec<u8>)>, String) {
    let mut routes = HashMap::new();
    let mut entries = vec![];
    for arch in ["amd64", "arm64"] {
        let mut config = sample_config();
        config["architecture"] = json!(arch);
        config["config"]["Labels"]["arch"] = json!(arch);
        let config = config.to_string().into_bytes();
        let m = serde_json::to_vec(&manifest(&config, &[("sha256:l1", 3000), ("sha256:l2", 500)])).unwrap();
        routes.insert(format!("/v2/acme/app/blobs/{}", sha(&config)), ("application/octet-stream".into(), config));
        entries.push(json!({"mediaType": "application/vnd.oci.image.manifest.v1+json", "digest": sha(&m), "size": m.len(),
                            "platform": {"architecture": arch, "os": "linux"}}));
        routes.insert(format!("/v2/acme/app/manifests/{}", sha(&m)), ("application/vnd.oci.image.manifest.v1+json".into(), m));
    }
    let index = serde_json::to_vec(&json!({"schemaVersion": 2, "mediaType": "application/vnd.oci.image.index.v1+json", "manifests": entries})).unwrap();
    let digest = sha(&index);
    routes.insert(format!("/v2/acme/app/manifests/{digest}"), ("application/vnd.oci.image.index.v1+json".into(), index));
    (routes, digest)
}

fn cluster(host: &str, index: &str, pull_secret: Option<&str>) -> MockApi {
    let api = MockApi::new();
    let mut extra = json!({
        "spec": {"containers": [{"name": "app", "image": format!("{host}/acme/app:1")}]},
        "status": {"containerStatuses": [{"name": "app", "ready": true, "restartCount": 0, "state": {"running": {}},
                    "image": format!("{host}/acme/app:1"), "imageID": format!("{host}/acme/app@{index}")}]}
    });
    if let Some(login) = pull_secret {
        let config = json!({"auths": {host: {"auth": b64(login)}}}).to_string();
        api.on("GET", "/api/v1/namespaces/shop/secrets/regcred", object("v1", "Secret", Some("shop"), "regcred", json!({
            "type": "kubernetes.io/dockerconfigjson", "data": {".dockerconfigjson": b64(&config)}
        })));
        // Pull secrets for another registry (the older .dockercfg form), with
        // no Docker config, or missing are skipped.
        extra["spec"]["imagePullSecrets"] = json!([{"name": "other"}, {"name": "opaque"}, {"name": "gone"}, {"name": "regcred"}]);
        let other = json!({"ghcr.io": {"auth": b64("x:y")}}).to_string();
        api.on("GET", "/api/v1/namespaces/shop/secrets/other", object("v1", "Secret", Some("shop"), "other", json!({
            "type": "kubernetes.io/dockercfg", "data": {".dockercfg": b64(&other)}
        })));
        api.on("GET", "/api/v1/namespaces/shop/secrets/opaque", object("v1", "Secret", Some("shop"), "opaque", json!({"data": {"k": b64("v")}})));
    }
    api.on("GET", "/api/v1/namespaces/shop/pods/web", pod("shop", "web", "n1", extra));
    api.on("GET", "/api/v1/nodes/n1", object("v1", "Node", None, "n1", json!({
        "status": {"nodeInfo": {"operatingSystem": "linux", "architecture": "arm64", "kubeletVersion": "v1.33",
            "machineID": "", "systemUUID": "", "bootID": "", "kernelVersion": "", "osImage": "", "containerRuntimeVersion": "", "kubeProxyVersion": ""}}
    })));
    api
}

#[tokio::test]
async fn reads_the_nodes_platform_with_the_pull_secret() {
    let (routes, index) = multi_arch_image();
    let reg = FakeRegistry::start(Some("robot:s3cret"), routes).await;
    let api = cluster(&reg.host, &index, Some("robot:s3cret"));

    let md = image_metadata(&api.client(), "shop", "web", "app", &reg.env()).await.unwrap();
    assert_eq!(md.reference, format!("{}/acme/app@{index}", reg.host));
    assert_eq!(md.index_digest.as_deref(), Some(index.as_str()));
    assert_eq!(md.platform, "linux/arm64/v8");
    assert!(md.labels.contains(&("arch".into(), "arm64".into())));
    assert_eq!(md.credentials, "pull secret regcred");
    assert_eq!(md.size, 3500);
    // Only manifests and the config; never a layer.
    assert!(reg.calls.lock().unwrap().iter().all(|c| !c.contains("sha256:l")));
}

#[tokio::test]
async fn anonymous_works_for_public_images() {
    let (routes, index) = multi_arch_image();
    let reg = FakeRegistry::start(None, routes).await;
    let api = cluster(&reg.host, &index, None);
    let md = image_metadata(&api.client(), "shop", "web", "app", &reg.env()).await.unwrap();
    assert_eq!(md.credentials, "no credentials");
}

#[tokio::test]
async fn says_which_credentials_were_refused() {
    let (routes, index) = multi_arch_image();
    let reg = FakeRegistry::start(Some("robot:s3cret"), routes).await;
    let api = cluster(&reg.host, &index, Some("robot:wrong"));
    let err = image_metadata(&api.client(), "shop", "web", "app", &reg.env()).await.unwrap_err().to_string();
    assert!(err.contains("refused to show acme/app with each of: pull secret regcred, no credentials"), "{err}");
}

#[tokio::test]
async fn deleted_images_and_unknown_containers() {
    let (_, index) = multi_arch_image();
    let reg = FakeRegistry::start(None, HashMap::new()).await;
    let api = cluster(&reg.host, &index, None);
    let err = image_metadata(&api.client(), "shop", "web", "app", &reg.env()).await.unwrap_err().to_string();
    assert!(err.contains("isn't in the registry"), "{err}");
    let err = image_metadata(&api.client(), "shop", "web", "nope", &reg.env()).await.unwrap_err().to_string();
    assert!(err.contains("container nope in pod web"), "{err}");
}

#[cfg(unix)]
#[tokio::test]
async fn docker_credential_helpers_and_the_aws_cli() {
    use std::os::unix::fs::PermissionsExt;
    let (routes, index) = multi_arch_image();
    let reg = FakeRegistry::start(Some("helper-user:helper-pw"), routes).await;
    let dir = std::env::temp_dir().join(format!("subduer-image-tools-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = |name: &str, body: &str| {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    // The helper reads the server from stdin.
    script("docker-credential-fake", &format!(
        "read server; [ \"$1\" = get ] && [ \"$server\" = \"{}\" ] && echo '{{\"Username\":\"helper-user\",\"Secret\":\"helper-pw\"}}'", reg.host));
    script("aws", "echo \"$* $AWS_PROFILE $AWS_EC2_METADATA_DISABLED\" > \"$(dirname \"$0\")/aws.args\"; echo ecr-password");
    let config = dir.join("config.json");
    std::fs::write(&config, json!({"credsStore": "fake"}).to_string()).unwrap();
    let env = RegistryEnv { docker_config: Some(config), tools_dir: dir.clone(), aws_profile: Some("uat".into()), ..reg.env() };

    let api = cluster(&reg.host, &index, None);
    let md = image_metadata(&api.client(), "shop", "web", "app", &env).await.unwrap();
    assert_eq!(md.credentials, "Docker credential helper fake");

    // The AWS CLI gives ECR's password, with the context's profile.
    let auth = Source::Ecr { region: "eu-west-1".into() }.auth(&env).await.unwrap();
    assert_eq!(auth, Some(RegistryAuth::Basic("AWS".into(), "ecr-password".into())));
    assert_eq!(std::fs::read_to_string(dir.join("aws.args")).unwrap().trim(), "ecr get-login-password --region eu-west-1 uat true");
    // A missing helper has nothing to offer.
    let none = Source::Helper { helper: "missing".into(), server: "x".into() }.auth(&env).await.unwrap();
    assert_eq!(none, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pick_platform_matches_os_and_arch() {
    let entries: Vec<ImageIndexEntry> = serde_json::from_value(json!([
        {"mediaType": "m", "digest": "sha256:amd", "size": 1, "platform": {"architecture": "amd64", "os": "linux"}},
        {"mediaType": "m", "digest": "sha256:arm", "size": 1, "platform": {"architecture": "arm64", "os": "linux", "variant": "v8"}},
        {"mediaType": "m", "digest": "sha256:att", "size": 1, "platform": {"architecture": "unknown", "os": "unknown"}}
    ])).unwrap();
    assert_eq!(pick_platform(&entries, "linux", "arm64").as_deref(), Some("sha256:arm"));
    assert_eq!(pick_platform(&entries, "linux", "amd64").as_deref(), Some("sha256:amd"));
    assert_eq!(pick_platform(&entries, "windows", "amd64"), None);
}

#[test]
fn system_env_reads_the_docker_config_from_home() {
    let env = RegistryEnv::system(Some("uat".into()));
    assert!(env.docker_config.unwrap().ends_with("config.json"));
    assert_eq!((env.aws_profile.as_deref(), env.tools_dir, env.timeout), (Some("uat"), PathBuf::new(), FETCH_TIMEOUT));
    assert!(env.plain_http.is_empty());
    let labels: Vec<String> = [Source::Ecr { region: "r".into() }, Source::Anonymous, Source::Helper { helper: "h".into(), server: "s".into() }]
        .iter()
        .map(Source::label)
        .collect();
    assert_eq!(labels, ["AWS CLI", "no credentials", "Docker credential helper h"]);
}

#[tokio::test]
async fn pending_pods_use_the_tag_and_amd64() {
    let (mut routes, index) = multi_arch_image();
    let index_body = routes[&format!("/v2/acme/app/manifests/{index}")].clone();
    routes.insert("/v2/acme/app/manifests/1".into(), index_body);
    let reg = FakeRegistry::start(None, routes).await;
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/pods/web", object("v1", "Pod", Some("shop"), "web", json!({
        "spec": {"containers": [{"name": "app", "image": format!("{}/acme/app:1", reg.host)}]},
        "status": {"phase": "Pending"}
    })));
    let md = image_metadata(&api.client(), "shop", "web", "app", &reg.env()).await.unwrap();
    assert_eq!(md.reference, format!("{}/acme/app:1", reg.host));
    assert_eq!(md.platform, "linux/amd64/v8");
}

#[tokio::test]
async fn unreachable_and_silent_registries() {
    let r = |host: &str| pinned_reference(&format!("{host}/acme/app:1"), None).unwrap();
    let p = ("linux".to_owned(), "amd64".to_owned());
    // Nothing listening.
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().to_string();
    let env = RegistryEnv { plain_http: vec![closed.clone()], ..env() };
    let err = fetch(&r(&closed), &p, vec![Source::Anonymous], &env).await.unwrap_err().to_string();
    assert!(err.starts_with(&format!("couldn't reach {closed}")), "{err}");
    // Listening but never answering.
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = silent.local_addr().unwrap().to_string();
    let env = RegistryEnv { plain_http: vec![host.clone()], timeout: Duration::from_millis(300), ..self::env() };
    let err = fetch(&r(&host), &p, vec![Source::Anonymous], &env).await.unwrap_err().to_string();
    assert_eq!(err, format!("{host} didn't answer within 0s"));
    drop(silent);
}

#[cfg(unix)]
#[tokio::test]
async fn failing_tools_are_reported_and_skipped() {
    use std::os::unix::fs::PermissionsExt;
    let (routes, _) = multi_arch_image();
    let reg = FakeRegistry::start(None, routes).await;
    let dir = std::env::temp_dir().join(format!("subduer-image-failing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let env = RegistryEnv { tools_dir: dir.clone(), ..reg.env() };
    let aws = dir.join("aws");
    let write = |body: &str, mode: u32| {
        std::fs::write(&aws, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&aws, std::fs::Permissions::from_mode(mode)).unwrap();
    };
    let ecr = Source::Ecr { region: "us-east-1".into() };

    write("echo 'Token has expired and refresh failed' >&2; exit 255", 0o755);
    assert_eq!(ecr.auth(&env).await, Err("Token has expired and refresh failed".into()));
    write("exit 3", 0o755);
    assert_eq!(ecr.auth(&env).await, Err("aws failed (exit status: 3)".into()));
    write("", 0o644);
    assert!(ecr.auth(&env).await.unwrap_err().starts_with("couldn't run aws: "));
    assert_eq!(Source::Ecr { region: "x".into() }.auth(&RegistryEnv { tools_dir: dir.join("none"), ..env.clone() }).await,
               Err("aws isn't installed or isn't on PATH".into()));

    // A failing source is named; a helper with nothing is passed over.
    let r = pinned_reference(&format!("{}/acme/app:1", reg.host), None).unwrap();
    let p = ("linux".to_owned(), "amd64".to_owned());
    let sources = vec![ecr, Source::Helper { helper: "missing".into(), server: reg.host.clone() }];
    let err = fetch(&r, &p, sources, &env).await.unwrap_err().to_string();
    assert!(err.contains("with each of: AWS CLI (couldn't run aws: "), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
