//! Server-side dry run only, never changes anything:
//! `cargo run -p subduer-core --example helm_rollback_dry_run -- <context> <namespace> <release> <revision>`
//! Runs `helm rollback --dry-run=server` (Helm 4) or `--dry-run` (Helm 3), then
//! checks that the locked gate refuses the real rollback.

use subduer_core::helm;
use subduer_core::safety::SafetyGate;

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [context, namespace, release, revision] = &args[..] else {
        eprintln!("usage: helm_rollback_dry_run <context> <namespace> <release> <revision>");
        std::process::exit(2);
    };
    let revision: i64 = revision.parse().unwrap_or(0);
    let gate = SafetyGate::default();
    match helm::rollback(&gate, context, namespace, release, revision, false, true).await {
        Ok(out) => println!("dry run OK:\n{out}"),
        Err(e) => println!("dry run refused: {}", e.user_message()),
    }
    let gated = helm::rollback(&gate, context, namespace, release, revision, false, false).await;
    println!("without unlocking: {}", gated.err().map_or("NOT refused (bug)".into(), |e| e.user_message()));
    Ok(())
}
