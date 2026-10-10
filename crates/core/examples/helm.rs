//! Read-only: `cargo run -p subduer-core --example helm -- <context> <namespace> <release>`
//! prints the Helm release summary the detail panel shows (no values or manifest).

use subduer_core::{contexts, helm};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [context, namespace, release] = &args[..] else {
        eprintln!("usage: helm <context> <namespace> <release>");
        std::process::exit(2);
    };
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    print!("{}", helm::release_yaml(&client, namespace, release).await?);
    Ok(())
}
