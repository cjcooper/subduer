//! Server-side dry run only, never changes anything:
//! `cargo run -p subduer-core --example config_dry_run -- <context> <core/ConfigMap|core/Secret|autoscaling/HorizontalPodAutoscaler> <namespace> <name>`
//! ConfigMap/Secret: checks a save adding the key `subduer-dry-run` (no
//! existing value is read or sent). HPA: checks setting its current range.
//! Then checks that the locked gate refuses the real call.

use subduer_core::safety::SafetyGate;
use subduer_core::types::DataChange;
use subduer_core::{actions, cluster, config, contexts};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [context, key, namespace, name] = &args[..] else {
        eprintln!("usage: config_dry_run <context> <group/Kind> <namespace> <name>");
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
    let d = config::object_details(&client, kind, Some(namespace), name).await?;
    // A fresh gate is locked; dry runs don't need it unlocked.
    let gate = SafetyGate::default();
    if let Some(data) = &d.data {
        let change = [DataChange { key: "subduer-dry-run".into(), value: Some("check".into()) }];
        let rv = &data.resource_version;
        match actions::save_data(&client, &gate, context, kind, namespace, name, rv, &change, true).await {
            Ok(()) => println!("save dry run OK"),
            Err(e) => println!("save dry run refused: {}", e.user_message()),
        }
        match actions::save_data(&client, &gate, context, kind, namespace, name, "1", &change, true).await {
            Ok(()) => println!("stale version NOT refused (bug)"),
            Err(e) => println!("stale version refused: {}", e.user_message()),
        }
        let gated = actions::save_data(&client, &gate, context, kind, namespace, name, rv, &change, false).await;
        println!("without unlocking: {}", gated.err().map_or("NOT refused (bug)".into(), |e| e.user_message()));
    } else {
        let get = |k: &str| d.info.iter().find(|r| r.key == k).and_then(|r| r.value.parse::<i32>().ok());
        let (Some(min), Some(max)) = (get("Min replicas"), get("Max replicas")) else {
            eprintln!("not an HPA, or no range found");
            std::process::exit(2);
        };
        match actions::set_hpa_range(&client, &gate, context, kind, namespace, name, min, max, true).await {
            Ok(()) => println!("range {min}..{max} dry run OK"),
            Err(e) => println!("range dry run refused: {}", e.user_message()),
        }
        let gated = actions::set_hpa_range(&client, &gate, context, kind, namespace, name, min, max, false).await;
        println!("without unlocking: {}", gated.err().map_or("NOT refused (bug)".into(), |e| e.user_message()));
    }
    Ok(())
}
