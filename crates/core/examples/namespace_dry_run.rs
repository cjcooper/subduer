//! Server-side dry run only, never changes anything:
//! `cargo run -p subduer-core --example namespace_dry_run -- <context> <new-name>`
//! Checks the API server would accept creating the namespace, then that the
//! locked gate refuses the real call.

use subduer_core::safety::SafetyGate;
use subduer_core::{contexts, namespace};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [context, name] = &args[..] else {
        eprintln!("usage: namespace_dry_run <context> <new-name>");
        std::process::exit(2);
    };
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let gate = SafetyGate::default();
    match namespace::create_namespace(&client, &gate, context, name, true).await {
        Ok(()) => println!("create {name} dry run OK"),
        Err(e) => println!("create dry run refused: {}", e.user_message()),
    }
    let gated = namespace::create_namespace(&client, &gate, context, name, false).await;
    println!("without unlocking: {}", gated.err().map_or("NOT refused (bug)".into(), |e| e.user_message()));
    Ok(())
}
