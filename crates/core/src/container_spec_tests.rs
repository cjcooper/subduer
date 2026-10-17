use super::*;
use serde_json::json;

fn rows(c: Value) -> Vec<(String, String)> {
    container_spec(&c)
}

fn get<'a>(rows: &'a [(String, String)], label: &str) -> &'a str {
    rows.iter().find(|(l, _)| l == label).map(|(_, v)| v.as_str()).unwrap_or_else(|| panic!("no {label} in {rows:?}"))
}

#[test]
fn command_and_args_read_like_a_shell() {
    let r = rows(json!({"name": "app", "image": "acme/app:1",
        "command": ["sh", "-c", "echo 'hi' && sleep 5"], "args": ["--port", "8080", ""]}));
    assert_eq!(get(&r, "Command"), r"sh -c 'echo '\''hi'\'' && sleep 5'");
    assert_eq!(get(&r, "Args"), "--port 8080 ''");
}

#[test]
fn common_fields_come_first_in_order() {
    let r = rows(json!({
        "name": "app", "image": "acme/app:1", "ports": [], "resources": {}, "env": [], "restartPolicy": "Always",
        "tty": true,
        "volumeMounts": [{"name": "data", "mountPath": "/data", "readOnly": true, "subPath": "x"},
                         {"name": "tmp", "mountPath": "/tmp"}],
        "readinessProbe": {"httpGet": {"path": "/ready", "port": 8080, "scheme": "HTTP"}, "periodSeconds": 5},
        "livenessProbe": {"tcpSocket": {"port": "http"}, "initialDelaySeconds": 10, "failureThreshold": 6},
        "startupProbe": {"exec": {"command": ["cat", "/tmp/ok"]}},
        "lifecycle": {"preStop": {"sleep": {"seconds": 5}}, "postStart": {"grpc": {"port": 9000}}},
        "workingDir": "/srv",
        "imagePullPolicy": "IfNotPresent",
        "securityContext": {"runAsUser": 1000, "readOnlyRootFilesystem": true, "capabilities": {"drop": ["ALL"]}},
        "resizePolicy": [{"resourceName": "cpu", "restartPolicy": "NotRequired"}],
        "terminationMessagePath": "/dev/termination-log",
        "terminationMessagePolicy": "FallbackToLogsOnError",
        "command": ["app"],
    }));
    let labels: Vec<&str> = r.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels, [
        "Command", "Working dir", "Image pull policy", "Startup probe", "Liveness probe", "Readiness probe",
        "Post-start hook", "Pre-stop hook", "Mounts", "Security context", "Resize policy",
        "Termination message policy", "Tty",
    ]);
    assert_eq!(get(&r, "Startup probe"), "exec cat /tmp/ok delay=0s timeout=1s period=10s #success=1 #failure=3");
    assert_eq!(get(&r, "Liveness probe"), "tcp-socket :http delay=10s timeout=1s period=10s #success=1 #failure=6");
    assert_eq!(get(&r, "Readiness probe"), "http-get http://:8080/ready delay=0s timeout=1s period=5s #success=1 #failure=3");
    assert_eq!(get(&r, "Post-start hook"), "grpc :9000");
    assert_eq!(get(&r, "Pre-stop hook"), "sleep 5s");
    assert_eq!(get(&r, "Mounts"), "data → /data (ro, subPath x)\ntmp → /tmp");
    assert_eq!(get(&r, "Security context"), "capabilities.drop=[ALL], readOnlyRootFilesystem=true, runAsUser=1000");
    assert_eq!(get(&r, "Resize policy"), "resourceName=cpu, restartPolicy=NotRequired");
    assert_eq!(get(&r, "Tty"), "true");
}

#[test]
fn a_bare_container_has_nothing_extra() {
    assert!(rows(json!({"name": "app", "image": "acme/app:1",
        "terminationMessagePath": "/dev/termination-log", "terminationMessagePolicy": "File"})).is_empty());
}

#[test]
fn an_ephemeral_container_names_its_target() {
    let r = rows(json!({"name": "debug", "image": "busybox", "targetContainerName": "app", "stdin": true}));
    assert_eq!(r, [("Target container".to_owned(), "app".to_owned()), ("Stdin".to_owned(), "true".to_owned())]);
}
