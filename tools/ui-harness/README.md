# UI harness (headless Chrome)

Stubs that fake `window.__TAURI__` so the UI can be rendered and screenshotted
without the app or a cluster. See RUNBOOK.md section 3 for the full recipe.

`run_e2e.py` turns them into an automated suite: it builds the UI, serves it
with each stub, runs 214 scenarios in headless Chrome and checks the page title
(where stubs record what the app sent) and text. Run it with
`python3 tools/ui-harness/run_e2e.py [filter…]`, or `--show <scenario>` to see
what one scenario saw. Stubs must hand out increasing watch ids, as the app
drops tables from older watches.

```sh
trunk build --dist /tmp/subduer-harness            # not dist/: `trunk serve` rewrites it
cp tools/ui-harness/*.js /tmp/subduer-harness/
cd /tmp/subduer-harness
sed -i '' 's|<head>|<head><script src="stub.js"></script>|' index.html   # pick one stub
python3 -m http.server 8765 &
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --disable-gpu \
  --hide-scrollbars --blink-settings=preferredColorScheme=1 --force-device-scale-factor=2 \
  --window-size=1400,900 --virtual-time-budget=9000 --screenshot=out.png \
  "http://localhost:8765/index.html?pod=limited&upload=choose"
```

`preferredColorScheme=0` renders dark mode.

| Stub | Shows | Query parameters |
|---|---|---|
| `stub.js` | A pod's Details tab, delete, upload, download | `pod=limited\|unlimited`, `hang=<command>` (that call never returns), `slow=<command>` (it answers after 1.5 s), `again=<ms>` (reopen the Download or Upload dialog), `hover=0..1`, `del=1\|2`, `ctx=prod`, `locked=1`, `upload=choose\|send`, `exists=1`, `browse=1`, `download=pick\|done` |
| `nav.js` | Sidebar with many kinds, Helm releases | `open=all\|some\|cr` |
| `fwd.js` | Port Forwards page and start dialog | `page=forwards\|dialog`, `hang=<command>` (that call never returns), `late=1` (a second Service, grafana, whose ports come at once while prometheus's take 1.5 s; the dialog is reopened for grafana) |
| `logstub.js` | A logs window | `logs=<label>`, `merged=1`, `filter=<text>` |
| `execstub.js` | A shell window (xterm) | `exec=<label>`, `exit=1` |
| `workloadstub.js` | Deployment/StatefulSet Details tab, Restart and Scale dialogs | `rolling=1`, `local=1`, `collapse=resources,pods,…`, `scroll=<px>`, `openpod=1`, `op=restart\|scale`, `sts=1`, `ctx=prod`, `typed=1`, `locked=1`, `to=<n>`, `go=1` (the sent args land in the page title; read with `--dump-dom`) |
| `batchstub.js` | CronJob/Job Details tab, Run now and Suspend/Resume dialogs | `job=1` (a Job; default a CronJob), `failed=1`, `suspended=1`, `op=run\|suspend\|resume`, `ctx=prod`, `typed=1`, `locked=1`, `go=1` (sent args land in the page title), `openjob=1`, `collapse=…`, `scroll=<px>` |
| `configstub.js` | Config-section Details tabs, ConfigMap/Secret editing and Save, HPA Edit range | `kind=cm\|secret\|hpa\|quota\|webhook\|pdb\|lease`, `show=1`, `edit=1`, `change=1`, `add=1`, `badkey=1`, `remove=1`, `save=1`, `range=1`, `ctx=prod`, `typed=1`, `locked=1`, `go=1` (sent args land in the page title), `collapse=…`, `scroll=<px>`, `yamledit=1` (YAML tab → Edit; `open_editor` args land in the title), `leave=row\|close\|yaml\|kind\|refresh\|link\|ns\|context\|cancel\|window\|quit` with `keep=1` or `discard=1` (the title then reports unsaved, prompt, editing, selection, tab, namespace and close) |
| `networkstub.js` | Network-section Details tabs (Service, Ingress, NetworkPolicy, EndpointSlice) | `kind=svc\|ingress\|netpol\|slice`, `open=1` (clicks the first Open; the URL lands in the page title), `link=1`, `collapse=…`, `scroll=<px>` |
| `storagestub.js` | Storage-section Details tabs, Expand and Reclaim policy | `kind=pvc\|pv\|sc\|va`, `pending=1`, `op=expand\|reclaim`, `size=<new size>`, `ctx=prod`, `typed=1`, `go=1` (sent args land in the page title), `collapse=…`, `scroll=<px>` |
| `rbacstub.js` | Access Control Details tabs and Check access | `kind=sa\|role\|binding`, `check=<resource>` (`secrets` is denied, others allowed; args land in the page title), `collapse=…`, `scroll=<px>` |
| `helmstub.js` | Helm release Details tab, Rollback and Delete | `op=rollback\|delete`, `keep=1` (keep history), `to=<revision>`, `nowait=1`, `fail=1`, `ctx=prod`, `typed=1`, `locked=1`, `go=1` (args land in the page title), `link=1` (follows the ClusterRole link), `collapse=…`, `scroll=<px>`. Watch ids count up, as the app drops tables from older watches. |
| `settingsstub.js` | Settings dialog and Appearance (preview cards) | `theme=auto\|light\|dark` (saved before start), `pick=auto\|light\|dark`, `open=1`, `noauto=1` (no system appearance), `click=outside\|inside` (on the backdrop, or the dialog's heading). The title reports `data-theme`, the background, what's saved, the `set_app_theme` calls and whether the dialog is open; vary `preferredColorScheme` to play the system. |
| `connstub.js` | Giving up on a failing connection: the "Not connected" page, Connect, Retry, Cluster page polling stopping | `start=fail`, `watch=stop`, `check=fail\|ok\|late`, `overview=fail\|hang`, `vpn=drop` (only the background check fails, until Connect), `click=connect\|retry\|refresh`, `at=<ms>`. Timers start at the app's first backend call, so long virtual-time budgets don't race the WebAssembly load. The title counts the backend calls and reports whether Connect or Retry is showing. |
| `nsstub.js` | Namespace Details tab, New namespace, Filter to it | `stuck=1`, `new=1`, `name=<text>`, `go=1`, `locked=1`, `ctx=prod`, `link=1` (Contents → Deployments), `filter=1`. The title reports what was sent, the list opened, or the namespace menu. |
| `editorstub.js` | A YAML edit window (Ace) | `edit=edit-1`, `hang=save\|unlock` (the real save, or allowing changes, never returns), `change=1` (replicas 2→3), `bad=1` (a syntax error), `save=1`, `confirm=1`, `typed=1`, `locked=1`, `prod=1`, `dryfail=1` (the server rejects the edit), `cancel=1`, `reload=1`, `closereq=1` (the window's close is held), `keep=1` / `discard=1`. The title reports the commands sent, the dirty flag, the open dialog, whether Ace highlighted, its error count and the status bar. Steps are timed from when the YAML is loaded. |
| `esc.js` | Loaded with every stub: presses Esc and reports the top dialog and detail panel before and after | `esc=dialog` (once a dialog is open), `esc=busy` (once its Cancel, the first action button, is disabled) or `esc=<ms>`, `eschold=1` (a key repeat follows), `escfrom=field` (from the dialog's text field, else the page's first one), `escfrom=select` (from the page's first dropdown) |
| `tracestub.js` | Traces: Trace… button, start dialog, indicator, running panel, Stop, quit and context-switch prompts | `open=1`, `menu=1`, `this=1` (a pod's Trace… button), `level=cluster\|ns\|workloads\|pod`, `ns=<namespace>`, `pick=<name>`, `go=1` (Save as and start), `cancel=1` (the Save dialog is cancelled), `running=1`, `gaps=<n>`, `err=1`, `noprom=1`, `panel=1`, `stop=1`, `quit=1\|close=1` (the backend held quitting / closing), `switch=1` (picks acme-prd), `confirm=1`, `keep=1`, `x=1` (the × button), `esc=1` (Escape). The title reports the trace commands sent, the open dialogs and the indicator. |
| `reviewstub.js` | Traces page and a trace's review: recent files, Open trace…, the right-sizing table, a row's charts and day-and-hour grid | `recent=1`, `running=1`, `page=1`, `open=1`, `cancel=1`, `bad=1`, `offline=1`, `file=1`, `sel=<name>`, `sort=<header>`, `flagged=1`, `ns=<namespace>`, `text=<filter>`, `quick=1h\|6h\|24h\|all`, `zoom=1` (drags across the CPU chart), `heat=mem`, `back=1`; applying: `apply=1`, `check=<name,…>`, `applysel=1`, `go=1`, `ctx=prod`, `typed=1`, `locked=1`, `unlock=1`, `other=1`, `refuse=1`, `fail=1`, `copy=1`; notes, compare, export: `noted=1`, `note=<text>`, `wnote=<text>`, `leave=1`, `compare=1`, `cmpwith=1`, `changed=1`, `export=1`, `exportfail=1`. The title reports the commands sent, the rows in order, the selected row, the active range and the grid's cells. |
| `ov.js` | Cluster page | needs `ov.json`: `cargo run -p subduer-core --example overview -- <context> --json`, with node and namespace names replaced by generic ones |

`appearance-previews.sh` regenerates the Appearance preview images in `ui/assets/` from `workloadstub.js`.

The stubs grew one feature at a time, so they're scrappy. Fixtures use generic
names (`acme-uat`, `shop`), never real cluster or employer names.
