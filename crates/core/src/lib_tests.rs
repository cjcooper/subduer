use super::*;

fn api_error(message: &str) -> Error {
    Error::Kube(kube::Error::Api(Box::new(kube::core::Status::failure(message, "Forbidden").with_code(403))))
}

#[test]
fn user_message_prefers_the_api_servers_words() {
    let err = api_error("pods \"web\" is forbidden: no access");
    assert_eq!(err.user_message(), "pods \"web\" is forbidden: no access");
    // An empty message falls back to the full description.
    let err = api_error("");
    assert!(!err.user_message().is_empty());
    assert_eq!(err.user_message(), err.to_string());
}

#[test]
fn user_message_for_our_own_errors() {
    assert_eq!(Error::Locked("acme-prd".into()).user_message(), "context \"acme-prd\" is read-only; unlock it to make changes");
    assert_eq!(Error::NotFound("pod web".into()).user_message(), "not found: pod web");
    assert_eq!(Error::UnknownContext("acme".into()).user_message(), "context \"acme\" not found in kubeconfig");
    assert_eq!(Error::Message("As is.".into()).user_message(), "As is.");
    let yaml: Error = serde_norway::from_str::<u32>("[").unwrap_err().into();
    assert!(yaml.user_message().starts_with("yaml: "));
}

/// The websocket exec fake, shared by the download, upload and exec tests.
#[path = "ws_fake_tests.rs"]
pub(crate) mod ws_fake;
