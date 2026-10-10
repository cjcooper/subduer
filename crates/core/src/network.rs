//! The Details tab of the Network section's kinds: Services (ports, the
//! endpoints behind them, the Ingresses routing to them), Ingresses (rules
//! with backend health, TLS, URLs to open), IngressClasses, NetworkPolicies
//! (what they let in and out, in words, and the pods they select),
//! EndpointSlices and Endpoints. Built on `config`'s generic shape.

use std::collections::{BTreeMap, HashMap, HashSet};

use k8s_openapi::api::core::v1::{Pod, Secret, Service};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use k8s_openapi::api::networking::v1::Ingress;
use kube::api::{Api, ListParams};
use kube::{Client, ResourceExt};
use serde_json::Value;

use crate::config::{int_at, label_selector, plural, str_at, table_section};
use crate::types::{InfoRow, InfoSection, InfoTable, InfoTableRow, ObjectDetails, ObjectLink};
use crate::Result;

/// Fills in what comes from the object itself.
pub(crate) fn describe(key: &str, d: &mut ObjectDetails, v: &Value) {
    match key {
        "core/Service" => service(d, v),
        "networking.k8s.io/Ingress" => ingress(d, v),
        "networking.k8s.io/IngressClass" => ingress_class(d, v),
        "networking.k8s.io/NetworkPolicy" => network_policy(d, v),
        "discovery.k8s.io/EndpointSlice" => endpoint_slice(d, v),
        "core/Endpoints" => endpoints(d, v),
        _ => {}
    }
}

/// Adds what needs other objects: endpoints, routes, selected pods.
pub(crate) async fn related(
    client: &Client,
    key: &str,
    namespace: &str,
    name: &str,
    v: &Value,
    d: &mut ObjectDetails,
) -> Result<()> {
    match key {
        "core/Service" => service_related(client, namespace, name, v, d).await,
        "networking.k8s.io/Ingress" => ingress_related(client, namespace, d).await,
        "networking.k8s.io/IngressClass" => ingress_class_related(client, name, v, d).await,
        "networking.k8s.io/NetworkPolicy" => policy_pods(client, namespace, v, d).await,
        _ => Ok(()),
    }
}

fn link(kind: &str, namespace: Option<&str>, name: &str) -> ObjectLink {
    ObjectLink { kind: kind.into(), namespace: namespace.map(str::to_owned), name: name.into() }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect()
}

/// Hostnames and IPs a load balancer published.
fn lb_addresses(status: &Value) -> Vec<String> {
    status["loadBalancer"]["ingress"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| str_at(i, "hostname").or_else(|| str_at(i, "ip")))
        .collect()
}

// ---- Service ----

fn service(d: &mut ObjectDetails, v: &Value) {
    let (spec, status) = (&v["spec"], &v["status"]);
    let type_ = str_at(spec, "type").unwrap_or_else(|| "ClusterIP".into());
    d.info.push(InfoRow::new("Type", type_.clone()));
    if type_ == "ExternalName" {
        d.info.push(InfoRow::new("Points to", str_at(spec, "externalName").unwrap_or_default()).mono());
        d.status = Some("External name".into());
    } else {
        let ips = strings(&spec["clusterIPs"]);
        let ip = if ips.is_empty() { str_at(spec, "clusterIP").unwrap_or_default() } else { ips.join(", ") };
        let ip = if ip == "None" { "None (headless: DNS returns the pod IPs)".to_owned() } else { ip };
        d.info.push(InfoRow::new("Cluster IP", ip).mono());
    }
    if type_ == "LoadBalancer" {
        let addresses = lb_addresses(status);
        if addresses.is_empty() {
            d.info.push(InfoRow::new("Load balancer", "no address yet"));
            d.status = Some("Pending address".into());
        } else {
            d.info.push(InfoRow::new("Load balancer", addresses.join(", ")).mono());
        }
        if let Some(class) = str_at(spec, "loadBalancerClass") {
            d.info.push(InfoRow::new("Load balancer class", class));
        }
    }
    let external = strings(&spec["externalIPs"]);
    if !external.is_empty() {
        d.info.push(InfoRow::new("External IPs", external.join(", ")).mono());
    }
    let selector: Vec<String> = spec["selector"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, val)| format!("{k}={}", val.as_str().unwrap_or_default()))
        .collect();
    if selector.is_empty() && type_ != "ExternalName" {
        d.info.push(InfoRow::new("Selector", "none: its endpoints are managed by hand or by a controller"));
    } else if !selector.is_empty() {
        d.info.push(InfoRow::new("Selector", selector.join(",")).mono());
    }
    if str_at(spec, "sessionAffinity").as_deref() == Some("ClientIP") {
        let timeout = int_at(&spec["sessionAffinityConfig"]["clientIP"], "timeoutSeconds").unwrap_or(10800);
        d.info.push(InfoRow::new("Session affinity", format!("by client IP, for {timeout}s")));
    }
    if matches!(type_.as_str(), "NodePort" | "LoadBalancer") {
        let policy = str_at(spec, "externalTrafficPolicy").unwrap_or_else(|| "Cluster".into());
        let text = if policy == "Local" { "Local: only nodes running its pods, client IP kept" } else { "Cluster: any node" };
        d.info.push(InfoRow::new("External traffic", text));
    }
    if str_at(spec, "internalTrafficPolicy").as_deref() == Some("Local") {
        d.info.push(InfoRow::new("Internal traffic", "Local: only pods on the caller's node"));
    }
    if spec["publishNotReadyAddresses"] == true {
        d.info.push(InfoRow::new("Not-ready pods", "published too"));
    }
    let families = strings(&spec["ipFamilies"]);
    if families.len() > 1 {
        d.info.push(InfoRow::new("IP families", families.join(", ")));
    }

    let rows: Vec<InfoTableRow> = spec["ports"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let port = int_at(p, "port").unwrap_or(0).to_string();
            let target = str_at(p, "targetPort").unwrap_or_else(|| port.clone());
            InfoTableRow::new(vec![
                str_at(p, "name").unwrap_or_else(|| "—".into()),
                port,
                target,
                str_at(p, "nodePort").unwrap_or_else(|| "—".into()),
                str_at(p, "protocol").unwrap_or_else(|| "TCP".into()),
                str_at(p, "appProtocol").unwrap_or_default(),
            ])
        })
        .collect();
    let table = InfoTable {
        columns: ["Name", "Port", "Target", "Node port", "Protocol", "App protocol"].map(String::from).to_vec(),
        numeric: vec![1, 3],
        mono: vec![2],
        rows,
    };
    if type_ != "ExternalName" {
        d.sections.push(table_section("ports", "Ports", table, "No ports."));
    }
}

async fn service_related(client: &Client, namespace: &str, name: &str, v: &Value, d: &mut ObjectDetails) -> Result<()> {
    if str_at(&v["spec"], "type").as_deref() == Some("ExternalName") {
        return Ok(());
    }
    let slices = Api::<EndpointSlice>::namespaced(client.clone(), namespace);
    let ingresses = Api::<Ingress>::namespaced(client.clone(), namespace);
    let by_service = ListParams::default().labels(&format!("kubernetes.io/service-name={name}"));
    let all = ListParams::default();
    let (slices, ingresses) = tokio::join!(slices.list(&by_service), ingresses.list(&all));
    let slices = slices?;
    let mut rows = Vec::new();
    for s in &slices.items {
        rows.extend(slice_rows(&serde_json::to_value(s).unwrap_or_default(), namespace));
    }
    rows.sort_by(|a, b| a.cells.cmp(&b.cells));
    // Several slices (IPv4 and IPv6) may list the same pod.
    rows.dedup_by(|a, b| a.cells == b.cells);
    let ready = rows.iter().filter(|r| !r.warn).count();
    let total = rows.len();
    let has_selector = v["spec"]["selector"].as_object().is_some_and(|m| !m.is_empty());
    if d.status.is_none() {
        d.status = Some(
            match (ready, total) {
                (0, 0) if !has_selector => "No endpoints",
                (0, _) => "No ready endpoints",
                (r, t) if r < t => "Partly ready",
                _ => "Ready",
            }
            .to_owned(),
        );
    }
    let mut section = table_section(
        "endpoints",
        "Endpoints",
        endpoint_table(rows),
        if has_selector { "No pods match its selector." } else { "No endpoints." },
    );
    section.summary = if total == 0 { "none".into() } else { format!("{ready} of {total} ready") };
    d.sections.push(section);

    // Ingresses with a rule or default backend sending traffic here.
    let mut routes = Vec::new();
    for ing in ingresses?.items {
        let spec = ing.spec.clone().unwrap_or_default();
        let to_here = |b: &Option<k8s_openapi::api::networking::v1::IngressBackend>| {
            b.as_ref().and_then(|b| b.service.as_ref()).is_some_and(|s| s.name == name)
        };
        if to_here(&spec.default_backend) {
            let mut row = InfoTableRow::new(vec![ing.name_any(), "(default backend)".into(), String::new()]);
            row.link = Some(link("networking.k8s.io/Ingress", Some(namespace), &ing.name_any()));
            routes.push(row);
        }
        for rule in spec.rules.iter().flatten() {
            for path in rule.http.iter().flat_map(|h| &h.paths) {
                if to_here(&Some(path.backend.clone())) {
                    let mut row = InfoTableRow::new(vec![
                        ing.name_any(),
                        rule.host.clone().unwrap_or_else(|| "*".into()),
                        path.path.clone().unwrap_or_else(|| "/".into()),
                    ]);
                    row.link = Some(link("networking.k8s.io/Ingress", Some(namespace), &ing.name_any()));
                    routes.push(row);
                }
            }
        }
    }
    if !routes.is_empty() {
        let table = InfoTable {
            columns: vec!["Ingress".into(), "Host".into(), "Path".into()],
            mono: vec![2],
            rows: routes,
            ..Default::default()
        };
        d.sections.push(table_section("routed-from", "Routed from", table, ""));
    }
    Ok(())
}

// ---- Endpoints (shared) ----

fn endpoint_table(rows: Vec<InfoTableRow>) -> InfoTable {
    InfoTable {
        columns: ["Address", "State", "Target", "Node", "Zone"].map(String::from).to_vec(),
        mono: vec![0],
        rows,
        ..Default::default()
    }
}

/// One row per endpoint of an EndpointSlice; not-ready ones are marked.
fn slice_rows(slice: &Value, namespace: &str) -> Vec<InfoTableRow> {
    slice["endpoints"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let c = &e["conditions"];
            // `ready` unset means ready, per the API.
            let ready = c["ready"] != false;
            let state = if c["terminating"] == true {
                "terminating"
            } else if ready {
                "ready"
            } else if c["serving"] == true {
                "serving, not ready"
            } else {
                "not ready"
            };
            target_row(&strings(&e["addresses"]).join(", "), state, &e["targetRef"], e, namespace, !ready)
        })
        .collect()
}

fn target_row(address: &str, state: &str, target: &Value, e: &Value, namespace: &str, warn: bool) -> InfoTableRow {
    let (kind, name) = (str_at(target, "kind"), str_at(target, "name"));
    let target_text = match (&kind, &name) {
        (Some(k), Some(n)) => format!("{k} {n}"),
        _ => "—".into(),
    };
    let mut row = InfoTableRow::new(vec![
        address.to_owned(),
        state.to_owned(),
        target_text,
        str_at(e, "nodeName").unwrap_or_else(|| "—".into()),
        str_at(e, "zone").unwrap_or_default(),
    ]);
    if kind.as_deref() == Some("Pod")
        && let Some(n) = name
    {
        let ns = str_at(target, "namespace").unwrap_or_else(|| namespace.to_owned());
        row.link = Some(link("core/Pod", Some(&ns), &n));
    }
    row.warn = warn;
    row
}

fn endpoint_status(rows: &[InfoTableRow]) -> String {
    let ready = rows.iter().filter(|r| !r.warn).count();
    match (ready, rows.len()) {
        (_, 0) => "No endpoints",
        (0, _) => "No ready endpoints",
        (r, t) if r < t => "Partly ready",
        _ => "Ready",
    }
    .to_owned()
}

fn endpoint_slice(d: &mut ObjectDetails, v: &Value) {
    let ns = d.namespace.clone().unwrap_or_default();
    let labels = &v["metadata"]["labels"];
    if let Some(svc) = str_at(labels, "kubernetes.io/service-name") {
        d.info.push(InfoRow::new("Service", svc.clone()).link(link("core/Service", Some(&ns), &svc)));
    }
    d.info.push(InfoRow::new("Address type", str_at(v, "addressType").unwrap_or_default()));
    if let Some(m) = str_at(labels, "endpointslice.kubernetes.io/managed-by") {
        d.info.push(InfoRow::new("Managed by", m));
    }
    let ports: Vec<String> = v["ports"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let port = format!("{}/{}", int_at(p, "port").unwrap_or(0), str_at(p, "protocol").unwrap_or_else(|| "TCP".into()));
            match str_at(p, "name").filter(|n| !n.is_empty()) {
                Some(n) => format!("{n} {port}"),
                None => port,
            }
        })
        .collect();
    d.info.push(InfoRow::new("Ports", if ports.is_empty() { "—".into() } else { ports.join(", ") }));
    let rows = slice_rows(v, &ns);
    d.status = Some(endpoint_status(&rows));
    let ready = rows.iter().filter(|r| !r.warn).count();
    let mut section = table_section("slice-endpoints", "Endpoints", endpoint_table(rows), "No endpoints.");
    section.summary = format!("{ready} of {} ready", section.table.as_ref().map_or(0, |t| t.rows.len()));
    d.sections.push(section);
}

fn endpoints(d: &mut ObjectDetails, v: &Value) {
    let ns = d.namespace.clone().unwrap_or_default();
    d.info.push(InfoRow::new("Service", d.name.clone()).link(link("core/Service", Some(&ns), &d.name)));
    d.info.push(InfoRow::new("Note", "Endpoints is deprecated; EndpointSlices carry the same addresses."));
    let mut rows = Vec::new();
    let mut ports = Vec::new();
    for subset in v["subsets"].as_array().into_iter().flatten() {
        for (key, ready) in [("addresses", true), ("notReadyAddresses", false)] {
            for a in subset[key].as_array().into_iter().flatten() {
                let state = if ready { "ready" } else { "not ready" };
                rows.push(target_row(&str_at(a, "ip").unwrap_or_default(), state, &a["targetRef"], a, &ns, !ready));
            }
        }
        for p in subset["ports"].as_array().into_iter().flatten() {
            ports.push(format!("{}/{}", int_at(p, "port").unwrap_or(0), str_at(p, "protocol").unwrap_or_else(|| "TCP".into())));
        }
    }
    ports.sort();
    ports.dedup();
    d.info.push(InfoRow::new("Ports", if ports.is_empty() { "—".into() } else { ports.join(", ") }));
    d.status = Some(endpoint_status(&rows));
    d.sections.push(table_section("addresses", "Addresses", endpoint_table(rows), "No addresses."));
}

// ---- Ingress ----

/// `name:port` of a backend, and the Service name if it is one.
fn backend(b: &Value) -> (String, Option<String>) {
    let svc = &b["service"];
    if let Some(name) = str_at(svc, "name") {
        let port = str_at(&svc["port"], "number").or_else(|| str_at(&svc["port"], "name")).unwrap_or_default();
        (format!("{name}:{port}"), Some(name))
    } else if let Some(kind) = str_at(&b["resource"], "kind") {
        (format!("{kind} {}", str_at(&b["resource"], "name").unwrap_or_default()), None)
    } else {
        ("—".into(), None)
    }
}

fn ingress(d: &mut ObjectDetails, v: &Value) {
    let (spec, status) = (&v["spec"], &v["status"]);
    let ns = d.namespace.clone().unwrap_or_default();
    let class = str_at(spec, "ingressClassName")
        .or_else(|| str_at(&v["metadata"]["annotations"], "kubernetes.io/ingress.class"));
    match class {
        Some(c) => d.info.push(InfoRow::new("Class", c.clone()).link(link("networking.k8s.io/IngressClass", None, &c))),
        None => d.info.push(InfoRow::new("Class", "none: the cluster's default class, if any")),
    }
    let addresses = lb_addresses(status);
    d.info.push(if addresses.is_empty() {
        InfoRow::new("Address", "none yet")
    } else {
        InfoRow::new("Address", addresses.join(", ")).mono()
    });
    if spec["defaultBackend"].is_object() {
        let (text, svc) = backend(&spec["defaultBackend"]);
        let row = InfoRow::new("Default backend", text).mono();
        d.info.push(match svc {
            Some(s) => row.link(link("core/Service", Some(&ns), &s)),
            None => row,
        });
    }

    let tls_hosts: HashSet<String> = spec["tls"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|t| strings(&t["hosts"]))
        .collect();
    let mut rows = Vec::new();
    let mut urls = Vec::new();
    for rule in spec["rules"].as_array().into_iter().flatten() {
        let host = str_at(rule, "host");
        for p in rule["http"]["paths"].as_array().into_iter().flatten() {
            let path = str_at(p, "path").unwrap_or_else(|| "/".into());
            let (text, svc) = backend(&p["backend"]);
            let mut row = InfoTableRow::new(vec![
                host.clone().unwrap_or_else(|| "*".into()),
                path.clone(),
                str_at(p, "pathType").unwrap_or_default(),
                text,
                String::new(),
            ]);
            row.link = svc.map(|s| link("core/Service", Some(&ns), &s));
            rows.push(row);
            // Something a browser can open: a real host (or the address) and a plain path.
            let target = host.clone().filter(|h| !h.starts_with('*')).or_else(|| addresses.first().cloned());
            if let Some(h) = target
                && !path.contains(['(', '[', '*', '$', '^', '\\'])
            {
                let scheme = if tls_hosts.contains(&h) { "https" } else { "http" };
                let url = format!("{scheme}://{h}{path}");
                if !urls.contains(&url) {
                    urls.push(url);
                }
            }
        }
    }
    let n = rows.len();
    d.sections.push(InfoSection {
        id: "rules".into(),
        title: "Rules".into(),
        summary: plural(n, "path"),
        rows: Vec::new(),
        table: Some(InfoTable {
            columns: ["Host", "Path", "Match", "Backend", "Ready"].map(String::from).to_vec(),
            numeric: vec![4],
            mono: vec![1, 3],
            rows,
        }),
        empty: Some("No rules: everything goes to the default backend.".into()),
    });
    if !urls.is_empty() {
        let rows = urls.into_iter().map(|u| InfoRow::new("URL", u.clone()).mono().url(u)).collect();
        d.sections.push(InfoSection {
            id: "urls".into(),
            title: "URLs".into(),
            summary: String::new(),
            rows,
            table: None,
            empty: None,
        });
    }
    let tls: Vec<InfoTableRow> = spec["tls"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let secret = str_at(t, "secretName");
            let mut row = InfoTableRow::new(vec![
                strings(&t["hosts"]).join(", "),
                secret.clone().unwrap_or_else(|| "(controller's default certificate)".into()),
            ]);
            row.link = secret.map(|s| link("core/Secret", Some(&ns), &s));
            row
        })
        .collect();
    if !tls.is_empty() {
        let table = InfoTable { columns: vec!["Hosts".into(), "Certificate secret".into()], rows: tls, ..Default::default() };
        d.sections.push(table_section("tls", "TLS", table, ""));
    }
    d.status = Some(if addresses.is_empty() { "No address" } else { "Ready" }.into());
}

/// Ready endpoints per backend Service, and missing Services and TLS Secrets.
async fn ingress_related(client: &Client, namespace: &str, d: &mut ObjectDetails) -> Result<()> {
    let services = Api::<Service>::namespaced(client.clone(), namespace);
    let slices = Api::<EndpointSlice>::namespaced(client.clone(), namespace);
    // Names only: Secret data is never read here.
    let secrets = Api::<Secret>::namespaced(client.clone(), namespace);
    let all = ListParams::default();
    let (services, slices, secrets) =
        tokio::join!(services.list_metadata(&all), slices.list(&all), secrets.list_metadata(&all));
    let services: HashSet<String> = services?.items.iter().map(ResourceExt::name_any).collect();
    let secrets: HashSet<String> = secrets.map(|l| l.items.iter().map(ResourceExt::name_any).collect()).unwrap_or_default();
    let mut ready: HashMap<String, (usize, usize)> = HashMap::new();
    for s in slices?.items {
        let Some(svc) = s.labels().get("kubernetes.io/service-name").cloned() else { continue };
        let entry = ready.entry(svc).or_default();
        for e in s.endpoints.iter().flatten() {
            entry.1 += 1;
            if e.conditions.as_ref().and_then(|c| c.ready) != Some(false) {
                entry.0 += 1;
            }
        }
    }
    let mut missing = false;
    let mut unready = false;
    for section in &mut d.sections {
        let Some(table) = section.table.as_mut() else { continue };
        match section.id.as_str() {
            "rules" => {
                for row in &mut table.rows {
                    let Some(svc) = row.link.as_ref().map(|l| l.name.clone()) else { continue };
                    let text = if !services.contains(&svc) {
                        missing = true;
                        row.warn = true;
                        "no such service".to_owned()
                    } else {
                        let (r, t) = ready.get(&svc).copied().unwrap_or_default();
                        if r == 0 {
                            unready = true;
                            row.warn = true;
                        }
                        format!("{r}/{t}")
                    };
                    if let Some(cell) = row.cells.last_mut() {
                        *cell = text;
                    }
                }
            }
            "tls" => {
                for row in &mut table.rows {
                    if let Some(l) = &row.link
                        && !secrets.is_empty()
                        && !secrets.contains(&l.name)
                    {
                        row.warn = true;
                        row.cells[1] = format!("{} (missing)", l.name);
                    }
                }
            }
            _ => {}
        }
    }
    if missing {
        d.status = Some("Backend missing".into());
    } else if unready {
        d.status = Some("Backend not ready".into());
    }
    Ok(())
}

// ---- IngressClass ----

fn ingress_class(d: &mut ObjectDetails, v: &Value) {
    let spec = &v["spec"];
    d.info.push(InfoRow::new("Controller", str_at(spec, "controller").unwrap_or_default()).mono());
    let default = str_at(&v["metadata"]["annotations"], "ingressclass.kubernetes.io/is-default-class").as_deref() == Some("true");
    d.info.push(InfoRow::new("Default class", if default { "yes: Ingresses without a class use it" } else { "no" }));
    let p = &spec["parameters"];
    if let Some(kind) = str_at(p, "kind") {
        let mut text = format!("{kind} {}", str_at(p, "name").unwrap_or_default());
        if let Some(ns) = str_at(p, "namespace") {
            text += &format!(" in {ns}");
        }
        d.info.push(InfoRow::new("Parameters", text));
    }
    if default {
        d.status = Some("Default".into());
    }
}

async fn ingress_class_related(client: &Client, name: &str, v: &Value, d: &mut ObjectDetails) -> Result<()> {
    let default = str_at(&v["metadata"]["annotations"], "ingressclass.kubernetes.io/is-default-class").as_deref() == Some("true");
    let list = Api::<Ingress>::all(client.clone()).list(&ListParams::default()).await?;
    let rows: Vec<InfoTableRow> = list
        .items
        .iter()
        .filter(|i| {
            let class = i
                .spec
                .as_ref()
                .and_then(|s| s.ingress_class_name.clone())
                .or_else(|| i.annotations().get("kubernetes.io/ingress.class").cloned());
            class.as_deref() == Some(name) || (class.is_none() && default)
        })
        .map(|i| {
            let hosts: Vec<String> = i
                .spec
                .iter()
                .flat_map(|s| s.rules.iter().flatten())
                .filter_map(|r| r.host.clone())
                .collect();
            let ns = i.namespace().unwrap_or_default();
            let mut row = InfoTableRow::new(vec![ns.clone(), i.name_any(), hosts.join(", ")]);
            row.link = Some(link("networking.k8s.io/Ingress", Some(&ns), &i.name_any()));
            row
        })
        .collect();
    let table = InfoTable { columns: vec!["Namespace".into(), "Ingress".into(), "Hosts".into()], rows, ..Default::default() };
    d.sections.push(table_section("class-ingresses", "Ingresses", table, "No Ingresses use it."));
    Ok(())
}

// ---- NetworkPolicy ----

/// Who a peer is, in words.
fn peer(p: &Value) -> String {
    if p["ipBlock"].is_object() {
        let cidr = str_at(&p["ipBlock"], "cidr").unwrap_or_default();
        let except = strings(&p["ipBlock"]["except"]);
        return if except.is_empty() { format!("addresses {cidr}") } else { format!("addresses {cidr} except {}", except.join(", ")) };
    }
    let pods = p["podSelector"].is_object().then(|| label_selector(&p["podSelector"]));
    let namespaces = p["namespaceSelector"].is_object().then(|| label_selector(&p["namespaceSelector"]));
    match (pods, namespaces) {
        (Some(None), None) => "all pods in this namespace".into(),
        (Some(Some(s)), None) => format!("pods {s} in this namespace"),
        (None, Some(None)) => "all pods in all namespaces".into(),
        (None, Some(Some(n))) => format!("all pods in namespaces {n}"),
        (Some(None), Some(None)) => "all pods in all namespaces".into(),
        (Some(Some(s)), Some(None)) => format!("pods {s} in any namespace"),
        (Some(None), Some(Some(n))) => format!("all pods in namespaces {n}"),
        (Some(Some(s)), Some(Some(n))) => format!("pods {s} in namespaces {n}"),
        (None, None) => "—".into(),
    }
}

fn ports(rule: &Value) -> String {
    let list: Vec<String> = rule["ports"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let proto = str_at(p, "protocol").unwrap_or_else(|| "TCP".into());
            match (str_at(p, "port"), int_at(p, "endPort")) {
                (Some(port), Some(end)) => format!("{proto} {port}–{end}"),
                (Some(port), None) => format!("{proto} {port}"),
                (None, _) => format!("all {proto}"),
            }
        })
        .collect();
    if list.is_empty() { "all ports".into() } else { list.join(", ") }
}

fn network_policy(d: &mut ObjectDetails, v: &Value) {
    let spec = &v["spec"];
    let applies = match label_selector(&spec["podSelector"]) {
        Some(s) => format!("pods {s}"),
        None => "every pod in the namespace".into(),
    };
    d.info.push(InfoRow::new("Applies to", applies));
    let mut types = strings(&spec["policyTypes"]);
    if types.is_empty() {
        // Unset: Ingress always, Egress only when egress rules are given.
        types.push("Ingress".into());
        if spec["egress"].is_array() {
            types.push("Egress".into());
        }
    }
    let mut denies = Vec::new();
    for (type_, key, title, peers_key, word) in [
        ("Ingress", "ingress", "Incoming", "from", "in"),
        ("Egress", "egress", "Outgoing", "to", "out"),
    ] {
        let rules: Vec<&Value> = spec[key].as_array().into_iter().flatten().collect();
        let row = if !types.iter().any(|t| t == type_) {
            "not restricted by this policy".to_owned()
        } else if rules.is_empty() {
            denies.push(word);
            "all blocked (unless another policy allows it)".to_owned()
        } else {
            format!("only what the {} below allow{}", plural(rules.len(), "rule"), if rules.len() == 1 { "s" } else { "" })
        };
        d.info.push(InfoRow::new(title, row));
        if !rules.is_empty() {
            let table_rows: Vec<InfoTableRow> = rules
                .iter()
                .map(|r| {
                    let peers: Vec<String> = r[peers_key].as_array().into_iter().flatten().map(peer).collect();
                    let who = if peers.is_empty() { "anywhere".to_owned() } else { peers.join("; ") };
                    InfoTableRow::new(vec![who, ports(r)])
                })
                .collect();
            let column = if type_ == "Ingress" { "From" } else { "To" };
            let table = InfoTable { columns: vec![column.into(), "Ports".into()], rows: table_rows, ..Default::default() };
            d.sections.push(table_section(key, title, table, ""));
        }
    }
    d.status = Some(
        match denies.as_slice() {
            ["in", "out"] => "Denies all",
            ["in"] => "Denies incoming",
            ["out"] => "Denies outgoing",
            _ => "Allows listed",
        }
        .to_owned(),
    );
}

async fn policy_pods(client: &Client, namespace: &str, v: &Value, d: &mut ObjectDetails) -> Result<()> {
    let params = match label_selector(&v["spec"]["podSelector"]) {
        Some(s) => ListParams::default().labels(&s),
        None => ListParams::default(),
    };
    let pods = Api::<Pod>::namespaced(client.clone(), namespace).list(&params).await?;
    let mut owners: BTreeMap<String, usize> = BTreeMap::new();
    let rows: Vec<InfoTableRow> = pods
        .items
        .iter()
        .map(|p| {
            let owner = p
                .owner_references()
                .iter()
                .find(|o| o.controller == Some(true))
                .map_or_else(|| "—".to_owned(), |o| format!("{}/{}", o.kind, o.name));
            *owners.entry(owner.clone()).or_default() += 1;
            let mut row = InfoTableRow::new(vec![p.name_any(), owner]);
            row.link = Some(link("core/Pod", Some(namespace), &p.name_any()));
            row
        })
        .collect();
    let table = InfoTable { columns: vec!["Pod".into(), "Controlled by".into()], rows, ..Default::default() };
    let mut section = table_section("policy-pods", "Pods it applies to", table, "No pods match right now.");
    if !owners.is_empty() {
        section.summary = plural(section.table.as_ref().map_or(0, |t| t.rows.len()), "pod");
    }
    d.sections.push(section);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::describe;
    use serde_json::json;

    fn now() -> jiff::Timestamp {
        "2026-10-10T12:00:00Z".parse().unwrap()
    }

    fn row<'a>(d: &'a ObjectDetails, key: &str) -> Option<&'a str> {
        d.info.iter().find(|r| r.key == key).map(|r| r.value.as_str())
    }

    #[test]
    fn service_rows_and_ports() {
        let v = json!({"kind": "Service", "metadata": {"name": "api", "namespace": "shop"},
            "spec": {"type": "LoadBalancer", "clusterIP": "10.0.0.5", "clusterIPs": ["10.0.0.5"],
                     "selector": {"app": "api"}, "externalTrafficPolicy": "Local",
                     "ports": [{"name": "http", "port": 80, "targetPort": "http", "nodePort": 31080, "protocol": "TCP"}]},
            "status": {"loadBalancer": {}}});
        let d = describe("core/Service", &v, now());
        assert_eq!(d.status.as_deref(), Some("Pending address"));
        assert_eq!(row(&d, "Selector"), Some("app=api"));
        assert!(row(&d, "External traffic").unwrap().starts_with("Local"));
        let ports = d.sections[0].table.as_ref().unwrap();
        assert_eq!(ports.rows[0].cells, ["http", "80", "http", "31080", "TCP", ""]);

        let headless = json!({"kind": "Service", "metadata": {"name": "db"}, "spec": {"clusterIP": "None", "selector": {"app": "db"}}});
        assert!(row(&describe("core/Service", &headless, now()), "Cluster IP").unwrap().contains("headless"));
    }

    #[test]
    fn endpoint_slice_states() {
        let v = json!({"kind": "EndpointSlice", "addressType": "IPv4",
            "metadata": {"name": "api-x", "namespace": "shop", "labels": {"kubernetes.io/service-name": "api"}},
            "ports": [{"name": "http", "port": 8080, "protocol": "TCP"}],
            "endpoints": [
                {"addresses": ["10.1.0.4"], "conditions": {"ready": true}, "nodeName": "n1", "zone": "a",
                 "targetRef": {"kind": "Pod", "name": "api-1", "namespace": "shop"}},
                {"addresses": ["10.1.0.5"], "conditions": {"ready": false, "serving": true, "terminating": true},
                 "targetRef": {"kind": "Pod", "name": "api-2"}}]});
        let d = describe("discovery.k8s.io/EndpointSlice", &v, now());
        assert_eq!(d.status.as_deref(), Some("Partly ready"));
        assert_eq!(row(&d, "Ports"), Some("http 8080/TCP"));
        assert_eq!(d.info[0].link.as_ref().unwrap().name, "api");
        let s = &d.sections[0];
        assert_eq!(s.summary, "1 of 2 ready");
        let rows = &s.table.as_ref().unwrap().rows;
        assert_eq!(rows[0].cells, ["10.1.0.4", "ready", "Pod api-1", "n1", "a"]);
        assert_eq!((rows[1].cells[1].as_str(), rows[1].warn), ("terminating", true));
        assert_eq!(rows[1].link.as_ref().unwrap().namespace.as_deref(), Some("shop"));
    }

    #[test]
    fn ingress_rules_and_urls() {
        let v = json!({"kind": "Ingress", "metadata": {"name": "web", "namespace": "shop"},
            "spec": {"ingressClassName": "alb",
                "tls": [{"hosts": ["shop.acme.io"], "secretName": "shop-tls"}],
                "rules": [{"host": "shop.acme.io", "http": {"paths": [
                    {"path": "/", "pathType": "Prefix", "backend": {"service": {"name": "web", "port": {"number": 80}}}},
                    {"path": "/api(/|$)(.*)", "pathType": "ImplementationSpecific", "backend": {"service": {"name": "api", "port": {"name": "http"}}}}]}},
                    {"host": "*.acme.io", "http": {"paths": [{"path": "/", "pathType": "Prefix", "backend": {"service": {"name": "web", "port": {"number": 80}}}}]}}]},
            "status": {"loadBalancer": {"ingress": [{"hostname": "k8s-shop.elb.amazonaws.com"}]}}});
        let d = describe("networking.k8s.io/Ingress", &v, now());
        assert_eq!(d.status.as_deref(), Some("Ready"));
        assert_eq!(d.info[0].link.as_ref().unwrap().kind, "networking.k8s.io/IngressClass");
        let rules = d.sections.iter().find(|s| s.id == "rules").unwrap().table.as_ref().unwrap();
        assert_eq!(rules.rows[1].cells[..4], ["shop.acme.io", "/api(/|$)(.*)", "ImplementationSpecific", "api:http"]);
        let urls: Vec<&str> = d.sections.iter().find(|s| s.id == "urls").unwrap().rows.iter().map(|r| r.value.as_str()).collect();
        // The regex path is skipped; the wildcard host falls back to the load balancer.
        assert_eq!(urls, ["https://shop.acme.io/", "http://k8s-shop.elb.amazonaws.com/"]);
        let tls = d.sections.iter().find(|s| s.id == "tls").unwrap();
        assert_eq!(tls.table.as_ref().unwrap().rows[0].link.as_ref().unwrap().name, "shop-tls");
    }

    #[test]
    fn network_policy_in_words() {
        let v = json!({"kind": "NetworkPolicy", "metadata": {"name": "api", "namespace": "shop"},
            "spec": {"podSelector": {"matchLabels": {"app": "api"}}, "policyTypes": ["Ingress", "Egress"],
                "ingress": [{"from": [{"podSelector": {"matchLabels": {"app": "web"}}},
                                      {"namespaceSelector": {"matchLabels": {"team": "ops"}}, "podSelector": {}},
                                      {"ipBlock": {"cidr": "10.0.0.0/8", "except": ["10.1.0.0/16"]}}],
                             "ports": [{"port": 8080}, {"protocol": "UDP", "port": 5000, "endPort": 5010}]}]}});
        let d = describe("networking.k8s.io/NetworkPolicy", &v, now());
        assert_eq!(row(&d, "Applies to"), Some("pods app=api"));
        assert_eq!(row(&d, "Incoming"), Some("only what the 1 rule below allows"));
        assert!(row(&d, "Outgoing").unwrap().starts_with("all blocked"));
        assert_eq!(d.status.as_deref(), Some("Denies outgoing"));
        let rule = &d.sections[0].table.as_ref().unwrap().rows[0].cells;
        assert_eq!(
            rule[0],
            "pods app=web in this namespace; all pods in namespaces team=ops; addresses 10.0.0.0/8 except 10.1.0.0/16"
        );
        assert_eq!(rule[1], "TCP 8080, UDP 5000–5010");

        let deny = json!({"kind": "NetworkPolicy", "metadata": {"name": "deny"}, "spec": {"podSelector": {}}});
        let d = describe("networking.k8s.io/NetworkPolicy", &deny, now());
        assert_eq!(row(&d, "Applies to"), Some("every pod in the namespace"));
        assert_eq!(row(&d, "Outgoing"), Some("not restricted by this policy"));
        assert_eq!(d.status.as_deref(), Some("Denies incoming"));
    }
}
