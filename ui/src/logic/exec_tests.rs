use super::*;

fn target() -> ExecTarget {
    ExecTarget { namespace: "shop".into(), pod: "web-1".into(), container: "app".into() }
}

#[test]
fn label_in_finds_the_exec_parameter() {
    assert_eq!(label_in("?exec=shell-3").as_deref(), Some("shell-3"));
    assert_eq!(label_in("?theme=dark&exec=shell-3&x=1").as_deref(), Some("shell-3"));
    assert_eq!(label_in("exec=abc").as_deref(), Some("abc"));
    assert_eq!(label_in("?exec="), None);
    assert_eq!(label_in("?logs=abc"), None);
    assert_eq!(label_in("?myexec=abc"), None);
    assert_eq!(label_in(""), None);
}

#[test]
fn theme_darkens_ansi_colours_on_light() {
    let dark = theme(true);
    assert_eq!(dark.len(), 4, "xterm's own ANSI colours on dark");
    assert_eq!(dark[0], ("background", "#1d2128"));
    let light = theme(false);
    assert_eq!(light.len(), 20);
    assert_eq!(light[0], ("background", "#ffffff"));
    assert!(light.contains(&("brightYellow", "#734500")));
}

#[test]
fn texts_and_sizes() {
    assert_eq!(dimension(Some(120.0), 80.0), 120);
    assert_eq!(dimension(None, 80.0), 80);
    assert_eq!(exited_line("exit 0"), "\r\n\x1b[2m[exit 0]\x1b[0m\r\n");
    assert_eq!(heading(None), "Connecting…");
    let s = ExecSession { context: "acme".into(), target: target(), title: "app · Shell".into(), event: "e".into() };
    assert_eq!(heading(Some(Ok(s))), "app · Shell");
    assert_eq!(heading(Some(Err("x".into()))), "Shell");
}

#[test]
fn args_serialize() {
    assert_eq!(serde_json::to_value(SizeArgs { cols: 80, rows: 24 }).unwrap(), serde_json::json!({ "cols": 80, "rows": 24 }));
    assert_eq!(serde_json::to_value(InputArgs { data: "ls\r".into() }).unwrap(), serde_json::json!({ "data": "ls\r" }));
    assert_eq!(serde_json::to_value(OpenArgs { target: target() }).unwrap()["target"]["pod"], "web-1");
}
