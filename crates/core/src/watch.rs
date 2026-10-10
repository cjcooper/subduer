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

/// Minimum time between snapshots, so busy clusters don't flood the UI.
const THROTTLE: Duration = Duration::from_millis(250);

/// Watches `kind` (optionally limited to `namespace`) and calls `emit` with a
/// full table snapshot whenever it changes. Runs until the future is dropped
/// or aborted. The watcher reconnects with backoff on errors.
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
        }
    };

    let mut stream = watcher(api, watcher::Config::default()).default_backoff().boxed();
    let mut rows: BTreeMap<String, ResourceRow> = BTreeMap::new();
    // Rows collected during a (re)list; swapped in at InitDone.
    let mut pending: Option<BTreeMap<String, ResourceRow>> = None;
    let mut dirty = false;
    let mut ticker = tokio::time::interval(THROTTLE);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            event = stream.next() => {
                let Some(event) = event else { break };
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
                        emit(snapshot(&rows, Some(err.to_string())));
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
