//! Read-only: `cargo run -p subduer-core --example workload -- <context> <apps/Deployment|apps/StatefulSet|batch/Job|batch/CronJob> <namespace> <name>`
//! Prints the Details tab data and where its usage history comes from.
//! Environment values are not printed, and Secret values are never fetched.

use subduer_core::history::HistoryStore;
use subduer_core::{cluster, contexts, workload};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [context, key, namespace, name] = &args[..] else {
        eprintln!("usage: workload <context> <group/Kind> <namespace> <name>");
        std::process::exit(2);
    };
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let kinds = cluster::discover_kinds(&client).await?;
    let Some(info) = kinds.iter().find(|k| k.kind.key() == *key) else {
        eprintln!("kind {key} not found");
        std::process::exit(2);
    };
    let d = workload::workload_details(&client, &info.kind, namespace, name).await?;
    println!("{} {}/{}: {}", d.kind, d.namespace, d.name, d.status);
    println!("replicas: {:?} suspended: {:?}", d.replicas, d.suspended);
    for (k, v) in &d.info {
        println!("  {k}: {v}");
    }
    for c in &d.conditions {
        println!("  condition {} = {} ({})", c.type_, c.status, c.reason);
    }
    for j in &d.jobs {
        println!("  job {} {} {} manual={} started {:?} finished {:?}", j.name, j.status, j.completions, j.manual, j.started, j.finished);
    }
    println!("cpu: {:?}\nmemory: {:?}\nmetrics: {}", d.cpu, d.memory, d.metrics_available);
    for p in &d.pods {
        println!(
            "  pod {} {} ready {} restarts {} updated {} finished {} cpu {:?} mem {:?}",
            p.name, p.status, p.ready, p.restarts, p.updated, p.finished, p.cpu, p.memory
        );
    }
    for c in &d.containers {
        let env: Vec<&str> = c.env.iter().map(|e| e.name.as_str()).collect();
        println!("  container {} init={} image={} env={env:?}", c.name, c.init, c.image);
    }
    println!("events: {}", d.events.len());
    for e in d.events.iter().take(5) {
        println!("  {} {} {:?}: {}", e.type_, e.reason, e.object, e.message);
    }
    let history = HistoryStore::default().workload_history(&client, &info.kind, namespace, name).await?;
    println!(
        "history: {:?}, {} cpu points, containers {:?}",
        history.source,
        history.pod.cpu.len(),
        history.containers.iter().map(|c| &c.name).collect::<Vec<_>>()
    );
    Ok(())
}
