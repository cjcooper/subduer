# k8s-subduer runbook

How to pick this project back up. It covers what exists, how to run and verify it, the decisions behind it, and what's next. `README.md` is the short user-facing summary; this file is the working notes.

_Last updated: 2026-10-10._

## 1. What it is

A macOS-first desktop app for browsing and controlling Kubernetes clusters. It's an it-problems project, so never put employer names in code, config, docs or tests; use generic names like `acme-prd`.

- **Stack:** Rust everywhere, using a Tauri v2 shell, a `kube-rs` backend and a Leptos 0.8 UI built to WASM with Trunk. Use another language only where the OS forces it.
- **Navigation:** by kind, with a namespace filter that persists across kinds.
- **Safety:** read-only by default. Delete is the only mutation so far, behind the safety gate.

## 2. Getting set up

| Need | How |
|---|---|
| Rust via **rustup** (not Homebrew `rust`) | `brew install rustup`. The package is keg-only, so put `/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin` first on `PATH`, then run `rustup default stable`. Homebrew's formula no longer ships `rustup-init`. |
| WASM target | `rustup target add wasm32-unknown-unknown` |
| Tools | `cargo install --locked trunk@0.21.14 tauri-cli@2.12.1` |
| Icon tools (optional) | `rsvg-convert` (Homebrew `librsvg`) |

Versions in use: Rust 1.99, tauri 2.12, leptos 0.8, kube 4.2, k8s-openapi 0.28 with the `latest` feature (v1.36). Run `cargo info <crate>@<major>` to see newer releases.

## 3. Daily commands

```sh
cargo tauri dev                       # run the app (Trunk serves the UI on :1420)
cargo tauri build                     # .app/.dmg in target/release/bundle
cargo test                            # core + types unit tests (backend crates)
cargo clippy --all-targets            # backend lint, kept at zero warnings
cargo clippy -p subduer-ui --target wasm32-unknown-unknown   # UI lint, zero warnings
trunk build                           # UI only, into dist/
```

### Read-only checks against a real cluster (no GUI)

```sh
cargo run -p subduer-core --example smoke    -- <context> [group/Kind]   # kinds, namespaces, first table snapshot
cargo run -p subduer-core --example overview -- <context> [--json]      # Cluster page data
cargo run -p subduer-core --example pod      -- <context> <ns> <pod>     # Pod Details data, never fetches secret values
cargo run -p subduer-core --example history  -- <context> <ns> <pod>     # last-hour CPU/memory; shows which Prometheus it found
cargo run -p subduer-core --example helm     -- <context> <ns> <release> # Helm release summary (no values/manifest)
cargo run -p subduer-core --example delete_dry_run -- <context> <group/Kind> <ns|-> <name>   # server-side dry run, never deletes
```

- Use **`eg-uat-uat-v2`** for real-cluster checks. Docker Desktop's Kubernetes is usually off.
- **Never point anything that writes at production.** All the examples are read-only (`delete_dry_run` uses the server's `dryRun=All`).
- **zsh doesn't word-split** unquoted variables. When passing `ns pod` from a variable, use `${=var}` or pass the words separately.

### Checking the UI without the app window (headless Chrome harness)

This is how the UI has been visually verified so far. Nothing is committed for it, so rebuild it each time:

1. Run `trunk build`, then copy `dist/` to a scratch folder.
2. Inject a `<script>` at the top of `<head>` in the copied `index.html` that defines `window.__TAURI__`:
   - `core.invoke(cmd, args)` returns canned replies per command: `list_contexts`, `connect`, `cluster_overview`, `pod_details`, `secret_value`, `stop_watch`, and so on.
   - `event.listen(name, cb)` stores `cb`. When `watch_resources` is invoked, call `cb({payload: <ResourceTable>})`.
3. Optionally script clicks with `setTimeout`, for example opening a sidebar section, clicking a kind or a row, or pressing Show.
4. Serve the folder with `python3 -m http.server 8765` and capture it:

   ```sh
   "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --disable-gpu --hide-scrollbars \
     --blink-settings=preferredColorScheme=1 --force-device-scale-factor=2 \
     --window-size=1400,900 --virtual-time-budget=9000 --screenshot=out.png "http://localhost:8765/index.html?..."
   ```

   Use `preferredColorScheme=0` for dark mode. The Mac defaults to dark otherwise.

Encoding gotcha: a `Containers` cell is `state␞name␞detail␞(i|)`, with entries separated by `␟`. The real separators are the control characters `\u001e` and `\u001f`; see `subduer_types::containers`.

A good follow-up is to commit this harness under `tools/ui-harness/` with JSON fixtures.

## 4. Layout and data flow

```
crates/types   serde types shared by backend + UI (must stay wasm-compatible)
crates/core    all cluster logic, no UI deps
  contexts.rs    kubeconfig contexts, per-context Client (exec auth works, e.g. aws eks get-token)
  safety.rs      SafetyGate: every context locked until unlocked; is_prod_like()
  cluster.rs     discovery (+ hidden kinds), namespaces, get_yaml
  catalog.rs     sidebar sections + TopLevel; CRD detection via CRD names (metadata-only list)
  table.rs       columns + rows per kind, pod status (kubectl rules), container states, events
  watch.rs       watch -> full snapshot, throttled to 250 ms
  overview.rs    Cluster page: nodes, pods, requests/limits, usage (metrics-server → Prometheus → kubelet), last-hour history, warnings (1h)
  pod_details.rs Pod Details tab + secret_value; usage from metrics-server, else kubelet /stats/summary via node proxy
  actions.rs     mutations; each checks SafetyGate first (delete so far)
  exec.rs        interactive shells (kubectl exec -it) over the websocket; callers check the gate
  logs.rs        container log streams: one container, or a workload's pods merged (stern-style)
  forward.rs     port-forwarding: 127.0.0.1 listener, one portforward stream per connection; services resolve to a ready pod
  helm.rs        Helm releases as a virtual kind (helm.sh/Release) from owner=helm Secrets
  usage.rs       kubelet /stats/summary helpers (per pod, per node totals)
  history.rs     last-hour pod usage: Prometheus (found once per connection) or app-recorded samples
  quantity.rs    "500m"/"1.5Gi" -> numbers
src-tauri      Tauri app: AppState (gate, session, one watch), commands.rs, shell_env.rs
ui             Leptos CSR app
  app.rs         Store (all signals), top bar, sidebar, resource table, detail panel, container icons
  overview.rs    Cluster page + Node page (shared NodeTable), refresh loop
  pod_detail.rs  Pod Details tab (collapsible sections, bullet bars, env + secret reveal, events)
  columns.rs     resizable columns (localStorage)
  prefs.rs       localStorage helpers (sidebar section order)
  tauri.rs       invoke/listen bindings to window.__TAURI__
```

### IPC commands (`src-tauri/src/commands.rs`)

| Command | Purpose |
|---|---|
| `list_contexts` | Contexts, with prod-like and unlocked flags |
| `connect` | Client, discovered kinds and namespaces |
| `set_unlocked` | Safety gate |
| `watch_resources` / `stop_watch` | One live watch. Snapshots arrive as `resource-table` events tagged with `watch_id`; the UI drops stale ids. |
| `get_yaml` | One object without `managedFields` |
| `cluster_overview` | Cluster page and Node page data |
| `pod_details` | Pod Details tab data |
| `pod_history` | Last hour of pod and container CPU/memory, polled every 15 s |
| `delete_resource` | Deletes one object; needs the context unlocked and still connected |
| `open_logs` | Opens (or focuses) a logs window bound to the current cluster |
| `logs_start` | Called by a logs window: (re)starts its stream, events on the window's own name |
| `open_exec` | Opens a shell window (needs the context unlocked) |
| `exec_start` / `exec_input` / `exec_resize` | Called by a shell window: (re)connect (re-checks the gate), keystrokes, terminal size |
| `forward_ports` / `start_forward` / `list_forwards` / `stop_forward` / `open_forward` | Port-forward dialog, start, list, stop, open in browser; changes arrive as `port-forwards` events |
| `secret_value` | One Secret value, on explicit click |

### How things fit together

- **One `Store`** holds every UI signal. It's `Copy` and passed to components.
- **Cluster and Node pages** run on `cluster_overview` polling (every 30 s, reused if under 10 s old), not on watches.
- **Every other kind** uses the live watch.
- **Pod Details** polls every 15 s while open.

## 5. What's built

- **Contexts and safety:** the context picker marks prod-like contexts with ⚠. The lock toggle uses an in-app modal; prod contexts require typing the name. JS `prompt` and `confirm` are avoided because WKWebView may not support them.
- **Sidebar:**
  - **Cluster** page, **Node** and **Event** at the top level.
  - Collapsible sections: Workloads, Config, Network, Storage, Namespaces, Access Control, Helm, AWS, Google Cloud, Azure, Custom Resources, and Other. The cloud sections and Custom Resources are grouped by API group. Empty sections are hidden, so a cloud section appears only when the cluster has that provider's kinds.
  - Sections start closed and can be reordered by dragging. The order persists, and new sections slot into their default position.
  - `events.k8s.io/Event` is hidden as a duplicate of `core/Event`.
- **Tables:**
  - Typed columns (`ColumnKind`), resizable and saved per kind; double-click a header edge to reset.
  - Badges for status-like columns.
  - Pod **Init** and **Ready** columns use per-container icons: ✓ running, ◐ starting, ■ stopped, ✕ failed. Shape and colour both carry the state, which keeps it readable for colour-blind users.
- **Pod status** follows kubectl's rules, with one change: exit 0 combined with `OOMKilled` shows as **`Completed (OOM)`** in amber. This came from a real UAT migration job where a child process was OOM-killed but the job succeeded.
- **Events:** columns are Type, Message, Namespace, Involved Object, Source, Count, Age and Last Seen, newest first. Values fall back to the newer `series`, `eventTime` and `reporting*` fields when the classic ones are missing.
- **Cluster page:** KPI tiles; memory (the headline number) and CPU, each with used, requested and limit meters; pods by status as a stacked bar; requests by namespace (top 8 plus Other); and a nodes table. Chart colours follow the dataviz skill palette (series blue plus the fixed status palette).
- **Node page:** the same table as the Cluster page, with selectable rows that open YAML.
- **Pod Details tab:** collapsible sections for resources (last-hour line charts with request and limit lines), pod info, each container (resources, state, ports, environment) and events. The collapsed state persists. Secret values are masked until **Show**, forgotten when the selection changes, and never logged.
- **App icon:** `src-tauri/icons/make_icon.py` generates `icon.svg`, a 7-spoke helm inside a control dial. Regeneration steps are in the script's header.

## 6. Decisions and why

- **Read-only by default.** The user's kubeconfig includes production EKS. Every mutation must call `SafetyGate::ensure_writable(context)` first. The gate may be removed later, and that's designed to be a one-function change.
- **Node and Cluster pages poll instead of watching.** Per-node pod counts and requests need a cluster-wide pod list, so they refresh every 30 s instead of live.
- **Secret reveal is a read and isn't gated by the lock.** This is an open question: the user may want unlocking required, which would be one check in the `secret_value` command.
- **Pod limits:** a pod-level limit is shown only if every long-running container has one.
- **No limit means the node's size.** Without a limit, the header shows `limit unlimited (node X)` and the share is against the node's allocatable. The chart's y-axis still fits the data and request/limit lines (not the node), so trends stay visible.
- **Delete** (`actions::delete`, `ui/src/delete.rs`):
  - The button sits in the detail-panel header for any kind whose discovery verbs include `delete`.
  - A locked context opens the unlock prompt first, then continues to the delete confirmation.
  - The dialog names the object, namespace and context, and says what else goes with it (namespace contents, CRD objects, pods of workloads, PV data).
  - You must type the object name on production-like contexts and for cascading kinds (Namespace, Node, PersistentVolume, CRD).
  - The command re-checks that the dialog's context is still the connected one.
  - Background propagation, matching kubectl. No force or grace-period option yet.
  - Tested only by server-side dry run on UAT.
- **Helm** (`helm.rs`):
  - Releases are a virtual kind, `helm.sh/Release`, listed only if any `owner=helm` Secret exists. Verbs are list/watch/get, so there's no Delete: deleting the Secret isn't `helm uninstall`.
  - The list is a metadata-only watch, one row per release at its latest revision.
  - Chart and app version are decoded from the latest revision's payload (Helm base64, then gzip, then JSON). `config`, `manifest`, `hooks` and everything in `chart` except `metadata` are dropped right away.
  - **Updated** is the revision Secret's creation time. The `modifiedAt` label is stale on UAT (identical across revisions).
  - The detail panel shows a summary (chart, status, notes, revision history) instead of YAML.
- **Sidebar icons** (`ui/src/icons.rs`): hand-drawn 24×24 stroke icons in `currentColor`, one per built-in kind and section. Unknown custom resources get a puzzle piece and unknown built-ins a box. Well-known projects get descriptive icons per kind and on their group heading: Kyverno (`kyverno.io`, `policies.`/`reports.kyverno.io`, `wgpolicyk8s.io`), Datadog (`datadoghq.com`), Prometheus Operator (`monitoring.coreos.com`), KEDA (`keda.sh`, `eventing.keda.sh`), External Secrets (`external-secrets.io`, `generators.`) and the cloud sections (AWS, Google Cloud, Azure; config-like kinds get sliders, the rest a cloud). These are drawn icons, not project logos (trademarks). Add more in `custom_paths`/`group_paths`. The group chevron is a 14px stroked SVG in body ink, also used in the Pod Details sections.
- **Cluster page usage:**
  - Order: metrics-server; else Prometheus `sum by (node)` of cAdvisor container usage, with kubelet stats for busy nodes Prometheus doesn't report; else kubelet stats for every busy node. Nodes with no running pods count as 0.
  - Every source counts usage as the sum of containers, so mixing them stays consistent.
  - On UAT, Prometheus misses about 17 of 50 nodes: Fargate nodes holding only finished Job pods, plus the node running Prometheus itself.
  - The last-hour charts (step 30 s, matching the page refresh) come from Prometheus only, so they exclude nodes it doesn't scrape. Expect them to sit a little below the "Used" meter (about 0.8 GiB on UAT).
  - The charts have no request or allocatable lines: at cluster scale those flatten the trend. The header shows the hour's peak instead.
- **Logs windows** (`src-tauri/src/logs.rs`, `ui/src/logs.rs`, `crates/core/src/logs.rs`):
  - Pods get a Logs button per container section. Workloads (Deployment, StatefulSet, DaemonSet, ReplicaSet, Job, ReplicationController) get one in the detail header, which merges every container of up to 20 selected pods. Pods started later aren't added.
  - Each is a native `WebviewWindow` labelled `logs-N`, loading `index.html?logs=logs-N`. `main.rs` mounts `LogsApp` for that query. Capabilities grant `main` and `logs-*`.
  - Opening the same target on the same context focuses the existing window.
  - The window keeps the client of the cluster it was opened from, so switching context in the main window doesn't affect it.
  - Once listening, the window calls `logs_start`. Batches arrive on an event named after the window label, because the JS global `listen` hears all windows' events.
  - The stream stops when the window is destroyed. Closing the main window closes all logs windows.
  - The stream keeps the tail (1000 lines, or 200 per container when merged), follows, adds timestamps and strips ANSI. Merged batches are sorted by time.
  - The window shows filter, Wrap, Time (local), Previous (last terminated container) and Reload. It keeps 10k lines, auto-follows, and scrolling up pauses following.
  - `menu.rs` marks the "Window" submenu as macOS's windows menu, so open windows (main and logs) are listed there with their titles.
- **Shell windows** (`src-tauri/src/exec.rs`, `ui/src/exec.rs`, `crates/core/src/exec.rs`):
  - Container sections get a Shell button, only while the container is running. It's gated like Delete: a locked context opens the unlock prompt, then the shell opens.
  - `open_exec` and every `exec_start` (reconnect) call `ensure_writable`.
  - Windows are labelled `exec-N` and load `index.html?exec=exec-N`; capabilities grant `exec-*`. Each click opens a new session; repeats get a number in the title.
  - When the shell process exits (`exit`, Ctrl-D, even with a non-zero code: status `Success` or reason `NonZeroExitCode`), the backend closes the window. Any other ending (no shell, dropped connection, gate) leaves the window open with the message and Reconnect. Checked on UAT for `exit`, `false; exit` and a distroless image.
  - The command is `/bin/sh -c 'exec bash, else sh'`. Distroless images report "This image has no shell".
  - Output is base64 in `ExecOutput::Data` (chunks can split UTF-8). Input goes one invoke per keystroke chunk; resizes go through the fit add-on.
  - Terminal: xterm.js 5.5.0 and addon-fit 0.10.0, vendored in `ui/vendor/xterm` (integrity-checked, MIT; see its README) and loaded as plain scripts in `index.html`. The light theme darkens the ANSI palette.
  - Tested on UAT with `examples/exec_check.rs`, which echoes, runs `uname` and exits, and refuses prod-like contexts. The datadog cluster-agent got a bash session; keda-operator is distroless.
- **Port-forwarding** (`crates/core/src/forward.rs`, `src-tauri/src/forward.rs`, `ui/src/forward.rs`):
  - Forward buttons on running pod containers that declare ports, and on Service detail panels. The dialog lists TCP ports and takes a local port (default the same, 0 for any).
  - Listens on 127.0.0.1 only. Each connection opens its own kube `portforward`, and kubelet errors (e.g. "connection refused") land in Last error.
  - Services resolve like kubectl: a Ready pod matching the selector, with named target ports looked up in the pod. They re-resolve after a failed connection.
  - Forwards live in `AppState.forwards`, keep their own client (they survive context switches), and stop on Stop or app quit. Changes push the whole list to the main window as `port-forwards` events.
  - Open uses the OS opener (`open`, `xdg-open`, `start`) on `http://127.0.0.1:<port>/`; Copy uses `navigator.clipboard`.
  - **Not gated:** a forward changes nothing in the cluster. Writes made through it come from the user's own client.
  - Tested on UAT with `examples/forward_check.rs` (read-only HTTP GET): Prometheus 9090 gave 200, and 8080 resolved the named target port.
- **Usage history** (`history.rs`): Prometheus first, found by listing services for a non-headless `*prometheus*` service on port 9090 that answers `query=1`, reached through the API server's service proxy. Queries are cAdvisor `container_cpu_usage_seconds_total` (rate 2m) and `container_memory_working_set_bytes`, `max by (container)` to survive double scrapes, step 15 s aligned to the step. If Prometheus is missing or has no series for the pod, the app records a sample per request (metrics-server, else kubelet stats), kept for an hour per connection. UAT's Prometheus doesn't cover every pod (its own pod isn't in cAdvisor data), so both paths get used.
- **Charts** (`ui/src/chart.rs`): one series per chart, hand-rolled SVG stretched with `preserveAspectRatio="none"` plus `non-scaling-stroke`; dots and text are HTML overlays so they don't distort. Gaps over 2.5 steps break the line. Crosshair tooltip on hover.
- **Cloud sections** (`Category::cloud_for_group`):
  - AWS: `*.k8s.aws`, `*.amazonaws.com`.
  - Google Cloud: `*.gke.io`, `cloud.google.com` (including Config Connector `*.cnrm.cloud.google.com`), `*.googleapis.com`.
  - Azure: `*.azure.com` (ASO, AKS `*.kubernetes.azure.com`), `azure.microsoft.com`, `aadpodidentity.k8s.io`.
  - Only custom (non-built-in) groups are checked. Matching is on whole domain labels, so `myazure.com` stays generic.
- **Discovery uses `resources_by_stability()`**: every kind at its most stable version. The previous `recommended_resources()` took only the group's preferred version, which hid kinds that exist only elsewhere. On UAT that was `CNINode` (`v1alpha1` in a `v1beta1` group) and `PushSecret`/`ClusterPushSecret`.
- **CRD detection** uses `<plural>.<group>` names from a metadata-only CRD list, because full CRDs are megabytes. It falls back to treating non-built-in API groups as custom.
- **Watches** send full snapshots (throttled), not diffs. That's simple and fine at current sizes; revisit for very large clusters.
- **`dragDropEnabled: false`** in `tauri.conf.json`, so HTML5 drag and drop works for section reordering. The app doesn't need file drops.
- **About panel** (`src-tauri/src/menu.rs`): Tauri's default menu, but the About item is rebuilt with an embedded icon (`icons/128x128@2x.png`, needs tauri's `image-png` feature) and "An it-problems application." credits. Without the explicit icon, unbundled runs (`cargo tauri dev`) show a generic icon. `bundle.publisher`/`copyright` in `tauri.conf.json` name it-problems.
- **GUI launched from Finder** gets a minimal `PATH`. `shell_env.rs` copies the login shell's `PATH` so `aws` exec auth works.

## 7. Next up

These are the v1 actions the user asked for at the start. All must go through the safety gate and be tested on docker-desktop or UAT, never prod:

1. ~~**Delete**~~ done: detail-panel button for any kind with the `delete` verb (see section 6).
2. **Scale / restart.** Patch replicas; restart by setting a `kubectl.kubernetes.io/restartedAt` annotation.
3. **Edit YAML and apply,** using CodeMirror in the webview with a diff before applying and server-side apply.
4. ~~**Logs / exec / port-forward**~~ done.

Smaller items that have come up:

- Delete follow-ups: force delete (grace 0) for pods stuck Terminating; multi-select delete from the table; a uid precondition so a recreated same-name object isn't deleted.


- Decide whether revealing secrets should require an unlocked context.
- Add a `Reason` column to Events (offered, not yet requested).
- Add a "reset section order" option for the sidebar (offered).
- Add column sorting by clicking headers. Not built; Events are sorted by Last Seen on the backend.
- Commit the UI test harness (section 3).
- Clean up unused icons that `cargo tauri icon` generated (`android/`, `ios/`, `Square*.png`, `StoreLogo.png`, `icon.ico`). The user hadn't approved deleting them.
- `response.json` at the repo root (`{"status":"success"}`) has an unknown origin and wasn't created during this work.

## 8. Working conventions

- **Git:** the user runs all git commands; a hook blocks git for Claude. Claude only edits files.
- **Before handing back:** keep lint at zero warnings for both clippy commands, run `cargo test`, and run `trunk build`.
- **Visual changes:** check with the headless harness in both light and dark mode.
- **Real-cluster checks:** read-only against UAT. Nothing writes to a cluster yet.
- **Charts:** load the dataviz skill before adding any chart.
