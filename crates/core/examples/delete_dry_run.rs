//! Server-side dry run only, never deletes:
//! `cargo run -p subduer-core --example delete_dry_run -- <context> <group/Kind> <namespace|-> <name>`
//! Checks that the API server would accept the delete (RBAC, admission, existence).

use subduer_core::safety::SafetyGate;
use subduer_core::{actions, cluster, contexts};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [context, key, namespace, name] = &args[..] else {
        eprintln!("usage: delete_dry_run <context> <group/Kind> <namespace|-> <name>");
        std::process::exit(2);
    };
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let kinds = cluster::discover_kinds(&client).await?;
    let Some(info) = kinds.iter().find(|k| k.kind.key() == *key) else {
        eprintln!("kind {key} not found");
        std::process::exit(2);
    };
    let namespace = (namespace != "-").then_some(namespace.as_str());
    // A fresh gate is locked; dry runs don't need it unlocked.
    let gate = SafetyGate::default();
    match actions::delete(&client, &gate, context, &info.kind, namespace, name, true).await {
        Ok(()) => println!("dry run OK: the server would delete {key} {name}"),
        Err(e) => println!("dry run refused: {}", e.user_message()),
    }
    Ok(())
}
