//! Browses, uploads to /tmp several ways, checks the error paths, then deletes
//! what it wrote. docker-desktop or UAT only, never production:
//! `cargo run -p subduer-core --example upload_check -- <context> <ns> <pod> <container>`

use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, AttachParams};
use subduer_core::types::UploadTarget;
use subduer_core::{contexts, safety, upload};
use tokio::io::AsyncReadExt;

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [context, namespace, pod, container] = &args[..] else {
        eprintln!("usage: upload_check <context> <ns> <pod> <container>");
        std::process::exit(2);
    };
    if safety::is_prod_like(context) {
        eprintln!("refusing: {context} looks like production");
        std::process::exit(2);
    }
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;

    let root = upload::browse(client.clone(), namespace, pod, container, "/", true).await;
    match &root {
        Ok(l) => println!(
            "browse /: writable={} entries={} (e.g. {:?}) suggestions={:?}",
            l.writable,
            l.entries.len(),
            l.entries.iter().take(4).map(|e| format!("{}{}{}", e.name, if e.dir { "/" } else { "" }, if e.writable { " w" } else { "" })).collect::<Vec<_>>(),
            l.suggestions
        ),
        Err(e) => println!("browse /: {}", e.user_message()),
    }

    let target = |path: &str, overwrite: bool| UploadTarget {
        context: context.clone(),
        namespace: namespace.clone(),
        pod: pod.clone(),
        container: container.clone(),
        path: path.to_owned(),
        file_name: "subduer check.txt".into(),
        overwrite,
    };
    let data = b"hello from k8s-subduer\n";
    for (label, path, overwrite) in [
        ("dir with slash", "/tmp/", false),
        ("dir without slash (exists)", "/tmp", false),
        ("dir without slash (replace)", "/tmp", true),
        ("explicit name", "/tmp/subduer renamed.txt", false),
        ("missing dir", "/no-such-dir/x.txt", false),
        ("read-only", "/subduer.txt", false),
    ] {
        let r = upload::upload(client.clone(), &target(path, overwrite), data).await;
        println!("{label:<28} {:?}", r.map_err(|e| e.user_message()));
    }

    let pods: Api<Pod> = Api::namespaced(client, namespace);
    let ap = AttachParams::default().container(container.clone()).stderr(false);
    let cleanup = r#"ls -1 /tmp | grep subduer; rm -f "/tmp/subduer check.txt" "/tmp/subduer renamed.txt"; ls /tmp | grep -c subduer"#;
    let mut p = pods.exec(pod, ["sh", "-c", cleanup], &ap).await?;
    let mut out = String::new();
    p.stdout().unwrap().read_to_string(&mut out).await.ok();
    println!("written then cleaned up: {}", out.trim().replace('\n', " | "));
    Ok(())
}
