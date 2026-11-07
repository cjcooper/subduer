use super::*;

fn conn(version: Option<&str>) -> TlsConnection {
    TlsConnection {
        first_seen: 0.0,
        client: "10.0.0.5:40000".into(),
        server: "192.0.2.9:443".into(),
        direction: TlsDirection::Outbound,
        server_name: None,
        offered: vec![],
        version: version.map(Into::into),
        cipher: None,
        retried: false,
    }
}

#[test]
fn filters_from_the_dialog() {
    let f = |traffic, handshakes_only| CaptureFilter { traffic, handshakes_only };
    assert_eq!(filter_from("all", "x", true), f(CaptureTraffic::All, true));
    assert_eq!(filter_from("port:9443", "", false), f(CaptureTraffic::Inbound(9443), false));
    assert_eq!(filter_from("custom", "  port 443 ", true), f(CaptureTraffic::Custom("port 443".into()), true));
    assert_eq!(filter_from("port:nope", "", true), f(CaptureTraffic::All, true));
    assert_eq!(DURATIONS.map(minutes), ["1 min", "5 min", "10 min", "30 min"]);
    // Context names can be ARNs; the suggestion stays a plain file name.
    assert_eq!(file_name("acme-uat", "shop", "web-1", "2026-10-12-1430"), "acme-uat-shop-web-1-2026-10-12-1430.pcap");
    assert_eq!(file_name("arn:aws:eks:us-east-1:1:cluster/uat", "shop", "web-1", "s"), "arn-aws-eks-us-east-1-1-cluster-uat-shop-web-1-s.pcap");
}

#[test]
fn the_pods_tcp_ports_once_each() {
    let container = |ports: &[&str], ephemeral: bool| subduer_types::ContainerDetails {
        name: "c".into(),
        init: false,
        sidecar: false,
        ephemeral,
        image: String::new(),
        state: subduer_types::containers::State::Running,
        detail: String::new(),
        ready: true,
        restarts: 0,
        started: None,
        last_termination: None,
        ports: ports.iter().map(|p| (*p).to_owned()).collect(),
        cpu: Default::default(),
        memory: Default::default(),
        env: vec![],
        spec: vec![],
    };
    let d = PodDetails {
        name: "web-1".into(),
        namespace: "shop".into(),
        status: "Running".into(),
        node: None,
        pod_ip: None,
        host_ip: None,
        qos_class: None,
        service_account: None,
        owner: None,
        restart_policy: None,
        priority_class: None,
        created: None,
        labels: vec![],
        cpu: Default::default(),
        memory: Default::default(),
        metrics_available: false,
        containers: vec![
            container(&["9443/TCP", "metrics 9090/TCP", "53/UDP"], false),
            container(&["https 9443/TCP", "8080/TCP", "web 9443/TCP"], false),
            container(&["7000/TCP"], true),
        ],
        events: vec![],
    };
    assert_eq!(inbound_ports(&d), [(8080, None), (9090, Some("metrics".into())), (9443, Some("https".into()))]);
    let details = Some(Ok(d));
    assert_eq!(pod_ports(details.as_ref(), "shop", "web-1").len(), 3);
    assert!(pod_ports(details.as_ref(), "shop", "web-2").is_empty(), "another pod's details");
    assert!(pod_ports(details.as_ref(), "ops", "web-1").is_empty());
    assert!(pod_ports(None, "shop", "web-1").is_empty());
    assert_eq!(port_choice(9090, Some("metrics")), ("port:9090".into(), "Inbound to port 9090 (metrics)".into()));
    assert_eq!(port_choice(8080, None), ("port:8080".into(), "Inbound to port 8080".into()));
}

fn update(phase: CapturePhase) -> CaptureUpdate {
    CaptureUpdate {
        phase,
        message: None,
        image_id: None,
        started: Some(1000.0),
        ends: Some(1300.0),
        bytes: 2048,
        scan: TlsScan { packets: 9, hellos: 2, connections: vec![conn(None)] },
    }
}

#[test]
fn the_dialogs_choices() {
    let c = Choices { image: " netshoot ".into(), secs: 300, choice: "custom".into(), custom: " port 443 ".into(), handshakes: false };
    assert!(c.ready(false, false, false, "", "acme-uat"));
    assert!(c.ready(false, false, true, " acme-prd ", "acme-prd"));
    assert!(!c.ready(false, false, true, "acme", "acme-prd"), "the name must be typed");
    assert!(!c.ready(true, false, false, "", "acme-uat"), "busy");
    assert!(!c.ready(false, true, false, "", "acme-uat"), "locked");
    assert!(!Choices { image: " ".into(), ..c.clone() }.ready(false, false, false, "", "acme-uat"), "no image");
    assert!(!Choices { custom: " ".into(), ..c.clone() }.ready(false, false, false, "", "acme-uat"), "no custom filter");
    assert!(Choices { choice: "all".into(), custom: String::new(), ..c.clone() }.ready(false, false, false, "", "acme-uat"));
    assert_eq!(c.request("shop", "web-1"), CaptureRequest {
        namespace: "shop".into(),
        pod: "web-1".into(),
        image: "netshoot".into(),
        filter: CaptureFilter { traffic: CaptureTraffic::Custom("port 443".into()), handshakes_only: false },
        secs: 300,
        path: String::new(),
    });
}

#[test]
fn the_capture_window() {
    let mut u = update(CapturePhase::Capturing);
    assert_eq!(status_text(&u, 1055.0), "Capturing · 4:05 left");
    assert_eq!(totals_text(&u), "9 packets · 2.0 KiB · 1 TLS connections");
    assert_eq!(progress(&u, 1150.0), Some(0.5));
    assert_eq!(progress(&u, 2000.0), Some(1.0), "clamped");
    assert_eq!(progress_attrs(0.5), ("50".into(), "width: 50.0%".into()));
    assert_eq!(image_text(&u, "netshoot"), "netshoot");
    u.image_id = Some("netshoot@sha256:1".into());
    assert_eq!(image_text(&u, "netshoot"), "netshoot@sha256:1");
    assert!(running(u.phase) && running(CapturePhase::Starting) && !running(CapturePhase::Done));
    assert_eq!(stop_button(false, CapturePhase::Capturing), (false, "Stop"));
    assert_eq!(stop_button(true, CapturePhase::Capturing), (true, "Stopping…"));
    assert_eq!(stop_button(false, CapturePhase::Stopping), (true, "Stopping…"));

    u.phase = CapturePhase::Done;
    assert_eq!(status_text(&u, 1055.0), "Finished");
    assert_eq!(progress(&u, 1150.0), Some(1.0));
    u.ends = None;
    assert_eq!(progress(&u, 1150.0), None, "no end yet");

    let phases = [CapturePhase::Starting, CapturePhase::Capturing, CapturePhase::Stopping, CapturePhase::Done, CapturePhase::Failed];
    assert_eq!(phases.map(phase_text), ["Starting the capture container…", "Capturing", "Stopping…", "Finished", "Failed"]);
    assert_eq!(phases.map(phase_class), [
        "capture-phase live",
        "capture-phase live",
        "capture-phase live",
        "capture-phase done",
        "capture-phase failed"
    ]);
    assert_eq!(count_badge("TLS 1.3", 2), ("flag good", "TLS 1.3 · 2".into()));
    assert_eq!(count_badge("no answer", 1).0, "flag info");
    assert_eq!(count_badge("TLS 1.0", 1).0, "flag serious");
    assert_eq!(size_text(1024), "1.0 KiB");
    assert_eq!(where_text("shop", "acme-uat"), "shop · acme-uat");
    assert_eq!([TlsDirection::Inbound, TlsDirection::Outbound, TlsDirection::Other].map(way), ["in", "out", ""]);
}

#[test]
fn newest_handshakes_first_unless_sorted() {
    let mut a = conn(Some("TLS 1.2"));
    a.first_seen = 1.0;
    let mut b = conn(Some("TLS 1.3"));
    b.first_seen = 2.0;
    let seen = |list: Vec<TlsConnection>| list.iter().map(|c| c.first_seen).collect::<Vec<_>>();
    assert_eq!(seen(rows(vec![a.clone(), b.clone()], None)), [2.0, 1.0]);
    assert_eq!(seen(rows(vec![a, b], Some((0, false)))), [1.0, 2.0]);
}

#[test]
fn versions_counted_and_flagged() {
    let scan = TlsScan {
        packets: 9,
        hellos: 9,
        connections: vec![conn(Some("TLS 1.2")), conn(Some("TLS 1.3")), conn(None), conn(Some("TLS 1.3")), conn(Some("TLS 1.0")), conn(Some("TLS 1.2"))],
    };
    assert_eq!(version_counts(&scan), [
        ("TLS 1.3".to_owned(), 2),
        ("TLS 1.2".to_owned(), 2),
        ("TLS 1.0".to_owned(), 1),
        ("no answer".to_owned(), 1),
    ]);
    assert!(flagged(&conn(Some("TLS 1.2"))));
    assert!(!flagged(&conn(Some("TLS 1.3"))));
    assert!(!flagged(&conn(None)), "no answer isn't a finding, just missing");
}

#[test]
fn small_text() {
    assert_eq!((clock_text(245.4), clock_text(-3.0)), ("4:05".into(), "0:00".into()));
    assert_eq!(label_in("?capture=capture-2"), Some("capture-2".into()));
    assert_eq!(label_in("?exec=exec-1"), None);
    assert_eq!(label_in("?capture=1"), None, "only capture window labels");
}

#[test]
fn handshakes_sort_by_each_column() {
    use crate::sort::Key;
    let c = conn(Some("TLS 1.2"));
    assert_eq!(connection_key(&c, 0), Key::Num(0.0));
    assert_eq!(connection_key(&c, 1), Key::Text("out".into()));
    assert_eq!(connection_key(&c, 2), Key::Text("10.0.0.5:40000".into()));
    assert_eq!(connection_key(&c, 3), Key::Text("192.0.2.9:443".into()));
    assert_eq!(connection_key(&c, 4), Key::Missing);
    assert_eq!(connection_key(&c, 5), Key::Text("tls 1.2".into()));
    assert_eq!(connection_key(&conn(None), 5), Key::Missing, "no answer goes last");
    assert_eq!(connection_key(&c, 6), Key::Missing);
}
