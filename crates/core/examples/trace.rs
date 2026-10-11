//! Read-only: records a trace into a file, then prints its summary.
//!
//! `cargo run -p subduer-core --example trace -- <context> <scope> <file> <minutes>`
//! `cargo run -p subduer-core --example trace -- --summary <file>`
//!
//! `<scope>` is `cluster`, `ns:<a>[,<b>…]`, `<kind>:<ns>/<name>` (kind is
//! deploy, sts, ds, job or cronjob) or `pod:<ns>/<name>`. The first pull
//! backfills up to two hours; `<minutes>` of 0 stops right after it. The
//! file must not exist yet.

use std::path::Path;
use std::time::Duration;

use subduer_core::contexts;
use subduer_core::trace::{self, Trace};
use subduer_core::types::{KindRef, TraceEnd, TraceScope, TraceSummary, TraceWorkload};

#[tokio::main]
async fn main() -> subduer_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, file] = &args[..]
        && flag == "--summary"
    {
        print(&trace::read_summary(Path::new(file)).await?);
        return Ok(());
    }
    let [context, scope, file, minutes] = &args[..] else {
        usage();
    };
    let Some(scope) = parse_scope(scope) else { usage() };
    let minutes: u64 = minutes.parse().unwrap_or_else(|_| usage());
    let kubeconfig = contexts::load_kubeconfig()?;
    let client = contexts::client_for(&kubeconfig, context).await?;
    let started = std::time::Instant::now();
    let trace = Trace::start(client, context, scope, Path::new(file), false).await?;
    println!("started in {:.1}s; pulling every {:?}", started.elapsed().as_secs_f64(), trace.pull_every());
    print(&trace.summary(true).await?);
    let stop = async {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(minutes * 60)) => TraceEnd::Stopped,
            _ = tokio::signal::ctrl_c() => TraceEnd::Quit,
        }
    };
    let summary = trace.run(stop).await?;
    println!("--- finished");
    print(&summary);
    Ok(())
}

fn usage() -> ! {
    eprintln!("usage: trace <context> <cluster|ns:a,b|deploy:ns/name|sts:…|ds:…|job:…|cronjob:…|pod:ns/name> <file> <minutes>");
    eprintln!("       trace --summary <file>");
    std::process::exit(2);
}

fn parse_scope(s: &str) -> Option<TraceScope> {
    if s == "cluster" {
        return Some(TraceScope::Cluster);
    }
    let (kind, rest) = s.split_once(':')?;
    if kind == "ns" {
        return Some(TraceScope::Namespaces(rest.split(',').map(str::to_owned).collect()));
    }
    let (namespace, name) = rest.split_once('/')?;
    let (namespace, name) = (namespace.to_owned(), name.to_owned());
    let (group, version, kind, plural) = match kind {
        "pod" => return Some(TraceScope::Pod { namespace, name }),
        "deploy" => ("apps", "v1", "Deployment", "deployments"),
        "sts" => ("apps", "v1", "StatefulSet", "statefulsets"),
        "ds" => ("apps", "v1", "DaemonSet", "daemonsets"),
        "job" => ("batch", "v1", "Job", "jobs"),
        "cronjob" => ("batch", "v1", "CronJob", "cronjobs"),
        _ => return None,
    };
    let kind = KindRef { group: group.into(), version: version.into(), kind: kind.into(), plural: plural.into(), namespaced: true };
    Some(TraceScope::Workloads(vec![TraceWorkload { kind, namespace, name }]))
}

fn print(s: &TraceSummary) {
    let span = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) => format!("{:.0} min", (b - a) / 60.0),
        _ => "none".into(),
    };
    println!("file {} ({:.1} KiB)", s.path, s.file_bytes as f64 / 1024.0);
    println!("context {} scope {:?}", s.context, s.scope);
    println!("prometheus {:?} retention {:?}s", s.prometheus, s.retention_secs);
    println!("ended {:?} reason {:?} unfinished {}", s.ended, s.end_reason, s.unfinished);
    println!("data span {}", span(s.data_start, s.data_end));
    println!(
        "pods {} containers {} minutes: {} from Prometheus, {} sampled by the app",
        s.pods, s.containers, s.prometheus_samples, s.local_samples
    );
    for g in &s.gaps {
        println!("gap {:.0} min: {}", (g.end - g.start) / 60.0, g.reason);
    }
}
