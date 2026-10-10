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
