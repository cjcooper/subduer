//! Read-only: downloads files from a container into a local folder and
//! compares sizes with `wc -c` in the container.
//! `cargo run -p subduer-core --example download_check -- <context> <ns> <pod> <container> <local-dir> <path>...`

use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, AttachParams};
use subduer_core::{contexts, download};
use tokio::io::AsyncReadExt;

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (context, namespace, pod, container, dir) = (&args[0], &args[1], &args[2], &args[3], std::path::Path::new(&args[4]));
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    for path in &args[5..] {
        let name = path.rsplit('/').next().unwrap_or("download");
        let dest = download::unique_path(dir, name);
        match download::download(client.clone(), namespace, pod, container, path, &dest).await {
            Ok(bytes) => {
                let ap = AttachParams::default().container(container.clone()).stderr(false);
                let mut p = pods.exec(pod, ["sh", "-c", r#"wc -c < "$1""#, "sh", path], &ap).await?;
                let mut out = String::new();
                p.stdout().unwrap().read_to_string(&mut out).await.ok();
                let local = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
                println!("{path}: {bytes} bytes -> {} (local {local}, remote {})", dest.display(), out.trim());
            }
            Err(e) => println!("{path}: {}", e.user_message()),
        }
    }
    Ok(())
}
