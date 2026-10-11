# k8s-subduer runbook

How to pick this project back up. It covers what exists, how to run and verify it, the decisions behind it, and what's next. `README.md` is the short user-facing summary; this file is the working notes.

_Last updated: 2026-10-11 (Traces phase 1: capture and the .trace file)._

## 0. Where things stand (read first)

- **Committed up to `22df4e7`** (workload Details tab); the user commits. Not yet committed:
  - **Jobs and CronJobs:** a Details tab for both, Run now and Suspend/Resume on CronJobs, and Suspend/Resume on Jobs. Files: `crates/core/src/cron.rs` (new), `crates/core/src/workload.rs`, `crates/core/src/actions.rs`, the types, the commands, the `workload` and `workload_dry_run` examples, `ui/src/workload.rs`, `ui/src/workload_detail.rs`, `ui/src/app.rs`, CSS, `tools/ui-harness/batchstub.js` (new) and docs.
  - **Config section:** Details tabs for all 13 Config kinds; editable ConfigMap and Secret data with Save; Edit range on HPAs. Files: `crates/core/src/config.rs` (new), `actions.rs` (`save_data`, `set_hpa_range`), `pod_details.rs` (`secret_values`), the types, the commands, `examples/config.rs` and `examples/config_dry_run.rs` (new), `ui/src/object_detail.rs` (new), `ui/src/workload.rs` (Save and Edit range in the shared dialog), `ui/src/app.rs`, `ui/src/pod_detail.rs`, CSS, `tools/ui-harness/configstub.js` (new) and docs.
  - **Unsaved-edit guard:** `app::guard`, the discard prompt, `set_unsaved`/`close_discarding` and the window close and Cmd+Q hold in `src-tauri/src/lib.rs`.
  - **Network section:** Details tabs for Service, Ingress, IngressClass, NetworkPolicy, EndpointSlice and Endpoints; Open buttons for Ingress URLs. Files: `crates/core/src/network.rs` (new), `config.rs` (dispatch), the types (`InfoRow.url`, `has_object_details`), `open_url` in the commands (`forward::open_in_browser` shared), `ui/src/object_detail.rs`, CSS, `tools/ui-harness/networkstub.js` (new) and docs.
  - **Storage section:** Details tabs for PVC, PV, StorageClass, VolumeAttachment, CSIDriver, CSINode and CSIStorageCapacity; Expand (PVC) and Reclaim policy (PV). Files: `crates/core/src/storage.rs` (new), `actions.rs` (`expand_claim`, `set_reclaim_policy`), `config.rs` (dispatch), the types, the commands, `examples/config_dry_run.rs`, `ui/src/workload.rs` (`Op::Expand`, `Op::Reclaim`, cluster-scoped targets), CSS, `tools/ui-harness/storagestub.js` (new) and docs.
  - **Access Control section:** Details tabs for ServiceAccount, Role, RoleBinding, ClusterRole and ClusterRoleBinding; Check access on ServiceAccounts. Files: `crates/core/src/rbac.rs` (new), `config.rs` (dispatch), the types (`AccessCheck`), `check_access` in the commands, `examples/access_check.rs` (new), `ui/src/object_detail.rs` (`CheckAccess`), CSS, `tools/ui-harness/rbacstub.js` (new) and docs.
  - **Helm section:** a Details tab for releases, Rollback and Delete (`helm uninstall`). Files: `crates/core/src/helm.rs` (`release_details`, `manifest_objects`, `rollback`, `uninstall`, `run_helm`), `config.rs` (dispatch), core's tokio `process` feature, the types (`InfoTable.times`), `helm_rollback`/`helm_uninstall` in the commands, `examples/helm_rollback_dry_run.rs` (new; dry-runs both), `ui/src/workload.rs` (`Op::Rollback`, `Op::Uninstall`), `ui/src/object_detail.rs` (time columns, multi-line rows), `ui/src/app.rs` (links to cluster-scoped kinds), CSS, `tools/ui-harness/helmstub.js` (new) and docs.
  - **Settings › Appearance:** a Settings… menu item (⌘, / Ctrl+,) and dialog with Auto (where supported), Light and Dark. Files: `src-tauri/src/menu.rs`, `set_app_theme` in the commands, `ui/src/prefs.rs` (theme), `ui/src/main.rs`, `ui/src/app.rs` (`SettingsDialog`), `ui/src/exec.rs`, CSS, `tools/ui-harness/settingsstub.js` (new) and docs. Also: bigger top-level sidebar items and icons.
  - **Builds on each system:** per-OS bundle configs (`src-tauri/tauri.{macos,linux,windows}.conf.json`), `icon.ico` bundled, Unix-only login-shell PATH, no console windows on Windows, per-OS reveal label and fonts, and `.github/workflows/build.yml`. Only the macOS build has been run.
  - **Namespaces section:** a Details tab for Namespace, New namespace and Filter to it. Files: `crates/core/src/namespace.rs` (new), `pod_details.rs` (`namespace_usage`), `config.rs` (dispatch), the types, `create_namespace` in the commands, `examples/namespace_dry_run.rs` (new), `ui/src/namespace.rs` (new), `ui/src/app.rs` (list links, unlock flow), `ui/src/object_detail.rs` (Recent warnings), `ui/src/workload_detail.rs` (namespace conditions), `tools/ui-harness/nsstub.js` (new) and docs.
  - **More colour:** CPU blue and memory purple across charts, meters and bars; tinted sidebar sections; tinted Cluster summary tiles; all per theme (see "Colours").
  - **Tests across the app:** the fake API (`crates/core/src/testing.rs`, `test-support` feature), `*_tests.rs` beside every core module and most UI modules, `src-tauri/src/tests.rs`, `crates/types/src/tests.rs`, `tools/ui-harness/run_e2e.py`, runtime-generic Tauri commands, small UI refactors for testability, five bug fixes (see "Tests"), and CI steps for UI tests, the end-to-end suite (macOS) and coverage (Linux).
  - **Quit everywhere:** File › Exit (Alt+F4) on Windows and File › Quit (Ctrl+Q) on Linux, both held for unsaved edits. Files: `src-tauri/src/menu.rs`, docs.
  - **Connection give-up:** watches stop after 5 failed attempts in a row (kube's backoff never grew before, see Decisions), Cluster page refreshes after 3; then `check_connection` decides between a "Not connected" page with Connect and a Retry/Refresh on the failing view. A failed connect shows the same page. Dropped VPNs too: a background `check_connection` every 10 s (15 s limit each; 3 failures in a row → "Not connected"), a 10 s connect timeout, and a connection generation so late results (a Cluster refresh hung on a dead connection) can't block things after Connect. Files: `crates/core/src/contexts.rs` (`CONNECT_TIMEOUT`), `crates/core/src/watch.rs` (`Backoff`, `Failures`, `MAX_ATTEMPTS`), `helm.rs`, `cluster.rs` (`check_connection`), the types (`ResourceTable.stopped`), `check_connection` in the commands, `ui/src/app.rs` (`gave_up`, `lose`, `Disconnected`), `ui/src/overview.rs`, CSS, `tools/ui-harness/connstub.js` (new) and docs.
  - **YAML editing:** an Edit button on every YAML tab (kinds with the `update` verb; not Helm releases) opens the object in its own window with Ace (YAML highlighting, syntax errors marked as you type, completion, Cmd/Ctrl+S). Save has the server dry-run the edit, shows a diff to confirm (prod-like contexts ask for the name), then replaces the object; a change made by someone else meanwhile is refused. Cancel, Reload and closing the window ask before discarding edits. Files: `crates/core/src/edit.rs` (new), `examples/edit_dry_run.rs` (new), the types (`EditTarget`, `EditSession`, `edit_events`), `src-tauri/src/editor.rs` (new), `lib.rs` (close/quit hold), `logs.rs` (`close_all`), `capabilities/default.json` (`edit-*`), `ui/src/editor.rs` (new), `ui/src/main.rs`, `ui/src/app.rs`, `ui/src/exec.rs`, `ui/index.html`, `ui/vendor/ace/` (new, vendored), CSS, `tools/ui-harness/editorstub.js` (new), `configstub.js`, `run_e2e.py` (a real-time mode) and docs.
  - **Linux:** Arch Linux setup commands next to the Debian ones. The app icon now shows on KDE and GNOME when the app isn't installed from a package: `desktop_entry::ensure` writes a user desktop entry, and the window icon is now 256 px. Files: `src-tauri/src/desktop_entry.rs` (new), `src-tauri/src/lib.rs`, `tauri.conf.json` (icon order, plus 48, 64 and 256 px), `icons/48x48.png` and `icons/256x256.png` (new), `make_icon.py`, docs. Not yet tried on a Linux desktop.
  - **Esc and `Modal`:** every dialog is drawn by `modal::Modal` (backdrop, card, `prod`/`wide`, optional click-outside), which requires `on_cancel`: what Esc does, the same as its Cancel, Close, Done, Keep editing, Back to editing or Back button, never the risky one. A new dialog can't compile without one. Upload's cancel goes back a step from "already exists"; the others close. Open dialogs form a stack per window (`modal.rs`): the newest draws on top (z-index from its depth) and takes Esc, even from its text field; one keydown listener per window serves it. `busy` (Deleting…, Saving…, Uploading…, allowing changes in the edit window) makes Esc do nothing and the dialog disables its Cancel too, so a request in flight is never hidden. Late results are dropped once their dialog closed or reopened: `modal::Generation` in Upload and Download (file and folder requests), the request itself in Port forward (which also has a Cancel while its ports load), and the edit window's unlock only moves on to Save if "Allow changes?" is still open. With no dialog open, Esc closes the detail panel through `guard` (unsaved edits ask first), except while a text field has focus (dropdowns, checkboxes and radios don't count); it asks `modal::any_open()` rather than the page. Key repeats and Esc that ends an input-method composition are ignored (`modal::is_plain_esc`, shared with the panel). The × tooltip says "Close (Esc)". Files: `ui/src/modal.rs` (new), `ui/src/main.rs`, `ui/src/app.rs` (`DetailPanel` and three dialogs), the dialogs in `delete.rs`, `download.rs`, `editor.rs`, `forward.rs`, `namespace.rs`, `upload.rs` and `workload.rs`, `tools/ui-harness/esc.js` (new), `run_e2e.py` (16 `esc/` scenarios: the panel, `Modal`'s rules once each, busy dialogs, late results, and the dialogs whose cancel is more than closing), `hang=`/`slow=`/`again=` in `stub.js`, `hang=` and `late=` in `fwd.js`, `hang=save|unlock` in `editorstub.js`, `click=` in `settingsstub.js`, and docs.
- **Verified:** clippy is clean (backend, and UI on wasm32); 536 tests pass (413 backend: core, Tauri and types; 123 UI) and the 160-scenario end-to-end suite passes. Earlier: `cargo test` passed (70 core + 1 tauri + 1 types; the UI icon test runs via `cargo test -p subduer-ui`), and `trunk build` succeeds. Each feature was checked against UAT through its `examples/*` script and visually in the headless harness, light and dark.
- **Not yet seen in the real app window** (macOS blocks scripted clicks and screenshots): the Settings… menu item and the native title bar following the theme, the unsaved-edits hold on window close and Cmd+Q, YAML edit windows (Ace in WKWebView, its syntax-check worker under the `tauri://` scheme, and the hold when closing with unsaved YAML), logs, shell and port-forward windows, their Window-menu entries, the shell window closing on exit, Open/Copy on port-forwards, real drag-and-drop in Upload, Show in Finder, and Esc in each system's web view. The user had `cargo tauri dev` running; restart it to pick up backend changes.
- **Untested:** downloading multi-MB files end to end (the user declined the SHA check command).
- **Checked on UAT (2026-10-10):** `workload` and `workload_dry_run` against a Deployment, a StatefulSet, two CronJobs (one with `timeZone: America/New_York`) and completed and failed Jobs. Every dry run was accepted, and the locked gate refused each real call. UAT's Prometheus has no series for the short CronJob pods, so their history falls back to local samples; Deployment and StatefulSet history come from Prometheus.
- **YAML editing checked on UAT (2026-10-10):** `edit_dry_run` on ConfigMap `default/kube-root-ca.crt` and Deployment `kube-system/coredns`: unchanged and annotated edits dry-run OK, a stale resourceVersion is refused as "changed by someone else", and the locked gate refused the real save; nothing was changed.
- **Namespaces checked on UAT (2026-10-10):** `config` read `eg-env-regalify`: 169 objects (19 pods by phase, 93 Secrets counted by metadata only), requests and the limits gap, 8 Helm releases. UAT has no metrics-server, so "Used now" says so. `namespace_dry_run` accepted a new name, refused an existing one, and the locked gate refused the real call; nothing was created.
- **Helm checked on UAT (2026-10-10):** `config` read the `canary` release (10 revisions with chart and description, 2 resources, chart description). `helm_rollback_dry_run` (54 → 53) ran `helm rollback --dry-run=server` (Helm 4.3 locally) and succeeded; `helm history` afterwards still ended at 54. `helm uninstall --dry-run` also succeeded and `helm status` still showed revision 54 deployed. The locked gate refused both real calls. No real rollback or uninstall has been run anywhere.
- **Access Control checked on UAT (2026-10-10):** `config` read a ServiceAccount with an IAM role (its bindings, effective rules and pods), the datadog ClusterRole (15 bindings), a ClusterRoleBinding, a RoleBinding and `cluster-admin` (Full access). `access_check` answered allowed with the granting binding for `list pods` and `get nodes/proxy`, and denied for `get secrets` and `create deployments.apps`, matching the rules shown.
- **Storage section checked on UAT (2026-10-10):** `config` read the EFS claim and volume (static, no class), the `gp2` class and the EFS CSI driver with its nodes. `config_dry_run` was accepted for the PV reclaim policy. The PVC expand dry run was refused by the server ("only dynamically provisioned pvc can be resized"), and the panel says so and hides Expand. The locked gate refused both real calls. Disk usage isn't available there: the kubelet on Fargate doesn't report it.
- **Network section checked on UAT (2026-10-10):** `config` read a NodePort Service, a headless Service, an EndpointSlice, Endpoints and the `alb` IngressClass; endpoints, links and statuses matched `kubectl`. UAT has no Ingresses or NetworkPolicies, so those are covered by unit tests and the harness only.
- **Config section checked on UAT (2026-10-10):** `config` read an HPA (KEDA-owned, external metric), the coredns PDB, a Helm-managed ConfigMap, a Secret (sizes only), a webhook configuration, a Lease and a PriorityClass. `config_dry_run` was accepted for a ConfigMap save, a Secret save and an HPA range. A save from a stale resourceVersion was refused with the "changed by someone else" message, and the locked gate refused the real calls. Nothing was written. The dry-run key was not persisted.
- **Traces, phase 1 (capture and file format), not committed:** `crates/core/src/trace.rs` (new; pulls, the app's own samples, the run loop), `trace_file.rs` (new; the SQLite `.trace` format), their `*_tests.rs` (24 tests), `examples/trace.rs` (new), the types (`TraceScope`, `TraceWorkload`, `TraceEnd`, `TraceGap`, `TraceSummary`), `rusqlite` (bundled) in core, `Prometheus::get` and `discover_prometheus` made `pub(crate)`, `pod_metrics_api`/`metrics_containers` likewise, and DaemonSet pod names in `workload::pod_name_regex`. No UI or Tauri commands yet (phase 2). See "Traces" in section 7.
- **Traces checked on UAT (2026-10-11):** `trace` backfilled 114 min for `deploy:kube-system/coredns` (owners resolved to the Deployment with its pod-template-hash; requests 100m/70Mi, memory limit 170Mi; no throttling column without a CPU limit). A 3-minute `cluster` trace: 54 pods, 77 containers, 118 min, 332 KiB; only Prometheus' own pod was sampled by the app (every other running pod is in cAdvisor; the 17 pods without usage are finished Job pods kube-state-metrics still lists). CronJob pods resolved through `kube_job_owner`. About 37 bytes per container-minute, so ~55 MB/day per 1,000 containers.
- **Open questions for the user:**
  - Should port-forwards on prod contexts require unlocking? (Currently not gated.)
  - Should revealing secrets require unlocking?
  - Should logs and shell windows stay open when the main window closes? (Currently they close with it.)


## 1. What it is

A desktop app for browsing and controlling Kubernetes clusters: macOS first, and building on Linux and Windows too. It's an it-problems project, so never put employer names in code, config, docs or tests; use generic names like `acme-prd`.

- **Stack:** Rust everywhere, using a Tauri v2 shell, a `kube-rs` backend and a Leptos 0.8 UI built to WASM with Trunk. Use another language only where the OS forces it.
- **Navigation:** by kind, with a namespace filter that persists across kinds.
- **Safety:** read-only by default. Mutations (Delete, Restart, Scale, Run now, Suspend/Resume, data Save, Edit range, Expand, Reclaim policy, Helm Rollback and Delete, New namespace, Shell, Upload) are behind the safety gate.

## 2. Getting set up

| Need | How |
|---|---|
| Rust via **rustup** (not Homebrew `rust`) | macOS: `brew install rustup`. The package is keg-only, so put `/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin` first on `PATH`, then run `rustup default stable`. Homebrew's formula no longer ships `rustup-init`. Arch: `sudo pacman -S rustup`, then `rustup default stable`. Other Linux and Windows: the installer from rustup.rs. |
| System build packages | macOS: Xcode Command Line Tools. Linux (Debian/Ubuntu): `sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libsoup-3.0-dev build-essential patchelf file`. Linux (Arch): `sudo pacman -S --needed webkit2gtk-4.1 gtk3 librsvg libsoup3 base-devel patchelf file` (Arch packages ship their headers, so there are no `-dev` packages). Windows: Visual Studio Build Tools ("Desktop development with C++"); WebView2 ships with Windows 10/11. No OpenSSL anywhere: TLS is rustls with ring. |
| WASM target | `rustup target add wasm32-unknown-unknown` |
| Tools | `cargo install --locked trunk@0.21.14 tauri-cli@2.12.1` |
| Icon tools (optional) | `rsvg-convert` (Homebrew `librsvg`) |

Versions in use: Rust 1.99, tauri 2.12, leptos 0.8, kube 4.2, k8s-openapi 0.28 with the `latest` feature (v1.36). Run `cargo info <crate>@<major>` to see newer releases.

## 3. Daily commands

```sh
cargo tauri dev                       # run the app (Trunk serves the UI on :1420)
cargo tauri build                     # this system's bundles in target/release/bundle (see "Building on each system")
cargo test                            # backend tests: core, types and the Tauri commands (~400)
cargo test -p subduer-ui              # UI logic tests on the host (~120)
python3 tools/ui-harness/run_e2e.py   # the real UI in headless Chrome, 160 scenarios (~3 min)
cargo llvm-cov --workspace --summary-only   # line coverage (needs cargo-llvm-cov + llvm-tools-preview)
cargo clippy --all-targets            # backend lint, kept at zero warnings
cargo clippy -p subduer-ui --target wasm32-unknown-unknown   # UI lint, zero warnings
trunk build                           # UI only, into dist/
```

### Tests

Five layers, none of which contacts a cluster:

| Layer | Where | What |
|---|---|---|
| Core logic | `crates/core/src/<module>_tests.rs`, included from each module as `mod coverage_tests` (plus the older inline `mod tests`) | Every module against **`crates/core/src/testing.rs`**, a fake Kubernetes API: routes by method and path (optionally a query substring) to canned JSON, 404 `Status` otherwise, records every request (method, path, query, body) so tests can check patches and `dryRun=All`. Helpers: `list`, `object`, `pod`, `merge`, `kind`, `status`. Exposed to other crates with the `test-support` feature. |
| WebSocket paths | `crates/core/src/ws_fake_tests.rs` | A small local server speaking the `v5.channel.k8s.io` exec protocol, so shell, upload, download and browse run end to end. |
| Types | `crates/types/src/tests.rs` | Keys, predicates, categories, encodings, titles, serde defaults. |
| Tauri commands | `src-tauri/src/tests.rs` | Commands called on Tauri's mock app (`tauri::test`) with a session on the fake API: not-connected errors, context mismatch, the safety gate, request bodies, logs/shell windows, port-forwards. Commands that take an `AppHandle`/`WebviewWindow` are generic over `R: Runtime` for this. |
| UI logic | `ui/src/<module>_tests.rs` (`mod tests_unit`) | Pure functions on the host: formatting, quantity parsing, the data-edit diff, validation, section order, prefs parsing, chart scales. Some logic was pulled out of browser-only closures for this (e.g. `age_since`, `relative_at`, `ColumnWidths::from_saved`, `theme_from`, `desktop_from`). |
| UI end to end | `tools/ui-harness/run_e2e.py` | Builds the UI, serves it with each harness stub, drives headless Chrome through 160 scenarios (every dialog, Esc, the unsaved-edit guard for all 10 ways out, settings against light/dark systems, …) and checks what the app sent to the backend (the stubs put it in the page title) and what the page shows. One browser at a time by default (`E2E_JOBS` to change); a failure is retried once and reported as needing a retry. `--show <id>` prints what a scenario saw. Scenarios marked `real_time` (Ace's syntax check runs in a web worker, which doesn't run under `--virtual-time-budget`) drive Chrome over `--remote-debugging-pipe` and wait in real time instead. |

**Coverage (2026-10-10, `cargo llvm-cov --workspace`):** core 99.0% of lines (26 modules; 19 at 100%), types 100%, Tauri crate 66.6%, backend overall 95.9% (from 46% before). UI host tests cover 14.5% of the UI crate's lines: every logic function, but not `view!` markup, which the end-to-end suite exercises instead (llvm-cov can't measure WebAssembly in Chrome).

What isn't covered, and why:
- `helm.rs` after the gate (running the real `helm` binary): tested up to argument building, validation and gate refusal. Faking `helm` would mean changing `PATH` for the whole test process.
- `forward.rs` `connect` success: needs a fake of the port-forward WebSocket protocol (only exec's is faked).
- Tauri crate: `lib.rs` `run()` and window setup, `main.rs`, `menu.rs` (macOS builds menus only on the main thread), `shell_env::inherit_login_path` (runs the login shell), `set_app_theme` (not implemented by the mock runtime), `connect`'s success path (needs a real server), `upload_file` (its IPC `Request` can't be built in tests), and anything that opens a browser or Finder.
- A few unreachable branches (streams that never end, "streams not opened" when they're always requested).

Bugs the tests found (fixed): workload usage included pods the workload doesn't own (and a CronJob with no Jobs showed the whole namespace); a running native sidecar made a healthy pod show `Init:1/2`; the Expand dialog rejected sizes like `1e9`; an Ingress whose TLS secret was missing wasn't flagged in a namespace with no Secrets; disk usage could show negative free space. Noted, not changed: `history::sum_series` would double-count two local samples in the same millisecond; `logs::millis` assumes UTC timestamps; `size_text(1048575)` shows "1024.0 KiB".

### Building on each system

`cargo tauri build` builds for the system it runs on; Tauri merges `src-tauri/tauri.<os>.conf.json` over `tauri.conf.json` (which says `"targets": "all"`):

| System | Bundles (in `target/release/bundle/`) | Notes |
|---|---|---|
| macOS | `macos/k8s-subduer.app`, `dmg/*.dmg` | Minimum macOS 11; category Developer Tools. Built and checked here (2026-10-10, Apple Silicon). Unsigned and not notarized. |
| Linux | `deb/*.deb`, `rpm/*.rpm`, `appimage/*.AppImage` | Needs WebKitGTK 4.1 (Ubuntu 22.04+, Debian 12+, Fedora 36+, current Arch). The AppImage step needs FUSE 2 (on Arch, `fuse2`), or `APPIMAGE_EXTRACT_AND_RUN=1`. On Arch, run the AppImage or the release binary; the `.deb` and `.rpm` are for those distributions. |
| Windows | `nsis/*-setup.exe`, `msi/*.msi` | Tauri downloads NSIS and WiX itself. Release builds use the windows subsystem, so no console window. |

You can't cross-build from a Mac: ring compiles C against the target's system headers (checked: `cargo check --target x86_64-pc-windows-msvc` stops in ring's build script). Build Linux and Windows on those systems, or with the CI workflow: `.github/workflows/build.yml` runs `cargo test` and `cargo tauri build` on `macos-latest`, `ubuntu-22.04` and `windows-latest` (on pushes to main, pull requests and by hand) and keeps each system's bundles as artifacts for 14 days. It hasn't run yet; it needs the repo on GitHub.

Platform differences in the code:
- `shell_env::inherit_login_path` (copying the login shell's `PATH` so `aws`, `helm` and other kubeconfig exec plugins are found) runs on macOS and Linux only; Windows apps already get the user's `PATH`.
- Commands the app starts (`helm`, the URL opener) don't open console windows on Windows (`CREATE_NO_WINDOW`). Exec auth plugins started by kube-rs might still flash one; not checked.
- Opening URLs: `open` / `cmd /C start` / `xdg-open`. Revealing a download: Finder / Explorer / the Downloads folder via `xdg-open`, labelled "Show in Finder" / "Show in Explorer" / "Show folder" (`prefs::reveal_label`, from the webview's user agent).
- Menus: Settings… is in the app menu on macOS, the File menu elsewhere; the Window menu is set up on macOS only.
- Quit on every system (`menu::add_quit`): macOS keeps the default app-menu Quit (⌘Q). Windows replaces the default File › Quit (no shortcut, and it posted a raw quit that skipped the unsaved-edits hold) with File › Exit labelled Alt+F4. The label is display-only, so Alt+F4 still closes just the focused window, and closing the main window quits. Linux had no File menu at all, so Settings… had landed in Edit; it now gets File with Settings… and Quit (Ctrl+Q). Exit and Quit call `app.exit(0)`, which goes through the `ExitRequested` hold. Only the macOS build has been checked; the Windows and Linux menus wait on CI or a real machine.
- Linux icons (`desktop_entry::ensure`, called at startup). On Wayland (the default in KDE Plasma 6 and GNOME), a window has no icon of its own. It has an app id, which GTK takes from the binary name (`k8s-subduer`; Tauri's `enableGTKAppId` is off), and the desktop looks up `k8s-subduer.desktop` to find the icon. The .deb and .rpm install that entry under that name, with `StartupWMClass=k8s-subduer` for X11. A bare binary, an AppImage or `cargo tauri dev` used to get a generic icon under KDE. For those runs, the app now writes `$XDG_DATA_HOME/applications/k8s-subduer.desktop` and a 256 px icon in `$XDG_DATA_HOME/icons/hicolor/`, marked `X-Subduer-Generated=true`. The entry is `NoDisplay` unless the run is an AppImage, so a dev build stays out of the app menu. Later runs update `Exec` if the binary has moved. The app removes its entry once a package's entry exists in `$XDG_DATA_DIRS`, and never touches an entry it didn't write. On X11, Tauri sets the window icon from the first PNG in `bundle.icon`, so `icons/256x256.png` is listed first; it used to be the 32 px one, which looked blurry. The bundle icon list also has 48, 64 and plain 256 px PNGs for the hicolor sizes. (`128x128@2x.png` installs only as `256x256@2`.) Tested only by unit tests and a standalone run on macOS; not yet checked on a real KDE or GNOME desktop.
- Fonts fall back to Segoe UI / Consolas on Windows and system-ui, Ubuntu, Cantarell, DejaVu Sans Mono on Linux.
- The headless UI harness and `appearance-previews.sh` assume macOS (Chrome's app path, `sips`).

### Read-only checks against a real cluster (no GUI)

```sh
cargo run -p subduer-core --example smoke    -- <context> [group/Kind]   # kinds, namespaces, first table snapshot
cargo run -p subduer-core --example overview -- <context> [--json]      # Cluster page data
cargo run -p subduer-core --example pod      -- <context> <ns> <pod>     # Pod Details data, never fetches secret values
cargo run -p subduer-core --example history  -- <context> <ns> <pod>     # last-hour CPU/memory; shows which Prometheus it found
cargo run -p subduer-core --example trace    -- <context> <cluster|ns:a,b|deploy:ns/name|sts:…|ds:…|job:…|cronjob:…|pod:ns/name> <file> <minutes>   # records a trace (backfill + minutes), prints its summary; the file must not exist
cargo run -p subduer-core --example trace    -- --summary <file>          # a trace file's header and counts
cargo run -p subduer-core --example helm     -- <context> <ns> <release> # Helm release summary (no values/manifest)
cargo run -p subduer-core --example delete_dry_run -- <context> <group/Kind> <ns|-> <name>   # server-side dry run, never deletes
cargo run -p subduer-core --example workload -- <context> <apps/Deployment|apps/StatefulSet|batch/Job|batch/CronJob> <ns> <name>   # Details data + history source; no env values or secrets
cargo run -p subduer-core --example workload_dry_run -- <context> <group/Kind> <ns> <name> [replicas]   # dry-run restart/scale, or suspend/resume and (CronJob) Run now; checks the gate refuses
cargo run -p subduer-core --example namespace_dry_run -- <context> <new-name>   # server-side dry run of creating a namespace; checks the gate refuses
cargo run -p subduer-core --example edit_dry_run -- <context> <group/Kind> <namespace|-> <name>   # YAML edit dry runs: unchanged, annotated, stale resourceVersion (conflict); checks the gate refuses; prints no YAML
cargo run -p subduer-core --example helm_rollback_dry_run -- <context> <ns> <release> <revision>   # helm rollback --dry-run=server and helm uninstall --dry-run; checks the gate refuses both
cargo run -p subduer-core --example access_check -- <context> <ns> <serviceaccount> <verb> <resource> [ns|-]   # SubjectAccessReview: may it? (changes nothing)
cargo run -p subduer-core --example config -- <context> <group/Kind> <ns|-> <name>   # Config, Network, Storage and Access Control Details data; ConfigMap values by size only, Secret values never fetched
cargo run -p subduer-core --example config_dry_run -- <context> <core/ConfigMap|core/Secret|autoscaling/HorizontalPodAutoscaler> <ns> <name>   # dry-run save (adds key subduer-dry-run), HPA range, PVC expand (+1Gi) or PV reclaim policy; checks stale versions and the gate
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
  namespace.rs   Namespace Details tab (contents, resources vs quota, releases, warnings) and create_namespace
  rbac.rs        Access Control Details tabs: rules with risky ones flagged, bindings both ways, a ServiceAccount's effective rules; check_access
  storage.rs     Storage-section Details tabs on config's shape: PVC usage from the kubelet and why it's pending, PV source and reclaim policy, class claims, CSI
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
  trace.rs       traces: Prometheus pulls into one-minute buckets, the app's samples for pods Prometheus misses, the run loop
  trace_file.rs  the .trace file (SQLite): schema, writes, summary
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
| `check_connection` | `/version` with the session's client, 15 s limit; an error means the connection is gone (403 counts as reachable). The UI calls it every 10 s and when something gives up. |
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
| `create_namespace` | Creates a namespace (DNS-1123 name), checked like `delete_resource` |
| `set_app_theme` | Native appearance of every window: `light`, `dark` or `auto` (follow the system) |
| `helm_uninstall` | `helm uninstall` (optionally `--keep-history`, `--wait`), checked like `delete_resource`; returns Helm's output |
| `helm_rollback` | `helm rollback` to a revision (optionally `--wait`), checked like `delete_resource`; returns Helm's output |
| `check_access` | SubjectAccessReview for a ServiceAccount; read-only, so not gated |
| `expand_claim` / `set_reclaim_policy` | Grow a PVC; set a PV's reclaim policy (Retain/Delete); checked like `delete_resource` |
| `open_url` | Opens an http(s) address (Ingress URLs) in the browser; anything else is refused |
| `secret_values` | Every value of a Secret, when editing starts (logged without values) |
| `save_config_data` / `set_hpa_range` | Save edited ConfigMap/Secret data (with its resourceVersion); set an HPA's min/max; checked like `delete_resource` |
| `suspend_workload` / `run_cron_job` | Set a Job's or CronJob's `spec.suspend`; start a Job from a CronJob now (returns its name); checked like `delete_resource` |
| `open_editor` | Opens (or focuses) a YAML edit window (`edit-N`) for one object, bound to the current cluster; refused for Helm releases |
| `editor_start` | Called by an edit window on open and Reload: the object's YAML (no `managedFields`), context, prod-like and unlocked |
| `editor_save` | Called by an edit window: `dry_run` validates on the server (not gated); otherwise replaces the object (PUT, keeping `resourceVersion`), gated, and emits `object-saved` to the main window |
| `editor_dirty` / `editor_close` | An edit window reports unsaved edits (holds its close and the app's quit); closes itself, discarding |
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
- **Give up instead of retrying forever.** Tokens can be expired at startup or expire in use, and retrying an expired token only re-runs the exec plugin. A watch gives up after `watch::MAX_ATTEMPTS` (5) failed attempts in a row, about 12 s, and its last table has `stopped` set. The Cluster/Node page stops polling after `overview::MAX_FAILURES` (3) failed refreshes. Either way the UI calls `check_connection` once: if it fails, the context shows "Not connected" with the reason and a Connect button (nothing retries until it's clicked; the namespace is kept); if it works, the view keeps its error with Retry (watch) or Refresh (Cluster page). A failed `connect` shows the same page.
  - **Dropped VPNs** often produce no errors: private API addresses stop answering instead of refusing, and connections already open go quiet. kube 4's client has no read timeout (so exec, logs and port-forwards can stay open), a watch only notices after its 295 s idle timeout, and any other request on a dead kept-alive connection can wait forever. So while connected, the UI calls `check_connection` every 10 s (`PROBE_EVERY`); each check gives up after 15 s (`cluster::CHECK_TIMEOUT`, long enough for an exec plugin to fetch a token), and 3 failures in a row (`MAX_PROBE_FAILURES`) show "Not connected", about 30–75 s after the drop. New connections time out after 10 s (`contexts::CONNECT_TIMEOUT`; kube's default is 30 s). Each connect bumps `Store.generation`; results from an older connection are dropped and connect resets `overview_loading`, so a refresh hung on the dead connection doesn't block the Cluster page afterwards.
  - kube's `StreamBackoff` resets on every `Ok`, and the watcher emits `Event::Init` before every relist, failed or not, so `default_backoff()` retried a failing list about once a second forever. `watch::Backoff` wraps `DefaultBackoff` and ignores those resets (its own 120 s reset timer still applies), and `Failures` doesn't count `Init` as a success.
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
- **Namespace Details** (`crates/core/src/namespace.rs`):
  - Status Active or Terminating; a stuck deletion says what it's waiting for (the true deletion conditions' messages) and lists finalizers; namespace conditions show red when True. Pod security from the `pod-security.kubernetes.io/{enforce,warn,audit}` labels.
  - **Resources:** CPU and memory used now (metrics-server; says so when absent), requested and limits of running pods (limits only when every container has one, otherwise "not set on N pods"), and the tightest ResourceQuota on requests, with a bar (amber from 90%).
  - **Contents:** pods by phase and counts of 14 common kinds, via metadata-only lists (Secrets included: no data read). Rows open that kind's list filtered to the namespace (`open_object` with an empty name).
  - **Quotas, limits and releases:** ResourceQuotas, LimitRanges and Helm releases (from `owner=helm` Secret labels), linked.
  - **Recent warnings:** the newest 20 Warning events from anything in the namespace, each with its object.
- **New namespace** (`ui/src/namespace.rs`): a header button on the Namespace list when the `create` verb is allowed. Gated: a locked context asks to allow changes first and then opens the dialog (`pending_new_namespace`). The name is checked as you type (DNS-1123 label, not already listed); on success the new namespace is selected when the watch lists it.
- **Filter to it:** on a selected namespace, sets the namespace menu to it (through `guard`, so unsaved edits are asked about first).
- **Appearance** (Settings…, `ui/src/prefs.rs`):
  - The menu item is "Settings…" with ⌘, (Ctrl+, elsewhere): after About in the app menu on macOS, first in the File menu elsewhere. It focuses the main window and sends `open-settings`.
  - The dialog shows each choice as a preview card of the app (`ui/assets/appearance-{light,dark}.jpg`, copied by Trunk; Auto overlays the two with a diagonal split). Regenerate them after visible UI changes with `sh tools/ui-harness/appearance-previews.sh` (generic demo data only).
  - It offers **Auto** only when the webview can report the system appearance (`prefers-color-scheme` is understood); **Light** and **Dark** always. Choices apply at once and are saved in localStorage (`appearance.theme`); the default is Auto, or Light where Auto isn't available.
  - Each window (main, logs, shell) applies it at startup (`prefs::follow_theme`) and again on the `storage` event when another window changes it. The CSS uses `data-theme` on `<html>`: dark colours come from the system unless it's `light`, and always when it's `dark`.
  - The main window also calls `set_app_theme` (Tauri `AppHandle::set_theme`) so title bars, scroll bars and form controls match; Auto passes `None`.
  - Shell windows pick xterm colours from the setting when they open (`prefs::is_dark`); an open terminal keeps its colours until reopened.
- **Helm release Details** (`helm::release_details`): the virtual `helm.sh/Release` kind gets a Details tab (YAML stays the summary).
  - Chart, app version, revision, last change, first and last deployed, chart description and home (Open). Values are never shown.
  - **History:** every kept revision (newest 20 decoded) with status, chart, app version, age and description; failed ones flagged.
  - **Resources:** the objects in the current manifest, linked. Only apiVersion, kind, name and namespace are read from each document (`manifest_objects`); Secret data in the manifest goes no further, and the manifest is dropped like before.
  - **Notes** as preformatted text.
  - Links to cluster-scoped kinds (e.g. a chart's ClusterRole) drop the namespace in `open_object`, since manifests don't say whether a kind is namespaced.
- **Rollback** (`helm::rollback`, `Op::Rollback`): shown when there's more than one revision (Helm uses your own credentials, so discovery verbs don't matter).
  - The dialog lists earlier revisions with chart, app version and status, defaulting to the latest one that didn't fail; picking a failed one warns. "Wait until ready" (on by default) adds `--wait --timeout 5m0s`. It explains that Helm re-applies that revision's manifest and values as a new revision and runs its hooks, and that changes made outside Helm may be overwritten.
  - It runs the `helm` CLI (found on the login-shell PATH) with arguments only, no shell: `rollback <release> <revision> --namespace=<ns> --kube-context=<context>`. Names must be valid Helm names starting with a letter or digit, so nothing can pass for a flag. Dry runs use `--dry-run=server` on Helm 4 and `--dry-run` on Helm 3. Times out after 6 minutes. Helm's errors are shown in the dialog.
  - Gated like Delete, with the prod name check; the command re-checks the context is still connected.
- **Delete a release** (`helm::uninstall`, `Op::Uninstall`): a red Delete button on releases (the generic Delete stays hidden: deleting release Secrets isn't an uninstall). The dialog lists what the release created (from Resources, first 8 and a count), warns that their data is lost and that `helm.sh/resource-policy: keep` objects and StatefulSet volume claims stay, and offers "Keep its history, so it can be rolled back later" (`--keep-history`, off) and "Wait until everything is deleted" (`--wait`, on). **The name must always be typed**, prod or not. Runs `helm uninstall <release> --namespace=… --kube-context=…` through the same no-shell runner as rollback; on success the detail panel closes and the watch drops the row.
- **Access Control Details** (`crates/core/src/rbac.rs`):
  - **Risky rules** are flagged in every rules table: full access (`*` verbs on `*` resources), can grant itself more (`escalate`, `bind`, `impersonate`), reads secrets, can exec into pods, reaches the kubelet (`nodes/proxy`). A role or binding with any shows Sensitive, or Full access.
  - **Role / ClusterRole:** rules (verbs, API groups, resources or URLs, resource names), aggregation selectors, which built-in roles it adds to (`aggregate-to-admin/edit/view`), built-in note; **Bound by**: the RoleBindings (all namespaces for a ClusterRole) and ClusterRoleBindings using it, with their subjects.
  - **RoleBinding / ClusterRoleBinding:** what it grants (role link) and where, in words (a RoleBinding to a ClusterRole is limited to its namespace); **What it grants**: the role's rules, or Role missing; **Subjects** with ServiceAccount links. Binding to `system:authenticated`/`system:unauthenticated` shows Everyone.
  - **ServiceAccount:** cloud identity (AWS IAM role, Google service account, Azure identity), token automount, the username it acts as, token and image pull secrets; **Bindings** naming it directly or through `system:serviceaccounts[:<ns>]` groups, across all namespaces; **What it can do**: every rule from those bindings with where it applies and which role it comes from; **Pods running as it**.
  - **Check access** (`rbac::check_access`): verb, resource (`pods`, `deployments.apps`, `pods/exec`), namespace (empty = all) and optional name, sent as a SubjectAccessReview for `system:serviceaccount:<ns>:<name>` with its groups. Shows Allowed with the granting rule, or Denied. Read-only, not gated.
- **Storage section Details** (`crates/core/src/storage.rs`):
  - **PersistentVolumeClaim:** requested vs capacity (an unfinished resize is called out), access modes in words, storage class (or the default, or none when there is no default), volume link, block mode, data source, selector. **Can expand** comes from its StorageClass (`allowVolumeExpansion`); static claims without a class can't grow. **Why pending**: WaitForFirstConsumer, or waiting for the provisioner. **Disk usage** (space and inodes, with bars, amber from 85%) from the kubelet `/stats/summary` of a node where a running pod mounts it; on Fargate it says unknown. **Mounted by**: the pods using it (including generic ephemeral volumes). Status Bound, Pending, Lost or Resizing.
  - **PersistentVolume:** capacity, access, reclaim policy in words, class and claim links (Released says the claim is gone), source (CSI driver and volume handle, or hostPath, NFS, EBS, …), node affinity (e.g. its zone), mount options, phase time.
  - **StorageClass:** provisioner, default, reclaim policy, binding mode in words, expansion, mount options, allowed topologies, parameters; the claims using it across namespaces (non-Bound ones flagged).
  - **VolumeAttachment:** attacher, node and volume links, attached, attach/detach errors; status Attached, Attaching, Detaching or Error. **CSIDriver:** its capabilities, and the nodes running it. **CSINode:** drivers with node ID, volume limit and topology keys. **CSIStorageCapacity:** class, capacity, largest volume, nodes.
- **Expand** (PVCs, `actions::expand_claim`, `Op::Expand`): shown only when "Can expand" is yes and `patch` is allowed. The dialog starts at the current size, accepts quantities like `40Gi`, and enables only for a bigger size; the backend checks again against the live claim. It says claims can't shrink and some drivers need a pod restart.
- **Reclaim policy** (PVs, `actions::set_reclaim_policy`, `Op::Reclaim`): switches Retain ⇄ Delete; switching to Delete warns that deleting the claim then deletes the data. PVs are cluster-scoped, so change dialogs now accept targets without a namespace.
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
- **Colours** (`ui/styles/app.css` tokens, each defined for light and for both dark blocks):
  - **CPU is blue, memory purple** everywhere they appear: usage charts (`UsageChart` adds `res-cpu`/`res-memory` via `Resource::css`), Cluster page cards, line charts, namespace bars and node-table meters. A `res-*` class sets `--series`/`--series-track`, which `.lchart-*`, `.meter`, `.meter-fill` and `.hbar` use (falling back to `--series-1`). Warning and critical meter fills keep amber/red. Light values are darker shades (blue `#2563eb`, purple `#9333ea`) for contrast on white; dark values lighter (`#60a5fa`, `#c084fc`). A `.res-dot` marks which is which beside titles and chart labels.
  - **Sidebar sections are tinted** (`app::category_class` → `cat-workloads`, `cat-config`, …; `--cat-*` tokens): the section icon and its kinds' icons take the tint, text stays neutral. Top-level items use the accent. Each tint has a darker light-mode and a lighter dark-mode value; a UI test checks every section has a tint in all three token blocks.
  - **Cluster summary tiles** get a coloured top edge and note by meaning (`StatTile` `tone`): green when all nodes are ready / no warnings, amber for pending pods or warnings, red for failed pods or nodes not ready.
  - After visible colour changes, regenerate the Appearance previews with `sh tools/ui-harness/appearance-previews.sh`.
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
- **YAML editing** (`crates/core/src/edit.rs`, `ui/src/editor.rs`):
  - Ace (`ace-builds` 1.44.0, BSD) is vendored in `ui/vendor/ace/` (source maps removed; integrity hash in its README) and copied to `dist/ace/` by Trunk, so the app works offline and loads nothing from the network. Monaco was too big; CodeMirror 6 needs a bundler.
  - Saving replaces the whole object (PUT) rather than patching, so removing a field removes it. The YAML keeps `metadata.resourceVersion`, so a change made meanwhile gets a 409, shown as "changed by someone else… Reload".
  - The edit must keep its apiVersion, kind, name and namespace; renaming would mean creating a new object.
  - Save always dry-runs first, so the confirmation diff only appears for an edit the server accepts. The diff is computed in the UI (line LCS, unchanged runs folded).
  - Each edit window is bound to the context it was opened from, like logs and shell windows, and keeps its own dirty flag in the backend, which holds the window's close and the app's quit.
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
- Tests: fake the port-forward WebSocket protocol to cover `forward::connect`; make `helm` runs testable by injecting the binary path instead of using `PATH`.
- The harness stubs are saved in `tools/ui-harness/` (uncommitted, with a README): `stub.js` (pod, delete, upload, download), `nav.js` (sidebar), `fwd.js`, `workloadstub.js`, `batchstub.js`, `configstub.js`, `networkstub.js`, `storagestub.js`, `rbacstub.js`, `helmstub.js`, `settingsstub.js`, `nsstub.js`, `logstub.js`, `execstub.js`, `ov.js`. The user can choose to commit them; tidying them into fixtures would help.
- Logs: add pods that start after the window opens to merged workload logs; add a download/save-logs button.
- Port-forward from workloads (pick a pod), and optional gating for prod.
- Helm follow-ups: a diff of the chosen revision against the current one before rolling back (`helm get manifest`), and showing values on request (gated like secrets).
- Access Control follow-ups: Check access for users and groups (and "can I" for yourself via SelfSubjectRulesReview); bindings for users and groups as their own view.
- Storage follow-ups: disk usage from Prometheus (`kubelet_volume_stats_*`) where the kubelet doesn't report it; VolumeSnapshots when the CRDs are installed.
- Network follow-ups: Gateway API kinds (HTTPRoute, Gateway) when installed; port-forward from an Ingress backend.
- Config follow-ups: TLS Secrets could show the certificate's subject and expiry (needs an X.509 parser); "restart the workloads using it" after a save; a PriorityClass's pod count.
- Jobs and CronJobs: merged Logs for a CronJob (its latest Job's pods), and Rerun for finished Jobs (a copy without the generated selector and labels).
- Upload/download of folders (tar), and a progress indicator for big files.
- Clean up unused icons that `cargo tauri icon` generated (`android/`, `ios/`, `Square*.png`, `StoreLogo.png`, `icon.ico`). The user hadn't approved deleting them.
- `response.json` at the repo root (`{"status":"success"}`) has an unknown origin and wasn't created during this work.

### Traces (designed with the user 2026-10-11; phase 1 built)

Why: Prometheus keeps only 1–2 h (UAT: `retention.time` 2h, 30 s scrapes), so right-sizing needs usage kept for days. The worry is over-provisioning: requests far above real usage.

- **Name:** a "trace". Started by choice; runs while the app is open. No background or tray mode.
- **Scope levels:** Cluster (current context), Namespace (one or more), Workload (one or more Deployments/StatefulSets/DaemonSets/CronJobs, matched by pod-name pattern as in `workload_history`, so new pods are picked up), Pod (ends with the pod). Also "Trace this…" from Details tabs. One context per trace.
- **Capture:** Prometheus range queries `by (namespace, pod, container)`, pulled at about half the server's retention (read from `/status/flags`) with overlap, deduped. Backfill whatever Prometheus still holds at start. Pods Prometheus doesn't cover (Fargate, Prometheus itself: 17 of 50 on UAT) are sampled from metrics-server/kubelet stats while running, and each workload records its data source. Missed spans are stored as gaps and shown.
- **What's recorded:** CPU and memory usage; requests and limits over time; owners (`kube_pod_owner`, `kube_replicaset_owner`) and pod template hash (so rollouts split); restarts, last terminated reason (OOMKilled), `container_oom_events_total`; CPU throttling (only exists for containers with CPU limits); replica counts. `kube_pod_labels` isn't exported on UAT, so labels come from the API.
- **Storage:** one SQLite `.trace` file per trace (bundled `rusqlite`, WAL), chosen with **Save as** when starting. 1-minute buckets of min/avg/max throughout (measured on UAT: ~37 bytes per container-minute, ~55 MB/day per 1,000 containers); charts re-bucket by zoom with `GROUP BY`. Each pull is one transaction. No secrets or env values in the file; names, numbers, labels and notes only. A Recent traces list remembers paths.
- **While running:** an indicator in the chrome (context, duration, gaps) with Stop, plus a Window-menu entry. Closing or quitting uses the unsaved-edits hold with "Stop trace and quit" / Cancel (combined with any unsaved-YAML prompt). Stop/quit does one final pull (about 10 s cap; on timeout it's a gap), writes end time and reason, commits and closes. A file left without an end time is marked "ended unexpectedly" when next opened. Switching context during a trace warns and stops it.
- **Review (works offline, no kubeconfig):** Open trace… or drop a file. Charts over the whole span with request and limit lines and gaps; a time-of-day view. The **right-sizing table**: per workload and container, current request, CPU p50/p95/p99/max, memory max, waste (request − p95) as the default sort, and risk flags (throttling, memory ≥ 90% of limit, OOMKilled, restarts, no request set). Notes on the trace and per workload, saved in the file. **Compare** two or more traces by workload and container (before/after).
- **Suggestions (adjustable in Settings later):** CPU request = p95; memory request = max × 1.15. Limits aren't suggested; they're only flagged when hit, except that a memory request above the current limit is called out. No suggestion when data is thin or gaps cover busy hours; each suggestion lists its basis.
- **Apply:** only when connected to the trace's cluster. Patches `spec.template.spec.containers[].resources.requests`, single row or selected rows, through the safety gate with a server dry-run diff (typed name on prod). Helm-managed workloads get a warning that `helm upgrade` will reset them, plus **Copy values snippet**.
- **Export:** a standalone HTML report (table and charts) for people without the app.
- **As built (phase 1):** `Trace::start` finds Prometheus, reads `storage.tsdb.retention.time`, creates the file and backfills up to 2 h (retention less 5 min). `Trace::run(stop)` pulls every 10 min (or half the retention) and samples every 30 s until `stop` resolves with a `TraceEnd`, then `finish` does a last pull (10 s limit; a failure is a gap), writes pending samples and switches the file out of WAL. Pulls re-read 5 min of overlap in chunks of up to an hour: raw points every 15 s for CPU (rate 2m), memory and throttling, folded into minutes; requests, limits, restarts, last termination reason and OOM events every minute, stored only as changes; owners from `kube_pod_owner` → `kube_replicaset_owner`/`kube_job_owner` as instant `max_over_time` queries. A failed pull is logged in the `pulls` table and retried by the next; falling behind retention records a gap. The app's samples: running pods in scope (listed every 2 min) less those with cAdvisor data in the last 5 min, from metrics-server or else kubelet stats, one minute written once it's over; Prometheus data replaces them, they never replace Prometheus'. CPU is stored in microcores and memory in bytes; `application_id` "SUBT" and `meta.format` 1 identify the file. Opening a file with no end that isn't being recorded reports `unfinished`.
- **Phases:** 1) capture and the `.trace` format, with an `examples/trace.rs` read-only check against UAT; 2) trace controls, indicator and the quit hold; 3) review charts and the right-sizing table; 4) Apply and Copy values snippet; 5) Compare and HTML export.

## 8. Working conventions

- **Git:** the user runs all git commands; a hook blocks git for Claude. Claude only edits files.
- **Before handing back:** keep lint at zero warnings for both clippy commands, run `cargo test`, and run `trunk build`.
- **Visual changes:** check with the headless harness in both light and dark mode.
- **Real-cluster checks:** read-only against UAT (writes only as server-side dry runs). Nothing has written to a cluster yet.
- **Charts:** load the dataviz skill before adding any chart.
