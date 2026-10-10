//! Read-only: `cargo run -p subduer-core --example access_check -- <context> <namespace> <serviceaccount> <verb> <resource> [namespace|-]`
//! Asks the API server whether a ServiceAccount may do something
//! (SubjectAccessReview). Nothing is changed.

use subduer_core::{contexts, rbac};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (context, sa_ns, sa, verb, resource, ns) = match &args[..] {
        [c, n, s, v, r] => (c, n, s, v, r, n.clone()),
        [c, n, s, v, r, ns] => (c, n, s, v, r, if ns == "-" { String::new() } else { ns.clone() }),
        _ => {
            eprintln!("usage: access_check <context> <namespace> <serviceaccount> <verb> <resource> [namespace|-]");
            std::process::exit(2);
        }
    };
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let r = rbac::check_access(&client, sa_ns, sa, verb, resource, &ns, "").await?;
    let place = if ns.is_empty() { "all namespaces".to_owned() } else { ns };
    println!("{sa_ns}/{sa} {verb} {resource} in {place}: {} {}", if r.allowed { "ALLOWED" } else { "denied" }, r.reason);
    Ok(())
}
