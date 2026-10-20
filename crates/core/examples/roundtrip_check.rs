//! Uploads pseudo-random files of many sizes to /tmp, checks `wc -c` in the
//! container, downloads each back and compares every byte, then deletes what
//! it wrote. docker-desktop or a throwaway cluster only, never production:
//! `cargo run -p subduer-core --example roundtrip_check -- <context> <ns> <pod> <container> [MiB...]`

use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, AttachParams};
use subduer_core::types::{UploadOutcome, UploadTarget};
use subduer_core::{contexts, download, safety, upload};
use tokio::io::AsyncReadExt;

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [context, namespace, pod, container, rest @ ..] = &args[..] else {
        eprintln!("usage: roundtrip_check <context> <ns> <pod> <container> [MiB...]");
        std::process::exit(2);
    };
    if safety::is_prod_like(context) {
        eprintln!("refusing: {context} looks like production");
        std::process::exit(2);
    }
    let mut sizes: Vec<usize> = vec![0, 1, 1023, 1024, 1025, 4096, 65_535, 65_536, 65_537, 1_000_000, 4 << 20, 16 << 20];
    sizes.extend(rest.iter().map(|m| m.parse::<usize>().expect("MiB") << 20));
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let local = std::env::temp_dir().join(format!("subduer-roundtrip-{}", std::process::id()));
    std::fs::create_dir_all(&local).expect("temp dir");
    let mut failed = false;
    for size in sizes {
        let data = noise(size);
        let target = UploadTarget {
            context: context.clone(),
            namespace: namespace.clone(),
            pod: pod.clone(),
            container: container.clone(),
            path: "/tmp/".into(),
            file_name: format!("subduer-roundtrip-{size}"),
            overwrite: true,
        };
        let started = std::time::Instant::now();
        let path = match upload::upload(client.clone(), &target, &data).await {
            Ok(UploadOutcome::Uploaded { path, .. }) => path,
            other => {
                println!("{size:>10}: upload failed: {:?}", other.map_err(|e| e.user_message()));
                failed = true;
                continue;
            }
        };
        let up = started.elapsed();
        let ap = AttachParams::default().container(container.clone()).stderr(false);
        let mut p = pods.exec(pod, ["sh", "-c", r#"wc -c < "$1""#, "sh", path.as_str()], &ap).await?;
        let mut remote = String::new();
        p.stdout().unwrap().read_to_string(&mut remote).await.ok();
        let dest = local.join(format!("{size}"));
        let back = download::download(client.clone(), namespace, pod, container, &path, &dest).await;
        let got = std::fs::read(&dest).unwrap_or_default();
        let ok = remote.trim() == size.to_string() && got == data && back.as_ref().ok() == Some(&(size as u64));
        failed |= !ok;
        println!(
            "{size:>10}: {} remote={} downloaded={:?} ({up:.1?} up, {:.1?} total)",
            if ok { "ok  " } else { "FAIL" },
            remote.trim(),
            back.map_err(|e| e.user_message()),
            started.elapsed()
        );
    }
    let mut p = pods.exec(pod, ["sh", "-c", "rm -f /tmp/subduer-roundtrip-*"], &AttachParams::default().container(container.clone())).await?;
    let _ = p.take_status().unwrap().await;
    let _ = std::fs::remove_dir_all(&local);
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

/// Deterministic bytes that don't compress or repeat (xorshift).
fn noise(len: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15 ^ len as u64;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}
