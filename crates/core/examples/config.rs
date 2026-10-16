//! Read-only: `cargo run -p subduer-core --example config -- <context> <group/Kind> <namespace|-> <name>`
//! Prints the Details tab data of a Config-section kind. ConfigMap values are
//! shown only by size, and Secret values are never fetched.

use subduer_core::{cluster, config, contexts};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [context, key, namespace, name] = &args[..] else {
        eprintln!("usage: config <context> <group/Kind> <namespace|-> <name>");
        std::process::exit(2);
    };
    let namespace = (namespace != "-").then_some(namespace.as_str());
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let kinds = cluster::discover_kinds(&client).await?;
    let Some(info) = kinds.iter().find(|k| k.kind.key() == *key) else {
        eprintln!("kind {key} not found");
        std::process::exit(2);
    };
    println!("served as {}", info.kind.api_version());
    let d = config::object_details(&client, &info.kind, namespace, name).await?;
    println!("{} {}: status {:?}", d.kind, d.name, d.status);
    for r in &d.info {
        println!("  {}: {}{}", r.key, r.value, r.link.as_ref().map(|l| format!("  → {} {}", l.kind, l.name)).unwrap_or_default());
    }
    for c in &d.conditions {
        println!("  condition {} = {} ({})", c.type_, c.status, c.reason);
    }
    if let Some(data) = &d.data {
        println!("data: secret={} rv={} read_only={:?} warning={:?}", data.secret, data.resource_version, data.read_only, data.warning);
        for e in &data.entries {
            println!("  key {} size {} binary {} value sent: {}", e.key, e.size, e.binary, e.value.is_some());
        }
    }
    for s in &d.sections {
        println!("section {} [{}]: {}", s.title, s.id, s.summary);
        for r in &s.rows {
            println!("    {}: {}", r.key, r.value);
        }
        if let Some(t) = &s.table {
            println!("    | {} |", t.columns.join(" | "));
            for row in t.rows.iter().take(8) {
                println!("    | {} | fill {:?}", row.cells.join(" | "), row.fill);
            }
        }
    }
    println!("events: {}", d.events.len());
    for e in d.events.iter().take(3) {
        println!("  {} {} {}", e.type_, e.reason, e.message);
    }
    if let Some(logs) = &d.logs {
        println!("logs: {}", logs.title(context));
    }
    Ok(())
}
