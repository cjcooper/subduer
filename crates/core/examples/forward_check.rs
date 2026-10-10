//! Read-only: forwards a port, sends one HTTP GET through it, prints the reply.
//! `cargo run -p subduer-core --example forward_check -- <context> <ns> pod|service <name> <port> [path]`

use std::sync::{Arc, Mutex};

use subduer_core::forward::{self, ForwardStats};
use subduer_core::types::{ForwardRequest, ForwardSource};
use subduer_core::contexts;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (context, namespace, kind, name, port) = (&args[0], &args[1], &args[2], &args[3], args[4].parse::<u16>().unwrap());
    let path = args.get(5).map_or("/", String::as_str);
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let source = if kind == "pod" { ForwardSource::Pod { pod: name.clone() } } else { ForwardSource::Service { service: name.clone() } };
    println!("ports: {:?}", forward::port_options(&client, namespace, &source).await?);
    let req = ForwardRequest { namespace: namespace.clone(), source, remote_port: port, local_port: 0 };
    let (listener, backend) = forward::bind(&client, &req).await?;
    let local = listener.local_addr().unwrap();
    println!("listening on {local} -> {backend:?}");
    let stats = Arc::new(Mutex::new(ForwardStats::default()));
    tokio::spawn(forward::serve(client, req, listener, backend, stats.clone(), Arc::new(|| {})));

    let mut conn = tokio::net::TcpStream::connect(local).await.unwrap();
    conn.write_all(format!("GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n").as_bytes()).await.unwrap();
    let mut reply = String::new();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), conn.read_to_string(&mut reply)).await;
    println!("reply: {}", reply.lines().take(3).collect::<Vec<_>>().join(" | "));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let s = stats.lock().unwrap();
    println!("stats: active {} total {} error {:?} backend {:?}", s.active, s.total, s.last_error, s.backend);
    Ok(())
}
