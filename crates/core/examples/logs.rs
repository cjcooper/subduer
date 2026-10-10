//! Read-only: prints a few seconds of logs.
//! `cargo run -p subduer-core --example logs -- <context> <ns> pod <pod> <container>`
//! `cargo run -p subduer-core --example logs -- <context> <ns> <group/Kind> <name>`

use std::time::Duration;

use subduer_core::types::{LogSource, LogTarget};
use subduer_core::{cluster, contexts, logs};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (context, namespace) = (&args[0], &args[1]);
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let source = if args[2] == "pod" {
        LogSource::Container { pod: args[3].clone(), container: args[4].clone() }
    } else {
        let kinds = cluster::discover_kinds(&client).await?;
        let kind = kinds.into_iter().find(|k| k.kind.key() == args[2]).expect("kind").kind;
        LogSource::Workload { kind, name: args[3].clone() }
    };
    let target = LogTarget { namespace: namespace.clone(), source };
    println!("title: {}", target.title(context));
    let run = logs::stream(client, target, false, |b| {
        for n in &b.notices {
            println!("[notice] {n}");
        }
        for l in b.lines.iter().rev().take(3).rev() {
            println!("{:?} {:?} {}", l.source, l.time, l.text.chars().take(100).collect::<String>());
        }
        println!("[batch of {} lines]", b.lines.len());
    });
    let _ = tokio::time::timeout(Duration::from_secs(4), run).await;
    Ok(())
}
