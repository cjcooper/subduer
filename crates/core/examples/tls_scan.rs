//! Reads a pcap file and lists each TLS connection's version and cipher.
//! Works on the file alone:
//! `cargo run -p subduer-core --example tls_scan -- <file.pcap> [pod-ip …]`
//! Given the pod's addresses, connections are marked in (to the pod) or out.

use std::collections::BTreeMap;
use std::net::IpAddr;

use subduer_core::tls_scan;
use subduer_core::types::TlsDirection;

fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(file) = args.first() else {
        eprintln!("usage: tls_scan <file.pcap> [pod-ip …]");
        std::process::exit(2);
    };
    let ips: Vec<IpAddr> = args[1..].iter().filter_map(|a| a.parse().ok()).collect();
    let bytes = std::fs::read(file).map_err(|e| subduer_core::Error::Message(e.to_string()))?;
    let scan = tls_scan::scan(&bytes, &ips)?;
    println!("{} packets, {} hellos, {} connections", scan.packets, scan.hellos, scan.connections.len());
    let mut by_version: BTreeMap<String, usize> = BTreeMap::new();
    for c in &scan.connections {
        let version = c.version.clone().unwrap_or_else(|| "no answer captured".into());
        *by_version.entry(version.clone()).or_default() += 1;
        let way = match c.direction {
            TlsDirection::Inbound => "in ",
            TlsDirection::Outbound => "out",
            TlsDirection::Other => "   ",
        };
        println!(
            "{way} {:<24} → {:<22} {:<28} {:<8} {} (offered {}){}",
            c.client,
            c.server,
            c.server_name.as_deref().unwrap_or("-"),
            version,
            c.cipher.as_deref().unwrap_or("-"),
            c.offered.join(", "),
            if c.retried { " retried" } else { "" },
        );
    }
    for (version, n) in by_version {
        println!("{version}: {n}");
    }
    Ok(())
}
