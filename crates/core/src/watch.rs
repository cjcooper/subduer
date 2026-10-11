//! Live resource lists backed by a Kubernetes watch.

use std::collections::BTreeMap;
use std::time::Duration;

use futures::StreamExt;
use kube::Client;
use kube::api::DynamicObject;
use kube::runtime::{WatchStreamExt, watcher};

use crate::cluster::dynamic_api;
use crate::table;
use crate::types::{KindRef, ResourceRow, ResourceTable};

/// kube's default backoff (0.8s doubling to 30s), except that it ignores
/// `StreamBackoff`'s reset on every `Ok`: the `Init` before each relist
/// would otherwise hold failing retries at about one a second. It still
/// starts over after 120s without a failure.
pub(crate) struct Backoff(watcher::DefaultBackoff);

impl Backoff {
    pub(crate) fn new() -> Self {
        Self(watcher::DefaultBackoff::default())
    }
}

impl Iterator for Backoff {
    type Item = Duration;

    fn next(&mut self) -> Option<Duration> {
        self.0.next()
    }
}

impl kube::runtime::utils::Backoff for Backoff {
    fn reset(&mut self) {}
}

/// Minimum time between snapshots, so busy clusters don't flood the UI.
const THROTTLE: Duration = Duration::from_millis(250);

/// Failed attempts in a row before a watch gives up (about 12s with kube's
/// backoff), so expired credentials aren't retried forever.
pub const MAX_ATTEMPTS: u32 = 5;

/// Counts failed attempts in a row. `Init` doesn't count as a success: the
/// watcher emits it before every relist, failed or not.
#[derive(Default)]
pub(crate) struct Failures(u32);

impl Failures {
    pub(crate) fn seen<K>(&mut self, event: &Result<watcher::Event<K>, watcher::Error>) {
        if matches!(event, Ok(e) if !matches!(e, watcher::Event::Init)) {
            self.0 = 0;
        }
    }

    /// Records a failure; true once the watch should give up.
    pub(crate) fn failed(&mut self) -> bool {
        self.0 += 1;
        self.0 >= MAX_ATTEMPTS
    }
}

/// Watches `kind` (optionally limited to `namespace`) and calls `emit` with a
/// full table snapshot whenever it changes. Runs until the future is dropped
/// or aborted, or until [`MAX_ATTEMPTS`] attempts in a row fail; the last
/// table then has `stopped` set. The watcher retries with backoff until then.
pub async fn watch_table<F>(client: Client, kind: KindRef, namespace: Option<String>, watch_id: u64, emit: F)
where
    F: Fn(ResourceTable),
{
    if crate::helm::is_release(&kind) {
        return crate::helm::watch_releases(client, namespace, watch_id, emit).await;
    }
    let api = dynamic_api(&client, &kind, namespace.as_deref());
    let columns = table::columns(&kind);
    let snapshot = |rows: &BTreeMap<String, ResourceRow>, error: Option<String>| {
        let mut rows: Vec<ResourceRow> = rows.values().cloned().collect();
        table::sort_rows(&kind, &mut rows);
        ResourceTable {
            watch_id,
            columns: columns.clone(),
            rows,
            error,
            stopped: false,
        }
    };

    let mut stream = watcher(api, watcher::Config::default()).backoff(Backoff::new()).boxed();
    let mut rows: BTreeMap<String, ResourceRow> = BTreeMap::new();
    // Rows collected during a (re)list; swapped in at InitDone.
    let mut pending: Option<BTreeMap<String, ResourceRow>> = None;
    let mut dirty = false;
    let mut failures = Failures::default();
    let mut ticker = tokio::time::interval(THROTTLE);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            event = stream.next() => {
                let Some(event) = event else { break };
                failures.seen(&event);
                match event {
                    Ok(watcher::Event::Init) => pending = Some(BTreeMap::new()),
                    Ok(watcher::Event::InitApply(obj)) => {
                        let row = table::row(&kind, &obj);
                        pending.get_or_insert_with(BTreeMap::new).insert(row.id.clone(), row);
                    }
                    Ok(watcher::Event::InitDone) => {
                        rows = pending.take().unwrap_or_default();
                        dirty = false;
                        emit(snapshot(&rows, None));
                    }
                    Ok(watcher::Event::Apply(obj)) => {
                        let row = table::row(&kind, &obj);
                        rows.insert(row.id.clone(), row);
                        dirty = true;
                    }
                    Ok(watcher::Event::Delete(obj)) => {
                        rows.remove(&row_key(&obj));
                        dirty = true;
                    }
                    Err(err) => {
                        tracing::warn!(%err, kind = %kind.key(), "watch error");
                        let stopped = failures.failed();
                        emit(ResourceTable { stopped, ..snapshot(&rows, Some(err.to_string())) });
                        if stopped {
                            tracing::warn!(kind = %kind.key(), "watch gave up");
                            return;
                        }
                    }
                }
            }
            _ = ticker.tick(), if dirty => {
                dirty = false;
                emit(snapshot(&rows, None));
            }
        }
    }
}

fn row_key(obj: &DynamicObject) -> String {
    table::row_id(obj.metadata.namespace.as_deref(), obj.metadata.name.as_deref().unwrap_or_default())
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod coverage_tests;
