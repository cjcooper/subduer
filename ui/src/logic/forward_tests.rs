use super::*;

fn info() -> ForwardInfo {
    ForwardInfo {
        id: 1,
        context: "acme-dev".into(),
        namespace: "shop".into(),
        source: ForwardSource::Pod { pod: "web-1".into() },
        remote_port: 80,
        local_port: 8080,
        backend: None,
        active: 2,
        total: 7,
        last_error: None,
    }
}

fn prompt() -> ForwardPrompt {
    ForwardPrompt { namespace: "shop".into(), source: ForwardSource::Service { service: "web".into() } }
}

fn port(port: u16, name: Option<&str>, detail: Option<&str>) -> PortOption {
    PortOption { port, name: name.map(str::to_owned), detail: detail.map(str::to_owned) }
}

#[test]
fn address_is_the_local_loopback_port() {
    assert_eq!(address(&info()), "127.0.0.1:8080");
}

#[test]
fn forwards_sort_by_ports_and_connections() {
    let f = info();
    assert_eq!(forward_key(&f, 0), Key::Text("shop/pod/web-1".into()));
    assert_eq!(forward_key(&f, 1), Key::Text("acme-dev".into()));
    assert_eq!((forward_key(&f, 2), forward_key(&f, 3)), (Key::Num(8080.0), Key::Num(80.0)));
    let busier = ForwardInfo { active: 3, total: 0, ..f.clone() };
    let (Key::Num(a), Key::Num(b)) = (forward_key(&f, 4), forward_key(&busier, 4)) else { panic!() };
    assert!(b > a, "open connections count first");
    assert_eq!(forward_key(&f, 5), Key::Missing);
}

#[test]
fn the_first_port_is_chosen() {
    assert_eq!(first_choice(&Ok(vec![port(8080, None, None), port(9090, None, None)])), Some((Some(8080), "8080".into())));
    assert_eq!(first_choice(&Ok(vec![])), Some((None, String::new())));
    assert_eq!(first_choice(&Err("boom".into())), None);
}

#[test]
fn local_port_and_start() {
    assert_eq!(local_port(" 8080 "), Some(8080));
    assert_eq!(local_port("0"), Some(0));
    assert_eq!(local_port("70000"), None);
    assert_eq!(local_port("http"), None);
    assert!(can_start(false, Some(80), "8080"));
    assert!(!can_start(true, Some(80), "8080"));
    assert!(!can_start(false, None, "8080"));
    assert!(!can_start(false, Some(80), "x"));
}

#[test]
fn texts() {
    assert_eq!(target("shop", &ForwardSource::Service { service: "web".into() }), "shop/service/web");
    assert_eq!(port_label(&port(80, Some("http"), Some("app"))), "80 · http · app");
    assert_eq!(port_label(&port(80, Some("http"), None)), "80 · http");
    assert_eq!(port_label(&port(80, None, Some("8080"))), "80 · 8080");
    assert_eq!(port_label(&port(80, None, None)), "80");
    assert_eq!(remote(&info()), "80");
    assert_eq!(remote(&ForwardInfo { backend: Some("web-1:8080".into()), ..info() }), "80 → web-1:8080");
    assert_eq!(connections(&info()), "2 open · 7 total");
    assert_eq!(HEADERS.len(), 6);
}

#[test]
fn requests() {
    let ports = serde_json::to_value(ports_args(prompt())).unwrap();
    assert_eq!(ports["namespace"], "shop");
    assert!(ports["source"].is_object());
    let start = start_args(prompt(), 80, 0);
    assert_eq!(
        start.request,
        ForwardRequest { namespace: "shop".into(), source: ForwardSource::Service { service: "web".into() }, remote_port: 80, local_port: 0 }
    );
    assert_eq!(serde_json::to_value(&start).unwrap()["request"]["remote_port"], 80);
    assert_eq!(serde_json::to_value(IdArgs { id: 4 }).unwrap(), serde_json::json!({ "id": 4 }));
}
