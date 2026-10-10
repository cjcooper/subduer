//! Container logs: one container, or every container of a workload's pods
//! merged (like `stern`). Lines are batched so a chatty pod doesn't flood the UI.

use std::time::Duration;

use futures::{AsyncBufReadExt, StreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams, LogParams};
use kube::{Client, ResourceExt};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::Result;
use crate::cluster::dynamic_api;
use crate::types::{LogBatch, LogLine, LogSource, LogTarget};

/// Lines of history per container when a window opens.
const TAIL_SINGLE: i64 = 1000;
const TAIL_MERGED: i64 = 200;
/// Merged windows follow at most this many pods.
const MAX_PODS: usize = 20;
const FLUSH_EVERY: Duration = Duration::from_millis(150);
const MAX_BATCH: usize = 2000;

/// One container stream to follow.
#[derive(Debug, Clone, PartialEq)]
struct Stream {
    pod: String,
    container: String,
    /// `pod/container` label, set when merging.
    label: Option<String>,
}

enum Msg {
    Line(LogLine),
    Notice(String),
}

/// Streams `target`'s logs, calling `emit` with batches until every stream
/// ends or the future is dropped. `previous` reads the last terminated
/// container instead (useful after a crash).
pub async fn stream<F>(client: Client, target: LogTarget, previous: bool, emit: F) -> Result<()>
where
    F: Fn(LogBatch),
{
    let pods: Api<Pod> = Api::namespaced(client.clone(), &target.namespace);
    let (streams, mut notices) = resolve(&client, &pods, &target).await?;
    let merged = matches!(target.source, LogSource::Workload { .. });
    if streams.is_empty() {
        notices.push("No running containers to follow.".into());
        emit(LogBatch { lines: vec![], notices });
        return Ok(());
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let tail = if merged { TAIL_MERGED } else { TAIL_SINGLE };
    let mut tasks = Vec::new();
    for s in streams {
        let (pods, tx) = (pods.clone(), tx.clone());
        tasks.push(tokio::spawn(async move { follow(pods, s, tail, previous, tx).await }));
    }
    drop(tx);
    // Abort the per-container tasks when this future is dropped.
    let _guard = AbortOnDrop(tasks);

    let mut batch = LogBatch { lines: vec![], notices };
    let mut ticker = tokio::time::interval(FLUSH_EVERY);
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(Msg::Line(line)) => {
                    batch.lines.push(line);
                    if batch.lines.len() >= MAX_BATCH {
                        flush(&mut batch, merged, &emit);
                    }
                }
                Some(Msg::Notice(n)) => batch.notices.push(n),
                None => break,
            },
            _ = ticker.tick() => flush(&mut batch, merged, &emit),
        }
    }
    batch.notices.push(if previous { "End of previous logs.".into() } else { "All streams ended.".into() });
    flush(&mut batch, merged, &emit);
    Ok(())
}

fn flush<F: Fn(LogBatch)>(batch: &mut LogBatch, merged: bool, emit: &F) {
    if batch.lines.is_empty() && batch.notices.is_empty() {
        return;
    }
    let mut out = std::mem::take(batch);
    if merged {
        // Interleave streams by time within a batch; RFC 3339 sorts as text.
        out.lines.sort_by(|a, b| a.time.cmp(&b.time));
    }
    emit(out);
}

struct AbortOnDrop(Vec<tokio::task::JoinHandle<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        for t in &self.0 {
            t.abort();
        }
    }
}

async fn follow(pods: Api<Pod>, s: Stream, tail: i64, previous: bool, tx: mpsc::UnboundedSender<Msg>) {
    let name = s.label.clone().unwrap_or_else(|| s.container.clone());
    let params = LogParams {
        container: Some(s.container.clone()),
        follow: !previous,
        previous,
        tail_lines: Some(tail),
        timestamps: true,
        ..LogParams::default()
    };
    let reader = match pods.log_stream(&s.pod, &params).await {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(Msg::Notice(format!("{name}: {}", crate::Error::from(e).user_message())));
            return;
        }
    };
    let mut lines = reader.lines();
    while let Some(line) = lines.next().await {
        match line {
            Ok(raw) => {
                let (time, text) = split_timestamp(&raw);
                let line = LogLine { source: s.label.clone(), time, text: strip_ansi(text) };
                if tx.send(Msg::Line(line)).is_err() {
                    return;
                }
            }
            Err(e) => {
                let _ = tx.send(Msg::Notice(format!("{name}: stream error: {e}")));
                return;
            }
        }
    }
    if !previous && s.label.is_some() {
        let _ = tx.send(Msg::Notice(format!("{name}: stream ended")));
    }
}

/// The containers to follow, plus notices about anything left out.
async fn resolve(client: &Client, pods: &Api<Pod>, target: &LogTarget) -> Result<(Vec<Stream>, Vec<String>)> {
    match &target.source {
        LogSource::Container { pod, container } => Ok((
            vec![Stream { pod: pod.clone(), container: container.clone(), label: None }],
            vec![],
        )),
        LogSource::Workload { kind, name } => {
            let obj = dynamic_api(client, kind, Some(&target.namespace)).get(name).await?;
            let Some(selector) = label_selector(&obj.data["spec"]["selector"]) else {
                return Ok((vec![], vec![format!("{} {name} has no pod selector.", kind.kind)]));
            };
            let mut list: Vec<Pod> = pods.list(&ListParams::default().labels(&selector)).await?.items;
            list.retain(|p| p.metadata.deletion_timestamp.is_none());
            list.sort_by_key(|p| p.name_any());
            let mut notices = vec![];
            if list.len() > MAX_PODS {
                notices.push(format!("Following the first {MAX_PODS} of {} pods.", list.len()));
                list.truncate(MAX_PODS);
            }
            let streams: Vec<Stream> = list.iter().flat_map(pod_streams).collect();
            notices.push(format!(
                "Following {} container{} in {} pod{} (pods started later aren't added).",
                streams.len(),
                plural(streams.len()),
                list.len(),
                plural(list.len()),
            ));
            Ok((streams, notices))
        }
    }
}

/// App containers and sidecars (init containers with `restartPolicy: Always`).
fn pod_streams(pod: &Pod) -> Vec<Stream> {
    let name = pod.name_any();
    let Some(spec) = &pod.spec else { return vec![] };
    let sidecars = spec.init_containers.iter().flatten().filter(|c| c.restart_policy.as_deref() == Some("Always"));
    sidecars
        .chain(&spec.containers)
        .map(|c| Stream { pod: name.clone(), container: c.name.clone(), label: Some(format!("{name}/{}", c.name)) })
        .collect()
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// A label selector string from `spec.selector`: either a LabelSelector
/// (`matchLabels`/`matchExpressions`) or, for ReplicationControllers, a plain map.
fn label_selector(sel: &Value) -> Option<String> {
    let obj = sel.as_object()?;
    let mut parts: Vec<String> = vec![];
    let is_label_selector = obj.contains_key("matchLabels") || obj.contains_key("matchExpressions");
    let labels = if is_label_selector { sel["matchLabels"].as_object() } else { Some(obj) };
    for (k, v) in labels.into_iter().flatten() {
        parts.push(format!("{k}={}", v.as_str()?));
    }
    for e in sel["matchExpressions"].as_array().into_iter().flatten() {
        let key = e["key"].as_str()?;
        let values: Vec<&str> = e["values"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        parts.push(match e["operator"].as_str()? {
            "In" => format!("{key} in ({})", values.join(",")),
            "NotIn" => format!("{key} notin ({})", values.join(",")),
            "Exists" => key.to_owned(),
            "DoesNotExist" => format!("!{key}"),
            _ => return None,
        });
    }
    (!parts.is_empty()).then(|| parts.join(","))
}

/// Splits the kubelet's `timestamps=true` prefix off a line.
fn split_timestamp(raw: &str) -> (Option<String>, &str) {
    match raw.split_once(' ') {
        Some((ts, rest)) if ts.len() >= 20 && ts.as_bytes()[4] == b'-' && ts.ends_with('Z') => (Some(ts.to_owned()), rest),
        _ => (None, raw),
    }
}

/// Removes ANSI escape sequences (colours, cursor moves) that many apps log.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.next_if_eq(&'[').is_some() {
            // CSI: parameters, then one final byte in @..~.
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        } else {
            chars.next();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selectors() {
        let sel = json!({"matchLabels": {"app": "web", "tier": "api"}});
        assert_eq!(label_selector(&sel).as_deref(), Some("app=web,tier=api"));
        let sel = json!({"matchExpressions": [
            {"key": "env", "operator": "In", "values": ["prd", "uat"]},
            {"key": "canary", "operator": "DoesNotExist"},
        ]});
        assert_eq!(label_selector(&sel).as_deref(), Some("env in (prd,uat),!canary"));
        // ReplicationController: a plain map.
        assert_eq!(label_selector(&json!({"app": "old"})).as_deref(), Some("app=old"));
        assert_eq!(label_selector(&json!({})), None);
    }

    #[test]
    fn timestamps_and_ansi() {
        let (t, rest) = split_timestamp("2026-10-10T00:00:01.123456789Z hello world");
        assert_eq!(t.as_deref(), Some("2026-10-10T00:00:01.123456789Z"));
        assert_eq!(rest, "hello world");
        assert_eq!(split_timestamp("no timestamp here"), (None, "no timestamp here"));
        assert_eq!(strip_ansi("\u{1b}[32mINFO\u{1b}[0m ready"), "INFO ready");
    }
}
