//! Read-only: `cargo run -p subduer-core --example pod -- <context> <namespace> <pod>`
//! prints the Details-tab data. Secret values are never fetched here.

use subduer_core::types::EnvValue;
use subduer_core::{contexts, pod_details};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [context, namespace, pod] = &args[..] else {
        eprintln!("usage: pod <context> <namespace> <pod>");
        std::process::exit(2);
    };
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let d = pod_details::pod_details(&client, namespace, pod).await?;
    println!("{} {} node={:?} qos={:?} owner={:?}", d.name, d.status, d.node, d.qos_class, d.owner);
    println!("metrics={}\ncpu {:?}\nmem {:?}", d.metrics_available, d.cpu, d.memory);
    for c in &d.containers {
        println!("-- {} init={} {:?} {} restarts={} cpu={:?}", c.name, c.init, c.state, c.detail, c.restarts, c.cpu);
        for e in &c.env {
            let v = match &e.value {
                EnvValue::Secret { name, key } => format!("<secret {name}/{key}>"),
                EnvValue::ConfigMap { name, key, value } => format!("cm {name}/{key} = {}", value.is_some()),
                other => format!("{:?}", std::mem::discriminant(other)),
            };
            println!("   {} {v} {:?}", e.name, e.from);
        }
    }
    for e in d.events.iter().take(5) {
        println!("event {} {} x{} {}", e.type_, e.reason, e.count, e.last_seen);
    }
    Ok(())
}
