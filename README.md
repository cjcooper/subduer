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

- **Navigation by kind.** All listable kinds come from API discovery. Nodes and Events are listed at the top of the sidebar. Everything else is sorted into collapsible sections: Workloads, Config, Network, Storage, Namespaces and Access Control. Custom Resources holds CRDs and aggregated APIs, grouped by API group, and Other holds the remaining built-ins. Sections start collapsed. Drag a section header to reorder the sections; the order is saved. The section map is in `crates/core/src/catalog.rs`.
- **Namespace filter.** One namespace or all of them. The choice stays when you switch kinds and is ignored for cluster-scoped kinds.
- **Cluster page.** Nodes, pods, namespaces and recent warnings, plus memory and CPU (available, used, requested and limits). Also requests by namespace and a per-node table. It refreshes every 30 s. "Used" needs metrics-server.
- **Nodes.** The Node kind uses the Cluster page's node table, with per-node pods and requests, and usage when metrics-server is available. It refreshes every 30 s, and clicking a row opens its YAML.
- **Column widths.** Drag a column header's right edge to resize it. Widths are saved per kind (the node tables share one setting). Double-click the edge to reset.
- **Pod containers.** The Init and Ready columns show one icon per init container and app container respectively: ✓ running, ◐ starting, ■ stopped, ✕ failed. Hover for details.
- **Pod details.** Selecting a pod opens a Details tab (YAML is still one click away), refreshed every 15 s. It shows pod resources (used, requested and limit), pod info, and each container with its resources, state, ports and environment, then events. Every section collapses. Secret-backed variables stay masked until you click Show, which fetches that one value.
- **Live tables.** Each table is backed by a Kubernetes watch, with snapshots throttled to 250 ms.
- **Read-only by default.** Every context starts locked. To unlock a production-like context (a name containing `prd`/`prod`/`production`/`live`), you have to type its name. All mutations must go through `SafetyGate::ensure_writable`.

## License

MIT, see [LICENSE](LICENSE). Copyright (c) 2026 it-problems.
