//! Server-side dry run only, never changes anything:
//! `cargo run -p subduer-core --example config_dry_run -- <context> <core/ConfigMap|core/Secret|autoscaling/HorizontalPodAutoscaler|core/PersistentVolumeClaim|core/PersistentVolume> <namespace|-> <name>`
//! ConfigMap/Secret: checks a save adding the key `subduer-dry-run` (no
//! existing value is read or sent). HPA: checks setting its current range.
//! PVC: checks growing it by 1Gi. PV: checks setting its current reclaim policy.
//! ExternalSecret/ClusterExternalSecret (`external-secrets.io/…`): checks Force sync.
//! ScaledObject/ScaledJob (`keda.sh/…`): checks pausing (and for a ScaledObject,
//! pausing at its current replicas), resuming and setting its current range.
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
    // A fresh gate is locked; dry runs don't need it unlocked.
    let gate = SafetyGate::default();
    let ns = (namespace != "-").then_some(namespace.as_str());
    let d = config::object_details(&client, kind, ns, name).await?;
    let row = |k: &str| d.info.iter().find(|r| r.key == k).map(|r| r.value.clone());
    if key == "core/PersistentVolumeClaim" {
        let current = row("Requested").unwrap_or_default();
        let gib = subduer_core::quantity::parse(&current).unwrap_or(0.0) / 1024f64.powi(3);
        let bigger = format!("{}Gi", gib.ceil() as i64 + 1);
        println!("can expand: {:?}", row("Can expand"));
        match actions::expand_claim(&client, &gate, context, namespace, name, &bigger, true).await {
            Ok(()) => println!("expand {current} -> {bigger} dry run OK"),
            Err(e) => println!("expand dry run refused: {}", e.user_message()),
        }
        let gated = actions::expand_claim(&client, &gate, context, namespace, name, &bigger, false).await;
        println!("without unlocking: {}", gated.err().map_or("NOT refused (bug)".into(), |e| e.user_message()));
        return Ok(());
    }
    if subduer_core::types::can_force_sync(kind) {
        match subduer_core::external_secrets::force_sync(&client, &gate, context, kind, ns, name, true).await {
            Ok(n) => println!("force sync dry run OK ({n} ExternalSecrets)"),
            Err(e) => println!("force sync dry run refused: {}", e.user_message()),
        }
        let gated = subduer_core::external_secrets::force_sync(&client, &gate, context, kind, ns, name, false).await;
        println!("without unlocking: {}", gated.err().map_or("NOT refused (bug)".into(), |e| e.user_message()));
        return Ok(());
    }
    if subduer_core::types::is_keda_scaler(kind) {
        use subduer_core::keda;
        let range = row("Range").or_else(|| row("Jobs at once")).unwrap_or_default();
        let mut parts = range.split_whitespace();
        let lo: i32 = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        let hi: i32 = parts.nth(1).and_then(|n| n.parse().ok()).unwrap_or(1);
        let now = row("Replicas").and_then(|r| r.split_whitespace().next()?.parse().ok());
        let ns = namespace.as_str();
        let report = |what: &str, r: subduer_core::Result<()>| match r {
            Ok(()) => println!("{what} dry run OK"),
            Err(e) => println!("{what} dry run refused: {}", e.user_message()),
        };
        report("pause", keda::set_paused(&client, &gate, context, kind, ns, name, true, None, true).await);
        if kind.kind == "ScaledObject" {
            report(&format!("pause at {now:?}"), keda::set_paused(&client, &gate, context, kind, ns, name, true, now.or(Some(1)), true).await);
        }
        report("resume", keda::set_paused(&client, &gate, context, kind, ns, name, false, None, true).await);
        report(&format!("range {lo}-{hi}"), keda::set_replica_range(&client, &gate, context, kind, ns, name, lo, hi, true).await);
        let gated = keda::set_paused(&client, &gate, context, kind, ns, name, true, None, false).await;
        println!("pause without unlocking: {}", gated.err().map_or("NOT refused (bug)".into(), |e| e.user_message()));
        let gated = keda::set_replica_range(&client, &gate, context, kind, ns, name, lo, hi, false).await;
        println!("range without unlocking: {}", gated.err().map_or("NOT refused (bug)".into(), |e| e.user_message()));
        return Ok(());
    }
    if key == "core/PersistentVolume" {
        let policy = row("Reclaim policy").and_then(|p| p.split(':').next().map(str::to_owned)).unwrap_or_default();
        match actions::set_reclaim_policy(&client, &gate, context, name, &policy, true).await {
            Ok(()) => println!("reclaim policy {policy} dry run OK"),
            Err(e) => println!("reclaim dry run refused: {}", e.user_message()),
        }
        let gated = actions::set_reclaim_policy(&client, &gate, context, name, &policy, false).await;
        println!("without unlocking: {}", gated.err().map_or("NOT refused (bug)".into(), |e| e.user_message()));
        return Ok(());
    }
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
