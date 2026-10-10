# k8s-subduer runbook

How to pick this project back up. It covers what exists, how to run and verify it, the decisions behind it, and what's next. `README.md` is the short user-facing summary; this file is the working notes.

_Last updated: 2026-10-10 (Jobs and CronJobs)._

## 0. Where things stand (read first)

- **Committed up to `22df4e7`** (workload Details tab); the user commits. Not yet committed:
  - **Jobs and CronJobs:** a Details tab for both, Run now and Suspend/Resume on CronJobs, and Suspend/Resume on Jobs. Files: `crates/core/src/cron.rs` (new), `crates/core/src/workload.rs`, `crates/core/src/actions.rs`, the types, the commands, the `workload` and `workload_dry_run` examples, `ui/src/workload.rs`, `ui/src/workload_detail.rs`, `ui/src/app.rs`, CSS, `tools/ui-harness/batchstub.js` (new) and docs.
  - **Config section:** Details tabs for all 13 Config kinds; editable ConfigMap and Secret data with Save; Edit range on HPAs. Files: `crates/core/src/config.rs` (new), `actions.rs` (`save_data`, `set_hpa_range`), `pod_details.rs` (`secret_values`), the types, the commands, `examples/config.rs` and `examples/config_dry_run.rs` (new), `ui/src/object_detail.rs` (new), `ui/src/workload.rs` (Save and Edit range in the shared dialog), `ui/src/app.rs`, `ui/src/pod_detail.rs`, CSS, `tools/ui-harness/configstub.js` (new) and docs.
  - **Unsaved-edit guard:** `app::guard`, the discard prompt, `set_unsaved`/`close_discarding` and the window close and Cmd+Q hold in `src-tauri/src/lib.rs`.
  - **Network section:** Details tabs for Service, Ingress, IngressClass, NetworkPolicy, EndpointSlice and Endpoints; Open buttons for Ingress URLs. Files: `crates/core/src/network.rs` (new), `config.rs` (dispatch), the types (`InfoRow.url`, `has_object_details`), `open_url` in the commands (`forward::open_in_browser` shared), `ui/src/object_detail.rs`, CSS, `tools/ui-harness/networkstub.js` (new) and docs.
- **Verified:** clippy is clean (backend, and UI on wasm32), `cargo test` passes (53 core + 1 tauri + 1 types; the UI icon test runs via `cargo test -p subduer-ui`), and `trunk build` succeeds. Each feature was checked against UAT through its `examples/*` script and visually in the headless harness, light and dark.
- **Not yet seen in the real app window** (macOS blocks scripted clicks and screenshots): the unsaved-edits hold on window close and Cmd+Q, logs, shell and port-forward windows, their Window-menu entries, the shell window closing on exit, Open/Copy on port-forwards, real drag-and-drop in Upload, and Show in Finder. The user had `cargo tauri dev` running; restart it to pick up backend changes.
- **Untested:** downloading multi-MB files end to end (the user declined the SHA check command).
- **Checked on UAT (2026-10-10):** `workload` and `workload_dry_run` against a Deployment, a StatefulSet, two CronJobs (one with `timeZone: America/New_York`) and completed and failed Jobs. Every dry run was accepted, and the locked gate refused each real call. UAT's Prometheus has no series for the short CronJob pods, so their history falls back to local samples; Deployment and StatefulSet history come from Prometheus.
- **Network section checked on UAT (2026-10-10):** `config` read a NodePort Service, a headless Service, an EndpointSlice, Endpoints and the `alb` IngressClass; endpoints, links and statuses matched `kubectl`. UAT has no Ingresses or NetworkPolicies, so those are covered by unit tests and the harness only.
- **Config section checked on UAT (2026-10-10):** `config` read an HPA (KEDA-owned, external metric), the coredns PDB, a Helm-managed ConfigMap, a Secret (sizes only), a webhook configuration, a Lease and a PriorityClass. `config_dry_run` was accepted for a ConfigMap save, a Secret save and an HPA range. A save from a stale resourceVersion was refused with the "changed by someone else" message, and the locked gate refused the real calls. Nothing was written. The dry-run key was not persisted.
- **Open questions for the user:**
  - Should port-forwards on prod contexts require unlocking? (Currently not gated.)
  - Should revealing secrets require unlocking?
  - Should logs and shell windows stay open when the main window closes? (Currently they close with it.)


## 1. What it is

A macOS-first desktop app for browsing and controlling Kubernetes clusters. It's an it-problems project, so never put employer names in code, config, docs or tests; use generic names like `acme-prd`.

- **Stack:** Rust everywhere, using a Tauri v2 shell, a `kube-rs` backend and a Leptos 0.8 UI built to WASM with Trunk. Use another language only where the OS forces it.
- **Navigation:** by kind, with a namespace filter that persists across kinds.
- **Safety:** read-only by default. Mutations (Delete, Restart, Scale, Run now, Suspend/Resume, data Save, Edit range, Shell, Upload) are behind the safety gate.

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
cargo run -p subduer-core --example workload -- <context> <apps/Deployment|apps/StatefulSet|batch/Job|batch/CronJob> <ns> <name>   # Details data + history source; no env values or secrets
cargo run -p subduer-core --example workload_dry_run -- <context> <group/Kind> <ns> <name> [replicas]   # dry-run restart/scale, or suspend/resume and (CronJob) Run now; checks the gate refuses
cargo run -p subduer-core --example config -- <context> <group/Kind> <ns|-> <name>   # Config- and Network-section Details data; ConfigMap values by size only, Secret values never fetched
cargo run -p subduer-core --example config_dry_run -- <context> <core/ConfigMap|core/Secret|autoscaling/HorizontalPodAutoscaler> <ns> <name>   # dry-run save (adds key subduer-dry-run) or HPA range; checks stale versions and the gate
```

- Use **`eg-uat-uat-v2`** for real-cluster checks. Docker Desktop's Kubernetes is usually off.
- **Never point anything that writes at production.** All the examples are read-only (`delete_dry_run` and `workload_dry_run` use the server's `dryRun=All`).
- **zsh doesn't word-split** unquoted variables. When passing `ns pod` from a variable, use `${=var}` or pass the words separately.

### Checking the UI without the app window (headless Chrome harness)

This is how the UI has been visually verified so far. Nothing is committed for it, so rebuild it each time:

1. Run `trunk build --dist <scratch>/build` and work from that copy. A running `trunk serve` rewrites `dist/` on every change, so copying it mid-rebuild gives a blank page.
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

The stubs are in `tools/ui-harness/` (see its README for the query parameters each one takes).

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
  workload.rs    Deployment/StatefulSet/Job/CronJob Details tab: status, owned pods (and a CronJob's Jobs), template containers, events
  config.rs      Config-section Details tabs: one generic ObjectDetails (info rows, sections, tables) per kind, read as JSON at the served version; ConfigMap/Secret data and the pods using them
  network.rs     Network-section Details tabs on config's shape: Service endpoints and routes, Ingress backend health/TLS/URLs, NetworkPolicy in words
  cron.rs        a CronJob schedule's next run (five fields, names, steps, @shorthands, time zones), on jiff
  actions.rs     mutations; each checks SafetyGate first (delete, restart, scale)
  exec.rs        interactive shells (kubectl exec -it) over the websocket; callers check the gate
  logs.rs        container log streams: one container, or a workload's pods merged (stern-style)
  forward.rs     port-forwarding: 127.0.0.1 listener, one portforward stream per connection; services resolve to a ready pod
  helm.rs        Helm releases as a virtual kind (helm.sh/Release) from owner=helm Secrets
  upload.rs      file upload into a container via exec `sh -c 'cat > "$1"'` (gated)
  download.rs    file download out of a container (`cat` streamed to a .part file, then renamed)
  usage.rs       kubelet /stats/summary helpers (per pod, per node totals)
  history.rs     last-hour pod usage: Prometheus (found once per connection) or app-recorded samples
  quantity.rs    "500m"/"1.5Gi" -> numbers
src-tauri      Tauri app: AppState (gate, session, one watch), commands.rs, shell_env.rs
ui             Leptos CSR app
  app.rs         Store (all signals), top bar, sidebar, resource table, detail panel, container icons
  overview.rs    Cluster page + Node page (shared NodeTable), refresh loop
  pod_detail.rs  Pod Details tab (collapsible sections, bullet bars, env + secret reveal, events)
  workload_detail.rs  Deployment/StatefulSet/Job/CronJob Details tab, reusing pod_detail's sections, charts, env and events
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
| `workload_details` / `workload_history` | Deployment/StatefulSet/Job/CronJob Details tab, and its last hour summed over its pods |
| `delete_resource` | Deletes one object; needs the context unlocked and still connected |
| `get_replicas` | Desired and current replicas (scale subresource), for the Scale dialog |
| `restart_workload` / `scale_workload` | Rollout restart, set replicas; checked like `delete_resource` |
| `object_details` | Details tab of a Config- or Network-section kind |
| `open_url` | Opens an http(s) address (Ingress URLs) in the browser; anything else is refused |
| `secret_values` | Every value of a Secret, when editing starts (logged without values) |
| `save_config_data` / `set_hpa_range` | Save edited ConfigMap/Secret data (with its resourceVersion); set an HPA's min/max; checked like `delete_resource` |
| `suspend_workload` / `run_cron_job` | Set a Job's or CronJob's `spec.suspend`; start a Job from a CronJob now (returns its name); checked like `delete_resource` |
| `open_logs` | Opens (or focuses) a logs window bound to the current cluster |
| `logs_start` | Called by a logs window: (re)starts its stream, events on the window's own name |
| `open_exec` | Opens a shell window (needs the context unlocked) |
| `exec_start` / `exec_input` / `exec_resize` | Called by a shell window: (re)connect (re-checks the gate), keystrokes, terminal size |
| `forward_ports` / `start_forward` / `list_forwards` / `stop_forward` / `open_forward` | Port-forward dialog, start, list, stop, open in browser; changes arrive as `port-forwards` events |
| `upload_file` | Raw binary body + `x-upload` header (URL-encoded JSON `UploadTarget`); gated, context re-checked |
| `download_file` / `reveal_download` | Save a container file into ~/Downloads (numbered if taken); show it in Finder |
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
- **Deployment/StatefulSet Details tab** (`crates/core/src/workload.rs`, `ui/src/workload_detail.rs`): opens by default, like pods, and refreshes every 15 s. Sections:
  - **Resources:** last-hour charts summed over the pods. Request and limit lines are the sum over live pods (limits only if every long-running container has one).
  - **Deployment/StatefulSet:** a status badge (Available/Ready, Rolling out, Degraded, Failed, Paused, Scaled to 0), replicas (desired, ready, up to date, available), strategy, revision, last restart, selector and so on, with a nested Conditions section. StatefulSets also show pod management, service, volume claims and claim retention.
  - **Pods:** the pods the workload controls (via its ReplicaSets for a Deployment), sorted by StatefulSet ordinal. Container icons, status, restarts, live CPU and memory, and age. Pods on the previous template get an "old" chip during a rollout. Clicking a row switches to the Pod list and selects that pod (`open_pod` with `pending_select`, applied when the Pod table arrives).
  - **One section per template container:** charts summed over the pods with per-pod limits scaled by pod count (none for init containers that run to completion), then image, per-pod request and limit, ports and environment (shared `EnvTable`, so secret reveal works the same).
  - **Events:** the workload's, its ReplicaSets' and its current pods' events, each labelled with its object. Events of pods already replaced are dropped.
- **Job/CronJob Details tab** (same files): the same layout, built from the same `WorkloadDetails`. `replicas` is `None`, `suspended` is set, and for CronJobs so is `jobs`.
  - **Job:** status Complete, Failed, Suspended, Running or Pending, from its conditions (`SuccessCriteriaMet` and `FailureTarget` count too). Rows include completions, active pods, failed pods against the retry limit, parallelism, indexed progress, started, finished, duration, active deadline and clean-up TTL. A failed Job has no `completionTime`, so its finish time is when the Failed condition was set. Failed, FailureTarget and ReplicaFailure conditions show red when True.
  - **CronJob:** status Suspended, Running or Scheduled. Rows include:
    - Schedule and time zone.
    - Next run (`cron::next_run`). Without a `timeZone` it assumes UTC, the usual controller default.
    - Last scheduled and last successful.
    - Concurrency policy, starting deadline and history limits.
  - **CronJob Jobs table:** its Jobs, newest first, with status, completions, duration and age. Run now Jobs get a "manual" chip. Clicking a Job opens it in the Job list (`open_object`).
  - **Pods:** a Job's pods, or all pods of a CronJob's Jobs (selector `job-name in (…)`), newest first, finished ones included. Resources totals cover running pods; with none running, the template's per-pod request and limit are shown instead.
  - Time rows (Started, Finished, Next run and so on) show "5m ago" or "in 4h" next to the timestamp.
- **Pod containers** get action buttons: Forward (needs declared ports), Upload, Download, Shell (Upload and Shell are gated, and all four need the container running), and Logs.
- **Workloads** (Deployment, StatefulSet, DaemonSet, ReplicaSet, Job, ReplicationController) get merged Logs. Deployments and StatefulSets also get Restart and Scale, CronJobs get Run now, and Jobs and CronJobs get Suspend/Resume (all gated). Services get Forward. Every kind with the `delete` verb gets Delete (gated).
- **Separate windows** for logs and shells, listed under the Window menu. **Port Forwards** page at the top of the sidebar.
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
- **Restart and Scale** (`actions::restart`/`scale`, `ui/src/workload.rs`):
  - Header buttons for Deployments and StatefulSets (`subduer_types::is_scalable_workload`) whose discovery verbs include `patch`. Gated like Delete: a locked context gets the unlock prompt, then the dialog.
  - Restart is `kubectl rollout restart`: a merge patch setting `kubectl.kubernetes.io/restartedAt` (UTC, whole seconds) on the pod template.
  - Scale patches the `scale` subresource, like `kubectl scale`. The dialog reads the live counts (`get_replicas`) and defaults to the desired one; Scale is disabled until the number changes. It warns when scaling to 0, and when scaling a StatefulSet down (highest ordinals go, PVCs stay).
  - Production-like contexts need the name typed, as with Delete. Commands re-check that the dialog's context is still connected (`client_for_change`, shared with delete).
  - DaemonSet restart and ReplicaSet/CRD scaling would be small additions but weren't asked for. No HPA awareness: an HPA will undo a manual scale.
  - Checked in the headless harness (`workloadstub.js`) and by server-side dry run on UAT.
- **Config section Details** (`crates/core/src/config.rs`, `ui/src/object_detail.rs`):
  - One generic shape, `ObjectDetails`: a kind section (status badge, info rows that can be links or relative times, created, labels, nested Conditions and Annotations), optional Data, then any number of `InfoSection`s with rows and/or a table (numeric and mono columns, a fill bar per row, clickable rows), then Events (namespaced kinds). New kinds only need a `describe` arm.
  - Objects are read as JSON through the discovered version (`dynamic_api`), so HPA v1/v2 and admission policies in v1beta1 or v1 both work.
  - **ConfigMap:** keys, size, immutable; data with text values and binary keys by size; Used by (pods using it via env, envFrom, volume or projected volume).
  - **Secret:** type, keys, size; values masked with Show per key (the existing `secret_value`); binary values never shown; Used by also counts image pull secrets. Service-account tokens link to the account.
  - **HPA:** what it scales (link), replicas, min/max, last scaled; Metrics (now vs target for Resource, ContainerResource, Pods, Object and External; v1's CPU target too); Behavior; status Active, At limit, Inactive or Unable to scale from its conditions.
  - **PDB:** min available/max unavailable, healthy vs needed, evictions allowed, selector; matching pods; status Allowing, Blocking, Unhealthy or No pods.
  - **ResourceQuota:** used vs hard per resource with a bar (amber from 90%, red at 100%); scopes; status Within limits, Near limit or At limit. **LimitRange:** min, max, default request and limit, and ratio per type and resource.
  - **PriorityClass, RuntimeClass, Lease:** their fields; a Lease is Held, Expired (renew time plus duration has passed) or Free.
  - **Webhook configurations:** a section per webhook: what it calls (a service link or URL), failure policy in plain words, timeout, side effects, match policy, reinvocation, selectors, match conditions, and a rules table.
  - **ValidatingAdmissionPolicy:** failure policy, params, selectors; Applies to, Excludes, Validations, Variables, Match conditions, Audit annotations and Type warnings tables; Bindings (links), flagged "not enforced" when none. **Binding:** policy link, actions, param ref, narrowing rules.
- **Network section Details** (`crates/core/src/network.rs`, rendered by `object_detail.rs`):
  - **Service:** type, cluster IP (headless explained), load balancer address (status Pending address until one appears), external IPs, selector (or "none: endpoints managed by hand"), session affinity, external/internal traffic policy, IP families. Ports table. Endpoints from its EndpointSlices (`kubernetes.io/service-name`), deduplicated across IPv4/IPv6 slices, with pod links; status Ready, Partly ready, No ready endpoints or No endpoints. Routed from: Ingress rules and default backends that point at it. Forward stays in the header.
  - **Ingress:** class (link), load balancer address, default backend. Rules table with each backend's ready endpoints, or "no such service" (warn rows); status Backend missing, Backend not ready, No address or Ready. TLS table with secret links, marked "(missing)" when the Secret doesn't exist (checked with a metadata-only list: no Secret data is read). **URLs** with Open buttons: https for TLS hosts, http otherwise; wildcard hosts use the load balancer address; regex paths are skipped.
  - **IngressClass:** controller, default class, parameters; the Ingresses using it across namespaces (including class-less ones when it's the default).
  - **NetworkPolicy:** what it applies to; Incoming and Outgoing in words (not restricted, only the rules below, or all blocked), following the `policyTypes` defaulting rules; rule tables with peers in words ("pods app=web in this namespace", "addresses 10.0.0.0/8 except …") and ports (with `endPort` ranges); the pods it currently selects. Status Denies all, Denies incoming, Denies outgoing or Allows listed.
  - **EndpointSlice / Endpoints:** service link, address type, ports, and each endpoint's state (ready, serving, terminating, not ready), target pod, node and zone. Endpoints notes it's deprecated.
  - **Open URL** (`open_url`): only `http://`/`https://` addresses without whitespace or control characters, passed to `open` as one argument.
- **ConfigMap/Secret editing** (`DataSection` in `object_detail.rs`, `actions::save_data`):
  - **Edit** turns each key into a key field and a value textarea; keys can be added, renamed and removed. Binary values can only be kept or removed. Keys are checked as you type (letters, digits, `-`, `_`, `.`; no duplicates).
  - Secret values appear only after **Show** on one key, or **Edit (shows values)**, which fetches them all (`secret_values`). The draft lives only in the component: Cancel, Save, or a new selection drops it.
  - **Save…** opens the shared change dialog (`Op::Save`) listing changed, added and removed keys, with the prod name check; a locked context asks to unlock first. It notes that env-var users need a restart and mounted files update within about a minute.
  - The save is a merge patch of only the changed keys (Secret values base64-encoded; removed ConfigMap keys cleared from `binaryData` too), carrying the `resourceVersion` the edit started from. A 409 becomes "changed by someone else since you started editing".
  - Background refreshes pause while editing (`store.editing`), so they don't replace the draft.
  - **Only Save saves.** Nothing is written on the way out, and the Save dialog's button reads "Back to editing". While a draft differs from the original (`store.dirty`, set by `DataSection`), the Data header shows an "Unsaved" chip. Any way out goes through `app::guard`: another row, close, the YAML tab, Refresh, links (`open_object`), the sidebar (Cluster, Port Forwards, kinds), the context and namespace menus, and the Data section's Cancel. With unsaved changes, `guard` shows "Discard unsaved changes?" (Keep editing / Discard changes) and runs the navigation only on Discard; the menus show their old value until then. A clean edit just ends.
  - **Window close and Cmd+Q:** the UI reports `set_unsaved`, and while it's set the backend holds `CloseRequested` and `ExitRequested` and emits `unsaved-close` (payload: quitting). The UI shows the same prompt, and Discard calls `close_discarding`. Checked in the harness by firing the event; the real window close and Cmd+Q haven't been tried in the app window.
  - Edit is hidden without the `patch` verb and for immutable objects and Helm release records. Helm-managed objects, `kube-root-ca.crt` and controller-owned objects get a warning that edits may be undone.
- **Edit range** (HPAs, `actions::set_hpa_range`, `Op::Range`): min and max replicas (1 ≤ min ≤ max). Warns when the HPA has a controller (e.g. a KEDA ScaledObject), which would put its own range back.
- **Links** between objects (`open_object`) switch to the target kind's list and select the object; a namespace filter that would hide it is switched to its namespace.
- **Run now and Suspend/Resume** (`actions::run_cron_job`/`set_suspended`, same dialog):
  - **Run now** works like `kubectl create job --from=cronjob/<name>`:
    - The Job copies the job template's labels and annotations and adds `cronjob.kubernetes.io/instantiate: manual`.
    - A controller owner reference to the CronJob puts it among its Jobs, so it counts for the concurrency policy.
    - Its name is `<cronjob>-manual-<5 base-36 characters from the time>`, kept within 63 characters.
    - It needs `create` on `batch/Job`.
    - The dialog warns that Forbid skips scheduled runs while it runs, and that Replace may replace it.
  - **Suspend/Resume** merge-patches `spec.suspend` and needs `patch`. The button shows Suspend or Resume based on the loaded details, so it appears only once they've loaded. It's hidden for Complete or Failed Jobs.
  - After any of these dialogs succeeds, the Details tab refreshes.
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
- **File upload** (`crates/core/src/upload.rs`, `src-tauri/src/upload.rs`, `ui/src/upload.rs`):
  - An Upload button on running containers. It's gated like Shell: a locked context opens the unlock prompt, then the dialog.
  - The dialog takes a file chosen or dropped (HTML5 drop works because `dragDropEnabled` is false) and a destination, defaulting to `/tmp/`.
  - The name is optional: a path ending in `/`, or naming an existing directory, gets the original file name. Anything else is the file path.
  - The directory must already exist (exit 18 gives "directory … doesn't exist"; nothing is created). The script prints the final path, which the dialog shows.
  - Folder browser (`browse_dir`, read-only, not gated): one exec per folder lists entries with dir and writable flags (capped at 1000). The first call also returns writable suggestions: /tmp, `$HOME` and `rw` mounts from /proc/mounts, excluding /proc, /sys, /dev and the kubelet-managed /etc files.
  - `[ -w ]` uses access(2), so read-only root filesystems show as read-only.
  - Clicking a folder opens it and points the destination there. Clicking a file targets it (replacing it asks first). Enter in the field browses to its folder.
  - Transfer: the file goes as a raw IPC body via `invoke_bytes`, with no JSON or base64. The backend runs `sh -c 'cat > "$1"' sh <path>` (the path is an argv, never spliced into the script) and drops stdin to send EOF. That uses the v5 exec protocol (K8s 1.30+); kube falls back to closing the websocket.
  - Without overwrite, the script exits 17 if the file exists (read from the status's `ExitCode` cause), and the dialog offers Replace.
  - Limit 512 MiB, since the file is held in memory. No `tar` needed, but distroless images can't receive files (same as `kubectl cp`).
  - Tested on UAT with `examples/upload_check.rs` (refuses prod-like contexts): browse with suggestions, `/tmp/`, `/tmp` exists then replace, an explicit name, a missing dir, a read-only root, then clean-up. The Datadog cluster agent passed every case.
- **File download** (`crates/core/src/download.rs`, `src-tauri/src/upload.rs`, `ui/src/download.rs`):
  - A Download button on running containers. Not gated: it changes nothing.
  - The dialog shares `ui/src/browser.rs` (`FolderBrowser`, `BrowseMode::Download`) with upload. It starts in the container's working directory (`browse_dir` with `.`); unreadable entries are dimmed and disabled (listing entries now carry `readable`). Click or type a file; Enter downloads it.
  - The container runs `[ -e ] || exit 21; [ -f ] || exit 19; [ -r ] || exit 20; exec cat`, giving clear "doesn't exist", "not a regular file" and "can't read" messages.
  - Output streams to `<name>.part` in Downloads and is renamed on success (removed on failure), so it's never held in memory. `unique_path` picks `name (1).ext` like browsers do and sanitises the name.
  - "Show in Finder" runs `open -R`, restricted to the Downloads folder.
  - Tested on UAT with `examples/download_check.rs` (read-only): `/etc/os-release` and `/bin/sh` (130 KB) matched `wc -c`, a repeat was numbered, and `/etc` and a missing path gave clear errors. Multi-MB files weren't checked end to end.
- **Workload history** (`HistoryStore::workload_history`): Prometheus queries match pods by name pattern (`<deploy>-<hash>-<5>`, `<sts>-<ordinal>`, `<job>[-<index>]-<5>`, or `<cronjob>-<minute>|manual-<x>[-<index>]-<5>`), so pods replaced during the hour still count; queries are `sum by (container) (max by (pod, container) …)`. Without Prometheus it records summed samples of the current pods, keyed by kind, namespace and name.
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
2. ~~**Scale / restart.**~~ done for Deployments and StatefulSets, plus Run now and Suspend/Resume for CronJobs and Jobs (see section 6).
3. **Edit YAML and apply,** using CodeMirror in the webview with a diff before applying and server-side apply.
4. ~~**Logs / exec / port-forward**~~ done.

Smaller items that have come up:

- Delete follow-ups: force delete (grace 0) for pods stuck Terminating; multi-select delete from the table; a uid precondition so a recreated same-name object isn't deleted.


- Decide whether revealing secrets should require an unlocked context (this now covers Edit on a Secret, which reveals every value).
- Add a `Reason` column to Events (offered, not yet requested).
- Add a "reset section order" option for the sidebar (offered).
- Add column sorting by clicking headers. Not built; Events are sorted by Last Seen on the backend.
- The harness stubs are saved in `tools/ui-harness/` (uncommitted, with a README): `stub.js` (pod, delete, upload, download), `nav.js` (sidebar), `fwd.js`, `workloadstub.js`, `batchstub.js`, `configstub.js`, `networkstub.js`, `logstub.js`, `execstub.js`, `ov.js`. The user can choose to commit them; tidying them into fixtures would help.
- Logs: add pods that start after the window opens to merged workload logs; add a download/save-logs button.
- Port-forward from workloads (pick a pod), and optional gating for prod.
- Network follow-ups: Gateway API kinds (HTTPRoute, Gateway) when installed; port-forward from an Ingress backend.
- Config follow-ups: TLS Secrets could show the certificate's subject and expiry (needs an X.509 parser); "restart the workloads using it" after a save; a PriorityClass's pod count.
- Jobs and CronJobs: merged Logs for a CronJob (its latest Job's pods), and Rerun for finished Jobs (a copy without the generated selector and labels).
- Upload/download of folders (tar), and a progress indicator for big files.
- Clean up unused icons that `cargo tauri icon` generated (`android/`, `ios/`, `Square*.png`, `StoreLogo.png`, `icon.ico`). The user hadn't approved deleting them.
- `response.json` at the repo root (`{"status":"success"}`) has an unknown origin and wasn't created during this work.

## 8. Working conventions

- **Git:** the user runs all git commands; a hook blocks git for Claude. Claude only edits files.
- **Before handing back:** keep lint at zero warnings for both clippy commands, run `cargo test`, and run `trunk build`.
- **Visual changes:** check with the headless harness in both light and dark mode.
- **Real-cluster checks:** read-only against UAT (writes only as server-side dry runs). Nothing has written to a cluster yet.
- **Charts:** load the dataviz skill before adding any chart.
