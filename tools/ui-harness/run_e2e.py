#!/usr/bin/env python3
"""End-to-end UI tests: the real UI (built by Trunk) in headless Chrome,
with the backend replaced by the harness stubs in this folder. Each scenario
loads one stub with a query, lets the stub click through a flow, and checks
the page: the stubs put what the app sent to the backend (or the state they
observed) in the page title, and the rest of the page is checked as text.

    python3 tools/ui-harness/run_e2e.py              # build, then run everything
    python3 tools/ui-harness/run_e2e.py helm config  # only scenarios whose id contains a word
    python3 tools/ui-harness/run_e2e.py --show helm/rollback-default   # print what a scenario saw

Needs Google Chrome and Trunk; macOS paths by default (set CHROME to override).
"""

import concurrent.futures
import functools
import html
import http.server
import os
import re
import shutil
import socketserver
import subprocess
import sys
import tempfile
import threading

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
HARNESS = os.path.join(ROOT, "tools", "ui-harness")
CHROME = os.environ.get("CHROME", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
LIGHT, DARK = 1, 0


def S(id, stub, query, has=(), lacks=(), scheme=LIGHT, budget=8000):
    """A scenario. `has`/`lacks`: strings that must (not) appear in the title
    or page text; a string starting with `re:` is a regular expression."""
    return dict(id=id, stub=stub, query=query, has=list(has), lacks=list(lacks), scheme=scheme, budget=budget)


SCENARIOS = [
    # ---- Pods (stub.js): details, delete, upload, download ----
    S("pod/details", "stub.js", "pod=limited", has=["Resources", "QoS class", "Burstable", "Forward", "Upload", "Shell", "Logs"]),
    S("pod/details-dark", "stub.js", "pod=unlimited", scheme=DARK, has=["Details", "YAML"]),
    # ---- Navigation ----
    # Section headings are uppercase on screen only (CSS); the text is title case.
    S("nav/sidebar", "nav.js", "open=all", has=["Workloads 7", "Config 9", "Network 5", "Storage 3", "Port Forwards", "Cluster"]),
    S("nav/forwards", "fwd.js", "page=forwards", has=["Port Forwards"]),
    # ---- Deployments / StatefulSets ----
    S("workload/details", "workloadstub.js", "rolling=1", has=["Rolling out", "Pods", "Conditions", "Resources"]),
    S("workload/restart", "workloadstub.js", "op=restart&go=1", has=["restart_workload", '"name":"acme-api"']),
    S("workload/scale", "workloadstub.js", "op=scale&to=5&go=1", has=["scale_workload", '"replicas":5']),
    S("workload/prod-needs-name", "workloadstub.js", "op=restart&ctx=prod&go=1", lacks=["restart_workload"]),
    S("workload/prod-typed", "workloadstub.js", "op=restart&ctx=prod&typed=1&go=1", has=["restart_workload", '"context":"acme-prd"']),
    S("workload/statefulset", "workloadstub.js", "sts=1", has=["StatefulSet", "acme-db"]),
    # ---- Jobs / CronJobs ----
    S("batch/cronjob", "batchstub.js", "", has=["Jobs", "Next run", "manual", "Run now"]),
    S("batch/run-now", "batchstub.js", "op=run&go=1", has=["run_cron_job", '"name":"nightly"']),
    S("batch/suspend-cron", "batchstub.js", "op=suspend&go=1", has=["suspend_workload", '"suspend":true']),
    S("batch/resume-cron", "batchstub.js", "op=resume&suspended=1&go=1", has=["suspend_workload", '"suspend":false']),
    S("batch/suspend-job", "batchstub.js", "job=1&op=suspend&go=1", has=["suspend_workload", '"kind":"Job"']),
    S("batch/failed-job", "batchstub.js", "job=1&failed=1", has=["Failed", "took 30m 0s"], lacks=["Suspend"]),
    S("batch/open-job", "batchstub.js", "openjob=1", has=["opened jobs list"]),
    # ---- ConfigMaps / Secrets ----
    S("config/configmap", "configstub.js", "kind=cm", has=["Data", "LOG_LEVEL", "Used by", "Managed by Helm"]),
    S("config/secret-masked", "configstub.js", "kind=secret", lacks=["hunter2", "tok_8f2a9c1e77"], has=["Show"]),
    S("config/secret-show", "configstub.js", "kind=secret&show=1", has=["tok_8f2a9c1e77"], lacks=["hunter2"]),
    S("config/save-configmap", "configstub.js", "kind=cm&edit=1&change=1&add=1&remove=1&save=1&go=1",
      has=["save_config_data", '"resource_version":"4711"', '"key":"FEATURE_FLAGS"', '"key":"LOG_LEVEL","value":null']),
    S("config/save-prod-typed", "configstub.js", "kind=cm&ctx=prod&edit=1&change=1&save=1&typed=1&go=1",
      has=["save_config_data", '"context":"acme-prd"']),
    S("config/save-prod-untyped", "configstub.js", "kind=cm&ctx=prod&edit=1&change=1&save=1&go=1", lacks=["save_config_data"]),
    S("config/save-secret", "configstub.js", "kind=secret&edit=1&change=1&save=1&go=1",
      has=["save_config_data", '"key":"API_TOKEN","value":"tok_rotated_2026"']),
    S("config/bad-key", "configstub.js", "kind=cm&edit=1&badkey=1", has=["isn't a valid key"]),
    S("config/locked-save", "configstub.js", "kind=cm&locked=1&edit=1&change=1&save=1", has=["Allow changes?"]),
    S("config/hpa-range", "configstub.js", "kind=hpa&range=1&go=1", has=["set_hpa_range", '"min":2', '"max":14']),
    S("config/quota", "configstub.js", "kind=quota", has=["At limit", "requests.memory"]),
    S("config/webhook", "configstub.js", "kind=webhook", has=["check.acme.io", "Fail: requests are rejected"]),
    S("config/pdb", "configstub.js", "kind=pdb", has=["Blocking"]),
    S("config/lease", "configstub.js", "kind=lease", has=["Held", "Changed hands"]),
    # ---- Unsaved edits: every way out asks, Keep stays, Discard leaves ----
    *[
        scenario
        for leave in ["row", "close", "yaml", "kind", "refresh", "link", "ns", "cancel", "window", "quit"]
        for scenario in [
            S(f"unsaved/{leave}-asks", "configstub.js", f"kind=cm&edit=1&change=1&leave={leave}",
              has=["prompt=true", "editing=true", "unsaved=true"]),
            S(f"unsaved/{leave}-keep", "configstub.js", f"kind=cm&edit=1&change=1&leave={leave}&keep=1",
              has=["prompt=false", "editing=true", "unsaved=true"]),
            S(f"unsaved/{leave}-discard", "configstub.js", f"kind=cm&edit=1&change=1&leave={leave}&discard=1",
              has=["prompt=false", "editing=false", "unsaved=false"]),
        ]
    ],
    S("unsaved/clean-edit-leaves", "configstub.js", "kind=cm&edit=1&leave=row", has=["prompt=false", "selected=othershop"]),
    S("unsaved/window-discard-closes", "configstub.js", "kind=cm&edit=1&change=1&leave=window&discard=1", has=["closed=window"]),
    S("unsaved/quit-discard-quits", "configstub.js", "kind=cm&edit=1&change=1&leave=quit&discard=1", has=["closed=quit"]),
    # ---- Network ----
    S("network/service", "networkstub.js", "kind=svc", has=["Partly ready", "Endpoints", "Routed from"]),
    S("network/ingress", "networkstub.js", "kind=ingress", has=["Backend missing", "no such service", "URLs"]),
    S("network/open-url", "networkstub.js", "kind=ingress&open=1", has=['open_url {"url":"https://shop.acme.io/"}']),
    S("network/netpol", "networkstub.js", "kind=netpol", has=["Denies outgoing", "Pods it applies to"]),
    S("network/slice", "networkstub.js", "kind=slice", has=["2 of 3 ready"]),
    # ---- Storage ----
    S("storage/pvc", "storagestub.js", "kind=pvc", has=["Disk usage", "87% used", "Mounted by", "Expand"]),
    S("storage/pending", "storagestub.js", "kind=pvc&pending=1", has=["Why pending", "WaitForFirstConsumer"]),
    S("storage/expand", "storagestub.js", "kind=pvc&op=expand&size=40Gi&go=1", has=["expand_claim", '"size":"40Gi"']),
    S("storage/expand-smaller", "storagestub.js", "kind=pvc&op=expand&size=10Gi&go=1", lacks=["expand_claim"]),
    S("storage/reclaim-prod", "storagestub.js", "kind=pv&op=reclaim&ctx=prod&typed=1&go=1",
      has=["set_reclaim_policy", '"policy":"Retain"', '"namespace":""']),
    S("storage/reclaim-prod-untyped", "storagestub.js", "kind=pv&op=reclaim&ctx=prod&go=1", lacks=["set_reclaim_policy"]),
    S("storage/class", "storagestub.js", "kind=sc", has=["Default", "Claims", "Parameters"]),
    S("storage/attachment", "storagestub.js", "kind=va", has=["Error", "Attach error"]),
    # ---- Access control ----
    S("rbac/serviceaccount", "rbacstub.js", "kind=sa", has=["Sensitive", "What it can do", "reaches the kubelet", "Check access"]),
    S("rbac/check-allowed", "rbacstub.js", "kind=sa&check=pods", has=["check_access", "Allowed"]),
    S("rbac/check-denied", "rbacstub.js", "kind=sa&check=secrets", has=["Denied"]),
    S("rbac/role", "rbacstub.js", "kind=role", has=["Bound by", "reads secrets"]),
    S("rbac/binding", "rbacstub.js", "kind=binding", has=["Everyone", "What it grants"]),
    # ---- Helm ----
    S("helm/release", "helmstub.js", "", has=["History", "Resources", "Notes", "Rollback", "Delete"]),
    S("helm/rollback-default", "helmstub.js", "op=rollback&go=1", has=["helm_rollback", '"revision":40', '"wait":true']),
    S("helm/rollback-pick", "helmstub.js", "op=rollback&to=39&nowait=1&go=1", has=['"revision":39', '"wait":false']),
    S("helm/rollback-failed-warns", "helmstub.js", "op=rollback&to=41", has=["That revision failed"]),
    S("helm/rollback-prod-untyped", "helmstub.js", "op=rollback&ctx=prod&go=1", lacks=["helm_rollback"]),
    S("helm/rollback-error", "helmstub.js", "op=rollback&go=1&fail=1", has=["timed out waiting"]),
    S("helm/delete-needs-name", "helmstub.js", "op=delete&go=1", lacks=["helm_uninstall"]),
    S("helm/delete", "helmstub.js", "op=delete&typed=1&go=1", has=["helm_uninstall", '"keep_history":false']),
    S("helm/delete-keep-history", "helmstub.js", "op=delete&keep=1&typed=1&go=1", has=['"keep_history":true']),
    S("helm/cluster-scoped-link", "helmstub.js", "link=1", has=["selected=acme-api-reader"]),
    # ---- Namespaces ----
    S("ns/details", "nsstub.js", "", has=["Contents", "Recent warnings", "Quota on requests", "Filter to it"]),
    S("ns/stuck", "nsstub.js", "stuck=1", has=["Terminating", "widgets.acme.io"]),
    S("ns/create", "nsstub.js", "new=1&name=team-sandbox&go=1", has=['create_namespace {"context":"acme-uat","name":"team-sandbox"}']),
    S("ns/create-bad-name", "nsstub.js", "new=1&name=Bad_Name&go=1", lacks=["create_namespace"], has=["Use lowercase letters"]),
    S("ns/create-existing", "nsstub.js", "new=1&name=ops&go=1", lacks=["create_namespace"], has=["already exists"]),
    S("ns/create-after-unlock", "nsstub.js", "new=1&locked=1&name=team-sandbox&go=1", has=["create_namespace"]),
    S("ns/contents-link", "nsstub.js", "link=1", has=["list=Deployment ns=shop"]),
    S("ns/filter", "nsstub.js", "filter=1", has=["ns-menu=shop"]),
    # ---- Settings / appearance ----
    S("settings/auto-follows-dark", "settingsstub.js", "open=1", scheme=DARK, has=["data-theme=none", "bg=rgb(22, 25, 30)", "Auto*"]),
    S("settings/light-over-dark", "settingsstub.js", "pick=light", scheme=DARK, has=["data-theme=light", "bg=rgb(246, 247, 249)", "native=auto,light"]),
    S("settings/dark-over-light", "settingsstub.js", "pick=dark", has=["data-theme=dark", "bg=rgb(22, 25, 30)"]),
    S("settings/saved-dark-restart", "settingsstub.js", "theme=dark", has=["data-theme=dark", "native=dark"]),
    S("settings/back-to-auto", "settingsstub.js", "theme=light&pick=auto", scheme=DARK, has=["data-theme=none", "saved=auto"]),
    S("settings/no-auto", "settingsstub.js", "open=1&noauto=1", has=["buttons=Light*|Dark"], lacks=["Auto"]),
    # ---- Logs and shell windows ----
    S("logs/window", "logstub.js", "logs=app&merged=1", has=["GET /healthz", "web-7d9f-abcde/app", "Wrap"], lacks=["bad response"]),
    S("exec/window", "execstub.js", "exec=app", has=["Shell"]),
]


def build(out):
    subprocess.run(["trunk", "build", "--dist", out], cwd=ROOT, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    index = open(os.path.join(out, "index.html")).read()
    for f in os.listdir(HARNESS):
        if f.endswith(".js"):
            shutil.copy(os.path.join(HARNESS, f), out)
            page = index.replace("<head>", f'<head><script src="{f}"></script>', 1)
            open(os.path.join(out, f"page-{f[:-3]}.html"), "w").write(page)


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


def serve(directory):
    handler = functools.partial(QuietHandler, directory=directory)
    server = socketserver.ThreadingTCPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def run(port, scenario):
    url = f"http://127.0.0.1:{port}/page-{scenario['stub'][:-3]}.html?{scenario['query']}"
    dom = subprocess.run(
        [CHROME, "--headless=new", "--disable-gpu", f"--blink-settings=preferredColorScheme={scenario['scheme']}",
         f"--virtual-time-budget={scenario['budget']}", "--dump-dom", url],
        capture_output=True, text=True, timeout=120,
    ).stdout
    title = html.unescape(m.group(1)) if (m := re.search(r"<title>(.*?)</title>", dom, re.S)) else ""
    body = dom.split("</head>", 1)[-1]
    body = re.sub(r"<script.*?</script>", " ", body, flags=re.S)
    text = html.unescape(re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", body)))
    seen = title + "\n" + text

    def found(pattern):
        return re.search(pattern[3:], seen) is not None if pattern.startswith("re:") else pattern in seen

    failures = [f"missing {p!r}" for p in scenario["has"] if not found(p)]
    failures += [f"unexpected {p!r}" for p in scenario["lacks"] if found(p)]
    return failures, title, text


def main(args):
    show = None
    if args[:1] == ["--show"]:
        show, args = args[1], []
    picked = [s for s in SCENARIOS if (show and s["id"] == show) or (not show and (not args or any(a in s["id"] for a in args)))]
    if not picked:
        sys.exit("no scenarios match")
    out = tempfile.mkdtemp(prefix="subduer-e2e-")
    try:
        print(f"building the UI into {out}…", flush=True)
        build(out)
        server = serve(out)
        port = server.server_address[1]
        if show:
            failures, title, text = run(port, picked[0])
            print(f"title: {title}\ntext: {text[:3000]}\nfailures: {failures}")
            return
        failed = flaky = 0
        jobs = int(os.environ.get("E2E_JOBS", "1"))
        with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
            for scenario, result in zip(picked, pool.map(lambda s: run(port, s), picked)):
                failures, title, _ = result
                if failures:
                    # Headless Chrome can run out of time on a busy machine:
                    # retry once on its own before calling it a failure.
                    failures, title, _ = run(port, scenario)
                    if not failures:
                        flaky += 1
                        print(f"ok   {scenario['id']}   (passed on retry)")
                        continue
                mark = "ok  " if not failures else "FAIL"
                print(f"{mark} {scenario['id']}")
                for f in failures:
                    print(f"       {f}   (title: {title[:200]})")
                failed += bool(failures)
        server.shutdown()
        note = f", {flaky} needed a retry" if flaky else ""
        print(f"\n{len(picked) - failed} passed, {failed} failed, {len(picked)} scenarios{note}")
        sys.exit(1 if failed else 0)
    finally:
        shutil.rmtree(out, ignore_errors=True)


if __name__ == "__main__":
    main(sys.argv[1:])
