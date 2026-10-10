//! Server-side dry run only, never changes anything:
//! `cargo run -p subduer-core --example edit_dry_run -- <context> <group/Kind> <namespace|-> <name>`
//! Loads the object's YAML as the edit window does, then has the server
//! dry-run: saving it unchanged, saving it with an added annotation, and
//! saving it with a stale resourceVersion (should be refused as a conflict).
//! Then checks that the locked gate refuses the real save. Prints no YAML.

use subduer_core::safety::SafetyGate;
use subduer_core::{cluster, contexts, edit};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [context, key, namespace, name] = &args[..] else {
        eprintln!("usage: edit_dry_run <context> <group/Kind> <namespace|-> <name>");
        std::process::exit(2);
    };
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let kinds = cluster::discover_kinds(&client).await?;
    let Some(info) = kinds.iter().find(|k| k.kind.key() == *key) else {
        eprintln!("kind {key} not found");
        std::process::exit(2);
    };
    let kind = &info.kind;
    // A fresh gate is locked; dry runs don't need it unlocked.
    let gate = SafetyGate::default();
    let ns = (namespace != "-").then_some(namespace.as_str());
    let yaml = edit::load(&client, kind, ns, name).await?;
    println!("loaded {} lines", yaml.lines().count());

    let report = |what: &str, r: subduer_core::Result<String>| match r {
        Ok(saved) => println!("{what}: dry run OK ({} lines back)", saved.lines().count()),
        Err(e) => println!("{what}: refused: {}", e.user_message()),
    };
    report("unchanged", edit::save(&client, &gate, context, kind, ns, name, &yaml, true).await);

    let mut doc: serde_json::Value = serde_norway::from_str(&yaml)?;
    doc["metadata"]["annotations"]["subduer-dry-run"] = "1".into();
    let annotated = serde_norway::to_string(&doc)?;
    report("annotated", edit::save(&client, &gate, context, kind, ns, name, &annotated, true).await);

    doc["metadata"]["resourceVersion"] = "1".into();
    let stale = serde_norway::to_string(&doc)?;
    report("stale resourceVersion", edit::save(&client, &gate, context, kind, ns, name, &stale, true).await);

    let gated = edit::save(&client, &gate, context, kind, ns, name, &annotated, false).await;
    println!("without unlocking: {}", gated.err().map_or("NOT refused (bug)".into(), |e| e.user_message()));
    Ok(())
}
