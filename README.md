# Subduer

Desktop app for browsing and controlling Kubernetes clusters, on macOS, Linux and Windows.
Rust throughout: Tauri v2 shell, `kube-rs` backend, Leptos (WASM) UI.

## Layout

| Path | What |
|------|------|
| `crates/types` | Serde types shared by backend and UI (must stay wasm-compatible) |
| `crates/core` | Cluster logic, no UI deps: contexts, discovery, live watches, table rows, safety gate |
| `src-tauri` | Tauri app: IPC commands and events bridging `core` ↔ UI |
| `ui` | Leptos CSR frontend, built by Trunk (`Trunk.toml` at repo root) |

## Prerequisites

- Rust from rustup (not Homebrew `rust`) with `wasm32-unknown-unknown`
- `cargo install --locked trunk@0.21.14 tauri-cli@2.12.1`
- The system's webview build packages (Tauri's prerequisites):
  - macOS: Xcode Command Line Tools (`xcode-select --install`)
  - Linux (Debian/Ubuntu): `sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libsoup-3.0-dev build-essential patchelf file`
  - Linux (Arch): `sudo pacman -S --needed webkit2gtk-4.1 gtk3 librsvg libsoup3 base-devel patchelf file`
  - Linux on a Raspberry Pi 4 or other older ARM64 machine: the `wasm-opt` that Trunk downloads can stop with `signal: 4 (SIGILL)`, because it uses CPU instructions these chips lack. Build version 123 on the machine itself and put it in Trunk's cache:

    ```sh
    sudo apt install cmake ninja-build g++ git
    git clone --depth 1 --branch version_123 https://github.com/WebAssembly/binaryen.git ~/src/binaryen
    cd ~/src/binaryen && git submodule update --init --depth 1
    cmake -G Ninja -DCMAKE_BUILD_TYPE=Release -DBUILD_TESTS=OFF -DBUILD_STATIC_LIB=ON -B build && ninja -C build wasm-opt   # 30–60 min on a Pi 4
    cp build/bin/wasm-opt ~/.cache/trunk/wasm-opt-version_123/bin/wasm-opt
    ```

    `-DBUILD_STATIC_LIB=ON` builds Binaryen's library into `wasm-opt`. Without it, the build fails with `libbinaryen.so: cannot open shared object file`; either rebuild with the flag or copy `build/lib/libbinaryen.so` to `~/.cache/trunk/wasm-opt-version_123/lib/`. Debian's `binaryen` package is too old for Trunk to use.

    At runtime the Pi's GPU driver draws WebKitGTK's first frame as random lines until the window is resized. On a Raspberry Pi (recognised from `/proc/device-tree/model`) the app sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` at startup to prevent this, unless you've already set that variable. On another ARM board with the same symptom, start the app with `WEBKIT_DISABLE_DMABUF_RENDERER=1` yourself.
  - Windows: Visual Studio Build Tools with "Desktop development with C++"; WebView2 is built into Windows 10 and 11
- At runtime: `kubectl`-style kubeconfig access, and the `helm` CLI for Helm rollback and delete

## Run

```sh
cargo tauri dev                  # app with hot-reloading UI
cargo tauri build                # bundles for the system you build on, in target/release/bundle:
                                 #   macOS .app + .dmg · Linux .deb + .rpm + .AppImage · Windows NSIS .exe + .msi
cargo test                       # backend tests (core, types, Tauri commands)
cargo test -p subduer-ui         # UI logic tests
python3 tools/ui-harness/run_e2e.py   # UI end to end in headless Chrome
cargo run -p subduer-core --example smoke -- <context>   # read-only CLI check
```

Each system builds only its own bundles. `.github/workflows/build.yml` builds all three on GitHub's macOS, Ubuntu and Windows runners and keeps the bundles as workflow artifacts. Those bundles, and a plain `cargo tauri build`, are unsigned; for a signed macOS release see below.

### Code signing (macOS)

`cargo tauri build` makes an unsigned app, which Gatekeeper blocks on other Macs, and leaves the version alone. For a release to hand out, run:

```sh
tools/build-signed.sh                    # bump patch, sign, notarize, commit + tag
tools/build-signed.sh --minor            # or --major, or --version 1.2.3
tools/build-signed.sh --no-bump          # rebuild the current version, no commit
tools/build-signed.sh --no-notarize      # sign only, no bump (quick local check)
```

It signs with the hardened runtime using your Developer ID Application certificate, has Apple notarize the app and the dmg, staples the tickets, and checks the result with `codesign`, `stapler` and `spctl`. Notarizing takes a few minutes per submission (two: app, then dmg).

The version lives only in `[workspace.package]` in the root `Cargo.toml`; every crate inherits it, and `tauri.conf.json` has none, so Tauri reads it from there. A bump needs a clean working tree; once everything passes the script commits `chore(release): v<version>` and tags `v<version>` (not pushed). If anything fails, the bump is reverted.

Never export `APPLE_SIGNING_IDENTITY` in your shell profile: `cargo tauri build` signs whenever it is set.

If `cargo tauri build` fails with "Failed to create app Assets.car … Bad file descriptor": Tauri runs `actool` with stdin closed, which makes it fail and leaves Xcode's `ibtoold` daemon broken, so later `actool` runs fail too. `tools/build-signed.sh` works around it with an `actool` wrapper that supplies `/dev/null` as stdin. For a plain build, use the wrapper the script leaves in `target/actool-shim` and restart the daemon:

```sh
killall ibtoold; PATH="$PWD/target/actool-shim:$PATH" cargo tauri build
```

The release dmg is `target/release/bundle/dmg/Subduer_<version>_aarch64.dmg` (Apple Silicon only; the script refuses to run on an Intel Mac), with `Subduer_<version>_aarch64.dmg.sha256` next to it, hashed after stapling.

### Publishing (GitHub release + Homebrew)

The Homebrew cask (`Casks/subduer.rb` in `cjcooper/homebrew-tap`) installs `https://github.com/cjcooper/subduer/releases/download/v<version>/Subduer_<version>_aarch64.dmg`. To publish, on `main`:

```sh
tools/build-signed.sh                    # bump, sign, notarize, commit + tag
tools/publish-release.sh --dry-run       # optional: every check, nothing pushed
tools/publish-release.sh                 # push main + tag, create the GitHub release
```

`publish-release.sh` needs `gh` logged in. It checks that a clean `main` has the tag `v<version>` in its history, with nothing built into the app changed since (only `tools/`, `README.md`, `RUNBOOK.md` and `.github/` may change); that the dmg has the right name, matches its `.sha256`, and is notarized and stapled; and that it holds an arm64 `Subduer.app` at that version. Then it pushes, creates the release marked latest (the cask's livecheck follows the latest release), downloads the asset again to verify its SHA-256, and prints the `version` / `sha256` for the cask, along with a `brew bump-cask-pr` command. `--clobber` replaces the assets of a release that already exists. A changed dmg means a new `sha256` in the cask.

One-time setup:

1. **Certificate.** In Xcode → Settings → Accounts → (team) → Manage Certificates, add a **Developer ID Application** certificate (or create it at developer.apple.com → Certificates and import it). Only the Account Holder can create one. Check it's in the keychain: `security find-identity -v -p codesigning`. With more than one, set `APPLE_SIGNING_IDENTITY` to the full name.
2. **Notarization credentials**, either:
   - **API key** (recommended): App Store Connect → Users and Access → Integrations → App Store Connect API → Team Keys, create a key with the Developer role. Save `AuthKey_<KEY_ID>.p8` to `~/.appstoreconnect/private_keys/` and set:
     ```sh
     export APPLE_API_KEY=<KEY_ID> APPLE_API_ISSUER=<ISSUER_ID>
     ```
   - **Apple ID**: create an app-specific password at account.apple.com → Sign-In and Security, store it in the login keychain, and set:
     ```sh
     security add-generic-password -s subduer-notarization -a <apple-id> -w
     export APPLE_ID=<apple-id> APPLE_TEAM_ID=<TEAM_ID>
     ```

   The same key or password works for Deployer; only the keychain item name differs (`deployer-notarization` there).

Making the dmg drives Finder with AppleScript; the first run asks to let your terminal control Finder. If it fails with "Not authorized to send Apple events to Finder", allow it in System Settings → Privacy & Security → Automation.

Never commit the `.p8` key or the password (`*.p8` and `private_keys/` are git-ignored).

## Behaviour

- **Kubeconfig changes outside the app.** The kubeconfig files are checked every 2 seconds and a change is acted on once it has settled (and no `.lock` file is held). Every window's Context menu follows. A window whose context was deleted (`kubectl config delete-context`, an editor) moves to kubectl's current context, else the first, and says so. If no contexts are left, every main window but one closes (unsaved edits and running traces ask first, as any close does) and that one shows Add Cluster; the first context to appear again, from here or from kubectl, is connected to.
- **Manage Clusters.** File › Manage Clusters… (only there) lists the kubeconfig's contexts in the order the Context menu shows them, with the cloud each is in. Drag a row (or use its arrows) to reorder; the order is saved in the kubeconfig's `contexts` list (kubectl ignores it), and with several `$KUBECONFIG` files a context moves only within its own file. **Remove** takes out the context, plus its cluster and user if no other context uses them, as `kubectl config delete-context` / `delete-cluster` / `delete-user` would; a context any window is on (connected, or still connecting) can't be removed, and the list shows which ones other windows are on as they change. Each row's warning-sign button marks the context as production or not, overriding what its name suggests; the mark is kept in the app's own settings (`production.json` in its config folder), not the kubeconfig, and decides the red sign in the Context menu and the extra confirmation for changes. **Add Cluster…** adds a context to the kubeconfig kubectl uses, so other tools see it too: the first file in `$KUBECONFIG` that exists (else the last), or `~/.kube/config`. **Kubernetes** takes a server URL, a CA certificate (pasted PEM is kept in the file; a chosen file is pointed to), skip TLS verification, a bearer token, client certificate or exec plugin, a default namespace and a proxy URL; names already in the kubeconfig are refused. **Google Cloud**, **AWS** and **Azure** list your clusters with `gcloud`, `aws` or `az` (installed and signed in) and let that CLI write the entries (`get-credentials` / `update-kubeconfig`), optionally under a name you choose. The kubeconfig's current context only changes if you tick it. Every window's context list picks the new context up, and the window connects to it unless you untick that. Logos: Kubernetes and Google Cloud from Simple Icons (CC0), AWS and Azure from Devicon (MIT).
- **Several windows.** File › New Window (⌘N, Ctrl+N elsewhere) opens another main window. Each one connects on its own, to the same cluster or a different one, and is titled by its context. Nothing is shared between them: "allow changes", the live table, traces and port forwards all belong to one window. The logs, shell and edit windows you open from a window close with it, and its port forwards stop.
- **Navigation by kind.** All listable kinds come from API discovery. Nodes and Events are listed at the top of the sidebar. Everything else is sorted into collapsible sections: Workloads, Config, Network, Storage, Namespaces, Access Control, Helm (releases), AWS, Google Cloud and Azure (shown only when present), Custom Resources (CRDs and aggregated APIs, grouped by API group) and Other. Kinds have icons, including well-known projects (Kyverno, Datadog, KEDA, External Secrets, Prometheus Operator). Sections start collapsed. Drag a section header to reorder the sections; the order is saved. The section map is in `crates/core/src/catalog.rs`.
- **Namespace filter.** One namespace or all of them. The choice stays when you switch kinds and is ignored for cluster-scoped kinds.
- **Cluster page.** Nodes, pods, namespaces and recent warnings, plus memory and CPU (available, used, requested and limits). Also requests by namespace and a per-node table. It refreshes every 30 s. "Used" comes from metrics-server, Prometheus or kubelet stats, and the last hour is charted when Prometheus is available.
- **Nodes.** The Node kind uses the Cluster page's node table, with per-node pods and requests, and usage when metrics-server is available. It refreshes every 30 s. Clicking a row opens its Details tab: Resources first (last-hour CPU and memory charts like a pod's, with requested and allocatable lines, and a pods meter), then status, roles, zone, addresses, taints and system info, conditions, every pod running on it (click one to open that pod), and its events.
- **Expired credentials.** Nothing retries forever. A list that keeps failing stops after five attempts (about 12 s), and the Cluster page stops refreshing after three failures. If the cluster no longer answers (an expired token, say), the app shows "Not connected" with the reason and a Connect button: sign in again, then click it. A connection that fails at startup shows the same page, and so does a dropped VPN: the app checks the connection every 10 s and gives up after three failed checks in a row (within about a minute), even when nothing else reports an error.
- **Column widths.** Drag a column header's right edge to resize it. Widths are saved per kind (the node tables share one setting). Double-click the edge to reset.
- **Pod containers.** The Init and Ready columns show one icon per init container and app container respectively: ✓ running, ◐ starting, ■ stopped, ✕ failed. Hover for details.
- **Pod details.** Selecting a pod opens a Details tab (YAML is still one click away), refreshed every 15 s. It shows pod resources (used, requested and limit), pod info, and each container with its resources, state, ports and environment, then events. Every section collapses. Secret-backed variables stay masked until you click Show, which fetches that one value.
- **Deployment and StatefulSet details.** The same layout for workloads: usage summed over their pods (last hour), status, replicas, rollout strategy and conditions, a pods table (click a pod to open it), each template container with its per-pod resources and environment, and the events of the workload, its ReplicaSets and its pods.
- **Config details.** Every kind in the Config section gets a Details tab. ConfigMaps and Secrets list their keys and the pods that use them. HPAs show their metrics against targets, PDBs whether evictions are allowed, and quotas how close each resource is to its limit. Webhooks and admission policies show what they apply to and what happens when they fail. Leases show who holds them. Links open the related object.
- **Network details.** Services show their ports, the pods behind them (ready or not) and the Ingresses routing to them. Ingresses show each rule's backend health, flag missing Services and certificate Secrets, and offer their URLs to open in the browser. NetworkPolicies spell out what they let in and out, and which pods they apply to. EndpointSlices and Endpoints list each address and its pod.
- **Storage details.** Claims show their size, how full the disk is, which pods mount them, and why one is still pending. Volumes show where their data lives and what happens to it when the claim goes. Storage classes list the claims using them; attachments show attach errors.
- **KEDA details.** ScaledObjects show what they scale, the replica range, what happens when idle, fallback and pause, the HPA KEDA made (linked, with replicas now) and each trigger's value now against its target, with failing triggers flagged; each trigger's settings follow, with secret-looking values and passwords in addresses hidden. ScaledJobs show the job they run, how many at once, the scaling and rollout strategies and the Jobs they made. TriggerAuthentications (and cluster ones) show where each parameter comes from (Secret, ConfigMap, env var, Vault, cloud secret managers) and their pod identity, never the values, and who uses them. CloudEventSources show where events go. Controller logs opens the KEDA operator's logs filtered to the object. **Pause** stops KEDA scaling a ScaledObject (where it is, or at a replica count you choose) or starting new Jobs for a ScaledJob, and **Resume** lets it scale again. **Edit range** changes the min and max replicas (or Jobs at once) without editing YAML, and warns when a Helm upgrade would put them back. Both are gated like Delete.
- **Helm releases.** Each release shows its chart and app version, deploy times, the full revision history with descriptions, the objects it created (linked) and its notes. Values are never shown. **Rollback** to an earlier revision runs `helm rollback`, and **Delete** runs `helm uninstall` after you type the release's name (the `helm` CLI must be installed).
- **Namespace details.** What's in a namespace (click a count to see that list), what its pods ask for and use against its quota, its quotas and Helm releases, recent warnings, and, when a deletion is stuck, what it's waiting for. **New namespace** creates one; **Filter to it** narrows every list to it.
- **Access control details.** Roles list their rules with risky ones flagged (full access, reading secrets, exec, granting more) and who they're bound to. Bindings show what they grant, where and to whom. ServiceAccounts show their cloud identity, every binding that applies, what they can actually do, and the pods running as them. **Check access** asks the API server whether a ServiceAccount may do something.
- **Edit ConfigMaps and Secrets.** Edit values, add, rename and remove keys, then Save. Secret values stay masked until you click Show or start editing. A save is refused if someone else changed the object since you started. Nothing is saved until you click Save: leaving an edit with unsaved changes (another object, another tab, closing the window, quitting) asks whether to discard them first.
- **Job and CronJob details.** The same layout again. Jobs show completions, failures against the retry limit, start, finish and duration. CronJobs show the schedule and time zone, the next run, the last scheduled and last successful runs, and a table of their Jobs (click one to open it).
- **Container actions.** Logs (also merged across a workload's pods) and Shell open in their own windows, listed under the Window menu. Port-forward pods and Services (Port Forwards page). Upload files into, and download files out of, containers, with a folder browser that shows writable locations.
- **Edit YAML.** Every YAML tab has an Edit button (for kinds that can be updated). It opens the object in its own window with an editor that highlights YAML, marks syntax errors as you type, and completes common keys. Save has the server check the edit, shows what changed, and saves only after you confirm. If someone else changed the object since you opened it, the save is refused rather than overwriting their change. Cancel, Reload and closing the window ask first if you have unsaved edits.
- **Traces.** Prometheus often keeps only an hour or two, so a trace records CPU and memory for as long as the app stays open, into a `.trace` file you choose (Trace… in the top bar, Window › Trace…, or Trace… on a workload, namespace or pod). It backfills what Prometheus still has, samples pods Prometheus misses, and shows a red indicator while recording; closing or switching context asks first and saves the file. The Traces page opens any trace without a cluster: a right-sizing table (requests against p95 CPU and peak memory, spare, flags such as OOM kills, throttling or no request) with suggested requests, per-workload charts and a day-and-hour grid, notes saved in the file, Compare for traces side by side (before and after a resize), and Export report… for a standalone HTML page. While connected to the trace's cluster, Apply… sets the suggested requests (one rollout per workload, a server dry run first, through the safety gate), and offers a values snippet for Helm-managed workloads.
- **Delete** any object whose kind allows it, after a confirmation that names the context.
- **Restart** (rollout restart) and **Scale** Deployments and StatefulSets, after the same kind of confirmation.
- **Run now** a CronJob (like `kubectl create job --from=cronjob/…`), and **Suspend/Resume** CronJobs and Jobs, after the same kind of confirmation.
- **Edit range** of an HPA (min and max replicas), **Expand** a PersistentVolumeClaim, and switch a PersistentVolume's **Reclaim policy** between Retain and Delete.
- **Live tables.** Each table is backed by a Kubernetes watch, with snapshots throttled to 250 ms.
- **Esc closes things.** Esc cancels the open dialog (never its risky action) or, with no dialog open, closes the detail panel. It leaves the panel alone while you type in a field, and asks first if you have unsaved edits.
- **Light or dark.** Settings… (⌘,) shows previews of Light, Dark and, where the system reports one, Auto; pick one. Every window follows it.
- **Read-only by default.** Every context starts locked. To unlock a production-like context (a name containing `prd`/`prod`/`production`/`live`), you have to type its name; that's the only typed confirmation it asks for. Once it's writable, each change there is confirmed with a plain OK (the dialog is still framed in red). All mutations go through `SafetyGate::ensure_writable`: Delete, Restart, Scale, Run now, Suspend/Resume, Save, Edit range, Expand, Reclaim policy, Rollback, release Delete, New namespace, Shell and Upload ask to unlock first.

## License

MIT, see [LICENSE](LICENSE). Copyright (c) 2026 it-problems.

The licenses of everything the app includes are in
[THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt), which ships inside every
installer and is compiled into the app: the About window (app menu on macOS,
Help elsewhere) shows the app's own license and every third-party license with
the software it covers. Regenerate it with `tools/third-party-notices.sh` after changing
dependencies (needs `cargo install cargo-about --features cli`); the script
fails if a crate brings in a license not accepted in `about.toml`.
