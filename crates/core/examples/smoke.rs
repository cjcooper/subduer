//! Read-only end-to-end check:
//! `cargo run -p subduer-core --example smoke -- [context] [group/Kind]`
//! (defaults: docker-desktop, core/Pod). Discovers kinds, lists namespaces,
//! and prints the first table snapshot for the kind.

use std::time::Duration;

use subduer_core::safety::SafetyGate;
use subduer_core::{cluster, contexts, watch};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let context = std::env::args().nth(1).unwrap_or_else(|| "docker-desktop".into());
    let kubeconfig = contexts::load_kubeconfig()?;
    for c in contexts::list_contexts(&kubeconfig, &SafetyGate::default()).contexts {
        println!("context {:<20} prod_like={}", c.name, c.prod_like);
    }

    let client = contexts::client_for(&kubeconfig, &context).await?;
    let kinds = cluster::discover_kinds(&client).await?;
    println!("{} listable kinds on {context}", kinds.len());
    for info in &kinds {
        println!("  {:<16} {}", info.category.label(), info.kind.key());
    }
    println!("namespaces: {:?}", cluster::list_namespaces(&client).await?);

    let key = std::env::args().nth(2).unwrap_or_else(|| "core/Pod".into());
    let kind = kinds.iter().find(|k| k.kind.key() == key).expect("kind not found").kind.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(watch::watch_table(client, kind, None, 1, move |t| {
        let _ = tx.send(t);
    }));
    let table = tokio::time::timeout(Duration::from_secs(15), rx.recv()).await.expect("snapshot").unwrap();
    task.abort();
    let headers: Vec<_> = table.columns.iter().map(|c| c.header.as_str()).collect();
    println!("{headers:?}");
    for row in table.rows {
        println!("{:?}", row.cells);
    }
    Ok(())
}
