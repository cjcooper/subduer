use super::*;

use kube::config::Kubeconfig;

const PEM: &str = "-----BEGIN CERTIFICATE-----\nMIIBfake\n-----END CERTIFICATE-----";

fn new_cluster(name: &str) -> NewCluster {
    NewCluster {
        name: name.into(),
        server: "https://10.0.0.1:6443/".into(),
        certificate_authority: String::new(),
        insecure_skip_tls_verify: false,
        proxy_url: String::new(),
        auth: ClusterAuth::Token { token: "not-a-real-token".into() },
        namespace: String::new(),
        make_current: false,
    }
}

/// A scratch folder that's removed afterwards.
struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Self {
        crate::testing::record_all_traces();
        let dir = std::env::temp_dir().join(format!("subduer-add-cluster-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn config(&self) -> PathBuf {
        self.0.join("config")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn destination_follows_client_go() {
    let home = Path::new("/home/me");
    assert_eq!(destination(None, Some(home), |_| false).unwrap(), home.join(".kube/config"));
    let one = std::env::join_paths(["/a/one"]).unwrap();
    assert_eq!(destination(Some(&one), Some(home), |_| false).unwrap(), Path::new("/a/one"));
    // Several: the first that exists, else the last.
    let many = std::env::join_paths(["/a/one", "", "/a/two", "/a/three"]).unwrap();
    assert_eq!(destination(Some(&many), Some(home), |p| p == Path::new("/a/two")).unwrap(), Path::new("/a/two"));
    assert_eq!(destination(Some(&many), Some(home), |_| false).unwrap(), Path::new("/a/three"));
    // Set but empty counts as unset.
    assert_eq!(destination(Some(std::ffi::OsStr::new("")), Some(home), |_| false).unwrap(), home.join(".kube/config"));
    assert!(destination(None, None, |_| false).is_err());
}

#[test]
fn creates_a_kubeconfig_kubectl_can_read() {
    let dir = Scratch::new("create");
    let path = dir.0.join("nested/config");
    let mut new = new_cluster("lab");
    new.certificate_authority = PEM.into();
    new.namespace = " shop ".into();
    new.make_current = true;
    assert_eq!(add_cluster(&path, &new, &TakenNames::default()).unwrap(), "lab");

    let config = Kubeconfig::read_from(&path).unwrap();
    assert_eq!(config.current_context.as_deref(), Some("lab"));
    let cluster = config.clusters[0].cluster.as_ref().unwrap();
    assert_eq!(config.clusters[0].name, "lab");
    assert_eq!(cluster.server.as_deref(), Some("https://10.0.0.1:6443"));
    let ca = cluster.certificate_authority_data.as_deref().unwrap();
    assert_eq!(STANDARD.decode(ca).unwrap(), format!("{PEM}\n").into_bytes());
    let context = config.contexts[0].context.as_ref().unwrap();
    assert_eq!((context.cluster.as_str(), context.user.as_deref(), context.namespace.as_deref()), ("lab", Some("lab"), Some("shop")));
    let user = config.auth_infos[0].auth_info.as_ref().unwrap();
    assert!(user.token.is_some());
    assert!(!lock_left(&path));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

fn lock_left(path: &Path) -> bool {
    PathBuf::from(format!("{}.lock", path.display())).exists()
}

#[test]
fn appends_and_keeps_what_it_does_not_know() {
    let dir = Scratch::new("append");
    let path = dir.config();
    fs::write(
        &path,
        "apiVersion: v1\nkind: Config\ncurrent-context: old\nclusters:\n- name: old\n  cluster:\n    server: https://old\n    x-custom: kept\ncontexts:\n- name: old\n  context:\n    cluster: old\nusers: []\nextensions:\n- name: tool\n  extension: {a: 1}\n",
    )
    .unwrap();
    let mut new = new_cluster("lab");
    new.auth = ClusterAuth::Exec { command: "kubelogin".into(), args: vec!["get-token".into(), " ".into(), "--login=azurecli".into()] };
    new.insecure_skip_tls_verify = true;
    new.proxy_url = "socks5://localhost:1080".into();
    add_cluster(&path, &new, &TakenNames::default()).unwrap();

    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("x-custom: kept"), "{text}");
    assert!(text.contains("extensions:"), "{text}");
    assert!(text.contains("current-context: old"), "{text}");
    let config = Kubeconfig::read_from(&path).unwrap();
    assert_eq!(config.contexts.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["old", "lab"]);
    let cluster = config.clusters[1].cluster.as_ref().unwrap();
    assert_eq!(cluster.insecure_skip_tls_verify, Some(true));
    assert_eq!(cluster.proxy_url.as_deref(), Some("socks5://localhost:1080"));
    let exec = config.auth_infos[0].auth_info.as_ref().unwrap().exec.as_ref().unwrap();
    assert_eq!(exec.command.as_deref(), Some("kubelogin"));
    assert_eq!(exec.args.as_deref(), Some(&["get-token".to_owned(), "--login=azurecli".to_owned()][..]));
    assert_eq!(exec.api_version.as_deref(), Some("client.authentication.k8s.io/v1beta1"));
}

#[test]
fn files_are_referenced_and_pem_embedded() {
    let dir = Scratch::new("files");
    let cert = dir.0.join("client.crt");
    fs::write(&cert, PEM).unwrap();
    let mut new = new_cluster("lab");
    new.auth = ClusterAuth::ClientCertificate { certificate: cert.display().to_string(), key: "-----BEGIN PRIVATE KEY-----\nx\n-----END PRIVATE KEY-----".into() };
    add_cluster(&dir.config(), &new, &TakenNames::default()).unwrap();
    let config = Kubeconfig::read_from(dir.config()).unwrap();
    let user = config.auth_infos[0].auth_info.as_ref().unwrap();
    assert_eq!(user.client_certificate.as_deref(), Some(cert.display().to_string().as_str()));
    assert!(user.client_key_data.is_some());

    let mut missing = new_cluster("other");
    missing.certificate_authority = dir.0.join("nope.crt").display().to_string();
    assert!(add_cluster(&dir.config(), &missing, &TakenNames::default()).unwrap_err().to_string().contains("no such file"));
    missing.certificate_authority = "relative.crt".into();
    assert!(add_cluster(&dir.config(), &missing, &TakenNames::default()).unwrap_err().to_string().contains("paste the PEM"));
}

#[test]
fn no_auth_adds_no_user() {
    let dir = Scratch::new("noauth");
    let mut new = new_cluster("lab");
    new.auth = ClusterAuth::None;
    add_cluster(&dir.config(), &new, &TakenNames::default()).unwrap();
    let config = Kubeconfig::read_from(dir.config()).unwrap();
    assert!(config.auth_infos.is_empty());
    assert_eq!(config.contexts[0].context.as_ref().unwrap().user, None);
}

#[test]
fn refuses_names_in_use_and_bad_input() {
    let dir = Scratch::new("clash");
    let taken = TakenNames { contexts: vec!["a".into()], clusters: vec!["b".into()], users: vec!["c".into()] };
    for (name, message) in [("a", "context named"), ("b", "cluster named"), ("c", "user named")] {
        let err = add_cluster(&dir.config(), &new_cluster(name), &taken).unwrap_err().to_string();
        assert!(err.contains(message), "{name}: {err}");
    }
    // A user name clash doesn't matter when no user is added.
    let mut none = new_cluster("c");
    none.auth = ClusterAuth::None;
    assert!(add_cluster(&dir.config(), &none, &taken).is_ok());
    let mut bad = new_cluster("d");
    bad.server = "10.0.0.1".into();
    assert!(add_cluster(&dir.config(), &bad, &taken).is_err());
    assert!(!lock_left(&dir.config()));
}

#[test]
fn refuses_while_another_program_holds_the_lock() {
    let dir = Scratch::new("lock");
    let lock = dir.0.join("config.lock");
    fs::write(&lock, "").unwrap();
    let err = add_cluster(&dir.config(), &new_cluster("lab"), &TakenNames::default()).unwrap_err().to_string();
    assert!(err.contains("another program"), "{err}");
    // Someone else's lock stays.
    assert!(lock.exists());
    assert!(!dir.config().exists());
}

#[test]
fn current_context_and_rename() {
    let dir = Scratch::new("rename");
    let path = dir.config();
    assert_eq!(current_context(&path).unwrap(), None);
    let mut new = new_cluster("gke_proj_us-central1_web");
    new.make_current = true;
    add_cluster(&path, &new, &TakenNames::default()).unwrap();
    rename_context(&path, "gke_proj_us-central1_web", "web").unwrap();
    assert_eq!(current_context(&path).unwrap().as_deref(), Some("web"));
    let config = Kubeconfig::read_from(&path).unwrap();
    assert_eq!(config.contexts[0].name, "web");
    // The cluster entry keeps its name, as `kubectl config rename-context` does.
    assert_eq!(config.contexts[0].context.as_ref().unwrap().cluster, "gke_proj_us-central1_web");
    assert!(rename_context(&path, "nope", "x").is_err());

    set_current_context(&path, None).unwrap();
    assert_eq!(current_context(&path).unwrap(), None);
    set_current_context(&path, Some("web")).unwrap();
    assert_eq!(current_context(&path).unwrap().as_deref(), Some("web"));
}

#[cfg(unix)]
#[test]
fn writes_through_a_symlink() {
    let dir = Scratch::new("symlink");
    let real = dir.0.join("dotfiles-config");
    fs::write(&real, "apiVersion: v1\nkind: Config\n").unwrap();
    std::os::unix::fs::symlink(&real, dir.config()).unwrap();
    add_cluster(&dir.config(), &new_cluster("lab"), &TakenNames::default()).unwrap();
    assert!(fs::symlink_metadata(dir.config()).unwrap().file_type().is_symlink());
    assert!(fs::read_to_string(&real).unwrap().contains("name: lab"));
}

#[test]
fn refuses_a_file_that_is_not_a_kubeconfig() {
    let dir = Scratch::new("notconfig");
    fs::write(dir.config(), "- just\n- a list\n").unwrap();
    assert!(add_cluster(&dir.config(), &new_cluster("lab"), &TakenNames::default()).unwrap_err().to_string().contains("isn't a kubeconfig"));
}

#[test]
fn the_process_environment_is_what_the_free_functions_use() {
    let env = KubeconfigEnv::current();
    assert_eq!(env.kubeconfig, std::env::var_os("KUBECONFIG"));
    assert_eq!(env.home, std::env::home_dir());
    assert_eq!(kubeconfig_path().ok(), env.path().ok());
    assert_eq!(kubeconfig_files().ok(), env.files().ok());
    // Read-only: whatever this machine has, both ways agree.
    let (a, b) = (TakenNames::read(), TakenNames::read_in(&env));
    assert_eq!(a.is_ok(), b.is_ok());
    if let (Ok(a), Ok(b)) = (a, b) {
        assert_eq!((a.contexts, a.clusters, a.users), (b.contexts, b.clusters, b.users));
    }
}

#[test]
fn files_and_names_follow_the_environment() {
    let dir = Scratch::new("env");
    let home = KubeconfigEnv { kubeconfig: None, home: Some(dir.0.clone()) };
    assert_eq!(home.files().unwrap(), [dir.0.join(".kube/config")]);
    // No kubeconfig yet: nothing is taken.
    let none = TakenNames::read_in(&home).unwrap();
    assert!(none.contexts.is_empty() && none.clusters.is_empty() && none.users.is_empty());
    assert!(TakenNames::read_in(&KubeconfigEnv::default()).is_err());

    let (one, two) = (dir.0.join("one"), dir.0.join("two"));
    add_cluster(&one, &new_cluster("a"), &TakenNames::default()).unwrap();
    let mut b = new_cluster("b");
    b.auth = ClusterAuth::None;
    add_cluster(&two, &b, &TakenNames::default()).unwrap();
    add_cluster(&two, &new_cluster("a"), &TakenNames::default()).unwrap();
    let both = KubeconfigEnv { kubeconfig: Some(std::env::join_paths([&one, &two]).unwrap()), home: None };
    assert_eq!(both.files().unwrap(), [one.clone(), two.clone()]);
    // Merged like kubectl: the first file wins a name both have.
    let taken = TakenNames::read_in(&both).unwrap();
    assert_eq!(taken.contexts, ["a", "b"]);
    assert_eq!(taken.clusters, ["a", "b"]);
    assert_eq!(taken.users, ["a"]);
    // A file in $KUBECONFIG that isn't there is an error, as in kubectl.
    let missing = KubeconfigEnv { kubeconfig: Some(dir.0.join("missing").into_os_string()), home: None };
    assert!(TakenNames::read_in(&missing).is_err());
    // An empty $KUBECONFIG falls back to the home folder.
    let empty = KubeconfigEnv { kubeconfig: Some("".into()), home: Some(dir.0.clone()) };
    assert_eq!(empty.files().unwrap(), [dir.0.join(".kube/config")]);
    assert!(TakenNames::read_in(&empty).is_err());
}

#[test]
fn the_root_folder_is_not_a_kubeconfig() {
    // No parent folder to make; then it's a folder, not a file.
    let err = add_cluster(Path::new("/"), &new_cluster("lab"), &TakenNames::default()).unwrap_err().to_string();
    assert!(err.starts_with('/'), "{err}");
}

#[test]
fn a_user_entry_for_no_auth_is_empty() {
    assert!(user_entry(&ClusterAuth::None).unwrap().is_empty());
}

#[test]
fn reads_empty_and_null_files_as_none() {
    let dir = Scratch::new("empty");
    fs::write(dir.config(), "  \n").unwrap();
    assert_eq!(read(&dir.config()).unwrap(), None);
    fs::write(dir.config(), "~\n").unwrap();
    assert_eq!(read(&dir.config()).unwrap(), None);
    // A folder where the file should be can't be read.
    let folder = dir.0.join("folder");
    fs::create_dir(&folder).unwrap();
    assert!(read(&folder).unwrap_err().to_string().starts_with(&folder.display().to_string()));
}

#[test]
fn null_lists_are_replaced_and_other_values_refused() {
    let dir = Scratch::new("lists");
    fs::write(dir.config(), "apiVersion: v1\nclusters:\nusers: ~\ncontexts:\n").unwrap();
    add_cluster(&dir.config(), &new_cluster("lab"), &TakenNames::default()).unwrap();
    let config = Kubeconfig::read_from(dir.config()).unwrap();
    assert_eq!((config.clusters.len(), config.auth_infos.len(), config.contexts.len()), (1, 1, 1));
    fs::write(dir.config(), "apiVersion: v1\nclusters: 5\n").unwrap();
    let err = add_cluster(&dir.config(), &new_cluster("lab2"), &TakenNames::default()).unwrap_err().to_string();
    assert_eq!(err, "the kubeconfig's clusters isn't a list");
    assert!(!lock_left(&dir.config()));
}

#[test]
fn reports_folders_it_cannot_make_or_write() {
    let dir = Scratch::new("unwritable");
    // The parent "folder" is a file.
    let blocked = dir.0.join("file");
    fs::write(&blocked, "").unwrap();
    let err = add_cluster(&blocked.join("config"), &new_cluster("lab"), &TakenNames::default()).unwrap_err().to_string();
    assert!(err.starts_with(&blocked.display().to_string()), "{err}");

    // The temporary file's name is taken by a folder: nothing is written,
    // and the lock is released.
    fs::create_dir(dir.0.join(format!(".config.subduer-{}", std::process::id()))).unwrap();
    let err = add_cluster(&dir.config(), &new_cluster("lab"), &TakenNames::default()).unwrap_err().to_string();
    assert!(err.starts_with(&dir.config().display().to_string()), "{err}");
    assert!(!dir.config().exists());
    assert!(!lock_left(&dir.config()));
}

#[cfg(unix)]
#[test]
fn reports_a_lock_it_cannot_create() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Scratch::new("readonly");
    let ro = dir.0.join("ro");
    fs::create_dir(&ro).unwrap();
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
    let result = add_cluster(&ro.join("config"), &new_cluster("lab"), &TakenNames::default());
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
    let err = result.unwrap_err().to_string();
    assert!(err.starts_with(&format!("{}.lock", ro.join("config").display())), "{err}");
}
