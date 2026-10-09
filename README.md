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

- **Navigation by kind.** All listable kinds come from API discovery. The sidebar sorts them into collapsible sections: Workloads, Config, Network, Storage, Namespaces and Access Control. Custom Resources holds CRDs and aggregated APIs, grouped by API group, and Other holds the remaining built-ins. Sections start collapsed. Drag a section header to reorder the sections; the order is saved. The section map is in `crates/core/src/catalog.rs`.
- **Namespace filter.** One namespace or all of them. The choice stays when you switch kinds and is ignored for cluster-scoped kinds.
- **Live tables.** Each table is backed by a Kubernetes watch, with snapshots throttled to 250 ms.
- **Read-only by default.** Every context starts locked. To unlock a production-like context (a name containing `prd`/`prod`/`production`/`live`), you have to type its name. All mutations must go through `SafetyGate::ensure_writable`.
