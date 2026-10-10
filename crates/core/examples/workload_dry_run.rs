//! Server-side dry run only, never changes anything:
//! `cargo run -p subduer-core --example workload_dry_run -- <context> <group/Kind> <namespace> <name> [replicas]`
//! Deployments and StatefulSets: reads the replicas, then checks that the API
//! server would accept a rollout restart and, given a count, a scale.
//! Jobs and CronJobs: checks a suspend (or resume, if suspended) and, for a
//! CronJob, a Run now. Each check covers RBAC, admission and existence.

use subduer_core::safety::SafetyGate;
use subduer_core::{actions, cluster, contexts};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (context, key, namespace, name, replicas) = match &args[..] {
        [c, k, ns, n] => (c, k, ns, n, None),
        [c, k, ns, n, r] => (c, k, ns, n, r.parse::<i32>().ok()),
        _ => {
            eprintln!("usage: workload_dry_run <context> <group/Kind> <namespace> <name> [replicas]");
            std::process::exit(2);
        }
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
    if subduer_core::types::is_batch_workload(kind) {
        let d = subduer_core::workload::workload_details(&client, kind, namespace, name).await?;
        let suspend = d.suspended != Some(true);
        println!("{}: {} (suspended: {:?})", d.kind, d.status, d.suspended);
        match actions::set_suspended(&client, &gate, context, kind, namespace, name, suspend, true).await {
            Ok(()) => println!("{} dry run OK", if suspend { "suspend" } else { "resume" }),
            Err(e) => println!("suspend dry run refused: {}", e.user_message()),
        }
        if kind.kind == "CronJob" {
            match actions::run_cron_job(&client, &gate, context, namespace, name, true).await {
                Ok(job) => println!("run now dry run OK: would create Job {job}"),
                Err(e) => println!("run now dry run refused: {}", e.user_message()),
            }
            let gated = actions::run_cron_job(&client, &gate, context, namespace, name, false).await;
            println!("without unlocking: {}", gated.err().map_or("NOT refused (bug)".into(), |e| e.user_message()));
        }
        let gated = actions::set_suspended(&client, &gate, context, kind, namespace, name, suspend, false).await;
        println!("without unlocking: {}", gated.err().map_or("NOT refused (bug)".into(), |e| e.user_message()));
        return Ok(());
    }
    let r = actions::replicas(&client, kind, namespace, name).await?;
    println!("replicas: {} desired, {} running", r.desired, r.current);
    match actions::restart(&client, &gate, context, kind, namespace, name, true).await {
        Ok(()) => println!("restart dry run OK"),
        Err(e) => println!("restart dry run refused: {}", e.user_message()),
    }
    if let Some(n) = replicas {
        match actions::scale(&client, &gate, context, kind, namespace, name, n, true).await {
            Ok(()) => println!("scale to {n} dry run OK"),
            Err(e) => println!("scale dry run refused: {}", e.user_message()),
        }
    }
    let gated = actions::restart(&client, &gate, context, kind, namespace, name, false).await;
    println!("without unlocking: {}", gated.err().map_or("NOT refused (bug)".into(), |e| e.user_message()));
    Ok(())
}
