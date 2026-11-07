use super::*;

#[test]
fn messages() {
    assert_eq!(bad_response("connect", "missing field `name`"), "connect: bad response: missing field `name`");
    assert_eq!(call_error(Some("not connected".into()), &|| unreachable!()), "not connected");
    assert_eq!(call_error(None, &|| "JsValue(Object)".into()), "JsValue(Object)");
    assert_eq!(bad_payload("resource-table", "expected a map"), "bad resource-table payload: expected a map");
}
