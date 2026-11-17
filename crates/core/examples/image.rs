//! Read-only: `cargo run -p subduer-core --example image -- <context> <namespace> <pod> <container>`
//! prints the container's image metadata, read from its registry.

use subduer_core::{contexts, image};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [context, namespace, pod, container] = &args[..] else {
        eprintln!("usage: image <context> <namespace> <pod> <container>");
        std::process::exit(2);
    };
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let env = image::RegistryEnv::system(image::aws_profile(&kubeconfig, context));
    let m = image::image_metadata(&client, namespace, pod, container, &env).await?;
    println!("{} via {}", m.reference, m.credentials);
    println!("digest {} index {:?} id {}", m.digest, m.index_digest, m.id);
    println!("{} created {:?} author {:?} docker {:?} size {}", m.platform, m.created, m.author, m.docker_version, m.size);
    println!("entrypoint {:?} cmd {:?} workdir {:?} user {:?}", m.entrypoint, m.cmd, m.working_dir, m.user);
    println!("ports {:?} volumes {:?} stop {:?}", m.exposed_ports, m.volumes, m.stop_signal);
    println!("env {:?}", m.env.iter().map(|e| e.split('=').next().unwrap_or_default()).collect::<Vec<_>>());
    for (k, v) in &m.labels {
        println!("label {k}={v}");
    }
    for h in &m.history {
        println!("{:>10} {}", h.size.map(|s| s.to_string()).unwrap_or_default(), h.created_by.chars().take(100).collect::<String>());
    }
    Ok(())
}
