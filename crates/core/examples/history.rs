//! Read-only: `cargo run -p subduer-core --example history -- <context> <namespace> <pod>`
//! prints a pod's last-hour CPU and memory summary.

use subduer_core::contexts;
use subduer_core::history::HistoryStore;

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [context, namespace, pod] = &args[..] else {
        eprintln!("usage: history <context> <namespace> <pod>");
        std::process::exit(2);
    };
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let store = HistoryStore::default();
    println!("prometheus: {:?}", store.prometheus(&client).await);
    let h = store.pod_history(&client, namespace, pod).await?;
    println!("source {:?} window {}..{} step {}", h.source, h.start, h.end, h.step);
    let summary = |s: &[(f64, f64)]| {
        let max = s.iter().map(|p| p.1).fold(0.0, f64::max);
        format!("{} points, last {:?}, max {max:.1}", s.len(), s.last())
    };
    println!("pod cpu: {}\npod mem: {}", summary(&h.pod.cpu), summary(&h.pod.memory));
    for c in &h.containers {
        println!("  {} cpu: {} | mem: {}", c.name, summary(&c.usage.cpu), summary(&c.usage.memory));
    }
    Ok(())
}
