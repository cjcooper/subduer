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
| `ov.js` | Cluster page | needs `ov.json`: `cargo run -p subduer-core --example overview -- <context> --json`, with node and namespace names replaced by generic ones |

The stubs grew one feature at a time, so they're scrappy. Fixtures use generic
names (`acme-uat`, `shop`), never real cluster or employer names.
