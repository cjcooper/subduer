use super::*;
use crate::app::tests_unit::store;

#[test]
fn request_opens_the_dialog_without_unlocking() {
    let s = store();
    request(s, "shop".into(), "web-1".into(), "app".into());
    assert_eq!(
        s.download_prompt.get_untracked(),
        Some(DownloadPrompt { namespace: "shop".into(), pod: "web-1".into(), container: "app".into() })
    );
    assert_eq!(s.unlock_prompt.get_untracked(), None);
}
