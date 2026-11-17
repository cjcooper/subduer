use super::*;
use subduer_types::ImageLayer;

fn meta() -> ImageMetadata {
    ImageMetadata {
        reference: "ghcr.io/acme/app@sha256:aa".into(),
        digest: "sha256:bb".into(),
        index_digest: Some("sha256:aa".into()),
        id: "sha256:cc".into(),
        platform: "linux/arm64/v8".into(),
        created: Some("2026-11-01T10:00:00Z".into()),
        author: None,
        docker_version: None,
        size: 3 * 1024 * 1024,
        entrypoint: vec!["/entrypoint.sh".into()],
        cmd: vec!["nginx".into(), "-g".into(), "daemon off;".into()],
        working_dir: Some("/app".into()),
        user: None,
        env: vec!["PATH=/usr/bin".into(), "EMPTY=".into(), "BARE".into()],
        exposed_ports: vec!["443/tcp".into(), "80/tcp".into()],
        volumes: vec![],
        stop_signal: None,
        labels: vec![("maintainer".into(), "ops".into())],
        history: vec![
            ImageLayer { created_by: "ADD file:abc in /".into(), size: Some(2048), ..Default::default() },
            ImageLayer { created_by: "CMD [\"bash\"]".into(), comment: Some("buildkit".into()), ..Default::default() },
        ],
        credentials: "pull secret regcred".into(),
    }
}

#[test]
fn args_are_the_command_arguments() {
    let args = ImageArgs { namespace: "shop".into(), pod: "web-1".into(), container: "app".into() };
    assert_eq!(serde_json::to_value(&args).unwrap(), serde_json::json!({"namespace": "shop", "pod": "web-1", "container": "app"}));
}

#[test]
fn loads_once_when_open() {
    assert!(needs_load(true, None));
    assert!(!needs_load(false, None));
    assert!(!needs_load(true, Some(&None)));
    assert!(!needs_load(true, Some(&Some(Err("x".into())))));
}

#[test]
fn summaries() {
    assert_eq!(summary(None), "read from the registry when opened");
    assert_eq!(summary(Some(&None)), "reading from the registry…");
    assert_eq!(summary(Some(&Some(Err("denied".into())))), "couldn't read");
    assert_eq!(summary(Some(&Some(Ok(meta())))), "linux/arm64/v8 · 3.0 MiB · built 2026-11-01");
    let undated = ImageMetadata { created: None, ..meta() };
    assert_eq!(summary(Some(&Some(Ok(undated)))), "linux/arm64/v8 · 3.0 MiB");
    let short = ImageMetadata { created: Some("2026".into()), ..meta() };
    assert_eq!(summary(Some(&Some(Ok(short)))), "linux/arm64/v8 · 3.0 MiB · built 2026");
    assert_eq!(env_summary(&meta()), "3 variables");
    assert_eq!(labels_summary(&meta()), "1 label");
    assert_eq!(history_summary(&meta()), "2 steps · 1 layer");
}

#[test]
fn rows_skip_what_is_empty() {
    let rows = rows(&meta());
    let keys: Vec<&str> = rows.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, [
        "Reference", "Platform", "Size", "Image ID", "Manifest digest", "Index digest", "Entrypoint", "Command",
        "Working dir", "User", "Exposed ports", "Read with",
    ]);
    let get = |k: &str| rows.iter().find(|(key, _)| *key == k).unwrap().1.clone();
    assert_eq!(get("Size"), "3.0 MiB compressed");
    assert_eq!(get("Command"), r#"["nginx", "-g", "daemon off;"]"#);
    assert_eq!(get("User"), "root (not set)");
    assert_eq!(get("Exposed ports"), "443/tcp, 80/tcp");
    let set = ImageMetadata { user: Some("101".into()), author: Some("ACME".into()), ..meta() };
    assert!(super::rows(&set).contains(&("User", "101".into())));
    assert!(super::rows(&set).contains(&("Author", "ACME".into())));
}

#[test]
fn commands_quote_like_a_dockerfile() {
    assert_eq!(command(&[]), "");
    assert_eq!(command(&["sh".into(), "-c".into(), "echo \"hi\"".into()]), r#"["sh", "-c", "echo \"hi\""]"#);
}

#[test]
fn env_and_history_rows() {
    assert_eq!(env_pair("PATH=/usr/bin:/bin"), ("PATH".into(), "/usr/bin:/bin".into()));
    assert_eq!(env_pair("A=b=c"), ("A".into(), "b=c".into()));
    assert_eq!(env_pair("BARE"), ("BARE".into(), String::new()));
    assert_eq!(history_rows(&meta()), vec![
        ("2.0 KiB".to_owned(), "ADD file:abc in /".to_owned(), None),
        (String::new(), "CMD [\"bash\"]".to_owned(), Some("buildkit".to_owned())),
    ]);
}
