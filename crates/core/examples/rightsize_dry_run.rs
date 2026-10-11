//! Server-side dry run only, never changes anything:
//! `cargo run -p subduer-core --example rightsize_dry_run -- <context> <Kind> <namespace> <name> <container>=<cpu m>/<memory Mi> …`
//! e.g. `… acme-uat Deployment shop web app=15/2944`. Prints the planned
//! requests and limits per container, the Helm release if any, the server's
//! dry-run answer and the values snippet, then shows that the real apply is
//! refused by the (locked) safety gate.

use subduer_core::contexts;
use subduer_core::rightsize;
use subduer_core::safety::SafetyGate;
use subduer_core::types::{ContainerRequests, RightsizeChange};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [context, kind, namespace, name, wanted @ ..] = &args[..] else { usage() };
    if wanted.is_empty() {
        usage();
    }
    let containers = wanted
        .iter()
        .map(|w| {
            let (container, values) = w.split_once('=').unwrap_or_else(|| usage());
            let (cpu, mem) = values.split_once('/').unwrap_or_else(|| usage());
            ContainerRequests {
                container: container.into(),
                cpu_request: cpu.parse().ok(),
                mem_request: mem.parse::<f64>().ok().map(|m| m * 1024.0 * 1024.0),
            }
        })
        .collect();
    let change = RightsizeChange { kind: kind.clone(), namespace: namespace.clone(), name: name.clone(), containers };
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let p = rightsize::preview(&client, &change).await?;
    println!("helm release: {:?}\neffect: {}", p.helm_release, p.effect);
    for c in &p.containers {
        if c.injected {
            println!("  {}: not in the pod template (injected)", c.container);
            continue;
        }
        let show = |q: &subduer_core::types::QuantityChange| format!("{:?} -> {:?}", q.before, q.after);
        println!(
            "  {}: cpu request {}, cpu limit {}, memory request {}, memory limit {}",
            c.container,
            show(&c.cpu_request),
            show(&c.cpu_limit),
            show(&c.mem_request),
            show(&c.mem_limit)
        );
    }
    match (&p.blocked, &p.dry_run_error) {
        (Some(b), _) => println!("blocked: {b}"),
        (None, Some(e)) => println!("dry run refused: {e}"),
        (None, None) => println!("dry run OK"),
    }
    if let Some(release) = &p.helm_release {
        println!("--- values snippet\n{}", rightsize::helm_snippet(release, &p));
    }
    // A fresh gate is locked, so this must be refused before any request.
    match rightsize::apply(&client, &SafetyGate::default(), context, &change).await {
        Ok(()) => println!("UNEXPECTED: the locked gate allowed a real change"),
        Err(e) => println!("real apply refused as expected: {}", e.user_message()),
    }
    Ok(())
}

fn usage() -> ! {
    eprintln!("usage: rightsize_dry_run <context> <Kind> <namespace> <name> <container>=<cpu m>/<memory Mi> …");
    std::process::exit(2);
}
