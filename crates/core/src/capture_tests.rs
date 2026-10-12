use super::*;
use crate::testing::{MockApi, pod};

const PATCH_PATH: &str = "/api/v1/namespaces/shop/pods/web-1/ephemeralcontainers";
const IMAGE: &str = "123456789012.dkr.ecr.us-east-1.amazonaws.com/netshoot:latest";

#[test]
fn filters_and_the_command() {
    let f = |traffic: Traffic, handshakes_only: bool| Filter { traffic, handshakes_only };
    let hs = "tcp[((tcp[12:1] & 0xf0) >> 2):1] = 0x16";
    let ips = ["10.0.1.9".to_owned()];
    assert_eq!(expression(&f(Traffic::All, false), &ips), "tcp");
    assert_eq!(expression(&f(Traffic::All, true), &ips), format!("tcp and {hs}"));
    // Inbound: to the pod's own address on that port, and its answers from it.
    let inbound = "tcp and ((dst port 9443 and (dst host 10.0.1.9)) or (src port 9443 and (src host 10.0.1.9)))";
    assert_eq!(expression(&f(Traffic::Inbound(9443), false), &ips), inbound);
    assert_eq!(expression(&f(Traffic::Inbound(9443), true), &ips), format!("{inbound} and {hs}"));
    let both = ["10.0.1.9".to_owned(), "fd00::9".to_owned()];
    assert!(expression(&f(Traffic::Inbound(443), false), &both).contains("(dst host 10.0.1.9 or dst host fd00::9)"));
    assert_eq!(expression(&f(Traffic::Inbound(9443), false), &[]), "tcp port 9443", "no addresses known");
    assert_eq!(expression(&f(Traffic::Custom("  port 443 ".into()), false), &ips), "port 443");
    assert_eq!(expression(&f(Traffic::Custom("port 443".into()), true), &ips), format!("(port 443) and tcp and {hs}"));
    assert_eq!(command("tcp", 60), ["timeout", "-s", "INT", "60", "tcpdump", "-i", "eth0", "-n", "-U", "-w", "-", "tcp"]);
    // An empty expression captures everything on the interface.
    assert_eq!(command(" ", 5).last().map(String::as_str), Some("-"));
}

#[test]
fn the_container_asks_for_nothing_extra() {
    let c = container_spec("subduer-capture-1", IMAGE, 120);
    assert_eq!(c["command"], json!(["sleep", "120"]));
    assert_eq!(c["image"], IMAGE);
    assert!(c.get("securityContext").is_none(), "default capabilities, like --profile=baseline");
}

#[test]
fn states_from_the_pod_status() {
    let with = |status: Value| json!({"status": {"ephemeralContainerStatuses": [
        {"name": "other", "state": {"running": {}}},
        merge_name("subduer-capture-1", status),
    ]}});
    assert_eq!(state(&json!({"status": {}}), "subduer-capture-1"), State::Starting);
    assert_eq!(state(&with(json!({"state": {"waiting": {"reason": "ContainerCreating"}}})), "subduer-capture-1"), State::Starting);
    assert_eq!(
        state(&with(json!({"state": {"running": {}}, "imageID": "ecr/netshoot@sha256:abc"})), "subduer-capture-1"),
        State::Running { image_id: "ecr/netshoot@sha256:abc".into() }
    );
    assert_eq!(
        state(&with(json!({"state": {"waiting": {"reason": "ErrImagePull", "message": "403 Forbidden"}}})), "subduer-capture-1"),
        State::Failed("ErrImagePull: 403 Forbidden".into())
    );
    assert_eq!(
        state(&with(json!({"state": {"terminated": {"reason": "Completed"}}})), "subduer-capture-1"),
        State::Failed("the capture container ended (Completed)".into())
    );
}

fn merge_name(name: &str, mut status: Value) -> Value {
    status["name"] = json!(name);
    status
}

#[tokio::test]
async fn attaching_is_gated_and_dry_runs_first() {
    let api = MockApi::new();
    api.on("PATCH", PATCH_PATH, pod("shop", "web-1", "n1", json!({})));
    let gate = SafetyGate::default();

    // A dry run needs no unlocking and changes nothing on the server.
    let name = attach(&api.client(), &gate, "acme-uat", "shop", "web-1", IMAGE, 300, true).await.unwrap();
    assert!(name.starts_with(CONTAINER_PREFIX), "{name}");
    let call = api.calls_to("PATCH", PATCH_PATH).pop().unwrap();
    assert!(call.query.contains("dryRun=All"), "{}", call.query);
    let sent = &call.json()["spec"]["ephemeralContainers"][0];
    assert_eq!((sent["name"].as_str(), sent["image"].as_str()), (Some(name.as_str()), Some(IMAGE)));

    // The real thing is refused while the context is locked.
    let locked = attach(&api.client(), &gate, "acme-uat", "shop", "web-1", IMAGE, 300, false).await.unwrap_err();
    assert!(matches!(locked, Error::Locked(_)), "{locked:?}");
    assert_eq!(api.calls_to("PATCH", PATCH_PATH).len(), 1);
    gate.set_unlocked("acme-uat", true);
    attach(&api.client(), &gate, "acme-uat", "shop", "web-1", IMAGE, 300, false).await.unwrap();
    assert!(!api.calls_to("PATCH", PATCH_PATH).pop().unwrap().query.contains("dryRun"));

    // No image, or a time limit out of range, is refused before asking.
    for (image, secs) in [(" ", 300), (IMAGE, 0), (IMAGE, MAX_LIFE_SECS + 1)] {
        assert!(attach(&api.client(), &gate, "acme-uat", "shop", "web-1", image, secs, true).await.is_err());
    }
    assert_eq!(api.calls_to("PATCH", PATCH_PATH).len(), 2);
}

#[tokio::test]
async fn waiting_reports_the_image_or_why_not() {
    let api = MockApi::new();
    let status = |s: Value| pod("shop", "web-1", "n1", json!({"status": {"ephemeralContainerStatuses": [s]}}));
    api.on("GET", "/api/v1/namespaces/shop/pods/web-1", status(json!({"name": "c", "state": {"running": {}}, "imageID": "img@sha256:1"})));
    assert_eq!(wait_running(&api.client(), "shop", "web-1", "c", Duration::from_secs(1)).await.unwrap(), "img@sha256:1");
    api.on("GET", "/api/v1/namespaces/shop/pods/web-1", status(json!({"name": "c", "state": {"waiting": {"reason": "ImagePullBackOff"}}})));
    let err = wait_running(&api.client(), "shop", "web-1", "c", Duration::from_secs(1)).await.unwrap_err();
    assert_eq!(err.to_string(), "ImagePullBackOff");
    api.on("GET", "/api/v1/namespaces/shop/pods/web-1", status(json!({"name": "c", "state": {"waiting": {"reason": "ContainerCreating"}}})));
    let err = wait_running(&api.client(), "shop", "web-1", "c", Duration::ZERO).await.unwrap_err();
    assert!(err.to_string().contains("didn't start"), "{err}");
}
