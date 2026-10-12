//! Packet capture of one pod, for docker-desktop or UAT only, never production:
//! `cargo run -p subduer-core --example capture_check -- <context> <ns> <pod> <image> [--go] [--secs N] [--all-tcp] [--out file.pcap]`
//!
//! Without `--go`: a server-side dry run of attaching the capture container
//! (RBAC, admission, the pod), and a check that the locked gate refuses the
//! real thing. Changes nothing.
//!
//! With `--go`: CHANGES THE POD. Attaches an ephemeral capture container
//! (no restart; it sleeps for the capture plus a minute, then exits, but stays
//! listed in the pod until the pod is replaced), waits for it to run, captures
//! for `--secs` (default 30) into `--out` (default `capture-<pod>.pcap` here),
//! TLS handshakes only unless `--all-tcp`, and reports what tcpdump said.

use std::time::Duration;

use subduer_core::capture::{self, Filter, Traffic};
use subduer_core::safety::{self, SafetyGate};
use subduer_core::contexts;

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [context, namespace, pod, image, rest @ ..] = &args[..] else { usage() };
    if safety::is_prod_like(context) {
        eprintln!("refusing: {context} looks like production");
        std::process::exit(2);
    }
    let flag = |name: &str| rest.iter().any(|a| a == name);
    let value = |name: &str| rest.iter().position(|a| a == name).and_then(|i| rest.get(i + 1)).cloned();
    let go = flag("--go");
    let secs: u32 = value("--secs").map_or(30, |s| s.parse().unwrap_or_else(|_| usage()));
    let filter = Filter { traffic: Traffic::All, handshakes_only: !flag("--all-tcp") };
    let out = value("--out").unwrap_or_else(|| format!("capture-{pod}.pcap"));

    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let gate = SafetyGate::default();
    let lives = secs + 60;
    match capture::attach(&client, &gate, context, namespace, pod, image, lives, true).await {
        Ok(_) => println!("attach dry run OK"),
        Err(e) => {
            println!("attach dry run refused: {}", e.user_message());
            std::process::exit(1);
        }
    }
    let locked = capture::attach(&client, &gate, context, namespace, pod, image, lives, false).await;
    println!("without unlocking: {}", locked.err().map_or("NOT refused (bug)".into(), |e| e.user_message()));
    if !go {
        println!("dry run only; add --go to attach and capture");
        return Ok(());
    }

    gate.set_unlocked(context, true);
    let container = capture::attach(&client, &gate, context, namespace, pod, image, lives, false).await?;
    println!("attached {container}; waiting for it to run");
    let image_id = capture::wait_running(&client, namespace, pod, &container, Duration::from_secs(180)).await?;
    println!("running {image_id}");
    let expression = capture::expression(&filter, &capture::pod_ips(&client, namespace, pod).await.unwrap_or_default());
    println!("capturing for {secs} s ({expression}) into {out}");
    let mut file = tokio::fs::File::create(&out).await.map_err(|e| subduer_core::Error::Message(e.to_string()))?;
    let captured = capture::run(&client, namespace, pod, &container, &expression, secs, &mut file, |_| {}).await?;
    println!("{} bytes of pcap", captured.bytes);
    println!("tcpdump said: {}", if captured.messages.is_empty() { "(nothing)" } else { &captured.messages });
    let head = tokio::fs::read(&out).await.map(|b| b.get(..4).map(<[u8]>::to_vec)).ok().flatten();
    let pcap = matches!(head.as_deref(), Some([0xd4, 0xc3, 0xb2, 0xa1] | [0xa1, 0xb2, 0xc3, 0xd4] | [0x4d, 0x3c, 0xb2, 0xa1] | [0xa1, 0xb2, 0x3c, 0x4d]));
    println!("pcap header: {}", if pcap { "yes" } else { "NO" });
    println!("{container} exits by itself within a minute; it stays listed in the pod until the pod is replaced");
    Ok(())
}

fn usage() -> ! {
    eprintln!("usage: capture_check <context> <ns> <pod> <image> [--go] [--secs N] [--all-tcp] [--out file.pcap]");
    std::process::exit(2);
}
