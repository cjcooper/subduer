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
            container(&["https 9443/TCP", "8080/TCP"], false),
            container(&["7000/TCP"], true),
        ],
        events: vec![],
    };
    assert_eq!(inbound_ports(&d), [(8080, None), (9090, Some("metrics".into())), (9443, Some("https".into()))]);
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
    assert_eq!(connection_key(&c, 3), Key::Text("192.0.2.9:443".into()));
    assert_eq!(connection_key(&c, 5), Key::Text("tls 1.2".into()));
    assert_eq!(connection_key(&conn(None), 5), Key::Missing, "no answer goes last");
    assert_eq!(connection_key(&c, 6), Key::Missing);
}
