# k8s-subduer

Desktop app (macOS first) for browsing and controlling Kubernetes clusters.
Rust throughout: Tauri v2 shell, `kube-rs` backend, Leptos (WASM) UI.

## Layout

| Path | What |
|------|------|
| `crates/types` | Serde types shared by backend and UI (must stay wasm-compatible) |
| `crates/core` | Cluster logic, no UI deps: contexts, discovery, live watches, table rows, safety gate |
| `src-tauri` | Tauri app: IPC commands and events bridging `core` ↔ UI |
| `ui` | Leptos CSR frontend, built by Trunk (`Trunk.toml` at repo root) |

## Prerequisites

- rustup toolchain (not Homebrew `rust`) with `wasm32-unknown-unknown`
- `cargo install --locked trunk@0.21.14 tauri-cli@2.12.1`

## Run

```sh
cargo tauri dev                  # app with hot-reloading UI
cargo tauri build                # .app / .dmg in target/release/bundle
cargo test -p subduer-core       # unit tests
cargo run -p subduer-core --example smoke -- <context>   # read-only CLI check
```

## Behaviour

- **Navigation by kind.** All listable kinds come from API discovery. Nodes and Events are listed at the top of the sidebar. Everything else is sorted into collapsible sections: Workloads, Config, Network, Storage, Namespaces, Access Control, Helm (releases), AWS, Google Cloud and Azure (shown only when present), Custom Resources (CRDs and aggregated APIs, grouped by API group) and Other. Kinds have icons, including well-known projects (Kyverno, Datadog, KEDA, External Secrets, Prometheus Operator). Sections start collapsed. Drag a section header to reorder the sections; the order is saved. The section map is in `crates/core/src/catalog.rs`.
- **Namespace filter.** One namespace or all of them. The choice stays when you switch kinds and is ignored for cluster-scoped kinds.
- **Cluster page.** Nodes, pods, namespaces and recent warnings, plus memory and CPU (available, used, requested and limits). Also requests by namespace and a per-node table. It refreshes every 30 s. "Used" comes from metrics-server, Prometheus or kubelet stats, and the last hour is charted when Prometheus is available.
- **Nodes.** The Node kind uses the Cluster page's node table, with per-node pods and requests, and usage when metrics-server is available. It refreshes every 30 s, and clicking a row opens its YAML.
- **Column widths.** Drag a column header's right edge to resize it. Widths are saved per kind (the node tables share one setting). Double-click the edge to reset.
- **Pod containers.** The Init and Ready columns show one icon per init container and app container respectively: ✓ running, ◐ starting, ■ stopped, ✕ failed. Hover for details.
- **Pod details.** Selecting a pod opens a Details tab (YAML is still one click away), refreshed every 15 s. It shows pod resources (used, requested and limit), pod info, and each container with its resources, state, ports and environment, then events. Every section collapses. Secret-backed variables stay masked until you click Show, which fetches that one value.
- **Deployment and StatefulSet details.** The same layout for workloads: usage summed over their pods (last hour), status, replicas, rollout strategy and conditions, a pods table (click a pod to open it), each template container with its per-pod resources and environment, and the events of the workload, its ReplicaSets and its pods.
- **Config details.** Every kind in the Config section gets a Details tab. ConfigMaps and Secrets list their keys and the pods that use them. HPAs show their metrics against targets, PDBs whether evictions are allowed, and quotas how close each resource is to its limit. Webhooks and admission policies show what they apply to and what happens when they fail. Leases show who holds them. Links open the related object.
- **Network details.** Services show their ports, the pods behind them (ready or not) and the Ingresses routing to them. Ingresses show each rule's backend health, flag missing Services and certificate Secrets, and offer their URLs to open in the browser. NetworkPolicies spell out what they let in and out, and which pods they apply to. EndpointSlices and Endpoints list each address and its pod.
- **Edit ConfigMaps and Secrets.** Edit values, add, rename and remove keys, then Save. Secret values stay masked until you click Show or start editing. A save is refused if someone else changed the object since you started. Nothing is saved until you click Save: leaving an edit with unsaved changes (another object, another tab, closing the window, quitting) asks whether to discard them first.
- **Job and CronJob details.** The same layout again. Jobs show completions, failures against the retry limit, start, finish and duration. CronJobs show the schedule and time zone, the next run, the last scheduled and last successful runs, and a table of their Jobs (click one to open it).
- **Container actions.** Logs (also merged across a workload's pods) and Shell open in their own windows, listed under the Window menu. Port-forward pods and Services (Port Forwards page). Upload files into, and download files out of, containers, with a folder browser that shows writable locations.
- **Delete** any object whose kind allows it, after a confirmation that names the context.
- **Restart** (rollout restart) and **Scale** Deployments and StatefulSets, after the same kind of confirmation.
- **Run now** a CronJob (like `kubectl create job --from=cronjob/…`), and **Suspend/Resume** CronJobs and Jobs, after the same kind of confirmation.
- **Edit range** of an HPA (min and max replicas).
- **Live tables.** Each table is backed by a Kubernetes watch, with snapshots throttled to 250 ms.
- **Read-only by default.** Every context starts locked. To unlock a production-like context (a name containing `prd`/`prod`/`production`/`live`), you have to type its name. All mutations go through `SafetyGate::ensure_writable`: Delete, Restart, Scale, Run now, Suspend/Resume, Save, Edit range, Shell and Upload ask to unlock first.

## License

MIT, see [LICENSE](LICENSE). Copyright (c) 2026 it-problems.
