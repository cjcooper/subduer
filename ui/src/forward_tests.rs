use super::*;
use crate::app::tests_unit::store;

#[test]
fn request_opens_the_start_dialog() {
    let s = store();
    request(s, "shop".into(), ForwardSource::Pod { pod: "web-1".into() });
    assert_eq!(
        s.forward_prompt.get_untracked(),
        Some(ForwardPrompt { namespace: "shop".into(), source: ForwardSource::Pod { pod: "web-1".into() } })
    );
}
