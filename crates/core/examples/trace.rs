//! Read-only: records a trace into a file, then prints its summary.
//!
//! `cargo run -p subduer-core --example trace -- <context> <scope> <file> <minutes>`
//! `cargo run -p subduer-core --example trace -- --summary <file>`
//! `cargo run -p subduer-core --example trace -- --review <file> [<namespace/kind/name/container>]`
//! `cargo run -p subduer-core --example trace -- --report <file> <out.html>` (the out file must not exist)
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
    if let [flag, file, out] = &args[..]
        && flag == "--report"
    {
        let out = Path::new(out);
        if out.exists() {
            eprintln!("{} already exists", out.display());
            std::process::exit(2);
        }
        let html = subduer_core::trace_report::report_html(Path::new(file), false, &jiff::tz::TimeZone::system(), "now")?;
        std::fs::write(out, html).map_err(|e| subduer_core::Error::Message(e.to_string()))?;
        println!("wrote {}", out.display());
        return Ok(());
    }
    if let [flag, file, rest @ ..] = &args[..]
        && flag == "--review"
    {
        review(Path::new(file), rest.first().map(String::as_str))?;
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
    eprintln!("       trace --review <file> [<namespace/kind/name/container>]");
    eprintln!("       trace --report <file> <out.html>");
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

/// The right-sizing table, or one row's series and day-and-hour grid.
fn review(path: &Path, key: Option<&str>) -> subduer_core::Result<()> {
    use subduer_core::trace_review;
    use subduer_core::types::TraceKey;
    let r = trace_review::review(path, false)?;
    print(&r.summary);
    let Some(key) = key else {
        println!("{:<44} {:>5} {:>7} {:>7} {:>7} {:>8} {:>9} {:>9}  suggestion / flags", "row", "pods", "cpu req", "p95", "max", "spare", "mem req", "peak");
        for row in &r.rows {
            let m = |v: Option<f64>| v.map_or("-".into(), |v| format!("{:.0}m", v));
            let b = |v: Option<f64>| v.map_or("-".into(), |v| format!("{:.0}Mi", v / 1048576.0));
            let k = &row.key;
            println!(
                "{:<44} {:>5.1} {:>7} {:>6.1}m {:>6.0}m {:>8} {:>9} {:>9}  {} {:?}",
                format!("{}/{}/{}/{}", k.namespace, k.kind, k.name, k.container),
                row.avg_pods,
                m(row.cpu_request),
                row.cpu_p95,
                row.cpu_max,
                m(row.cpu_spare),
                b(row.mem_request),
                b(Some(row.mem_max)),
                row.suggestion.as_ref().map_or("-".into(), |s| format!("{:.0}m/{:.0}Mi", s.cpu_request, s.mem_request / 1048576.0)),
                row.flags,
            );
        }
        return Ok(());
    };
    let parts: Vec<&str> = key.splitn(4, '/').collect();
    let [namespace, kind, name, container] = parts[..] else { usage() };
    let key = TraceKey { namespace: namespace.into(), kind: kind.into(), name: name.into(), container: container.into() };
    let s = trace_review::series(path, &key, None, None, 60)?;
    println!("series {}..{} step {}s: {} cpu points, {} memory points, requests {:?}, limits {:?}, {} events, {} gaps",
        s.start, s.end, s.step, s.cpu_avg.len(), s.mem_max.len(), s.cpu_request, s.mem_limit, s.events.len(), s.gaps.len());
    let d = trace_review::daily(path, &key, &jiff::tz::TimeZone::system())?;
    for day in &d.days {
        let hours: Vec<String> = day.cpu.iter().map(|c| c.map_or("  .".into(), |c| format!("{c:3.0}"))).collect();
        println!("{:<12} {}", day.label, hours.join(""));
    }
    Ok(())
}
