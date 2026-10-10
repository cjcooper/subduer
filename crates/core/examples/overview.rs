//! Read-only: `cargo run -p subduer-core --example overview -- <context>`
//! prints the Cluster page data (`--json` for the raw payload).

use subduer_core::history::HistoryStore;
use subduer_core::{contexts, overview};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let context = std::env::args().nth(1).unwrap_or_else(|| "docker-desktop".into());
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, &context).await?;
    let o = overview::cluster_overview(&client, &HistoryStore::default()).await?;
    if std::env::args().any(|a| a == "--json") {
        println!("{}", serde_json::to_string(&o).unwrap());
        return Ok(());
    }
    let gib = |b: f64| b / 1024f64.powi(3);
    println!("version {:?}  usage {:?}  warnings(1h) {:?}", o.server_version, o.usage_source, o.warnings_last_hour);
    if let Some(h) = &o.history {
        let last = |s: &[(f64, f64)]| s.last().map(|p| p.1);
        println!("history: {} cpu points (last {:?}m), {} memory points (last {:?} GiB)",
            h.usage.cpu.len(), last(&h.usage.cpu), h.usage.memory.len(), last(&h.usage.memory).map(gib));
    }
    println!("pods {:?}", o.pods);
    println!(
        "memory alloc {:.1} GiB  req {:.1}  lim {:.1}  used {:?}",
        gib(o.memory.allocatable),
        gib(o.memory.requests),
        gib(o.memory.limits),
        o.memory.used.map(gib)
    );
    println!("cpu alloc {:.0}m  req {:.0}m  lim {:.0}m  used {:?}", o.cpu.allocatable, o.cpu.requests, o.cpu.limits, o.cpu.used);
    println!("{} nodes, e.g. {:?}", o.nodes.len(), o.nodes.first());
    for ns in o.namespaces.iter().take(5) {
        println!("  {:<24} pods {:>3}  mem {:.2} GiB  cpu {:.0}m", ns.namespace, ns.pods, gib(ns.memory), ns.cpu);
    }
    Ok(())
}
