//! Opens a shell, runs `echo` and `uname`, and exits; changes nothing.
//! For docker-desktop or UAT only, never production:
//! `cargo run -p subduer-core --example exec_check -- <context> <ns> <pod> <container> [input]`

use std::time::Duration;

use base64::Engine;
use subduer_core::types::{ExecOutput, ExecTarget};
use subduer_core::{contexts, exec, safety};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(context), Some(namespace), Some(pod), Some(container)) = (args.first(), args.get(1), args.get(2), args.get(3)) else {
        eprintln!("usage: exec_check <context> <ns> <pod> <container>");
        std::process::exit(2);
    };
    if safety::is_prod_like(context) {
        eprintln!("refusing: {context} looks like production");
        std::process::exit(2);
    }
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let target = ExecTarget { namespace: namespace.clone(), pod: pod.clone(), container: container.clone() };
    let (input, input_rx) = tokio::sync::mpsc::unbounded_channel();
    let (_resize, resize_rx) = tokio::sync::mpsc::unbounded_channel();
    let typed = args.get(4).map_or("echo subduer-exec-ok; uname -s; exit", String::as_str);
    input.send(format!("{typed}\n").into_bytes()).unwrap();
    let print = |out: ExecOutput| match out {
        ExecOutput::Data(b64) => {
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
            print!("{}", String::from_utf8_lossy(&bytes));
        }
        ExecOutput::Exited(msg) => println!("\n[exited] {msg}"),
    };
    let run = exec::run(client, &target, (100, 30), input_rx, resize_rx, print);
    match tokio::time::timeout(Duration::from_secs(15), run).await {
        Ok(Err(e)) => println!("error: {}", e.user_message()),
        Err(_) => println!("timed out"),
        Ok(Ok(ended)) => println!("\n[ended] {ended:?}"),
    }
    Ok(())
}
