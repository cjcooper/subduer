# UI harness (headless Chrome)

Stubs that fake `window.__TAURI__` so the UI can be rendered and screenshotted
without the app or a cluster. See RUNBOOK.md section 3 for the full recipe.

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
| `stub.js` | A pod's Details tab, delete, upload, download | `pod=limited\|unlimited`, `hover=0..1`, `del=1\|2`, `ctx=prod`, `locked=1`, `upload=choose\|send`, `exists=1`, `browse=1`, `download=pick\|done` |
| `nav.js` | Sidebar with many kinds, Helm releases | `open=all\|some\|cr` |
| `fwd.js` | Port Forwards page and start dialog | `page=forwards\|dialog` |
| `logstub.js` | A logs window | `logs=<label>`, `merged=1`, `filter=<text>` |
| `execstub.js` | A shell window (xterm) | `exec=<label>`, `exit=1` |
| `workloadstub.js` | Deployment/StatefulSet Details tab, Restart and Scale dialogs | `rolling=1`, `local=1`, `collapse=resources,pods,…`, `scroll=<px>`, `openpod=1`, `op=restart\|scale`, `sts=1`, `ctx=prod`, `typed=1`, `locked=1`, `to=<n>`, `go=1` (the sent args land in the page title; read with `--dump-dom`) |
| `batchstub.js` | CronJob/Job Details tab, Run now and Suspend/Resume dialogs | `job=1` (a Job; default a CronJob), `failed=1`, `suspended=1`, `op=run\|suspend\|resume`, `ctx=prod`, `typed=1`, `locked=1`, `go=1` (sent args land in the page title), `openjob=1`, `collapse=…`, `scroll=<px>` |
| `configstub.js` | Config-section Details tabs, ConfigMap/Secret editing and Save, HPA Edit range | `kind=cm\|secret\|hpa\|quota\|webhook\|pdb\|lease`, `show=1`, `edit=1`, `change=1`, `add=1`, `badkey=1`, `remove=1`, `save=1`, `range=1`, `ctx=prod`, `typed=1`, `locked=1`, `go=1` (sent args land in the page title), `collapse=…`, `scroll=<px>`, `leave=row\|close\|yaml\|kind\|refresh\|link\|ns\|context\|cancel\|window\|quit` with `keep=1` or `discard=1` (the title then reports unsaved, prompt, editing, selection, tab, namespace and close) |
| `networkstub.js` | Network-section Details tabs (Service, Ingress, NetworkPolicy, EndpointSlice) | `kind=svc\|ingress\|netpol\|slice`, `open=1` (clicks the first Open; the URL lands in the page title), `link=1`, `collapse=…`, `scroll=<px>` |
| `storagestub.js` | Storage-section Details tabs, Expand and Reclaim policy | `kind=pvc\|pv\|sc\|va`, `pending=1`, `op=expand\|reclaim`, `size=<new size>`, `ctx=prod`, `typed=1`, `go=1` (sent args land in the page title), `collapse=…`, `scroll=<px>` |
| `rbacstub.js` | Access Control Details tabs and Check access | `kind=sa\|role\|binding`, `check=<resource>` (`secrets` is denied, others allowed; args land in the page title), `collapse=…`, `scroll=<px>` |
| `helmstub.js` | Helm release Details tab, Rollback and Delete | `op=rollback\|delete`, `keep=1` (keep history), `to=<revision>`, `nowait=1`, `fail=1`, `ctx=prod`, `typed=1`, `locked=1`, `go=1` (args land in the page title), `link=1` (follows the ClusterRole link), `collapse=…`, `scroll=<px>`. Watch ids count up, as the app drops tables from older watches. |
| `settingsstub.js` | Settings dialog and Appearance (preview cards) | `theme=auto\|light\|dark` (saved before start), `pick=auto\|light\|dark`, `open=1`, `noauto=1` (no system appearance). The title reports `data-theme`, the background, what's saved and the `set_app_theme` calls; vary `preferredColorScheme` to play the system. |
| `nsstub.js` | Namespace Details tab, New namespace, Filter to it | `stuck=1`, `new=1`, `name=<text>`, `go=1`, `locked=1`, `ctx=prod`, `link=1` (Contents → Deployments), `filter=1`. The title reports what was sent, the list opened, or the namespace menu. |
| `ov.js` | Cluster page | needs `ov.json`: `cargo run -p subduer-core --example overview -- <context> --json`, with node and namespace names replaced by generic ones |

`appearance-previews.sh` regenerates the Appearance preview images in `ui/assets/` from `workloadstub.js`.

The stubs grew one feature at a time, so they're scrappy. Fixtures use generic
names (`acme-uat`, `shop`), never real cluster or employer names.
