use super::*;
use crate::app::tests_unit::store;

#[test]
fn address_is_the_local_loopback_port() {
    let f = ForwardInfo {
        id: 7,
        context: "acme-dev".into(),
        namespace: "shop".into(),
        source: ForwardSource::Service { service: "web".into() },
        remote_port: 80,
        local_port: 8080,
        backend: None,
        active: 0,
        total: 0,
        last_error: None,
    };
    assert_eq!(address(&f), "127.0.0.1:8080");
}

#[test]
fn request_opens_the_start_dialog() {
    let s = store();
    request(s, "shop".into(), ForwardSource::Pod { pod: "web-1".into() });
    assert_eq!(
        s.forward_prompt.get_untracked(),
        Some(ForwardPrompt { namespace: "shop".into(), source: ForwardSource::Pod { pod: "web-1".into() } })
    );
}

#[test]
fn forwards_sort_by_ports_and_connections() {
    use crate::sort::Key;
    let f = ForwardInfo {
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
    };
    assert_eq!(forward_key(&f, 0), Key::Text("shop/pod/web-1".into()));
    assert_eq!((forward_key(&f, 2), forward_key(&f, 3)), (Key::Num(8080.0), Key::Num(80.0)));
    let busier = ForwardInfo { active: 3, total: 0, ..f.clone() };
    let (Key::Num(a), Key::Num(b)) = (forward_key(&f, 4), forward_key(&busier, 4)) else { panic!() };
    assert!(b > a, "open connections count first");
    assert_eq!(forward_key(&f, 5), Key::Missing);
}
